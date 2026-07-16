//! CityGML 2.0 appearance emission (writer side): `app:X3DMaterial`.
//!
//! A geometry's stored `material` map is `{theme: {values|value: <global-index
//! tree>}}` where indices reference the dataset-global `materials.parquet`
//! table. This module flattens that tree to per-face global ids (in face-walk
//! order — the material `values` leaves are already in that order, so no shell
//! partition is needed), accumulates which face `gml:id`s use which material per
//! theme, and emits one `app:appearance/app:Appearance` per theme, each material
//! a full literal `app:X3DMaterial` (CityGML has no shared material library) with
//! its `app:target` face references.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;

use cityparquet_schema::CityParquetError;
use quick_xml::Writer;
use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use serde_json::Value;

use super::WriteReport;
use super::geometry::FaceIds;
use crate::Result;
use crate::wkb_read::DecodedKind;

/// One geometry's faces, each a `Vec` of its rings' `(global texture id, UVs)`
/// (`None` = untextured ring), in face-walk order.
pub type FaceRingTextures = Vec<Vec<Option<(usize, Vec<[f64; 2]>)>>>;

/// One geometry's per-theme flat texture maps. See [`texture_face_maps`].
pub type TextureFaceMaps = BTreeMap<String, FaceRingTextures>;

/// The textured polygons of one texture def: polygon `gml:id` -> its rings'
/// `(ring gml:id, UVs)`.
type TexPolys = BTreeMap<String, Vec<(String, Vec<[f64; 2]>)>>;

fn io_err(e: std::io::Error) -> CityParquetError {
    CityParquetError::Io(e.to_string())
}

fn err(m: impl Into<String>) -> CityParquetError {
    CityParquetError::Geometry(m.into())
}

/// The number of faces (polygons) of a geometry, in face-walk order — the length
/// every per-face appearance array must have.
pub fn count_faces(kind: &DecodedKind) -> usize {
    match kind {
        DecodedKind::PolyhedralSurface(faces) => faces.len(),
        DecodedKind::MultiPolygon(faces) => faces.len(),
        DecodedKind::GeometryCollection(members) => members.iter().map(count_faces).sum(),
        _ => 0,
    }
}

/// The ring count of each face in face-walk order — the shape a `texture` map's
/// `[face][ring]` tree must match (a mismatch would leave ring ids dangling).
pub fn face_ring_counts(kind: &DecodedKind) -> Vec<usize> {
    fn push(kind: &DecodedKind, out: &mut Vec<usize>) {
        match kind {
            DecodedKind::PolyhedralSurface(faces) | DecodedKind::MultiPolygon(faces) => {
                out.extend(faces.iter().map(Vec::len));
            }
            DecodedKind::GeometryCollection(members) => members.iter().for_each(|m| push(m, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    push(kind, &mut out);
    out
}

/// Flatten a material `values` tree's leaves (a non-negative integer -> a global
/// material id, `null` -> no material) in DFS (face-walk) order, range-checked
/// against the table length.
fn flatten_leaves(v: &Value, n_materials: usize, out: &mut Vec<Option<usize>>) -> Result<()> {
    match v {
        Value::Array(items) => {
            for it in items {
                flatten_leaves(it, n_materials, out)?;
            }
            Ok(())
        }
        Value::Null => {
            out.push(None);
            Ok(())
        }
        Value::Number(n) => {
            let i = n
                .as_u64()
                .ok_or_else(|| err("material index is not a non-negative integer"))?
                as usize;
            if i >= n_materials {
                return Err(err(format!(
                    "material index {i} >= materials table length {n_materials}"
                )));
            }
            out.push(Some(i));
            Ok(())
        }
        _ => Err(err("material values leaf is neither null nor an integer")),
    }
}

/// One geometry's per-theme flat material ids (each vec has length `n_faces`).
/// The `values` form is flattened; the scalar `value` form (whole-geometry
/// material) is expanded to every face.
pub fn material_face_maps(
    material_map: &Value,
    n_faces: usize,
    n_materials: usize,
) -> Result<BTreeMap<String, Vec<Option<usize>>>> {
    let obj = material_map
        .as_object()
        .ok_or_else(|| err("material map must be a JSON object of theme -> {values|value}"))?;
    let mut out = BTreeMap::new();
    for (theme, inner) in obj {
        let inner = inner
            .as_object()
            .ok_or_else(|| err(format!("material theme '{theme}' must be an object")))?;
        let flat = if let Some(values) = inner.get("values") {
            let mut leaves = Vec::new();
            flatten_leaves(values, n_materials, &mut leaves)?;
            if leaves.len() != n_faces {
                return Err(err(format!(
                    "material theme '{theme}' has {} values but geometry has {n_faces} faces",
                    leaves.len()
                )));
            }
            leaves
        } else if let Some(value) = inner.get("value") {
            let gid = value
                .as_u64()
                .ok_or_else(|| err("scalar material value is not a non-negative integer"))?
                as usize;
            if gid >= n_materials {
                return Err(err(format!(
                    "material index {gid} >= materials table length {n_materials}"
                )));
            }
            vec![Some(gid); n_faces]
        } else {
            return Err(err(format!(
                "material theme '{theme}' has neither 'values' nor 'value'"
            )));
        };
        out.insert(theme.clone(), flat);
    }
    Ok(out)
}

/// The per-face union across themes: `true` where any theme colours that face
/// (so the face needs a `gml:id` for `app:target`).
pub fn material_union(maps: &BTreeMap<String, Vec<Option<usize>>>, n_faces: usize) -> Vec<bool> {
    let mut u = vec![false; n_faces];
    for flat in maps.values() {
        for (k, m) in flat.iter().enumerate() {
            if m.is_some() && k < u.len() {
                u[k] = true;
            }
        }
    }
    u
}

/// Accumulates, per theme and global material id, the face `gml:id`s to target.
/// A `BTreeMap` keys deterministically (theme, then ascending global id).
#[derive(Default)]
pub struct AppearanceAcc {
    themes: BTreeMap<String, BTreeMap<usize, Vec<String>>>,
}

impl AppearanceAcc {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.themes.is_empty()
    }

    /// Record one geometry's contributions: for each theme, each face that has a
    /// material id AND an allocated `gml:id`, add `(theme, global id, face id)`.
    pub fn add(
        &mut self,
        maps: &BTreeMap<String, Vec<Option<usize>>>,
        face_ids: &[Option<String>],
    ) {
        for (theme, flat) in maps {
            for (k, m) in flat.iter().enumerate() {
                if let (Some(gid), Some(Some(fid))) = (m, face_ids.get(k)) {
                    self.themes
                        .entry(theme.clone())
                        .or_default()
                        .entry(*gid)
                        .or_default()
                        .push(fid.clone());
                }
            }
        }
    }
}

/// Flatten one geometry's `texture` map to per-theme `[face][ring]` textures
/// (global id + UVs, `None` = untextured ring). The stored texture tree mirrors
/// `boundaries` with each ring's `[t, [u,v]…]` (or `[null]`) leaf; the walk over
/// FACES (in walk order) collapses the shell/solid nesting.
pub fn texture_face_maps(texture_map: &Value, n_textures: usize) -> Result<TextureFaceMaps> {
    let obj = texture_map
        .as_object()
        .ok_or_else(|| err("texture map must be a JSON object of theme -> {values}"))?;
    let mut out = BTreeMap::new();
    for (theme, inner) in obj {
        let values = inner
            .as_object()
            .and_then(|o| o.get("values"))
            .ok_or_else(|| err(format!("texture theme '{theme}' is missing 'values'")))?;
        let mut faces = Vec::new();
        flatten_texture_faces(values, n_textures, &mut faces)?;
        out.insert(theme.clone(), faces);
    }
    Ok(out)
}

/// The per-face-per-ring union across themes: `true` where any theme textures
/// that ring (so the ring needs a `gml:id`). Faces beyond the geometry are
/// ignored; shorter per-face vecs pad with `false`.
pub fn texture_ring_needs(maps: &TextureFaceMaps, n_faces: usize) -> Vec<Vec<bool>> {
    let mut needs: Vec<Vec<bool>> = vec![Vec::new(); n_faces];
    for faces in maps.values() {
        for (fi, rings) in faces.iter().enumerate() {
            if fi >= n_faces {
                continue;
            }
            if needs[fi].len() < rings.len() {
                needs[fi].resize(rings.len(), false);
            }
            for (ri, ring) in rings.iter().enumerate() {
                if ring.is_some() {
                    needs[fi][ri] = true;
                }
            }
        }
    }
    needs
}

/// Whether a node is a ring leaf `[t, [u,v]…]` or `[null]` (its first element is
/// a number or null) — distinguishing a FACE (an array of ring leaves) from a
/// shell/solid container (an array of faces).
fn is_ring_leaf(v: &Value) -> bool {
    matches!(v, Value::Array(a) if matches!(a.first(), Some(Value::Number(_)) | Some(Value::Null) | None))
}

fn parse_ring_leaf(v: &Value, n_textures: usize) -> Result<Option<(usize, Vec<[f64; 2]>)>> {
    let a = v
        .as_array()
        .ok_or_else(|| err("texture ring leaf must be an array"))?;
    match a.first() {
        Some(Value::Number(n)) => {
            let tex = n
                .as_u64()
                .ok_or_else(|| err("texture id is not an integer"))? as usize;
            if tex >= n_textures {
                return Err(err(format!(
                    "texture id {tex} >= textures table length {n_textures}"
                )));
            }
            let uvs = a[1..]
                .iter()
                .map(|uv| {
                    let p = uv
                        .as_array()
                        .ok_or_else(|| err("UV must be a [u,v] array"))?;
                    let u = p.first().and_then(Value::as_f64);
                    let v = p.get(1).and_then(Value::as_f64);
                    match (u, v) {
                        (Some(u), Some(v)) => Ok([u, v]),
                        _ => Err(err("UV must be two numbers")),
                    }
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(Some((tex, uvs)))
        }
        _ => Ok(None), // [null]
    }
}

fn flatten_texture_faces(v: &Value, n_textures: usize, out: &mut FaceRingTextures) -> Result<()> {
    let Value::Array(items) = v else {
        return Err(err("texture values must be an array"));
    };
    if items.first().is_some_and(is_ring_leaf) {
        // `v` is a FACE: its children are ring leaves.
        let rings = items
            .iter()
            .map(|r| parse_ring_leaf(r, n_textures))
            .collect::<Result<Vec<_>>>()?;
        out.push(rings);
    } else {
        for it in items {
            flatten_texture_faces(it, n_textures, out)?;
        }
    }
    Ok(())
}

/// Accumulates, per theme and global texture id, the textured polygons and their
/// per-ring UVs. `BTreeMap`s key deterministically.
#[derive(Default)]
pub struct TextureAcc {
    themes: BTreeMap<String, BTreeMap<usize, TexPolys>>,
}

impl TextureAcc {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.themes.is_empty()
    }

    /// Record one geometry's textured rings: for each theme, each ring with a
    /// texture AND an allocated polygon+ring `gml:id`, add `(theme, texture id,
    /// polygon id, (ring id, UVs))`.
    pub fn add(&mut self, maps: &TextureFaceMaps, face_ids: &[FaceIds]) {
        for (theme, faces) in maps {
            for (fi, rings) in faces.iter().enumerate() {
                let Some(fids) = face_ids.get(fi) else {
                    continue;
                };
                let Some(poly) = fids.poly.as_ref() else {
                    continue;
                };
                for (ri, ring) in rings.iter().enumerate() {
                    if let (Some((tex, uvs)), Some(Some(rid))) = (ring, fids.rings.get(ri)) {
                        self.themes
                            .entry(theme.clone())
                            .or_default()
                            .entry(*tex)
                            .or_default()
                            .entry(poly.clone())
                            .or_default()
                            .push((rid.clone(), uvs.clone()));
                    }
                }
            }
        }
    }
}

/// Emit one `app:appearance/app:Appearance` per theme (union of the material and
/// texture themes), each with its used `app:X3DMaterial`s and
/// `app:ParameterizedTexture`s. `report` counts each emitted element.
pub fn write_appearance<W: Write>(
    w: &mut Writer<W>,
    materials: &AppearanceAcc,
    textures: &TextureAcc,
    material_table: &[Value],
    texture_table: &[Value],
    report: &mut WriteReport,
) -> Result<()> {
    let themes: BTreeSet<&String> = materials
        .themes
        .keys()
        .chain(textures.themes.keys())
        .collect();
    for theme in themes {
        w.write_event(Event::Start(BytesStart::new("app:appearance")))
            .map_err(io_err)?;
        w.write_event(Event::Start(BytesStart::new("app:Appearance")))
            .map_err(io_err)?;
        // The empty-string theme round-trips to an ABSENT app:theme.
        if !theme.is_empty() {
            text_elem(w, "app:theme", theme)?;
        }
        if let Some(mats) = materials.themes.get(theme) {
            for (gid, targets) in mats {
                let def = material_table.get(*gid).ok_or_else(|| {
                    err(format!(
                        "material global id {gid} out of range (table length {})",
                        material_table.len()
                    ))
                })?;
                write_x3d_material(w, def, targets)?;
                report.materials_written += 1;
            }
        }
        if let Some(texs) = textures.themes.get(theme) {
            for (tid, polys) in texs {
                let def = texture_table.get(*tid).ok_or_else(|| {
                    err(format!(
                        "texture global id {tid} out of range (table length {})",
                        texture_table.len()
                    ))
                })?;
                write_parameterized_texture(w, def, polys)?;
                report.textures_written += 1;
            }
        }
        w.write_event(Event::End(BytesEnd::new("app:Appearance")))
            .map_err(io_err)?;
        w.write_event(Event::End(BytesEnd::new("app:appearance")))
            .map_err(io_err)?;
    }
    Ok(())
}

/// CityJSON texture `type` -> CityGML `app:mimeType`.
fn type_to_mime(ty: &str) -> Option<&'static str> {
    match ty {
        "PNG" => Some("image/png"),
        "JPG" => Some("image/jpeg"),
        _ => None,
    }
}

/// Emit one `<app:surfaceDataMember><app:ParameterizedTexture>` from a CityJSON
/// texture definition and its textured polygons' per-ring UVs (re-closed:
/// the closing pair the reader dropped is re-appended).
fn write_parameterized_texture<W: Write>(
    w: &mut Writer<W>,
    def: &Value,
    polys: &TexPolys,
) -> Result<()> {
    w.write_event(Event::Start(BytesStart::new("app:surfaceDataMember")))
        .map_err(io_err)?;
    w.write_event(Event::Start(BytesStart::new("app:ParameterizedTexture")))
        .map_err(io_err)?;

    if let Some(image) = def.get("image").and_then(Value::as_str) {
        text_elem(w, "app:imageURI", image)?;
    }
    if let Some(mime) = def
        .get("type")
        .and_then(Value::as_str)
        .and_then(type_to_mime)
    {
        text_elem(w, "app:mimeType", mime)?;
    }
    if let Some(tt) = def.get("textureType").and_then(Value::as_str) {
        text_elem(w, "app:textureType", tt)?;
    }
    if let Some(wm) = def.get("wrapMode").and_then(Value::as_str) {
        text_elem(w, "app:wrapMode", wm)?;
    }
    if let Some(bc) = def.get("borderColor").and_then(Value::as_array) {
        let s = bc
            .iter()
            .filter_map(Value::as_f64)
            .map(|x| format!("{x}"))
            .collect::<Vec<_>>()
            .join(" ");
        text_elem(w, "app:borderColor", &s)?;
    }

    for (polyid, rings) in polys {
        let mut target = BytesStart::new("app:target");
        target.push_attribute(("uri", format!("#{polyid}").as_str()));
        w.write_event(Event::Start(target)).map_err(io_err)?;
        w.write_event(Event::Start(BytesStart::new("app:TexCoordList")))
            .map_err(io_err)?;
        for (ringid, uvs) in rings {
            let mut tc = BytesStart::new("app:textureCoordinates");
            tc.push_attribute(("ring", format!("#{ringid}").as_str()));
            w.write_event(Event::Start(tc)).map_err(io_err)?;
            w.write_event(Event::Text(BytesText::new(&closed_uvs(uvs))))
                .map_err(io_err)?;
            w.write_event(Event::End(BytesEnd::new("app:textureCoordinates")))
                .map_err(io_err)?;
        }
        w.write_event(Event::End(BytesEnd::new("app:TexCoordList")))
            .map_err(io_err)?;
        w.write_event(Event::End(BytesEnd::new("app:target")))
            .map_err(io_err)?;
    }

    w.write_event(Event::End(BytesEnd::new("app:ParameterizedTexture")))
        .map_err(io_err)?;
    w.write_event(Event::End(BytesEnd::new("app:surfaceDataMember")))
        .map_err(io_err)?;
    Ok(())
}

/// UV pairs as `u v` text, RE-CLOSED (the first pair re-appended) — GML texture
/// rings are closed. Numbers use the shortest round-tripping `f64` `Display`.
fn closed_uvs(uvs: &[[f64; 2]]) -> String {
    let mut out = String::new();
    let mut push = |uv: [f64; 2]| {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&format!("{} {}", uv[0], uv[1]));
    };
    for &uv in uvs {
        push(uv);
    }
    if let Some(&first) = uvs.first() {
        push(first);
    }
    out
}

/// Emit one `<app:surfaceDataMember><app:X3DMaterial>` from a CityJSON material
/// definition and its target face ids. Only present fields are written; numbers
/// use the shortest round-tripping `f64` `Display` (bit-exact re-parse).
fn write_x3d_material<W: Write>(w: &mut Writer<W>, def: &Value, targets: &[String]) -> Result<()> {
    w.write_event(Event::Start(BytesStart::new("app:surfaceDataMember")))
        .map_err(io_err)?;
    w.write_event(Event::Start(BytesStart::new("app:X3DMaterial")))
        .map_err(io_err)?;

    if let Some(name) = def.get("name").and_then(Value::as_str) {
        text_elem(w, "gml:name", name)?;
    }
    for (key, elem) in [
        ("ambientIntensity", "app:ambientIntensity"),
        ("shininess", "app:shininess"),
        ("transparency", "app:transparency"),
    ] {
        if let Some(x) = def.get(key).and_then(Value::as_f64) {
            text_elem(w, elem, &format!("{x}"))?;
        }
    }
    for (key, elem) in [
        ("diffuseColor", "app:diffuseColor"),
        ("emissiveColor", "app:emissiveColor"),
        ("specularColor", "app:specularColor"),
    ] {
        if let Some(rgb) = def.get(key).and_then(Value::as_array) {
            let s = rgb
                .iter()
                .filter_map(Value::as_f64)
                .map(|x| format!("{x}"))
                .collect::<Vec<_>>()
                .join(" ");
            text_elem(w, elem, &s)?;
        }
    }
    if let Some(b) = def.get("isSmooth").and_then(Value::as_bool) {
        text_elem(w, "app:isSmooth", if b { "true" } else { "false" })?;
    }
    for t in targets {
        text_elem(w, "app:target", &format!("#{t}"))?;
    }

    w.write_event(Event::End(BytesEnd::new("app:X3DMaterial")))
        .map_err(io_err)?;
    w.write_event(Event::End(BytesEnd::new("app:surfaceDataMember")))
        .map_err(io_err)?;
    Ok(())
}

/// A `<tag>text</tag>` element (text auto-escaped by quick-xml).
fn text_elem<W: Write>(w: &mut Writer<W>, tag: &str, text: &str) -> Result<()> {
    w.write_event(Event::Start(BytesStart::new(tag)))
        .map_err(io_err)?;
    w.write_event(Event::Text(BytesText::new(text)))
        .map_err(io_err)?;
    w.write_event(Event::End(BytesEnd::new(tag)))
        .map_err(io_err)?;
    Ok(())
}
