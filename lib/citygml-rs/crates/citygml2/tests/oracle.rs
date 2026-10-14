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

/// The relative geometries `header` (a CityJSON header) declares, each
/// resolved to what it means, with no index left: its type and LoD, its
/// rings as millimetre coordinates, and per face the material and per ring
/// the texture image and its UVs for each theme.
fn templates(header: &Value) -> Vec<Value> {
    let mm = |v: &Value| -> Vec<i64> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|c| (c.as_f64().unwrap() * 1e3).round() as i64)
            .collect()
    };
    let vertices = header["geometry-templates"]["vertices-templates"]
        .as_array()
        .unwrap();
    let app = &header["appearance"];
    let uv = |i: &Value| -> Vec<i64> {
        app["vertices-texture"][i.as_u64().unwrap() as usize]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| (c.as_f64().unwrap() * 1e4).round() as i64)
            .collect()
    };
    let themes = |member: &str, t: &Value, f: &dyn Fn(&Value) -> Value| -> Value {
        let resolved: BTreeMap<String, Value> = t[member]
            .as_object()
            .into_iter()
            .flatten()
            .map(|(theme, r)| {
                // `value` is one value for every surface.
                let surfaces = t["boundaries"].as_array().unwrap().len();
                let values = match r.get("value") {
                    Some(v) => vec![f(v); surfaces],
                    None => r["values"].as_array().unwrap().iter().map(f).collect(),
                };
                (theme.clone(), Value::Array(values))
            })
            .collect();
        serde_json::to_value(resolved).unwrap()
    };
    header["geometry-templates"]["templates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            let rings: Vec<Vec<Vec<i64>>> = t["boundaries"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|surface| surface.as_array().unwrap())
                .map(|ring| {
                    ring.as_array()
                        .unwrap()
                        .iter()
                        .map(|i| mm(&vertices[i.as_u64().unwrap() as usize]))
                        .collect()
                })
                .collect();
            // A material by what it looks like: citygml-tools keeps one of
            // two materials that differ only in name, and its name.
            let material = |i: &Value| match i.as_u64() {
                Some(i) => {
                    let mut m = app["materials"][i as usize].clone();
                    m.as_object_mut().unwrap().remove("name");
                    m
                }
                None => Value::Null,
            };
            let texture = |surface: &Value| -> Value {
                surface
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|ring| match ring.as_array() {
                        Some(ring) if !ring.is_empty() && !ring[0].is_null() => {
                            let image =
                                &app["textures"][ring[0].as_u64().unwrap() as usize]["image"];
                            let uvs: Vec<Vec<i64>> = ring[1..].iter().map(uv).collect();
                            serde_json::json!([image, uvs])
                        }
                        _ => Value::Null,
                    })
                    .collect()
            };
            serde_json::json!({
                "type": t["type"],
                "lod": t["lod"],
                "rings": rings,
                "material": themes("material", t, &material),
                "texture": themes("texture", t, &texture),
            })
        })
        .collect()
}

/// Each City Object's GeometryInstances, by id: the template, the matrix and
/// the reference point in millimetres.
fn instances(features: &[Value]) -> BTreeMap<String, Vec<Value>> {
    let mut out = BTreeMap::new();
    for f in features {
        let t = &f["transform"];
        for (id, co) in f["CityObjects"].as_object().unwrap() {
            for g in co["geometry"].as_array().into_iter().flatten() {
                if g["type"] != "GeometryInstance" {
                    continue;
                }
                let v = &f["vertices"][g["boundaries"][0].as_u64().unwrap() as usize];
                let point: Vec<i64> = (0..3)
                    .map(|i| {
                        let real = v[i].as_f64().unwrap() * t["scale"][i].as_f64().unwrap()
                            + t["translate"][i].as_f64().unwrap();
                        (real * 1e3).round() as i64
                    })
                    .collect();
                out.entry(id.clone())
                    .or_insert_with(Vec::new)
                    .push(serde_json::json!({
                        "template": g["template"],
                        "transformationMatrix": g["transformationMatrix"],
                        "point": point,
                    }));
            }
        }
    }
    out
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
    let (geometry_templates, appearance) =
        citygml_core::convert::cjseq::geometry_templates(&citygml2::read_templates(&gml).unwrap())
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

    let text = std::fs::read_to_string(data("oracle/railway_trees_fragment.city.jsonl")).unwrap();
    let mut lines = text
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap());
    let oracle_header = lines.next().unwrap();
    let oracle_features: Vec<Value> = lines
        .map(|mut f| {
            f["transform"] = oracle_header["transform"].clone();
            f
        })
        .collect();

    assert_eq!(templates(&header), templates(&oracle_header));
    let instances_ours = instances(&our_features);
    assert_eq!(instances_ours.len(), 15);
    assert_eq!(instances_ours, instances(&oracle_features));
    assert_eq!(
        ours("railway_trees_fragment.gml"),
        oracle("oracle/railway_trees_fragment.city.jsonl")
    );
}
