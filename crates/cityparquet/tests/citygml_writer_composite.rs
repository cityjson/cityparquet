//! W-M2 CompositeSolid round-trip oracle over a real fixture.
//!
//! `b1_lod2_cs_w_sem.gml` (one Building whose `bldg:lod2Solid` is a 2-member
//! `gml:CompositeSolid`) is converted to a package, written back to `.gml`, and
//! re-converted. Equal stored geometry across the round trip, keyed by
//! `(building_id, major_lod)`, proves the CompositeSolid survived.

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

type CoordsByBuildingLod = BTreeMap<(String, u8), BTreeSet<(i64, i64, i64)>>;

fn fixture() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/b1_lod2_cs_w_sem.gml")
}

fn mm(v: f64) -> i64 {
    (v * 1000.0).round() as i64
}

/// Per Building `id`, per major LoD, the distinct world coords of its
/// CompositeSolid (`GeometryCollection` of `PolyhedralSurface`) on the 1 mm grid.
fn composite_coords(pkg: &Path) -> CoordsByBuildingLod {
    let manifest: PackageManifest =
        serde_json::from_str(&fs::read_to_string(pkg.join("metadata.json")).unwrap()).unwrap();
    let meta = ParquetRecordBatchReaderBuilder::try_new(
        fs::File::open(pkg.join(&manifest.tables[0])).unwrap(),
    )
    .unwrap()
    .cityparquet_metadata()
    .unwrap();

    let mut map = CoordsByBuildingLod::new();
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
                    if matches!(decoded.kind, DecodedKind::GeometryCollection(_)) {
                        let Some(major) = lod
                            .as_ref()
                            .map(|l| l.major())
                            .filter(|m| (1..=4).contains(m))
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

    convert(&ConvertOptions::new(out_gml.clone(), pkg2.clone())).unwrap();

    let before = composite_coords(&pkg);
    let after = composite_coords(&pkg2);
    assert!(
        !before.is_empty(),
        "the original package must have a CompositeSolid"
    );
    assert_eq!(
        before, after,
        "CompositeSolid coordinates must survive the round trip"
    );
}
