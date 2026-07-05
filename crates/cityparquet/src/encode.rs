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
    StringBuilder, StringDictionaryBuilder, TimestampMillisecondBuilder,
};
use arrow_array::types::Int32Type;
use arrow_array::{ArrayRef, Float64Array, RecordBatch, StructArray};
use arrow_buffer::NullBufferBuilder;
use arrow_schema::{DataType, Schema};
use cjseq::{CityJSONFeature, CityObject, Geometry, GeometryType, Transform};
use serde_json::Value;

use cityparquet_schema::{AttributeType, Lod, Result, normalise_attribute_name};

use crate::scan::ScanResult;
use crate::source::{FeatureIter, Source};
use crate::wkb_write::{VertexPool, geometry_to_wkb, point_to_wkb};

/// Counters for the row-population edge cases the binding rules ask us to
/// track rather than surface as errors.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EncodeStats {
    /// Extra geometries for an (object, LoD) pair beyond the first one kept.
    pub skipped_same_lod_geometries: usize,
    /// Attribute values present (non-null) but not representable as the
    /// column's inferred type; encoded as null instead of panicking.
    pub attribute_coercion_nulls: usize,
    /// Structurally degenerate rings the writer dropped ([a,b,a] closure
    /// shape; see `wkb_write`), counted over STORED geometries.
    pub degenerate_rings_dropped: usize,
    /// Surfaces the writer dropped because their exterior ring was
    /// degenerate, counted over STORED geometries.
    pub degenerate_surfaces_dropped: usize,
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

/// Union of bboxes over an object's own geometries only (no descendant walk).
fn own_geometry_bbox(co: &CityObject, pool: &VertexPool) -> Result<Option<[f64; 6]>> {
    let mut acc = None;
    if let Some(geoms) = &co.geometry {
        for geom in geoms {
            if let Some(outcome) = geometry_to_wkb(geom, pool)? {
                union_bbox(&mut acc, outcome.bbox);
            }
        }
    }
    Ok(acc)
}

/// Recursive descendant-bbox fallback: union over every descendant's own
/// bbox (recursing further when a child itself has no geometry), cycle
/// guarded with a visited set.
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
            match own_geometry_bbox(child, pool)? {
                Some(bbox) => union_bbox(&mut acc, bbox),
                None => {
                    if let Some(bbox) = descendant_bbox(child, feature, pool, visited)? {
                        union_bbox(&mut acc, bbox);
                    }
                }
            }
        }
    }
    Ok(acc)
}

/// `bbox` binding rule: the object's own geometry bboxes, falling back to a
/// cycle-guarded recursive union over descendant bboxes when the object has
/// none of its own; `None` if the whole subtree has no geometry.
fn resolve_bbox(
    own_bbox: Option<[f64; 6]>,
    id: &str,
    co: &CityObject,
    feature: &CityJSONFeature,
    pool: &VertexPool,
) -> Result<Option<[f64; 6]>> {
    if own_bbox.is_some() {
        return Ok(own_bbox);
    }
    let mut visited = HashSet::new();
    visited.insert(id.to_string());
    descendant_bbox(co, feature, pool, &mut visited)
}

/// `solid_shell_counts` + `solid_shell_faces` payload, only meaningful for
/// Solid/MultiSolid/CompositeSolid (what the WKB `PolyhedralSurfaceZ`
/// flattening loses): the former is the number of shells per solid, the
/// latter the number of faces per shell — enough for a reader to
/// re-partition a flattened `PolyhedralSurfaceZ` back into shells.
struct SolidShellInfo {
    /// `solid_shell_counts`: number of shells per solid.
    counts: Vec<usize>,
    /// `solid_shell_faces`: `[n_faces_shell0, ...]` for `Solid`, or one such
    /// array per solid (nested) for `MultiSolid`/`CompositeSolid`.
    faces: Value,
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

/// `dropped` are the writer-reported flat face positions removed from the
/// WKB; the per-shell face counts must describe the STORED geometry, so
/// each shell's count is reduced by the drops that fell inside it (the sum
/// must equal the WKB `PolyhedralSurfaceZ` face count).
fn solid_shell_info(geom: &Geometry, dropped: &[usize]) -> Result<Option<SolidShellInfo>> {
    match geom.thetype {
        GeometryType::Solid => {
            let shells: Vec<Vec<Vec<Vec<usize>>>> =
                serde_json::from_value(geom.boundaries.clone())?;
            let mut pos = 0;
            let faces: Vec<usize> = shells
                .iter()
                .map(|shell| shell.len() - dropped_in_shell(dropped, &mut pos, shell.len()))
                .collect();
            Ok(Some(SolidShellInfo {
                counts: vec![shells.len()],
                faces: serde_json::to_value(faces)?,
            }))
        }
        GeometryType::MultiSolid | GeometryType::CompositeSolid => {
            let solids: Vec<Vec<Vec<Vec<Vec<usize>>>>> =
                serde_json::from_value(geom.boundaries.clone())?;
            let counts = solids.iter().map(|s| s.len()).collect();
            let mut pos = 0;
            let faces: Vec<Vec<usize>> = solids
                .iter()
                .map(|solid| {
                    solid
                        .iter()
                        .map(|shell| shell.len() - dropped_in_shell(dropped, &mut pos, shell.len()))
                        .collect()
                })
                .collect();
            Ok(Some(SolidShellInfo {
                counts,
                faces: serde_json::to_value(faces)?,
            }))
        }
        _ => Ok(None),
    }
}

/// Remove the entries at `dropped` (original positions, ascending) from a
/// per-surface JSON array, in place. Positions beyond the array are ignored
/// (defensive: a malformed source array shorter than the boundaries).
fn remove_dropped_entries(values: &mut Vec<Value>, dropped: &[usize]) {
    for &pos in dropped.iter().rev() {
        if pos < values.len() {
            values.remove(pos);
        }
    }
}

/// True when the writer's per-surface drop positions index straight into
/// this geometry type's per-surface appearance/semantics arrays. Only the
/// surface-list types qualify; the solid types nest their semantics values
/// per shell, which no fixture exercises with drops — left unrealigned (the
/// `dropped_degenerate` key still records what happened).
fn drops_align_with_surface_arrays(thetype: &GeometryType) -> bool {
    matches!(
        thetype,
        GeometryType::MultiSurface | GeometryType::CompositeSurface
    )
}

/// Realign every material/texture theme's per-surface `values` array after
/// the writer dropped `dropped` surface positions. Theme-level scalar
/// `value` entries apply to all surfaces and need no realignment.
fn realign_appearance_themes(appearance: &mut Value, dropped: &[usize]) {
    let Some(themes) = appearance.as_object_mut() else {
        return;
    };
    for theme in themes.values_mut() {
        if let Some(values) = theme.get_mut("values").and_then(Value::as_array_mut) {
            remove_dropped_entries(values, dropped);
        }
    }
}

/// `geometry_properties_lod*` JSON: `{"type", "semantics"?,
/// "solid_shell_counts"?, "solid_shell_faces"?, "dropped_degenerate"?}`.
/// `dropped_surfaces` are the writer-reported original surface positions;
/// for the surface-list types the semantics `values` array is realigned to
/// match the stored WKB, and any drop is recorded under
/// `dropped_degenerate` so downstream can trace it back to the source.
fn geometry_properties_json(
    geom: &Geometry,
    dropped_rings: usize,
    dropped_surfaces: &[usize],
) -> Result<String> {
    let mut map = serde_json::Map::new();
    map.insert("type".to_string(), serde_json::to_value(&geom.thetype)?);
    if let Some(semantics) = &geom.semantics {
        let mut semantics = semantics.clone();
        if !dropped_surfaces.is_empty()
            && drops_align_with_surface_arrays(&geom.thetype)
            && let Some(values) = semantics.get_mut("values").and_then(Value::as_array_mut)
        {
            remove_dropped_entries(values, dropped_surfaces);
        }
        map.insert("semantics".to_string(), semantics);
    }
    if let Some(info) = solid_shell_info(geom, dropped_surfaces)? {
        map.insert(
            "solid_shell_counts".to_string(),
            serde_json::to_value(info.counts)?,
        );
        map.insert("solid_shell_faces".to_string(), info.faces);
    }
    if dropped_rings > 0 || !dropped_surfaces.is_empty() {
        map.insert(
            "dropped_degenerate".to_string(),
            serde_json::json!({"rings": dropped_rings, "surfaces": dropped_surfaces}),
        );
    }
    Ok(serde_json::to_string(&Value::Object(map))?)
}

/// `(template id, WKB point, transformationMatrix JSON)` — one resolved
/// `template` column's worth of data.
type TemplateFields = (String, Vec<u8>, Option<String>);

/// `template` binding rule: built from the first `GeometryInstance`
/// geometry on the object; `None` when it can't be resolved (missing
/// template index, empty/malformed boundaries) so callers null the column
/// rather than panic.
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
        .map(serde_json::to_string)
        .transpose()?;
    Ok(Some((template_id.to_string(), point, matrix)))
}

/// Per-object accumulator filled by [`accumulate_geometry`], consumed by
/// [`RowWriter::push_object`] to populate the row's columns.
#[derive(Default)]
struct GeometryAccumulator {
    /// Column slot key (LoD suffix, or `""` for the un-suffixed `geometry`
    /// column) -> (WKB bytes, bbox, geometry_properties JSON).
    slots: HashMap<String, (Vec<u8>, [f64; 6], String)>,
    material: serde_json::Map<String, Value>,
    texture: serde_json::Map<String, Value>,
    template: Option<TemplateFields>,
    own_bbox: Option<[f64; 6]>,
}

/// Walk one object's own geometries, bucketing them into `acc`. `per_lod`
/// mirrors the dataset-wide binding rule from [`crate::scan`]: `true` means
/// every kept geometry needs an `lod` to have a column to live in (lod-less
/// geometries are silently unplaceable, already counted at the dataset
/// level by `scan`'s `lodless_geometries`); `false` means every kept
/// geometry shares the single un-suffixed `geometry` column.
fn accumulate_geometry(
    acc: &mut GeometryAccumulator,
    co: &CityObject,
    pool: &VertexPool,
    per_lod: bool,
    stats: &mut EncodeStats,
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
        // Row bbox deliberately covers ALL source geometry (including skipped
        // duplicate-LoD and lod-less entries): the object occupies that
        // extent, and a superset bbox can only cause false-positive reads,
        // never false-negative pruning.
        union_bbox(&mut acc.own_bbox, outcome.bbox);

        let slot_key = if per_lod {
            match geom.lod.as_deref().and_then(|s| Lod::parse(s).ok()) {
                Some(lod) => lod.column_suffix(),
                None => continue, // lod-less in a mixed dataset: no column to place it in
            }
        } else {
            String::new()
        };

        if acc.slots.contains_key(&slot_key) {
            stats.skipped_same_lod_geometries += 1;
            continue;
        }

        // Counted over STORED geometries only, so the totals describe the
        // data downstream actually sees.
        stats.degenerate_rings_dropped += outcome.dropped_rings;
        stats.degenerate_surfaces_dropped += outcome.dropped_surfaces.len();

        // Writer owns consistency: per-surface appearance arrays must index
        // the STORED surfaces, so the writer-dropped positions are removed
        // here, before anything is written.
        let realign =
            drops_align_with_surface_arrays(&geom.thetype) && !outcome.dropped_surfaces.is_empty();
        let lod_key = geom.lod.clone().unwrap_or_default();
        if let Some(material) = &geom.material {
            let mut material = serde_json::to_value(material)?;
            if realign {
                realign_appearance_themes(&mut material, &outcome.dropped_surfaces);
            }
            acc.material.insert(lod_key.clone(), material);
        }
        if let Some(texture) = &geom.texture {
            let mut texture = serde_json::to_value(texture)?;
            if realign {
                realign_appearance_themes(&mut texture, &outcome.dropped_surfaces);
            }
            acc.texture.insert(lod_key, texture);
        }

        let props =
            geometry_properties_json(geom, outcome.dropped_rings, &outcome.dropped_surfaces)?;
        acc.slots
            .insert(slot_key, (outcome.bytes, outcome.bbox, props));
    }
    Ok(())
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

/// One geometry/geometry_properties column pair for a single LoD (or the
/// single un-suffixed pair when the dataset has no LoDs).
struct GeometrySlot {
    key: String,
    geometry: BinaryBuilder,
    properties: StringBuilder,
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
    bbox_cols: [Vec<f64>; 6],
    bbox_nulls: NullBufferBuilder,
    per_lod: bool,
    geometry_slots: Vec<GeometrySlot>,
    material: StringBuilder,
    texture: StringBuilder,
    template_id: StringBuilder,
    template_point: BinaryBuilder,
    template_matrix: StringBuilder,
    template_nulls: NullBufferBuilder,
    other: StringBuilder,
    attributes: Vec<(String, AttrBuilder)>,
    len: usize,
}

impl RowWriter {
    fn new(scan: &ScanResult) -> Self {
        let per_lod = !scan.lods.is_empty();
        let geometry_slots = if per_lod {
            scan.lods
                .iter()
                .map(|lod| GeometrySlot {
                    key: lod.column_suffix(),
                    geometry: BinaryBuilder::new(),
                    properties: StringBuilder::new(),
                })
                .collect()
        } else {
            vec![GeometrySlot {
                key: String::new(),
                geometry: BinaryBuilder::new(),
                properties: StringBuilder::new(),
            }]
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
            bbox_cols: Default::default(),
            bbox_nulls: NullBufferBuilder::new(0),
            per_lod,
            geometry_slots,
            material: StringBuilder::new(),
            texture: StringBuilder::new(),
            template_id: StringBuilder::new(),
            template_point: BinaryBuilder::new(),
            template_matrix: StringBuilder::new(),
            template_nulls: NullBufferBuilder::new(0),
            other: StringBuilder::new(),
            attributes,
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
                    Some(m) => self.template_matrix.append_value(m),
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

    fn push_string_list(builder: &mut ListBuilder<StringBuilder>, values: Option<&[String]>) {
        match values {
            Some(v) => builder.append_value(v.iter().map(|s| Some(s.as_str()))),
            None => builder.append_null(),
        }
    }

    /// Encode one CityObject row. `id` must be a key of `feature.city_objects`.
    fn push_object(
        &mut self,
        feature: &CityJSONFeature,
        id: &str,
        transform: &Transform,
        stats: &mut EncodeStats,
    ) -> Result<()> {
        let co = feature
            .city_objects
            .get(id)
            .expect("id came from this feature's own city_objects keys");
        let pool = VertexPool::new(&feature.vertices, transform);

        self.id.append_value(id);
        self.feature_id.append_value(&feature.id);
        self.object_type.append(&co.thetype)?;
        Self::push_string_list(&mut self.parents, co.parents.as_deref());
        Self::push_string_list(&mut self.children, co.children.as_deref());
        self.children_roles.append_null(); // M2 limitation: always null
        self.other.append_null(); // M2 limitation: always null

        let mut acc = GeometryAccumulator::default();
        accumulate_geometry(&mut acc, co, &pool, self.per_lod, stats)?;

        for slot in &mut self.geometry_slots {
            match acc.slots.get(&slot.key) {
                Some((bytes, _bbox, props)) => {
                    slot.geometry.append_value(bytes);
                    slot.properties.append_value(props);
                }
                None => {
                    slot.geometry.append_null();
                    slot.properties.append_null();
                }
            }
        }

        if acc.material.is_empty() {
            self.material.append_null();
        } else {
            self.material
                .append_value(serde_json::to_string(&Value::Object(acc.material))?);
        }
        if acc.texture.is_empty() {
            self.texture.append_null();
        } else {
            self.texture
                .append_value(serde_json::to_string(&Value::Object(acc.texture))?);
        }

        let bbox = resolve_bbox(acc.own_bbox, id, co, feature, &pool)?;
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
            self.finish_bbox(),
        ];
        for slot in &mut self.geometry_slots {
            arrays.push(Arc::new(slot.geometry.finish()));
            arrays.push(Arc::new(slot.properties.finish()));
        }
        arrays.push(Arc::new(self.material.finish()));
        arrays.push(Arc::new(self.texture.finish()));
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
pub struct BatchIter<'a> {
    features: FeatureIter<'a>,
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
}

impl BatchIter<'_> {
    /// Running totals of the row-population edge cases counted so far
    /// (final once the iterator is exhausted).
    pub fn stats(&self) -> EncodeStats {
        self.stats
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
            if let Err(e) = self
                .writer
                .push_object(feature, &id, &self.transform, &mut self.stats)
            {
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

/// Encode `source` into `RecordBatch`es matching `scan.schema.to_arrow_schema()`
/// exactly, `batch_size` rows per batch (the schema was already computed by
/// `scan`; this pass never re-infers it).
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
        features,
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
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// No fixture carries MultiSolid/CompositeSolid, so the nested
    /// `solid_shell_faces` branch gets direct coverage here: 2 solids —
    /// first with shells of (1, 2) faces, second with one 1-face shell.
    /// Nesting is the real 5-level MultiSolid shape:
    /// solids → shells → surfaces → rings → indices.
    #[test]
    fn multisolid_shell_faces_nest_per_solid() {
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
        let info = solid_shell_info(&geom, &[])
            .unwrap()
            .expect("MultiSolid info");
        assert_eq!(info.counts, vec![2, 1]);
        assert_eq!(info.faces, serde_json::json!([[1, 2], [1]]));

        let props: Value =
            serde_json::from_str(&geometry_properties_json(&geom, 0, &[]).unwrap()).unwrap();
        assert_eq!(props["solid_shell_counts"], serde_json::json!([2, 1]));
        assert_eq!(props["solid_shell_faces"], serde_json::json!([[1, 2], [1]]));

        // With writer-dropped flat face positions, the per-shell face counts
        // must describe the STORED geometry: positions 1 and 2 are the two
        // faces of the first solid's second shell; position 3 is the second
        // solid's only face.
        let info = solid_shell_info(&geom, &[1, 3])
            .unwrap()
            .expect("MultiSolid info");
        assert_eq!(info.faces, serde_json::json!([[1, 1], [0]]));
    }

    /// When the writer drops a degenerate surface, the encoder must remove
    /// the SAME index from the semantics values and every material/texture
    /// theme's per-surface array, record the drop in geometry_properties,
    /// and count it in EncodeStats — downstream sees aligned data only.
    #[test]
    fn dropped_surface_realigns_semantics_material_and_texture() {
        let co: CityObject = serde_json::from_value(serde_json::json!({
            "type": "Building",
            "geometry": [{
                "type": "MultiSurface",
                "lod": "2",
                // surface 0 is the [a,b,a] structural-degenerate shape
                "boundaries": [[[0, 1, 0]], [[0, 1, 2, 3]]],
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

        let mut acc = GeometryAccumulator::default();
        let mut stats = EncodeStats::default();
        accumulate_geometry(&mut acc, &co, &pool, true, &mut stats).unwrap();

        assert_eq!(stats.degenerate_rings_dropped, 1);
        assert_eq!(stats.degenerate_surfaces_dropped, 1);

        let (_, _, props) = acc.slots.get("lod2").expect("lod2 slot populated");
        let props: Value = serde_json::from_str(props).unwrap();
        assert_eq!(
            props["dropped_degenerate"],
            serde_json::json!({"rings": 1, "surfaces": [0]})
        );
        assert_eq!(
            props["semantics"]["values"],
            serde_json::json!([1]),
            "semantics values must lose the dropped surface's entry"
        );
        assert_eq!(
            props["semantics"]["surfaces"],
            serde_json::json!([{"type": "WallSurface"}, {"type": "RoofSurface"}]),
            "the surfaces lookup table itself is untouched (values index into it)"
        );

        assert_eq!(
            acc.material["2"],
            serde_json::json!({"visual": {"values": [7]}}),
            "material per-surface values must be realigned"
        );
        assert_eq!(
            acc.texture["2"],
            serde_json::json!({"visual": {"values": [[[1, 0, 1, 2, 3]]]}}),
            "texture per-surface values must be realigned"
        );
    }
}
