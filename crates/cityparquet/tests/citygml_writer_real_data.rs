//! W-M1 round-trip oracle for the CityGML 2.0 writer, over a real fixture.
//!
//! `savenow_ingolstadt_lod2.gml` (3 `bldg:Building` with `bldg:lod2Solid`) is
//! converted to a CityParquet package, written back out to `.gml` by the
//! writer, and re-converted. That the second `convert` succeeds proves the
//! writer's output is parseable by the existing reader; that the two packages'
//! stored Building **solid coordinates** match proves the geometry survived
//! the package -> CityGML -> package round-trip. Attributes and semantic
//! surfaces are intentionally NOT compared — W-M1 emits geometry only.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use cityparquet::citygml::writer::{WriteOptions, write_package};
use cityparquet::decode::decode_batch;
use cityparquet::package::{ConvertOptions, convert};
use cityparquet::reader::CityParquetReaderBuilder;
use cityparquet::schema::PackageManifest;
use cityparquet::wkb_read::DecodedKind;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

fn fixture() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/savenow_ingolstadt_lod2.gml")
}

/// 1 mm grid: both packages come from `convert`'s 1 mm quantiser and the
/// writer preserves world coordinates, so this rounding is exact and makes the
/// comparison independent of float formatting.
fn mm(v: f64) -> i64 {
    (v * 1000.0).round() as i64
}

/// Per Building `id`, per major LoD, the set of distinct world coordinates of
/// its `Solid` (`PolyhedralSurface`) geometry, on the 1 mm grid. Keying by
/// `(id, major)` — not one global set — means a swapped building, a lost LoD,
/// or geometry attached to the wrong object is caught, not masked.
type CoordsByBuildingLod = BTreeMap<(String, u8), BTreeSet<(i64, i64, i64)>>;

fn solid_coords(pkg: &Path) -> CoordsByBuildingLod {
    let manifest: PackageManifest =
        serde_json::from_str(&fs::read_to_string(pkg.join("metadata.json")).unwrap()).unwrap();
    let meta = ParquetRecordBatchReaderBuilder::try_new(
        fs::File::open(pkg.join(&manifest.tables[0])).unwrap(),
    )
    .unwrap()
    .cityparquet_metadata()
    .unwrap();

    let mut map: CoordsByBuildingLod = BTreeMap::new();
    for name in &manifest.tables {
        let reader = ParquetRecordBatchReaderBuilder::try_new(fs::File::open(pkg.join(name)).unwrap())
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
                    if matches!(decoded.kind, DecodedKind::PolyhedralSurface(_)) {
                        // W-M1 only emits majors 1..=4; mirror that here so the
                        // two sides compare the same projection.
                        let Some(major) = lod.as_ref().map(|l| l.major()).filter(|m| (1..=4).contains(m))
                        else {
                            continue;
                        };
                        let entry = map.entry((obj.id.clone(), major)).or_default();
                        for c in &decoded.coords {
                            entry.insert((mm(c[0]), mm(c[1]), mm(c[2])));
                        }
                    }
                }
            }
        }
    }
    map
}

type AttrsByBuilding = BTreeMap<String, serde_json::Map<String, serde_json::Value>>;

/// Per Building `id`, its decoded attribute map, for round-trip comparison.
fn building_attributes(pkg: &Path) -> AttrsByBuilding {
    let manifest: PackageManifest =
        serde_json::from_str(&fs::read_to_string(pkg.join("metadata.json")).unwrap()).unwrap();
    let meta = ParquetRecordBatchReaderBuilder::try_new(
        fs::File::open(pkg.join(&manifest.tables[0])).unwrap(),
    )
    .unwrap()
    .cityparquet_metadata()
    .unwrap();

    let mut map = AttrsByBuilding::new();
    for name in &manifest.tables {
        let reader = ParquetRecordBatchReaderBuilder::try_new(fs::File::open(pkg.join(name)).unwrap())
            .unwrap()
            .build()
            .unwrap();
        for batch in reader {
            let batch = batch.unwrap();
            for obj in decode_batch(&batch, &meta).unwrap() {
                if obj.object.thetype != "Building" {
                    continue;
                }
                let attrs = obj
                    .object
                    .attributes
                    .as_ref()
                    .and_then(serde_json::Value::as_object)
                    .cloned()
                    .unwrap_or_default();
                map.insert(obj.id.clone(), attrs);
            }
        }
    }
    map
}

#[test]
fn ingolstadt_lod2_solids_round_trip_gml_to_parquet_to_gml() {
    let tmp = tempfile::tempdir().unwrap();
    let pkg = tmp.path().join("pkg");
    let out_gml = tmp.path().join("out.gml");
    let pkg2 = tmp.path().join("pkg2");

    // 1. original .gml -> CityParquet package.
    convert(&ConvertOptions::new(fixture(), pkg.clone())).unwrap();

    // 2. package -> .gml.
    let report =
        write_package(&WriteOptions { package_dir: pkg.clone(), output: out_gml.clone() }).unwrap();
    assert_eq!(report.buildings_written, 3, "3 Buildings with lod2Solid expected");

    // 3. the writer's output must be reader-parseable: re-convert it.
    //    (A convert failure here would mean the emitted CityGML is malformed.)
    convert(&ConvertOptions::new(out_gml.clone(), pkg2.clone())).unwrap();

    // 4. the stored Building solid geometry must be identical across the
    //    round trip (geometry projection — not attributes/semantics).
    let before = solid_coords(&pkg);
    let after = solid_coords(&pkg2);
    assert!(!before.is_empty(), "the original package must have solid geometry");
    assert_eq!(before, after, "Building solid coordinates must survive the round trip");

    // 5. Attributes must survive the round trip too (measuredHeight/roofType/
    //    storeysAboveGround + gen: string attributes on this fixture).
    let attrs_before = building_attributes(&pkg);
    let attrs_after = building_attributes(&pkg2);
    assert!(
        attrs_before.values().any(|m| !m.is_empty()),
        "the original package must carry building attributes"
    );
    assert_eq!(attrs_before, attrs_after, "Building attributes must survive the round trip");
    assert_eq!(report.attributes_skipped, 0, "Ingolstadt attributes are all representable");
}
