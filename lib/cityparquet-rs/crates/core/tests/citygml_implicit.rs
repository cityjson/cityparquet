//! A CityGML `ImplicitGeometry` survives CityGML → package → CityJSONSeq.
//!
//! In `railway_trees_fragment.gml` fifteen trees instantiate three relative
//! geometries, each written inline at its first use and referenced by
//! `xlink:href` after, and coloured and textured by the CityModel's
//! appearance. The package holds them as implicit geometries, and the export
//! carries them as `geometry-templates` with the appearance they index.

use std::path::PathBuf;

use cityparquet::compare::{CompareOptions, compare_datasets};
use cityparquet::export::{ExportOptions, export};
use cityparquet::package::{ConvertOptions, convert};
use cityparquet::validate::validate_package;

fn data_fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(name)
}

#[test]
fn implicit_geometries_round_trip_through_a_package() {
    let source = data_fixture("railway_trees_fragment.gml");
    let dir = tempfile::tempdir().unwrap();
    let pkg = dir.path().join("pkg");
    convert(&ConvertOptions::new(source.clone(), pkg.clone())).unwrap();
    let report = validate_package(&pkg).unwrap();
    assert!(report.is_conformant(), "{report}");

    let out = dir.path().join("out.city.jsonl");
    export(&ExportOptions::new(pkg, out.clone())).unwrap();
    let text = std::fs::read_to_string(&out).unwrap();
    let mut lines = text
        .lines()
        .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap());
    let header = lines.next().unwrap();
    let templates = &header["geometry-templates"]["templates"];
    assert_eq!(templates.as_array().map(Vec::len), Some(3));
    assert!(header["appearance"]["textures"].is_array());
    let instances = lines
        .flat_map(|f| f["CityObjects"].as_object().unwrap().clone())
        .flat_map(|(_, co)| co["geometry"].as_array().cloned().unwrap_or_default())
        .filter(|g| g["type"] == "GeometryInstance")
        .count();
    assert_eq!(instances, 15);

    let cmp = compare_datasets(&source, &out, &CompareOptions::default()).unwrap();
    assert!(cmp.equal, "{:#?}", cmp.differences);
}
