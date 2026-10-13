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
use cjseq::{
    CityJSONFeature, CityObject, CityObjectType, Geometry, MaterialObject, TextureObject, Transform,
};
use serde_json::Value;

use cityparquet_schema::crs::AxisOrder;
use cityparquet_schema::{AttributeType, CityParquetError, ExtensionNaming, Lod, Result};

use crate::appearance::AppearanceInterner;
use crate::appearance_columns::{
    MaterialCell, MaterialCellBuilder, TextureCell, TextureCellBuilder,
};
use crate::geometry_properties::{GeometryProperties, GeometryPropertiesBuilder};
use crate::scan::ScanResult;
use crate::source::{FeatureIter, Source};
use crate::wkb_write::{VertexPool, geometry_bbox, geometry_to_wkb, point_to_wkb};

/// Members carried by a dedicated column, stripped from the `other` payload
/// (§5.1). `children_roles` has its own column (G5); `address` has its own
/// reserved struct column; `geographicalExtent` is carried by `bbox`, which
/// unions it with the object's computed subtree extent; the rest are cjseq's
/// typed fields.
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

/// The object's members cjseq has no typed field for (`address`, and the
/// `other` leftovers) — read once per row by [`RowWriter::push_object`] and
/// shared by every consumer of them. The typed fields (`type`, `attributes`,
/// `geometry`, `children`, `children_roles`, `parents`,
/// `geographicalExtent`) are never in it.
pub(crate) fn object_json(co: &CityObject) -> Result<serde_json::Map<String, Value>> {
    Ok(co.other.clone())
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

/// The column a source attribute is stored under: its extension name with the
/// namespace prefix (spec "Extensions"), escaped with a leading `_` when it
/// collides with a reserved column name (spec "Column naming and reservation
/// rules"). `scan` and the row writer both key attributes by this name.
pub(crate) fn stored_attribute_name(naming: &ExtensionNaming, source: &str) -> Result<String> {
    Ok(cityparquet_schema::reserved::escape_attribute_name(
        &naming.encode_name(source, "attribute")?,
    ))
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
    /// LoD0 footprints synthesised into the `geometry_lod0_0` column for
    /// objects lacking a source LoD0 (spec "LoD0 synthesis").
    pub synthesized_lod0_footprints: usize,
    /// Unmapped top-level members dropped because an attribute of the same
    /// name exists on the object: the attribute
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
/// Reorder a `[minx, miny, minz, maxx, maxy, maxz]` extent between the
/// dataset's axis order and WKB's. Its own inverse, like [`AxisOrder::apply`].
pub(crate) fn reorder_extent(extent: [f64; 6], axis_order: AxisOrder) -> [f64; 6] {
    let lo = axis_order.apply([extent[0], extent[1], extent[2]]);
    let hi = axis_order.apply([extent[3], extent[4], extent[5]]);
    [lo[0], lo[1], lo[2], hi[0], hi[1], hi[2]]
}

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
    let mut pos = 0;
    let mut stored = |shells: &[cjseq::Shell]| -> Result<Vec<i32>> {
        shells
            .iter()
            .map(|shell| {
                face_count_i32(shell.len() - dropped_in_shell(dropped, &mut pos, shell.len()))
            })
            .collect()
    };
    match geom {
        // One solid -> one inner list, even though a Solid's own boundaries
        // have no outer "per-solid" nesting of their own.
        Geometry::Solid { boundaries, .. } => Ok(Some(vec![stored(boundaries)?])),
        Geometry::MultiSolid { boundaries, .. } | Geometry::CompositeSolid { boundaries, .. } => {
            Ok(Some(
                boundaries
                    .iter()
                    .map(|solid| stored(solid))
                    .collect::<Result<_>>()?,
            ))
        }
        _ => Ok(None),
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
    let type_name = crate::geometry_properties::cm_geometry_type(&geom.geometry_type())?;

    let mut surfaces = None;
    let mut face_semantics = None;
    if let Some((defs, flat)) = crate::faces::semantics(geom) {
        surfaces = Some(serde_json::to_value(defs)?);
        // `flat` has exactly one entry per ORIGINAL face (a short source
        // `values` padded with null, §8 "defensive on source"); align it to
        // the EMITTED faces by dropping the writer-removed positions.
        let dropped: std::collections::HashSet<usize> = dropped_surfaces.iter().copied().collect();
        let entries: Vec<Option<i32>> = flat
            .into_iter()
            .enumerate()
            .filter(|(i, _)| !dropped.contains(i))
            .map(|(_, v)| v.map(|s| face_count_i32(s.0)).transpose())
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

/// `(relative-geometry id, WKB point, transformationMatrix)` — one resolved
/// `implicit_geometry` column's worth of data. The matrix is the flat, row-major
/// 16-value list the reserved `LIST<DOUBLE>` column stores (spec "Appearance
/// & implicit geometries").
type ImplicitGeometryFields = (i64, Vec<u8>, Option<Vec<f64>>);

/// Checks a CityJSON `transformationMatrix` — a flat row-major 4x4, which
/// the geometry's type already holds to exactly 16 values — is affine: its
/// last row MUST be exactly `0, 0, 0, 1` (spec "Placement").
fn check_transformation_matrix(values: &[f64; 16]) -> Result<Vec<f64>> {
    if values[12..] != [0.0, 0.0, 0.0, 1.0] {
        return Err(CityParquetError::Schema(format!(
            "implicit_geometry.transformationMatrix must be affine, its last row 0, 0, 0, 1; \
             got {:?} (a projective matrix has no meaning for an implicit geometry)",
            &values[12..]
        )));
    }
    Ok(values.to_vec())
}

/// `implicit_geometry` binding rule: built from the first CityJSON
/// `GeometryInstance` geometry on the object; `None` for any other geometry.
/// A projective `transformationMatrix` is a hard error, not a graceful drop —
/// see [`check_transformation_matrix`].
fn build_implicit_geometry(
    geom: &Geometry,
    pool: &VertexPool,
) -> Result<Option<ImplicitGeometryFields>> {
    let Geometry::GeometryInstance {
        boundaries: [vertex],
        template,
        transformation_matrix,
    } = geom
    else {
        return Ok(None);
    };
    let point = point_to_wkb(pool.coord(vertex.0)?);
    let matrix = check_transformation_matrix(transformation_matrix)?;
    // CityJSON's `template` is the index into `geometry-templates.templates`,
    // which `build_implicit_geometry_rows` also uses as the sidecar row's `id` — so this
    // reference resolves by value against `implicit_geometries.parquet`.
    Ok(Some((template.0 as i64, point, Some(matrix))))
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
/// whole conversion (mirrors [`build_implicit_geometry`]'s graceful-degradation
/// style).
fn build_location_wkb(location: &Geometry, pool: &VertexPool) -> Option<Vec<u8>> {
    match location {
        Geometry::MultiPoint { boundaries, .. } if !boundaries.is_empty() => {}
        _ => return None,
    }
    // The location's `lod` plays no part in its WKB.
    geometry_to_wkb(location, pool)
        .ok()
        .flatten()
        .map(|o| o.bytes)
}

/// Build the reserved `address` column's row value: one [`AddressRow`] per
/// source entry, in order — cardinality is preserved even when an entry maps
/// to nothing recognised (an all-`None` struct still occupies its list
/// position), since "how many addresses" is itself meaningful. `None` when
/// the object carries no `address` member at all (column cell null).
fn build_address_rows(co: &CityObject, pool: &VertexPool) -> Option<Vec<AddressRow>> {
    let entries = co.address.as_ref()?;
    let rows = entries
        .iter()
        .map(|entry| AddressRow {
            postal: crate::address::map_postal_fields(&entry.other),
            location: entry
                .location
                .as_ref()
                .and_then(|loc| build_location_wkb(loc, pool)),
        })
        .collect();
    Some(rows)
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
    implicit_geometry: Option<ImplicitGeometryFields>,
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
fn feature_local_materials(feature: &CityJSONFeature) -> &[MaterialObject] {
    feature
        .appearance
        .as_ref()
        .and_then(|a| a.materials.as_deref())
        .unwrap_or(&[])
}

/// This feature's local texture definitions (see [`feature_local_materials`]).
fn feature_local_textures(feature: &CityJSONFeature) -> &[TextureObject] {
    feature
        .appearance
        .as_ref()
        .and_then(|a| a.textures.as_deref())
        .unwrap_or(&[])
}

/// This feature's local UV vertex pool (see [`feature_local_materials`]).
fn feature_local_uvs(feature: &CityJSONFeature) -> &[[f64; 2]] {
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
    pub materials: &'a [MaterialObject],
    pub textures: &'a [TextureObject],
    pub uvs: &'a [[f64; 2]],
}

/// Flatten one geometry's `material`/`texture` maps into the typed cells the
/// `material_lod*`/`texture_lod*` columns store — flat per WKB face, with
/// dataset-global ids from `interner` and UV pairs inlined — and build its
/// `geometry_properties` struct value. This is the exact per-geometry
/// appearance pipeline [`accumulate_geometry`] runs for a feature's own
/// geometries, factored out so the implicit-geometries sidecar
/// (`crate::package`) can run the identical rules over `Source::header`'s
/// `geometry_templates` after the main encode pass, through the SAME
/// interner — a CityJSON template's `material`/`texture`/`semantics` follow the same
/// CityJSON shapes as a regular geometry's, so the same rules apply verbatim.
///
/// A cell is `None` when the geometry carries no map at all AND when the map
/// it carries has no themes: the columns are nullable and the spec reserves
/// the null cell for "no material (or texture) in any theme", so an empty
/// map is written as a null cell rather than as an empty MAP.
///
/// `context` names the geometry in any interner error surfaced (e.g.
/// `"object abc123"` or `"relative geometry 0"`).
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
    naming: &ExtensionNaming,
) -> Result<(
    Option<MaterialCell>,
    Option<TextureCell>,
    GeometryProperties,
)> {
    let faces = crate::faces::Faces::of(geom);
    let material = match crate::faces::material_themes(geom) {
        Some(themes) => {
            let cell = interner
                .flatten_material_map(&themes, faces.count(), dropped_surfaces, defs.materials)
                .map_err(|e| {
                    CityParquetError::Schema(format!(
                        "{context}: cannot resolve material map to global ids: {e}"
                    ))
                })?;
            (!cell.themes.is_empty()).then_some(cell)
        }
        None => None,
    };

    let texture = match crate::faces::texture_themes(geom) {
        Some(themes) => {
            let cell = interner
                .flatten_texture_map(
                    &themes,
                    &faces.ring_vertex_counts(),
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

    let mut props = compute_geometry_properties(geom, dropped_surfaces)?;
    namespace_surface_types(&mut props, naming)?;
    Ok((material, texture, props))
}

/// Stores each semantic surface's `type` through `naming` (spec
/// "Extensions"): a CityJSON `+` surface type takes its extension's
/// namespace prefix (`+PartyWallSurface` → `energy_PartyWallSurface`); a core
/// type is kept as is.
fn namespace_surface_types(props: &mut GeometryProperties, naming: &ExtensionNaming) -> Result<()> {
    let Some(Value::Array(surfaces)) = props.surfaces.as_mut() else {
        return Ok(());
    };
    for surface in surfaces {
        if let Some(Value::String(surface_type)) = surface.get_mut("type") {
            *surface_type = naming.encode_name(surface_type, "semantic surface type")?;
        }
    }
    Ok(())
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
    naming: &ExtensionNaming,
) -> Result<()> {
    let Some(geoms) = &co.geometry else {
        return Ok(());
    };
    for geom in geoms {
        if matches!(geom, Geometry::GeometryInstance { .. }) {
            if acc.implicit_geometry.is_none() {
                // Name the object in a malformed-matrix error.
                acc.implicit_geometry =
                    build_implicit_geometry(geom, pool).map_err(|e| match e {
                        CityParquetError::Schema(msg) => {
                            CityParquetError::Schema(format!("object '{id}': {msg}"))
                        }
                        other => other,
                    })?;
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
        // payload (instances are routed to `implicit_geometry` above). For a
        // `ScanResult` that matches this `Source`, [`crate::scan`] guarantees
        // such a geometry carries a valid lod and that the dataset is
        // therefore per-LoD — so both a missing/unparseable lod and
        // `per_lod == false` mean the scan does not match the source
        // (`encode` is public and takes an independent scan; a Seq file is
        // also reopened between the scan and encode passes). Reject rather
        // than silently drop or misplace the geometry.
        let lod = match crate::cjtypes::geometry_lod(geom) {
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
        // implicit-geometries sidecar, see its doc comment).
        let (material, texture, props) = rewrite_geometry_appearance(
            geom,
            &dropped_surfaces,
            interner,
            defs,
            &format!("object {id}"),
            naming,
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
            geom,
            Geometry::Solid { .. }
                | Geometry::MultiSolid { .. }
                | Geometry::CompositeSolid { .. }
                | Geometry::MultiSurface { .. }
                | Geometry::CompositeSurface { .. }
        ) {
            continue;
        }
        let Some(lod) = crate::cjtypes::geometry_lod(geom) else {
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
    // `verts` were produced from `pool` — already in WKB axis order — so this
    // pool must NOT reorder them a second time.
    let raw = VertexPool::raw(&verts, AxisOrder::LonLat);

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
    implicit_geometry_id: Int64Builder,
    implicit_geometry_point: BinaryBuilder,
    implicit_geometry_matrix: ListBuilder<Float64Builder>,
    implicit_geometry_nulls: NullBufferBuilder,
    other: StringBuilder,
    attributes: Vec<(String, AttrBuilder)>,
    /// When `Some`, synthesise an LoD0 footprint into the `geometry_lod0_0`
    /// slot for any object lacking a source LoD0 (spec "LoD0 synthesis").
    /// Carries the thresholds.
    synthesize_lod0: Option<crate::lod0::Lod0Options>,
    /// The dataset's axis order ([`ScanResult::axis_order`]), applied to every
    /// vertex this writer turns into WKB or a `bbox`.
    axis_order: AxisOrder,
    /// The declared extensions' naming policy ([`ScanResult::naming`]):
    /// every extension attribute, surface type and class is stored under its
    /// namespace prefix.
    naming: ExtensionNaming,
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
        // A dataset without LoDs carries no geometry column (spec "Levels of
        // detail"), so it has no slot either.
        let geometry_slots = scan
            .lods
            .iter()
            .map(|lod| new_slot(lod.column_suffix()))
            .collect();
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
            implicit_geometry_id: Int64Builder::new(),
            implicit_geometry_point: BinaryBuilder::new(),
            // Item field explicitly pinned to non-null Float64: a
            // `ListBuilder`'s default-derived item field is always nullable,
            // which would mismatch `implicit_geometry_data_type()`'s non-null matrix
            // entries at `StructArray::new` time.
            implicit_geometry_matrix: ListBuilder::new(Float64Builder::new())
                .with_field(Arc::new(Field::new("item", DataType::Float64, false))),
            implicit_geometry_nulls: NullBufferBuilder::new(0),
            other: StringBuilder::new(),
            attributes,
            synthesize_lod0: scan.synthesize_lod0,
            axis_order: scan.axis_order,
            naming: scan.naming(),
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

    fn push_implicit_geometry(&mut self, implicit_geometry: Option<ImplicitGeometryFields>) {
        match implicit_geometry {
            Some((id, point, matrix)) => {
                self.implicit_geometry_id.append_value(id);
                self.implicit_geometry_point.append_value(point);
                match matrix {
                    Some(m) => self
                        .implicit_geometry_matrix
                        .append_value(m.into_iter().map(Some)),
                    None => self.implicit_geometry_matrix.append_null(),
                }
                self.implicit_geometry_nulls.append(true);
            }
            None => {
                self.implicit_geometry_id.append_null();
                self.implicit_geometry_point.append_null();
                self.implicit_geometry_matrix.append_null();
                self.implicit_geometry_nulls.append(false);
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
    /// never `children_roles`.
    ///
    /// A present `children_roles` MUST have exactly one entry per child
    /// (CityJSON 2.0.1 §2.5), each a string; anything else is rejected, not
    /// silently coerced, so a corrupt role list can never be stored.
    fn children_roles(co: &CityObject, id: &str) -> Result<Option<Vec<String>>> {
        if co.thetype != CityObjectType::CityObjectGroup {
            return Ok(None);
        }
        let Some(roles) = &co.children_roles else {
            return Ok(None);
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
                role.clone().ok_or_else(|| {
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
        let pool = VertexPool::new(&feature.vertices, transform, self.axis_order);
        // The ONE whole-object serialisation this row performs (review P5b);
        // children_roles / other read from this map.
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
        // so it keeps its own class name, with CityJSON's leading `+` marker
        // replaced by the extension's namespace prefix (spec "Extensions":
        // `+ThermalZone` → `energy_ThermalZone`) so export can restore it.
        let thetype = crate::cjtypes::object_type_name(&co.thetype);
        let object_type = match cityparquet_schema::class_info(&thetype) {
            Some(info) => info.citygml_class.to_string(),
            None => self.naming.encode_name(&thetype, "city-object type")?,
        };
        self.object_type.append(&object_type)?;
        Self::push_string_list(&mut self.parents, co.parents.as_deref());
        Self::push_string_list(&mut self.children, co.children.as_deref());
        let children_roles = Self::children_roles(co, id)?;
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
            &self.naming,
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

        let address_rows = build_address_rows(co, &pool);

        // `other`: the single escape hatch (§5.1) for source members with no
        // dedicated column. Null when the object has none.
        //
        // Where an unmapped top-level member and an attribute share a key, the
        // attribute wins ("warn and prefer attribute"): keeping the member
        // would make this writer emit an `other` entry duplicating the
        // attribute's own (restored) name, which a reader MUST-errors on
        // (`merge_other_members` in `src/decode.rs`). The member is dropped
        // instead, counted and diagnosed rather than failing the conversion.
        let unmapped = unmapped_from_json(co_json);
        let attrs = co.attributes.as_ref();
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
            // A declared `geographicalExtent` is in the SOURCE's axis order,
            // while `bbox` — like every geometry column — is stored in WKB's.
            // Unioning the two unreordered would inflate the box across half
            // the globe for a latitude-first CRS.
            union_bbox(&mut bbox, reorder_extent(extent, self.axis_order));
        }
        self.push_bbox(bbox);
        self.push_implicit_geometry(acc.implicit_geometry);

        let normalised: HashMap<String, &Value> = co
            .attributes
            .as_ref()
            .map(|obj| {
                obj.iter()
                    .map(|(k, v)| Ok((stored_attribute_name(&self.naming, k)?, v)))
                    .collect::<Result<_>>()
            })
            .transpose()?
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

    fn finish_implicit_geometry(&mut self) -> ArrayRef {
        let DataType::Struct(fields) = cityparquet_schema::model::implicit_geometry_data_type()
        else {
            unreachable!("implicit_geometry_data_type always returns Struct")
        };
        let arrays: Vec<ArrayRef> = vec![
            Arc::new(self.implicit_geometry_id.finish()),
            Arc::new(self.implicit_geometry_point.finish()),
            Arc::new(self.implicit_geometry_matrix.finish()),
        ];
        let nulls = self.implicit_geometry_nulls.finish();
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
        arrays.push(self.finish_implicit_geometry());
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
pub struct BatchIter<'a> {
    features: FeatureIter<'a>,
    transform: Transform,
    schema: Arc<Schema>,
    batch_size: usize,
    writer: RowWriter,
    current_feature: Option<CityJSONFeature>,
    current_object_ids: Vec<String>,
    /// Set by [`BatchIter::with_hilbert_keys`]: each row is tagged with the
    /// Hilbert key of its feature.
    hilbert: Option<HilbertKeys>,
    /// The current feature's Hilbert key.
    current_key: u32,
    /// The keys of the rows of the batch being built, then of the batch last
    /// returned.
    keys: Vec<u32>,
    batch_keys: Vec<u32>,
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

/// What a row's Hilbert key is computed against.
struct HilbertKeys {
    dataset_bbox: [f64; 6],
    axis_order: AxisOrder,
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
    /// implicit-geometries sidecar in `crate::package`) can fold MORE
    /// definitions into it — e.g. entries reachable only from
    /// `Source::header`'s `geometry_templates`, which this encode loop never
    /// visits — before [`Self::appearance`] is read to write the
    /// materials/textures sidecars. Doing so keeps every appearance
    /// definition in ONE dataset-global id space, however it was
    /// discovered.
    pub fn appearance_mut(&mut self) -> &mut AppearanceInterner {
        &mut self.appearance
    }

    /// Tag every row with the Hilbert key of its feature — the curve index
    /// of the centre of the feature's own vertex pool against the dataset
    /// bbox (`crate::order::centre_hilbert_key`) — read after each batch with
    /// [`Self::take_batch_keys`].
    pub(crate) fn with_hilbert_keys(mut self, scan: &ScanResult) -> Self {
        self.hilbert = Some(HilbertKeys {
            dataset_bbox: scan.dataset_bbox.unwrap_or([0.0; 6]),
            axis_order: scan.axis_order,
        });
        self
    }

    /// The Hilbert keys of the rows of the batch `next` last returned (empty
    /// without [`Self::with_hilbert_keys`]).
    pub(crate) fn take_batch_keys(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.batch_keys)
    }

    fn finish_batch(&mut self) -> Result<RecordBatch> {
        self.batch_keys = std::mem::take(&mut self.keys);
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
                        // spec 02 "Feature families are contiguous": the
                        // root row opens its family's run, the rest follow
                        // in id order.
                        let mut ids: Vec<String> = feature.city_objects.keys().cloned().collect();
                        ids.sort_by(|a, b| (*a != feature.id, a).cmp(&(*b != feature.id, b)));
                        if let Some(keys) = &self.hilbert {
                            self.current_key = crate::order::centre_hilbert_key(
                                crate::order::feature_centre(&feature.vertices, &self.transform),
                                &keys.dataset_bbox,
                                keys.axis_order,
                            );
                        }
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
            if self.hilbert.is_some() {
                self.keys.push(self.current_key);
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
    let features = source.features_batched()?;
    let transform = source.header().transform.clone();
    let writer = RowWriter::new(scan);
    Ok(BatchIter {
        features,
        transform,
        schema,
        batch_size: batch_size.max(1),
        writer,
        current_feature: None,
        current_object_ids: Vec::new(),
        hilbert: None,
        current_key: 0,
        keys: Vec::new(),
        batch_keys: Vec::new(),
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
        let pool = VertexPool::raw(&[], AxisOrder::LonLat);
        let rows = build_address_rows(&co, &pool).unwrap();
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
        let pool = VertexPool::raw(&[], AxisOrder::LonLat);
        assert!(build_address_rows(&co, &pool).is_none());
    }

    #[test]
    fn check_transformation_matrix_accepts_an_affine_matrix() {
        let mut values = [0.0; 16];
        for (i, v) in values.iter_mut().take(12).enumerate() {
            *v = i as f64;
        }
        values[15] = 1.0;
        assert_eq!(
            check_transformation_matrix(&values).unwrap(),
            values.to_vec()
        );
    }

    /// spec "Placement": the last row MUST be exactly `0, 0, 0, 1`.
    #[test]
    fn check_transformation_matrix_rejects_a_projective_matrix() {
        let values: [f64; 16] = std::array::from_fn(|i| i as f64);
        let err = check_transformation_matrix(&values).unwrap_err();
        assert!(format!("{err}").contains("0, 0, 0, 1"), "{err}");
    }

    /// No fixture carries MultiSolid/CompositeSolid, so the nested `shells`
    /// branch gets direct coverage here: 2 solids — first with shells of
    /// (1, 2) faces, second with one 1-face shell.
    /// 5-level MultiSolid shape: solids → shells → surfaces → rings → indices.
    #[test]
    fn multisolid_shells_nest_per_solid() {
        let geom = crate::faces::geometry(serde_json::json!({
            "type": "MultiSolid",
            "lod": "2",
            "boundaries": [
                [[[[0, 1, 2]]], [[[0, 1, 2]], [[0, 1, 3]]]],
                [[[[0, 2, 3]]]]
            ]
        }));
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
        let vertices = [[0, 0, 0], [1000, 0, 0], [1000, 1000, 0], [0, 1000, 0]];
        let transform = Transform {
            scale: [1.0; 3],
            translate: [0.0; 3],
        };
        let pool = VertexPool::new(&vertices, &transform, AxisOrder::LonLat);

        // Local defs sized to cover every raw index the fixture geometry
        // above references (material indices up to 7, texture index up to
        // 1, UV indices up to 3): the interner rewrite now resolves every
        // index against a real local def, so these must exist.
        let local_materials: Vec<MaterialObject> = (0..8)
            .map(|i| serde_json::from_value(serde_json::json!({"name": format!("m{i}")})).unwrap())
            .collect();
        let local_textures: Vec<TextureObject> = (0..2)
            .map(|i| {
                serde_json::from_value(
                    serde_json::json!({"type": "PNG", "image": format!("t{i}.png")}),
                )
                .unwrap()
            })
            .collect();
        let local_uvs = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];

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
            &ExtensionNaming::default(),
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
        let gid_material_7 =
            interner.intern_material(&serde_json::to_value(&local_materials[7]).unwrap()) as i64;
        let gid_texture_1 =
            interner.intern_texture(&serde_json::to_value(&local_textures[1]).unwrap()) as i64;
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
                    "surfaces": [{"type": "WallSurface"}, {"type": "RoofSurface"}, {"type": "GroundSurface"}],
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
        let vertices = [[0, 0, 0], [1000, 0, 0], [1000, 1000, 0], [0, 1000, 0]];
        let transform = Transform {
            scale: [1.0; 3],
            translate: [0.0; 3],
        };
        let pool = VertexPool::new(&vertices, &transform, AxisOrder::LonLat);

        // Local defs sized to cover the raw indices above (material 1..3,
        // texture 0..2, UV 0..2).
        let local_materials: Vec<MaterialObject> = (0..4)
            .map(|i| serde_json::from_value(serde_json::json!({"name": format!("m{i}")})).unwrap())
            .collect();
        let local_textures: Vec<TextureObject> = (0..3)
            .map(|i| {
                serde_json::from_value(
                    serde_json::json!({"type": "PNG", "image": format!("t{i}.png")}),
                )
                .unwrap()
            })
            .collect();
        let local_uvs = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]];

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
            &ExtensionNaming::default(),
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
            Some(
                serde_json::json!([{"type": "WallSurface"}, {"type": "RoofSurface"}, {"type": "GroundSurface"}])
            ),
            "the surfaces lookup table is stored verbatim (face_semantics indexes into it)"
        );

        // Face 1 (material index 2, texture index 1) was dropped; faces 0
        // and 2 survive, rewritten to dataset-global ids and flattened out
        // of the shell nesting.
        let gid_material_1 =
            interner.intern_material(&serde_json::to_value(&local_materials[1]).unwrap()) as i64;
        let gid_material_3 =
            interner.intern_material(&serde_json::to_value(&local_materials[3]).unwrap()) as i64;
        let gid_texture_0 =
            interner.intern_texture(&serde_json::to_value(&local_textures[0]).unwrap()) as i64;
        let gid_texture_2 =
            interner.intern_texture(&serde_json::to_value(&local_textures[2]).unwrap()) as i64;
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
                    "surfaces": [{"type": "WallSurface"}],
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
        let vertices = [[0, 0, 0], [1000, 0, 0], [1000, 1000, 0], [0, 1000, 0]];
        let transform = Transform {
            scale: [1.0; 3],
            translate: [0.0; 3],
        };
        let pool = VertexPool::new(&vertices, &transform, AxisOrder::LonLat);

        // Local defs sized to cover the raw indices above (material 1..5,
        // texture 0..4, UV 0..2).
        let local_materials: Vec<MaterialObject> = (0..6)
            .map(|i| serde_json::from_value(serde_json::json!({"name": format!("m{i}")})).unwrap())
            .collect();
        let local_textures: Vec<TextureObject> = (0..5)
            .map(|i| {
                serde_json::from_value(
                    serde_json::json!({"type": "PNG", "image": format!("t{i}.png")}),
                )
                .unwrap()
            })
            .collect();
        let local_uvs = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]];

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
            &ExtensionNaming::default(),
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
        let gid_material_1 =
            interner.intern_material(&serde_json::to_value(&local_materials[1]).unwrap()) as i64;
        let gid_material_2 =
            interner.intern_material(&serde_json::to_value(&local_materials[2]).unwrap()) as i64;
        let gid_material_4 =
            interner.intern_material(&serde_json::to_value(&local_materials[4]).unwrap()) as i64;
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
        let gid_texture_0 =
            interner.intern_texture(&serde_json::to_value(&local_textures[0]).unwrap()) as i64;
        let gid_texture_1 =
            interner.intern_texture(&serde_json::to_value(&local_textures[1]).unwrap()) as i64;
        let gid_texture_3 =
            interner.intern_texture(&serde_json::to_value(&local_textures[3]).unwrap()) as i64;
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
