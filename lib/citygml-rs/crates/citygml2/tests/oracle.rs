//! The reader against an oracle it did not write: `citygml-tools
//! to-cityjson` (citygml4j) on the same PLATEAU files, committed under
//! `tests/data/oracle`. Each file's features are reduced to what both
//! converters must agree on (objects per type, geometry types and LoDs,
//! semantic surfaces, attribute names and JSON types, addresses, appearance
//! palettes) and the two reductions compared whole.

#![cfg(feature = "cjseq")]

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use citygml2::FeatureReader;
use serde_json::Value;

fn data(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(name)
}

/// What both converters must agree on, counted over every feature.
#[derive(Debug, Default, PartialEq)]
struct Summary {
    features: usize,
    /// City Objects per type.
    types: BTreeMap<String, usize>,
    /// Geometries per (object type, geometry type, LoD).
    geometries: BTreeMap<(String, String, String), usize>,
    /// Semantic surfaces per (object type, surface type).
    surfaces: BTreeMap<(String, String), usize>,
    /// Attribute names and JSON types per object type.
    attributes: BTreeMap<String, BTreeSet<(String, &'static str)>>,
    /// City Object members other than the ones above (`address`), per
    /// (object type, member).
    members: BTreeMap<(String, String), usize>,
    /// Appearance palette entries (`materials`, `textures`,
    /// `vertices-texture`), summed over the features.
    palettes: BTreeMap<String, usize>,
    /// Geometries carrying a `material` or `texture`, per (member, type).
    appearance_on: BTreeMap<(String, String), usize>,
}

fn json_type(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(n) if n.is_f64() => "float",
        Value::Number(_) => "integer",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

impl Summary {
    fn add(&mut self, feature: &Value) {
        if feature["type"] != "CityJSONFeature" {
            return;
        }
        self.features += 1;
        if let Some(app) = feature["appearance"].as_object() {
            for (k, v) in app {
                if let Some(list) = v.as_array() {
                    *self.palettes.entry(k.clone()).or_default() += list.len();
                }
            }
        }
        for co in feature["CityObjects"].as_object().unwrap().values() {
            let ty = co["type"].as_str().unwrap().to_string();
            *self.types.entry(ty.clone()).or_default() += 1;
            for (k, v) in co.as_object().unwrap() {
                match k.as_str() {
                    "type" | "geographicalExtent" | "parents" | "children" | "geometry" => {}
                    "attributes" => {
                        let set = self.attributes.entry(ty.clone()).or_default();
                        for (name, value) in v.as_object().unwrap() {
                            set.insert((name.clone(), json_type(value)));
                        }
                    }
                    _ => *self.members.entry((ty.clone(), k.clone())).or_default() += 1,
                }
            }
            for g in co["geometry"].as_array().into_iter().flatten() {
                let key = (
                    ty.clone(),
                    g["type"].as_str().unwrap().to_string(),
                    g["lod"].as_str().unwrap_or_default().to_string(),
                );
                *self.geometries.entry(key).or_default() += 1;
                for s in g["semantics"]["surfaces"].as_array().into_iter().flatten() {
                    let key = (ty.clone(), s["type"].as_str().unwrap().to_string());
                    *self.surfaces.entry(key).or_default() += 1;
                }
                for member in ["material", "texture"] {
                    if g.get(member).is_some() {
                        let key = (member.to_string(), ty.clone());
                        *self.appearance_on.entry(key).or_default() += 1;
                    }
                }
            }
        }
    }
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
    let text = std::fs::read_to_string(data(jsonl)).unwrap();
    let mut summary = Summary::default();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        summary.add(&serde_json::from_str(line).unwrap());
    }
    summary
}

fn agrees(name: &str) {
    assert_eq!(
        ours(&format!("plateau/{name}.gml")),
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
