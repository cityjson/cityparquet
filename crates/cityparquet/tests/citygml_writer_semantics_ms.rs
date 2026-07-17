//! W-M3 MultiSurface + semantics round-trip oracle over a real fixture.
//!
//! `railway_lod3_fragment.gml` (a Building whose geometry lives only in its
//! `boundedBy` semantic surfaces — no solid — including `opening` Door/Window
//! surfaces) is converted to a package, written back to `.gml`, and
//! re-converted. The oracle asserts the MultiSurface geometry (faces/rings) and
//! its `semantics` (`surfaces` + `values`) survive, and no surface is dropped.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use cityparquet::citygml::writer::{WriteOptions, write_package};
use cityparquet::decode::decode_batch;
use cityparquet::package::{ConvertOptions, convert};
use cityparquet::reader::CityParquetReaderBuilder;
use cityparquet::schema::PackageManifest;
use cityparquet::wkb_read::DecodedKind;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use serde_json::Value;

type Ring = Vec<(i64, i64, i64)>;
type FaceStruct = BTreeMap<(String, u8), Vec<Vec<Ring>>>;
type SemanticsMap = BTreeMap<(String, u8), Value>;

fn fixture() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/railway_lod3_fragment.gml")
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

fn open_meta(pkg: &Path) -> (PackageManifest, cityparquet::schema::CityParquetMetadata) {
    let manifest: PackageManifest =
        serde_json::from_str(&fs::read_to_string(pkg.join("metadata.json")).unwrap()).unwrap();
    let meta = ParquetRecordBatchReaderBuilder::try_new(
        fs::File::open(pkg.join(&manifest.tables[0])).unwrap(),
    )
    .unwrap()
    .cityparquet_metadata()
    .unwrap();
    (manifest, meta)
}

/// The MultiSurface faces (as canonical rings) and stored semantics, keyed by
/// `(id, major)`.
fn multisurface(pkg: &Path) -> (FaceStruct, SemanticsMap) {
    let (manifest, meta) = open_meta(pkg);
    let mut faces_map = FaceStruct::new();
    let mut sem_map = SemanticsMap::new();
    for name in &manifest.tables {
        let reader =
            ParquetRecordBatchReaderBuilder::try_new(fs::File::open(pkg.join(name)).unwrap())
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

    convert(&ConvertOptions::new(fixture(), pkg.clone())).unwrap();
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
