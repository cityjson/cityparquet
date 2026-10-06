//! Remove every geometry of the given LoDs from one CityJSONSeq feature.
//!
//! `fcb-slice` uses this to cut the 3DBAG slice without LoD 1.2.
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

use anyhow::{Context, Result, bail};
use serde_json::Value;

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
        .context("a CityJSONSeq feature has a `vertices` array")?
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

/// Every remaining geometry's `boundaries`, across all CityObjects.
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
