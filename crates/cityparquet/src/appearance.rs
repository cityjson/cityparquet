//! Global appearance interner.
//!
//! CityJSONSeq gives each feature its own *localised* `appearance` block:
//! `materials`/`textures` definition arrays plus per-geometry index maps
//! that index into those local arrays. CityParquet needs dataset-global
//! identity instead, so this module:
//!
//! - dedupes material/texture definitions across every feature (and, for
//!   whole-document CityJSON, geometry templates) by canonical JSON content,
//!   assigning each distinct definition a single global row id;
//! - rewrites each geometry's `material`/`texture` index map so integer
//!   indices reference the global ids instead of the feature-local ones;
//! - inlines every texture UV index as a `[u, v]` pair, since UV vertex
//!   pools are themselves per-feature-local and have no stable global id.
//!
//! The rewrite walks the CityJSON map shapes generically (see the CityJSON
//! spec): a `material` map is `{"<theme>": {"values": <nested ints|null>}}`
//! or `{"<theme>": {"value": <int>}}`, where the nesting of `values` differs
//! by geometry type; a `texture` map is `{"<theme>": {"values": <nested
//! rings>}}` where a *ring* is the innermost array `[t, uv0, uv1, ...]`
//! (texture index followed by UV indices), or the sentinel `[null]` for "no
//! texture". Rather than special-case each `GeometryType`'s nesting depth,
//! the walk recurses through arrays generically and recognises a ring by
//! shape: an array whose first element is a number or `null` and whose
//! elements are not themselves arrays.

use std::collections::HashMap;

use cityparquet_schema::{CityParquetError, Result};
use serde_json::Value;

fn schema_err(msg: impl Into<String>) -> CityParquetError {
    CityParquetError::Schema(msg.into())
}

/// Dedupes CityJSON material/texture definitions across an entire dataset
/// and rewrites per-geometry local index maps to reference the resulting
/// dataset-global rows, inlining UV coordinates for textures.
#[derive(Debug, Default)]
pub struct AppearanceInterner {
    materials: Vec<Value>,
    material_ids: HashMap<String, usize>,
    textures: Vec<Value>,
    texture_ids: HashMap<String, usize>,
}

impl AppearanceInterner {
    pub fn new() -> Self {
        Self::default()
    }

    /// Dedupe by canonical JSON (serde_json's default `BTreeMap` key
    /// ordering makes `to_string` canonical); returns the global row id.
    pub fn intern_material(&mut self, def: &Value) -> usize {
        intern(def, &mut self.materials, &mut self.material_ids)
    }

    /// Dedupe by canonical JSON; returns the global row id.
    pub fn intern_texture(&mut self, def: &Value) -> usize {
        intern(def, &mut self.textures, &mut self.texture_ids)
    }

    pub fn materials(&self) -> &[Value] {
        &self.materials
    }

    pub fn textures(&self) -> &[Value] {
        &self.textures
    }

    /// Rewrite one geometry's `material` member: `{"<theme>": {"values":
    /// <nested ints|null>} | {"value": <int>}}`. Every integer index `i` is
    /// replaced by `intern_material(&local_defs[i])`; `null` is preserved;
    /// an out-of-range `i` is a `Schema` error naming the theme.
    pub fn rewrite_material_map(&mut self, map: &Value, local_defs: &[Value]) -> Result<Value> {
        let obj = map.as_object().ok_or_else(|| {
            schema_err("material map must be a JSON object of theme -> {value|values}")
        })?;
        let mut out = serde_json::Map::with_capacity(obj.len());
        for (theme, inner) in obj {
            let inner_obj = inner
                .as_object()
                .ok_or_else(|| schema_err(format!("material theme '{theme}' must be an object")))?;
            let mut new_inner = serde_json::Map::with_capacity(inner_obj.len());
            if let Some(v) = inner_obj.get("value") {
                new_inner.insert(
                    "value".to_string(),
                    self.rewrite_material_index(v, local_defs, theme)?,
                );
            }
            if let Some(v) = inner_obj.get("values") {
                new_inner.insert(
                    "values".to_string(),
                    self.rewrite_material_tree(v, local_defs, theme)?,
                );
            }
            out.insert(theme.clone(), Value::Object(new_inner));
        }
        Ok(Value::Object(out))
    }

    fn rewrite_material_tree(
        &mut self,
        v: &Value,
        local_defs: &[Value],
        theme: &str,
    ) -> Result<Value> {
        match v {
            Value::Array(items) => {
                let mapped = items
                    .iter()
                    .map(|x| self.rewrite_material_tree(x, local_defs, theme))
                    .collect::<Result<Vec<_>>>()?;
                Ok(Value::Array(mapped))
            }
            _ => self.rewrite_material_index(v, local_defs, theme),
        }
    }

    fn rewrite_material_index(
        &mut self,
        v: &Value,
        local_defs: &[Value],
        theme: &str,
    ) -> Result<Value> {
        match v {
            Value::Null => Ok(Value::Null),
            Value::Number(n) => {
                let idx = n.as_u64().ok_or_else(|| {
                    schema_err(format!(
                        "material index in theme '{theme}' is not a non-negative integer: {n}"
                    ))
                })? as usize;
                let def = local_defs.get(idx).ok_or_else(|| {
                    schema_err(format!(
                        "material index {idx} in theme '{theme}' out of range (local defs len {})",
                        local_defs.len()
                    ))
                })?;
                Ok(Value::from(self.intern_material(def)))
            }
            other => Err(schema_err(format!(
                "material index in theme '{theme}' must be an integer or null, got {other}"
            ))),
        }
    }

    /// Rewrite one geometry's `texture` member: within each innermost ring
    /// array `[t, uv0, uv1, ...]`, `t` (position 0) becomes the global
    /// texture id and every following UV index becomes the inlined `[u, v]`
    /// pair from `local_uvs`; an all-null ring `[null]` is preserved as-is.
    pub fn rewrite_texture_map(
        &mut self,
        map: &Value,
        local_defs: &[Value],
        local_uvs: &[Vec<f64>],
    ) -> Result<Value> {
        let obj = map
            .as_object()
            .ok_or_else(|| schema_err("texture map must be a JSON object of theme -> {values}"))?;
        let mut out = serde_json::Map::with_capacity(obj.len());
        for (theme, inner) in obj {
            let inner_obj = inner
                .as_object()
                .ok_or_else(|| schema_err(format!("texture theme '{theme}' must be an object")))?;
            let values = inner_obj.get("values").ok_or_else(|| {
                schema_err(format!("texture theme '{theme}' is missing 'values'"))
            })?;
            let rewritten = self.rewrite_texture_tree(values, local_defs, local_uvs, theme)?;
            let mut new_inner = serde_json::Map::with_capacity(1);
            new_inner.insert("values".to_string(), rewritten);
            out.insert(theme.clone(), Value::Object(new_inner));
        }
        Ok(Value::Object(out))
    }

    fn rewrite_texture_tree(
        &mut self,
        v: &Value,
        local_defs: &[Value],
        local_uvs: &[Vec<f64>],
        theme: &str,
    ) -> Result<Value> {
        match v {
            Value::Array(items) => {
                if is_texture_ring(items) {
                    self.rewrite_texture_ring(items, local_defs, local_uvs, theme)
                } else {
                    let mapped = items
                        .iter()
                        .map(|x| self.rewrite_texture_tree(x, local_defs, local_uvs, theme))
                        .collect::<Result<Vec<_>>>()?;
                    Ok(Value::Array(mapped))
                }
            }
            other => Err(schema_err(format!(
                "unexpected non-array node in texture theme '{theme}': {other}"
            ))),
        }
    }

    fn rewrite_texture_ring(
        &mut self,
        items: &[Value],
        local_defs: &[Value],
        local_uvs: &[Vec<f64>],
        theme: &str,
    ) -> Result<Value> {
        if items.len() == 1 && items[0].is_null() {
            return Ok(Value::Array(vec![Value::Null]));
        }
        let mut out = Vec::with_capacity(items.len());
        out.push(match &items[0] {
            Value::Null => Value::Null,
            Value::Number(n) => {
                let idx = n.as_u64().ok_or_else(|| {
                    schema_err(format!(
                        "texture index in theme '{theme}' is not a non-negative integer: {n}"
                    ))
                })? as usize;
                let def = local_defs.get(idx).ok_or_else(|| {
                    schema_err(format!(
                        "texture index {idx} in theme '{theme}' out of range (local defs len {})",
                        local_defs.len()
                    ))
                })?;
                Value::from(self.intern_texture(def))
            }
            other => {
                return Err(schema_err(format!(
                    "texture index in theme '{theme}' must be an integer or null, got {other}"
                )));
            }
        });
        for uv_ref in &items[1..] {
            let idx = uv_ref.as_u64().ok_or_else(|| {
                schema_err(format!(
                    "UV index in theme '{theme}' must be a non-negative integer, got {uv_ref}"
                ))
            })? as usize;
            let uv = local_uvs.get(idx).ok_or_else(|| {
                schema_err(format!(
                    "UV index {idx} in theme '{theme}' out of range (local uvs len {})",
                    local_uvs.len()
                ))
            })?;
            if uv.len() < 2 {
                return Err(schema_err(format!(
                    "UV vertex {idx} in theme '{theme}' has fewer than 2 coordinates"
                )));
            }
            out.push(serde_json::json!([uv[0], uv[1]]));
        }
        Ok(Value::Array(out))
    }
}

fn intern(def: &Value, defs: &mut Vec<Value>, ids: &mut HashMap<String, usize>) -> usize {
    let key = def.to_string();
    if let Some(&id) = ids.get(&key) {
        return id;
    }
    let id = defs.len();
    defs.push(def.clone());
    ids.insert(key, id);
    id
}

/// A *ring* is the innermost texture-map array: `[t, uv0, uv1, ...]` where
/// `t` is a texture index (or `null`), and the rest are UV indices. It is
/// recognised, before any rewriting, as an array whose first element is a
/// number or null and whose elements are not themselves arrays.
fn is_texture_ring(items: &[Value]) -> bool {
    !items.is_empty()
        && matches!(items[0], Value::Number(_) | Value::Null)
        && items.iter().all(|x| !x.is_array())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    use cjseq::CityJSON;
    use serde_json::json;

    use crate::source::Source;

    fn fixture(name: &str) -> PathBuf {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures")
            .join(name);
        assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
        p
    }

    // ---- intern_material / intern_texture: dedupe by canonical JSON ----

    #[test]
    fn intern_material_dedupes_identical_definitions() {
        let mut interner = AppearanceInterner::new();
        let a = json!({"name": "roof", "diffuseColor": [1.0, 0.0, 0.0]});
        let b = json!({"diffuseColor": [1.0, 0.0, 0.0], "name": "roof"}); // different key order
        let id_a = interner.intern_material(&a);
        let id_b = interner.intern_material(&b);
        assert_eq!(id_a, id_b, "same content (different key order) must dedupe");
        assert_eq!(interner.materials().len(), 1);
        assert_eq!(interner.materials()[id_a], a);
    }

    #[test]
    fn intern_texture_dedupes_identical_definitions() {
        let mut interner = AppearanceInterner::new();
        let a = json!({"type": "PNG", "image": "a.png"});
        let b = json!({"type": "PNG", "image": "b.png"});
        let id_a = interner.intern_texture(&a);
        let id_b = interner.intern_texture(&b);
        assert_ne!(id_a, id_b);
        assert_eq!(interner.textures().len(), 2);
        assert_eq!(
            interner.intern_texture(&a),
            id_a,
            "re-interning must return the same id"
        );
    }

    // ---- rewrite_material_map: scalar `value` form ----

    #[test]
    fn rewrite_material_map_scalar_value_form() {
        let mut interner = AppearanceInterner::new();
        let local_defs = vec![json!({"name": "m0"}), json!({"name": "m1"})];
        let map = json!({"visual": {"value": 1}});
        let rewritten = interner.rewrite_material_map(&map, &local_defs).unwrap();
        let gid = rewritten["visual"]["value"].as_u64().unwrap() as usize;
        assert_eq!(interner.materials()[gid], local_defs[1]);
    }

    #[test]
    fn rewrite_material_map_nested_values_preserves_nulls_and_structure() {
        let mut interner = AppearanceInterner::new();
        let local_defs = vec![json!({"name": "m0"}), json!({"name": "m1"})];
        // Solid-shaped nesting: Vec<Vec<Option<usize>>>
        let map = json!({"visual": {"values": [[0, null, 1], [1, 0]]}});
        let rewritten = interner.rewrite_material_map(&map, &local_defs).unwrap();
        let values = rewritten["visual"]["values"].as_array().unwrap();
        assert_eq!(values[0][1], Value::Null);
        let g0 = values[0][0].as_u64().unwrap() as usize;
        let g1 = values[0][2].as_u64().unwrap() as usize;
        assert_eq!(interner.materials()[g0], local_defs[0]);
        assert_eq!(interner.materials()[g1], local_defs[1]);
    }

    #[test]
    fn rewrite_material_map_out_of_range_index_is_schema_error() {
        let mut interner = AppearanceInterner::new();
        let local_defs = vec![json!({"name": "m0"})];
        let map = json!({"visual": {"value": 5}});
        let err = interner
            .rewrite_material_map(&map, &local_defs)
            .unwrap_err();
        match err {
            CityParquetError::Schema(msg) => assert!(msg.contains("visual"), "{msg}"),
            other => panic!("expected Schema error, got {other:?}"),
        }
    }

    // ---- rewrite_texture_map: ring recognition + UV inlining ----

    #[test]
    fn rewrite_texture_map_inlines_uvs_and_preserves_null_ring() {
        let mut interner = AppearanceInterner::new();
        let local_defs = vec![json!({"image": "t0.png"})];
        let local_uvs = vec![vec![0.0, 0.0], vec![1.0, 0.0], vec![1.0, 1.0]];
        // MultiSurface-shaped nesting: Vec<Vec<Ring>>
        let map = json!({"visual": {"values": [[[0, 0, 1, 2]], [[null]]]}});
        let rewritten = interner
            .rewrite_texture_map(&map, &local_defs, &local_uvs)
            .unwrap();
        let values = rewritten["visual"]["values"].as_array().unwrap();
        let ring0 = values[0][0].as_array().unwrap();
        assert_eq!(ring0[0], json!(0));
        assert_eq!(ring0[1], json!([0.0, 0.0]));
        assert_eq!(ring0[2], json!([1.0, 0.0]));
        assert_eq!(ring0[3], json!([1.0, 1.0]));
        let ring1 = values[1][0].as_array().unwrap();
        assert_eq!(ring1, &vec![Value::Null]);
    }

    #[test]
    fn rewrite_texture_map_out_of_range_uv_is_schema_error() {
        let mut interner = AppearanceInterner::new();
        let local_defs = vec![json!({"image": "t0.png"})];
        let local_uvs = vec![vec![0.0, 0.0]];
        let map = json!({"visual": {"values": [[[0, 99]]]}});
        let err = interner
            .rewrite_texture_map(&map, &local_defs, &local_uvs)
            .unwrap_err();
        match err {
            CityParquetError::Schema(msg) => assert!(msg.contains("visual"), "{msg}"),
            other => panic!("expected Schema error, got {other:?}"),
        }
    }

    // ---- railway sweep: real CityJSON data, dataset-wide dedupe ----

    /// Finds the first JSON number reachable via a pre-order walk of objects
    /// (in key order) then arrays (in element order). Used to correlate a
    /// position in an unrewritten map with the same position in the
    /// rewritten map, without hard-coding the map's nesting shape.
    fn find_first_int(v: &Value) -> Option<u64> {
        match v {
            Value::Number(n) => n.as_u64(),
            Value::Array(items) => items.iter().find_map(find_first_int),
            Value::Object(map) => map.values().find_map(find_first_int),
            _ => None,
        }
    }

    /// Recursively asserts every innermost ring in a *rewritten* texture
    /// tree: first element is an integer `< texture_len` (or the ring is
    /// `[null]`), and every subsequent element is a 2-element numeric pair.
    fn assert_rewritten_rings_valid(v: &Value, texture_len: usize) {
        match v {
            Value::Array(items) => {
                let is_ring =
                    !items.is_empty() && matches!(items[0], Value::Number(_) | Value::Null);
                if is_ring {
                    if let Value::Number(n) = &items[0] {
                        let t = n.as_u64().expect("global texture id must be a u64");
                        assert!(
                            (t as usize) < texture_len,
                            "global texture id {t} must be < {texture_len}"
                        );
                    }
                    for uv in &items[1..] {
                        let pair = uv
                            .as_array()
                            .unwrap_or_else(|| panic!("expected inlined [u, v] pair, got {uv}"));
                        assert_eq!(pair.len(), 2, "UV pair must have exactly 2 coordinates");
                        assert!(pair[0].is_number() && pair[1].is_number());
                    }
                } else {
                    for x in items {
                        assert_rewritten_rings_valid(x, texture_len);
                    }
                }
            }
            Value::Object(map) => {
                for x in map.values() {
                    assert_rewritten_rings_valid(x, texture_len);
                }
            }
            _ => {}
        }
    }

    #[test]
    fn railway_appearance_sweep_dedupes_and_rewrites() {
        let src = Source::open(&fixture("lod3_railway.city.json")).unwrap();
        let mut interner = AppearanceInterner::new();

        let mut sample_rewritten_texture: Option<Value> = None;
        let mut fidelity_check: Option<(usize, Value)> = None;
        let mut swept_any_material = false;
        let mut swept_any_texture = false;

        for feature in src.features().unwrap() {
            let feature = feature.unwrap();
            let Some(appearance) = &feature.appearance else {
                continue;
            };
            let local_materials = appearance.materials.clone().unwrap_or_default();
            let local_textures = appearance.textures.clone().unwrap_or_default();
            let local_uvs = appearance.vertices_texture.clone().unwrap_or_default();

            for co in feature.city_objects.values() {
                let Some(geoms) = &co.geometry else { continue };
                for g in geoms {
                    if let Some(material) = &g.material {
                        let map = serde_json::to_value(material).unwrap();
                        let rewritten = interner
                            .rewrite_material_map(&map, &local_materials)
                            .unwrap();
                        swept_any_material = true;
                        if fidelity_check.is_none()
                            && let (Some(lid), Some(gid)) =
                                (find_first_int(&map), find_first_int(&rewritten))
                        {
                            fidelity_check =
                                Some((gid as usize, local_materials[lid as usize].clone()));
                        }
                    }
                    if let Some(texture) = &g.texture {
                        let map = serde_json::to_value(texture).unwrap();
                        let rewritten = interner
                            .rewrite_texture_map(&map, &local_textures, &local_uvs)
                            .unwrap();
                        swept_any_texture = true;
                        if sample_rewritten_texture.is_none() {
                            sample_rewritten_texture = Some(rewritten);
                        }
                    }
                }
            }
        }

        // The railway fixture is a whole CityJSON document (not a
        // CityJSONSeq stream): its `geometry-templates` carry their own
        // material/texture members, generically shaped exactly like a
        // regular geometry's, but indexed against the document's *raw,
        // unsliced* appearance arrays rather than any per-feature local
        // slice (cjseq's `CityJSON::get_metadata` clones the templates
        // before reslicing their indices, so the clone actually attached to
        // `Source::header()` keeps the original document-global indices).
        // Two materials and one texture in this fixture are referenced only
        // from templates, never from a regular feature geometry, so the
        // per-feature sweep above alone would land on 83/33 rather than the
        // dataset's true 85/34. Sweep the templates too, against the raw
        // document's own appearance arrays, to reach every definition.
        let raw_text = std::fs::read_to_string(fixture("lod3_railway.city.json")).unwrap();
        let raw_doc = CityJSON::from_str(&raw_text).unwrap();
        if let Some(templates) = &raw_doc.geometry_templates {
            let raw_appearance = raw_doc.appearance.as_ref();
            let raw_materials = raw_appearance
                .and_then(|a| a.materials.clone())
                .unwrap_or_default();
            let raw_textures = raw_appearance
                .and_then(|a| a.textures.clone())
                .unwrap_or_default();
            let raw_uvs = raw_appearance
                .and_then(|a| a.vertices_texture.clone())
                .unwrap_or_default();
            for g in &templates.templates {
                if let Some(material) = &g.material {
                    let map = serde_json::to_value(material).unwrap();
                    interner.rewrite_material_map(&map, &raw_materials).unwrap();
                }
                if let Some(texture) = &g.texture {
                    let map = serde_json::to_value(texture).unwrap();
                    interner
                        .rewrite_texture_map(&map, &raw_textures, &raw_uvs)
                        .unwrap();
                }
            }
        }

        assert!(
            swept_any_material,
            "expected at least one material geometry in railway"
        );
        assert!(
            swept_any_texture,
            "expected at least one texture geometry in railway"
        );

        // (a) pinned: railway has no duplicate material/texture definitions,
        // and (feature geometries) ∪ (template geometries) together
        // reference every entry in the document's appearance arrays.
        assert_eq!(interner.materials().len(), 85);
        assert_eq!(interner.textures().len(), 34);

        // (b) every innermost ring in one concrete rewritten texture map is
        // well-formed: global texture id < 34 (or `[null]`), UVs inlined as
        // 2-element numeric pairs.
        let tex = sample_rewritten_texture.expect("expected at least one rewritten texture map");
        assert_rewritten_rings_valid(&tex, interner.textures().len());

        // (c) global-id fidelity: the global definition a rewritten index
        // points to is byte-for-byte the feature-local definition it
        // replaced.
        let (gid, expected_def) = fidelity_check.expect("expected at least one material index");
        assert_eq!(interner.materials()[gid], expected_def);
    }
}
