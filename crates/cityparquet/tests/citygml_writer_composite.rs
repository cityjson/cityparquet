//! W-M3 CompositeSolid + semantics round-trip oracle over a real fixture.
//!
//! `b1_lod2_cs_w_sem.gml` (one Building whose `bldg:lod2Solid` is a 2-member
//! `gml:CompositeSolid` with `boundedBy` semantic surfaces) is converted to a
//! package, written back to `.gml`, and re-converted. The oracle asserts the
//! CompositeSolid's STRUCTURE (member/face/ring) AND its `semantics`
//! (`surfaces` + `values`) survive the round trip, that all 9 surfaces are
//! emitted (not dropped), and that the re-read package still carries semantics.

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
type Structure = BTreeMap<(String, u8), Vec<Vec<Vec<Ring>>>>;
type SemanticsMap = BTreeMap<(String, u8), Value>;

fn fixture() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/b1_lod2_cs_w_sem.gml")
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

/// The structural decomposition of every CompositeSolid, keyed by `(id, major)`.
fn composite_structure(pkg: &Path) -> Structure {
    let (manifest, meta) = open_meta(pkg);
    let mut map = Structure::new();
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
    let (manifest, meta) = open_meta(pkg);
    let mut map = SemanticsMap::new();
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

    convert(&ConvertOptions::new(fixture(), pkg.clone())).unwrap();
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
