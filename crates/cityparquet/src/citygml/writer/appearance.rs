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

use std::collections::BTreeMap;
use std::io::Write;

use cityparquet_schema::CityParquetError;
use quick_xml::Writer;
use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use serde_json::Value;

use super::WriteReport;
use crate::Result;
use crate::wkb_read::DecodedKind;

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

/// Emit one `app:appearance/app:Appearance` per theme (each with its used
/// `app:X3DMaterial`s and their `app:target`s), from the accumulated targets and
/// the global materials table. `report.materials_written` counts each emitted
/// `app:X3DMaterial`.
pub fn write_appearance<W: Write>(
    w: &mut Writer<W>,
    acc: &AppearanceAcc,
    table: &[Value],
    report: &mut WriteReport,
) -> Result<()> {
    for (theme, mats) in &acc.themes {
        w.write_event(Event::Start(BytesStart::new("app:appearance")))
            .map_err(io_err)?;
        w.write_event(Event::Start(BytesStart::new("app:Appearance")))
            .map_err(io_err)?;
        // The empty-string theme round-trips to an ABSENT app:theme.
        if !theme.is_empty() {
            w.write_event(Event::Start(BytesStart::new("app:theme")))
                .map_err(io_err)?;
            w.write_event(Event::Text(BytesText::new(theme)))
                .map_err(io_err)?;
            w.write_event(Event::End(BytesEnd::new("app:theme")))
                .map_err(io_err)?;
        }
        for (gid, targets) in mats {
            let def = table.get(*gid).ok_or_else(|| {
                err(format!(
                    "material global id {gid} out of range (table length {})",
                    table.len()
                ))
            })?;
            write_x3d_material(w, def, targets)?;
            report.materials_written += 1;
        }
        w.write_event(Event::End(BytesEnd::new("app:Appearance")))
            .map_err(io_err)?;
        w.write_event(Event::End(BytesEnd::new("app:appearance")))
            .map_err(io_err)?;
    }
    Ok(())
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
