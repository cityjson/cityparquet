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
//! same blind spot. The "fewer than 3" INDEX-based threshold counts remaining
//! ring ELEMENTS, not geometrically distinct coordinates: a 3-element
//! zero-area ring made of 3 genuinely distinct (if colinear) vertices passes
//! through untouched — the CityJSON spec's stricter "at least 3 distinct
//! vertices" wording is deliberately not enforced beyond this structural
//! element count in general (data quality is not the format's business,
//! matching the writer's policy). It is applied to BOTH sides unconditionally.
//!
//! One narrow exception to "data quality is not the format's business":
//! coordinate-DEGENERATE rings, i.e. rings whose vertex INDICES are pairwise
//! distinct (so the INDEX-based check above does not fire) but which
//! dequantise to fewer than 3 DISTINCT coordinates. This is a real 3DBAG tile
//! finding (`9-284-556.city.json`, object
//! `NL.IMBAG.Pand.0503100000025101-0`, LoD 2.2, face 497): a 3-index exterior
//! ring `[49590, 49127, 49595]` whose three distinct vertex indices are all
//! duplicate entries of the identical quantised vertex `(31653, 359040,
//! -33533)`. `crate::wkb_write::normalise_ring` is deliberately INDEX-based
//! (see its docs) and does not drop this ring on write; the WKB writer emits
//! it as a real (degenerate) ring, and the WKB reader's coordinate interner
//! (`crate::wkb_read::CoordInterner`, bitwise `f64::to_bits` dedup) then
//! collapses its 3 written points down to 1 repeated pool index on read. The
//! net, only-visible-after-a-round-trip effect is exactly as if the writer
//! had dropped the face: the source side keeps a normal-looking 3-distinct-
//! index ring, while the round-tripped side's boundary — after going through
//! WKB and back — carries the SAME index 3 times over, which the INDEX-based
//! check above already treats as closed-to-nothing and drops. Comparing the
//! two sides without this extension therefore reports a spurious
//! boundary/semantics difference on real production data. The fix: after the
//! INDEX-based fixpoint strip above, also drop a ring whose surviving indices
//! dequantise (via that side's own `VertexPool`, bitwise `f64::to_bits`
//! comparison — deterministic scale/translate arithmetic makes this exactly
//! as exact, and cheaper, than comparing the underlying quantised i64
//! triples) to fewer than 3 DISTINCT coordinates — applied identically to
//! BOTH sides, reusing the same `normalise_ring`/`normalise_surface`
//! machinery (fixpoint stripping, dropped-position tracking, exterior-ring
//! surface drop, semantics/material/texture realignment) rather than
//! duplicating it. This does not affect genuine (distinct-vertex) zero-area
//! rings, which still pass through unchanged.
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

use cityparquet_schema::{CityParquetError, Lod, Result};
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

/// One `address[]` entry as compared: the mapped postal fields (spec
/// "Addresses", gap 10) plus its `location`, if any, dequantised into a
/// [`Node`] tree exactly like a real `MultiPoint` geometry's boundaries —
/// `None` when `location` is absent or malformed, mirroring the encoder's
/// own graceful-degradation rule ([`crate::encode::build_location_wkb`]) so
/// the comparator never flags as a difference something the encoder itself
/// silently drops on both sides identically.
#[derive(Debug, Clone, PartialEq)]
struct AddressCompare {
    postal: crate::address::AddressPostal,
    location: Option<Node>,
}

/// Resolves one address entry's `location` member into a [`Node`] tree,
/// reusing [`points_node`] exactly as [`normalise_geometry`]'s `MultiPoint`
/// arm does — this IS structurally a `MultiPoint` geometry, just reached via
/// `address[].location` rather than the object's own `geometry` array.
fn address_location_node(location: &Value, pool: &VertexPool) -> Option<Node> {
    let obj = location.as_object()?;
    if obj.get("type").and_then(Value::as_str) != Some("MultiPoint") {
        return None;
    }
    let idxs: Vec<usize> = serde_json::from_value(obj.get("boundaries")?.clone()).ok()?;
    if idxs.is_empty() {
        return None;
    }
    points_node(pool, &idxs).ok()
}

/// One object's `address[]` list as compared (spec "Addresses"): the SAME
/// recognised-member mapping the encoder uses
/// ([`crate::address::map_postal_fields`]) applied to the raw source
/// `address` array, via [`crate::encode::raw_address_members`] — both sides
/// of a comparison go through this identical extraction (the source's own
/// address, and the exported CityJSON's re-emitted one, which uses the same
/// canonical member names), so the two can never diverge on what "the
/// address" means. `co` with no `address` member (or a malformed one) yields
/// an empty `Vec` — indistinguishable, for comparison, from an explicit
/// empty array, since both round-trip to the same thing.
fn address_comparables(co: &cjseq::CityObject, pool: &VertexPool) -> Result<Vec<AddressCompare>> {
    let Some(entries) = crate::encode::raw_address_members(co)? else {
        return Ok(Vec::new());
    };
    Ok(entries
        .iter()
        .map(|entry| AddressCompare {
            postal: crate::address::map_postal_fields(entry),
            location: entry
                .get("location")
                .and_then(|loc| address_location_node(loc, pool)),
        })
        .collect())
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
///
/// After the index-based fixpoint strip, also drops a ring whose surviving
/// indices dequantise (bitwise, via [`distinct_coord_count`]) to fewer than
/// 3 DISTINCT coordinates — the 3DBAG-tile blind spot described in the
/// module docs: an index-distinct ring (so the loop above does not fire)
/// that is nonetheless coordinate-degenerate cannot form a real ring either.
/// `pool` is the SAME side's own `VertexPool` the ring's indices belong to
/// (source or exported — this function does not know or care which).
fn normalise_ring(ring: &[usize], pool: &VertexPool) -> Result<Option<Vec<usize>>> {
    let mut stripped = ring;
    while stripped.len() >= 2 && stripped.first() == stripped.last() {
        stripped = &stripped[..stripped.len() - 1];
    }
    if stripped.len() < 3 {
        return Ok(None);
    }
    if distinct_coord_count(stripped, pool)? < 3 {
        return Ok(None);
    }
    Ok(Some(stripped.to_vec()))
}

/// Number of DISTINCT dequantised coordinates a ring's vertex indices
/// resolve to, deduped bitwise (`f64::to_bits`) exactly like
/// `crate::wkb_read::CoordInterner`'s coordinate pool. Deterministic
/// floating-point arithmetic guarantees that two indices which quantise to
/// the same integer triple always dequantise (through the same `VertexPool`,
/// i.e. the same scale/translate) to a bit-identical `f64` triple, so this
/// bitwise comparison on the dequantised value is exactly as exact as (and
/// cheaper than plumbing through) a direct comparison of the underlying
/// quantised i64 triples.
fn distinct_coord_count(ring: &[usize], pool: &VertexPool) -> Result<usize> {
    let mut seen: HashSet<[u64; 3]> = HashSet::with_capacity(ring.len());
    for &idx in ring {
        let c = pool.coord(idx)?;
        seen.insert([c[0].to_bits(), c[1].to_bits(), c[2].to_bits()]);
    }
    Ok(seen.len())
}

/// Normalise one surface's rings, counting every dropped ring (interior or
/// exterior) into `dropped_rings`. Returns `None` (surface dropped entirely)
/// when the EXTERIOR ring (index 0) was dropped — interior rings cannot
/// stand without it.
fn normalise_surface(
    rings: &[Vec<usize>],
    pool: &VertexPool,
    dropped_rings: &mut usize,
) -> Result<Option<Vec<Vec<usize>>>> {
    let mut kept = Vec::with_capacity(rings.len());
    let mut exterior_dropped = false;
    for (i, ring) in rings.iter().enumerate() {
        match normalise_ring(ring, pool)? {
            Some(r) => kept.push(r),
            None => {
                *dropped_rings += 1;
                if i == 0 {
                    exterior_dropped = true;
                }
            }
        }
    }
    Ok(if exterior_dropped { None } else { Some(kept) })
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
/// realigned `semantics` and `material`/`texture` (realigned flat for the
/// surface-list types, or walked `depth` shell/solid levels deep for the
/// solid types — see `solid_face_nesting_depth`/`realign_nested_values`,
/// this module's independent counterpart of `crate::encode`'s functions of
/// the same name), and how much was normalised away (for the `excluded`
/// log).
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

/// Canonicalise a source geometry's `semantics` to `{surfaces, face_semantics}`
/// — the flat, face-aligned form the stored column uses (§8) — so BOTH sides of
/// a comparison are reduced to the same representation. This flattens the
/// nested CityJSON `values` (expanding the null shorthand via `boundaries`) and
/// removes the writer-dropped face positions, so a source that used the null
/// shorthand (`values: [null]`) compares EQUAL to the exporter's expanded
/// per-face form (spec §17's up-to-canonicalisation round-trip). Reuses the
/// exact flatten the encoder uses, so the two never diverge.
fn canonical_semantics(
    semantics: &Option<Value>,
    boundaries: &Value,
    thetype: &GeometryType,
    dropped: &[usize],
) -> Option<Value> {
    let semantics = semantics.as_ref()?;
    let surfaces = semantics.get("surfaces")?.clone();
    let depth = crate::encode::values_nesting_depth(thetype);
    let mut flat = Vec::new();
    if let Some(values) = semantics.get("values") {
        crate::encode::flatten_values(values, boundaries, depth, &mut flat);
    }
    flat.resize(
        crate::encode::count_boundary_faces(boundaries, depth),
        Value::Null,
    );
    let drop_set: std::collections::HashSet<usize> = dropped.iter().copied().collect();
    let face_semantics: Vec<Value> = flat
        .into_iter()
        .enumerate()
        .filter_map(|(i, v)| (!drop_set.contains(&i)).then_some(v))
        .collect();
    Some(serde_json::json!({ "surfaces": surfaces, "face_semantics": face_semantics }))
}

/// Number of shell/solid nesting levels above the per-face entries in a
/// Solid-family `semantics`/`material`/`texture` values array: `Solid`
/// nests one level (shells -> faces), `MultiSolid`/`CompositeSolid` nest
/// two (solids -> shells -> faces). `None` for the non-solid types, whose
/// per-surface arrays sit directly at the top level (realigned flat by
/// `realign_semantics`/`realigned_appearance` instead).
///
/// This is `crate::encode`'s function of the same name, duplicated here
/// rather than shared — per this module's independent-reimplementation
/// policy (see the module docs): a comparator that reused the writer's own
/// structural helpers could not catch a bug in them, since both sides would
/// share the same blind spot. Derived from the geometry type rather than
/// inferred from the values' own shape for the same reason `crate::encode`
/// gives: a shape-only heuristic cannot distinguish a single shell holding
/// scalar semantics values from a single face holding one texture ring —
/// they collide byte-for-byte whenever there is exactly one shell, the
/// common case for a real-world `Solid`.
fn solid_face_nesting_depth(thetype: &GeometryType) -> Option<usize> {
    match thetype {
        GeometryType::Solid => Some(1),
        GeometryType::MultiSolid | GeometryType::CompositeSolid => Some(2),
        _ => None,
    }
}

/// Remove entries at flat positions `dropped` from a Solid-family nested
/// values hierarchy, walking exactly `depth` levels of shell/solid nesting
/// before treating an array as the face list to filter by position — this
/// module's independent counterpart of `crate::encode::realign_nested_values`
/// (see `solid_face_nesting_depth`'s docs for why it is duplicated, not
/// shared). `dropped` are flat positions counted depth-first across shells
/// (and solids), matching both `wkb_write`'s writer-side `pos` counter and
/// this module's own `normalise_geometry` traversal order.
fn realign_nested_values(values: &mut Value, depth: usize, dropped: &[usize]) {
    fn walk(v: &mut Value, depth: usize, flat: &mut usize, dropped: &[usize]) {
        let Some(arr) = v.as_array_mut() else {
            return;
        };
        if depth == 0 {
            let mut kept = Vec::with_capacity(arr.len());
            for e in arr.drain(..) {
                if !dropped.contains(flat) {
                    kept.push(e);
                }
                *flat += 1;
            }
            *arr = kept;
        } else {
            for e in arr.iter_mut() {
                walk(e, depth - 1, flat, dropped);
            }
        }
    }
    let mut flat = 0usize;
    walk(values, depth, &mut flat, dropped);
}

/// Feature-scoped `material`/`texture`/`vertices-texture` DEFINITIONS a
/// geometry's `material`/`texture` index references resolve against —
/// `cjseq::CityJSONFeature::appearance`'s three arrays, borrowed for the
/// lifetime of one `load_side` feature. Empty (never `None`, so callers never
/// have to special-case "no appearance") when the feature carries no
/// `appearance` block at all.
///
/// Dereferencing through this (rather than comparing the raw index numbers
/// `geom.material`/`geom.texture` carry) is required because feature-local
/// index NUMBERING is an implementation detail of whatever produced the
/// file, not part of a CityJSON's semantics — the same reason boundary
/// vertex INDICES are dereferenced through `VertexPool` into real
/// coordinates instead of compared by index identity. The analogy stops at
/// the dereferencing step, though: dereferenced BOUNDARY coordinates
/// compare under the per-axis `coord_tolerance`, while dereferenced UV
/// coordinates (and every other value in a resolved definition) compare as
/// plain JSON through `values_equal` — i.e. under its generic 1e-9
/// RELATIVE float tolerance, not any quantisation-derived one (UVs are
/// stored unquantised on both sides, so there is no scale to derive a
/// tolerance from). Two independently-produced
/// packages routinely disagree on numbering for two INDEPENDENT reasons that
/// this module's own M4 task 11 round-trip gate against a real Compatibility
/// package exposed: (1) `crate::encode`'s `BatchIter` sorts a feature's
/// CityObject ids alphabetically before writing rows, while `cjseq`'s own
/// `get_cjfeature` walks parent-then-children in the file's own `children`
/// list order — different row/local-index assignment order for the exact
/// same feature; (2) `crate::export::LocalAppearance::local_uv_id` dedupes
/// inlined `[u, v]` pairs BY VALUE, while `cjseq::update_texture` maps by
/// REFERENCED GLOBAL INDEX identity (no value-dedup) — so two different
/// source UV entries that happen to hold the same coordinate pair get ONE
/// local id after a round trip but TWO in the original file. Neither
/// disagreement is a data-loss bug; a byte-exact-index comparator cannot
/// tell that apart from one, which is exactly why it does not compare index
/// numbers at all.
struct AppearanceDefs<'a> {
    materials: &'a [Value],
    textures: &'a [Value],
    vertices_texture: &'a [Vec<f64>],
}

impl AppearanceDefs<'_> {
    /// No `appearance` block at all (or none of the three arrays present).
    fn empty() -> Self {
        AppearanceDefs {
            materials: &[],
            textures: &[],
            vertices_texture: &[],
        }
    }

    fn from_appearance(appearance: Option<&cjseq::Appearance>) -> AppearanceDefs<'_> {
        match appearance {
            Some(a) => AppearanceDefs {
                materials: a.materials.as_deref().unwrap_or(&[]),
                textures: a.textures.as_deref().unwrap_or(&[]),
                vertices_texture: a.vertices_texture.as_deref().unwrap_or(&[]),
            },
            None => AppearanceDefs::empty(),
        }
    }

    fn from_feature(feature: &cjseq::CityJSONFeature) -> AppearanceDefs<'_> {
        Self::from_appearance(feature.appearance.as_ref())
    }

    /// The RAW DOCUMENT appearance arrays that a `GeometryInstance` TEMPLATE's
    /// `material`/`texture` indices actually reference — used in
    /// [`resolve_instance`], mirroring `crate::package::build_template_rows`'s
    /// write-side counterpart (which dereferences template appearance against
    /// the exact same array when building `geometry_templates.parquet`).
    ///
    /// Deliberately NOT `header.appearance`: see
    /// [`crate::source::Source::doc_appearance`]'s doc comment — for a
    /// whole-document CityJSON source, `Source::header()` is `cjseq`'s
    /// `get_metadata()`, which SLICES `appearance` down to only the entries
    /// referenced by templates and renumbers them, but does so against a
    /// separate clone; the header's own `geometry_templates.material`/
    /// `texture` maps it hands back still carry the ORIGINAL document's
    /// global indices. Resolving those against the sliced-and-renumbered
    /// `header.appearance` would therefore go out of range (or silently
    /// dereference the wrong definition) for any `.city.json` source with
    /// more appearance definitions than its templates alone reference — the
    /// M4 Codex-review Finding 3 fix must use `Source::doc_appearance()`
    /// instead, never a per-feature `AppearanceDefs` like
    /// [`Self::from_feature`].
    fn from_doc_appearance(source: &Source) -> AppearanceDefs<'_> {
        Self::from_appearance(source.doc_appearance())
    }
}

/// One material index -> its actual definition in `defs.materials`.
/// `defs.materials.is_empty()` (no `appearance.materials` array to resolve
/// against at all — e.g. a hand-built geometry in this module's own unit
/// tests, or a real feature whose material index is a leftover reference
/// with nothing behind it) passes the raw index number through UNCHANGED
/// rather than erroring: comparing the bare number is still strictly better
/// than refusing to compare at all, and every REAL round-tripped package
/// this module compares carries the array whenever it carries any material
/// index reference in the first place.
fn resolve_material_index(v: &Value, defs: &AppearanceDefs) -> Result<Value> {
    match v {
        Value::Null => Ok(Value::Null),
        Value::Number(n) => {
            if defs.materials.is_empty() {
                return Ok(Value::Number(n.clone()));
            }
            let idx = n
                .as_u64()
                .ok_or_else(|| err(format!("material index is not a non-negative integer: {n}")))?
                as usize;
            defs.materials.get(idx).cloned().ok_or_else(|| {
                err(format!(
                    "material index {idx} out of range (have {} definitions)",
                    defs.materials.len()
                ))
            })
        }
        other => Err(err(format!(
            "material index must be an integer or null, got {other}"
        ))),
    }
}

/// Walks a material theme's `values` tree (arbitrarily nested for the Solid
/// family; flat for the surface-list types), resolving each leaf index.
fn resolve_material_tree(v: &Value, defs: &AppearanceDefs) -> Result<Value> {
    match v {
        Value::Array(items) => Ok(Value::Array(
            items
                .iter()
                .map(|x| resolve_material_tree(x, defs))
                .collect::<Result<Vec<_>>>()?,
        )),
        _ => resolve_material_index(v, defs),
    }
}

/// One geometry's whole `material` map (`{"<theme>": {"value": idx} |
/// {"values": <tree>}}`) with every index resolved to its actual
/// definition — the dereferencing counterpart of the old (index-comparing)
/// `realigned_appearance`.
fn resolve_material_map(map: &Value, defs: &AppearanceDefs) -> Result<Value> {
    let obj = map.as_object().ok_or_else(|| {
        err("material map must be a JSON object of theme -> {value|values}".to_string())
    })?;
    let mut out = Map::with_capacity(obj.len());
    for (theme, inner) in obj {
        let inner_obj = inner
            .as_object()
            .ok_or_else(|| err(format!("material theme '{theme}' must be an object")))?;
        let mut new_inner = Map::with_capacity(inner_obj.len());
        if let Some(v) = inner_obj.get("value") {
            new_inner.insert("value".to_string(), resolve_material_index(v, defs)?);
        }
        if let Some(v) = inner_obj.get("values") {
            new_inner.insert("values".to_string(), resolve_material_tree(v, defs)?);
        }
        out.insert(theme.clone(), Value::Object(new_inner));
    }
    Ok(Value::Object(out))
}

/// A *ring* is the innermost texture-map array: `[textureIdx|null, uvIdx0,
/// uvIdx1, ...]` (standard CityJSON index form — never the CityParquet-
/// internal inlined `[u, v]` form, which exists only inside a package's own
/// stored columns and is always undone back to this index form by
/// `crate::export` before a file reaches this comparator). Recognised the
/// same way `crate::export::is_localised_texture_ring` recognises it: first
/// element a number or `null`.
fn is_texture_ring(items: &[Value]) -> bool {
    !items.is_empty() && matches!(items[0], Value::Number(_) | Value::Null)
}

/// One texture ring with its texture-definition index and every
/// vertices-texture index resolved to real values: `[textureIdx, uv0, uv1,
/// ...]` becomes `[<texture definition>, [u0, v0], [u1, v1], ...]`. Mirrors
/// [`resolve_material_index`]'s empty-`defs` passthrough for both the
/// texture id and the UV indices independently (a feature could in
/// principle carry `textures` but no `vertices-texture`, or vice versa,
/// though never in practice for a real package).
fn resolve_texture_ring(items: &[Value], defs: &AppearanceDefs) -> Result<Value> {
    if items.len() == 1 && items[0].is_null() {
        return Ok(Value::Array(vec![Value::Null]));
    }
    let mut out = Vec::with_capacity(items.len());
    out.push(match &items[0] {
        Value::Null => Value::Null,
        Value::Number(n) => {
            if defs.textures.is_empty() {
                Value::Number(n.clone())
            } else {
                let idx = n.as_u64().ok_or_else(|| {
                    err(format!("texture index is not a non-negative integer: {n}"))
                })? as usize;
                defs.textures.get(idx).cloned().ok_or_else(|| {
                    err(format!(
                        "texture index {idx} out of range (have {} definitions)",
                        defs.textures.len()
                    ))
                })?
            }
        }
        other => {
            return Err(err(format!(
                "texture index must be an integer or null, got {other}"
            )));
        }
    });
    for uv in &items[1..] {
        if defs.vertices_texture.is_empty() {
            out.push(uv.clone());
            continue;
        }
        let idx = uv.as_u64().ok_or_else(|| {
            err(format!(
                "vertex-texture index is not a non-negative integer: {uv}"
            ))
        })? as usize;
        let pair = defs.vertices_texture.get(idx).ok_or_else(|| {
            err(format!(
                "vertex-texture index {idx} out of range (have {} entries)",
                defs.vertices_texture.len()
            ))
        })?;
        // A malformed (hand-edited/third-party) file can carry a
        // vertices-texture entry with fewer than 2 coordinates — an error
        // naming the entry, never an index-out-of-bounds panic.
        let u = pair.first().ok_or_else(|| {
            err(format!(
                "vertex-texture entry {idx} is malformed: expected [u, v], got an empty entry"
            ))
        })?;
        let v = pair.get(1).ok_or_else(|| {
            err(format!(
                "vertex-texture entry {idx} is malformed: expected [u, v], got {} coordinate(s)",
                pair.len()
            ))
        })?;
        out.push(serde_json::json!([u, v]));
    }
    Ok(Value::Array(out))
}

/// Walks a texture theme's `values` tree down to the innermost rings,
/// resolving each one.
fn resolve_texture_tree(v: &Value, defs: &AppearanceDefs) -> Result<Value> {
    match v {
        Value::Array(items) => {
            if is_texture_ring(items) {
                resolve_texture_ring(items, defs)
            } else {
                Ok(Value::Array(
                    items
                        .iter()
                        .map(|x| resolve_texture_tree(x, defs))
                        .collect::<Result<Vec<_>>>()?,
                ))
            }
        }
        other => Err(err(format!(
            "unexpected non-array node in texture tree: {other}"
        ))),
    }
}

/// One geometry's whole `texture` map (`{"<theme>": {"values": <tree>}}`)
/// with every ring resolved — the dereferencing counterpart of the old
/// (index-comparing) `realigned_appearance`.
fn resolve_texture_map(map: &Value, defs: &AppearanceDefs) -> Result<Value> {
    let obj = map
        .as_object()
        .ok_or_else(|| err("texture map must be a JSON object of theme -> {values}".to_string()))?;
    let mut out = Map::with_capacity(obj.len());
    for (theme, inner) in obj {
        let inner_obj = inner
            .as_object()
            .ok_or_else(|| err(format!("texture theme '{theme}' must be an object")))?;
        let values = inner_obj
            .get("values")
            .ok_or_else(|| err(format!("texture theme '{theme}' is missing 'values'")))?;
        let resolved = resolve_texture_tree(values, defs)?;
        let mut new_inner = Map::with_capacity(1);
        new_inner.insert("values".to_string(), resolved);
        out.insert(theme.clone(), Value::Object(new_inner));
    }
    Ok(Value::Object(out))
}

/// One geometry's `material` map, DEREFERENCED via `defs` (see
/// [`AppearanceDefs`]) then realigned for the surfaces the degenerate
/// normalisation dropped (mirrors `crate::encode`'s
/// `realign_appearance_themes`). Theme-level scalar `value` entries apply to
/// all surfaces and need no realignment. Only for the flat surface-list
/// types (`MultiSurface`/`CompositeSurface`) and the non-polygonal types
/// (called with an always-empty `dropped_surfaces`); the solid types nest
/// their per-surface arrays per shell and are realigned instead via
/// [`realigned_nested_material`].
fn realigned_material(
    map: &Option<HashMap<String, cjseq::Material>>,
    dropped_surfaces: &[usize],
    defs: Option<&AppearanceDefs>,
) -> Result<Option<Value>> {
    // `defs == None` means `exclusions.appearance` already decided this
    // geometry's appearance is skipped: don't resolve (or even validate)
    // anything the caller is about to discard.
    let (Some(map), Some(defs)) = (map, defs) else {
        return Ok(None);
    };
    // Realign the RAW (unresolved) index tree BEFORE dereferencing through
    // `defs` — mirroring `crate::encode`'s own pipeline order (drop
    // realignment happens before the dataset-global appearance rewrite). A
    // dangling/out-of-range index sitting only at a to-be-dropped position
    // is real, valid writer output; resolving it first (the old order)
    // would error on data a real round trip happily produces (M5 debt
    // item 1).
    let mut raw = serde_json::to_value(map)?;
    if !dropped_surfaces.is_empty()
        && let Some(themes) = raw.as_object_mut()
    {
        for theme in themes.values_mut() {
            if let Some(values) = theme.get_mut("values").and_then(Value::as_array_mut) {
                remove_dropped_entries(values, dropped_surfaces);
            }
        }
    }
    let value = resolve_material_map(&raw, defs)?;
    Ok(Some(value))
}

/// One geometry's `texture` map — the [`realigned_material`] counterpart for
/// `texture`.
fn realigned_texture(
    map: &Option<HashMap<String, cjseq::Texture>>,
    dropped_surfaces: &[usize],
    defs: Option<&AppearanceDefs>,
) -> Result<Option<Value>> {
    let (Some(map), Some(defs)) = (map, defs) else {
        return Ok(None);
    };
    // Realign before resolving — see [`realigned_material`]'s doc comment.
    let mut raw = serde_json::to_value(map)?;
    if !dropped_surfaces.is_empty()
        && let Some(themes) = raw.as_object_mut()
    {
        for theme in themes.values_mut() {
            if let Some(values) = theme.get_mut("values").and_then(Value::as_array_mut) {
                remove_dropped_entries(values, dropped_surfaces);
            }
        }
    }
    let value = resolve_texture_map(&raw, defs)?;
    Ok(Some(value))
}

/// One geometry's `material` map with each theme's `values` array
/// DEREFERENCED via `defs` (see [`AppearanceDefs`]) then realigned `depth`
/// shell/solid levels deep — the Solid-family counterpart of
/// [`realigned_material`], which only handles the flat surface-list shape.
fn realigned_nested_material(
    map: &Option<HashMap<String, cjseq::Material>>,
    depth: usize,
    dropped: &[usize],
    defs: Option<&AppearanceDefs>,
) -> Result<Option<Value>> {
    let (Some(map), Some(defs)) = (map, defs) else {
        return Ok(None);
    };
    // Realign before resolving — see [`realigned_material`]'s doc comment.
    let mut raw = serde_json::to_value(map)?;
    if !dropped.is_empty()
        && let Some(themes) = raw.as_object_mut()
    {
        for theme in themes.values_mut() {
            if let Some(values) = theme.get_mut("values") {
                realign_nested_values(values, depth, dropped);
            }
        }
    }
    let value = resolve_material_map(&raw, defs)?;
    Ok(Some(value))
}

/// One geometry's `texture` map — the [`realigned_nested_material`]
/// counterpart for `texture`.
fn realigned_nested_texture(
    map: &Option<HashMap<String, cjseq::Texture>>,
    depth: usize,
    dropped: &[usize],
    defs: Option<&AppearanceDefs>,
) -> Result<Option<Value>> {
    let (Some(map), Some(defs)) = (map, defs) else {
        return Ok(None);
    };
    // Realign before resolving — see [`realigned_material`]'s doc comment.
    let mut raw = serde_json::to_value(map)?;
    if !dropped.is_empty()
        && let Some(themes) = raw.as_object_mut()
    {
        for theme in themes.values_mut() {
            if let Some(values) = theme.get_mut("values") {
                realign_nested_values(values, depth, dropped);
            }
        }
    }
    let value = resolve_texture_map(&raw, defs)?;
    Ok(Some(value))
}

/// Normalises and dequantises one non-instance geometry against `pool`,
/// dereferencing its `material`/`texture` indices against `defs`.
fn normalise_geometry(
    geom: &Geometry,
    pool: &VertexPool,
    defs: Option<&AppearanceDefs>,
) -> Result<NormalisedGeometry> {
    match geom.thetype {
        GeometryType::GeometryInstance => Err(err(
            "normalise_geometry must not be called on a GeometryInstance",
        )),
        GeometryType::MultiPoint => {
            let idxs: Vec<usize> = parse_boundaries(geom)?;
            Ok(NormalisedGeometry {
                tree: points_node(pool, &idxs)?,
                semantics: canonical_semantics(
                    &geom.semantics,
                    &geom.boundaries,
                    &geom.thetype,
                    &[],
                ),
                material: realigned_material(&geom.material, &[], defs)?,
                texture: realigned_texture(&geom.texture, &[], defs)?,
                dropped_rings: 0,
                dropped_surfaces: 0,
            })
        }
        GeometryType::MultiLineString => {
            let lines: Vec<Vec<usize>> = parse_boundaries(geom)?;
            Ok(NormalisedGeometry {
                tree: ring_list_node(pool, &lines)?,
                semantics: canonical_semantics(
                    &geom.semantics,
                    &geom.boundaries,
                    &geom.thetype,
                    &[],
                ),
                material: realigned_material(&geom.material, &[], defs)?,
                texture: realigned_texture(&geom.texture, &[], defs)?,
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
                match normalise_surface(surface, pool, &mut dropped_rings)? {
                    Some(s) => kept.push(s),
                    None => dropped_positions.push(pos),
                }
            }
            Ok(NormalisedGeometry {
                tree: surface_list_node(pool, &kept)?,
                semantics: canonical_semantics(
                    &geom.semantics,
                    &geom.boundaries,
                    &geom.thetype,
                    &dropped_positions,
                ),
                material: realigned_material(&geom.material, &dropped_positions, defs)?,
                texture: realigned_texture(&geom.texture, &dropped_positions, defs)?,
                dropped_rings,
                dropped_surfaces: dropped_positions.len(),
            })
        }
        GeometryType::Solid => {
            let shells: Vec<Vec<Vec<Vec<usize>>>> = parse_boundaries(geom)?;
            let mut dropped_rings = 0usize;
            // Flat face positions, counted depth-first across shells —
            // matches `wkb_write::normalise_shells`'s `pos` counter, so
            // these line up with the writer-dropped positions the encoder
            // realigns against.
            let mut dropped_positions = Vec::new();
            let mut pos = 0usize;
            let mut kept_shells = Vec::with_capacity(shells.len());
            for shell in &shells {
                let mut kept_faces = Vec::with_capacity(shell.len());
                for face in shell {
                    match normalise_surface(face, pool, &mut dropped_rings)? {
                        Some(f) => kept_faces.push(f),
                        None => dropped_positions.push(pos),
                    }
                    pos += 1;
                }
                kept_shells.push(kept_faces);
            }
            let tree = Node::List(
                kept_shells
                    .iter()
                    .map(|faces| surface_list_node(pool, faces))
                    .collect::<Result<Vec<_>>>()?,
            );
            let depth = solid_face_nesting_depth(&geom.thetype).expect("Solid has a depth");
            Ok(NormalisedGeometry {
                tree,
                semantics: canonical_semantics(
                    &geom.semantics,
                    &geom.boundaries,
                    &geom.thetype,
                    &dropped_positions,
                ),
                material: realigned_nested_material(
                    &geom.material,
                    depth,
                    &dropped_positions,
                    defs,
                )?,
                texture: realigned_nested_texture(&geom.texture, depth, &dropped_positions, defs)?,
                dropped_rings,
                dropped_surfaces: dropped_positions.len(),
            })
        }
        GeometryType::MultiSolid | GeometryType::CompositeSolid => {
            let solids: Vec<Vec<Vec<Vec<Vec<usize>>>>> = parse_boundaries(geom)?;
            let mut dropped_rings = 0usize;
            // Flat face positions, counted depth-first across solids AND
            // shells — matches `wkb_write::normalise_shells`'s `pos`
            // counter, which is NOT reset between solids.
            let mut dropped_positions = Vec::new();
            let mut pos = 0usize;
            let mut kept_solids = Vec::with_capacity(solids.len());
            for shells in &solids {
                let mut kept_shells = Vec::with_capacity(shells.len());
                for shell in shells {
                    let mut kept_faces = Vec::with_capacity(shell.len());
                    for face in shell {
                        match normalise_surface(face, pool, &mut dropped_rings)? {
                            Some(f) => kept_faces.push(f),
                            None => dropped_positions.push(pos),
                        }
                        pos += 1;
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
            let depth = solid_face_nesting_depth(&geom.thetype).expect("MultiSolid has a depth");
            Ok(NormalisedGeometry {
                tree,
                semantics: canonical_semantics(
                    &geom.semantics,
                    &geom.boundaries,
                    &geom.thetype,
                    &dropped_positions,
                ),
                material: realigned_nested_material(
                    &geom.material,
                    depth,
                    &dropped_positions,
                    defs,
                )?,
                texture: realigned_nested_texture(&geom.texture, depth, &dropped_positions, defs)?,
                dropped_rings,
                dropped_surfaces: dropped_positions.len(),
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
    /// `Some` only for a kept `GeometryInstance` (`tree` is then just its
    /// reference point): the dereferenced content of the template it
    /// points to, plus its own `transformationMatrix`. See
    /// [`InstanceContent`] and [`resolve_instance`].
    instance: Option<InstanceContent>,
}

/// One `GeometryInstance`'s resolved content, dereferenced through THAT
/// side's own header `geometry-templates` — the `GeometryInstance`
/// counterpart of [`AppearanceDefs`]-based material/texture dereferencing
/// this module already does. Comparing this (see [`compare_object`])
/// alongside the reference point is what actually proves a
/// `GeometryInstance` round-trip preserved the INSTANCED geometry and its
/// placement, not merely its anchor: two datasets could share every
/// reference point while pointing at differently-shaped templates, or the
/// same template with a corrupted `transformationMatrix`, and a
/// reference-point-only comparison would call that "equal" (M4 final-review
/// Fix 1 — the earlier code did exactly that).
#[derive(Debug, Clone, PartialEq)]
struct InstanceContent {
    template_type: GeometryType,
    template_lod: Option<String>,
    /// The referenced template's own boundary tree, dequantised through a
    /// [`VertexPool::raw`] over that side's `vertices-templates` (raw
    /// floats, never the dataset's quantised `vertices` — CityJSON spec
    /// §3.4). Compared via [`values_equal`] after conversion to a plain JSON
    /// tree ([`node_to_value`]), NOT [`node_matches`]'s per-axis
    /// `coord_tolerance`: that tolerance is derived from the dataset's own
    /// quantisation scale, which has no bearing on a template's always-raw
    /// floats — the same reasoning [`resolve_texture_ring`]'s UV coordinates
    /// already follow.
    template_tree: Node,
    /// The instance's own `transformationMatrix`, compared under
    /// [`values_equal`] (a plain JSON array comparison, so its generic 1e-9
    /// relative float tolerance applies — again mirroring UV coordinates,
    /// not boundary coordinates).
    matrix: Vec<f64>,
    /// The referenced template's own `semantics`, realigned exactly like a
    /// non-instance geometry's (see [`normalise_geometry`]'s
    /// `MultiSurface`/`Solid` arms). `None` when the template carries none.
    semantics: Option<Value>,
    /// The referenced template's own `material`, DEREFERENCED against THAT
    /// side's RAW DOCUMENT appearance arrays (never `header.appearance` — a
    /// template's indices reference the raw document's arrays directly; see
    /// [`AppearanceDefs::from_doc_appearance`] and [`resolve_instance`]'s
    /// `defs` parameter). M4 Codex-review Finding 3: before this field
    /// existed, a corrupted template material/texture (or semantics) was
    /// invisible to the comparator entirely.
    material: Option<Value>,
    /// The referenced template's own `texture`, dereferenced the same way as
    /// [`Self::material`].
    texture: Option<Value>,
}

/// Converts a [`Node`] into a plain JSON value tree (points become `[x, y,
/// z]` arrays) so it can be compared via [`values_equal`] instead of
/// [`node_matches`]'s per-axis `coord_tolerance` — see
/// [`InstanceContent::template_tree`]'s docs for why that distinction
/// matters for template coordinates specifically.
fn node_to_value(n: &Node) -> Value {
    match n {
        Node::Point(p) => Value::from(vec![p[0], p[1], p[2]]),
        Node::List(items) => Value::Array(items.iter().map(node_to_value).collect()),
    }
}

/// Dereferences one `GeometryInstance`'s `template` index against `templates`
/// (that side's own `header.geometry-templates.templates`), `template_pool`
/// (a [`VertexPool::raw`] over that side's own `vertices-templates`), and
/// `defs` (that side's own RAW DOCUMENT [`AppearanceDefs`] — see
/// [`AppearanceDefs::from_doc_appearance`] — since a template's
/// `material`/`texture` indices reference the raw document's arrays
/// directly, never `header.appearance` nor a feature-local pool), producing
/// its comparable [`InstanceContent`]. A missing `template`
/// index or an index out of range of `templates` is a `Schema` error naming
/// the object — matching this module's existing style for malformed-side
/// handling (e.g. [`resolve_material_index`]'s out-of-range error), not a
/// silently-skipped comparison: a wrong template join is exactly the bug
/// class this fix exists to catch.
fn resolve_instance(
    geom: &Geometry,
    templates: &[Geometry],
    template_pool: &VertexPool,
    defs: &AppearanceDefs,
    label: &str,
    id: &str,
) -> Result<InstanceContent> {
    let idx = geom.template.ok_or_else(|| {
        err(format!(
            "{label}: object {id}: GeometryInstance has no 'template' index"
        ))
    })?;
    let template = templates.get(idx).ok_or_else(|| {
        err(format!(
            "{label}: object {id}: GeometryInstance template index {idx} out of range \
             (have {} templates)",
            templates.len()
        ))
    })?;
    // Templates' `material`/`texture` (when present) reference the HEADER's
    // own appearance arrays directly (mirrors `crate::export::rebuild_templates`'s
    // write-side counterpart) — `Some(defs)` here dereferences them exactly
    // like a non-instance geometry's, instead of the earlier `None` that
    // left the comparator blind to a corrupted template semantics/appearance
    // (M4 Codex-review Finding 3).
    let normalised = normalise_geometry(template, template_pool, Some(defs))?;
    let matrix: Vec<f64> = match &geom.transformation_matrix {
        Some(v) => serde_json::from_value(v.clone()).map_err(|e| {
            err(format!(
                "{label}: object {id}: GeometryInstance transformationMatrix malformed: {e}"
            ))
        })?,
        None => {
            return Err(err(format!(
                "{label}: object {id}: GeometryInstance has no transformationMatrix"
            )));
        }
    };
    Ok(InstanceContent {
        template_type: template.thetype.clone(),
        template_lod: template.lod.clone(),
        template_tree: normalised.tree,
        matrix,
        semantics: normalised.semantics,
        material: normalised.material,
        texture: normalised.texture,
    })
}

struct ObjectData {
    thetype: String,
    attributes: Value,
    parents: HashSet<String>,
    children: HashSet<String>,
    /// `child id -> role`, pairing `children[i]` with `children_roles[i]`
    /// (§5.1). Keyed by child rather than compared positionally so it is, like
    /// `children`, independent of the order the round-trip lists children in.
    children_roles: HashMap<String, String>,
    /// The source object's unmapped members (the `other` column, §5.1/G9): a
    /// per-object `geographicalExtent`, Extension `+members` — anything with
    /// no dedicated column. `address` has its own reserved column and its own
    /// comparison (see [`Self::address`]), so it is excluded here. Compared
    /// verbatim so a silently-dropped member registers as a difference. Same
    /// extraction the encoder uses, so the two sides never diverge on
    /// definition.
    other: Value,
    /// The object's `address[]` list, mapped to the reserved struct's fields
    /// (spec "Addresses", gap 10) — see [`address_comparables`].
    address: Vec<AddressCompare>,
    geometries: HashMap<Option<String>, NormGeometry>,
}

/// Extract a CityObject's `children_roles` paired with its `children` as a
/// `child id -> role` map (order-independent, like the `children` set).
/// `children_roles` lives only on `CityObjectGroup` (CityJSON 2.0.1 §2.5) and,
/// like everything in cjseq's private `#[serde(flatten)]`, is read through a
/// serialize round-trip — mirroring `crate::encode`.
///
/// This must not normalise malformation away, or a corrupt export could
/// false-pass against a valid source: it pairs over `max(children, roles)`, so
/// a surplus/missing role or a non-string entry yields a distinguishing
/// sentinel key/value rather than being silently truncated. For well-formed,
/// equal-length, aligned input it is a pure `child -> role` map, so an aligned
/// reorder compares equal.
fn children_roles_map(co: &cjseq::CityObject) -> HashMap<String, String> {
    if co.thetype != "CityObjectGroup" {
        return HashMap::new();
    }
    let children = co.children.clone().unwrap_or_default();
    let roles = serde_json::to_value(co)
        .ok()
        .and_then(|v| v.get("children_roles").cloned())
        .and_then(|v| match v {
            Value::Array(roles) => Some(roles),
            _ => None,
        })
        .unwrap_or_default();
    (0..children.len().max(roles.len()))
        .map(|i| {
            let key = children
                .get(i)
                .cloned()
                .unwrap_or_else(|| format!("\u{0}surplus_role_{i}"));
            let value = roles
                .get(i)
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| format!("\u{0}missing_or_nonstring_{i}"));
            (key, value)
        })
        .collect()
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
/// `%Y-%m-%d` dates compare as dates; integer-vs-integer numbers compare
/// EXACTLY (semantics/material/texture indices must never be equated by a
/// tolerance), while the 1e-9 relative f64 tolerance applies only when at
/// least one side is a genuine float (mixed int/float compares as float);
/// objects compare by key (null-valued entries excluded from the key set on
/// both sides — the null-vs-absent rule); arrays compare order-preserving,
/// element-wise.
fn values_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Null, Value::Null) => true,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Number(na), Value::Number(nb)) => {
            // Integer-vs-integer compares exactly (semantics/material/texture
            // indices); the 1e-9 relative tolerance exists only for genuine
            // floats. Mixed int/float (e.g. `2` vs `2.0`) falls through to
            // the float comparison below.
            if let (Some(x), Some(y)) = (na.as_i64(), nb.as_i64()) {
                return x == y;
            }
            if let (Some(x), Some(y)) = (na.as_u64(), nb.as_u64()) {
                return x == y;
            }
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

/// Canonicalise a raw source `geom.lod` string for use as a comparison key:
/// a source `"1"` and a source `"1.0"` are the same LoD (spec "Levels of
/// detail" — a canonicalisation of the LoD string, not its value), so an
/// original CityJSON's bare-major LoD must compare equal to the same LoD
/// re-exported in its canonical `"{major}.{minor}"` spelling. Falls back to
/// the raw string, unchanged, when it does not parse as a `Lod` — comparison
/// then simply falls back to exact string equality for that malformed value,
/// same as before this canonicalisation existed. `None` (a `GeometryInstance`
/// or a genuinely lod-less geometry) passes through unchanged.
fn canonical_lod_key(lod: &Option<String>) -> Option<String> {
    lod.as_ref().map(|s| {
        Lod::parse(s)
            .map(|l| l.to_string())
            .unwrap_or_else(|_| s.clone())
    })
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
/// everything it skips into `excluded`. `templates`/`template_pool`/
/// `template_defs` are that side's own header `geometry-templates` and
/// header-scope [`AppearanceDefs`] (see [`resolve_instance`]) — used only for
/// `GeometryInstance` geometries; `defs` (feature-scope) is unrelated and
/// used for every other geometry type.
#[allow(clippy::too_many_arguments)]
fn build_geometries(
    geoms: &[Geometry],
    pool: &VertexPool,
    defs: &AppearanceDefs,
    opts: &CompareOptions,
    excluded: &mut Vec<String>,
    label: &str,
    id: &str,
    templates: &[Geometry],
    template_pool: &VertexPool,
    template_defs: &AppearanceDefs,
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
            // The instance's CONTENT (its resolved template, dereferenced
            // through this side's own `geometry-templates`, plus its
            // `transformationMatrix`) — not just the reference point above —
            // is what actually proves the instanced geometry round-tripped.
            // See `InstanceContent`'s docs (M4 final-review Fix 1).
            let instance = Some(resolve_instance(
                geom,
                templates,
                template_pool,
                template_defs,
                label,
                id,
            )?);
            claim_or_log(
                &mut geometries,
                canonical_lod_key(&geom.lod),
                NormGeometry {
                    gtype: geom.thetype.clone(),
                    tree,
                    semantics: None,
                    material: None,
                    texture: None,
                    instance,
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

        // `skip_appearance` already decided this geometry's material/texture
        // are excluded from the comparison: hand `normalise_geometry` no
        // defs at all, so it never resolves (or errors on) appearance data
        // whose comparison is skipped anyway.
        let normalised = normalise_geometry(geom, pool, (!skip_appearance).then_some(defs))?;
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
            canonical_lod_key(&geom.lod),
            NormGeometry {
                gtype: geom.thetype.clone(),
                tree: normalised.tree,
                semantics: normalised.semantics,
                // Already `None` when `skip_appearance` (see the
                // `then_some(defs)` above).
                material: normalised.material,
                texture: normalised.texture,
                instance: None,
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

    // This side's own `geometry-templates`, dereferenced through by every
    // `GeometryInstance` below (see `resolve_instance`) — `vertices-templates`
    // are raw floats (CityJSON spec §3.4), so `VertexPool::raw` is used
    // rather than the dataset's quantised `VertexPool::new`. Empty when the
    // header carries no templates at all: any `GeometryInstance` encountered
    // in that case is then, correctly, a `Schema` error from `resolve_instance`.
    let (templates, template_vertices): (&[Geometry], Vec<Vec<f64>>) =
        match &header.geometry_templates {
            Some(gt) => {
                let verts: Vec<Vec<f64>> = serde_json::from_value(gt.vertices_templates.clone())
                    .map_err(|e| {
                        err(format!(
                            "{label}: header geometry-templates 'vertices-templates' malformed: {e}"
                        ))
                    })?;
                (gt.templates.as_slice(), verts)
            }
            None => (&[], Vec::new()),
        };
    let template_pool = VertexPool::raw(&template_vertices);
    // This side's own RAW DOCUMENT appearance arrays: a template's
    // `material`/`texture` indices reference these directly (never
    // `header.appearance`, and never a feature-local pool — see
    // [`AppearanceDefs::from_doc_appearance`] and [`resolve_instance`]).
    // Built once per side, like `templates`/`template_pool` above.
    let template_defs = AppearanceDefs::from_doc_appearance(&source);

    let mut objects = HashMap::new();
    let mut excluded = Vec::new();
    for feature in source.features()? {
        let feature = feature?;
        let pool = VertexPool::new(&feature.vertices, &header.transform);
        let defs = AppearanceDefs::from_feature(&feature);
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
                Some(geoms) => build_geometries(
                    geoms,
                    &pool,
                    &defs,
                    opts,
                    &mut excluded,
                    &label,
                    id,
                    templates,
                    &template_pool,
                    &template_defs,
                )?,
                None => HashMap::new(),
            };
            objects.insert(
                id.clone(),
                ObjectData {
                    thetype: co.thetype.clone(),
                    attributes,
                    parents,
                    children,
                    children_roles: children_roles_map(co),
                    other: Value::Object(crate::encode::unmapped_object_members(co)?),
                    address: address_comparables(co, &pool)?,
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

/// Which of one side's `metadata` members other than `referenceSystem`
/// (compared separately, in [`compare_datasets`]) are set, in the same
/// fixed order [`header_metadata_members`] reports them in. An EXHAUSTIVE
/// destructure of `cjseq::Metadata` (`reference_system` explicitly ignored,
/// every other field bound) rather than a hand-picked field list: a cjseq
/// upgrade that adds a `Metadata` member then fails to COMPILE here instead
/// of silently never appearing in `excluded` (M5 debt item 4 — drift-proofing
/// against exactly the kind of member the hand-picked 5-entry array used to
/// hard-code).
fn metadata_member_flags(m: Option<&cjseq::Metadata>) -> [bool; 5] {
    let Some(m) = m else {
        return [false; 5];
    };
    let cjseq::Metadata {
        geographical_extent,
        identifier,
        point_of_contact,
        reference_date,
        reference_system: _,
        title,
    } = m;
    [
        title.is_some(),
        geographical_extent.is_some(),
        identifier.is_some(),
        point_of_contact.is_some(),
        reference_date.is_some(),
    ]
}

/// The `metadata` members other than `referenceSystem` (compared above):
/// `("label", present)` pairs, where `present` is true when EITHER side's
/// `header.metadata` has that member set. These are documented exclusions
/// (M4-codex-5) — logged in `excluded`, never compared, never silently
/// dropped.
fn header_metadata_members(a: &CityJSON, b: &CityJSON) -> [(&'static str, bool); 5] {
    let fa = metadata_member_flags(a.metadata.as_ref());
    let fb = metadata_member_flags(b.metadata.as_ref());
    [
        ("title", fa[0] || fb[0]),
        ("geographicalExtent", fa[1] || fb[1]),
        ("identifier", fa[2] || fb[2]),
        ("pointOfContact", fa[3] || fb[3]),
        ("referenceDate", fa[4] || fb[4]),
    ]
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
    if a.children_roles != b.children_roles {
        out.push(format!(
            "object {id}: children_roles differ: {:?} vs {:?}",
            a.children_roles, b.children_roles
        ));
    }
    if !values_equal(&a.attributes, &b.attributes) {
        out.push(format!(
            "object {id}: attributes differ: {} vs {}",
            a.attributes, b.attributes
        ));
    }
    // Exact structural equality, NOT `values_equal`: `other` must round-trip
    // verbatim, so the attribute-oriented tolerances (numeric/date fuzz, and
    // treating a null-valued member as absent) would hide a real loss here.
    // serde_json object equality is already key-order-independent.
    if a.other != b.other {
        out.push(format!(
            "object {id}: unmapped members (other) differ: {} vs {}",
            a.other, b.other
        ));
    }

    // `address` (spec "Addresses", gap 10): postal fields compare exactly;
    // `location` compares as a coordinate tree, with the same tolerance as
    // any other geometry — round-tripping through WKB requantises it.
    if a.address.len() != b.address.len() {
        out.push(format!(
            "object {id}: address list length differs: {} vs {}",
            a.address.len(),
            b.address.len()
        ));
    } else {
        for (i, (aa, bb)) in a.address.iter().zip(&b.address).enumerate() {
            if aa.postal != bb.postal {
                out.push(format!(
                    "object {id}: address[{i}] postal fields differ: {:?} vs {:?}",
                    aa.postal, bb.postal
                ));
            }
            match (&aa.location, &bb.location) {
                (Some(la), Some(lb)) if !node_matches(la, lb, tol) => {
                    out.push(format!(
                        "object {id}: address[{i}] location coordinates differ"
                    ));
                }
                (Some(_), None) | (None, Some(_)) => {
                    out.push(format!(
                        "object {id}: address[{i}] location presence differs: {} vs {}",
                        aa.location.is_some(),
                        bb.location.is_some()
                    ));
                }
                _ => {}
            }
        }
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
        // `GeometryInstance`: `tree` above is just the reference point — the
        // instanced geometry ITSELF is `instance`'s resolved template
        // content plus `transformationMatrix` (M4 final-review Fix 1). Both
        // sides are `Some` here whenever `ga.gtype == gb.gtype ==
        // GeometryInstance` (the only way `build_geometries` produces a
        // `NormGeometry` of that type), so the `(None, None)` arm below only
        // guards non-instance geometries.
        match (&ga.instance, &gb.instance) {
            (Some(ia), Some(ib)) => {
                if ia.template_type != ib.template_type {
                    out.push(format!(
                        "object {id}: geometry at lod {lod:?}: instance template type differs: {:?} vs {:?}",
                        ia.template_type, ib.template_type
                    ));
                }
                if ia.template_lod != ib.template_lod {
                    out.push(format!(
                        "object {id}: geometry at lod {lod:?}: instance template lod differs: {:?} vs {:?}",
                        ia.template_lod, ib.template_lod
                    ));
                }
                if !values_equal(
                    &node_to_value(&ia.template_tree),
                    &node_to_value(&ib.template_tree),
                ) {
                    out.push(format!(
                        "object {id}: geometry at lod {lod:?}: instance template geometry (boundaries/coordinates) differs"
                    ));
                }
                let ma = serde_json::to_value(&ia.matrix).unwrap_or(Value::Null);
                let mb = serde_json::to_value(&ib.matrix).unwrap_or(Value::Null);
                if !values_equal(&ma, &mb) {
                    out.push(format!(
                        "object {id}: geometry at lod {lod:?}: instance transformationMatrix differs: {ma} vs {mb}"
                    ));
                }
                // M4 Codex-review Finding 3: the referenced template's own
                // semantics/material/texture, dereferenced through THAT
                // side's raw document appearance (see `resolve_instance`) —
                // before this, a corrupted template semantics or appearance
                // was invisible to the comparator.
                let sema = ia.semantics.clone().unwrap_or(Value::Null);
                let semb = ib.semantics.clone().unwrap_or(Value::Null);
                if !values_equal(&sema, &semb) {
                    out.push(format!(
                        "object {id}: geometry at lod {lod:?}: instance template semantics differ: {sema} vs {semb}"
                    ));
                }
                for (what, va, vb) in [
                    ("material", &ia.material, &ib.material),
                    ("texture", &ia.texture, &ib.texture),
                ] {
                    let na = va.clone().unwrap_or(Value::Null);
                    let nb = vb.clone().unwrap_or(Value::Null);
                    if !values_equal(&na, &nb) {
                        out.push(format!(
                            "object {id}: geometry at lod {lod:?}: instance template {what} differs: {na} vs {nb}"
                        ));
                    }
                }
            }
            (None, None) => {}
            _ => out.push(format!(
                "object {id}: geometry at lod {lod:?}: instance-ness differs between sides"
            )),
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
/// [`Source`]) for semantic equality: header `transform` (exact),
/// `referenceSystem`, `version`, and `extensions`, the object-id set, and per
/// common object its `type`, `parents`/`children` (as sets), `attributes`
/// (JSON-value equality with timestamp/date/numeric-tolerance
/// normalisation), and geometry per LoD (type, boundary-tree shape,
/// coordinates within `opts.coord_tolerance`, and `semantics`). Header
/// `metadata` members other than `referenceSystem` (`title`,
/// `geographicalExtent`, `identifier`, `pointOfContact`, `referenceDate`)
/// are documented exclusions: logged in `excluded` when either side sets
/// them, never compared, never silently dropped. See the module docs for
/// the degenerate-ring normalisation applied to both sides, and
/// [`Exclusions`] for what `export`'s deliberate drops let this skip
/// instead of flagging.
pub fn compare_datasets(a: &Path, b: &Path, opts: &CompareOptions) -> Result<CompareReport> {
    let side_a = load_side(a, opts)?;
    let side_b = load_side(b, opts)?;

    let mut differences = Vec::new();
    let mut excluded = side_a.excluded;
    excluded.extend(side_b.excluded);

    // `transform` is NOT compared for exact equality any more (spec-alignment
    // M3): `export` now SYNTHESISES its own quantisation transform rather
    // than reading `city.other.transform` verbatim (spec "Informational
    // only" — a reader/writer MUST NOT need `other` to decode the file; see
    // `crate::export::synthesize_transform`), so a source and its own
    // round-tripped export legitimately carry DIFFERENT transforms even when
    // every coordinate they quantise is identical. `transform` was always an
    // implementation-chosen encoding parameter, never semantic content — the
    // per-axis `tol` derived from `scale_a`/`scale_b` below (and every
    // per-object coordinate comparison against it) is what actually proves
    // losslessness.

    let rsa = reference_system_url(&side_a.header);
    let rsb = reference_system_url(&side_b.header);
    if rsa != rsb {
        differences.push(format!(
            "header: referenceSystem differs: {rsa:?} vs {rsb:?}"
        ));
    }

    if side_a.header.version != side_b.header.version {
        differences.push(format!(
            "header: version differs: {} vs {}",
            side_a.header.version, side_b.header.version
        ));
    }
    let ea = side_a.header.extensions.clone().unwrap_or(Value::Null);
    let eb = side_b.header.extensions.clone().unwrap_or(Value::Null);
    if !values_equal(&ea, &eb) {
        differences.push(format!("header: extensions differ: {ea} vs {eb}"));
    }
    // Metadata members other than referenceSystem are documented exclusions
    // (M4-codex-5): logged, never silently ignored, never a difference.
    for (label, present) in header_metadata_members(&side_a.header, &side_b.header) {
        if present {
            excluded.push(format!(
                "header: metadata member '{label}' not compared (documented exclusion)"
            ));
        }
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

    /// The real `lod3_railway.city.json` fixture carries no `referenceSystem`
    /// at all (a genuine open-data limitation). Since `scan` now hard-fails
    /// on coordinate-bearing input with no resolvable CRS (spec "CRS rules"),
    /// tests below that need a clean railway convert write a small on-disk
    /// COPY with a CRS injected via JSON mutation of the real fixture —
    /// never hand-written CityJSON.
    fn railway_fixture_with_crs() -> (tempfile::TempDir, std::path::PathBuf) {
        let mut doc: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(fixture("lod3_railway.city.json")).unwrap())
                .unwrap();
        doc["metadata"]["referenceSystem"] =
            serde_json::json!("https://www.opengis.net/def/crs/EPSG/0/7415");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("railway_with_crs.city.json");
        fs::write(&path, serde_json::to_string(&doc).unwrap()).unwrap();
        (dir, path)
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
    fn integer_json_numbers_compare_exactly() {
        use serde_json::json;
        // Indices in semantics/material/texture values arrays are integers;
        // a 1e-9 relative tolerance on large ints would equate distinct
        // indices.
        let a = json!(9_007_199_254_740_993_i64);
        let b = json!(9_007_199_254_740_992_i64);
        assert!(!values_equal(&a, &b), "adjacent large integers must differ");
        assert!(values_equal(&json!(7), &json!(7)));
        // Genuine floats keep the tolerance; mixed int/float compares as
        // float.
        assert!(values_equal(&json!(2.0000000000001), &json!(2.0)));
        assert!(values_equal(&json!(2), &json!(2.0)));
    }

    /// Pins the u64 comparator arm: values_equal handles unsigned 64-bit
    /// integers that exceed i64::MAX correctly.
    #[test]
    fn u64_json_numbers_compare_exactly() {
        use serde_json::json;
        assert!(values_equal(&json!(u64::MAX), &json!(u64::MAX)));
        assert!(!values_equal(&json!(u64::MAX), &json!(u64::MAX - 1)));
    }

    /// Derived-fixture pattern: copy delft, bump the header's `"version"` in
    /// the copy — a real mismatch that today's comparator (checking only
    /// `transform`/`referenceSystem`) silently ignores. Must be a
    /// `difference`.
    #[test]
    fn header_version_mismatch_is_a_difference() {
        let original = fixture("delft.city.jsonl");
        let text = fs::read_to_string(&original).unwrap();
        let mut lines: Vec<String> = text.lines().map(str::to_string).collect();

        let mut header: Value = serde_json::from_str(&lines[0]).unwrap();
        assert_eq!(header["version"], Value::String("2.0".to_string()));
        header["version"] = Value::String("1.1".to_string());
        lines[0] = serde_json::to_string(&header).unwrap();

        let dir = tempfile::tempdir().unwrap();
        let modified_path = dir.path().join("delft-version-bumped.city.jsonl");
        fs::write(&modified_path, lines.join("\n") + "\n").unwrap();

        let report =
            compare_datasets(&original, &modified_path, &CompareOptions::default()).unwrap();
        assert!(
            report
                .differences
                .iter()
                .any(|d| d.contains("header: version")),
            "a header version mismatch must be reported as a difference, got: {:#?}",
            report.differences
        );
    }

    /// Same derived-fixture pattern for `extensions`: inject an extension
    /// declaration into the copy's header. Must be a `difference`.
    #[test]
    fn header_extensions_mismatch_is_a_difference() {
        let original = fixture("delft.city.jsonl");
        let text = fs::read_to_string(&original).unwrap();
        let mut lines: Vec<String> = text.lines().map(str::to_string).collect();

        let mut header: Value = serde_json::from_str(&lines[0]).unwrap();
        header["extensions"] = serde_json::json!({
            "Noise": {"url": "x", "version": "1.0"}
        });
        lines[0] = serde_json::to_string(&header).unwrap();

        let dir = tempfile::tempdir().unwrap();
        let modified_path = dir.path().join("delft-extensions-added.city.jsonl");
        fs::write(&modified_path, lines.join("\n") + "\n").unwrap();

        let report =
            compare_datasets(&original, &modified_path, &CompareOptions::default()).unwrap();
        assert!(
            report
                .differences
                .iter()
                .any(|d| d.contains("header: extensions")),
            "a header extensions mismatch must be reported as a difference, got: {:#?}",
            report.differences
        );
    }

    /// Metadata members other than `referenceSystem` (already compared) are
    /// documented exclusions, not silent: comparing delft against itself
    /// must log `excluded` entries for the members delft's header actually
    /// sets (`title`, `geographicalExtent`), never a `difference`.
    #[test]
    fn header_metadata_members_are_logged_as_excluded_not_silently_ignored() {
        let path = fixture("delft.city.jsonl");
        let report = compare_datasets(&path, &path, &CompareOptions::default()).unwrap();
        assert!(report.differences.is_empty());
        for member in ["title", "geographicalExtent"] {
            assert!(
                report
                    .excluded
                    .iter()
                    .any(|e| e.starts_with("header: ") && e.contains(&format!("'{member}'"))),
                "delft's header sets '{member}'; it must be logged as an excluded header \
                 metadata member, got: {:#?}",
                report.excluded
            );
        }
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

    /// M4 task 4: before this fix, a Solid's `semantics`/`material` were
    /// cloned verbatim regardless of drops (the "not realigned" comment this
    /// change removes) — comparing a side with a degenerate face still
    /// embedded against an already-reduced counterpart would then report a
    /// spurious `semantics differ`/`material differs` difference even
    /// though both sides describe the same real geometry.
    ///
    /// Derived from delft's own `NL.IMBAG.Pand.0503100000012869-0` lod-1.2
    /// Solid (single shell, 6 faces, `semantics.values == [[0,2,2,2,2,1]]`):
    /// side A keeps face 2's exterior ring degenerate (`[a,b,a]`, built from
    /// the ring's own first two indices) with the full, not-yet-realigned
    /// 6-entry semantics/material arrays; side B has face 2 removed
    /// entirely from the boundaries AND the semantics/material arrays — the
    /// already-realigned shape a real writer round-trip produces. The two
    /// must compare equal: `normalise_geometry`'s own degenerate-face
    /// detection on side A must realign its semantics/material to match
    /// side B's, not merely detect the same face count.
    #[test]
    fn compare_realigns_solid_semantics_and_material_for_a_degenerate_face() {
        let original = fixture("delft.city.jsonl");
        let text = fs::read_to_string(&original).unwrap();
        let lines: Vec<String> = text.lines().map(str::to_string).collect();

        const OBJ_ID: &str = "NL.IMBAG.Pand.0503100000012869-0";
        let mut side_a_line = None;
        let mut side_b_line = None;
        for line in lines.iter().skip(1) {
            if !line.contains(OBJ_ID) {
                continue;
            }
            let feature: Value = serde_json::from_str(line).unwrap();

            fn find_solid(f: &mut Value) -> &mut Value {
                const OBJ_ID: &str = "NL.IMBAG.Pand.0503100000012869-0";
                f["CityObjects"][OBJ_ID]["geometry"]
                    .as_array_mut()
                    .unwrap()
                    .iter_mut()
                    .find(|g| g["lod"] == "1.2" && g["type"] == "Solid")
                    .expect("delft's Pand-0 must carry a lod 1.2 Solid")
            }

            let sem_values: Vec<i64> = {
                let mut probe = feature.clone();
                serde_json::from_value(find_solid(&mut probe)["semantics"]["values"][0].clone())
                    .unwrap()
            };
            assert_eq!(sem_values.len(), 6, "fixture fact: shell 0 has 6 faces");

            // Side A: face 2's exterior ring degenerates to [a, b, a]; full
            // 6-entry semantics/material arrays untouched (what a real,
            // not-yet-normalised source looks like).
            let mut feature_a = feature.clone();
            {
                let geom_a = find_solid(&mut feature_a);
                let ring = &mut geom_a["boundaries"][0][2][0];
                let indices: Vec<i64> = serde_json::from_value(ring.clone()).unwrap();
                let (a, b) = (indices[0], indices[1]);
                *ring = serde_json::json!([a, b, a]);
                geom_a["material"] =
                    serde_json::json!({"visual": {"values": [[0, 1, 0, 1, 0, 1]]}});
            }
            side_a_line = Some(serde_json::to_string(&feature_a).unwrap());

            // Side B: face 2 removed entirely from boundaries AND from the
            // semantics/material arrays.
            let mut feature_b = feature.clone();
            {
                let geom_b = find_solid(&mut feature_b);
                geom_b["boundaries"][0].as_array_mut().unwrap().remove(2);
                geom_b["semantics"]["values"][0]
                    .as_array_mut()
                    .unwrap()
                    .remove(2);
                geom_b["material"] = serde_json::json!({"visual": {"values": [[0, 1, 1, 0, 1]]}});
            }
            side_b_line = Some(serde_json::to_string(&feature_b).unwrap());
            break;
        }
        let side_a_line = side_a_line.expect("delft.city.jsonl must contain the target object");
        let side_b_line = side_b_line.unwrap();

        let header_line = lines[0].clone();
        let dir = tempfile::tempdir().unwrap();
        let path_a = dir.path().join("side_a.city.jsonl");
        let path_b = dir.path().join("side_b.city.jsonl");
        fs::write(&path_a, format!("{header_line}\n{side_a_line}\n")).unwrap();
        fs::write(&path_b, format!("{header_line}\n{side_b_line}\n")).unwrap();

        let report = compare_datasets(&path_a, &path_b, &CompareOptions::default()).unwrap();
        assert!(
            report.equal,
            "side A's degenerate face must realign to match side B's already-reduced shape, \
             got differences: {:#?}",
            report.differences
        );
    }

    /// M5 debt item 1: the encoder realigns degenerate-drop positions BEFORE
    /// rewriting appearance to dataset-global ids (`crate::encode`'s pipeline
    /// order), so a dangling/out-of-range material index sitting ONLY at a
    /// position the degenerate-ring normalisation is about to drop is
    /// perfectly fine real output. The comparator must mirror that order —
    /// realign first, resolve after — or it resolves the dangling index
    /// before it is ever dropped and errors on data a real round trip would
    /// happily accept. Derived from the same delft Solid as
    /// [`compare_realigns_solid_semantics_and_material_for_a_degenerate_face`]
    /// (degenerate face 2), but comparing the SAME file to itself: the
    /// feature gains a 2-entry `appearance.materials` array (so
    /// `resolve_material_index` actually bounds-checks instead of passing a
    /// bare index through) and the full, not-yet-realigned 6-entry
    /// `material.visual.values` carries an out-of-range index (99, no 3rd
    /// materials definition exists) at EXACTLY the degenerate face's
    /// position (2) — every other position references a valid index (0 or
    /// 1). `compare_datasets(derived, derived, default opts)` must be `Ok`
    /// and `equal`: resolve-before-realign order would instead surface a
    /// `Schema` error ("material index 99 out of range") on both sides
    /// before the drop ever gets a chance to remove it.
    #[test]
    fn compare_realigns_before_resolving_a_dangling_material_index_at_a_dropped_position() {
        let original = fixture("delft.city.jsonl");
        let text = fs::read_to_string(&original).unwrap();
        let lines: Vec<String> = text.lines().map(str::to_string).collect();

        const OBJ_ID: &str = "NL.IMBAG.Pand.0503100000012869-0";
        let mut derived_line = None;
        for line in lines.iter().skip(1) {
            if !line.contains(OBJ_ID) {
                continue;
            }
            let mut feature: Value = serde_json::from_str(line).unwrap();

            fn find_solid(f: &mut Value) -> &mut Value {
                const OBJ_ID: &str = "NL.IMBAG.Pand.0503100000012869-0";
                f["CityObjects"][OBJ_ID]["geometry"]
                    .as_array_mut()
                    .unwrap()
                    .iter_mut()
                    .find(|g| g["lod"] == "1.2" && g["type"] == "Solid")
                    .expect("delft's Pand-0 must carry a lod 1.2 Solid")
            }

            let sem_values: Vec<i64> = {
                let mut probe = feature.clone();
                serde_json::from_value(find_solid(&mut probe)["semantics"]["values"][0].clone())
                    .unwrap()
            };
            assert_eq!(sem_values.len(), 6, "fixture fact: shell 0 has 6 faces");

            // Face 2's exterior ring degenerates to [a, b, a] (same
            // derivation as the sibling test above); the full 6-entry
            // material array carries a real, in-range index at every KEPT
            // position and an out-of-range index ONLY at the dropped one.
            {
                let geom = find_solid(&mut feature);
                let ring = &mut geom["boundaries"][0][2][0];
                let indices: Vec<i64> = serde_json::from_value(ring.clone()).unwrap();
                let (a, b) = (indices[0], indices[1]);
                *ring = serde_json::json!([a, b, a]);
                geom["material"] = serde_json::json!({"visual": {"values": [[0, 1, 99, 1, 0, 1]]}});
            }
            feature["appearance"] = serde_json::json!({
                "materials": [{"name": "roof"}, {"name": "wall"}]
            });
            derived_line = Some(serde_json::to_string(&feature).unwrap());
            break;
        }
        let derived_line = derived_line.expect("delft.city.jsonl must contain the target object");

        let header_line = lines[0].clone();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("derived.city.jsonl");
        fs::write(&path, format!("{header_line}\n{derived_line}\n")).unwrap();

        let report = compare_datasets(&path, &path, &CompareOptions::default()).unwrap();
        assert!(
            report.equal,
            "comparing the derived file to itself must succeed: the dangling material index \
             sits only at the position the degenerate-ring drop removes before resolving, \
             got differences: {:#?}",
            report.differences
        );
    }

    /// M5 real-data finding, reproduced as a derived delft fixture: 3DBAG
    /// tile `9-284-556.city.json`, object
    /// `NL.IMBAG.Pand.0503100000025101-0`, LoD 2.2, face 497 has a 3-index
    /// exterior ring `[49590, 49127, 49595]` whose three DISTINCT vertex
    /// indices are all duplicate entries of the identical quantised vertex
    /// `(31653, 359040, -33533)` (verified against the raw tile JSON:
    /// `bench/data/9-284-556.city.json`, run `just bench-data` to fetch it —
    /// the CLI `convert`/`export`/`compare` sequence on just that one
    /// extracted object reproduces this exact "boundary/coordinates differ"
    /// and "semantics differ" failure, confirming this derived fixture
    /// exercises the same bug). Fixtures are always present (unlike
    /// `bench/data/`, network-only), so THIS is the binding gate; the real
    /// tile is manual-verification-only.
    ///
    /// Derived from delft's `NL.IMBAG.Pand.0503100000012869-0` lod-1.2 Solid
    /// (same object as the sibling degenerate-face tests above): face 2's
    /// exterior ring is replaced with 3 BRAND NEW `vertices` entries appended
    /// to the feature — 3 distinct indices (so the writer's own
    /// `normalise_ring`, deliberately index-only — see `crate::wkb_write`'s
    /// docs — does NOT drop this ring), each a bitwise-exact copy of
    /// `vertices[0]`'s raw quantised triple (so all 3 dequantise to the same
    /// coordinate) — the exact `9-284-556.city.json` shape, minimised.
    ///
    /// Mechanism this pins (see the module docs' "3DBAG tile" paragraph):
    /// the writer emits the ring as a real (degenerate) WKB ring; the WKB
    /// reader's coordinate interner (`crate::wkb_read::CoordInterner`,
    /// bitwise dedup) then collapses its 3 written points down to 1 repeated
    /// pool index on read. Before this fix, the OLD index-only comparator
    /// normalisation treated that repeated-index shape as closed-to-nothing
    /// and dropped it — but ONLY on the exported side (the source side's 3
    /// distinct indices are untouched by an index-only check) — so
    /// `compare_datasets` reported a spurious `boundary/coordinates differ` +
    /// `semantics differ` even though both sides describe the same
    /// (degenerate) face. After this fix, the SAME ring is dropped
    /// identically on both sides, and the two compare equal again.
    #[test]
    fn compare_drops_a_3dbag_style_index_distinct_coordinate_degenerate_ring_from_both_sides() {
        let original = fixture("delft.city.jsonl");
        let text = fs::read_to_string(&original).unwrap();
        let lines: Vec<String> = text.lines().map(str::to_string).collect();

        const OBJ_ID: &str = "NL.IMBAG.Pand.0503100000012869-0";
        let mut derived_line = None;
        for line in lines.iter().skip(1) {
            if !line.contains(OBJ_ID) {
                continue;
            }
            let mut feature: Value = serde_json::from_str(line).unwrap();

            // 3 brand new vertex entries, appended to the feature's own
            // `vertices` — index-distinct (3 new positions) but
            // coordinate-identical (bitwise copies of vertex 0's raw
            // quantised triple), mirroring the tile's duplicate-vertex data.
            let dup = feature["vertices"][0].clone();
            let base = {
                let vertices = feature["vertices"].as_array_mut().unwrap();
                let base = vertices.len() as i64;
                vertices.push(dup.clone());
                vertices.push(dup.clone());
                vertices.push(dup);
                base
            };

            let geom = feature["CityObjects"][OBJ_ID]["geometry"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|g| g["lod"] == "1.2" && g["type"] == "Solid")
                .expect("delft's Pand-0 must carry a lod 1.2 Solid");
            // Face 2's exterior ring: replaced wholesale with the 3 new,
            // coordinate-identical indices (the tile's ring shape exactly:
            // 3 distinct indices, 1 real point).
            geom["boundaries"][0][2][0] = serde_json::json!([base, base + 1, base + 2]);

            derived_line = Some(serde_json::to_string(&feature).unwrap());
            break;
        }
        let derived_line = derived_line.expect("delft.city.jsonl must contain the target object");

        let header_line = lines[0].clone();
        let dir = tempfile::tempdir().unwrap();
        let source_path = dir.path().join("delft_coord_degenerate.city.jsonl");
        fs::write(&source_path, format!("{header_line}\n{derived_line}\n")).unwrap();

        let package_dir = tempfile::tempdir().unwrap();
        crate::package::convert(&crate::package::ConvertOptions::new(
            source_path.clone(),
            package_dir.path().to_path_buf(),
        ))
        .unwrap();

        let export_dir = tempfile::tempdir().unwrap();
        let exported = export_dir.path().join("export.city.jsonl");
        crate::export::export(&crate::export::ExportOptions {
            package_dir: package_dir.path().to_path_buf(),
            output: exported.clone(),
        })
        .unwrap();

        let report = compare_datasets(&source_path, &exported, &CompareOptions::default()).unwrap();
        assert!(
            report.equal,
            "the coordinate-degenerate face must be normalised away identically on both sides \
             of the round trip; differences: {:#?}",
            report.differences
        );
        assert!(report.differences.is_empty());
        assert!(
            report
                .excluded
                .iter()
                .any(|e| e.contains("normalised away 1 degenerate ring(s), 1 surface(s)")),
            "expected the coordinate-degenerate ring/surface to be logged as an excluded \
             normalisation, got: {:#?}",
            report.excluded
        );
    }

    /// No real fixture carries MultiSolid/CompositeSolid (the same gap
    /// `crate::encode`'s `multisolid_shell_faces_nest_per_solid` documents),
    /// so `normalise_geometry`'s MultiSolid arm — specifically its `depth ==
    /// 2` realignment — is exercised directly here: 2 solids, drops
    /// spanning both a shell-within-solid boundary (solid0 shell1's only
    /// face) and the last face overall (solid1 shell0's second face).
    #[test]
    fn normalise_geometry_realigns_multisolid_semantics_and_material_across_solids_and_shells() {
        let geom: Geometry = serde_json::from_value(serde_json::json!({
            "type": "MultiSolid",
            "lod": "2",
            "boundaries": [
                [
                    [[[0, 1, 2]], [[1, 2, 3]]],
                    [[[0, 1, 0]]]
                ],
                [
                    [[[0, 2, 3]], [[2, 3, 2]]]
                ]
            ],
            "semantics": {
                "surfaces": [{"type": "A"}],
                "values": [[[10, 11], [12]], [[13, 14]]]
            },
            "material": {"visual": {"values": [[[1, 2], [3]], [[4, 5]]]}}
        }))
        .unwrap();
        let vertices: Vec<Vec<i64>> = vec![
            vec![0, 0, 0],
            vec![1000, 0, 0],
            vec![1000, 1000, 0],
            vec![0, 1000, 0],
        ];
        let transform = Transform {
            scale: vec![1.0; 3],
            translate: vec![0.0; 3],
        };
        let pool = VertexPool::new(&vertices, &transform);

        let normalised = normalise_geometry(&geom, &pool, Some(&AppearanceDefs::empty())).unwrap();
        assert_eq!(normalised.dropped_rings, 2);
        assert_eq!(normalised.dropped_surfaces, 2);
        // Semantics are canonicalised to the flat face_semantics form (§8), so
        // the solid/shell nesting collapses to one entry per emitted face.
        assert_eq!(
            normalised.semantics.unwrap()["face_semantics"],
            serde_json::json!([10, 11, 13]),
            "semantics canonicalise to a flat per-face list, dropping positions 2 and 4"
        );
        // Material still uses the nested CityJSON theme values (§11.1), realigned
        // across the solid/shell nesting.
        assert_eq!(
            normalised.material.unwrap()["visual"]["values"],
            serde_json::json!([[[1, 2], []], [[4]]]),
            "material must realign across the solid/shell nesting"
        );
    }

    /// Reviewer follow-up (M4 task 11): a malformed `vertices-texture`
    /// entry with fewer than 2 coordinates — something only a hand-edited
    /// or third-party file can carry, never this crate's own writer — must
    /// surface as a `Schema` error naming the entry, not an
    /// index-out-of-bounds panic in `resolve_texture_ring`. Derived from a
    /// real railway feature's appearance (sanctioned modification): the
    /// entry referenced by the first UV index of the first texture ring is
    /// truncated to a single element.
    #[test]
    fn malformed_vertices_texture_entry_is_an_error_not_a_panic() {
        let source = Source::open(&fixture("lod3_railway.city.json")).unwrap();
        let header = source.header().clone();

        // Innermost-ring walk: the first ring's first UV index, if any.
        fn first_uv_index(v: &Value) -> Option<usize> {
            let items = v.as_array()?;
            if is_texture_ring(items) {
                items.get(1)?.as_u64().map(|n| n as usize)
            } else {
                items.iter().find_map(first_uv_index)
            }
        }

        for feature in source.features().unwrap() {
            let feature = feature.unwrap();
            let Some(appearance) = &feature.appearance else {
                continue;
            };
            let Some(vertices_texture) = &appearance.vertices_texture else {
                continue;
            };
            let pool = VertexPool::new(&feature.vertices, &header.transform);
            for co in feature.city_objects.values() {
                let Some(geoms) = &co.geometry else { continue };
                for geom in geoms {
                    let Some(texture) = &geom.texture else {
                        continue;
                    };
                    let raw = serde_json::to_value(texture).unwrap();
                    let Some(uv_idx) = raw
                        .as_object()
                        .and_then(|themes| themes.values().find_map(|t| t.get("values")))
                        .and_then(first_uv_index)
                    else {
                        continue;
                    };

                    // Derived modification: the referenced entry loses its
                    // second coordinate.
                    let mut corrupted = vertices_texture.clone();
                    corrupted[uv_idx].truncate(1);

                    let defs = AppearanceDefs {
                        materials: appearance.materials.as_deref().unwrap_or(&[]),
                        textures: appearance.textures.as_deref().unwrap_or(&[]),
                        vertices_texture: &corrupted,
                    };
                    let err = match normalise_geometry(geom, &pool, Some(&defs)) {
                        Ok(_) => panic!(
                            "expected an error for the malformed vertex-texture entry {uv_idx}"
                        ),
                        Err(e) => e,
                    };
                    assert!(
                        matches!(err, CityParquetError::Schema(_)),
                        "expected a Schema error, got {err:?}"
                    );
                    let msg = err.to_string();
                    assert!(
                        msg.contains(&format!("vertex-texture entry {uv_idx}")),
                        "the error must name the malformed entry {uv_idx}, got: {msg}"
                    );
                    return;
                }
            }
        }
        panic!("railway must contain at least one textured geometry with vertices-texture");
    }

    /// M4 final-review Fix 1: before this fix, a kept `GeometryInstance`
    /// compared ONLY by its reference point — the `template` index and
    /// `transformationMatrix` were silently uncompared, so a wrong template
    /// join or a corrupted matrix would pass every gate. Derived from the
    /// real railway fixture (sanctioned modification): the first
    /// `GeometryInstance` found gets its `transformationMatrix` changed in a
    /// copy (the X-axis scale term, matrix index 0, `1.0 -> 3.0`);
    /// `compare_datasets` must report exactly one difference naming that
    /// object and mentioning the matrix.
    #[test]
    fn compare_detects_a_changed_geometry_instance_transformation_matrix() {
        let original = fixture("lod3_railway.city.json");
        let text = fs::read_to_string(&original).unwrap();
        let mut doc: Value = serde_json::from_str(&text).unwrap();

        let mut changed_id: Option<String> = None;
        {
            let objects = doc["CityObjects"].as_object_mut().unwrap();
            'outer: for (id, co) in objects.iter_mut() {
                let Some(geoms) = co.get_mut("geometry").and_then(Value::as_array_mut) else {
                    continue;
                };
                for g in geoms.iter_mut() {
                    if g.get("type").and_then(Value::as_str) != Some("GeometryInstance") {
                        continue;
                    }
                    let matrix = g
                        .get_mut("transformationMatrix")
                        .and_then(Value::as_array_mut)
                        .expect("a GeometryInstance must carry a transformationMatrix");
                    matrix[0] = serde_json::json!(3.0);
                    changed_id = Some(id.clone());
                    break 'outer;
                }
            }
        }
        let changed_id = changed_id.expect("railway must have at least one GeometryInstance");

        let dir = tempfile::tempdir().unwrap();
        let modified_path = dir.path().join("railway-matrix-modified.city.json");
        fs::write(&modified_path, serde_json::to_string(&doc).unwrap()).unwrap();

        let report =
            compare_datasets(&original, &modified_path, &CompareOptions::default()).unwrap();
        assert!(
            !report.equal,
            "a changed GeometryInstance transformationMatrix must be detected, not silently accepted"
        );
        assert!(
            report
                .differences
                .iter()
                .any(|d| d.contains(&changed_id) && d.contains("transformationMatrix")),
            "the difference must name object {changed_id} and mention transformationMatrix, got: {:#?}",
            report.differences
        );
    }

    /// [`compare_detects_a_changed_geometry_instance_transformation_matrix`]'s
    /// counterpart for the `template` index itself: swapping which template a
    /// `GeometryInstance` references (to a template with genuinely different
    /// content — railway's 3 templates differ in vertex count) must be a
    /// difference too, proving the comparator dereferences `template`
    /// through each side's own `geometry-templates` rather than ignoring it.
    #[test]
    fn compare_detects_a_swapped_geometry_instance_template_index() {
        let original = fixture("lod3_railway.city.json");
        let text = fs::read_to_string(&original).unwrap();
        let mut doc: Value = serde_json::from_str(&text).unwrap();

        let template_count = doc["geometry-templates"]["templates"]
            .as_array()
            .expect("railway has geometry-templates")
            .len();
        assert!(template_count > 1, "need at least 2 templates to swap");

        let mut changed_id: Option<String> = None;
        {
            let objects = doc["CityObjects"].as_object_mut().unwrap();
            'outer: for (id, co) in objects.iter_mut() {
                let Some(geoms) = co.get_mut("geometry").and_then(Value::as_array_mut) else {
                    continue;
                };
                for g in geoms.iter_mut() {
                    if g.get("type").and_then(Value::as_str) != Some("GeometryInstance") {
                        continue;
                    }
                    let old = g
                        .get("template")
                        .and_then(Value::as_u64)
                        .expect("a GeometryInstance must carry a template index")
                        as usize;
                    let new = (old + 1) % template_count;
                    g["template"] = serde_json::json!(new);
                    changed_id = Some(id.clone());
                    break 'outer;
                }
            }
        }
        let changed_id = changed_id.expect("railway must have at least one GeometryInstance");

        let dir = tempfile::tempdir().unwrap();
        let modified_path = dir.path().join("railway-template-swapped.city.json");
        fs::write(&modified_path, serde_json::to_string(&doc).unwrap()).unwrap();

        let report =
            compare_datasets(&original, &modified_path, &CompareOptions::default()).unwrap();
        assert!(
            !report.equal,
            "a swapped GeometryInstance template index must be detected, not silently accepted"
        );
        assert!(
            report.differences.iter().any(|d| d.contains(&changed_id)),
            "the difference must name object {changed_id}, got: {:#?}",
            report.differences
        );
    }

    /// Converts the real railway fixture (which carries materials/textures,
    /// written unconditionally now that sidecars are content-gated rather
    /// than profile-gated — spec-alignment gap 19) and exports it back to a
    /// single `.city.json` DOCUMENT (not Seq): templates' `material`/`texture`
    /// are localised at HEADER scope by `crate::export::rebuild_templates`, so
    /// this is the shape whose corruption the M4 Codex-review Finding 3 tests
    /// below target. Returns the exported file's path and the tempdirs
    /// backing it (kept alive for the caller).
    fn compat_railway_export_doc() -> (std::path::PathBuf, tempfile::TempDir, tempfile::TempDir) {
        use crate::export::{ExportOptions, export};
        use crate::package::{ConvertOptions, convert};

        let package_dir = tempfile::tempdir().unwrap();
        let (_crs_dir, railway_path) = railway_fixture_with_crs();
        let opts = ConvertOptions::new(railway_path, package_dir.path().to_path_buf());
        convert(&opts).unwrap();

        let export_dir = tempfile::tempdir().unwrap();
        let output = export_dir.path().join("export.city.json");
        export(&ExportOptions {
            package_dir: package_dir.path().to_path_buf(),
            output: output.clone(),
        })
        .unwrap();
        (output, package_dir, export_dir)
    }

    /// M4 Codex-review Finding 3: before the fix, `resolve_instance` passed
    /// `None` appearance defs and never even read the template's own
    /// `material`/`texture`, so a `GeometryInstance`'s round-trip proof
    /// stopped at boundaries/type/lod — a corrupted template appearance was
    /// completely invisible to the no-exclusions gate. Derived from the real
    /// railway package (sanctioned): export to `.city.json` (templates'
    /// material/texture become HEADER-scope indices — see
    /// `crate::export::rebuild_templates`), then in a mutated copy repoint
    /// template 1's `material.visual.value` from its real index (`1`) to a
    /// DIFFERENT, still-valid header material index (`0`) — a corruption
    /// that stays entirely within the header's own (85-entry) `appearance`
    /// array, so it can never be a mere out-of-range Schema error.
    #[test]
    fn compare_detects_a_mutated_geometry_instance_template_material() {
        let (original, _package_dir, _export_dir) = compat_railway_export_doc();

        let text = fs::read_to_string(&original).unwrap();
        let mut doc: Value = serde_json::from_str(&text).unwrap();

        let templates = doc["geometry-templates"]["templates"]
            .as_array_mut()
            .expect("exported doc must carry geometry-templates");
        assert_eq!(templates.len(), 3, "railway must carry exactly 3 templates");
        let material_value = templates[1]["material"]["visual"]["value"].clone();
        assert_eq!(
            material_value,
            serde_json::json!(1),
            "precondition: template 1's material.visual.value starts at header index 1"
        );
        // Repoint to a DIFFERENT, still-valid header material index — the
        // corruption must stay a real material-content mismatch, never an
        // out-of-range Schema error.
        templates[1]["material"]["visual"]["value"] = serde_json::json!(0);
        let n_materials = doc["appearance"]["materials"]
            .as_array()
            .expect("exported header must carry appearance.materials")
            .len();
        assert!(
            n_materials > 1,
            "need at least 2 header materials to repoint to a genuinely different one, got {n_materials}"
        );

        let dir = tempfile::tempdir().unwrap();
        let mutated_path = dir
            .path()
            .join("railway-template-material-mutated.city.json");
        fs::write(&mutated_path, serde_json::to_string(&doc).unwrap()).unwrap();

        let report =
            compare_datasets(&original, &mutated_path, &CompareOptions::default()).unwrap();
        assert!(
            !report.equal,
            "a mutated GeometryInstance template material must be detected, not silently accepted"
        );
        assert!(
            report.differences.iter().any(|d| d.contains("instance")
                && d.contains("template")
                && d.contains("material")),
            "expected a difference naming the instance template material, got: {:#?}",
            report.differences
        );
    }

    /// [`compare_detects_a_mutated_geometry_instance_template_material`]'s
    /// counterpart for `semantics`. Railway's own 3 templates carry NO
    /// `semantics` member at all (checked directly against the fixture), so
    /// there is no existing value to mutate — instead, per the M4
    /// Codex-review Finding 3 brief, this ADDS a `semantics` block to one
    /// template on one side only: absence-vs-presence must be a difference,
    /// not silence, proving the comparator actually reads
    /// `InstanceContent::semantics` rather than defaulting both sides to
    /// `None` unconditionally.
    #[test]
    fn compare_detects_an_added_geometry_instance_template_semantics_block() {
        let (original, _package_dir, _export_dir) = compat_railway_export_doc();

        let text = fs::read_to_string(&original).unwrap();
        let mut doc: Value = serde_json::from_str(&text).unwrap();

        let templates = doc["geometry-templates"]["templates"]
            .as_array_mut()
            .expect("exported doc must carry geometry-templates");
        assert!(
            templates[0].get("semantics").is_none(),
            "precondition: railway's templates carry no semantics of their own"
        );
        templates[0]["semantics"] = serde_json::json!({
            "surfaces": [{"type": "WallSurface"}],
            "values": [0],
        });

        let dir = tempfile::tempdir().unwrap();
        let mutated_path = dir
            .path()
            .join("railway-template-semantics-added.city.json");
        fs::write(&mutated_path, serde_json::to_string(&doc).unwrap()).unwrap();

        let report =
            compare_datasets(&original, &mutated_path, &CompareOptions::default()).unwrap();
        assert!(
            !report.equal,
            "an added GeometryInstance template semantics block must be detected, not silently accepted"
        );
        assert!(
            report.differences.iter().any(|d| d.contains("instance")
                && d.contains("template")
                && d.contains("semantics")),
            "expected a difference naming the instance template semantics, got: {:#?}",
            report.differences
        );
    }
}
