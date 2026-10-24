//! A CityJSON feature reduced to what it says, so that two features
//! holding the same model compare equal however their palettes are
//! ordered: every vertex, semantic surface, material, texture and
//! texture-coordinate index replaced by what it points at, the palettes
//! dropped, a material given for every surface as one `value` spelled out
//! per surface, empty `attributes` dropped, and every object's keys
//! sorted. Each writer test includes it by path.

#![allow(dead_code)]

use serde_json::{Map, Value};

/// `feature` (a CityJSONFeature as JSON) with its indices resolved.
pub fn canonical(feature: &Value) -> Value {
    let vertices = feature["vertices"].as_array().cloned().unwrap_or_default();
    let appearance = &feature["appearance"];
    let palettes = Palettes {
        vertices: &vertices,
        materials: appearance["materials"].as_array(),
        textures: appearance["textures"].as_array(),
        uvs: appearance["vertices-texture"].as_array(),
    };
    let mut out = Map::new();
    out.insert("id".into(), feature["id"].clone());
    let objects = feature["CityObjects"]
        .as_object()
        .map(|objects| {
            objects
                .iter()
                .map(|(id, co)| (id.clone(), palettes.object(co)))
                .collect::<Map<_, _>>()
        })
        .unwrap_or_default();
    out.insert("CityObjects".into(), Value::Object(objects));
    sorted(Value::Object(out))
}

/// A document's geometry templates with their indices resolved against
/// the template vertices and the header appearance.
pub fn canonical_templates(templates: &Value, appearance: &Value) -> Value {
    let vertices = templates["vertices-templates"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let palettes = Palettes {
        vertices: &vertices,
        materials: appearance["materials"].as_array(),
        textures: appearance["textures"].as_array(),
        uvs: appearance["vertices-texture"].as_array(),
    };
    sorted(Value::Array(
        templates["templates"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|g| palettes.geometry(g))
            .collect(),
    ))
}

struct Palettes<'a> {
    vertices: &'a [Value],
    materials: Option<&'a Vec<Value>>,
    textures: Option<&'a Vec<Value>>,
    uvs: Option<&'a Vec<Value>>,
}

impl Palettes<'_> {
    fn object(&self, co: &Value) -> Value {
        let mut co = co.clone();
        if co["attributes"].as_object().is_some_and(|a| a.is_empty()) {
            co.as_object_mut().unwrap().remove("attributes");
        }
        if let Some(geometries) = co.get_mut("geometry").and_then(Value::as_array_mut) {
            for g in geometries.iter_mut() {
                *g = self.geometry(g);
            }
        }
        if let Some(addresses) = co.get_mut("address").and_then(Value::as_array_mut) {
            for a in addresses.iter_mut() {
                if let Some(location) = a.get("location") {
                    let location = self.geometry(location);
                    a["location"] = location;
                }
            }
        }
        co
    }

    fn geometry(&self, g: &Value) -> Value {
        let mut g = g.clone();
        if let Some(semantics) = g.get("semantics") {
            let surfaces = semantics["surfaces"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            g["semantics"] = resolve(&semantics["values"], &|i| surface(&surfaces, i, 0));
        }
        let per_surface = g.get("boundaries").map(|b| shape(&g["type"], b));
        if let Some(b) = g.get("boundaries") {
            g["boundaries"] = self.vertices_of(b);
        }
        if let Some(themes) = g.get_mut("material").and_then(Value::as_object_mut) {
            for m in themes.values_mut() {
                if let (Some(v), Some(per_surface)) = (m.get("value").cloned(), &per_surface) {
                    m["values"] = resolve(per_surface, &|_| v.clone());
                    m.as_object_mut().unwrap().remove("value");
                }
                if let Some(v) = m.get("values") {
                    m["values"] = self.materials_of(v);
                }
            }
        }
        if let Some(themes) = g.get_mut("texture").and_then(Value::as_object_mut) {
            for t in themes.values_mut() {
                if let Some(v) = t.get("values") {
                    t["values"] = self.textures_of(v);
                }
            }
        }
        g
    }

    fn vertices_of(&self, v: &Value) -> Value {
        match v {
            Value::Array(items) => {
                Value::Array(items.iter().map(|i| self.vertices_of(i)).collect())
            }
            Value::Number(n) => self.vertices[n.as_u64().unwrap() as usize].clone(),
            other => other.clone(),
        }
    }

    fn material(&self, i: &Value) -> Value {
        match i.as_u64() {
            Some(i) => self.materials.unwrap()[i as usize].clone(),
            None => i.clone(),
        }
    }

    fn materials_of(&self, v: &Value) -> Value {
        match v {
            Value::Array(items) => {
                Value::Array(items.iter().map(|i| self.materials_of(i)).collect())
            }
            other => self.material(other),
        }
    }

    /// Texture values: arrays down to a ring, `[texture, uv, …]` or
    /// `[null]`, whose first entry is a texture and the rest UVs.
    fn textures_of(&self, v: &Value) -> Value {
        let Value::Array(items) = v else {
            return v.clone();
        };
        let is_ring = items.first().is_some_and(|f| f.is_null() || f.is_number());
        if !is_ring {
            return Value::Array(items.iter().map(|i| self.textures_of(i)).collect());
        }
        let mut ring = items.iter();
        let mut out = Vec::new();
        match ring.next() {
            Some(Value::Number(t)) => {
                out.push(self.textures.unwrap()[t.as_u64().unwrap() as usize].clone());
                out.extend(ring.map(|uv| self.uvs.unwrap()[uv.as_u64().unwrap() as usize].clone()));
            }
            _ => out.push(Value::Null),
        }
        Value::Array(out)
    }
}

/// `values` (nested arrays of indices or nulls) with each index `i`
/// replaced by `f(i)`.
fn resolve(values: &Value, f: &dyn Fn(usize) -> Value) -> Value {
    match values {
        Value::Array(items) => Value::Array(items.iter().map(|v| resolve(v, f)).collect()),
        Value::Number(n) => f(n.as_u64().unwrap() as usize),
        other => other.clone(),
    }
}

/// Semantic surface `i` of `surfaces` with its parent resolved in place of
/// its index, and without its children, which the faces say.
fn surface(surfaces: &[Value], i: usize, depth: usize) -> Value {
    let mut s = surfaces[i].clone();
    let map = s.as_object_mut().unwrap();
    map.remove("children");
    if let Some(p) = map.remove("parent").and_then(|p| p.as_u64())
        && depth < 8
    {
        map.insert("parent".into(), surface(surfaces, p as usize, depth + 1));
    }
    s
}

/// One `0` per surface of a geometry of type `ty` with `boundaries`,
/// nested as its semantic or material values are.
fn shape(ty: &Value, boundaries: &Value) -> Value {
    let zeros = |v: &Value| {
        Value::Array(
            v.as_array()
                .unwrap()
                .iter()
                .map(|_| Value::from(0))
                .collect(),
        )
    };
    let arr = |v: &Value, f: &dyn Fn(&Value) -> Value| {
        Value::Array(v.as_array().unwrap().iter().map(f).collect())
    };
    match ty.as_str() {
        Some("MultiSurface" | "CompositeSurface") => zeros(boundaries),
        Some("Solid") => arr(boundaries, &zeros),
        Some("MultiSolid" | "CompositeSolid") => arr(boundaries, &|solid| arr(solid, &zeros)),
        _ => Value::Null,
    }
}

/// `v` with every object's keys in sorted order.
pub fn sorted(v: Value) -> Value {
    match v {
        Value::Object(map) => {
            let mut entries: Vec<(String, Value)> = map.into_iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            Value::Object(entries.into_iter().map(|(k, v)| (k, sorted(v))).collect())
        }
        Value::Array(items) => Value::Array(items.into_iter().map(sorted).collect()),
        other => other,
    }
}

/// Where `a` and `b` differ, as JSON paths with both values, at most
/// `limit` of them.
pub fn diff(a: &Value, b: &Value, limit: usize) -> Vec<String> {
    let mut out = Vec::new();
    walk(a, b, "", limit, &mut out);
    out
}

fn walk(a: &Value, b: &Value, path: &str, limit: usize, out: &mut Vec<String>) {
    if out.len() >= limit || a == b {
        return;
    }
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            for k in x.keys().chain(y.keys().filter(|k| !x.contains_key(*k))) {
                let p = format!("{path}/{k}");
                match (x.get(k), y.get(k)) {
                    (Some(a), Some(b)) => walk(a, b, &p, limit, out),
                    (a, b) => out.push(format!("{p}: {} vs {}", short(a), short(b))),
                }
            }
        }
        (Value::Array(x), Value::Array(y)) if x.len() == y.len() => {
            for (i, (a, b)) in x.iter().zip(y).enumerate() {
                walk(a, b, &format!("{path}/{i}"), limit, out);
            }
        }
        _ => out.push(format!("{path}: {} vs {}", short(Some(a)), short(Some(b)))),
    }
}

fn short(v: Option<&Value>) -> String {
    let s = v.map_or_else(|| "(absent)".to_string(), Value::to_string);
    if s.len() > 200 {
        format!(
            "{}…",
            &s[..s.char_indices().nth(200).map_or(s.len(), |(i, _)| i)]
        )
    } else {
        s
    }
}
