//! Global appearance interner.
//!
//! CityJSONSeq gives each feature its own *localised* `appearance` block:
//! `materials`/`textures` definition arrays plus per-geometry index maps
//! that index into those local arrays. CityParquet needs dataset-global
//! identity instead, and stores appearance flat per WKB face, so this module:
//!
//! - dedupes material/texture definitions across every feature (and, for
//!   whole-document CityJSON, geometry templates) by canonical JSON content,
//!   assigning each distinct definition a single global row id;
//! - flattens each geometry's `material`/`texture` index map into the typed
//!   cells of [`crate::appearance_columns`] — one entry per WKB face, in WKB
//!   face order, keyed by theme — with every local index replaced by its
//!   global id;
//! - inlines every texture UV index as a `[u, v]` pair, one pair per distinct
//!   ring vertex, since UV vertex pools are themselves per-feature-local and
//!   have no stable global id.
//!
//! A `material` map is `{"<theme>": {"values": <nested ints|null>}}` or
//! `{"<theme>": {"value": <int>}}`; a `texture` map is `{"<theme>":
//! {"values": <nested rings>}}`, a *ring* being `[t, uv0, uv1, ...]` (texture
//! index followed by UV indices) or the sentinel `[null]` for "no texture".
//! The nesting of `values` differs by geometry type, so the flattening is
//! driven by the geometry's own boundaries through [`crate::faces`], the walk
//! that also produces `face_semantics`: one traversal defines WKB face order
//! for semantics and appearance alike, which is what makes "entry `i` is WKB
//! face `i`" meaningful. CityJSON's shorthands are expanded on the way — a
//! `null` standing for a whole shell or solid becomes one `null` per face
//! beneath it, a theme's whole-geometry `value` becomes one entry per face —
//! and the writer-dropped face positions are removed afterwards, so a theme's
//! list is exactly as long as the geometry's WKB face count. Texture goes one
//! level deeper: a face's entry is the list of its STORED rings
//! ([`crate::faces::Faces::ring_vertex_counts`]), each carrying one `[u, v]`
//! per distinct ring vertex.

use std::collections::{BTreeMap, HashMap, HashSet};

use cityparquet_schema::{CityParquetError, Result};
use cjseq::{MaterialIndex, MaterialObject, TextureObject, TexturedRing};
use serde::Serialize;
use serde_json::Value;

use crate::appearance_columns::{MaterialCell, TextureCell, TextureRing};
use crate::faces::{MaterialTheme, TextureTheme};

fn schema_err(msg: impl Into<String>) -> CityParquetError {
    CityParquetError::Schema(msg.into())
}

/// One appearance map's themes, in sorted name order.
///
/// A cell's MAP entry order is exactly this iteration order, and therefore
/// part of the bytes the writer lays down, so it must be a property of the
/// theme names alone: a geometry's themes come in source order, which would
/// otherwise leak into the column.
fn sorted_themes<'a, T>(themes: &'a [(&'a str, T)]) -> BTreeMap<&'a str, &'a T> {
    themes.iter().map(|(k, v)| (*k, v)).collect()
}

/// Dedupes CityJSON material/texture definitions across an entire dataset,
/// assigning each distinct definition a single global row id, and flattens
/// each geometry's per-geometry local index maps into the typed per-WKB-face
/// cells of [`crate::appearance_columns`] — one entry per WKB face, every
/// local index replaced by its global id, every texture UV index inlined as
/// a `[u, v]` pair.
#[derive(Debug, Default)]
pub struct AppearanceInterner {
    materials: Vec<Value>,
    material_ids: HashMap<String, usize>,
    textures: Vec<Value>,
    texture_ids: HashMap<String, usize>,
    /// When set by [`Self::set_tolerate_invalid_refs`], an out-of-range
    /// material/texture index is dropped instead of erroring.
    tolerate_invalid_refs: bool,
    /// Out-of-range material/texture indices dropped because
    /// [`Self::set_tolerate_invalid_refs`] was set — `0` in the (default)
    /// strict mode, where such an index is a `Schema` error instead. Counted
    /// the same way [`crate::encode::EncodeStats::degenerate_rings_dropped`]
    /// counts a writer-dropped ring: never silently.
    pub invalid_refs_dropped: usize,
    /// The global id each entry of the local definitions the current
    /// `flatten_*_map` call resolves against has been interned as — so a
    /// definition referenced by every face of a geometry is keyed (its
    /// canonical JSON built) once, not once per face. Reset by each call.
    local_ids: Vec<Option<usize>>,
}

impl AppearanceInterner {
    pub fn new() -> Self {
        Self::default()
    }

    /// Opt into dropping (rather than erroring on) a material/texture index
    /// that falls outside its local definitions array, and a texture ring
    /// with fewer UV indices than vertices or a UV index outside the UV pool
    /// (that ring is left untextured). Strict (the default,
    /// `tolerate = false`) is what every constructor leaves this at: the
    /// reference implementation is the appearance-resolution oracle, so a
    /// dangling reference stays fatal unless a caller explicitly opts out —
    /// see `ConvertOptions::tolerate_invalid_appearance`.
    pub fn set_tolerate_invalid_refs(&mut self, tolerate: bool) {
        self.tolerate_invalid_refs = tolerate;
    }

    /// Dedupe by canonical JSON: the key is `def` serialized with object
    /// members sorted by key (recursively; array order is semantic and left
    /// alone), so dedupe identity does not depend on serde_json's map type
    /// (`BTreeMap` by default, `IndexMap` under the `preserve_order`
    /// feature, which other workspace members may enable). Returns the
    /// global row id.
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

    /// Flatten one geometry's `material` themes (see
    /// [`crate::faces::material_themes`]) into a [`MaterialCell`]: theme ->
    /// one global material id (or `None`) per WKB face.
    ///
    /// `faces` is the geometry's face count before any drop; a theme's
    /// whole-geometry `value` is expanded to one entry per stored face.
    /// `dropped` are the writer-dropped ORIGINAL flat face positions, removed
    /// AFTER flattening.
    ///
    /// Every index `i` is replaced by `intern_material(&local_defs[i])`; an
    /// out-of-range `i` is a `Schema` error naming the theme, unless
    /// [`Self::set_tolerate_invalid_refs`] is set. A theme all of whose
    /// entries are `null` stays present as a same-length all-null list; an
    /// empty source map yields an empty [`MaterialCell::themes`], which the
    /// caller writes as a null cell.
    pub(crate) fn flatten_material_map(
        &mut self,
        themes: &[(&str, MaterialTheme)],
        faces: usize,
        dropped: &[usize],
        local_defs: &[MaterialObject],
    ) -> Result<MaterialCell> {
        self.local_ids.clear();
        self.local_ids.resize(local_defs.len(), None);
        let dropped: HashSet<usize> = dropped.iter().copied().collect();
        let stored = (0..faces).filter(|i| !dropped.contains(i)).count();

        let mut cell = Vec::with_capacity(themes.len());
        for (theme, entries) in sorted_themes(themes) {
            let ids = match entries {
                MaterialTheme::PerFace(flat) => flat
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| !dropped.contains(i))
                    .map(|(_, v)| self.resolve_material_index(*v, local_defs, theme))
                    .collect::<Result<Vec<_>>>()?,
                // A whole-geometry broadcast: resolve once, then repeat.
                MaterialTheme::Whole(value) => {
                    vec![self.resolve_material_index(Some(*value), local_defs, theme)?; stored]
                }
                MaterialTheme::Missing => {
                    return Err(schema_err(format!(
                        "material theme '{theme}' has neither 'values' nor 'value'"
                    )));
                }
            };
            cell.push((theme.to_string(), ids));
        }
        Ok(MaterialCell { themes: cell })
    }

    /// One `material` entry as a global id: `None` stays `None`, an index
    /// becomes the global id of `local_defs[i]`. An out-of-range index is a
    /// `Schema` error naming the theme, or — with
    /// [`Self::set_tolerate_invalid_refs`] — `None` plus one count in
    /// [`Self::invalid_refs_dropped`].
    fn resolve_material_index(
        &mut self,
        v: Option<MaterialIndex>,
        local_defs: &[MaterialObject],
        theme: &str,
    ) -> Result<Option<i64>> {
        let Some(MaterialIndex(idx)) = v else {
            return Ok(None);
        };
        match local_defs.get(idx) {
            Some(def) => Ok(Some(match self.local_ids[idx] {
                Some(id) => id,
                None => {
                    let id = self.intern_material(&definition_json(def)?);
                    self.local_ids[idx] = Some(id);
                    id
                }
            } as i64)),
            None if self.tolerate_invalid_refs => {
                self.invalid_refs_dropped += 1;
                Ok(None)
            }
            None => Err(schema_err(format!(
                "material index {idx} in theme '{theme}' out of range (local defs len {})",
                local_defs.len()
            ))),
        }
    }

    /// Flatten one geometry's `texture` themes (see
    /// [`crate::faces::texture_themes`]) into a [`TextureCell`]: theme -> per
    /// WKB face -> per STORED ring.
    ///
    /// `rings_per_face` is [`crate::faces::Faces::ring_vertex_counts`]. Within
    /// a face the source rings are matched positionally against the
    /// boundary's own rings: a ring the writer drops consumes its texture
    /// entry and contributes nothing, a source array that stops short leaves
    /// the remaining stored rings untextured, and a surplus source entry is
    /// ignored. A textured ring carries the first `n` of its UV indices
    /// resolved through `local_uvs`, `n` being the ring's distinct vertex
    /// count — fewer is an error naming the face and ring, and a surplus is
    /// the closing repeat the source ring carried, which the WKB ring's own
    /// closing point replaces.
    pub(crate) fn flatten_texture_map(
        &mut self,
        themes: &[TextureTheme<'_>],
        rings_per_face: &[Vec<Option<usize>>],
        dropped: &[usize],
        local_defs: &[TextureObject],
        local_uvs: &[[f64; 2]],
    ) -> Result<TextureCell> {
        self.local_ids.clear();
        self.local_ids.resize(local_defs.len(), None);
        let dropped: HashSet<usize> = dropped.iter().copied().collect();

        let mut cell = Vec::with_capacity(themes.len());
        for (theme, flat) in sorted_themes(themes) {
            let flat = flat.as_ref().ok_or_else(|| {
                schema_err(format!("texture theme '{theme}' is missing 'values'"))
            })?;
            let mut per_face = Vec::with_capacity(flat.len());
            for (face, entry) in flat.iter().enumerate() {
                if dropped.contains(&face) {
                    continue;
                }
                let ring_lens = rings_per_face.get(face).map_or(&[][..], Vec::as_slice);
                // Faces and rings are named by their STORED (WKB) position,
                // which is what the column's invariants are stated over.
                let stored_face = per_face.len();
                let rings = self.flatten_texture_face(
                    entry.unwrap_or(&[]),
                    ring_lens,
                    local_defs,
                    local_uvs,
                    theme,
                    stored_face,
                )?;
                per_face.push(rings);
            }
            cell.push((theme.to_string(), per_face));
        }
        Ok(TextureCell { themes: cell })
    }

    /// One face's stored rings (see [`Self::flatten_texture_map`]). `source`
    /// is empty when the face has no texture entry: every stored ring gets
    /// the untextured struct.
    fn flatten_texture_face(
        &mut self,
        source: &[TexturedRing],
        ring_lens: &[Option<usize>],
        local_defs: &[TextureObject],
        local_uvs: &[[f64; 2]],
        theme: &str,
        face: usize,
    ) -> Result<Vec<TextureRing>> {
        let mut out = Vec::with_capacity(ring_lens.len());
        let mut source = source.iter();
        for len in ring_lens {
            // A dropped ring still consumes its source entry: the entries
            // that follow belong to the rings that follow.
            let entry = source.next();
            let Some(len) = len else { continue };
            let site = RingSite {
                theme,
                face,
                ring: out.len(),
            };
            out.push(self.texture_ring(entry, *len, local_defs, local_uvs, &site)?);
        }
        Ok(out)
    }

    /// One stored ring's `{id, uv}` (see [`Self::flatten_texture_map`]).
    fn texture_ring(
        &mut self,
        entry: Option<&TexturedRing>,
        vertices: usize,
        local_defs: &[TextureObject],
        local_uvs: &[[f64; 2]],
        site: &RingSite,
    ) -> Result<TextureRing> {
        let RingSite { theme, face, ring } = *site;
        let bare = TextureRing { id: None, uv: None };
        // `[null]` is CityJSON's untextured-ring sentinel.
        let Some(TexturedRing::Textured { texture, uvs }) = entry else {
            return Ok(bare);
        };
        let idx = texture.0;
        let id = match local_defs.get(idx) {
            Some(def) => match self.local_ids[idx] {
                Some(id) => id as i64,
                None => {
                    let id = self.intern_texture(&definition_json(def)?);
                    self.local_ids[idx] = Some(id);
                    id as i64
                }
            },
            None if self.tolerate_invalid_refs => {
                self.invalid_refs_dropped += 1;
                // The UV entries are meaningless without a resolved texture,
                // so the whole ring becomes untextured.
                return Ok(bare);
            }
            None => {
                return Err(schema_err(format!(
                    "texture index {idx} in theme '{theme}' out of range (local defs len {})",
                    local_defs.len()
                )));
            }
        };
        if uvs.len() < vertices {
            // Fewer UVs than the ring has vertices (CityJSON 2.0.1 §6.2 asks
            // for one per vertex): which vertex lacks one is not stated, so
            // the ring's texture cannot be placed.
            if self.tolerate_invalid_refs {
                self.invalid_refs_dropped += 1;
                return Ok(bare);
            }
            return Err(schema_err(format!(
                "texture theme '{theme}' face {face} ring {ring}: {} uv indices for {vertices} \
                 distinct vertices",
                uvs.len()
            )));
        }
        let mut uv = Vec::with_capacity(vertices);
        // Anything past the distinct vertex count is the closing repeat the
        // source ring carried; the WKB ring closes itself and takes no pair.
        for uv_ref in &uvs[..vertices] {
            let idx = uv_ref.0;
            let Some(pair) = local_uvs.get(idx) else {
                if self.tolerate_invalid_refs {
                    self.invalid_refs_dropped += 1;
                    return Ok(bare);
                }
                return Err(schema_err(format!(
                    "UV index {idx} in theme '{theme}' out of range (local uvs len {})",
                    local_uvs.len()
                )));
            };
            uv.push(*pair);
        }
        Ok(TextureRing {
            id: Some(id),
            uv: Some(uv),
        })
    }
}

/// A material or texture definition as the JSON the sidecar stores and the
/// interner dedupes by.
fn definition_json(def: &impl Serialize) -> Result<Value> {
    Ok(serde_json::to_value(def)?)
}

/// Where a texture ring sits, for the errors that name it: the theme, and the
/// ring's STORED face and ring positions.
struct RingSite<'a> {
    theme: &'a str,
    face: usize,
    ring: usize,
}

fn intern(def: &Value, defs: &mut Vec<Value>, ids: &mut HashMap<String, usize>) -> usize {
    let key = canonical_json_string(def);
    if let Some(&id) = ids.get(&key) {
        return id;
    }
    let id = defs.len();
    defs.push(def.clone());
    ids.insert(key, id);
    id
}

/// Serializes `v` to a JSON string with object members sorted by key at
/// every level (arrays keep their original order — array order is
/// semantic). Unlike `Value::to_string()`, this is independent of
/// serde_json's internal object-map type: correct whether the crate is
/// built with the default sorted `BTreeMap` or, because some other
/// workspace member enables serde_json's `preserve_order` feature and
/// Cargo unifies features workspace-wide, an insertion-order `IndexMap`.
pub(crate) fn canonical_json_string(v: &Value) -> String {
    let mut out = String::new();
    write_canonical(v, &mut out);
    out
}

fn write_canonical(v: &Value, out: &mut String) {
    match v {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(k).unwrap());
                out.push(':');
                write_canonical(&map[*k], out);
            }
            out.push('}');
        }
        Value::Array(arr) => {
            out.push('[');
            for (i, e) in arr.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canonical(e, out);
            }
            out.push(']');
        }
        other => out.push_str(&other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    use cjseq::{CityJSON, Geometry};
    use serde_json::json;

    use crate::faces::Faces;

    use crate::appearance_columns::{TextureCell, TextureRing};
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

    // ---- flatten_material_map: flat per WKB face ----

    /// A Solid with `extra` members (`material`/`texture`); shell 0: 2 faces
    /// (the first with a hole), shell 1: 1 face.
    fn solid_two_shells(extra: Value) -> Geometry {
        multi_surface_or_solid(
            "Solid",
            json!([[[[0, 1, 2, 3], [4, 5, 6]], [[0, 1, 2]]], [[[7, 8, 9]]]]),
            extra,
        )
    }

    fn multi_surface_or_solid(thetype: &str, boundaries: Value, extra: Value) -> Geometry {
        let mut g = json!({"type": thetype, "lod": "2", "boundaries": boundaries});
        for (k, v) in extra.as_object().unwrap() {
            g[k] = v.clone();
        }
        crate::faces::geometry(g)
    }

    fn typed<T: serde::de::DeserializeOwned>(v: Value) -> T {
        serde_json::from_value(v).unwrap()
    }

    fn flatten_materials(
        i: &mut AppearanceInterner,
        g: &Geometry,
        dropped: &[usize],
        defs: &[MaterialObject],
    ) -> Result<MaterialCell> {
        let themes = crate::faces::material_themes(g).expect("a material member");
        i.flatten_material_map(&themes, Faces::of(g).count(), dropped, defs)
    }

    fn flatten_textures(
        i: &mut AppearanceInterner,
        g: &Geometry,
        dropped: &[usize],
        defs: &[TextureObject],
        uvs: &[[f64; 2]],
    ) -> Result<TextureCell> {
        let themes = crate::faces::texture_themes(g).expect("a texture member");
        i.flatten_texture_map(
            &themes,
            &Faces::of(g).ring_vertex_counts(),
            dropped,
            defs,
            uvs,
        )
    }

    #[test]
    fn material_values_flatten_per_wkb_face_and_expand_the_broadcast() {
        let g = solid_two_shells(json!({"material":
            {"": {"values": [[0, null], [1]]}, "night": {"value": 1}}}));
        let defs: Vec<MaterialObject> = typed(json!([{"name": "a"}, {"name": "b"}]));
        let mut i = AppearanceInterner::new();
        let cell = flatten_materials(&mut i, &g, &[], &defs).unwrap();
        assert_eq!(
            cell.themes,
            vec![
                ("".to_string(), vec![Some(0), None, Some(1)]),
                ("night".to_string(), vec![Some(1), Some(1), Some(1)]),
            ]
        );
    }

    #[test]
    fn material_null_shorthand_and_dropped_face_are_honoured() {
        // whole first shell null; face 1 (the hole-bearing face's neighbour)
        // dropped by the writer
        let g = solid_two_shells(json!({"material": {"": {"values": [null, [0]]}}}));
        let defs: Vec<MaterialObject> = typed(json!([{"name": "a"}]));
        let mut i = AppearanceInterner::new();
        let cell = flatten_materials(&mut i, &g, &[1], &defs).unwrap();
        assert_eq!(cell.themes, vec![("".to_string(), vec![None, Some(0)])]);
    }

    #[test]
    fn an_all_null_theme_stays_present() {
        let g = solid_two_shells(json!({"material":
            {"x": {"values": [[null, null], [null]]}}}));
        let mut i = AppearanceInterner::new();
        let cell = flatten_materials(&mut i, &g, &[], &[]).unwrap();
        assert_eq!(cell.themes, vec![("x".to_string(), vec![None, None, None])]);
    }

    #[test]
    fn material_out_of_range_index_is_a_schema_error_naming_the_theme() {
        let g = solid_two_shells(json!({"material": {"visual": {"value": 5}}}));
        let defs: Vec<MaterialObject> = typed(json!([{"name": "a"}]));
        let mut i = AppearanceInterner::new();
        let err = flatten_materials(&mut i, &g, &[], &defs).unwrap_err();
        match err {
            CityParquetError::Schema(msg) => assert!(msg.contains("visual"), "{msg}"),
            other => panic!("expected Schema error, got {other:?}"),
        }
    }

    /// A cell's MAP entry order is what the writer lays down on disk, so it
    /// must not depend on the source object's member order — which, under
    /// serde_json's `preserve_order` (an `IndexMap`, which cjseq enables), is
    /// the source's literal order. Both flatteners sort their themes by name,
    /// so the same multi-theme geometry writes the same bytes on every run.
    #[test]
    fn themes_are_flattened_in_sorted_order() {
        let g = solid_two_shells(json!({"material":
            {"night": {"value": 0}, "day": {"value": 0}}}));
        let defs: Vec<MaterialObject> = typed(json!([{"name": "a"}]));
        let mut i = AppearanceInterner::new();
        let cell = flatten_materials(&mut i, &g, &[], &defs).unwrap();
        assert_eq!(
            cell.themes
                .iter()
                .map(|(k, _)| k.as_str())
                .collect::<Vec<_>>(),
            vec!["day", "night"],
            "material themes must be written sorted, not in source order"
        );

        let tex_defs: Vec<TextureObject> = typed(json!([{"type": "PNG", "image": "a.png"}]));
        let uvs = [[0.0, 0.0]];
        let values = json!([
            [[[0, 0, 0, 0, 0], [0, 0, 0, 0]], [[0, 0, 0, 0]]],
            [[[0, 0, 0, 0]]]
        ]);
        let g = solid_two_shells(json!({"texture":
            {"night": {"values": values.clone()}, "day": {"values": values}}}));
        let cell = flatten_textures(&mut i, &g, &[], &tex_defs, &uvs).unwrap();
        assert_eq!(
            cell.themes
                .iter()
                .map(|(k, _)| k.as_str())
                .collect::<Vec<_>>(),
            vec!["day", "night"],
            "texture themes must be written sorted, not in source order"
        );
    }

    // ---- flatten_texture_map: per stored ring, UVs inlined ----

    #[test]
    fn texture_rings_inline_uvs_per_distinct_vertex_and_keep_ring_count() {
        let defs: Vec<TextureObject> = typed(json!([{"type": "PNG", "image": "a.png"}]));
        let uvs = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0], [0.5, 0.5]];
        let mut i = AppearanceInterner::new();
        let g = solid_two_shells(json!({"texture":
            {"": {"values": [ [ [[0, 0, 1, 2, 3], [null]], [] ], [ [[0, 4, 4, 4]] ] ]}}}));
        let cell = flatten_textures(&mut i, &g, &[], &defs, &uvs).unwrap();
        let faces = &cell.themes[0].1;
        assert_eq!(faces.len(), 3);
        assert_eq!(faces[0].len(), 2, "exterior + hole");
        assert_eq!(
            faces[0][0],
            TextureRing {
                id: Some(0),
                uv: Some(vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]),
            }
        );
        assert_eq!(faces[0][1], TextureRing { id: None, uv: None });
        assert_eq!(
            faces[1],
            vec![TextureRing { id: None, uv: None }],
            "a face with no ring entries is one bare ring per stored ring"
        );
        assert_eq!(faces[2][0].uv.as_ref().unwrap().len(), 3);
    }

    #[test]
    fn texture_uv_count_mismatch_is_an_error_naming_the_ring() {
        let defs: Vec<TextureObject> = typed(json!([{"type": "PNG", "image": "a.png"}]));
        let uvs = [[0.0, 0.0]; 5];
        let mut i = AppearanceInterner::new();
        // face 0's exterior has 4 vertices but only 2 uvs
        let g = solid_two_shells(json!({"texture":
            {"": {"values": [ [ [[0, 0, 1]], [[null]] ], [ [[null]] ] ]}}}));
        let e = flatten_textures(&mut i, &g, &[], &defs, &uvs)
            .unwrap_err()
            .to_string();
        assert!(e.contains("face 0") && e.contains("ring 0"), "{e}");
    }

    #[test]
    fn a_degenerate_middle_hole_removes_its_texture_entry_not_the_last_one() {
        // face 0 has an exterior, a 2-index (dropped) hole, and a real hole:
        // the stored face has 2 rings and the texture list must lose entry 1,
        // keeping the real hole's texture.
        let g = multi_surface_or_solid(
            "MultiSurface",
            json!([[[0, 1, 2, 3], [4, 5], [6, 7, 8]]]),
            json!({"texture":
                {"": {"values": [ [ [0, 0, 1, 2, 3], [1, 4, 5], [1, 6, 7, 8] ] ]}}}),
        );
        let defs: Vec<TextureObject> = typed(json!([
            {"type": "PNG", "image": "a.png"},
            {"type": "PNG", "image": "b.png"}
        ]));
        let uvs = [[0.0, 0.0]; 9];
        let mut i = AppearanceInterner::new();
        let cell = flatten_textures(&mut i, &g, &[], &defs, &uvs).unwrap();
        let rings = &cell.themes[0].1[0];
        assert_eq!(rings.len(), 2);
        assert_eq!(rings[0].id, Some(0));
        assert_eq!(
            (rings[1].id, rings[1].uv.as_ref().map(Vec::len)),
            (Some(1), Some(3)),
            "the real hole keeps its own texture"
        );
    }

    #[test]
    fn texture_uv_list_drops_the_closing_repeat_a_source_ring_carried() {
        // boundary ring [0,1,2,0] is closed in the source; the writer strips
        // the repeat, so the stored ring has 3 distinct vertices and the 4th
        // uv (for the repeat) is dropped.
        let g = multi_surface_or_solid(
            "MultiSurface",
            json!([[[0, 1, 2, 0]]]),
            json!({"texture": {"": {"values": [ [[0, 0, 1, 2, 3]] ]}}}),
        );
        let defs: Vec<TextureObject> = typed(json!([{"type": "PNG", "image": "a.png"}]));
        let uvs = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 0.0]];
        let mut i = AppearanceInterner::new();
        let cell = flatten_textures(&mut i, &g, &[], &defs, &uvs).unwrap();
        assert_eq!(cell.themes[0].1[0][0].uv.as_ref().unwrap().len(), 3);
    }

    #[test]
    fn texture_out_of_range_uv_index_is_a_schema_error_naming_the_theme() {
        let g = multi_surface_or_solid(
            "MultiSurface",
            json!([[[0, 1, 2]]]),
            json!({"texture": {"visual": {"values": [[[0, 0, 99, 0]]]}}}),
        );
        let defs: Vec<TextureObject> = typed(json!([{"image": "t0.png"}]));
        let uvs = [[0.0, 0.0]];
        let mut i = AppearanceInterner::new();
        let err = flatten_textures(&mut i, &g, &[], &defs, &uvs).unwrap_err();
        match err {
            CityParquetError::Schema(msg) => assert!(msg.contains("visual"), "{msg}"),
            other => panic!("expected Schema error, got {other:?}"),
        }
    }

    // ---- railway sweep: real CityJSON data, dataset-wide dedupe ----

    /// The first material index a geometry's themes refer to, in source
    /// order: used to correlate a feature-local index with the global id the
    /// flattened cell carries at the same position.
    fn first_material_index(g: &Geometry) -> Option<usize> {
        crate::faces::material_themes(g)?
            .into_iter()
            .find_map(|(_, theme)| match theme {
                MaterialTheme::PerFace(ids) => ids.into_iter().flatten().next(),
                MaterialTheme::Whole(id) => Some(id),
                MaterialTheme::Missing => None,
            })
            .map(|MaterialIndex(i)| i)
    }

    /// Asserts every ring of a flattened texture cell: `id` and `uv` are
    /// present together (or both absent), the id is `< texture_len`, and
    /// every pair is a real `[u, v]`.
    fn assert_cell_rings_valid(cell: &TextureCell, texture_len: usize) {
        for (theme, faces) in &cell.themes {
            for (f, rings) in faces.iter().enumerate() {
                for (r, ring) in rings.iter().enumerate() {
                    let where_ = || format!("theme '{theme}' face {f} ring {r}");
                    match (&ring.id, &ring.uv) {
                        (Some(id), Some(uv)) => {
                            assert!(
                                (*id as usize) < texture_len,
                                "{}: global texture id {id} must be < {texture_len}",
                                where_()
                            );
                            assert!(!uv.is_empty(), "{}: a textured ring needs uvs", where_());
                        }
                        (None, None) => {}
                        _ => panic!("{}: id and uv must be null together", where_()),
                    }
                }
            }
        }
    }

    #[test]
    fn railway_appearance_sweep_dedupes_and_flattens() {
        let src = Source::open(&fixture("lod3_railway.city.json")).unwrap();
        let mut interner = AppearanceInterner::new();

        let mut sample_texture_cell: Option<TextureCell> = None;
        let mut fidelity_check: Option<(usize, MaterialObject)> = None;
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
                    if crate::faces::material_themes(g).is_some() {
                        let cell =
                            flatten_materials(&mut interner, g, &[], &local_materials).unwrap();
                        swept_any_material = true;
                        if fidelity_check.is_none()
                            && let (Some(lid), Some(gid)) = (
                                first_material_index(g),
                                cell.themes
                                    .iter()
                                    .flat_map(|(_, ids)| ids.iter())
                                    .find_map(|id| *id),
                            )
                        {
                            fidelity_check = Some((gid as usize, local_materials[lid].clone()));
                        }
                    }
                    if crate::faces::texture_themes(g).is_some() {
                        let cell =
                            flatten_textures(&mut interner, g, &[], &local_textures, &local_uvs)
                                .unwrap();
                        swept_any_texture = true;
                        if sample_texture_cell.is_none() {
                            sample_texture_cell = Some(cell);
                        }
                    }
                }
            }
        }

        // Pin the features-only intermediate: everything below about the
        // template sweep rests on exactly this gap existing in the fixture.
        assert_eq!(
            interner.materials().len(),
            83,
            "features-only sweep: 2 materials are template-only"
        );
        assert_eq!(
            interner.textures().len(),
            33,
            "features-only sweep: 1 texture is template-only"
        );

        // The railway fixture is a whole CityJSON document (not a
        // CityJSONSeq stream): its `geometry-templates` carry their own
        // material/texture members, indexed against the document's
        // appearance arrays rather than any per-feature slice. Two materials
        // and one texture in this fixture are referenced only from
        // templates, never from a regular feature geometry, so the
        // per-feature sweep above alone lands on 83/33 rather than the
        // dataset's true 85/34. Sweep the templates too, against the
        // document's own appearance arrays, to reach every definition.
        let raw_text = std::fs::read_to_string(fixture("lod3_railway.city.json")).unwrap();
        let raw_doc: CityJSON = raw_text.parse().unwrap();
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
                if crate::faces::material_themes(g).is_some() {
                    flatten_materials(&mut interner, g, &[], &raw_materials).unwrap();
                }
                if crate::faces::texture_themes(g).is_some() {
                    flatten_textures(&mut interner, g, &[], &raw_textures, &raw_uvs).unwrap();
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

        // (b) every ring in one concrete flattened texture cell is
        // well-formed: global texture id < 34 (or an untextured ring), UVs
        // inlined as `[u, v]` pairs.
        let tex = sample_texture_cell.expect("expected at least one flattened texture cell");
        assert_cell_rings_valid(&tex, interner.textures().len());

        // (c) global-id fidelity: the global definition a rewritten index
        // points to is the feature-local definition it replaced.
        let (gid, expected_def) = fidelity_check.expect("expected at least one material index");
        assert_eq!(
            interner.materials()[gid],
            serde_json::to_value(expected_def).unwrap()
        );
    }
}
