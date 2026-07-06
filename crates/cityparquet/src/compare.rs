//! Semantic-equality comparator: proves a CityParquet round-trip
//! (source CityJSON/CityJSONSeq -> package -> exported CityJSON/CityJSONSeq)
//! is lossless at the semantic level, rather than byte-for-byte.
//!
//! Both `a` and `b` are opened as plain [`crate::source::Source`]s — the
//! exported file is itself valid CityJSON/CityJSONSeq, so no special-casing
//! is needed for "the export side". Every `CityObject` across every feature
//! is flattened into one `id -> ObjectData` map per side (ids are globally
//! unique and preserved verbatim end-to-end), so feature grouping/ordering
//! differences between the two sides are irrelevant to the comparison.
//!
//! Degenerate-ring normalisation (stripping a ring's trailing duplicates of
//! its first vertex INDEX; dropping rings left with fewer than 3 ENTRIES;
//! dropping a surface whose exterior ring was dropped) is reimplemented here
//! independently of [`crate::wkb_write`]'s writer-side normalisation — on
//! purpose: a comparator that reused the writer's own normalisation function
//! could not catch a bug in that function, since both sides would share the
//! same blind spot. The "fewer than 3" threshold counts remaining ring
//! ELEMENTS, not geometrically distinct coordinates: a 3-element zero-area
//! ring passes through untouched — the CityJSON spec's stricter "at least 3
//! distinct vertices" wording is deliberately not enforced beyond this
//! structural element count (data quality is not the format's business,
//! matching the writer's policy). It is applied to BOTH sides
//! unconditionally; on the export side (whose degenerate rings were already
//! dropped at write time) this is a no-op by construction.
//!
//! `material`/`texture` blocks ARE part of the comparison by default: they
//! compare under the same JSON equality as `semantics`, after the same
//! surface-index realignment the degenerate normalisation applies. Only
//! `Exclusions::appearance` turns that off (skip + `excluded` log), because
//! the Core-profile exporter deliberately drops the blocks it cannot safely
//! re-attach.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use chrono::{DateTime, NaiveDate};
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use cityparquet_schema::{CityParquetError, Result};
use cjseq::{CityJSON, Geometry, GeometryType, Transform};

use crate::source::Source;
use crate::wkb_write::VertexPool;

/// Options controlling one [`compare_datasets`] call.
#[derive(Debug, Clone, Copy, Default)]
pub struct CompareOptions {
    /// Per-axis coordinate tolerance. A component of `0.0` (the `Default`
    /// value) means "derive it": the larger of the two sides' own transform
    /// scale on that axis, since neither side can be dequantised more
    /// precisely than its own scale.
    pub coord_tolerance: [f64; 3],
    pub exclusions: Exclusions,
}

/// What the comparator should skip rather than flag as a difference, because
/// the exporter deliberately drops it (see `export`'s module docs) and the
/// dropped content lives in data this pass does not read.
#[derive(Debug, Clone, Copy, Default)]
pub struct Exclusions {
    /// Skip material/texture presence on the source side.
    pub appearance: bool,
    /// Skip `GeometryInstance` geometries on the source side.
    pub geometry_instances: bool,
}

/// Outcome of one [`compare_datasets`] call.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CompareReport {
    pub equal: bool,
    /// Human-readable, object-id-prefixed. Capped at 50 entries; beyond that
    /// a final `"... N more differences truncated"` entry is appended.
    pub differences: Vec<String>,
    /// What was skipped and why (degenerate-ring normalisation,
    /// `exclusions`-driven skips, and writer's first-geometry-per-LoD rule).
    pub excluded: Vec<String>,
}

fn err(msg: impl Into<String>) -> CityParquetError {
    CityParquetError::Schema(msg.into())
}

const MAX_DIFFERENCES: usize = 50;

// ---------------------------------------------------------------------
// Coordinate tree: a generic nested-list shape with dequantised leaves.
// Comparing two trees checks list-length (ring/face/shell counts) at every
// level and coordinate closeness at the leaves — exactly the brief's
// "boundary tree shape" + "coordinates within tolerance" requirement.
// ---------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Node {
    Point([f64; 3]),
    List(Vec<Node>),
}

fn node_is_empty(n: &Node) -> bool {
    match n {
        Node::Point(_) => false,
        Node::List(v) => v.is_empty() || v.iter().all(node_is_empty),
    }
}

fn node_matches(a: &Node, b: &Node, tol: [f64; 3]) -> bool {
    match (a, b) {
        (Node::Point(pa), Node::Point(pb)) => (0..3).all(|i| (pa[i] - pb[i]).abs() <= tol[i]),
        (Node::List(la), Node::List(lb)) => {
            la.len() == lb.len() && la.iter().zip(lb).all(|(x, y)| node_matches(x, y, tol))
        }
        _ => false,
    }
}

fn points_node(pool: &VertexPool, idxs: &[usize]) -> Result<Node> {
    Ok(Node::List(
        idxs.iter()
            .map(|&i| Ok(Node::Point(pool.coord(i)?)))
            .collect::<Result<Vec<_>>>()?,
    ))
}

fn ring_list_node(pool: &VertexPool, rings: &[Vec<usize>]) -> Result<Node> {
    Ok(Node::List(
        rings
            .iter()
            .map(|r| points_node(pool, r))
            .collect::<Result<Vec<_>>>()?,
    ))
}

fn surface_list_node(pool: &VertexPool, surfaces: &[Vec<Vec<usize>>]) -> Result<Node> {
    Ok(Node::List(
        surfaces
            .iter()
            .map(|s| ring_list_node(pool, s))
            .collect::<Result<Vec<_>>>()?,
    ))
}

// ---------------------------------------------------------------------
// Degenerate-ring normalisation, reimplemented independently of
// crate::wkb_write (see module docs for why).
// ---------------------------------------------------------------------

/// Strip trailing duplicates of the first vertex INDEX (a pre-baked WKB
/// closure seen in the wild); drop the ring ([`None`]) if fewer than 3
/// vertices remain.
///
/// Loops to a fixpoint rather than stripping a single trailing duplicate:
/// the writer (`crate::wkb_write::normalise_ring`) only strips one, which is
/// enough for its own output to satisfy the reader's ring-closure check, but
/// is NOT idempotent for a ring the source doubly-closed (e.g.
/// `[a,b,b,a,a]`, seen in the railway fixture) — stripping once there still
/// leaves a closed ring (`[a,b,b,a]`). Since this same normalisation is
/// applied independently to BOTH the raw source ring and the writer's
/// already-once-stripped output, it must converge to the identical result
/// from either starting point, or a doubly-closed ring would compare as a
/// false difference. Looping to a fixpoint guarantees that convergence
/// regardless of how many redundant closures the source (or the writer)
/// left in place, while remaining a no-op for the common single-closure
/// case.
///
/// This fixpoint loop is therefore DELIBERATELY more lenient than the
/// writer's single-strip contract: the writer only ever needs to undo one
/// pre-baked closure (its output feeds the WKB reader, which re-checks
/// closure itself), whereas the comparator's job is to decide whether two
/// differently-normalised encodings of the same source ring denote the same
/// ring — which requires an idempotent canonical form, not a faithful
/// replay of the writer's one pass.
fn normalise_ring(ring: &[usize]) -> Option<Vec<usize>> {
    let mut stripped = ring;
    while stripped.len() >= 2 && stripped.first() == stripped.last() {
        stripped = &stripped[..stripped.len() - 1];
    }
    (stripped.len() >= 3).then(|| stripped.to_vec())
}

/// Normalise one surface's rings, counting every dropped ring (interior or
/// exterior) into `dropped_rings`. Returns `None` (surface dropped entirely)
/// when the EXTERIOR ring (index 0) was dropped — interior rings cannot
/// stand without it.
fn normalise_surface(rings: &[Vec<usize>], dropped_rings: &mut usize) -> Option<Vec<Vec<usize>>> {
    let mut kept = Vec::with_capacity(rings.len());
    let mut exterior_dropped = false;
    for (i, ring) in rings.iter().enumerate() {
        match normalise_ring(ring) {
            Some(r) => kept.push(r),
            None => {
                *dropped_rings += 1;
                if i == 0 {
                    exterior_dropped = true;
                }
            }
        }
    }
    if exterior_dropped { None } else { Some(kept) }
}

/// Removes the entries at `dropped` (original positions) from a per-surface
/// JSON array, in place. Positions beyond the array are ignored (defensive:
/// a source `semantics.values` shorter than the boundaries).
fn remove_dropped_entries(values: &mut Vec<Value>, dropped: &[usize]) {
    for &pos in dropped.iter().rev() {
        if pos < values.len() {
            values.remove(pos);
        }
    }
}

/// Result of normalising+dequantising one geometry: its coordinate tree, the
/// realigned `semantics` and `material`/`texture` (only realigned for the
/// surface-list types — the solid types nest their per-surface arrays per
/// shell, which the writer itself leaves unrealigned; see `crate::encode`'s
/// `drops_align_with_surface_arrays`), and how much was normalised away
/// (for the `excluded` log).
struct NormalisedGeometry {
    tree: Node,
    semantics: Option<Value>,
    material: Option<Value>,
    texture: Option<Value>,
    dropped_rings: usize,
    dropped_surfaces: usize,
}

fn parse_boundaries<T: DeserializeOwned>(geom: &Geometry) -> Result<T> {
    serde_json::from_value(geom.boundaries.clone()).map_err(|e| {
        err(format!(
            "boundaries do not match the expected shape for {:?}: {e}",
            geom.thetype
        ))
    })
}

fn realign_semantics(semantics: &Option<Value>, dropped_surfaces: &[usize]) -> Option<Value> {
    let mut semantics = semantics.clone()?;
    if !dropped_surfaces.is_empty()
        && let Some(values) = semantics.get_mut("values").and_then(Value::as_array_mut)
    {
        remove_dropped_entries(values, dropped_surfaces);
    }
    Some(semantics)
}

/// One geometry's `material` or `texture` map as a JSON value with each
/// theme's per-surface `values` array realigned for the surfaces the
/// degenerate normalisation dropped (mirrors `crate::encode`'s
/// `realign_appearance_themes`). Theme-level scalar `value` entries apply
/// to all surfaces and need no realignment. `dropped_surfaces` must be
/// empty for the non-surface-list types (whose per-surface arrays nest per
/// shell and are left unrealigned, matching the writer).
fn realigned_appearance<T: serde::Serialize>(
    map: &Option<T>,
    dropped_surfaces: &[usize],
) -> Result<Option<Value>> {
    let Some(map) = map else {
        return Ok(None);
    };
    let mut value = serde_json::to_value(map)?;
    if !dropped_surfaces.is_empty()
        && let Some(themes) = value.as_object_mut()
    {
        for theme in themes.values_mut() {
            if let Some(values) = theme.get_mut("values").and_then(Value::as_array_mut) {
                remove_dropped_entries(values, dropped_surfaces);
            }
        }
    }
    Ok(Some(value))
}

/// Normalises and dequantises one non-instance geometry against `pool`.
fn normalise_geometry(geom: &Geometry, pool: &VertexPool) -> Result<NormalisedGeometry> {
    match geom.thetype {
        GeometryType::GeometryInstance => Err(err(
            "normalise_geometry must not be called on a GeometryInstance",
        )),
        GeometryType::MultiPoint => {
            let idxs: Vec<usize> = parse_boundaries(geom)?;
            Ok(NormalisedGeometry {
                tree: points_node(pool, &idxs)?,
                semantics: geom.semantics.clone(),
                material: realigned_appearance(&geom.material, &[])?,
                texture: realigned_appearance(&geom.texture, &[])?,
                dropped_rings: 0,
                dropped_surfaces: 0,
            })
        }
        GeometryType::MultiLineString => {
            let lines: Vec<Vec<usize>> = parse_boundaries(geom)?;
            Ok(NormalisedGeometry {
                tree: ring_list_node(pool, &lines)?,
                semantics: geom.semantics.clone(),
                material: realigned_appearance(&geom.material, &[])?,
                texture: realigned_appearance(&geom.texture, &[])?,
                dropped_rings: 0,
                dropped_surfaces: 0,
            })
        }
        GeometryType::MultiSurface | GeometryType::CompositeSurface => {
            let surfaces: Vec<Vec<Vec<usize>>> = parse_boundaries(geom)?;
            let mut dropped_rings = 0usize;
            let mut dropped_positions = Vec::new();
            let mut kept = Vec::with_capacity(surfaces.len());
            for (pos, surface) in surfaces.iter().enumerate() {
                match normalise_surface(surface, &mut dropped_rings) {
                    Some(s) => kept.push(s),
                    None => dropped_positions.push(pos),
                }
            }
            Ok(NormalisedGeometry {
                tree: surface_list_node(pool, &kept)?,
                semantics: realign_semantics(&geom.semantics, &dropped_positions),
                material: realigned_appearance(&geom.material, &dropped_positions)?,
                texture: realigned_appearance(&geom.texture, &dropped_positions)?,
                dropped_rings,
                dropped_surfaces: dropped_positions.len(),
            })
        }
        GeometryType::Solid => {
            let shells: Vec<Vec<Vec<Vec<usize>>>> = parse_boundaries(geom)?;
            let mut dropped_rings = 0usize;
            let mut dropped_surfaces = 0usize;
            let mut kept_shells = Vec::with_capacity(shells.len());
            for shell in &shells {
                let mut kept_faces = Vec::with_capacity(shell.len());
                for face in shell {
                    match normalise_surface(face, &mut dropped_rings) {
                        Some(f) => kept_faces.push(f),
                        None => dropped_surfaces += 1,
                    }
                }
                kept_shells.push(kept_faces);
            }
            let tree = Node::List(
                kept_shells
                    .iter()
                    .map(|faces| surface_list_node(pool, faces))
                    .collect::<Result<Vec<_>>>()?,
            );
            Ok(NormalisedGeometry {
                tree,
                // Solid semantics/appearance nest per shell; not realigned
                // here, same limitation as the writer (no real fixture
                // exercises a Solid/MultiSolid degenerate ring, so this
                // never bites).
                semantics: geom.semantics.clone(),
                material: realigned_appearance(&geom.material, &[])?,
                texture: realigned_appearance(&geom.texture, &[])?,
                dropped_rings,
                dropped_surfaces,
            })
        }
        GeometryType::MultiSolid | GeometryType::CompositeSolid => {
            let solids: Vec<Vec<Vec<Vec<Vec<usize>>>>> = parse_boundaries(geom)?;
            let mut dropped_rings = 0usize;
            let mut dropped_surfaces = 0usize;
            let mut kept_solids = Vec::with_capacity(solids.len());
            for shells in &solids {
                let mut kept_shells = Vec::with_capacity(shells.len());
                for shell in shells {
                    let mut kept_faces = Vec::with_capacity(shell.len());
                    for face in shell {
                        match normalise_surface(face, &mut dropped_rings) {
                            Some(f) => kept_faces.push(f),
                            None => dropped_surfaces += 1,
                        }
                    }
                    kept_shells.push(kept_faces);
                }
                kept_solids.push(kept_shells);
            }
            let tree = Node::List(
                kept_solids
                    .iter()
                    .map(|shells| {
                        Ok(Node::List(
                            shells
                                .iter()
                                .map(|faces| surface_list_node(pool, faces))
                                .collect::<Result<Vec<_>>>()?,
                        ))
                    })
                    .collect::<Result<Vec<_>>>()?,
            );
            Ok(NormalisedGeometry {
                tree,
                semantics: geom.semantics.clone(),
                material: realigned_appearance(&geom.material, &[])?,
                texture: realigned_appearance(&geom.texture, &[])?,
                dropped_rings,
                dropped_surfaces,
            })
        }
    }
}

// ---------------------------------------------------------------------
// Per-object comparable data.
// ---------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
struct NormGeometry {
    gtype: GeometryType,
    tree: Node,
    semantics: Option<Value>,
    /// `None` when the geometry has no material, OR when
    /// `exclusions.appearance` skipped it (logged in `excluded`).
    material: Option<Value>,
    texture: Option<Value>,
}

struct ObjectData {
    thetype: String,
    attributes: Value,
    parents: HashSet<String>,
    children: HashSet<String>,
    geometries: HashMap<Option<String>, NormGeometry>,
}

/// Recursively drops `null`-valued object entries so an explicit-null
/// attribute compares equal to an absent one on the other side.
fn strip_nulls(v: Value) -> Value {
    match v {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .filter(|(_, v)| !v.is_null())
                .map(|(k, v)| (k, strip_nulls(v)))
                .collect(),
        ),
        Value::Array(arr) => Value::Array(arr.into_iter().map(strip_nulls).collect()),
        other => other,
    }
}

fn canonicalise_attrs(v: Option<&Value>) -> Value {
    let base = match v {
        None | Some(Value::Null) => Value::Object(Map::new()),
        Some(other) => other.clone(),
    };
    strip_nulls(base)
}

/// JSON-value equality after normalisation: strings that both parse as
/// RFC3339 instants compare as instants; strings that both parse as
/// `%Y-%m-%d` dates compare as dates; numbers compare as f64 within 1e-9
/// relative; objects compare by key (null-valued entries excluded from the
/// key set on both sides — the null-vs-absent rule); arrays compare
/// order-preserving, element-wise.
fn values_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Null, Value::Null) => true,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Number(_), Value::Number(_)) => {
            let (x, y) = (a.as_f64().unwrap(), b.as_f64().unwrap());
            let scale = x.abs().max(y.abs()).max(1.0);
            (x - y).abs() <= 1e-9 * scale
        }
        (Value::String(sa), Value::String(sb)) => strings_equal(sa, sb),
        (Value::Array(xa), Value::Array(xb)) => {
            xa.len() == xb.len() && xa.iter().zip(xb).all(|(p, q)| values_equal(p, q))
        }
        (Value::Object(oa), Value::Object(ob)) => {
            let ka: HashSet<&String> = oa
                .iter()
                .filter(|(_, v)| !v.is_null())
                .map(|(k, _)| k)
                .collect();
            let kb: HashSet<&String> = ob
                .iter()
                .filter(|(_, v)| !v.is_null())
                .map(|(k, _)| k)
                .collect();
            ka == kb && ka.iter().all(|k| values_equal(&oa[*k], &ob[*k]))
        }
        _ => a == b,
    }
}

fn strings_equal(sa: &str, sb: &str) -> bool {
    if let (Ok(ta), Ok(tb)) = (
        DateTime::parse_from_rfc3339(sa),
        DateTime::parse_from_rfc3339(sb),
    ) {
        return ta.with_timezone(&chrono::Utc) == tb.with_timezone(&chrono::Utc);
    }
    if let (Ok(da), Ok(db)) = (
        NaiveDate::parse_from_str(sa, "%Y-%m-%d"),
        NaiveDate::parse_from_str(sb, "%Y-%m-%d"),
    ) {
        return da == db;
    }
    sa == sb
}

fn claim_or_log(
    geometries: &mut HashMap<Option<String>, NormGeometry>,
    key: Option<String>,
    value: NormGeometry,
    excluded: &mut Vec<String>,
    label: &str,
    id: &str,
) {
    match geometries.entry(key) {
        std::collections::hash_map::Entry::Occupied(entry) => {
            excluded.push(format!(
                "{label}: object {id}: extra geometry at lod {:?} skipped (writer keeps only the first per object/LoD)",
                entry.key()
            ));
        }
        std::collections::hash_map::Entry::Vacant(entry) => {
            entry.insert(value);
        }
    }
}

/// Builds one object's comparable geometry map, applying (in order):
/// `exclusions.geometry_instances` / `exclusions.appearance`, degenerate-ring
/// normalisation, and the writer's first-geometry-per-LoD rule. Logs
/// everything it skips into `excluded`.
fn build_geometries(
    geoms: &[Geometry],
    pool: &VertexPool,
    opts: &CompareOptions,
    excluded: &mut Vec<String>,
    label: &str,
    id: &str,
) -> Result<HashMap<Option<String>, NormGeometry>> {
    let mut geometries = HashMap::new();
    for geom in geoms {
        if geom.thetype == GeometryType::GeometryInstance {
            if opts.exclusions.geometry_instances {
                excluded.push(format!(
                    "{label}: object {id}: GeometryInstance at lod {:?} excluded (exclusions.geometry_instances)",
                    geom.lod
                ));
                continue;
            }
            let idxs: Vec<usize> =
                serde_json::from_value(geom.boundaries.clone()).unwrap_or_default();
            let tree = match idxs.first() {
                Some(&i) => Node::Point(pool.coord(i)?),
                None => Node::List(vec![]),
            };
            claim_or_log(
                &mut geometries,
                geom.lod.clone(),
                NormGeometry {
                    gtype: geom.thetype.clone(),
                    tree,
                    semantics: None,
                    // Instance appearance rides the template definition
                    // (M4 sidecar data); a kept instance compares by its
                    // reference point only.
                    material: None,
                    texture: None,
                },
                excluded,
                label,
                id,
            );
            continue;
        }

        let skip_appearance =
            opts.exclusions.appearance && (geom.material.is_some() || geom.texture.is_some());
        if skip_appearance {
            excluded.push(format!(
                "{label}: object {id}: material/texture at lod {:?} excluded (exclusions.appearance)",
                geom.lod
            ));
        }

        let normalised = normalise_geometry(geom, pool)?;
        if normalised.dropped_rings > 0 || normalised.dropped_surfaces > 0 {
            excluded.push(format!(
                "{label}: object {id}: geometry at lod {:?}: normalised away {} degenerate ring(s), {} surface(s)",
                geom.lod, normalised.dropped_rings, normalised.dropped_surfaces
            ));
        }
        if node_is_empty(&normalised.tree) {
            excluded.push(format!(
                "{label}: object {id}: geometry at lod {:?} fully degenerate, dropped entirely",
                geom.lod
            ));
            continue;
        }

        claim_or_log(
            &mut geometries,
            geom.lod.clone(),
            NormGeometry {
                gtype: geom.thetype.clone(),
                tree: normalised.tree,
                semantics: normalised.semantics,
                material: if skip_appearance {
                    None
                } else {
                    normalised.material
                },
                texture: if skip_appearance {
                    None
                } else {
                    normalised.texture
                },
            },
            excluded,
            label,
            id,
        );
    }
    Ok(geometries)
}

// ---------------------------------------------------------------------
// One side of the comparison.
// ---------------------------------------------------------------------

struct Side {
    header: CityJSON,
    objects: HashMap<String, ObjectData>,
    excluded: Vec<String>,
}

fn load_side(path: &Path, opts: &CompareOptions) -> Result<Side> {
    let source = Source::open(path)?;
    let header = source.header().clone();
    let label = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());

    let mut objects = HashMap::new();
    let mut excluded = Vec::new();
    for feature in source.features()? {
        let feature = feature?;
        let pool = VertexPool::new(&feature.vertices, &header.transform);
        for (id, co) in &feature.city_objects {
            let attributes = canonicalise_attrs(co.attributes.as_ref());
            let parents: HashSet<String> =
                co.parents.clone().unwrap_or_default().into_iter().collect();
            let children: HashSet<String> = co
                .children
                .clone()
                .unwrap_or_default()
                .into_iter()
                .collect();
            let geometries = match &co.geometry {
                Some(geoms) => build_geometries(geoms, &pool, opts, &mut excluded, &label, id)?,
                None => HashMap::new(),
            };
            objects.insert(
                id.clone(),
                ObjectData {
                    thetype: co.thetype.clone(),
                    attributes,
                    parents,
                    children,
                    geometries,
                },
            );
        }
    }
    Ok(Side {
        header,
        objects,
        excluded,
    })
}

fn transform_axes(t: &Transform) -> [f64; 3] {
    let take = |v: &[f64]| {
        [
            *v.first().unwrap_or(&1.0),
            *v.get(1).unwrap_or(&1.0),
            *v.get(2).unwrap_or(&1.0),
        ]
    };
    take(&t.scale)
}

fn reference_system_url(header: &CityJSON) -> Option<String> {
    header
        .metadata
        .as_ref()
        .and_then(|m| m.reference_system.as_ref())
        .map(cjseq::ReferenceSystem::to_url)
}

fn compare_object(id: &str, a: &ObjectData, b: &ObjectData, tol: [f64; 3], out: &mut Vec<String>) {
    if a.thetype != b.thetype {
        out.push(format!(
            "object {id}: type differs: {} vs {}",
            a.thetype, b.thetype
        ));
    }
    if a.parents != b.parents {
        out.push(format!(
            "object {id}: parents differ: {:?} vs {:?}",
            a.parents, b.parents
        ));
    }
    if a.children != b.children {
        out.push(format!(
            "object {id}: children differ: {:?} vs {:?}",
            a.children, b.children
        ));
    }
    if !values_equal(&a.attributes, &b.attributes) {
        out.push(format!(
            "object {id}: attributes differ: {} vs {}",
            a.attributes, b.attributes
        ));
    }

    let lods_a: HashSet<&Option<String>> = a.geometries.keys().collect();
    let lods_b: HashSet<&Option<String>> = b.geometries.keys().collect();
    for lod in lods_a.difference(&lods_b) {
        out.push(format!(
            "object {id}: geometry at lod {lod:?} present in A, missing in B"
        ));
    }
    for lod in lods_b.difference(&lods_a) {
        out.push(format!(
            "object {id}: geometry at lod {lod:?} present in B, missing in A"
        ));
    }
    let mut common: Vec<&Option<String>> = lods_a.intersection(&lods_b).copied().collect();
    common.sort();
    for lod in common {
        let ga = &a.geometries[lod];
        let gb = &b.geometries[lod];
        if ga.gtype != gb.gtype {
            out.push(format!(
                "object {id}: geometry at lod {lod:?}: type differs: {:?} vs {:?}",
                ga.gtype, gb.gtype
            ));
            continue;
        }
        if !node_matches(&ga.tree, &gb.tree, tol) {
            out.push(format!(
                "object {id}: geometry at lod {lod:?}: boundary/coordinates differ"
            ));
        }
        let sa = ga.semantics.clone().unwrap_or(Value::Null);
        let sb = gb.semantics.clone().unwrap_or(Value::Null);
        if !values_equal(&sa, &sb) {
            out.push(format!(
                "object {id}: geometry at lod {lod:?}: semantics differ: {sa} vs {sb}"
            ));
        }
        // material/texture: compared unless `exclusions.appearance` already
        // dropped them at load time (in which case both sides are None here
        // and the skip was logged in `excluded`). One-side-present is a
        // difference like any other mismatch.
        for (what, va, vb) in [
            ("material", &ga.material, &gb.material),
            ("texture", &ga.texture, &gb.texture),
        ] {
            let na = va.clone().unwrap_or(Value::Null);
            let nb = vb.clone().unwrap_or(Value::Null);
            if !values_equal(&na, &nb) {
                out.push(format!(
                    "object {id}: geometry at lod {lod:?}: {what} differs: {na} vs {nb}"
                ));
            }
        }
    }
}

/// Compares `a` and `b` (each a CityJSON or CityJSONSeq path, opened via
/// [`Source`]) for semantic equality: header `transform` (exact) and
/// `referenceSystem`, the object-id set, and per common object its `type`,
/// `parents`/`children` (as sets), `attributes` (JSON-value equality with
/// timestamp/date/numeric-tolerance normalisation), and geometry per LoD
/// (type, boundary-tree shape, coordinates within `opts.coord_tolerance`,
/// and `semantics`). See the module docs for the degenerate-ring
/// normalisation applied to both sides, and [`Exclusions`] for what
/// `export`'s deliberate drops let this skip instead of flagging.
pub fn compare_datasets(a: &Path, b: &Path, opts: &CompareOptions) -> Result<CompareReport> {
    let side_a = load_side(a, opts)?;
    let side_b = load_side(b, opts)?;

    let mut differences = Vec::new();
    let mut excluded = side_a.excluded;
    excluded.extend(side_b.excluded);

    let ta = serde_json::to_value(&side_a.header.transform)?;
    let tb = serde_json::to_value(&side_b.header.transform)?;
    if ta != tb {
        differences.push(format!("header: transform differs: {ta} vs {tb}"));
    }

    let rsa = reference_system_url(&side_a.header);
    let rsb = reference_system_url(&side_b.header);
    if rsa != rsb {
        differences.push(format!(
            "header: referenceSystem differs: {rsa:?} vs {rsb:?}"
        ));
    }

    let scale_a = transform_axes(&side_a.header.transform);
    let scale_b = transform_axes(&side_b.header.transform);
    let mut tol = [0.0; 3];
    for i in 0..3 {
        tol[i] = if opts.coord_tolerance[i] > 0.0 {
            opts.coord_tolerance[i]
        } else {
            scale_a[i].abs().max(scale_b[i].abs())
        };
    }

    let ids_a: HashSet<&String> = side_a.objects.keys().collect();
    let ids_b: HashSet<&String> = side_b.objects.keys().collect();
    for id in ids_a.difference(&ids_b) {
        differences.push(format!("object {id}: present in A, missing in B"));
    }
    for id in ids_b.difference(&ids_a) {
        differences.push(format!("object {id}: present in B, missing in A"));
    }
    let mut common: Vec<&String> = ids_a.intersection(&ids_b).copied().collect();
    common.sort();
    for id in common {
        compare_object(
            id,
            &side_a.objects[id],
            &side_b.objects[id],
            tol,
            &mut differences,
        );
    }

    if differences.len() > MAX_DIFFERENCES {
        let extra = differences.len() - MAX_DIFFERENCES;
        differences.truncate(MAX_DIFFERENCES);
        differences.push(format!("... {extra} more differences truncated"));
    }

    Ok(CompareReport {
        equal: differences.is_empty(),
        differences,
        excluded,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn fixture(name: &str) -> std::path::PathBuf {
        let p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures")
            .join(name);
        assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
        p
    }

    /// A comparator that always says "equal" is worthless — this test pins
    /// the other direction: two genuinely different real datasets (one
    /// attribute value on one object changed) must produce exactly one
    /// difference naming that object.
    #[test]
    fn compare_detects_a_single_changed_attribute() {
        let original = fixture("delft.city.jsonl");
        let text = fs::read_to_string(&original).unwrap();
        let mut lines: Vec<String> = text.lines().map(str::to_string).collect();

        // Find the first feature line with a non-empty `attributes` object
        // on some CityObject, and flip one attribute value on that object.
        let mut changed_id: Option<String> = None;
        for line in lines.iter_mut().skip(1) {
            let mut feature: Value = serde_json::from_str(line).unwrap();
            let Some(objects) = feature["CityObjects"].as_object_mut() else {
                continue;
            };
            for (id, co) in objects.iter_mut() {
                let Some(attrs) = co.get_mut("attributes").and_then(Value::as_object_mut) else {
                    continue;
                };
                let Some((_, v)) = attrs.iter_mut().find(|(_, v)| v.is_string()) else {
                    continue;
                };
                let s = v.as_str().unwrap().to_string();
                *v = Value::String(format!("{s}-MODIFIED"));
                changed_id = Some(id.clone());
                break;
            }
            if changed_id.is_some() {
                *line = serde_json::to_string(&feature).unwrap();
                break;
            }
        }
        let changed_id = changed_id.expect("delft must have at least one string attribute");

        let dir = tempfile::tempdir().unwrap();
        let modified_path = dir.path().join("delft-modified.city.jsonl");
        fs::write(&modified_path, lines.join("\n") + "\n").unwrap();

        let report =
            compare_datasets(&original, &modified_path, &CompareOptions::default()).unwrap();
        assert!(
            !report.equal,
            "a real attribute change must be detected, not silently accepted"
        );
        assert_eq!(
            report.differences.len(),
            1,
            "exactly one attribute changed on one object must yield exactly one difference, got {:?}",
            report.differences
        );
        assert!(
            report.differences[0].contains(&changed_id),
            "the single difference must name the changed object {changed_id}, got: {}",
            report.differences[0]
        );
    }

    /// The reviewer's probe: with `exclusions.appearance == false` (the
    /// default), a material block present on one side only must be a
    /// DIFFERENCE — not silently ignored. Derived from the real delft
    /// fixture: one geometry of one object gains a `material` block; the
    /// copy must compare `!equal` with exactly one difference naming that
    /// object.
    #[test]
    fn compare_detects_an_added_material_block_when_appearance_not_excluded() {
        let original = fixture("delft.city.jsonl");
        let text = fs::read_to_string(&original).unwrap();
        let mut lines: Vec<String> = text.lines().map(str::to_string).collect();

        // Add a material block to the first geometry of the first object
        // that has one.
        let mut changed_id: Option<String> = None;
        for line in lines.iter_mut().skip(1) {
            let mut feature: Value = serde_json::from_str(line).unwrap();
            let Some(objects) = feature["CityObjects"].as_object_mut() else {
                continue;
            };
            for (id, co) in objects.iter_mut() {
                let Some(geoms) = co.get_mut("geometry").and_then(Value::as_array_mut) else {
                    continue;
                };
                let Some(geom) = geoms.first_mut().and_then(Value::as_object_mut) else {
                    continue;
                };
                geom.insert(
                    "material".to_string(),
                    serde_json::json!({"visual": {"value": 0}}),
                );
                changed_id = Some(id.clone());
                break;
            }
            if changed_id.is_some() {
                *line = serde_json::to_string(&feature).unwrap();
                break;
            }
        }
        let changed_id = changed_id.expect("delft must have at least one geometry");

        let dir = tempfile::tempdir().unwrap();
        let modified_path = dir.path().join("delft-with-material.city.jsonl");
        fs::write(&modified_path, lines.join("\n") + "\n").unwrap();

        let report =
            compare_datasets(&original, &modified_path, &CompareOptions::default()).unwrap();
        assert!(
            !report.equal,
            "an added material block must be a difference when appearance is not excluded"
        );
        assert_eq!(
            report.differences.len(),
            1,
            "exactly one geometry gained a material block: expected exactly one difference, got {:?}",
            report.differences
        );
        assert!(
            report.differences[0].contains(&changed_id),
            "the difference must name the changed object {changed_id}, got: {}",
            report.differences[0]
        );

        // And with the exclusion ON, the same pair compares equal (the block
        // is skipped and logged instead).
        let opts = CompareOptions {
            coord_tolerance: [0.0; 3],
            exclusions: Exclusions {
                appearance: true,
                geometry_instances: false,
            },
        };
        let report = compare_datasets(&original, &modified_path, &opts).unwrap();
        assert!(
            report.equal,
            "with exclusions.appearance the added block must be skipped, got: {:?}",
            report.differences
        );
        assert!(
            report.excluded.iter().any(|e| e.contains(&changed_id)),
            "the skipped block must be logged in excluded naming {changed_id}, got: {:?}",
            report.excluded
        );
    }

    /// More than [`MAX_DIFFERENCES`] real differences must truncate: the
    /// list keeps the first 50 and appends one final "... N more" entry.
    /// Synthesised from the real fixture by editing a string attribute on
    /// every object that has one (delft has far more than 50).
    #[test]
    fn compare_truncates_beyond_max_differences() {
        let original = fixture("delft.city.jsonl");
        let text = fs::read_to_string(&original).unwrap();
        let mut lines: Vec<String> = text.lines().map(str::to_string).collect();

        let mut changed = 0usize;
        for line in lines.iter_mut().skip(1) {
            let mut feature: Value = serde_json::from_str(line).unwrap();
            let Some(objects) = feature["CityObjects"].as_object_mut() else {
                continue;
            };
            let mut line_changed = false;
            for (_, co) in objects.iter_mut() {
                let Some(attrs) = co.get_mut("attributes").and_then(Value::as_object_mut) else {
                    continue;
                };
                let Some((_, v)) = attrs.iter_mut().find(|(_, v)| v.is_string()) else {
                    continue;
                };
                let s = v.as_str().unwrap().to_string();
                *v = Value::String(format!("{s}-MODIFIED"));
                changed += 1;
                line_changed = true;
            }
            if line_changed {
                *line = serde_json::to_string(&feature).unwrap();
            }
        }
        assert!(
            changed > MAX_DIFFERENCES,
            "need more than {MAX_DIFFERENCES} edits to exercise truncation, only made {changed}"
        );

        let dir = tempfile::tempdir().unwrap();
        let modified_path = dir.path().join("delft-many-edits.city.jsonl");
        fs::write(&modified_path, lines.join("\n") + "\n").unwrap();

        let report =
            compare_datasets(&original, &modified_path, &CompareOptions::default()).unwrap();
        assert!(!report.equal);
        assert_eq!(
            report.differences.len(),
            MAX_DIFFERENCES + 1,
            "the first {MAX_DIFFERENCES} differences plus one truncation notice"
        );
        let last = report.differences.last().unwrap();
        assert!(
            last.contains("truncated") && last.contains(&(changed - MAX_DIFFERENCES).to_string()),
            "the final entry must name the truncated count ({}), got: {last}",
            changed - MAX_DIFFERENCES
        );
    }

    #[test]
    fn compare_delft_against_itself_is_equal() {
        let path = fixture("delft.city.jsonl");
        let report = compare_datasets(&path, &path, &CompareOptions::default()).unwrap();
        assert!(
            report.equal,
            "identical inputs must compare equal, got differences: {:?}",
            report.differences
        );
        assert!(report.differences.is_empty());
    }
}
