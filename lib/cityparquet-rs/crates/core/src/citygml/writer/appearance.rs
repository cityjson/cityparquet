//! CityGML 2.0 appearance emission (writer side): `app:X3DMaterial`.
//!
//! A geometry's stored `material` map is `{theme: {values: [id|null, …]}}`, one
//! entry per WKB face in face-walk order, referencing the dataset-global
//! `materials.parquet` table by id. This module reads that flat per-face shape
//! directly, accumulates which face `gml:id`s use which material per theme, and
//! emits one `app:appearance/app:Appearance` per theme, each material a full
//! literal `app:X3DMaterial` (CityGML has no shared material library) with its
//! `app:target` face references.

use std::collections::{BTreeMap, BTreeSet, HashMap};
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
/// `[face][ring]` flat per-face list must match (a mismatch would leave ring
/// ids dangling).
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

/// Parse one theme's flat material `values` list — one entry per WKB face, in
/// face-walk order, each a non-negative integer -> a global material id, or
/// `null` -> no material — checking each id against `materials` BY ID, not by
/// position: a merged package's ids are offset-shifted and gapped, so "less
/// than the table length" is not the same question as "exists".
fn parse_material_values(
    theme: &str,
    values: &Value,
    n_faces: usize,
    materials: &HashMap<i64, Value>,
) -> Result<Vec<Option<usize>>> {
    let items = values
        .as_array()
        .ok_or_else(|| err(format!("material theme '{theme}' values must be an array")))?;
    if items.len() != n_faces {
        return Err(err(format!(
            "material theme '{theme}' has {} values but geometry has {n_faces} faces",
            items.len()
        )));
    }
    items
        .iter()
        .map(|v| match v {
            Value::Null => Ok(None),
            Value::Number(n) => {
                let i = n
                    .as_u64()
                    .ok_or_else(|| err("material index is not a non-negative integer"))?
                    as usize;
                if !materials.contains_key(&(i as i64)) {
                    return Err(err(format!(
                        "material id {i} does not name a row in materials.parquet"
                    )));
                }
                Ok(Some(i))
            }
            _ => Err(err("material values entry is neither null nor an integer")),
        })
        .collect()
}

/// One geometry's per-theme flat material ids: each theme's `values` list is
/// one entry per WKB face, in face-walk order (a non-negative integer -> a
/// global material id, `null` -> no material), of length `n_faces` exactly.
pub fn material_face_maps(
    material_map: &Value,
    n_faces: usize,
    materials: &HashMap<i64, Value>,
) -> Result<BTreeMap<String, Vec<Option<usize>>>> {
    let obj = material_map
        .as_object()
        .ok_or_else(|| err("material map must be a JSON object of theme -> {values}"))?;
    let mut out = BTreeMap::new();
    for (theme, inner) in obj {
        let values = inner
            .as_object()
            .and_then(|o| o.get("values"))
            .ok_or_else(|| err(format!("material theme '{theme}' is missing 'values'")))?;
        let flat = parse_material_values(theme, values, n_faces, materials)?;
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

/// Parse one theme's flat texture `values` list — one entry per WKB face, in
/// face-walk order, each face an array of its rings' `[t, [u,v]…]` (or
/// `[null]`) leaves, parsed by [`parse_ring_leaf`] — of length `n_faces`
/// exactly, as [`parse_material_values`] demands of its own list.
fn parse_texture_values(
    theme: &str,
    values: &Value,
    n_faces: usize,
    textures: &HashMap<i64, Value>,
) -> Result<FaceRingTextures> {
    let items = values
        .as_array()
        .ok_or_else(|| err(format!("texture theme '{theme}' values must be an array")))?;
    if items.len() != n_faces {
        return Err(err(format!(
            "texture theme '{theme}' has {} values but geometry has {n_faces} faces",
            items.len()
        )));
    }
    items
        .iter()
        .map(|face| {
            face.as_array()
                .ok_or_else(|| err(format!("texture theme '{theme}' face must be an array")))?
                .iter()
                .map(|ring| parse_ring_leaf(ring, textures))
                .collect::<Result<Vec<_>>>()
        })
        .collect()
}

/// One geometry's per-theme flat `[face][ring]` textures (global id + UVs,
/// `None` = untextured ring): each theme's `values` list is one entry per WKB
/// face, in face-walk order, and each face entry is an array of its rings'
/// leaves, of length `n_faces` exactly.
pub fn texture_face_maps(
    texture_map: &Value,
    n_faces: usize,
    textures: &HashMap<i64, Value>,
) -> Result<TextureFaceMaps> {
    let obj = texture_map
        .as_object()
        .ok_or_else(|| err("texture map must be a JSON object of theme -> {values}"))?;
    let mut out = BTreeMap::new();
    for (theme, inner) in obj {
        let values = inner
            .as_object()
            .and_then(|o| o.get("values"))
            .ok_or_else(|| err(format!("texture theme '{theme}' is missing 'values'")))?;
        let faces = parse_texture_values(theme, values, n_faces, textures)?;
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

fn parse_ring_leaf(
    v: &Value,
    textures: &HashMap<i64, Value>,
) -> Result<Option<(usize, Vec<[f64; 2]>)>> {
    let a = v
        .as_array()
        .ok_or_else(|| err("texture ring leaf must be an array"))?;
    match a.first() {
        Some(Value::Number(n)) => {
            let tex = n
                .as_u64()
                .ok_or_else(|| err("texture id is not an integer"))? as usize;
            if !textures.contains_key(&(tex as i64)) {
                return Err(err(format!(
                    "texture id {tex} does not name a row in textures.parquet"
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
    material_table: &HashMap<i64, Value>,
    texture_table: &HashMap<i64, Value>,
    report: &mut WriteReport,
) -> Result<()> {
    let themes: BTreeSet<&String> = materials
        .themes
        .keys()
        .chain(textures.themes.keys())
        .collect();
    for theme in themes {
        w.write_event(Event::Start(BytesStart::new("app:appearance")))?;
        w.write_event(Event::Start(BytesStart::new("app:Appearance")))?;
        // The empty-string theme round-trips to an ABSENT app:theme.
        if !theme.is_empty() {
            text_elem(w, "app:theme", theme)?;
        }
        if let Some(mats) = materials.themes.get(theme) {
            for (gid, targets) in mats {
                let def = material_table.get(&(*gid as i64)).ok_or_else(|| {
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
                let def = texture_table.get(&(*tid as i64)).ok_or_else(|| {
                    err(format!(
                        "texture global id {tid} out of range (table length {})",
                        texture_table.len()
                    ))
                })?;
                write_parameterized_texture(w, def, polys)?;
                report.textures_written += 1;
            }
        }
        w.write_event(Event::End(BytesEnd::new("app:Appearance")))?;
        w.write_event(Event::End(BytesEnd::new("app:appearance")))?;
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
    w.write_event(Event::Start(BytesStart::new("app:surfaceDataMember")))?;
    w.write_event(Event::Start(BytesStart::new("app:ParameterizedTexture")))?;

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
        w.write_event(Event::Start(target))?;
        w.write_event(Event::Start(BytesStart::new("app:TexCoordList")))?;
        for (ringid, uvs) in rings {
            let mut tc = BytesStart::new("app:textureCoordinates");
            tc.push_attribute(("ring", format!("#{ringid}").as_str()));
            w.write_event(Event::Start(tc))?;
            w.write_event(Event::Text(BytesText::new(&closed_uvs(uvs))))?;
            w.write_event(Event::End(BytesEnd::new("app:textureCoordinates")))?;
        }
        w.write_event(Event::End(BytesEnd::new("app:TexCoordList")))?;
        w.write_event(Event::End(BytesEnd::new("app:target")))?;
    }

    w.write_event(Event::End(BytesEnd::new("app:ParameterizedTexture")))?;
    w.write_event(Event::End(BytesEnd::new("app:surfaceDataMember")))?;
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
    w.write_event(Event::Start(BytesStart::new("app:surfaceDataMember")))?;
    w.write_event(Event::Start(BytesStart::new("app:X3DMaterial")))?;

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

    w.write_event(Event::End(BytesEnd::new("app:X3DMaterial")))?;
    w.write_event(Event::End(BytesEnd::new("app:surfaceDataMember")))?;
    Ok(())
}

/// A `<tag>text</tag>` element (text auto-escaped by quick-xml).
fn text_elem<W: Write>(w: &mut Writer<W>, tag: &str, text: &str) -> Result<()> {
    w.write_event(Event::Start(BytesStart::new(tag)))?;
    w.write_event(Event::Text(BytesText::new(text)))?;
    w.write_event(Event::End(BytesEnd::new(tag)))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn face_maps_read_the_flat_per_face_shape() {
        let mut materials = HashMap::new();
        materials.insert(3i64, serde_json::json!({"name": "m"}));
        let m = material_face_maps(
            &serde_json::json!({"": {"values": [3, null, 3]}}),
            3,
            &materials,
        )
        .unwrap();
        assert_eq!(m[""], vec![Some(3), None, Some(3)]);
        assert!(
            material_face_maps(&serde_json::json!({"": {"values": [3]}}), 3, &materials).is_err()
        );
        // `material_face_maps` only accepts the flat per-face list — a
        // nested per-shell tree (what a `Solid`'s `values` would look like
        // before flattening) must be rejected, not silently accepted as a
        // one-shell geometry.
        assert!(
            material_face_maps(
                &serde_json::json!({"": {"values": [[3, null, 3]]}}),
                3,
                &materials
            )
            .is_err()
        );

        let mut textures = HashMap::new();
        textures.insert(7i64, serde_json::json!({"type": "PNG", "image": "a.png"}));
        let t = texture_face_maps(&serde_json::json!({"": {"values": [ [ [7, [0.0, 0.0], [1.0, 0.0], [1.0, 1.0]], [null] ], [ [null] ] ]}}), 2, &textures).unwrap();
        assert_eq!(t[""].len(), 2);
        assert_eq!(t[""][0][0].as_ref().unwrap().0, 7);
        assert!(t[""][0][1].is_none() && t[""][1][0].is_none());
        // A theme whose face list is shorter than the geometry is refused, as
        // the material side refuses a short `values`: padding it silently
        // would leave the trailing faces unaddressed by `app:target`.
        assert!(
            texture_face_maps(
                &serde_json::json!({"": {"values": [ [ [null] ] ]}}),
                2,
                &textures
            )
            .is_err()
        );
    }
}
