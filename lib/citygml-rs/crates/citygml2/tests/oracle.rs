//! The reader against an oracle it did not write: `citygml-tools
//! to-cityjson` (citygml4j) on the same PLATEAU files, committed under
//! `tests/data/oracle`. Each file's features are reduced to what both
//! converters must agree on (objects per type, geometry types and LoDs,
//! semantic surfaces, attribute names and JSON types, addresses, appearance
//! palettes) and the two reductions compared whole.

#![cfg(feature = "cjseq")]

#[path = "../../../test-support/oracle.rs"]
mod support;

use std::path::PathBuf;

use citygml2::FeatureReader;
use serde_json::Value;
use support::{Summary, instances, read_seq, templates};

fn data(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(name)
}

fn ours(gml: &str) -> Summary {
    let transform = cjseq::Transform {
        scale: [1e-9, 1e-9, 1e-3],
        translate: [0.0; 3],
    };
    let mut summary = Summary::default();
    for feature in FeatureReader::open(&data(gml)).unwrap().features(transform) {
        summary.add(&serde_json::to_value(feature.unwrap()).unwrap());
    }
    summary
}

fn oracle(jsonl: &str) -> Summary {
    support::oracle(&data(jsonl))
}

fn agrees(name: &str) {
    assert_eq!(
        ours(&format!(
            "plateau/udx/{}/{name}.gml",
            name.split('_').nth(1).unwrap()
        )),
        oracle(&format!("oracle/{name}.city.jsonl")),
        "{name}"
    );
}

#[test]
fn a_building_file_agrees_with_citygml_tools() {
    agrees("53394519_bldg_6697_op");
}

#[test]
fn a_bridge_file_agrees_with_citygml_tools() {
    agrees("53394610_brid_6697_op");
}

#[test]
fn a_city_furniture_file_agrees_with_citygml_tools() {
    agrees("53394631_frn_6697_op");
}

#[test]
fn a_transportation_file_agrees_with_citygml_tools() {
    agrees("53394529_tran_6697_op");
}

#[test]
fn a_vegetation_file_agrees_with_citygml_tools() {
    agrees("53394600_veg_6697_op");
}

#[test]
fn a_water_body_file_agrees_with_citygml_tools() {
    agrees("533945_wtr_6697_op");
}

/// `tran:Track`, which CityJSON has no type for, is a `Road`.
#[test]
fn a_track_agrees_with_citygml_tools() {
    assert_eq!(
        ours("plateau_trk_fragment.gml"),
        oracle("oracle/plateau_trk_fragment.city.jsonl")
    );
}

/// Fifteen trees of the LoD 3 Railway instantiate three relative geometries
/// through `ImplicitGeometry`, the first use of each inline and the others by
/// `xlink:href`, coloured and textured by the CityModel's appearance. The
/// templates, the appearance they index, and every instance agree with
/// citygml-tools.
#[test]
fn implicit_geometries_agree_with_citygml_tools() {
    let gml = data("railway_trees_fragment.gml");
    let transform = cjseq::Transform {
        scale: [1e-3; 3],
        translate: [0.0; 3],
    };
    let document = citygml2::read_document(&gml).unwrap();
    let (geometry_templates, appearance) =
        citygml_core::convert::cjseq::geometry_templates(document.templates())
            .unwrap()
            .expect("the trees' relative geometries");
    let header = serde_json::json!({
        "geometry-templates": geometry_templates,
        "appearance": appearance,
    });
    let our_features: Vec<Value> = FeatureReader::open(&gml)
        .unwrap()
        .features(transform.clone())
        .map(|f| {
            let mut f = serde_json::to_value(f.unwrap()).unwrap();
            f["transform"] = serde_json::to_value(&transform).unwrap();
            f
        })
        .collect();

    let (oracle_header, oracle_features) =
        read_seq(&data("oracle/railway_trees_fragment.city.jsonl"));

    assert_eq!(templates(&header), templates(&oracle_header));
    let instances_ours = instances(&our_features);
    assert_eq!(instances_ours.len(), 15);
    assert_eq!(instances_ours, instances(&oracle_features));
    assert_eq!(
        ours("railway_trees_fragment.gml"),
        oracle("oracle/railway_trees_fragment.city.jsonl")
    );
}
