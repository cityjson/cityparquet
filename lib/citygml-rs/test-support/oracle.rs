//! What the readers' oracle tests share: a CityJSONSeq reduced to what
//! two converters must agree on, and the geometry templates and instances
//! of a document resolved to what they mean. Each reader crate's tests
//! include it by path.

#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde_json::Value;

/// What both converters must agree on, counted over every feature.
#[derive(Debug, Default, PartialEq)]
pub struct Summary {
    pub features: usize,
    /// City Objects per type.
    pub types: BTreeMap<String, usize>,
    /// Geometries per (object type, geometry type, LoD).
    pub geometries: BTreeMap<(String, String, String), usize>,
    /// Semantic surfaces per (object type, surface type).
    pub surfaces: BTreeMap<(String, String), usize>,
    /// Attribute names and JSON types per semantic surface type. The `id`
    /// citygml-tools gives a surface from its `gml:id` is not compared.
    pub surface_attributes: BTreeMap<String, BTreeSet<(String, &'static str)>>,
    /// Attribute names and JSON types per object type.
    pub attributes: BTreeMap<String, BTreeSet<(String, &'static str)>>,
    /// City Object members other than the ones above (`address`), per
    /// (object type, member).
    pub members: BTreeMap<(String, String), usize>,
    /// Appearance palette entries (`materials`, `textures`,
    /// `vertices-texture`), summed over the features.
    pub palettes: BTreeMap<String, usize>,
    /// Geometries carrying a `material` or `texture`, per (member, type).
    pub appearance_on: BTreeMap<(String, String), usize>,
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
    pub fn add(&mut self, feature: &Value) {
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
                        // citygml-tools leaves an ADE's properties out unless
                        // it is given the ADE, so `+` names are not compared.
                        for (name, value) in v.as_object().unwrap() {
                            if !name.starts_with('+') {
                                set.insert((name.clone(), json_type(value)));
                            }
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
                    let surface = s["type"].as_str().unwrap().to_string();
                    let set = self.surface_attributes.entry(surface.clone()).or_default();
                    for (name, value) in s.as_object().unwrap() {
                        if !matches!(name.as_str(), "type" | "parent" | "children" | "id") {
                            set.insert((name.clone(), json_type(value)));
                        }
                    }
                    *self.surfaces.entry((ty.clone(), surface)).or_default() += 1;
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

/// The summary of every feature of the CityJSONSeq at `path`.
pub fn oracle(path: &Path) -> Summary {
    let mut summary = Summary::default();
    for feature in read_seq(path).1 {
        summary.add(&feature);
    }
    summary
}

/// The header and the features of the CityJSONSeq at `path`, each feature
/// given the header's transform.
pub fn read_seq(path: &Path) -> (Value, Vec<Value>) {
    let text = std::fs::read_to_string(path).unwrap();
    let mut lines = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str::<Value>(l).unwrap());
    let header = lines.next().unwrap();
    let features = lines
        .map(|mut f| {
            f["transform"] = header["transform"].clone();
            f
        })
        .collect();
    (header, features)
}

/// The relative geometries `header` (a CityJSON header) declares, each
/// resolved to what it means, with no index left: its type and LoD, its
/// rings as millimetre coordinates, and per face the material and per ring
/// the texture image and its UVs for each theme.
pub fn templates(header: &Value) -> Vec<Value> {
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
pub fn instances(features: &[Value]) -> BTreeMap<String, Vec<Value>> {
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
