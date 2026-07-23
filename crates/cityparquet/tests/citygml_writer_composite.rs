//! W-M3 CompositeSolid + semantics round-trip oracle over a real fixture.
//!
//! `b1_lod2_cs_w_sem.gml` (one Building whose `bldg:lod2Solid` is a 2-member
//! `gml:CompositeSolid` with `boundedBy` semantic surfaces) is converted to a
//! package, written back to `.gml`, and re-converted. The oracle asserts the
//! CompositeSolid's STRUCTURE (member/face/ring) AND its `semantics`
//! (`surfaces` + `values`) survive the round trip, that all 9 surfaces are
//! emitted (not dropped), and that the re-read package still carries semantics.

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
type Structure = BTreeMap<(String, u8), Vec<Vec<Vec<Ring>>>>;
type SemanticsMap = BTreeMap<(String, u8), Value>;

fn fixture() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/b1_lod2_cs_w_sem.gml")
}

/// `b1_lod2_cs_w_sem.gml` (a real, hand-authored TU Delft test fixture) has
/// no `srsName`/envelope at all. Since `scan` now hard-fails on
/// coordinate-bearing input with no resolvable CRS (spec "CRS rules"), a CRS
/// is injected onto the REAL parsed header before converting — the writer
/// (`crate::citygml::writer`) then emits `srsName` into the round-tripped
/// `.gml`'s own envelope, so the SECOND convert below resolves a CRS from
/// that file natively, without needing a second injection.
fn source_with_crs(path: &Path) -> Source {
    let raw = Source::open(path).unwrap();
    let mut header = raw.header().clone();
    header
        .metadata
        .get_or_insert(cityparquet::cjseq::Metadata {
            geographical_extent: None,
            identifier: None,
            point_of_contact: None,
            reference_date: None,
            reference_system: None,
            title: None,
        })
        .reference_system = Some(cityparquet::citygml::crs::reference_system("7415"));
    let features: Vec<_> = raw.features().unwrap().map(|f| f.unwrap()).collect();
    Source::from_parts(
        header,
        features,
        raw.doc_appearance().cloned(),
        raw.format(),
    )
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

/// The structural decomposition of every CompositeSolid, keyed by `(id, major)`.
fn composite_structure(pkg: &Path) -> Structure {
    let (tables, meta) = open_meta(pkg);
    let mut map = Structure::new();
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
                for (lod, decoded, _props) in &obj.geometries {
                    let DecodedKind::GeometryCollection(members) = &decoded.kind else {
                        continue;
                    };
                    let Some(major) = lod
                        .as_ref()
                        .map(|l| l.major())
                        .filter(|m| (1..=4).contains(m))
                    else {
                        continue;
                    };
                    let structure: Vec<Vec<Vec<Ring>>> = members
                        .iter()
                        .map(|member| match member {
                            DecodedKind::PolyhedralSurface(faces) => faces
                                .iter()
                                .map(|face| {
                                    face.iter()
                                        .map(|r| canonical_ring(r, &decoded.coords))
                                        .collect()
                                })
                                .collect(),
                            _ => Vec::new(),
                        })
                        .collect();
                    map.insert((obj.id.clone(), major), structure);
                }
            }
        }
    }
    map
}

/// The stored `geometry_properties.semantics` of every CompositeSolid, keyed by
/// `(id, major)`.
fn semantics_map(pkg: &Path) -> SemanticsMap {
    let (tables, meta) = open_meta(pkg);
    let mut map = SemanticsMap::new();
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
                    if !matches!(decoded.kind, DecodedKind::GeometryCollection(_)) {
                        continue;
                    }
                    let Some(major) = lod
                        .as_ref()
                        .map(|l| l.major())
                        .filter(|m| (1..=4).contains(m))
                    else {
                        continue;
                    };
                    // G7: semantics are the flattened top-level `surfaces` +
                    // `face_semantics` (§8), not a nested `semantics` object.
                    let surfaces = props.as_ref().and_then(|p| p.get("surfaces"));
                    let face_semantics = props.as_ref().and_then(|p| p.get("face_semantics"));
                    if surfaces.is_some() || face_semantics.is_some() {
                        map.insert(
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
    map
}

#[test]
fn b1_composite_solid_semantics_round_trip_gml_to_parquet_to_gml() {
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

    // All 9 b1 semantic surfaces (1 Ground, 4 Roof, 4 Wall) are now emitted.
    assert_eq!(
        report.semantic_surfaces_written, 9,
        "all 9 surfaces emitted"
    );
    assert_eq!(report.semantic_surfaces_dropped, 0, "nothing dropped");
    assert_eq!(report.multi_solids_skipped, 0);

    convert(&ConvertOptions::new(out_gml.clone(), pkg2.clone())).unwrap();

    // Geometry structure (members/faces/rings) survives.
    let before = composite_structure(&pkg);
    let after = composite_structure(&pkg2);
    assert!(
        !before.is_empty(),
        "the original package must have a CompositeSolid"
    );
    assert_eq!(
        before, after,
        "CompositeSolid structure must survive the round trip"
    );

    // Semantics (surfaces + values) survive.
    let sem_before = semantics_map(&pkg);
    let sem_after = semantics_map(&pkg2);
    assert!(
        !sem_before.is_empty(),
        "the original package must carry semantics"
    );
    assert_eq!(
        sem_before, sem_after,
        "CompositeSolid semantics must survive the round trip"
    );
}
