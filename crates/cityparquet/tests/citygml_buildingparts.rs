//! W-M4 BuildingParts round-trip oracles.
//!
//! (1) Hand-authored `building_with_parts.gml` (a Building with its own
//! lod1Solid + two BuildingParts, one a lod2Solid, one a boundedBy-only
//! MultiSurface) → package → `.gml` → package: assert every CityObject's
//! type / parents / children / geometry+semantics survive, keyed by id, and
//! that `consistsOfBuildingPart` follows the last solid/boundedBy in the parent.
//! (2) Real 3DBAG `delft.city.jsonl` (geometry-less parents + geometry-bearing
//! parts) → package → `.gml` → package: the independent-authoring anchor.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use cityparquet::citygml::writer::{WriteOptions, write_package};
use cityparquet::decode::decode_batch;
use cityparquet::package::{ConvertOptions, convert};
use cityparquet::reader::CityParquetReaderBuilder;
use cityparquet::schema::PackageManifest;
use cityparquet::wkb_read::DecodedKind;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

fn data_fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(name)
}

fn workspace_fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

fn mm(v: f64) -> i64 {
    (v * 1000.0).round() as i64
}

/// One geometry's fingerprint: (kind, lod, semantics JSON, canonical coord set).
type GeomInfo = (String, Option<String>, String, Vec<(i64, i64, i64)>);

/// A stable per-CityObject fingerprint: type, sorted parents/children, geometries.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ObjInfo {
    thetype: String,
    parents: Vec<String>,
    children: Vec<String>,
    geoms: Vec<GeomInfo>,
}

fn objects(pkg: &Path) -> BTreeMap<String, ObjInfo> {
    let manifest: PackageManifest =
        serde_json::from_str(&fs::read_to_string(pkg.join("metadata.json")).unwrap()).unwrap();
    let meta = ParquetRecordBatchReaderBuilder::try_new(
        fs::File::open(pkg.join(&manifest.tables[0])).unwrap(),
    )
    .unwrap()
    .cityparquet_metadata()
    .unwrap();

    let mut map = BTreeMap::new();
    for name in &manifest.tables {
        let reader =
            ParquetRecordBatchReaderBuilder::try_new(fs::File::open(pkg.join(name)).unwrap())
                .unwrap()
                .build()
                .unwrap();
        for batch in reader {
            let batch = batch.unwrap();
            for obj in decode_batch(&batch, &meta).unwrap() {
                let mut parents: Vec<String> = obj.object.parents.clone().unwrap_or_default();
                parents.sort();
                let mut children: Vec<String> = obj.object.children.clone().unwrap_or_default();
                children.sort();
                let mut geoms = Vec::new();
                for (lod, decoded, props) in &obj.geometries {
                    let kind = match decoded.kind {
                        DecodedKind::PolyhedralSurface(_) => "PolyhedralSurface",
                        DecodedKind::GeometryCollection(_) => "GeometryCollection",
                        DecodedKind::MultiPolygon(_) => "MultiPolygon",
                        _ => "other",
                    };
                    let sem = props
                        .as_ref()
                        .and_then(|p| p.get("semantics"))
                        .map(|s| s.to_string())
                        .unwrap_or_default();
                    let mut coords: Vec<(i64, i64, i64)> = decoded
                        .coords
                        .iter()
                        .map(|c| (mm(c[0]), mm(c[1]), mm(c[2])))
                        .collect();
                    coords.sort();
                    geoms.push((
                        kind.to_string(),
                        lod.as_ref().map(|l| l.to_string()),
                        sem,
                        coords,
                    ));
                }
                geoms.sort();
                map.insert(
                    obj.id.clone(),
                    ObjInfo {
                        thetype: obj.object.thetype.clone(),
                        parents,
                        children,
                        geoms,
                    },
                );
            }
        }
    }
    map
}

#[test]
fn hand_fixture_building_parts_round_trip() {
    let tmp = tempfile::tempdir().unwrap();
    let pkg = tmp.path().join("pkg");
    let out_gml = tmp.path().join("out.gml");
    let pkg2 = tmp.path().join("pkg2");

    convert(&ConvertOptions::new(
        data_fixture("building_with_parts.gml"),
        pkg.clone(),
    ))
    .unwrap();
    let report = write_package(&WriteOptions {
        package_dir: pkg.clone(),
        output: out_gml.clone(),
    })
    .unwrap();
    assert_eq!(report.buildings_written, 1);
    assert_eq!(report.building_parts_written, 2, "B_p1 + B_p2");
    assert_eq!(report.building_parts_skipped, 0);
    assert_eq!(report.building_parts_orphaned, 0);
    assert_eq!(report.children_unresolved, 0);

    // Element order: consistsOfBuildingPart after the parent's last solid/boundedBy.
    let gml = fs::read_to_string(&out_gml).unwrap();
    let first_cbp = gml.find("consistsOfBuildingPart").unwrap();
    let last_solid = gml.rfind("<bldg:lod1Solid>").unwrap();
    assert!(
        last_solid < first_cbp,
        "consistsOfBuildingPart follows the parent's own solid"
    );

    convert(&ConvertOptions::new(out_gml.clone(), pkg2.clone())).unwrap();

    let before = objects(&pkg);
    let after = objects(&pkg2);
    assert_eq!(before.keys().collect::<Vec<_>>(), vec!["B", "B_p1", "B_p2"]);
    assert_eq!(
        before, after,
        "every CityObject (type/parents/children/geometry) survives"
    );
}

/// The BuildingPart LINK structure only (type + sorted parents/children). delft
/// geometry does not fully round-trip for reasons OUTSIDE W-M4: its parts carry
/// minor LoDs (`lod1.2` + `lod1.3` collapse to one CityGML `lod1Solid`, W-M1)
/// and its parents carry a lod0 `MultiSurface` without semantics (skipped,
/// W-M2/M3). W-M4's contribution — the parent/part nesting — is what this asserts.
fn links(pkg: &Path) -> BTreeMap<String, (String, Vec<String>, Vec<String>)> {
    objects(pkg)
        .into_iter()
        .map(|(id, o)| (id, (o.thetype, o.parents, o.children)))
        .collect()
}

#[test]
fn delft_building_parts_round_trip() {
    let tmp = tempfile::tempdir().unwrap();
    let pkg = tmp.path().join("pkg");
    let out_gml = tmp.path().join("out.gml");
    let pkg2 = tmp.path().join("pkg2");

    convert(&ConvertOptions::new(
        workspace_fixture("delft.city.jsonl"),
        pkg.clone(),
    ))
    .unwrap();
    let before = objects(&pkg);
    let n_parts = before
        .values()
        .filter(|o| o.thetype == "BuildingPart")
        .count();
    let n_buildings = before.values().filter(|o| o.thetype == "Building").count();
    assert!(
        n_parts > 0 && n_buildings > 0,
        "delft has Buildings + BuildingParts"
    );

    let report = write_package(&WriteOptions {
        package_dir: pkg.clone(),
        output: out_gml.clone(),
    })
    .unwrap();
    assert_eq!(
        report.building_parts_written, n_parts,
        "every delft part emitted"
    );
    assert_eq!(
        report.non_building_skipped, 0,
        "delft has only Building/BuildingPart"
    );
    assert_eq!(report.building_parts_orphaned, 0);
    assert_eq!(report.children_unresolved, 0);

    convert(&ConvertOptions::new(out_gml.clone(), pkg2.clone())).unwrap();
    // The parent/part nesting (type + parents + children) survives for every object.
    assert_eq!(
        links(&pkg),
        links(&pkg2),
        "delft Building/BuildingPart links survive"
    );
    // Every re-read part still carries geometry (not emitted as an empty husk).
    for (id, o) in &objects(&pkg2) {
        if o.thetype == "BuildingPart" {
            assert!(!o.geoms.is_empty(), "re-read part {id} must carry geometry");
        }
    }
}
