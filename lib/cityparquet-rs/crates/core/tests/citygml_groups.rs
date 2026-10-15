//! A CityGML `CityObjectGroup` whose members it references by `xlink:href`
//! survives CityGML → package → CityJSONSeq with its membership.
//!
//! `railway_lod3_fragment.gml` ends with the "Forest KIT" group, which
//! references fourteen trees; one of them, `GMLID_SO015374_872_14131`, is a
//! member of the same document, read before the group. The tree stays a
//! feature of its own, as the spec's feature_id rule allows for a group
//! member, and still names the group among its parents.

use std::path::PathBuf;

use cityparquet::export::{ExportOptions, export};
use cityparquet::package::{ConvertOptions, convert};
use cityparquet::validate::validate_package;

const GROUP: &str = "UUID_f488e8ce-b953-4b35-a3fe-a394fb203868";
const TREE: &str = "GMLID_SO015374_872_14131";

fn data_fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(name)
}

#[test]
fn a_group_and_a_member_it_references_keep_their_link_in_separate_features() {
    let dir = tempfile::tempdir().unwrap();
    let pkg = dir.path().join("pkg");
    convert(&ConvertOptions::new(
        data_fixture("railway_lod3_fragment.gml"),
        pkg.clone(),
    ))
    .unwrap();
    let report = validate_package(&pkg).unwrap();
    assert!(report.is_conformant(), "{report}");

    let out = dir.path().join("out.city.jsonl");
    export(&ExportOptions::new(pkg, out.clone())).unwrap();
    let features: Vec<serde_json::Value> = std::fs::read_to_string(&out)
        .unwrap()
        .lines()
        .skip(1)
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let feature = |id: &str| {
        features
            .iter()
            .find(|f| f["id"] == id)
            .unwrap_or_else(|| panic!("no feature {id}"))
    };

    let group = &feature(GROUP)["CityObjects"][GROUP];
    let children = group["children"].as_array().unwrap();
    assert_eq!(children.len(), 14);
    assert!(children.iter().any(|c| c == TREE));

    let tree = &feature(TREE)["CityObjects"][TREE];
    assert_eq!(tree["parents"], serde_json::json!([GROUP]));
}
