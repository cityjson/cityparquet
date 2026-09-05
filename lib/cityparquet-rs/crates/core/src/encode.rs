//! Pass 2: encode a [`Source`] into `RecordBatch`es conforming exactly to
//! [`ScanResult::schema`]'s rendered Arrow schema.
//!
//! One row per `CityObject` (parents and children both get their own row);
//! geometry is bucketed into per-LoD columns (or a single un-suffixed
//! `geometry` column when the dataset has no LoDs at all, per
//! [`crate::scan`]'s binding rule). This pass never re-scans for schema
//! shape — it trusts the [`ScanResult`] passed in and fails fast if asked to
//! encode a dataset the schema doesn't describe.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use arrow_array::builder::{
    BinaryBuilder, BooleanBuilder, Date32Builder, Float64Builder, Int64Builder, ListBuilder,
    StringBuilder, StringDictionaryBuilder, StructBuilder, TimestampMillisecondBuilder,
};
use arrow_array::types::Int32Type;
use arrow_array::{ArrayRef, Float64Array, RecordBatch, StructArray};
use arrow_buffer::NullBufferBuilder;
use arrow_schema::{DataType, Field, Schema};
use cjseq::{CityJSON, CityJSONFeature, CityObject, Geometry, GeometryType, Transform};
use serde_json::Value;

use cityparquet_schema::{AttributeType, CityParquetError, Lod, Result, normalise_attribute_name};

use crate::appearance::AppearanceInterner;
use crate::appearance_columns::{
    MaterialCell, MaterialCellBuilder, TextureCell, TextureCellBuilder,
};
use crate::geometry_properties::{GeometryProperties, GeometryPropertiesBuilder};
use crate::scan::ScanResult;
use crate::source::{FeatureIter, Source};
use crate::wkb_write::{VertexPool, geometry_bbox, geometry_to_wkb, point_to_wkb};

/// Members carried by a dedicated column, stripped from the `other` payload
/// (§5.1). `children_roles` rides the flatten member but has its own column
/// (G5); `address` has its own reserved struct column; `geographicalExtent`
/// is carried by `bbox`, which unions it with the object's computed subtree
/// extent; the rest are cjseq's typed fields.
pub(crate) const OTHER_RESERVED_MEMBERS: [&str; 8] = [
    "type",
    "attributes",
    "geometry",
    "children",
    "parents",
    "children_roles",
    "address",
    "geographicalExtent",
];

/// The object serialised to its JSON member map — computed ONCE per row by
/// [`RowWriter::push_object`] and shared by every consumer needing members
/// cjseq has no typed field for (`children_roles`, `address`, the `other`
/// leftovers), instead of each consumer re-serialising the whole object
/// (review P5b: three whole-object serialisations per row).
pub(crate) fn object_json(co: &CityObject) -> Result<serde_json::Map<String, Value>> {
    match serde_json::to_value(co)? {
        Value::Object(map) => Ok(map),
        _ => Ok(serde_json::Map::new()),
    }
}

/// The `other` payload from an already-serialised member map: every member
/// not carried by a dedicated column (§5.1, G9). Consumes the map — the
/// leftovers ARE the return value, no clone.
pub(crate) fn unmapped_from_json(
    mut co_json: serde_json::Map<String, Value>,
) -> serde_json::Map<String, Value> {
    for key in OTHER_RESERVED_MEMBERS {
        co_json.remove(key);
    }
    co_json
}

/// The source object's unmapped members — every member not carried by a
/// dedicated column — as the `other` payload (§5.1, G9). Empty when the object
/// has none; cjseq skips `None` typed fields, so no null members appear.
/// Standalone convenience over [`object_json`] + [`unmapped_from_json`] for
/// callers outside the row loop (the comparator); `push_object` uses the split
/// pieces to serialise the object exactly once per row.
pub(crate) fn unmapped_object_members(co: &CityObject) -> Result<serde_json::Map<String, Value>> {
    Ok(unmapped_from_json(object_json(co)?))
}

/// Collect an object's diverted attributes (those whose name is in
/// `diverted`) into a plain JSON map keyed by source attribute name — merged
/// into the `other` column's cell value alongside the object's unmapped
/// members (spec "Column naming and reservation rules"; gap 14 — this used
/// to ride inside `other` under a `cityparquet:diverted_attributes`
/// transport key, then briefly its own dedicated reserved column, now
/// folded back into `other` itself since a reader restores every `other`
/// entry into `attributes` regardless of why it is there). `None` when there
/// is nothing to divert for this row. Null attribute values are skipped —
/// the column path drops them and the comparator treats null as absent, so
/// keeping them would make the diverted path spuriously non-null and
/// inconsistent.
fn collect_diverted_attributes(
    co: &CityObject,
    diverted: &[String],
) -> Option<serde_json::Map<String, Value>> {
    if diverted.is_empty() {
        return None;
    }
    let attrs = co.attributes.as_ref().and_then(Value::as_object)?;
    let mut map = serde_json::Map::new();
    for name in diverted {
        if let Some(v) = attrs.get(name)
            && !v.is_null()
        {
            map.insert(name.clone(), v.clone());
        }
    }
    (!map.is_empty()).then_some(map)
}

/// Counters for the row-population edge cases the binding rules ask us to
/// track rather than surface as errors.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EncodeStats {
    /// Extra geometries for an (object, LoD) pair beyond the first one kept.
    pub skipped_same_lod_geometries: usize,
    /// Attribute values present (non-null) but not representable as the
    /// column's inferred type; encoded as null instead of panicking.
    pub attribute_coercion_nulls: usize,
    /// Rings the writer dropped because they had fewer than 3 vertices to
    /// begin with (see `wkb_write::normalise_ring` — stripping a pre-baked
    /// WKB closure never reduces a ring below 3), counted over STORED
    /// geometries.
    pub degenerate_rings_dropped: usize,
    /// Surfaces the writer dropped because their exterior ring was one of
    /// the above, counted over STORED geometries.
    pub degenerate_surfaces_dropped: usize,
    /// Attribute values diverted into the `other` column because their name
    /// collides with a reserved/geometry column name (§5.2, G12). Counted
    /// over all objects: a diverted attribute is preserved but is not
    /// a queryable column, so the conversion report surfaces it (see
    /// [`ScanResult::diverted_attribute_names`] for the names).
    pub diverted_attribute_values: usize,
    /// LoD0 footprints synthesised into the `geometry_lod0_0` column for
    /// objects lacking a source LoD0 (spec "LoD0 synthesis").
    pub synthesized_lod0_footprints: usize,
    /// Unmapped top-level members dropped because an attribute of the same
    /// name exists on the object (diverted or column-backed): the attribute
    /// wins ("warn and prefer attribute"), so `other` never carries an entry
    /// duplicating one of the object's own attributes — a reader
    /// MUST-errors on that (`merge_other_members` in `src/decode.rs`), so
    /// keeping the member would make this writer produce a file its own
    /// reader rejects. Counted over all objects; see
    /// [`Self::dropped_colliding_member_diagnostics`] for the per-drop detail.
    pub dropped_colliding_members: usize,
    /// One diagnostic per member [`Self::dropped_colliding_members`] counts,
    /// naming the object id and the colliding key — the same
    /// library-diagnostic idiom as [`crate::scan::ScanResult::crs_diagnostic`],
    /// which the CLI prints as a `warning:` line.
    pub dropped_colliding_member_diagnostics: Vec<String>,
}

/// Expand `acc` to also cover `bbox` (same union rule as [`crate::scan`]'s).
fn union_bbox(acc: &mut Option<[f64; 6]>, bbox: [f64; 6]) {
    *acc = Some(match acc.take() {
        None => bbox,
        Some(mut cur) => {
            for i in 0..3 {
                cur[i] = cur[i].min(bbox[i]);
                cur[i + 3] = cur[i + 3].max(bbox[i + 3]);
            }
            cur
        }
    });
}

/// The source's declared per-object `geographicalExtent` as a bbox, when it
/// is well-formed — exactly six finite numbers (CityJSON 2.0.1 §2). A
/// malformed extent is ignored rather than fatal: it is an optional,
/// derivable source member, and `bbox` is fully recoverable from geometry.
fn source_extent(co: &CityObject) -> Option<[f64; 6]> {
    let extent: [f64; 6] = co
        .geographical_extent
        .as_ref()?
        .as_slice()
        .try_into()
        .ok()?;
    extent.iter().all(|v| v.is_finite()).then_some(extent)
}

/// Union of bboxes over an object's own geometries only (no descendant
/// walk) — via the bbox-only walker, not a full WKB encode whose bytes were
/// discarded (review P4; the walker's bbox is bitwise-equal by test).
fn own_geometry_bbox(co: &CityObject, pool: &VertexPool) -> Result<Option<[f64; 6]>> {
    let mut acc = None;
    if let Some(geoms) = &co.geometry {
        for geom in geoms {
            if let Some(bbox) = geometry_bbox(geom, pool)? {
                union_bbox(&mut acc, bbox);
            }
        }
    }
    Ok(acc)
}

/// Recursive descendant-bbox union: every descendant's own bbox, unioned
/// over the whole subtree (a child's own geometry does not stop the walk —
/// its own children may extend further), cycle guarded with a visited set.
fn descendant_bbox(
    co: &CityObject,
    feature: &CityJSONFeature,
    pool: &VertexPool,
    visited: &mut HashSet<String>,
) -> Result<Option<[f64; 6]>> {
    let mut acc = None;
    if let Some(children) = &co.children {
        for child_id in children {
            if !visited.insert(child_id.clone()) {
                continue;
            }
            let Some(child) = feature.city_objects.get(child_id) else {
                continue;
            };
            if let Some(bbox) = own_geometry_bbox(child, pool)? {
                union_bbox(&mut acc, bbox);
            }
            if let Some(bbox) = descendant_bbox(child, feature, pool, visited)? {
                union_bbox(&mut acc, bbox);
            }
        }
    }
    Ok(acc)
}

/// `bbox` binding rule: the union of the object's own geometry bboxes and a
/// cycle-guarded recursive union over its whole descendant subtree (spec
/// "Spatial metadata"). `None` only when nothing in the subtree has
/// geometry. Unioned, not own-first: a `Building` carrying a flat LoD0
/// footprint over solid `BuildingPart`s would otherwise get a z-flat box
/// that prunes the building away from any query above ground.
fn resolve_bbox(
    own_bbox: Option<[f64; 6]>,
    id: &str,
    co: &CityObject,
    feature: &CityJSONFeature,
    pool: &VertexPool,
) -> Result<Option<[f64; 6]>> {
    let mut acc = own_bbox;
    let mut visited = HashSet::new();
    visited.insert(id.to_string());
    if let Some(bbox) = descendant_bbox(co, feature, pool, &mut visited)? {
        union_bbox(&mut acc, bbox);
    }
    Ok(acc)
}

/// Count how many writer-dropped flat face positions fall inside one
/// shell's `[pos, pos + n)` range, advancing `pos` past the shell.
fn dropped_in_shell(dropped: &[usize], pos: &mut usize, n: usize) -> usize {
    let start = *pos;
    *pos += n;
    dropped
        .iter()
        .filter(|&&p| p >= start && p < start + n)
        .count()
}

/// A `usize` face count as an `i32`, erroring rather than silently wrapping
/// on the (never realistically reachable, but untrusted-input-adjacent)
/// overflow case — `geometry_properties.shells` is a native `LIST<INT>`
/// (spec), so this is the one place a shell face count crosses into Arrow's
/// 32-bit domain.
fn face_count_i32(n: usize) -> Result<i32> {
    i32::try_from(n)
        .map_err(|_| CityParquetError::Schema(format!("shell face count {n} exceeds i32::MAX")))
}

/// The `shells` payload of `geometry_properties` (§8): the STORED (post-drop)
/// per-shell face count, **always nested one inner list per solid** (spec
/// "Geometry properties and semantics" — a `Solid` gets `[[12]]`, never the
/// flat `[12]`, so a reader never special-cases `Solid` vs `MultiSolid`/
/// `CompositeSolid`). `None` for the non-solid types. `dropped` are the
/// writer-reported flat face positions removed from the WKB; each shell's
/// count is reduced by the drops inside it, so the total equals the WKB face
/// count.
fn solid_shells(geom: &Geometry, dropped: &[usize]) -> Result<Option<Vec<Vec<i32>>>> {
    match geom.thetype {
        GeometryType::Solid => {
            let shells: Vec<Vec<Vec<Vec<usize>>>> = crate::wkb_write::boundaries(geom)?;
            let mut pos = 0;
            let faces: Vec<i32> = shells
                .iter()
                .map(|shell| {
                    face_count_i32(shell.len() - dropped_in_shell(dropped, &mut pos, shell.len()))
                })
                .collect::<Result<_>>()?;
            // One solid -> one inner list, even though a Solid's own
            // boundaries have no outer "per-solid" nesting of their own.
            Ok(Some(vec![faces]))
        }
        GeometryType::MultiSolid | GeometryType::CompositeSolid => {
            let solids: Vec<Vec<Vec<Vec<Vec<usize>>>>> = crate::wkb_write::boundaries(geom)?;
            let mut pos = 0;
            let faces: Vec<Vec<i32>> = solids
                .iter()
                .map(|solid| {
                    solid
                        .iter()
                        .map(|shell| {
                            face_count_i32(
                                shell.len() - dropped_in_shell(dropped, &mut pos, shell.len()),
                            )
                        })
                        .collect::<Result<_>>()
                })
                .collect::<Result<_>>()?;
            Ok(Some(faces))
        }
        _ => Ok(None),
    }
}

/// Number of leaf faces beneath a boundary subtree that sits `depth`
/// array-levels above the face list (a "face" is a list of rings). Used to
/// size CityJSON's null shorthand in `semantics.values` when flattening.
pub(crate) fn count_boundary_faces(boundaries: &Value, depth: usize) -> usize {
    match boundaries {
        Value::Array(arr) if depth == 0 => arr.len(),
        Value::Array(arr) => arr.iter().map(|b| count_boundary_faces(b, depth - 1)).sum(),
        _ => 0,
    }
}

/// `semantics.values` nesting above the flat per-face list: 0 for the
/// surface-list types (`MultiSurface`/`CompositeSurface`, already flat), 1 for
/// `Solid` (shells → faces), 2 for `MultiSolid`/`CompositeSolid`.
pub(crate) fn values_nesting_depth(thetype: &GeometryType) -> usize {
    solid_face_nesting_depth(thetype).unwrap_or(0)
}

/// Per face — in the same depth-first order [`count_boundary_faces`] counts
/// and [`flatten_values`] flattens, so entry `i` describes the face at flat
/// position `i` BEFORE the writer-dropped positions are filtered out — the
/// per-ring STORED vertex count: `Some(n)` for a ring the writer keeps,
/// `None` for one it drops (fewer than three source indices, the
/// [`crate::wkb_write::normalise_ring`] rule). A face's STORED ring count is
/// therefore the number of `Some` entries, and `n` is the number of `[u, v]`
/// pairs a textured ring carries — the WKB ring's closing repeat takes none.
pub(crate) fn face_ring_vertex_counts(boundaries: &Value, depth: usize) -> Vec<Vec<Option<usize>>> {
    fn walk(b: &Value, depth: usize, out: &mut Vec<Vec<Option<usize>>>) {
        match b {
            Value::Array(arr) if depth == 0 => out.extend(arr.iter().map(face_ring_lengths)),
            Value::Array(arr) => {
                for child in arr {
                    walk(child, depth - 1, out);
                }
            }
            // Matches `count_boundary_faces`, which counts no face here.
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(boundaries, depth, &mut out);
    out
}

/// One face's rings as stored vertex counts (see
/// [`face_ring_vertex_counts`]). A malformed ring — not an array, or holding
/// anything but non-negative integers — is `None`, the same answer a ring
/// the writer drops gets: neither reaches the WKB.
fn face_ring_lengths(face: &Value) -> Vec<Option<usize>> {
    face.as_array()
        .map(|rings| rings.iter().map(stored_ring_len).collect())
        .unwrap_or_default()
}

fn stored_ring_len(ring: &Value) -> Option<usize> {
    let indices: Vec<usize> = ring
        .as_array()?
        .iter()
        .map(|v| v.as_u64().map(|n| n as usize))
        .collect::<Option<_>>()?;
    crate::wkb_write::distinct_ring_len(&indices)
}

/// Flatten `semantics.values` into a per-face `face_semantics` list (§8) in
/// depth-first WKB face order, expanding CityJSON's null shorthand — a single
/// `null` standing for a whole shell/solid — into one `null` per face of that
/// part (using `boundaries` to size it). Entries are kept verbatim (a surface
/// index or `null`). The result covers the ORIGINAL faces, before the
/// degenerate-drop filter the caller then applies.
pub(crate) fn flatten_values(
    values: &Value,
    boundaries: &Value,
    depth: usize,
    out: &mut Vec<Value>,
) {
    match (values, depth) {
        // A whole subtree with no semantics: one null per face beneath it.
        (Value::Null, _) => {
            out.extend(std::iter::repeat_n(
                Value::Null,
                count_boundary_faces(boundaries, depth),
            ));
        }
        // Flat per-face entries. Size EXACTLY to this boundary's face count —
        // pad a short list with null, ignore a long one's overflow — so a
        // malformed shell/solid can never shift a later part's entries onto the
        // wrong faces (each part stays aligned to its own boundary).
        (Value::Array(varr), 0) => {
            for i in 0..count_boundary_faces(boundaries, 0) {
                out.push(varr.get(i).cloned().unwrap_or(Value::Null));
            }
        }
        // A nesting level: recurse per BOUNDARY child (not per values child),
        // so a missing values child expands to nulls and an extra one is
        // ignored — again keeping every part aligned to its boundary.
        (Value::Array(varr), _) => {
            let barr = boundaries.as_array();
            for i in 0..barr.map_or(0, Vec::len) {
                let v = varr.get(i).unwrap_or(&Value::Null);
                let b = barr.and_then(|b| b.get(i)).unwrap_or(&Value::Null);
                flatten_values(v, b, depth - 1, out);
            }
        }
        // Malformed non-array where a nested value was expected: fill the whole
        // subtree with null so the face alignment is preserved.
        _ => out.extend(std::iter::repeat_n(
            Value::Null,
            count_boundary_faces(boundaries, depth),
        )),
    }
}

/// Number of shell/solid nesting levels above the per-face entries in a
/// Solid-family `semantics`/`material`/`texture` values array: `Solid`'s
/// values nest one level (shells -> faces), `MultiSolid`/`CompositeSolid`'s
/// nest two (solids -> shells -> faces). `None` for the non-solid types,
/// whose per-surface arrays sit directly at the top level.
///
/// Derived from the geometry type rather than inferred from the values'
/// own shape: a shape-only heuristic cannot tell "a shells array holding a
/// single scalar-valued face list" apart from "a face holding a single
/// texture ring" — they collide byte-for-byte whenever there is exactly one
/// shell, which is the common case for a `Solid` (e.g. delft's Pand Solids
/// each have a single shell). Getting that case wrong means the walker
/// would stop one level too early and filter by SHELL position instead of
/// FACE position, silently corrupting the realignment on the most common
/// shape rather than merely skipping it.
fn solid_face_nesting_depth(thetype: &GeometryType) -> Option<usize> {
    match thetype {
        GeometryType::Solid => Some(1),
        GeometryType::MultiSolid | GeometryType::CompositeSolid => Some(2),
        _ => None,
    }
}

/// A `semantics.values` entry (a surface index or `null`) as an
/// `Option<i32>` — the typed `face_semantics` item shape (spec: `LIST<INT>`,
/// items nullable).
fn face_semantics_entry(v: &Value) -> Result<Option<i32>> {
    match v {
        Value::Null => Ok(None),
        Value::Number(n) => {
            let i = n.as_i64().ok_or_else(|| {
                CityParquetError::Schema(format!("semantics value {n} is not an integer"))
            })?;
            Ok(Some(face_count_i32(usize::try_from(i).map_err(|_| {
                CityParquetError::Schema(format!("semantics value {i} is negative"))
            })?)?))
        }
        other => Err(CityParquetError::Schema(format!(
            "semantics value must be an integer or null, got {other}"
        ))),
    }
}

/// The `geometry_properties_lod*` `STRUCT` value (spec "Geometry properties
/// and semantics"): `type` (always), `surfaces` + `face_semantics` (present
/// together, only when the source carries semantics — §8), `shells`
/// (solids only, [`solid_shells`]).
///
/// - `surfaces` is the CityJSON `surfaces` array **verbatim** (order and
///   content preserved — `parent`/`children` indices must stay valid).
/// - `face_semantics` has one entry per EMITTED WKB face, in WKB face order:
///   the face's surface index, or `null`. CityJSON's nested `values` are
///   flattened (null shorthand expanded, §8) and the writer-dropped face
///   positions removed, so its length equals the WKB face count. A
///   `surfaces` array with every `face_semantics` entry `null` (real
///   surfaces defined, nothing currently references them) is kept distinct
///   from "no semantics at all": `face_semantics` is still emitted, as a
///   same-length all-null list — never collapsed to a null cell.
/// - `shells` (solids only) is the per-shell stored face count (§8,
///   [`solid_shells`]), always nested one inner list per solid.
///
/// Non-normative write-time diagnostics (which rings/surfaces the writer
/// dropped as structurally degenerate) are counted in [`EncodeStats`], not
/// stored here — the struct's shape is exactly these four fields, with no
/// extra key for them (unlike the old JSON encoding's `dropped_degenerate`).
pub(crate) fn compute_geometry_properties(
    geom: &Geometry,
    dropped_surfaces: &[usize],
) -> Result<GeometryProperties> {
    let type_name = serde_json::to_value(&geom.thetype)?
        .as_str()
        .ok_or_else(|| CityParquetError::Schema("geometry type is not a string".to_string()))?
        .to_string();

    let mut surfaces = None;
    let mut face_semantics = None;
    if let Some(semantics) = &geom.semantics {
        if let Some(s) = semantics.get("surfaces") {
            surfaces = Some(s.clone());
        }
        let depth = values_nesting_depth(&geom.thetype);
        let mut flat = Vec::new();
        if let Some(values) = semantics.get("values") {
            flatten_values(values, &geom.boundaries, depth, &mut flat);
        }
        // Force exactly one entry per ORIGINAL face: pad a source `values`
        // shorter than the face count with `null` rather than erroring (§8,
        // "defensive on source"), and truncate a malformed longer one. After
        // the drop-filter below this yields exactly the WKB face count.
        let original_faces = count_boundary_faces(&geom.boundaries, depth);
        flat.resize(original_faces, Value::Null);
        // Align to the EMITTED faces: drop the writer-removed positions.
        let dropped: std::collections::HashSet<usize> = dropped_surfaces.iter().copied().collect();
        let entries: Vec<Option<i32>> = flat
            .into_iter()
            .enumerate()
            .filter(|(i, _)| !dropped.contains(i))
            .map(|(_, v)| face_semantics_entry(&v))
            .collect::<Result<_>>()?;
        face_semantics = Some(entries);
    }
    let shells = solid_shells(geom, dropped_surfaces)?;

    Ok(GeometryProperties {
        type_name,
        surfaces,
        face_semantics,
        shells,
    })
}

/// `(template id, WKB point, transformationMatrix)` — one resolved
/// `template` column's worth of data. The matrix is the flat, row-major
/// 16-value list the reserved `LIST<DOUBLE>` column stores (spec "Appearance
/// & templates").
type TemplateFields = (i64, Vec<u8>, Option<Vec<f64>>);

/// Parses a CityJSON `transformationMatrix` value into the flat 16-value
/// list the reserved column stores, erroring — not silently truncating or
/// padding — when it is not an array of exactly 16 numbers (spec "Appearance
/// & templates": "exactly 16 values when non-null").
fn parse_transformation_matrix(v: &Value) -> Result<Vec<f64>> {
    let values: Vec<f64> = serde_json::from_value(v.clone()).map_err(|e| {
        CityParquetError::Schema(format!(
            "template.transformationMatrix is not an array of numbers: {e}"
        ))
    })?;
    if values.len() != 16 {
        return Err(CityParquetError::Schema(format!(
            "template.transformationMatrix must have exactly 16 values (a flat row-major \
             4x4), got {}",
            values.len()
        )));
    }
    Ok(values)
}

/// `template` binding rule: built from the first `GeometryInstance`
/// geometry on the object; `None` when it can't be resolved (missing
/// template index, empty/malformed boundaries) so callers null the column
/// rather than panic. A present but malformed `transformationMatrix` (wrong
/// length or non-numeric) is a hard error, not a graceful drop — see
/// [`parse_transformation_matrix`].
fn build_template(geom: &Geometry, pool: &VertexPool) -> Result<Option<TemplateFields>> {
    let Some(template_id) = geom.template else {
        return Ok(None);
    };
    let Ok(idxs) = serde_json::from_value::<Vec<usize>>(geom.boundaries.clone()) else {
        return Ok(None);
    };
    let Some(&first_idx) = idxs.first() else {
        return Ok(None);
    };
    let point = point_to_wkb(pool.coord(first_idx)?);
    let matrix = geom
        .transformation_matrix
        .as_ref()
        .map(parse_transformation_matrix)
        .transpose()?;
    // CityJSON's `template` is the index into `geometry-templates.templates`,
    // which `build_template_rows` also uses as the sidecar row's `id` — so this
    // reference resolves by value against `geometry_templates.parquet`.
    Ok(Some((template_id as i64, point, matrix)))
}

/// One `address[]` entry's resolved fields, ready for the reserved `address`
/// struct column: the mapped postal strings plus, when the source carried a
/// resolvable `location`, its WKB `MultiPointZ` bytes (spec "Addresses").
struct AddressRow {
    postal: crate::address::AddressPostal,
    location: Option<Vec<u8>>,
}

/// Resolves one address entry's `location` member (a CityJSON `MultiPoint`
/// geometry whose `boundaries` index the feature's vertex pool) into WKB
/// `MultiPointZ` bytes, reusing [`geometry_to_wkb`] exactly as a regular
/// `MultiPoint` geometry would. `None` when `location` is absent or
/// malformed (wrong `type`, unparsable `boundaries`, empty, or an
/// out-of-range vertex index) — a best-effort address field, not load-bearing
/// geometry, so a malformed one is silently dropped rather than aborting the
/// whole conversion (mirrors [`build_template`]'s graceful-degradation
/// style).
fn build_location_wkb(location: &Value, pool: &VertexPool) -> Option<Vec<u8>> {
    let obj = location.as_object()?;
    if obj.get("type").and_then(Value::as_str) != Some("MultiPoint") {
        return None;
    }
    let idxs: Vec<usize> = serde_json::from_value(obj.get("boundaries")?.clone()).ok()?;
    if idxs.is_empty() {
        return None;
    }
    let geom = Geometry {
        thetype: GeometryType::MultiPoint,
        lod: None,
        boundaries: serde_json::to_value(&idxs).ok()?,
        semantics: None,
        material: None,
        texture: None,
        template: None,
        transformation_matrix: None,
    };
    geometry_to_wkb(&geom, pool).ok().flatten().map(|o| o.bytes)
}

/// The source object's raw `address` array (spec "Addresses"), if any — read
/// directly since cjseq has no typed field for it (CityJSON does not
/// prescribe address member names; it rides the struct's private
/// `#[serde(flatten)]` member, like `children_roles`). `None` when the
/// object carries no `address` member, or a malformed (non-array) one —
/// treated as absent rather than an error, since a corrupt `address` member
/// should not abort an otherwise-valid conversion. `Some(vec![])` is kept
/// distinct from `None`: an explicit empty array is a genuine (if unusual)
/// value, not "no address at all".
pub(crate) fn raw_address_members(co: &CityObject) -> Result<Option<Vec<Value>>> {
    Ok(address_members_from_json(&object_json(co)?))
}

/// [`raw_address_members`]'s core over an already-serialised member map — the
/// form [`RowWriter::push_object`] uses, so the row loop serialises the object
/// only once (review P5b).
pub(crate) fn address_members_from_json(
    co_json: &serde_json::Map<String, Value>,
) -> Option<Vec<Value>> {
    match co_json.get("address") {
        Some(Value::Array(arr)) => Some(arr.clone()),
        _ => None,
    }
}

/// Build the reserved `address` column's row value: one [`AddressRow`] per
/// source entry, in order — cardinality is preserved even when an entry maps
/// to nothing recognised (an all-`None` struct still occupies its list
/// position), since "how many addresses" is itself meaningful. `None` when
/// the object carries no `address` member at all (column cell null).
fn build_address_rows(
    co_json: &serde_json::Map<String, Value>,
    pool: &VertexPool,
) -> Result<Option<Vec<AddressRow>>> {
    let Some(entries) = address_members_from_json(co_json) else {
        return Ok(None);
    };
    let rows = entries
        .iter()
        .map(|entry| AddressRow {
            postal: crate::address::map_postal_fields(entry),
            location: entry
                .get("location")
                .and_then(|loc| build_location_wkb(loc, pool)),
        })
        .collect();
    Ok(Some(rows))
}

/// Per-object accumulator filled by [`accumulate_geometry`], consumed by
/// [`RowWriter::push_object`] to populate the row's columns.
#[derive(Default)]
struct GeometryAccumulator {
    /// Column slot key (LoD suffix, or `""` for the un-suffixed `geometry`
    /// column) -> ([`GeometryPayload`] (WKB bytes), bbox, the
    /// `geometry_properties` struct, the material cell, the texture cell).
    /// Appearance is keyed by the SAME canonical slot key as
    /// the geometry it decorates (§11.1), so the raw-vs-canonical LoD-key
    /// mismatch that the old single-column layout had to guard against
    /// cannot arise: a LoD's geometry, semantics and appearance share one key.
    slots: HashMap<String, GeometrySlotData>,
    template: Option<TemplateFields>,
    own_bbox: Option<[f64; 6]>,
}

/// One geometry's encoded payload: the WKB bytes.
enum GeometryPayload {
    Wkb(Vec<u8>),
}

/// One geometry slot's per-object payload: its encoded geometry, typed
/// `geometry_properties` struct value, and the typed `material`/`texture`
/// cells that decorate it.
struct GeometrySlotData {
    payload: GeometryPayload,
    properties: GeometryProperties,
    material: Option<MaterialCell>,
    texture: Option<TextureCell>,
}

/// This feature's local material definitions, from `feature.appearance`
/// (empty when the feature carries no appearance block at all — a geometry
/// that still has a non-empty `material` map in that case is a dangling
/// reference, caught by [`accumulate_geometry`]'s interner rewrite).
fn feature_local_materials(feature: &CityJSONFeature) -> &[Value] {
    feature
        .appearance
        .as_ref()
        .and_then(|a| a.materials.as_deref())
        .unwrap_or(&[])
}

/// This feature's local texture definitions (see [`feature_local_materials`]).
fn feature_local_textures(feature: &CityJSONFeature) -> &[Value] {
    feature
        .appearance
        .as_ref()
        .and_then(|a| a.textures.as_deref())
        .unwrap_or(&[])
}

/// This feature's local UV vertex pool (see [`feature_local_materials`]).
fn feature_local_uvs(feature: &CityJSONFeature) -> &[Vec<f64>] {
    feature
        .appearance
        .as_ref()
        .and_then(|a| a.vertices_texture.as_deref())
        .unwrap_or(&[])
}

/// One feature's local appearance definitions: the material, texture, and UV
/// vertices arrays that a geometry's `material`/`texture` indices resolve
/// against before being rewritten to dataset-global ids.
pub(crate) struct LocalDefs<'a> {
    pub materials: &'a [Value],
    pub textures: &'a [Value],
    pub uvs: &'a [Vec<f64>],
}

/// Flatten one geometry's `material`/`texture` maps into the typed cells the
/// `material_lod*`/`texture_lod*` columns store — flat per WKB face, with
/// dataset-global ids from `interner` and UV pairs inlined — and build its
/// `geometry_properties` struct value. This is the exact per-geometry
/// appearance pipeline [`accumulate_geometry`] runs for a feature's own
/// geometries, factored out so the geometry-templates sidecar
/// (`crate::package`) can run the identical rules over `Source::header`'s
/// `geometry_templates` after the main encode pass, through the SAME
/// interner — a template's `material`/`texture`/`semantics` follow the same
/// CityJSON shapes as a regular geometry's, so the same rules apply verbatim.
///
/// A cell is `None` when the geometry carries no map at all AND when the map
/// it carries has no themes: the columns are nullable and the spec reserves
/// the null cell for "no material (or texture) in any theme", so an empty
/// map is written as a null cell rather than as an empty MAP.
///
/// `context` names the geometry in any interner error surfaced (e.g.
/// `"object abc123"` or `"geometry template 0"`).
///
/// `dropped_surfaces` are the writer-dropped flat surface positions (see
/// [`crate::wkb_write::WkbOutcome::dropped_surfaces`]) — the caller passes
/// the real ones straight from its `WkbOutcome`.
pub(crate) fn rewrite_geometry_appearance(
    geom: &Geometry,
    dropped_surfaces: &[usize],
    interner: &mut AppearanceInterner,
    defs: &LocalDefs,
    context: &str,
) -> Result<(
    Option<MaterialCell>,
    Option<TextureCell>,
    GeometryProperties,
)> {
    let material = match &geom.material {
        Some(material) => {
            let material = serde_json::to_value(material)?;
            let cell = interner
                .flatten_material_map(
                    &material,
                    &geom.boundaries,
                    &geom.thetype,
                    dropped_surfaces,
                    defs.materials,
                )
                .map_err(|e| {
                    CityParquetError::Schema(format!(
                        "{context}: cannot resolve material map to global ids: {e}"
                    ))
                })?;
            (!cell.themes.is_empty()).then_some(cell)
        }
        None => None,
    };

    let texture = match &geom.texture {
        Some(texture) => {
            let texture = serde_json::to_value(texture)?;
            let cell = interner
                .flatten_texture_map(
                    &texture,
                    &geom.boundaries,
                    &geom.thetype,
                    dropped_surfaces,
                    defs.textures,
                    defs.uvs,
                )
                .map_err(|e| {
                    CityParquetError::Schema(format!(
                        "{context}: cannot resolve texture map to global ids: {e}"
                    ))
                })?;
            (!cell.themes.is_empty()).then_some(cell)
        }
        None => None,
    };

    let props = compute_geometry_properties(geom, dropped_surfaces)?;
    Ok((material, texture, props))
}

/// Walk one object's own geometries, bucketing them into `acc`. `per_lod`
/// mirrors the dataset-wide binding rule from [`crate::scan`]: `true` means
/// the dataset has LoD-bearing geometry, so each geometry is placed by its
/// (now mandatory — [`crate::scan`] rejects a lod-less non-instance geometry)
/// LoD; `false` means the dataset has no analysis geometry and the single
/// un-suffixed `geometry` column is used.
///
/// `id` is `co`'s own CityObject id, used only to name the object in the
/// error surfaced when a geometry's `material`/`texture` map has indices
/// that cannot be resolved against `defs`'s appearance arrays (most commonly:
/// the feature carries no matching appearance block at all, so the defs'
/// slices are empty) — dangling local indices must never silently survive
/// into the dataset-global rewrite.
#[allow(clippy::too_many_arguments)]
fn accumulate_geometry(
    acc: &mut GeometryAccumulator,
    co: &CityObject,
    pool: &VertexPool,
    per_lod: bool,
    stats: &mut EncodeStats,
    interner: &mut AppearanceInterner,
    defs: &LocalDefs,
    id: &str,
) -> Result<()> {
    let Some(geoms) = &co.geometry else {
        return Ok(());
    };
    for geom in geoms {
        if geom.thetype == GeometryType::GeometryInstance {
            if acc.template.is_none() {
                acc.template = build_template(geom, pool)?;
            }
            continue;
        }

        let Some(outcome) = geometry_to_wkb(geom, pool)? else {
            continue;
        };
        let (payload, bbox, dropped_rings, dropped_surfaces) = (
            GeometryPayload::Wkb(outcome.bytes),
            outcome.bbox,
            outcome.dropped_rings,
            outcome.dropped_surfaces,
        );
        // Row bbox deliberately covers ALL of the object's analysis geometry,
        // including a duplicate-(object, LoD) geometry that is later skipped
        // (§10, G10): the object occupies that extent, and a superset bbox can
        // only cause false-positive reads, never false-negative pruning.
        union_bbox(&mut acc.own_bbox, bbox);

        // Every geometry reaching here is a non-instance that produced a
        // payload (instances are routed to `template` above). For a
        // `ScanResult` that matches this `Source`, [`crate::scan`] guarantees
        // such a geometry carries a valid lod and that the dataset is
        // therefore per-LoD — so both a missing/unparseable lod and
        // `per_lod == false` mean the scan does not match the source
        // (`encode` is public and takes an independent scan; a Seq file is
        // also reopened between the scan and encode passes). Reject rather
        // than silently drop or misplace the geometry.
        let lod = match geom.lod.as_deref().and_then(|s| Lod::parse(s).ok()) {
            Some(lod) if per_lod => lod,
            _ => {
                return Err(CityParquetError::Lod(format!(
                    "object {id}: geometry has no valid lod for a per-LoD column \
                     (the ScanResult does not match this source)"
                )));
            }
        };
        let slot_key = lod.column_suffix();

        if acc.slots.contains_key(&slot_key) {
            stats.skipped_same_lod_geometries += 1;
            continue;
        }

        // Counted over STORED geometries only, so the totals describe the
        // data downstream actually sees.
        stats.degenerate_rings_dropped += dropped_rings;
        stats.degenerate_surfaces_dropped += dropped_surfaces.len();

        // Writer owns consistency: per-surface appearance arrays must index
        // the STORED surfaces, so the writer-dropped positions are removed
        // before anything is written, and feature-local material/texture
        // indices are rewritten to dataset-global ids — both handled by the
        // shared pipeline in `rewrite_geometry_appearance` (also used by the
        // geometry-templates sidecar, see its doc comment).
        let (material, texture, props) = rewrite_geometry_appearance(
            geom,
            &dropped_surfaces,
            interner,
            defs,
            &format!("object {id}"),
        )?;

        // The LoD lives only in the column name (spec "Levels of detail") —
        // every geometry column, including LoD0, is suffixed, so there is no
        // bare column needing its LoD injected into `geometry_properties`.
        acc.slots.insert(
            slot_key,
            GeometrySlotData {
                payload,
                properties: props,
                material,
                texture,
            },
        );
    }
    Ok(())
}

/// Synthesise an LoD0 footprint slot for `co` from its lowest higher-LoD
/// boundary geometry (§9 "LoD0 synthesis"). Returns the `geometry_lod0_0`
/// slot payload (a `MultiPolygonZ`-shaped [`GeometryPayload`]
/// plus `geometry_properties`, with no `"lod"` field — the struct carries no
/// such field, and the LoD lives only in the column name) and the footprint
/// bbox. `None` when the object has no footprint-able geometry or no
/// acceptable ground is found.
fn synthesize_footprint(
    co: &CityObject,
    pool: &VertexPool,
    opts: &crate::lod0::Lod0Options,
) -> Result<Option<(GeometrySlotData, [f64; 6])>> {
    use crate::lod0::{faces_from_geometry, footprint_to_geometry, synthesize_lod0};

    let Some(geoms) = &co.geometry else {
        return Ok(None);
    };
    // Lowest-LoD footprint-able source geometry (prefer LoD1's extrusion base).
    let mut best: Option<(Lod, &Geometry)> = None;
    for geom in geoms {
        if !matches!(
            geom.thetype,
            GeometryType::Solid
                | GeometryType::MultiSolid
                | GeometryType::CompositeSolid
                | GeometryType::MultiSurface
                | GeometryType::CompositeSurface
        ) {
            continue;
        }
        let Some(lod) = geom.lod.as_deref().and_then(|s| Lod::parse(s).ok()) else {
            continue;
        };
        if lod.major() == 0 {
            continue; // an existing 0.* footprint means we would not synthesise
        }
        if best.as_ref().is_none_or(|(bl, _)| lod < *bl) {
            best = Some((lod, geom));
        }
    }
    let Some((_source_lod, geom)) = best else {
        return Ok(None);
    };

    let (faces, mask) = faces_from_geometry(geom, pool)?;
    if faces.is_empty() {
        return Ok(None);
    }
    let Some(fp) = synthesize_lod0(&faces, mask.as_deref(), opts) else {
        return Ok(None);
    };
    let (verts, ms) = footprint_to_geometry(&fp);
    let raw = VertexPool::raw(&verts);

    let Some(outcome) = geometry_to_wkb(&ms, &raw)? else {
        return Ok(None);
    };
    let (payload, bbox, dropped_surfaces) = (
        GeometryPayload::Wkb(outcome.bytes),
        outcome.bbox,
        outcome.dropped_surfaces,
    );

    let props = compute_geometry_properties(&ms, &dropped_surfaces)?;
    let data = GeometrySlotData {
        payload,
        properties: props,
        material: None,
        texture: None,
    };
    Ok(Some((data, bbox)))
}

/// One typed builder per inferred attribute column.
enum AttrBuilder {
    Boolean(BooleanBuilder),
    Int64(Int64Builder),
    Float64(Float64Builder),
    Date(Date32Builder),
    Timestamp(TimestampMillisecondBuilder),
    String(StringBuilder),
    StringList(ListBuilder<StringBuilder>),
    Json(StringBuilder),
}

impl AttrBuilder {
    fn new(ty: AttributeType) -> Self {
        match ty {
            AttributeType::Boolean => Self::Boolean(BooleanBuilder::new()),
            AttributeType::Int64 => Self::Int64(Int64Builder::new()),
            AttributeType::Float64 => Self::Float64(Float64Builder::new()),
            AttributeType::Date => Self::Date(Date32Builder::new()),
            AttributeType::Timestamp => {
                Self::Timestamp(TimestampMillisecondBuilder::new().with_timezone("UTC"))
            }
            AttributeType::String => Self::String(StringBuilder::new()),
            AttributeType::StringList => Self::StringList(ListBuilder::new(StringBuilder::new())),
            AttributeType::Json => Self::Json(StringBuilder::new()),
        }
    }

    fn finish(&mut self) -> ArrayRef {
        match self {
            Self::Boolean(b) => Arc::new(b.finish()),
            Self::Int64(b) => Arc::new(b.finish()),
            Self::Float64(b) => Arc::new(b.finish()),
            Self::Date(b) => Arc::new(b.finish()),
            Self::Timestamp(b) => Arc::new(b.finish()),
            Self::String(b) => Arc::new(b.finish()),
            Self::StringList(b) => Arc::new(b.finish()),
            Self::Json(b) => Arc::new(b.finish()),
        }
    }
}

fn days_since_epoch(date: chrono::NaiveDate) -> i32 {
    let epoch = chrono::NaiveDate::from_ymd_opt(1970, 1, 1).expect("valid constant date");
    (date - epoch).num_days() as i32
}

/// Count a coercion failure only when there was an actual (non-null) value
/// to coerce — an absent/null attribute is not a coercion failure.
fn note_coercion_failure(value: Option<&Value>, stats: &mut EncodeStats) {
    if value.is_some() {
        stats.attribute_coercion_nulls += 1;
    }
}

/// Encode one attribute value per the type-specific rules in the brief:
/// `Date`/`Timestamp` parsed via `chrono`, `Json` serialised verbatim,
/// anything absent/null/unparseable becomes a null (counted iff it was a
/// real coercion failure, never a panic).
fn push_attribute_value(
    builder: &mut AttrBuilder,
    value: Option<&Value>,
    stats: &mut EncodeStats,
) -> Result<()> {
    let value = value.filter(|v| !v.is_null());
    match builder {
        AttrBuilder::Boolean(b) => match value.and_then(Value::as_bool) {
            Some(x) => b.append_value(x),
            None => {
                b.append_null();
                note_coercion_failure(value, stats);
            }
        },
        AttrBuilder::Int64(b) => match value.and_then(Value::as_i64) {
            Some(x) => b.append_value(x),
            None => {
                b.append_null();
                note_coercion_failure(value, stats);
            }
        },
        AttrBuilder::Float64(b) => match value.and_then(Value::as_f64) {
            Some(x) => b.append_value(x),
            None => {
                b.append_null();
                note_coercion_failure(value, stats);
            }
        },
        AttrBuilder::Date(b) => match value
            .and_then(Value::as_str)
            .and_then(|s| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
        {
            Some(d) => b.append_value(days_since_epoch(d)),
            None => {
                b.append_null();
                note_coercion_failure(value, stats);
            }
        },
        AttrBuilder::Timestamp(b) => match value
            .and_then(Value::as_str)
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        {
            Some(dt) => b.append_value(dt.with_timezone(&chrono::Utc).timestamp_millis()),
            None => {
                b.append_null();
                note_coercion_failure(value, stats);
            }
        },
        AttrBuilder::String(b) => match value.and_then(Value::as_str) {
            Some(s) => b.append_value(s),
            None => {
                b.append_null();
                note_coercion_failure(value, stats);
            }
        },
        AttrBuilder::StringList(b) => {
            let items = value
                .and_then(Value::as_array)
                .filter(|a| a.iter().all(Value::is_string));
            match items {
                Some(items) => b.append_value(items.iter().map(Value::as_str)),
                None => {
                    b.append_null();
                    note_coercion_failure(value, stats);
                }
            }
        }
        AttrBuilder::Json(b) => match value {
            Some(v) => b.append_value(serde_json::to_string(v)?),
            None => b.append_null(),
        },
    }
    Ok(())
}

/// The geometry-column builder for one [`GeometrySlot`]: a WKB
/// `BinaryBuilder`.
enum GeometryBuilder {
    Wkb(BinaryBuilder),
}

/// The columns of a single LoD (or the un-suffixed set when the dataset has
/// no LoDs): geometry (WKB), geometry_properties, material,
/// texture (§9, §11.1). Appearance builders live here so a LoD's appearance
/// is paired to its geometry by the shared column suffix, never by a JSON
/// key.
struct GeometrySlot {
    key: String,
    geometry: GeometryBuilder,
    properties: GeometryPropertiesBuilder,
    material: MaterialCellBuilder,
    texture: TextureCellBuilder,
}

/// Owns one builder per rendered Arrow schema column; [`Self::finish_arrays`]
/// drains them (in schema order) into a fresh set of Arrow arrays and resets
/// every builder so the same `RowWriter` can start accumulating the next
/// batch.
struct RowWriter {
    id: StringBuilder,
    feature_id: StringBuilder,
    object_type: StringDictionaryBuilder<Int32Type>,
    parents: ListBuilder<StringBuilder>,
    children: ListBuilder<StringBuilder>,
    children_roles: ListBuilder<StringBuilder>,
    /// The reserved `address` column: one [`StructBuilder`] (per
    /// [`cityparquet_schema::model::address_item_fields`]) per list item.
    address: ListBuilder<StructBuilder>,
    bbox_cols: [Vec<f64>; 6],
    bbox_nulls: NullBufferBuilder,
    per_lod: bool,
    geometry_slots: Vec<GeometrySlot>,
    template_id: Int64Builder,
    template_point: BinaryBuilder,
    template_matrix: ListBuilder<Float64Builder>,
    template_nulls: NullBufferBuilder,
    other: StringBuilder,
    attributes: Vec<(String, AttrBuilder)>,
    /// Attribute names diverted into `other` because they collide with a
    /// reserved/geometry column name (§5.2, G12). Sorted, so the diverted
    /// map each row emits is deterministic.
    diverted_attributes: Vec<String>,
    /// When `Some`, synthesise an LoD0 footprint into the `geometry_lod0_0`
    /// slot for any object lacking a source LoD0 (spec "LoD0 synthesis").
    /// Carries the thresholds.
    synthesize_lod0: Option<crate::lod0::Lod0Options>,
    len: usize,
}

impl RowWriter {
    fn new(scan: &ScanResult) -> Self {
        let per_lod = !scan.lods.is_empty();
        let new_slot = |key: String| GeometrySlot {
            key,
            geometry: GeometryBuilder::Wkb(BinaryBuilder::new()),
            properties: GeometryPropertiesBuilder::new(),
            material: MaterialCellBuilder::new(),
            texture: TextureCellBuilder::new(),
        };
        let geometry_slots = if per_lod {
            scan.lods
                .iter()
                .map(|lod| new_slot(lod.column_suffix()))
                .collect()
        } else {
            vec![new_slot(String::new())]
        };
        let attributes = scan
            .schema
            .attributes
            .iter()
            .map(|(name, ty)| (name.clone(), AttrBuilder::new(*ty)))
            .collect();
        Self {
            id: StringBuilder::new(),
            feature_id: StringBuilder::new(),
            object_type: StringDictionaryBuilder::new(),
            parents: ListBuilder::new(StringBuilder::new()),
            children: ListBuilder::new(StringBuilder::new()),
            children_roles: ListBuilder::new(StringBuilder::new()),
            address: ListBuilder::new(StructBuilder::from_fields(
                cityparquet_schema::model::address_item_fields(),
                0,
            )),
            bbox_cols: Default::default(),
            bbox_nulls: NullBufferBuilder::new(0),
            per_lod,
            geometry_slots,
            template_id: Int64Builder::new(),
            template_point: BinaryBuilder::new(),
            // Item field explicitly pinned to non-null Float64: a
            // `ListBuilder`'s default-derived item field is always nullable,
            // which would mismatch `template_data_type()`'s non-null matrix
            // entries at `StructArray::new` time.
            template_matrix: ListBuilder::new(Float64Builder::new())
                .with_field(Arc::new(Field::new("item", DataType::Float64, false))),
            template_nulls: NullBufferBuilder::new(0),
            other: StringBuilder::new(),
            attributes,
            diverted_attributes: scan.diverted_attribute_names.iter().cloned().collect(),
            synthesize_lod0: scan.synthesize_lod0,
            len: 0,
        }
    }

    fn push_bbox(&mut self, bbox: Option<[f64; 6]>) {
        match bbox {
            Some(b) => {
                for (col, v) in self.bbox_cols.iter_mut().zip(b) {
                    col.push(v);
                }
                self.bbox_nulls.append(true);
            }
            None => {
                for col in &mut self.bbox_cols {
                    col.push(0.0);
                }
                self.bbox_nulls.append(false);
            }
        }
    }

    fn push_template(&mut self, template: Option<TemplateFields>) {
        match template {
            Some((id, point, matrix)) => {
                self.template_id.append_value(id);
                self.template_point.append_value(point);
                match matrix {
                    Some(m) => self.template_matrix.append_value(m.into_iter().map(Some)),
                    None => self.template_matrix.append_null(),
                }
                self.template_nulls.append(true);
            }
            None => {
                self.template_id.append_null();
                self.template_point.append_null();
                self.template_matrix.append_null();
                self.template_nulls.append(false);
            }
        }
    }

    /// Address struct field indices, matching
    /// [`cityparquet_schema::model::address_item_fields`]'s order exactly.
    const ADDRESS_STREET: usize = 0;
    const ADDRESS_HOUSE_NUMBER: usize = 1;
    const ADDRESS_PO_BOX: usize = 2;
    const ADDRESS_ZIP_CODE: usize = 3;
    const ADDRESS_CITY: usize = 4;
    const ADDRESS_STATE: usize = 5;
    const ADDRESS_COUNTRY: usize = 6;
    const ADDRESS_FREE_TEXT: usize = 7;
    const ADDRESS_LOCATION: usize = 8;

    fn push_address(&mut self, rows: Option<Vec<AddressRow>>) {
        match rows {
            Some(rows) => {
                for row in rows {
                    let sb = self.address.values();
                    sb.field_builder::<StringBuilder>(Self::ADDRESS_STREET)
                        .expect("address.street is a StringBuilder")
                        .append_option(row.postal.street.as_deref());
                    sb.field_builder::<StringBuilder>(Self::ADDRESS_HOUSE_NUMBER)
                        .expect("address.house_number is a StringBuilder")
                        .append_option(row.postal.house_number.as_deref());
                    sb.field_builder::<StringBuilder>(Self::ADDRESS_PO_BOX)
                        .expect("address.po_box is a StringBuilder")
                        .append_option(row.postal.po_box.as_deref());
                    sb.field_builder::<StringBuilder>(Self::ADDRESS_ZIP_CODE)
                        .expect("address.zip_code is a StringBuilder")
                        .append_option(row.postal.zip_code.as_deref());
                    sb.field_builder::<StringBuilder>(Self::ADDRESS_CITY)
                        .expect("address.city is a StringBuilder")
                        .append_option(row.postal.city.as_deref());
                    sb.field_builder::<StringBuilder>(Self::ADDRESS_STATE)
                        .expect("address.state is a StringBuilder")
                        .append_option(row.postal.state.as_deref());
                    sb.field_builder::<StringBuilder>(Self::ADDRESS_COUNTRY)
                        .expect("address.country is a StringBuilder")
                        .append_option(row.postal.country.as_deref());
                    sb.field_builder::<StringBuilder>(Self::ADDRESS_FREE_TEXT)
                        .expect("address.free_text is a StringBuilder")
                        .append_option(row.postal.free_text.as_deref());
                    sb.field_builder::<BinaryBuilder>(Self::ADDRESS_LOCATION)
                        .expect("address.location is a BinaryBuilder")
                        .append_option(row.location.as_deref());
                    sb.append(true);
                }
                self.address.append(true);
            }
            None => self.address.append(false),
        }
    }

    fn push_string_list(builder: &mut ListBuilder<StringBuilder>, values: Option<&[String]>) {
        match values {
            Some(v) => builder.append_value(v.iter().map(|s| Some(s.as_str()))),
            None => builder.append_null(),
        }
    }

    /// CityJSON's `children_roles`: the role of each child in a
    /// `CityObjectGroup`, one per child. The CityJSON 2.0.1 schema permits it
    /// ONLY on `CityObjectGroup` (§2.5), so only those objects are inspected —
    /// spec-correct, and it keeps the lookup off the hot path for the common
    /// `Building`/`BuildingPart` hierarchy, whose parents carry `children` but
    /// never `children_roles`. cjseq 0.4.1 has no typed field for it (it is
    /// captured in `CityObject`'s private `#[serde(flatten)]` member), so it
    /// has to be read off the serialised member map — which arrives
    /// pre-serialised from [`RowWriter::push_object`] ([`object_json`]), once
    /// per row and shared with the `address` and `other` paths.
    ///
    /// A present `children_roles` MUST be an array of strings with exactly one
    /// entry per child (CityJSON 2.0.1 §2.5); anything else is rejected, not
    /// silently coerced, so a corrupt role list can never be stored.
    fn children_roles(
        co: &CityObject,
        co_json: &serde_json::Map<String, Value>,
        id: &str,
    ) -> Result<Option<Vec<String>>> {
        if co.thetype != "CityObjectGroup" {
            return Ok(None);
        }
        let Some(roles) = co_json.get("children_roles").cloned() else {
            return Ok(None);
        };
        let Value::Array(roles) = roles else {
            return Err(CityParquetError::Schema(format!(
                "object {id}: children_roles must be an array of strings"
            )));
        };
        let child_count = co.children.as_ref().map_or(0, Vec::len);
        if roles.len() != child_count {
            return Err(CityParquetError::Schema(format!(
                "object {id}: children_roles has {} entries but the object has {child_count} \
                 children (CityJSON 2.0.1 requires one role per child)",
                roles.len()
            )));
        }
        roles
            .iter()
            .map(|role| {
                role.as_str().map(str::to_string).ok_or_else(|| {
                    CityParquetError::Schema(format!(
                        "object {id}: every children_roles entry must be a string"
                    ))
                })
            })
            .collect::<Result<Vec<String>>>()
            .map(Some)
    }

    /// Encode one CityObject row. `id` must be a key of `feature.city_objects`.
    fn push_object(
        &mut self,
        feature: &CityJSONFeature,
        id: &str,
        transform: &Transform,
        stats: &mut EncodeStats,
        interner: &mut AppearanceInterner,
    ) -> Result<()> {
        let co = feature
            .city_objects
            .get(id)
            .expect("id came from this feature's own city_objects keys");
        let pool = VertexPool::new(&feature.vertices, transform);
        // The ONE whole-object serialisation this row performs (review P5b);
        // children_roles / address / other all read from this map.
        let co_json = object_json(co)?;

        self.id.append_value(id);
        self.feature_id.append_value(&feature.id);
        // `object_type` stores the CityGML 3.0 class name, not the CityJSON
        // spelling (spec "object_table-schema" — "object_type vocabulary").
        // For the 4 classes where they differ (`TransportSquare`->`Square`,
        // `GenericCityObject`->`GenericOccupiedSpace`,
        // `BuildingStorey`->`Storey`, `TunnelHollowSpace`->`HollowSpace`),
        // `ClassInfo::citygml_class` carries the CityGML spelling; every
        // other core class has the same spelling in both fields. An
        // extension (ADE / CityJSON Extension) class has no taxonomy entry,
        // so it keeps its own source spelling, but with CityJSON's leading
        // `+` marker stripped (spec: "An extension ... type keeps its own
        // class name, with the CityJSON `+` prefix stripped").
        let object_type = cityparquet_schema::class_info(&co.thetype)
            .map(|info| info.citygml_class)
            .unwrap_or_else(|| cityparquet_schema::strip_plus(&co.thetype));
        self.object_type.append(object_type)?;
        Self::push_string_list(&mut self.parents, co.parents.as_deref());
        Self::push_string_list(&mut self.children, co.children.as_deref());
        let children_roles = Self::children_roles(co, &co_json, id)?;
        Self::push_string_list(&mut self.children_roles, children_roles.as_deref());
        // Geometry accumulation (incl. optional LoD0 synthesis) populates
        // geometry slots and bboxes that are written to columns below; both
        // must be complete before row output begins.
        let mut acc = GeometryAccumulator::default();
        let defs = LocalDefs {
            materials: feature_local_materials(feature),
            textures: feature_local_textures(feature),
            uvs: feature_local_uvs(feature),
        };
        accumulate_geometry(
            &mut acc,
            co,
            &pool,
            self.per_lod,
            stats,
            interner,
            &defs,
            id,
        )?;

        // Synthesise an LoD0 footprint into the `geometry_lod0_0` slot when
        // enabled and the object has no source LoD0 (spec "LoD0 synthesis").
        // The footprint slot key is LoD0's column suffix (`lod0_0`), the same
        // key `accumulate_geometry` would use for a real LoD0.
        if let Some(opts) = &self.synthesize_lod0 {
            let key = Lod::parse("0")
                .expect("literal 0 is a valid LoD")
                .column_suffix();
            if !acc.slots.contains_key(&key)
                && let Some((data, bbox)) = synthesize_footprint(co, &pool, opts)?
            {
                union_bbox(&mut acc.own_bbox, bbox);
                acc.slots.insert(key, data);
                stats.synthesized_lod0_footprints += 1;
            }
        }

        // Address rows must be derived BEFORE `co_json` is consumed for
        // `other` below (they are pushed further down, with the column).
        let address_rows = build_address_rows(&co_json, &pool)?;

        // `other`: the single escape hatch (§5.1). Two kinds of entry share
        // it — a source member with no dedicated column, and an attribute
        // whose name collides with a reserved column — and they are
        // deliberately not distinguished: the column's whole contract is
        // that a reader restores every entry into `attributes`. Null when
        // the object has neither.
        //
        // Where an unmapped top-level member and an attribute (diverted or
        // column-backed) share a key, the attribute wins ("warn and prefer
        // attribute"): keeping the member would either silently lose the
        // attribute's value (diverted case) or make this writer emit an
        // `other` entry duplicating the attribute's own column, which a
        // reader MUST-errors on (`merge_other_members` in `src/decode.rs`).
        // The member is dropped instead, counted and diagnosed rather than
        // failing the conversion.
        let unmapped = unmapped_from_json(co_json);
        let diverted = collect_diverted_attributes(co, &self.diverted_attributes);
        stats.diverted_attribute_values += diverted.as_ref().map_or(0, serde_json::Map::len);

        let attrs = co.attributes.as_ref().and_then(Value::as_object);
        let mut merged = serde_json::Map::new();
        for (key, value) in unmapped {
            let collides = attrs.is_some_and(|attrs| attrs.get(&key).is_some_and(|v| !v.is_null()));
            if collides {
                stats.dropped_colliding_members += 1;
                stats.dropped_colliding_member_diagnostics.push(format!(
                    "object '{id}': unmapped member '{key}' dropped from 'other' — an \
                     attribute of the same name takes precedence"
                ));
                continue;
            }
            merged.insert(key, value);
        }
        // Every key `diverted` carries came from `co.attributes` itself, so
        // the loop above has already dropped any unmapped member that would
        // collide with it — `extend` can never overwrite an entry here.
        if let Some(diverted) = diverted {
            merged.extend(diverted);
        }
        if merged.is_empty() {
            self.other.append_null();
        } else {
            self.other
                .append_value(serde_json::to_string(&Value::Object(merged))?);
        }

        // `address`: the reserved struct column (spec "Addresses", gap 10).
        self.push_address(address_rows);

        for slot in &mut self.geometry_slots {
            match acc.slots.get(&slot.key) {
                Some(data) => {
                    match (&mut slot.geometry, &data.payload) {
                        (GeometryBuilder::Wkb(b), GeometryPayload::Wkb(bytes)) => {
                            b.append_value(bytes)
                        }
                    }
                    slot.properties.append_value(&data.properties)?;
                    match &data.material {
                        Some(m) => slot.material.append_value(m)?,
                        None => slot.material.append_null(),
                    }
                    match &data.texture {
                        Some(t) => slot.texture.append_value(t)?,
                        None => slot.texture.append_null(),
                    }
                }
                None => {
                    match &mut slot.geometry {
                        GeometryBuilder::Wkb(b) => b.append_null(),
                    }
                    slot.properties.append_null();
                    slot.material.append_null();
                    slot.texture.append_null();
                }
            }
        }

        // `bbox` is the union of the object's stored geometry (whole subtree,
        // all LoDs) and the source's declared extent. A declared extent may
        // only widen the box: sources routinely declare one that does not
        // contain their own geometry, and a box narrower than the geometry
        // silently prunes the row out of spatial queries.
        let mut bbox = resolve_bbox(acc.own_bbox, id, co, feature, &pool)?;
        if let Some(extent) = source_extent(co) {
            union_bbox(&mut bbox, extent);
        }
        self.push_bbox(bbox);
        self.push_template(acc.template);

        let normalised: HashMap<String, &Value> = co
            .attributes
            .as_ref()
            .and_then(|v| v.as_object())
            .map(|obj| {
                obj.iter()
                    .map(|(k, v)| (normalise_attribute_name(k), v))
                    .collect()
            })
            .unwrap_or_default();
        for (name, builder) in &mut self.attributes {
            push_attribute_value(builder, normalised.get(name).copied(), stats)?;
        }

        self.len += 1;
        Ok(())
    }

    fn finish_bbox(&mut self) -> ArrayRef {
        let DataType::Struct(fields) = cityparquet_schema::model::bbox_data_type() else {
            unreachable!("bbox_data_type always returns Struct")
        };
        let arrays: Vec<ArrayRef> = self
            .bbox_cols
            .iter_mut()
            .map(|col| Arc::new(Float64Array::from(std::mem::take(col))) as ArrayRef)
            .collect();
        let nulls = self.bbox_nulls.finish();
        Arc::new(StructArray::new(fields, arrays, nulls))
    }

    fn finish_template(&mut self) -> ArrayRef {
        let DataType::Struct(fields) = cityparquet_schema::model::template_data_type() else {
            unreachable!("template_data_type always returns Struct")
        };
        let arrays: Vec<ArrayRef> = vec![
            Arc::new(self.template_id.finish()),
            Arc::new(self.template_point.finish()),
            Arc::new(self.template_matrix.finish()),
        ];
        let nulls = self.template_nulls.finish();
        Arc::new(StructArray::new(fields, arrays, nulls))
    }

    /// Drain every builder (in the exact column order `to_arrow_schema`
    /// renders) into arrays, resetting `len` back to 0.
    fn finish_arrays(&mut self) -> Vec<ArrayRef> {
        let mut arrays: Vec<ArrayRef> = vec![
            Arc::new(self.id.finish()),
            Arc::new(self.feature_id.finish()),
            Arc::new(self.object_type.finish()),
            Arc::new(self.parents.finish()),
            Arc::new(self.children.finish()),
            Arc::new(self.children_roles.finish()),
            Arc::new(self.address.finish()),
            self.finish_bbox(),
        ];
        for slot in &mut self.geometry_slots {
            match &mut slot.geometry {
                GeometryBuilder::Wkb(b) => arrays.push(Arc::new(b.finish())),
            }
            arrays.push(slot.properties.finish());
            arrays.push(slot.material.finish());
            arrays.push(slot.texture.finish());
        }
        arrays.push(self.finish_template());
        arrays.push(Arc::new(self.other.finish()));
        for (_, builder) in &mut self.attributes {
            arrays.push(builder.finish());
        }
        self.len = 0;
        arrays
    }
}

/// Lazily encodes `source` into `RecordBatch`es matching `scan.schema`'s
/// rendered Arrow schema exactly, `batch_size` rows at a time (the last
/// batch may be shorter). See [`EncodeStats`] for the row-population edge
/// cases tracked rather than surfaced as errors; call [`Self::stats`] to
/// read the running totals (e.g. via `iter.by_ref().collect(); iter.stats()`
/// so the iterator isn't consumed before you can read them).
///
/// # Error contract
///
/// The first `Err` is terminal: once `next()` has yielded an error, every
/// subsequent call returns `None` (the iterator fuses). Any partially
/// encoded row state is discarded with it — a mid-row failure leaves the
/// internal builders desynced, so no partially-desynced batch is ever
/// emitted, even to error-tolerant callers (e.g. `filter_map(Result::ok)`)
/// that keep pulling past the error.
/// The one-time source of `CityJSONFeature`s [`BatchIter`] pulls from: EITHER
/// a plain re-iteration of the [`Source`] the ordinary streaming path
/// (`encode`) opens, OR a `Vec` a caller has already collected and reordered
/// (M5 task 4, `crate::package::convert`'s Hilbert row ordering — buffering
/// every feature in memory, sorting by bbox-centroid Hilbert index, then
/// handing the sorted `Vec` in here).
///
/// Introduced so [`encode_buffered`] can share [`BatchIter::advance`]
/// (the whole batching/writer-push loop) with [`encode`] instead of
/// duplicating it: `advance` only ever calls `self.features.next()`, so
/// swapping WHICH stream backs `BatchIter` is the entire diff between the
/// two entry points.
enum FeatureStream<'a> {
    Source(FeatureIter<'a>),
    Buffered(std::vec::IntoIter<CityJSONFeature>),
}

impl Iterator for FeatureStream<'_> {
    type Item = Result<CityJSONFeature>;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            FeatureStream::Source(it) => it.next(),
            // A buffered `Vec` was already parsed successfully (it came
            // from a prior `Source::features()` pass whose `Result`s were
            // all unwrapped before buffering — see `crate::package`), so
            // yielding it back out wrapped in `Ok` never manufactures an
            // error `advance` didn't already have a chance to see.
            FeatureStream::Buffered(it) => it.next().map(Ok),
        }
    }
}

pub struct BatchIter<'a> {
    features: FeatureStream<'a>,
    transform: Transform,
    schema: Arc<Schema>,
    batch_size: usize,
    writer: RowWriter,
    current_feature: Option<CityJSONFeature>,
    current_object_ids: Vec<String>,
    current_idx: usize,
    exhausted_input: bool,
    /// Set when `next()` yields an `Err`; fuses the iterator (see the
    /// error contract above).
    errored: bool,
    stats: EncodeStats,
    /// Dataset-global material/texture interner, fed one feature's worth of
    /// local definitions at a time as rows are encoded; see [`Self::appearance`].
    appearance: AppearanceInterner,
}

impl BatchIter<'_> {
    /// Running totals of the row-population edge cases counted so far
    /// (final once the iterator is exhausted).
    pub fn stats(&self) -> EncodeStats {
        self.stats.clone()
    }

    /// The dataset-global material/texture interner accumulated so far
    /// (final once the iterator is exhausted) — read after the batch loop,
    /// before dropping the iterator (mirrors [`Self::stats`]).
    pub fn appearance(&self) -> &AppearanceInterner {
        &self.appearance
    }

    /// Mutable access to the same interner, so a post-encode pass (the
    /// geometry-templates sidecar in `crate::package`) can fold MORE
    /// definitions into it — e.g. entries reachable only from
    /// `Source::header`'s `geometry_templates`, which this encode loop never
    /// visits — before [`Self::appearance`] is read to write the
    /// materials/textures sidecars. Doing so keeps every appearance
    /// definition in ONE dataset-global id space, however it was
    /// discovered.
    pub fn appearance_mut(&mut self) -> &mut AppearanceInterner {
        &mut self.appearance
    }

    fn finish_batch(&mut self) -> Result<RecordBatch> {
        let arrays = self.writer.finish_arrays();
        Ok(RecordBatch::try_new(self.schema.clone(), arrays)?)
    }

    /// One `next()` step, without the fuse bookkeeping (see [`Self::next`]).
    fn advance(&mut self) -> Option<Result<RecordBatch>> {
        loop {
            if self.current_idx >= self.current_object_ids.len() {
                if self.exhausted_input {
                    return if self.writer.len > 0 {
                        Some(self.finish_batch())
                    } else {
                        None
                    };
                }
                match self.features.next() {
                    None => {
                        self.exhausted_input = true;
                        return if self.writer.len > 0 {
                            Some(self.finish_batch())
                        } else {
                            None
                        };
                    }
                    Some(Err(e)) => return Some(Err(e)),
                    Some(Ok(feature)) => {
                        let mut ids: Vec<String> = feature.city_objects.keys().cloned().collect();
                        ids.sort();
                        self.current_object_ids = ids;
                        self.current_idx = 0;
                        self.current_feature = Some(feature);
                        continue;
                    }
                }
            }

            let idx = self.current_idx;
            self.current_idx += 1;
            let id = self.current_object_ids[idx].clone();
            // Set alongside current_object_ids above; always Some here.
            let feature = self
                .current_feature
                .as_ref()
                .expect("feature set with its object ids");
            if let Err(e) = self.writer.push_object(
                feature,
                &id,
                &self.transform,
                &mut self.stats,
                &mut self.appearance,
            ) {
                return Some(Err(e));
            }
            if self.writer.len == self.batch_size {
                return Some(self.finish_batch());
            }
        }
    }
}

impl Iterator for BatchIter<'_> {
    type Item = Result<RecordBatch>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.errored {
            return None;
        }
        let item = self.advance();
        if matches!(item, Some(Err(_))) {
            // Fuse: a mid-row failure leaves the builders desynced, so the
            // partial state must never surface as a later (corrupt) batch.
            self.errored = true;
        }
        item
    }
}

/// Encode `source` into `RecordBatch`es matching
/// `scan.schema.to_arrow_schema()` exactly,
/// `batch_size` rows per batch (the schema was already computed by `scan`;
/// this pass never re-infers it).
pub fn encode<'a>(
    source: &'a Source,
    scan: &ScanResult,
    batch_size: usize,
) -> Result<BatchIter<'a>> {
    let schema = Arc::new(scan.schema.to_arrow_schema()?);
    let features = source.features()?;
    let transform = source.header().transform.clone();
    let writer = RowWriter::new(scan);
    Ok(BatchIter {
        features: FeatureStream::Source(features),
        transform,
        schema,
        batch_size: batch_size.max(1),
        writer,
        current_feature: None,
        current_object_ids: Vec::new(),
        current_idx: 0,
        exhausted_input: false,
        errored: false,
        stats: EncodeStats::default(),
        appearance: AppearanceInterner::new(),
    })
}

/// Sibling entry point to [`encode`] for a caller that has already decided
/// the exact feature order — M5 task 4's Hilbert row ordering
/// (`crate::package::convert`): `features` replaces the [`Source`]-driven
/// stream [`encode`] would otherwise open, entirely. Everything else
/// (schema, transform, batch size, appearance interning, stats) is
/// identical, because both entry points build the SAME [`BatchIter`] and
/// share its [`BatchIter::advance`] loop — see [`FeatureStream`]'s doc
/// comment for why that never duplicates the encode logic.
///
/// `header` supplies the `transform` a plain `encode` call would otherwise
/// read from `source.header()` — the caller already has `header` in hand
/// (it read it to compute each feature's Hilbert key in the first place),
/// so this never needs `Source` itself, and the returned `BatchIter` never
/// borrows from it: unlike `encode`'s stream, [`FeatureStream::Buffered`]
/// owns its `Vec` outright, which is why this can hand back a `BatchIter`
/// good for any lifetime the caller needs.
pub fn encode_buffered<'a>(
    features: Vec<CityJSONFeature>,
    header: &CityJSON,
    scan: &ScanResult,
    batch_size: usize,
) -> Result<BatchIter<'a>> {
    let schema = Arc::new(scan.schema.to_arrow_schema()?);
    let transform = header.transform.clone();
    let writer = RowWriter::new(scan);
    Ok(BatchIter {
        features: FeatureStream::Buffered(features.into_iter()),
        transform,
        schema,
        batch_size: batch_size.max(1),
        writer,
        current_feature: None,
        current_object_ids: Vec::new(),
        current_idx: 0,
        exhausted_input: false,
        errored: false,
        stats: EncodeStats::default(),
        appearance: AppearanceInterner::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::appearance_columns::TextureRing;

    // G12/gap 14: a colliding attribute is collected for merging into
    // `other`, skipping nulls.
    #[test]
    fn collect_diverted_attributes_diverts_present_non_null_values() {
        let co: CityObject = serde_json::from_value(serde_json::json!({
            "type": "Building",
            "attributes": {"bbox": "sentinel", "id": null, "keep": 1}
        }))
        .unwrap();
        let map = collect_diverted_attributes(&co, &["bbox".to_string(), "id".to_string()])
            .expect("bbox is diverted");
        assert_eq!(
            map,
            serde_json::json!({"bbox": "sentinel"})
                .as_object()
                .unwrap()
                .clone(),
            "only the non-null `bbox` is diverted; null `id` skipped"
        );
    }

    #[test]
    fn collect_diverted_attributes_is_none_when_nothing_diverts() {
        let co: CityObject = serde_json::from_value(serde_json::json!({
            "type": "Building",
            "attributes": {"keep": 1}
        }))
        .unwrap();
        assert!(collect_diverted_attributes(&co, &["bbox".to_string()]).is_none());
        assert!(collect_diverted_attributes(&co, &[]).is_none());
    }

    // spec "Addresses" (gap 10): only the recognised postal member names map
    // onto the reserved struct; anything else is dropped without disturbing
    // the fields that DID map.
    #[test]
    fn build_address_rows_maps_recognised_members_and_drops_the_rest() {
        let co: CityObject = serde_json::from_value(serde_json::json!({
            "type": "Building",
            "address": [
                {"locality": "Helsinki", "Locality": "should-not-map", "id": "dropped-id"}
            ]
        }))
        .unwrap();
        let pool = VertexPool::raw(&[]);
        let rows = build_address_rows(&object_json(&co).unwrap(), &pool)
            .unwrap()
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].postal.city.as_deref(), Some("Helsinki"));
        assert_eq!(
            rows[0].postal.country, None,
            "no recognised country member present"
        );
        assert_eq!(rows[0].location, None, "no location member present");
    }

    #[test]
    fn build_address_rows_is_none_without_an_address_member() {
        let co: CityObject =
            serde_json::from_value(serde_json::json!({"type": "Building"})).unwrap();
        let pool = VertexPool::raw(&[]);
        assert!(
            build_address_rows(&object_json(&co).unwrap(), &pool)
                .unwrap()
                .is_none()
        );
    }

    // spec "Appearance & templates": transformationMatrix MUST have exactly
    // 16 values when non-null.
    #[test]
    fn parse_transformation_matrix_rejects_a_non_16_length_matrix() {
        assert!(parse_transformation_matrix(&serde_json::json!([1.0, 2.0, 3.0])).is_err());
        assert!(parse_transformation_matrix(&serde_json::json!([])).is_err());
    }

    #[test]
    fn parse_transformation_matrix_accepts_exactly_16_values() {
        let values: Vec<f64> = (0..16).map(f64::from).collect();
        let parsed = parse_transformation_matrix(&serde_json::to_value(&values).unwrap()).unwrap();
        assert_eq!(parsed, values);
    }

    /// No fixture carries MultiSolid/CompositeSolid, so the nested `shells`
    /// branch gets direct coverage here: 2 solids — first with shells of
    /// (1, 2) faces, second with one 1-face shell. Nesting is the real
    /// G7 sol-review Finding 1: a malformed `semantics.values` with a shell
    /// SHORTER than its boundary must not shift the next shell's entries onto
    /// the wrong faces. A Solid with two 2-face shells and values
    /// `[[0], [1, 1]]` must flatten to `[0, null, 1, 1]` (shell 0 padded), not
    /// `[0, 1, 1, null]` (which a global pad would produce).
    #[test]
    fn flatten_sizes_each_shell_to_its_boundary() {
        // Two shells, each two faces (each face a single triangular ring).
        let boundaries =
            serde_json::json!([[[[0, 1, 2]], [[0, 1, 3]]], [[[0, 2, 3]], [[1, 2, 3]]]]);
        let mut out = Vec::new();
        flatten_values(&serde_json::json!([[0], [1, 1]]), &boundaries, 1, &mut out);
        assert_eq!(
            Value::Array(out),
            serde_json::json!([0, null, 1, 1]),
            "the short first shell must be padded to its two faces before the second shell"
        );
    }

    /// 5-level MultiSolid shape: solids → shells → surfaces → rings → indices.
    #[test]
    fn multisolid_shells_nest_per_solid() {
        let geom = Geometry {
            thetype: GeometryType::MultiSolid,
            lod: Some("2".into()),
            boundaries: serde_json::json!([
                [[[[0, 1, 2]]], [[[0, 1, 2]], [[0, 1, 3]]]],
                [[[[0, 2, 3]]]]
            ]),
            semantics: None,
            material: None,
            texture: None,
            template: None,
            transformation_matrix: None,
        };
        // `shells` nests one list per solid (§8), no separate counts key.
        assert_eq!(
            solid_shells(&geom, &[]).unwrap(),
            Some(vec![vec![1, 2], vec![1]])
        );

        let props = compute_geometry_properties(&geom, &[]).unwrap();
        assert_eq!(props.shells, Some(vec![vec![1, 2], vec![1]]));

        // With writer-dropped flat face positions, each shell's count must
        // describe the STORED geometry: positions 1 and 2 are the two faces of
        // the first solid's second shell; position 3 is the second solid's
        // only face.
        assert_eq!(
            solid_shells(&geom, &[1, 3]).unwrap(),
            Some(vec![vec![1, 1], vec![0]])
        );
    }

    /// When the writer drops a degenerate surface, the encoder must remove
    /// the SAME index from the semantics values and every material/texture
    /// theme's per-surface array, record the drop in geometry_properties,
    /// and count it in EncodeStats — downstream sees aligned data only.
    #[test]
    fn dropped_surface_flattens_semantics_material_and_texture() {
        let co: CityObject = serde_json::from_value(serde_json::json!({
            "type": "Building",
            "geometry": [{
                "type": "MultiSurface",
                "lod": "2",
                // surface 0's exterior ring has only 2 vertices — too short
                // to form a ring at all
                "boundaries": [[[0, 1]], [[0, 1, 2, 3]]],
                "semantics": {
                    "surfaces": [{"type": "WallSurface"}, {"type": "RoofSurface"}],
                    "values": [0, 1]
                },
                "material": {"visual": {"values": [5, 7]}},
                "texture": {"visual": {"values": [[[0, 0, 1, 2]], [[1, 0, 1, 2, 3]]]}}
            }]
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

        // Local defs sized to cover every raw index the fixture geometry
        // above references (material indices up to 7, texture index up to
        // 1, UV indices up to 3): the interner rewrite now resolves every
        // index against a real local def, so these must exist.
        let local_materials: Vec<Value> = (0..8)
            .map(|i| serde_json::json!({"name": format!("m{i}")}))
            .collect();
        let local_textures: Vec<Value> = (0..2)
            .map(|i| serde_json::json!({"type": "PNG", "image": format!("t{i}.png")}))
            .collect();
        let local_uvs: Vec<Vec<f64>> = vec![
            vec![0.0, 0.0],
            vec![1.0, 0.0],
            vec![1.0, 1.0],
            vec![0.0, 1.0],
        ];

        let mut acc = GeometryAccumulator::default();
        let mut stats = EncodeStats::default();
        let mut interner = AppearanceInterner::new();
        let defs = LocalDefs {
            materials: &local_materials,
            textures: &local_textures,
            uvs: &local_uvs,
        };
        accumulate_geometry(
            &mut acc,
            &co,
            &pool,
            true,
            &mut stats,
            &mut interner,
            &defs,
            "obj1",
        )
        .unwrap();

        assert_eq!(stats.degenerate_rings_dropped, 1);
        assert_eq!(stats.degenerate_surfaces_dropped, 1);

        let slot = acc.slots.get("lod2_0").expect("lod2_0 slot populated");
        let props = &slot.properties;
        assert_eq!(
            props.face_semantics,
            Some(vec![Some(1)]),
            "face_semantics must lose the dropped face's entry"
        );
        assert_eq!(
            props.surfaces,
            Some(serde_json::json!([{"type": "WallSurface"}, {"type": "RoofSurface"}])),
            "the surfaces lookup table is stored verbatim (face_semantics indexes into it)"
        );

        // Surface 0 (material index 5, texture index 0) was dropped; only
        // surface 1's material index 7 / texture index 1 survive, now
        // rewritten to dataset-global ids with inlined UVs.
        let gid_material_7 = interner.intern_material(&local_materials[7]) as i64;
        let gid_texture_1 = interner.intern_texture(&local_textures[1]) as i64;
        assert_eq!(
            *slot.material.as_ref().unwrap(),
            MaterialCell {
                themes: vec![("visual".to_string(), vec![Some(gid_material_7)])],
            },
            "the material cell must lose the dropped face and carry global ids"
        );
        assert_eq!(
            *slot.texture.as_ref().unwrap(),
            TextureCell {
                themes: vec![(
                    "visual".to_string(),
                    vec![vec![TextureRing {
                        id: Some(gid_texture_1),
                        uv: Some(vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]),
                    }]],
                )],
            },
            "the texture cell must lose the dropped face, carry global ids, and inline UVs"
        );
    }

    /// The planner's sketch heuristic classified a values array as "the face
    /// list to filter" purely from its own shape (any scalar/null element,
    /// or every element being an array of non-arrays). That collides
    /// exactly here: a Solid's single shell holding 3 scalar semantics
    /// values (`[[0, 1, 2]]`) has the SAME shape as a single face holding a
    /// texture ring of 3 indices — both are "one array whose one element is
    /// an array of non-arrays". The shape-only heuristic would stop
    /// recursing at the shells level (mistaking the shell for the face
    /// list) and filter by SHELL position instead of FACE position, which
    /// is a silent no-op here (only one shell, `dropped.contains(&0)` is
    /// false) rather than an error — exactly delft's own Solids, which each
    /// have a single shell. The depth-explicit fix sidesteps the ambiguity
    /// entirely by walking exactly `solid_face_nesting_depth(Solid) == 1`
    /// level before filtering, regardless of what the face entries look
    /// like.
    #[test]
    fn solid_single_shell_flattens_semantics_material_and_texture_when_face_dropped() {
        let co: CityObject = serde_json::from_value(serde_json::json!({
            "type": "Building",
            "geometry": [{
                "type": "Solid",
                "lod": "2",
                // one shell, 3 faces; face 1's ring has only 2 vertices —
                // too short to form a ring at all
                "boundaries": [[[[0, 1, 2]], [[0, 1]], [[1, 2, 3]]]],
                "semantics": {
                    "surfaces": [{"type": "A"}, {"type": "B"}, {"type": "C"}],
                    "values": [[0, 1, 2]]
                },
                "material": {"visual": {"values": [[1, 2, 3]]}},
                // one UV index per distinct ring vertex, as the column
                // requires: three for each of the three-vertex rings
                "texture": {"visual": {"values": [[
                    [[0, 0, 1, 2]], [[1, 0, 1, 2]], [[2, 0, 1, 2]]
                ]]}}
            }]
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

        // Local defs sized to cover the raw indices above (material 1..3,
        // texture 0..2, UV 0..2).
        let local_materials: Vec<Value> = (0..4)
            .map(|i| serde_json::json!({"name": format!("m{i}")}))
            .collect();
        let local_textures: Vec<Value> = (0..3)
            .map(|i| serde_json::json!({"type": "PNG", "image": format!("t{i}.png")}))
            .collect();
        let local_uvs: Vec<Vec<f64>> = vec![vec![0.0, 0.0], vec![1.0, 0.0], vec![1.0, 1.0]];

        let mut acc = GeometryAccumulator::default();
        let mut stats = EncodeStats::default();
        let mut interner = AppearanceInterner::new();
        let defs = LocalDefs {
            materials: &local_materials,
            textures: &local_textures,
            uvs: &local_uvs,
        };
        accumulate_geometry(
            &mut acc,
            &co,
            &pool,
            true,
            &mut stats,
            &mut interner,
            &defs,
            "obj1",
        )
        .unwrap();

        assert_eq!(stats.degenerate_rings_dropped, 1);
        assert_eq!(stats.degenerate_surfaces_dropped, 1);

        let slot = acc.slots.get("lod2_0").expect("lod2_0 slot populated");
        let props = &slot.properties;
        assert_eq!(
            props.shells,
            Some(vec![vec![2]]),
            "the single shell drops from 3 to 2 faces, nested one list per solid"
        );
        assert_eq!(
            props.face_semantics,
            Some(vec![Some(0), Some(2)]),
            "face_semantics is flat (one entry per emitted face) and loses face 1"
        );
        assert_eq!(
            props.surfaces,
            Some(serde_json::json!([{"type": "A"}, {"type": "B"}, {"type": "C"}])),
            "the surfaces lookup table is stored verbatim (face_semantics indexes into it)"
        );

        // Face 1 (material index 2, texture index 1) was dropped; faces 0
        // and 2 survive, rewritten to dataset-global ids and flattened out
        // of the shell nesting.
        let gid_material_1 = interner.intern_material(&local_materials[1]) as i64;
        let gid_material_3 = interner.intern_material(&local_materials[3]) as i64;
        let gid_texture_0 = interner.intern_texture(&local_textures[0]) as i64;
        let gid_texture_2 = interner.intern_texture(&local_textures[2]) as i64;
        let uv = vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]];
        assert_eq!(
            *slot.material.as_ref().unwrap(),
            MaterialCell {
                themes: vec![(
                    "visual".to_string(),
                    vec![Some(gid_material_1), Some(gid_material_3)],
                )],
            },
            "the material cell is flat per WKB face across the shell nesting"
        );
        assert_eq!(
            *slot.texture.as_ref().unwrap(),
            TextureCell {
                themes: vec![(
                    "visual".to_string(),
                    vec![
                        vec![TextureRing {
                            id: Some(gid_texture_0),
                            uv: Some(uv.clone()),
                        }],
                        vec![TextureRing {
                            id: Some(gid_texture_2),
                            uv: Some(uv),
                        }],
                    ],
                )],
            },
            "the texture cell is flat per WKB face across the shell nesting"
        );
    }

    /// MultiSolid variant: 2 solids (solid0 has 2 shells of 2+1 faces,
    /// solid1 has 1 shell of 2 faces), with drops spanning BOTH a
    /// shell-within-solid boundary (solid0 shell1's only face, flat
    /// position 2) and the last face overall (solid1 shell0's second face,
    /// flat position 4) — exercising `solid_face_nesting_depth(MultiSolid)
    /// == 2` and proving flat positions are counted depth-first across
    /// solids AND shells, matching `wkb_write::normalise_shells`'s `pos`
    /// counter. Solid0's shell1 ends up with zero faces after its only face
    /// is dropped — a legal (if degenerate) empty shell.
    #[test]
    fn multisolid_flattens_semantics_material_and_texture_across_solids_and_shells() {
        let co: CityObject = serde_json::from_value(serde_json::json!({
            "type": "Building",
            "geometry": [{
                "type": "MultiSolid",
                "lod": "2",
                "boundaries": [
                    [
                        [[[0, 1, 2]], [[1, 2, 3]]],
                        [[[0, 1]]]
                    ],
                    [
                        [[[0, 2, 3]], [[2, 3]]]
                    ]
                ],
                "semantics": {
                    "surfaces": [{"type": "A"}],
                    "values": [[[10, 11], [12]], [[13, 14]]]
                },
                "material": {"visual": {"values": [[[1, 2], [3]], [[4, 5]]]}},
                // one UV index per distinct ring vertex, as the column
                // requires
                "texture": {
                    "visual": {
                        "values": [
                            [[[[0, 0, 1, 2]], [[1, 0, 1, 2]]], [[[2, 0, 1, 2]]]],
                            [[[[3, 0, 1, 2]], [[4, 0, 1, 2]]]]
                        ]
                    }
                }
            }]
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

        // Local defs sized to cover the raw indices above (material 1..5,
        // texture 0..4, UV 0..2).
        let local_materials: Vec<Value> = (0..6)
            .map(|i| serde_json::json!({"name": format!("m{i}")}))
            .collect();
        let local_textures: Vec<Value> = (0..5)
            .map(|i| serde_json::json!({"type": "PNG", "image": format!("t{i}.png")}))
            .collect();
        let local_uvs: Vec<Vec<f64>> = vec![vec![0.0, 0.0], vec![1.0, 0.0], vec![1.0, 1.0]];

        let mut acc = GeometryAccumulator::default();
        let mut stats = EncodeStats::default();
        let mut interner = AppearanceInterner::new();
        let defs = LocalDefs {
            materials: &local_materials,
            textures: &local_textures,
            uvs: &local_uvs,
        };
        accumulate_geometry(
            &mut acc,
            &co,
            &pool,
            true,
            &mut stats,
            &mut interner,
            &defs,
            "obj1",
        )
        .unwrap();

        assert_eq!(stats.degenerate_rings_dropped, 2);
        assert_eq!(stats.degenerate_surfaces_dropped, 2);

        let slot = acc.slots.get("lod2_0").expect("lod2_0 slot populated");
        let props = &slot.properties;
        assert_eq!(
            props.shells,
            Some(vec![vec![2, 0], vec![1]]),
            "solid0's shells drop to (2, 0) faces, solid1's shell drops to 1"
        );
        assert_eq!(
            props.face_semantics,
            Some(vec![Some(10), Some(11), Some(13)]),
            "face_semantics is flat across all solids/shells, losing positions 2 and 4"
        );

        // Flat positions 2 (material 3, texture 2) and 4 (material 5,
        // texture 4) were dropped; the survivors are rewritten to
        // dataset-global ids, flat across solids and shells.
        let gid_material_1 = interner.intern_material(&local_materials[1]) as i64;
        let gid_material_2 = interner.intern_material(&local_materials[2]) as i64;
        let gid_material_4 = interner.intern_material(&local_materials[4]) as i64;
        assert_eq!(
            *slot.material.as_ref().unwrap(),
            MaterialCell {
                themes: vec![(
                    "visual".to_string(),
                    vec![
                        Some(gid_material_1),
                        Some(gid_material_2),
                        Some(gid_material_4),
                    ],
                )],
            },
            "the material cell is flat per WKB face across solids and shells"
        );
        let gid_texture_0 = interner.intern_texture(&local_textures[0]) as i64;
        let gid_texture_1 = interner.intern_texture(&local_textures[1]) as i64;
        let gid_texture_3 = interner.intern_texture(&local_textures[3]) as i64;
        let uv = vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]];
        let ring = |id: i64| {
            vec![TextureRing {
                id: Some(id),
                uv: Some(uv.clone()),
            }]
        };
        assert_eq!(
            *slot.texture.as_ref().unwrap(),
            TextureCell {
                themes: vec![(
                    "visual".to_string(),
                    vec![
                        ring(gid_texture_0),
                        ring(gid_texture_1),
                        ring(gid_texture_3),
                    ],
                )],
            },
            "the texture cell is flat per WKB face across solids and shells"
        );
    }
}
