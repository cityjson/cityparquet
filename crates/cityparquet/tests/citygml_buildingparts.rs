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
use std::fs::{self, File};
use std::path::{Path, PathBuf};

use cityparquet::citygml::writer::{WriteOptions, write_package};
use cityparquet::decode::decode_batch;
use cityparquet::package::{ConvertOptions, convert, convert_source};
use cityparquet::reader::CityParquetReaderBuilder;
use cityparquet::source::Source;
use cityparquet::stac::properties::PackageTables;
use cityparquet::wkb_read::DecodedKind;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

fn data_fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(name)
}

/// `building_with_parts.gml` (a committed, hand-authored CityGML fixture)
/// carries no `srsName`/envelope at all. Since `scan` now hard-fails on
/// coordinate-bearing input with no resolvable CRS (spec "CRS rules"), the
/// FIRST convert of the raw fixture injects one onto the REAL parsed header
/// — the writer (`crate::citygml::writer`) then emits `srsName` into the
/// round-tripped `.gml`'s own envelope, so the SECOND convert below resolves
/// a CRS from that file natively, without needing a second injection.
fn convert_with_crs(input: PathBuf, out: PathBuf) {
    let raw = Source::open(&input).unwrap();
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
    let src = Source::from_parts(header, features, raw.doc_appearance().cloned(), raw.format());
    convert_source(&src, &ConvertOptions::new(input, out)).unwrap();
}

fn workspace_fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

fn mm(v: f64) -> i64 {
    (v * 1000.0).round() as i64
}

type Ring = Vec<(i64, i64, i64)>;

fn canonical_ring(ring: &[usize], coords: &[[f64; 3]]) -> Ring {
    let t: Ring = ring
        .iter()
        .map(|&i| (mm(coords[i][0]), mm(coords[i][1]), mm(coords[i][2])))
        .collect();
    let Some(m) = (0..t.len()).min_by_key(|&i| t[i]) else {
        return t;
    };
    let mut r = t[m..].to_vec();
    r.extend_from_slice(&t[..m]);
    r
}

fn faces_struct(faces: &[Vec<Vec<usize>>], coords: &[[f64; 3]]) -> Vec<Vec<Ring>> {
    faces
        .iter()
        .map(|f| f.iter().map(|r| canonical_ring(r, coords)).collect())
        .collect()
}

/// One geometry's fingerprint: kind, lod, semantics JSON, and its structural
/// decomposition (member → face → canonical ring) — catches ring/topology
/// corruption, not just a coordinate set.
type GeomInfo = (String, Option<String>, String, Vec<Vec<Vec<Ring>>>);

/// A stable per-CityObject fingerprint: type, parents, `children` in DOCUMENT
/// order (order matters for round-trip), attributes, and geometries.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ObjInfo {
    thetype: String,
    parents: Vec<String>,
    children: Vec<String>,
    attributes: Vec<(String, String)>,
    geoms: Vec<GeomInfo>,
}

fn objects(pkg: &Path) -> BTreeMap<String, ObjInfo> {
    let tables = PackageTables::open(pkg).unwrap();
    let meta = ParquetRecordBatchReaderBuilder::try_new(File::open(&tables.tables[0]).unwrap())
        .unwrap()
        .cityparquet_metadata()
        .unwrap();

    let mut map = BTreeMap::new();
    for path in &tables.tables {
        let reader = ParquetRecordBatchReaderBuilder::try_new(File::open(path).unwrap())
            .unwrap()
            .build()
            .unwrap();
        for batch in reader {
            let batch = batch.unwrap();
            for obj in decode_batch(&batch, &meta).unwrap() {
                let parents: Vec<String> = obj.object.parents.clone().unwrap_or_default();
                let children: Vec<String> = obj.object.children.clone().unwrap_or_default();
                let attributes: Vec<(String, String)> = obj
                    .object
                    .attributes
                    .as_ref()
                    .and_then(serde_json::Value::as_object)
                    .map(|m| {
                        let mut v: Vec<(String, String)> = m
                            .iter()
                            .map(|(k, val)| (k.clone(), val.to_string()))
                            .collect();
                        v.sort();
                        v
                    })
                    .unwrap_or_default();
                let mut geoms = Vec::new();
                for (lod, decoded, props) in &obj.geometries {
                    let (kind, structure): (&str, Vec<Vec<Vec<Ring>>>) = match &decoded.kind {
                        DecodedKind::PolyhedralSurface(faces) => (
                            "PolyhedralSurface",
                            vec![faces_struct(faces, &decoded.coords)],
                        ),
                        DecodedKind::GeometryCollection(members) => (
                            "GeometryCollection",
                            members
                                .iter()
                                .map(|m| match m {
                                    DecodedKind::PolyhedralSurface(f) => {
                                        faces_struct(f, &decoded.coords)
                                    }
                                    _ => Vec::new(),
                                })
                                .collect(),
                        ),
                        DecodedKind::MultiPolygon(faces) => {
                            ("MultiPolygon", vec![faces_struct(faces, &decoded.coords)])
                        }
                        _ => ("other", Vec::new()),
                    };
                    let sem = props
                        .as_ref()
                        .and_then(|p| p.get("semantics"))
                        .map(|s| s.to_string())
                        .unwrap_or_default();
                    geoms.push((
                        kind.to_string(),
                        lod.as_ref().map(|l| l.to_string()),
                        sem,
                        structure,
                    ));
                }
                geoms.sort();
                map.insert(
                    obj.id.clone(),
                    ObjInfo {
                        thetype: obj.object.thetype.clone(),
                        parents,
                        children,
                        attributes,
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

    convert_with_crs(data_fixture("building_with_parts.gml"), pkg.clone());
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
