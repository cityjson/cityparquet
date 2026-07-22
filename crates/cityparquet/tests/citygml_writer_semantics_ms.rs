//! W-M3 MultiSurface + semantics round-trip oracle over a real fixture.
//!
//! `railway_lod3_fragment.gml` (a Building whose geometry lives only in its
//! `boundedBy` semantic surfaces — no solid — including `opening` Door/Window
//! surfaces) is converted to a package, written back to `.gml`, and
//! re-converted. The oracle asserts the MultiSurface geometry (faces/rings) and
//! its `semantics` (`surfaces` + `values`) survive, and no surface is dropped.

use std::collections::BTreeMap;
use std::fs::File;
use std::path::Path;

use cityparquet::citygml::writer::{WriteOptions, write_package};
use cityparquet::decode::decode_batch;
use cityparquet::package::{ConvertOptions, convert, convert_source};
use cityparquet::reader::CityParquetReaderBuilder;
use cityparquet::source::Source;
use cityparquet::stac::properties::PackageTables;
use cityparquet::wkb_read::DecodedKind;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use serde_json::Value;

type Ring = Vec<(i64, i64, i64)>;
type FaceStruct = BTreeMap<(String, u8), Vec<Vec<Ring>>>;
type SemanticsMap = BTreeMap<(String, u8), Value>;

fn fixture() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/railway_lod3_fragment.gml")
}

/// `railway_lod3_fragment.gml` has no `srsName`/envelope at all. Since `scan`
/// now hard-fails on coordinate-bearing input with no resolvable CRS (spec
/// "CRS rules"), a CRS is injected onto the REAL parsed header before
/// converting — the writer (`crate::citygml::writer`) then emits `srsName`
/// into the round-tripped `.gml`'s own envelope, so the SECOND convert below
/// resolves a CRS from that file natively, without needing a second
/// injection.
fn source_with_crs(path: &Path) -> Source {
    let raw = Source::open(path).unwrap();
    let mut header = raw.header().clone();
    header
        .metadata
        .get_or_insert_with(|| cityparquet::cjseq::Metadata {
            geographical_extent: None,
            identifier: None,
            point_of_contact: None,
            reference_date: None,
            reference_system: None,
            title: None,
        })
        .reference_system = Some(cityparquet::citygml::crs::reference_system("7415"));
    let features: Vec<_> = raw.features().unwrap().map(|f| f.unwrap()).collect();
    Source::from_parts(header, features, raw.doc_appearance().cloned(), raw.format())
}

fn mm(v: f64) -> i64 {
    (v * 1000.0).round() as i64
}

fn canonical_ring(ring: &[usize], coords: &[[f64; 3]]) -> Ring {
    let tuples: Ring = ring
        .iter()
        .map(|&i| (mm(coords[i][0]), mm(coords[i][1]), mm(coords[i][2])))
        .collect();
    let Some(min_idx) = (0..tuples.len()).min_by_key(|&i| tuples[i]) else {
        return tuples;
    };
    let mut rot = tuples[min_idx..].to_vec();
    rot.extend_from_slice(&tuples[..min_idx]);
    rot
}

fn open_meta(pkg: &Path) -> (PackageTables, cityparquet::schema::CityMetadata) {
    let tables = PackageTables::open(pkg).unwrap();
    let meta = ParquetRecordBatchReaderBuilder::try_new(File::open(&tables.tables[0]).unwrap())
        .unwrap()
        .cityparquet_metadata()
        .unwrap();
    (tables, meta)
}

/// The MultiSurface faces (as canonical rings) and stored semantics, keyed by
/// `(id, major)`.
fn multisurface(pkg: &Path) -> (FaceStruct, SemanticsMap) {
    let (tables, meta) = open_meta(pkg);
    let mut faces_map = FaceStruct::new();
    let mut sem_map = SemanticsMap::new();
    for path in &tables.tables {
        let reader = ParquetRecordBatchReaderBuilder::try_new(File::open(path).unwrap())
            .unwrap()
            .build()
            .unwrap();
        for batch in reader {
            let batch = batch.unwrap();
            for obj in decode_batch(&batch, &meta).unwrap() {
                if obj.object.thetype != "Building" {
                    continue;
                }
                for (lod, decoded, props) in &obj.geometries {
                    let DecodedKind::MultiPolygon(faces) = &decoded.kind else {
                        continue;
                    };
                    let Some(major) = lod
                        .as_ref()
                        .map(|l| l.major())
                        .filter(|m| (1..=4).contains(m))
                    else {
                        continue;
                    };
                    let structure: Vec<Vec<Ring>> = faces
                        .iter()
                        .map(|face| {
                            face.iter()
                                .map(|r| canonical_ring(r, &decoded.coords))
                                .collect()
                        })
                        .collect();
                    faces_map.insert((obj.id.clone(), major), structure);
                    // G7: semantics are the flattened top-level `surfaces` +
                    // `face_semantics` (§8), not a nested `semantics` object.
                    let surfaces = props.as_ref().and_then(|p| p.get("surfaces"));
                    let face_semantics = props.as_ref().and_then(|p| p.get("face_semantics"));
                    if surfaces.is_some() || face_semantics.is_some() {
                        sem_map.insert(
                            (obj.id.clone(), major),
                            serde_json::json!({
                                "surfaces": surfaces,
                                "face_semantics": face_semantics,
                            }),
                        );
                    }
                }
            }
        }
    }
    (faces_map, sem_map)
}

#[test]
fn railway_multisurface_semantics_round_trip_gml_to_parquet_to_gml() {
    let tmp = tempfile::tempdir().unwrap();
    let pkg = tmp.path().join("pkg");
    let out_gml = tmp.path().join("out.gml");
    let pkg2 = tmp.path().join("pkg2");

    convert_source(
        &source_with_crs(&fixture()),
        &ConvertOptions::new(fixture(), pkg.clone()),
    )
    .unwrap();
    let report = write_package(&WriteOptions {
        package_dir: pkg.clone(),
        output: out_gml.clone(),
    })
    .unwrap();
    assert!(
        report.semantic_surfaces_written > 0,
        "MultiSurface semantics must be emitted"
    );
    assert_eq!(
        report.multisurface_null_faces_dropped, 0,
        "no null MS faces in a CityGML source"
    );
    assert_eq!(report.semantic_surfaces_dropped, 0);

    convert(&ConvertOptions::new(out_gml.clone(), pkg2.clone())).unwrap();

    let (faces_before, sem_before) = multisurface(&pkg);
    let (faces_after, sem_after) = multisurface(&pkg2);
    assert!(
        !faces_before.is_empty(),
        "the original package must have a MultiSurface"
    );
    assert!(
        !sem_before.is_empty(),
        "the original package must carry semantics"
    );
    assert_eq!(
        faces_before, faces_after,
        "MultiSurface geometry must survive the round trip"
    );
    assert_eq!(
        sem_before, sem_after,
        "MultiSurface semantics must survive the round trip"
    );
}
