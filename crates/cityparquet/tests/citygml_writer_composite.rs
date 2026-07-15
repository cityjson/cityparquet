//! W-M2 CompositeSolid round-trip oracle over a real fixture.
//!
//! `b1_lod2_cs_w_sem.gml` (one Building whose `bldg:lod2Solid` is a 2-member
//! `gml:CompositeSolid`, plus `boundedBy` semantic surfaces) is converted to a
//! package, written back to `.gml`, and re-converted. The oracle asserts:
//!   1. the CompositeSolid's STRUCTURE (member partition, per-member faces,
//!      per-face rings — not just a coordinate set) survives the round trip; and
//!   2. the semantic surfaces are dropped, reported by
//!      `WriteReport::semantic_surfaces_dropped`, and absent from the re-read
//!      package (W-M2 emits geometry only; `bldg:boundedBy` is W-M3). Asserting
//!      the drop explicitly means a regression or half-landed W-M3 cannot pass
//!      silently.

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

/// A ring is a sequence of mm-grid coord tuples; a face is its rings; a member
/// is its faces; a geometry is its members. Keyed by `(building_id, major_lod)`.
type Ring = Vec<(i64, i64, i64)>;
type Structure = BTreeMap<(String, u8), Vec<Vec<Vec<Ring>>>>;

fn fixture() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/b1_lod2_cs_w_sem.gml")
}

fn mm(v: f64) -> i64 {
    (v * 1000.0).round() as i64
}

/// A ring rotated to start at its lexicographically-smallest vertex, so the
/// (arbitrary) start vertex of a closed ring does not spuriously differ across
/// the round trip while genuine vertex-order changes still do.
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

/// The structural decomposition of every CompositeSolid (`GeometryCollection`)
/// in a package, keyed by `(building_id, major_lod)`.
fn composite_structure(pkg: &Path) -> Structure {
    let manifest: PackageManifest =
        serde_json::from_str(&fs::read_to_string(pkg.join("metadata.json")).unwrap()).unwrap();
    let meta = ParquetRecordBatchReaderBuilder::try_new(
        fs::File::open(pkg.join(&manifest.tables[0])).unwrap(),
    )
    .unwrap()
    .cityparquet_metadata()
    .unwrap();

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

/// Whether any Building geometry in the package carries
/// `geometry_properties.semantics` — used to prove the writer dropped it.
fn any_semantics(pkg: &Path) -> bool {
    let manifest: PackageManifest =
        serde_json::from_str(&fs::read_to_string(pkg.join("metadata.json")).unwrap()).unwrap();
    let meta = ParquetRecordBatchReaderBuilder::try_new(
        fs::File::open(pkg.join(&manifest.tables[0])).unwrap(),
    )
    .unwrap()
    .cityparquet_metadata()
    .unwrap();

    for name in &manifest.tables {
        let reader =
            ParquetRecordBatchReaderBuilder::try_new(fs::File::open(pkg.join(name)).unwrap())
                .unwrap()
                .build()
                .unwrap();
        for batch in reader {
            let batch = batch.unwrap();
            for obj in decode_batch(&batch, &meta).unwrap() {
                for (_lod, _decoded, props) in &obj.geometries {
                    if props.as_ref().is_some_and(|p| p.get("semantics").is_some()) {
                        return true;
                    }
                }
            }
        }
    }
    false
}

#[test]
fn b1_composite_solid_round_trips_gml_to_parquet_to_gml() {
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
    assert_eq!(
        report.composite_solids_written, 1,
        "one CompositeSolid expected"
    );
    assert_eq!(report.multi_solids_skipped, 0);

    // The b1 fixture has 9 boundedBy semantic surfaces (1 Ground, 4 Roof, 4
    // Wall). W-M2 drops them (geometry only) but must REPORT the drop.
    assert_eq!(
        report.semantic_surfaces_dropped, 9,
        "all 9 b1 semantic surfaces must be counted as dropped"
    );

    convert(&ConvertOptions::new(out_gml.clone(), pkg2.clone())).unwrap();

    // Geometry structure (members/faces/rings) must be identical; a coord set
    // alone would miss member re-partitioning, face loss, or ring changes.
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

    // The original package carried semantics; the re-read one must not (the
    // drop is real, and this pins the W-M3 flip point so it can't regress).
    assert!(
        any_semantics(&pkg),
        "the original package must carry semantics"
    );
    assert!(
        !any_semantics(&pkg2),
        "the writer must have dropped semantics (W-M3 feature)"
    );
}
