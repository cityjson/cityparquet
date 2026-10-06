//! Remove geometries from a CityJSONSeq feature or a CityJSON document by
//! LoD: every geometry of the given LoDs ([`drop_lods`]), or every geometry
//! after the first at the same LoD on one CityObject ([`keep_first_per_lod`]).
//!
//! `fcb-slice` uses [`drop_lods`] to cut the 3DBAG slice without LoD 1.2.
//! CityGML 2.0 has integer LoDs only, so `citygml-tools from-cityjson` keeps
//! one LoD-1 solid per object (the 1.3 one) and drops the other: a slice
//! carrying both would give the CityGML artefact less geometry than the other
//! four formats. Without LoD 1.2, all five hold LoD 0, 1.3 and 2.2.
//!
//! The result stays a valid, compact feature. Every CityObject is kept —
//! one left with no geometry still counts as an object and carries its
//! attributes — and so is each remaining geometry's semantics and
//! appearance. Vertices no longer referenced by any remaining geometry are
//! removed and the remaining boundaries re-indexed, so the slice carries no
//! orphan coordinates a text format would pay for. A feature that loses
//! nothing is returned byte for byte.
//!
//! [`keep_first_per_lod`] normalises the benchmark corpus. CityParquet stores
//! one geometry column per LoD, so its writer keeps the FIRST geometry at a
//! given LoD on an object and counts the rest in
//! `ConvertReport::skipped_same_lod_geometries`; every corpus dataset is
//! normalised the same way before any artefact is built, so all five formats
//! hold the same geometries. "The same LoD" is the writer's key exactly: the
//! `lod` string canonicalised by `Lod::parse` (`"1"` and `"1.0"` are one LoD)
//! and named by its column suffix. A geometry the writer does not key is
//! never dropped and never claims a LoD: a `GeometryInstance` (stored in the
//! `template` column, not a LoD column), a geometry whose boundaries hold no
//! vertex index (the writer stores nothing for it), and one without a
//! parseable `lod`. Texture coordinates (`vertices-texture`) and the
//! `appearance` arrays are left as they are: the kept geometries' `material`
//! and `texture` indices still resolve, and a dropped geometry's texture
//! coordinates remain in the shared array.

use std::collections::HashSet;

use anyhow::{Context, Result, bail};
use cityparquet_schema::types::Lod;
use serde_json::Value;

/// What [`keep_first_per_lod`] did to one feature or document.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Normalised {
    /// Geometries removed: a second (or later) geometry at a LoD an object
    /// already holds.
    pub dropped: usize,
    pub geometries_before: usize,
    pub geometries_after: usize,
    pub vertices_before: usize,
    pub vertices_after: usize,
}

impl std::ops::AddAssign for Normalised {
    fn add_assign(&mut self, other: Self) {
        self.dropped += other.dropped;
        self.geometries_before += other.geometries_before;
        self.geometries_after += other.geometries_after;
        self.vertices_before += other.vertices_before;
        self.vertices_after += other.vertices_after;
    }
}

/// Leaves every CityObject of `doc` — a CityJSONFeature or a whole CityJSON
/// document, both of which hold `CityObjects` and the `vertices` their
/// boundaries index — with at most one geometry per LoD, the first in source
/// order. Vertices no longer referenced are removed and the boundaries
/// re-indexed; nothing else changes. `doc` is untouched when nothing is
/// dropped.
pub fn keep_first_per_lod(doc: &mut Value) -> Result<Normalised> {
    let vertices = doc
        .get("vertices")
        .and_then(Value::as_array)
        .context("a CityJSON document or feature has a `vertices` array")?
        .len();
    let objects = doc
        .get_mut("CityObjects")
        .and_then(Value::as_object_mut)
        .context("a CityJSON document or feature has a `CityObjects` object")?;

    let mut report = Normalised {
        vertices_before: vertices,
        vertices_after: vertices,
        ..Normalised::default()
    };
    for object in objects.values_mut() {
        let Some(geometries) = object.get_mut("geometry").and_then(Value::as_array_mut) else {
            continue;
        };
        report.geometries_before += geometries.len();
        let mut seen = HashSet::new();
        geometries.retain(|g| match writer_lod_key(g) {
            Some(key) => seen.insert(key),
            None => true,
        });
        report.geometries_after += geometries.len();
    }
    report.dropped = report.geometries_before - report.geometries_after;
    if report.dropped > 0 {
        compact_vertices(doc)?;
        report.vertices_after = doc["vertices"].as_array().map_or(0, Vec::len);
    }
    Ok(report)
}

/// [`keep_first_per_lod`] on one serialised feature (or document): the line
/// as given when nothing is dropped, otherwise the compact re-serialisation.
pub fn keep_first_per_lod_line(line: &str) -> Result<(String, Normalised)> {
    let mut doc: Value = serde_json::from_str(line).context("parsing a CityJSON feature")?;
    let report = keep_first_per_lod(&mut doc)?;
    if report.dropped == 0 {
        return Ok((line.to_string(), report));
    }
    let out = serde_json::to_string(&doc).context("serialising the feature")?;
    Ok((out, report))
}

/// The LoD column a geometry would be stored in by the CityParquet writer,
/// or `None` for a geometry the writer does not store in one.
fn writer_lod_key(geometry: &Value) -> Option<String> {
    if geometry.get("type").and_then(Value::as_str) == Some("GeometryInstance") {
        return None;
    }
    if !has_vertex_index(geometry.get("boundaries")?) {
        return None;
    }
    let lod = match geometry.get("lod")? {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => return None,
    };
    Lod::parse(&lod).ok().map(|lod| lod.column_suffix())
}

fn has_vertex_index(value: &Value) -> bool {
    match value {
        Value::Array(items) => items.iter().any(has_vertex_index),
        Value::Number(_) => true,
        _ => false,
    }
}

/// `feature_line` with every geometry whose `lod` is one of `lods` removed.
///
/// `lods` match the CityJSON `lod` value exactly, as a string (`"1.2"`); an
/// empty list, or a feature carrying none of them, returns the line as given.
pub fn drop_lods(feature_line: &str, lods: &[String]) -> Result<String> {
    if lods.is_empty() {
        return Ok(feature_line.to_string());
    }
    let mut feature: Value =
        serde_json::from_str(feature_line).context("parsing a CityJSONSeq feature line")?;
    let objects = feature
        .get_mut("CityObjects")
        .and_then(Value::as_object_mut)
        .context("a CityJSONSeq feature has a `CityObjects` object")?;

    let mut dropped = false;
    for object in objects.values_mut() {
        if let Some(geometries) = object.get_mut("geometry").and_then(Value::as_array_mut) {
            let before = geometries.len();
            geometries.retain(|g| !lods.iter().any(|lod| lod_matches(g, lod)));
            dropped |= geometries.len() != before;
        }
    }
    if !dropped {
        return Ok(feature_line.to_string());
    }

    compact_vertices(&mut feature)?;
    serde_json::to_string(&feature).context("serialising the feature")
}

fn lod_matches(geometry: &Value, lod: &str) -> bool {
    match geometry.get("lod") {
        Some(Value::String(s)) => s == lod,
        Some(Value::Number(n)) => n.to_string() == lod,
        _ => false,
    }
}

/// Keeps only the vertices the remaining boundaries reference, in their
/// original order, and rewrites every boundary index to its new position.
fn compact_vertices(feature: &mut Value) -> Result<()> {
    let count = feature
        .get("vertices")
        .and_then(Value::as_array)
        .context("a CityJSON document or feature has a `vertices` array")?
        .len();
    let mut used = vec![false; count];
    for boundaries in boundaries_mut(feature) {
        mark(boundaries, &mut used)?;
    }
    let mut remap = vec![u64::MAX; count];
    let mut next = 0u64;
    for (old, keep) in used.iter().enumerate() {
        if *keep {
            remap[old] = next;
            next += 1;
        }
    }
    for boundaries in boundaries_mut(feature) {
        rewrite(boundaries, &remap);
    }
    let vertices = feature
        .get_mut("vertices")
        .and_then(Value::as_array_mut)
        .expect("checked above");
    let mut index = 0;
    vertices.retain(|_| {
        let keep = used[index];
        index += 1;
        keep
    });
    Ok(())
}

/// Every remaining geometry's `boundaries`, across all CityObjects
/// (instances included: their one boundary index is a vertex too).
fn boundaries_mut(feature: &mut Value) -> Vec<&mut Value> {
    feature
        .get_mut("CityObjects")
        .and_then(Value::as_object_mut)
        .into_iter()
        .flat_map(|objects| objects.values_mut())
        .filter_map(|object| object.get_mut("geometry").and_then(Value::as_array_mut))
        .flat_map(|geometries| geometries.iter_mut())
        .filter_map(|geometry| geometry.get_mut("boundaries"))
        .collect()
}

fn mark(value: &Value, used: &mut [bool]) -> Result<()> {
    match value {
        Value::Array(items) => items.iter().try_for_each(|v| mark(v, used)),
        Value::Number(n) => {
            let index = n
                .as_u64()
                .context("a boundary index is a non-negative integer")?
                as usize;
            match used.get_mut(index) {
                Some(slot) => *slot = true,
                None => bail!("boundary index {index} is outside the feature's vertices"),
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn rewrite(value: &mut Value, remap: &[u64]) {
    match value {
        Value::Array(items) => items.iter_mut().for_each(|v| rewrite(v, remap)),
        Value::Number(n) => {
            let old = n.as_u64().expect("validated by `mark`") as usize;
            *value = Value::from(remap[old]);
        }
        _ => {}
    }
}
