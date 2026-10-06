//! The native visit shared by the three runners whose records are CityJSON
//! city objects once parsed: `cityjson` (one document), `cityjsonseq` (one
//! feature per line) and `citygml` (members streamed into CityJSON
//! features by the reader).
//!
//! The visit follows the return rule of READ_BENCHMARK.md:
//!
//! - [`visit_object`] reads every field of one city object: every vertex of
//!   every geometry resolved through `vertices` + `transform` into a real
//!   coordinate, every ring, every per-face semantic reference, every
//!   semantic-surface object and every attribute value. Appearance is not
//!   read, and a `GeometryInstance` is read by its anchor and transformation
//!   only (its template is not expanded).
//! - [`visit_highest_lod`] walks the coordinates of the object's most
//!   detailed geometry in place, which is what the spatial window returns.
//!
//! The comparable totals follow `cityparquet::visit::VisitTotals`:
//! `geometries` counts the explicit geometries (a `GeometryInstance` lives in
//! CityParquet's `template` column, not in a `geometry_lod*` column, so it is
//! not counted), `semantic_faces` counts the non-null `semantics.values`
//! leaves, and `extent` covers every resolved vertex of a counted geometry.

use std::collections::{HashMap, HashSet};

use anyhow::{Result, anyhow, bail};
use cityparquet::cjseq::{CityObject, Geometry, GeometryType, Transform};
use serde_json::Value;

use super::returned::{ComparableTotals, IdDigest, ReturnedGeometry};

/// A document's or feature's vertex array with the transform that turns its
/// integer vertices into real coordinates.
#[derive(Clone, Copy)]
pub(super) struct Vertices<'a> {
    pub vertices: &'a [Vec<i64>],
    pub transform: &'a Transform,
}

impl Vertices<'_> {
    fn coordinate(&self, index: usize) -> Result<[f64; 3]> {
        let vertex = self.vertices.get(index).ok_or_else(|| {
            anyhow!(
                "geometry boundary references vertex {index}, but there are only {}",
                self.vertices.len()
            )
        })?;
        if vertex.len() < 3 {
            bail!("vertex {index} has {} components, expected 3", vertex.len());
        }
        let t = self.transform;
        Ok([
            vertex[0] as f64 * t.scale[0] + t.translate[0],
            vertex[1] as f64 * t.scale[1] + t.translate[1],
            vertex[2] as f64 * t.scale[2] + t.translate[2],
        ])
    }

    /// Resolves every leaf of a `boundaries` tree and folds it into `extent`.
    /// Returns the number of rings walked (arrays whose items are indices).
    pub fn walk(&self, value: &Value, extent: &mut [f64; 6]) -> Result<u64> {
        match value {
            Value::Array(items) => {
                let mut rings = 0u64;
                let mut is_ring = false;
                for item in items {
                    if item.is_number() {
                        is_ring = true;
                    }
                    rings += self.walk(item, extent)?;
                }
                Ok(rings + u64::from(is_ring))
            }
            Value::Number(number) => {
                let index = number.as_u64().ok_or_else(|| {
                    anyhow!("geometry boundary index '{number}' is not a whole number")
                })? as usize;
                let c = self.coordinate(index)?;
                for axis in 0..3 {
                    extent[axis] = extent[axis].min(c[axis]);
                    extent[axis + 3] = extent[axis + 3].max(c[axis]);
                }
                Ok(0)
            }
            other => bail!("unexpected value in geometry boundaries: {other}"),
        }
    }
}

/// Reads every leaf of a JSON value (an attribute, a semantic-surface
/// object) and returns how many there were, so the read cannot be skipped.
pub(super) fn touch(value: &Value) -> u64 {
    match value {
        Value::Array(items) => items.iter().map(touch).sum(),
        Value::Object(map) => map
            .iter()
            .map(|(k, v)| {
                std::hint::black_box(k.as_str());
                touch(v)
            })
            .sum(),
        Value::String(s) => {
            std::hint::black_box(s.as_str());
            1
        }
        Value::Number(n) => {
            std::hint::black_box(n.as_f64());
            1
        }
        Value::Bool(b) => {
            std::hint::black_box(*b);
            1
        }
        Value::Null => 0,
    }
}

/// The non-null leaves of a `semantics.values` tree: the faces carrying a
/// semantic-surface reference.
fn semantic_faces(value: &Value) -> u64 {
    match value {
        Value::Array(items) => items.iter().map(semantic_faces).sum(),
        Value::Number(n) => {
            std::hint::black_box(n.as_u64());
            1
        }
        _ => 0,
    }
}

fn is_instance(geometry: &Geometry) -> bool {
    matches!(geometry.thetype, GeometryType::GeometryInstance)
}

/// Reads every field of one city object into `totals` (see the module
/// documentation for what is read and what is counted).
pub(super) fn visit_object(
    co: &CityObject,
    verts: Vertices<'_>,
    totals: &mut ComparableTotals,
) -> Result<()> {
    totals.objects += 1;
    std::hint::black_box(co.thetype.as_str());
    let mut leaves = 0u64;
    if let Some(attributes) = &co.attributes {
        leaves += touch(attributes);
    }
    if let Some(extent) = &co.geographical_extent {
        std::hint::black_box(extent.as_slice());
    }
    for id in co
        .children
        .iter()
        .flatten()
        .chain(co.parents.iter().flatten())
    {
        std::hint::black_box(id.as_str());
    }
    for geometry in co.geometry.iter().flatten() {
        std::hint::black_box(geometry.lod.as_deref());
        if is_instance(geometry) {
            // Anchor and transformation only; the template is not expanded.
            std::hint::black_box(geometry.template);
            leaves += touch(&geometry.boundaries);
            if let Some(matrix) = &geometry.transformation_matrix {
                leaves += touch(matrix);
            }
            continue;
        }
        totals.geometries += 1;
        leaves += verts.walk(&geometry.boundaries, &mut totals.extent)?;
        if let Some(semantics) = &geometry.semantics {
            if let Some(values) = semantics.get("values") {
                totals.semantic_faces += semantic_faces(values);
            }
            if let Some(surfaces) = semantics.get("surfaces") {
                leaves += touch(surfaces);
            }
        }
    }
    std::hint::black_box(leaves);
    Ok(())
}

/// `(major, minor)` of a CityJSON `lod` string (`"2.2"` → `(2, 2)`, `"1"` →
/// `(1, 0)`), so LoDs order numerically as CityParquet's columns do.
pub(super) fn lod_rank(lod: Option<&str>) -> (u32, u32) {
    let Some(lod) = lod else { return (0, 0) };
    let mut parts = lod.splitn(2, '.');
    let major = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
    let minor = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
    (major, minor)
}

/// The object's most detailed explicit geometry, if it has one.
fn highest_lod(co: &CityObject) -> Option<&Geometry> {
    co.geometry
        .iter()
        .flatten()
        .filter(|g| !is_instance(g))
        .max_by_key(|g| lod_rank(g.lod.as_deref()))
}

/// Walks the coordinates of the object's highest-LoD geometry in place into
/// `out`; an object without an explicit geometry adds nothing.
pub(super) fn visit_highest_lod(
    co: &CityObject,
    verts: Vertices<'_>,
    out: &mut ReturnedGeometry,
) -> Result<()> {
    if let Some(geometry) = highest_lod(co) {
        out.geometries += 1;
        std::hint::black_box(verts.walk(&geometry.boundaries, &mut out.extent)?);
    }
    Ok(())
}

/// An object's box, `(min, max)`, or `None` when it has no geometry.
pub(super) type MaybeBox = Option<([f64; 3], [f64; 3])>;

/// One object's box over its own geometries (instances included only by
/// their resolved boundaries, as before: a `GeometryInstance` anchor is a
/// vertex of the document).
fn own_box(co: &CityObject, verts: Vertices<'_>) -> Result<MaybeBox> {
    let mut e = super::returned::EMPTY_EXTENT;
    let mut any = false;
    for geometry in co.geometry.iter().flatten() {
        verts.walk(&geometry.boundaries, &mut e)?;
        any |= e[0].is_finite();
    }
    Ok(any.then_some(([e[0], e[1], e[2]], [e[3], e[4], e[5]])))
}

pub(super) fn union(a: MaybeBox, b: MaybeBox) -> MaybeBox {
    match (a, b) {
        (None, x) | (x, None) => x,
        (Some((amin, amax)), Some((bmin, bmax))) => Some((
            std::array::from_fn(|i| amin[i].min(bmin[i])),
            std::array::from_fn(|i| amax[i].max(bmax[i])),
        )),
    }
}

/// Every object's box over its whole `children` subtree, memoised and
/// guarded against a cycle in a malformed hierarchy.
pub(super) fn subtree_boxes<'a>(
    objects: &'a HashMap<String, CityObject>,
    verts: Vertices<'_>,
) -> Result<HashMap<&'a str, MaybeBox>> {
    let own: HashMap<&str, MaybeBox> = objects
        .iter()
        .map(|(id, co)| Ok((id.as_str(), own_box(co, verts)?)))
        .collect::<Result<_>>()?;
    fn rec<'a>(
        id: &'a str,
        objects: &'a HashMap<String, CityObject>,
        own: &HashMap<&'a str, MaybeBox>,
        memo: &mut HashMap<&'a str, MaybeBox>,
        path: &mut HashSet<&'a str>,
    ) -> MaybeBox {
        if let Some(done) = memo.get(id) {
            return *done;
        }
        let (key, co) = objects.get_key_value(id)?;
        let key = key.as_str();
        path.insert(key);
        let mut acc = own.get(key).copied().flatten();
        for child in co.children.iter().flatten() {
            if path.contains(child.as_str()) {
                continue;
            }
            if let Some((child_key, _)) = objects.get_key_value(child.as_str()) {
                acc = union(acc, rec(child_key.as_str(), objects, own, memo, path));
            }
        }
        path.remove(key);
        memo.insert(key, acc);
        acc
    }
    let mut memo = HashMap::with_capacity(objects.len());
    for id in objects.keys() {
        rec(id, objects, &own, &mut memo, &mut HashSet::new());
    }
    Ok(memo)
}

/// The spatial window over one set of objects: every object whose subtree
/// box intersects `window` (edges included) has its id folded into `ids` and
/// its highest-LoD geometry walked into `out`.
pub(super) fn window_objects(
    objects: &HashMap<String, CityObject>,
    verts: Vertices<'_>,
    window: &[f64; 6],
    ids: &mut IdDigest,
    out: &mut ReturnedGeometry,
) -> Result<()> {
    let boxes = subtree_boxes(objects, verts)?;
    for (id, co) in objects {
        if let Some(Some((min, max))) = boxes.get(id.as_str())
            && super::cityjsonseq::intersects(*min, *max, window)
        {
            ids.push(id);
            visit_highest_lod(co, verts, out)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lods_order_numerically() {
        assert!(lod_rank(Some("2.2")) > lod_rank(Some("1.3")));
        assert!(lod_rank(Some("1.10")) > lod_rank(Some("1.3")));
        assert!(lod_rank(Some("1.3")) > lod_rank(Some("0")));
        assert_eq!(lod_rank(Some("2")), (2, 0));
    }

    #[test]
    fn semantic_faces_skip_nulls() {
        let v: Value = serde_json::json!([[0, 1, null, 2], [null]]);
        assert_eq!(semantic_faces(&v), 3);
    }
}
