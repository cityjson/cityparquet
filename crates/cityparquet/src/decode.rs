//! Decode: `RecordBatch` rows back into `cjseq`-model objects. Inverse of
//! [`crate::encode`] at the row/attribute/geometry level — geometry is
//! decoded from WKB but deliberately kept OUT of the reassembled
//! `cjseq::CityObject` (that struct's `geometry` field expects CityJSON
//! boundary arrays, not decoded WKB); callers that need a CityJSON-shaped
//! `geometry` array own that re-encoding themselves (export, M4+).
//!
//! `cjseq::CityObject` cannot be constructed field-by-field (its `other`
//! field, `#[serde(flatten)]`, is private), so every object is assembled as a
//! JSON value and built via `serde_json::from_value` — the one supported
//! construction path.

use arrow_array::{
    Array, ArrayAccessor, ArrayRef, BinaryArray, BooleanArray, Date32Array, Float64Array,
    Int64Array, ListArray, RecordBatch, StringArray, StructArray, new_null_array,
};
use arrow_schema::extension::EXTENSION_TYPE_NAME_KEY;
use arrow_schema::{DataType, Field, Schema, TimeUnit};
use chrono::{SecondsFormat, TimeZone, Utc};
use serde_json::{Map, Value};

use cityparquet_schema::model::{address_data_type, template_data_type};
use cityparquet_schema::{CityMetadata, CityParquetError, GeometryEncoding, Lod, Result};

use crate::wkb_read::{self, DecodedGeometry};

/// Extension type name tagging a Utf8 column whose values are JSON text —
/// see [`cityparquet_schema::model`]'s `json_field` helper and
/// [`crate::reader`]'s identical use of this tag.
const ARROW_JSON_EXTENSION: &str = "arrow.json";

/// A resolved `template` struct column entry: which `GeometryInstance`
/// template an object references, at which point, with which (optional)
/// transformation matrix.
#[derive(Debug, Clone, PartialEq)]
pub struct TemplateInstance {
    /// The `geometry_templates.parquet` row this instance references, by
    /// `id` value — BIGINT, matching the sidecar's own `id` column.
    pub id: i64,
    pub point: [f64; 3],
    pub transformation_matrix: Option<Value>,
}

/// One `address` list item, decoded straight off the reserved struct column
/// (spec "Addresses"): the postal strings, plus `location` still as raw WKB
/// `MultiPointZ` bytes — resolving it into CityJSON `boundaries` needs a
/// feature-scoped vertex pool, which this module deliberately does not own
/// (see the module docs); that is `crate::export`'s job.
#[derive(Debug, Clone, PartialEq)]
pub struct AddressEntry {
    pub street: Option<String>,
    pub house_number: Option<String>,
    pub po_box: Option<String>,
    pub zip_code: Option<String>,
    pub city: Option<String>,
    pub state: Option<String>,
    pub country: Option<String>,
    pub free_text: Option<String>,
    pub location: Option<Vec<u8>>,
}

/// One decoded row: the reassembled `cjseq::CityObject` (attributes,
/// parents/children, `type`; geometry deliberately excluded, see the module
/// docs), its per-LoD geometries decoded from WKB alongside their
/// `geometry_properties`, its `template` reference if any, and its
/// `address` list if any.
#[derive(Debug, Clone)]
pub struct DecodedObject {
    pub id: String,
    pub feature_id: Option<String>,
    pub object: cjseq::CityObject,
    /// `(lod, decoded WKB geometry, geometry_properties)`, one entry per
    /// non-null geometry cell on this row, ascending by LoD. `None` LoD means a
    /// bare, un-suffixed `geometry` column — which the current writer never
    /// emits (a geometry-less table carries no geometry column); it appears
    /// only in a legacy/foreign file, read defensively here. In
    /// that dataset the column is all-null, so this variant does not arise in
    /// practice.
    pub geometries: Vec<(Option<Lod>, DecodedGeometry, Option<Value>)>,
    pub template: Option<TemplateInstance>,
    /// `None` when the row's `address` cell is null (no address at all);
    /// `Some(vec![])`/`Some(entries)` otherwise (spec "Addresses").
    pub address: Option<Vec<AddressEntry>>,
}

fn err(msg: impl Into<String>) -> CityParquetError {
    CityParquetError::Metadata(msg.into())
}

/// Merge the `other` column's unmapped members into the JSON that rebuilds a
/// CityObject (§5.1, G9): members with no dedicated column — a per-object
/// `geographicalExtent`, Extension `+members`. Typed fields route home; the
/// rest ride cjseq's private flatten and re-serialise on export. A
/// `None`/empty-`{}` cell contributes nothing.
///
/// Errors on a non-object cell or one carrying a reserved member. A well-formed
/// encoder strips the reserved keys (they have their own columns), so their
/// presence means a corrupt or foreign file — and on a losslessness-critical
/// path, silently dropping either side would mask that corruption.
fn merge_other_members(json: &mut Map<String, Value>, cell: Option<&str>, id: &str) -> Result<()> {
    let Some(cell) = cell else {
        return Ok(());
    };
    let Value::Object(members) = serde_json::from_str::<Value>(cell).map_err(|e| {
        err(format!(
            "object '{id}': 'other' column is not valid JSON: {e}"
        ))
    })?
    else {
        return Err(err(format!(
            "object '{id}': 'other' column must be a JSON object, got: {cell}"
        )));
    };
    for (key, value) in members {
        if crate::encode::OTHER_RESERVED_MEMBERS.contains(&key.as_str()) {
            return Err(err(format!(
                "object '{id}': 'other' column carries reserved member '{key}'"
            )));
        }
        json.insert(key, value);
    }
    Ok(())
}

/// Merge the `other_attributes` column's diverted entries back into the
/// object's `attributes` (spec "Column naming and reservation rules"; gap 14
/// — this used to ride inside `other` under a `cityparquet:diverted_attributes`
/// transport key, now it is its own reserved column keyed directly by the
/// diverted attribute's source name), creating `attributes` if the row had
/// none from its own columns. A `None`/absent cell contributes nothing.
/// Errors on a non-object cell, or a diverted name duplicating a decoded
/// column attribute — both mean a corrupt or foreign file, and silently
/// dropping either would mask it.
fn merge_other_attributes(
    json: &mut Map<String, Value>,
    cell: Option<&str>,
    id: &str,
) -> Result<()> {
    let Some(cell) = cell else {
        return Ok(());
    };
    let Value::Object(diverted) = serde_json::from_str::<Value>(cell).map_err(|e| {
        err(format!(
            "object '{id}': 'other_attributes' column is not valid JSON: {e}"
        ))
    })?
    else {
        return Err(err(format!(
            "object '{id}': 'other_attributes' column must be a JSON object, got: {cell}"
        )));
    };
    let attrs = json
        .entry("attributes")
        .or_insert_with(|| Value::Object(Map::new()));
    let Value::Object(attrs_map) = attrs else {
        return Err(err(format!(
            "object '{id}': 'attributes' is not a JSON object"
        )));
    };
    for (key, value) in diverted {
        if attrs_map.contains_key(&key) {
            return Err(err(format!(
                "object '{id}': diverted attribute '{key}' duplicates a column attribute"
            )));
        }
        attrs_map.insert(key, value);
    }
    Ok(())
}

fn get_column<'a>(batch: &'a RecordBatch, name: &str) -> Result<&'a arrow_array::ArrayRef> {
    batch
        .column_by_name(name)
        .ok_or_else(|| err(format!("record batch missing expected column '{name}'")))
}

/// `name`'s column, tolerant of the WHOLE column being absent from `batch`
/// (spec "Optional data is `NULL`" — a reader MUST tolerate an absent
/// `other_attributes`, and by the same information-equivalence any other
/// nullable reserved column a writer omits outright, e.g.
/// duckdb-cityjson's `address`/`template`/`children_roles`/
/// `other_attributes`): an absent column and an all-null one of `data_type`
/// carry identical information to every caller below, which already treats
/// a null cell as "no value". `id`/`feature_id`/`object_type` are non-null
/// per spec and stay on the strict [`get_column`] instead.
fn optional_column(batch: &RecordBatch, name: &str, data_type: &DataType) -> ArrayRef {
    match batch.column_by_name(name) {
        Some(col) => std::sync::Arc::clone(col),
        None => new_null_array(data_type, batch.num_rows()),
    }
}

/// The `List<Utf8>` shape `parents`/`children`/`children_roles` share (spec
/// "object-table-schema") — used only to synthesise an [`optional_column`]
/// fallback when one of them is absent; the item field's own name is
/// otherwise unobserved by any reader here.
fn string_list_type() -> DataType {
    DataType::List(std::sync::Arc::new(Field::new(
        "item",
        DataType::Utf8,
        true,
    )))
}

/// `Any`-based downcast with a `CityParquetError` instead of a panic on
/// mismatch — every column here is expected to have the shape
/// [`cityparquet_schema::model`] renders, but a hand-rolled or corrupted file
/// should surface as a decode error, not a panic.
fn downcast<'a, T: 'static>(array: &'a dyn Array, name: &str) -> Result<&'a T> {
    array
        .as_any()
        .downcast_ref::<T>()
        .ok_or_else(|| err(format!("column '{name}' has an unexpected array type")))
}

/// One physical geometry column set for one LoD (or the legacy `None`-LoD
/// case): its own [`GeometryEncoding`] — resolved once here from the file's
/// FOOTER declaration and checked against the physical column
/// ([`crate::geometry_encoding`]), never inferred per row — plus its
/// `geometry_properties_lod*` sibling and, named by the same convention but
/// only ever populated when `encoding == GeometryEncoding::ArrowNative`, its
/// `geometry_vertices_lod*` vertex-pool sibling.
struct GeometryColumnSpec {
    lod: Option<Lod>,
    geometry_name: String,
    properties_name: String,
    vertices_name: String,
    encoding: GeometryEncoding,
}

/// One [`GeometryColumnSpec`] per geometry column present in `schema`,
/// ascending by LoD. Mirrors `CityParquetReaderBuilder::cityparquet_arrow_schema`'s
/// LoD derivation: only `geometry_lod*` names parse as a LoD suffix —
/// `geometry_properties_lod*`/`geometry_vertices_lod*` also start with
/// `geometry_` but are excluded because `"properties_lod1"`/`"vertices_lod1"`
/// do not parse as one. A geometry-less table carries no geometry column at
/// all (the current writer prunes them; spec "Levels of detail"), so it
/// yields no entries here. A LEGACY/FOREIGN file may still carry the single
/// unsuffixed `geometry`/`geometry_properties` pair, read defensively as a
/// `None`-LoD entry; the two shapes are mutually exclusive by construction,
/// but both are checked unconditionally so a file carrying both would still
/// decode every geometry column.
fn geometry_columns(schema: &Schema, meta: &CityMetadata) -> Result<Vec<GeometryColumnSpec>> {
    let mut cols: Vec<(Option<Lod>, String, String, String)> = schema
        .fields()
        .iter()
        .filter_map(|f| {
            let name = f.name();
            let suffix = name.strip_prefix("geometry_")?;
            let lod = Lod::from_column_suffix(suffix)?;
            Some((
                Some(lod),
                name.clone(),
                format!("geometry_properties_{suffix}"),
                format!("geometry_vertices_{suffix}"),
            ))
        })
        .collect();
    cols.sort_by_key(|(lod, ..)| *lod);
    if schema.field_with_name("geometry").is_ok() {
        cols.push((
            None,
            "geometry".to_string(),
            "geometry_properties".to_string(),
            "geometry_vertices".to_string(),
        ));
    }
    cols.into_iter()
        .map(|(lod, geometry_name, properties_name, vertices_name)| {
            let encoding = crate::geometry_encoding::resolve_geometry_encoding(
                meta,
                schema,
                &geometry_name,
                &vertices_name,
            )?;
            Ok(GeometryColumnSpec {
                lod,
                geometry_name,
                properties_name,
                vertices_name,
                encoding,
            })
        })
        .collect()
}

/// One geometry column's decoded array handle: either the WKB `BinaryArray`,
/// or the arrow-native `geometry_lod*`/`geometry_vertices_lod*` `ListArray`
/// pair — resolved once per column (via [`GeometryColumnSpec::encoding`]),
/// never re-inferred per row.
enum GeometryColumnArrays<'a> {
    Wkb(&'a BinaryArray),
    ArrowNative {
        geometry: &'a ListArray,
        vertices: &'a ListArray,
    },
}

impl GeometryColumnArrays<'_> {
    fn is_null(&self, row: usize) -> bool {
        match self {
            Self::Wkb(geom) => geom.is_null(row),
            Self::ArrowNative { geometry, .. } => geometry.is_null(row),
        }
    }
}

/// A nullable `List<Utf8>` cell: `None` when the column is null at `row`,
/// else the (possibly empty) list of strings.
fn string_list_value(col: &ListArray, row: usize) -> Result<Option<Vec<String>>> {
    if col.is_null(row) {
        return Ok(None);
    }
    let values = col.value(row);
    let strs = downcast::<StringArray>(values.as_ref(), "list item")?;
    Ok(Some(
        (0..strs.len()).map(|i| strs.value(i).to_string()).collect(),
    ))
}

/// A nullable `Utf8` struct field named `field_name` at item `idx`: `None`
/// when either the struct item itself or the field within it is null.
/// Resolved by NAME — every `address` item field is `Utf8`, so an ordinal
/// lookup would silently swap two of them for a writer that orders the
/// struct's children differently (spec "Physical encoding and conformance").
fn struct_str_field(items: &StructArray, field_name: &str, idx: usize) -> Result<Option<String>> {
    if items.is_null(idx) {
        return Ok(None);
    }
    let arr = downcast::<StringArray>(
        crate::arrow_compat::struct_child(items, field_name)?.as_ref(),
        "address field",
    )?;
    Ok((!arr.is_null(idx)).then(|| arr.value(idx).to_string()))
}

/// The reserved `address` column at `row`: `None` when the cell itself is
/// null (no address at all — spec "Addresses"); `Some(entries)` otherwise,
/// one [`AddressEntry`] per list item, in order. `location` stays raw WKB —
/// resolving it into CityJSON `boundaries` needs a feature-scoped vertex
/// pool that this module deliberately does not own (see the module docs);
/// that is `crate::export`'s job.
fn decode_address_column(col: &ListArray, row: usize) -> Result<Option<Vec<AddressEntry>>> {
    if col.is_null(row) {
        return Ok(None);
    }
    let items = col.value(row);
    let items = downcast::<StructArray>(items.as_ref(), "address item")?;
    let mut out = Vec::with_capacity(items.len());
    for i in 0..items.len() {
        let location = if items.is_null(i) {
            None
        } else {
            let arr = downcast::<BinaryArray>(
                crate::arrow_compat::struct_child(items, "location")?.as_ref(),
                "address.location",
            )?;
            (!arr.is_null(i)).then(|| arr.value(i).to_vec())
        };
        out.push(AddressEntry {
            street: struct_str_field(items, "street", i)?,
            house_number: struct_str_field(items, "house_number", i)?,
            po_box: struct_str_field(items, "po_box", i)?,
            zip_code: struct_str_field(items, "zip_code", i)?,
            city: struct_str_field(items, "city", i)?,
            state: struct_str_field(items, "state", i)?,
            country: struct_str_field(items, "country", i)?,
            free_text: struct_str_field(items, "free_text", i)?,
            location,
        });
    }
    Ok(Some(out))
}

/// One attribute column's per-batch constants — name lookup, array, arrow
/// type, and the `arrow.json` extension tag — resolved ONCE by
/// [`decode_batch`] before its row loop. These used to be re-derived per
/// (row x attribute) (review P5e); the geometry columns at the top of
/// `decode_batch` already follow this hoisted pattern.
struct AttributeColumn<'a> {
    name: &'a str,
    array: &'a arrow_array::ArrayRef,
    data_type: &'a DataType,
    is_json: bool,
}

/// Resolve every `names` attribute column against `batch`/`schema` once, in
/// the order given, so the row loop only indexes.
fn resolve_attribute_columns<'a>(
    batch: &'a RecordBatch,
    schema: &'a Schema,
    names: &'a [String],
) -> Result<Vec<AttributeColumn<'a>>> {
    names
        .iter()
        .map(|name| {
            let field = schema.field_with_name(name).map_err(|_| {
                err(format!(
                    "attribute column '{name}' missing from batch schema"
                ))
            })?;
            let array = get_column(batch, name)?;
            let is_json = field
                .metadata()
                .get(EXTENSION_TYPE_NAME_KEY)
                .map(String::as_str)
                == Some(ARROW_JSON_EXTENSION);
            Ok(AttributeColumn {
                name: name.as_str(),
                array,
                data_type: field.data_type(),
                is_json,
            })
        })
        .collect()
}

/// One reconstructed attribute value at `row`, per the binding rules: `Date32`
/// -> `"%Y-%m-%d"` string; `Timestamp(ms, UTC)` -> RFC3339 `Z` string;
/// `List<Utf8>` -> JSON array; a Utf8 column tagged `arrow.json` -> the parsed
/// `Value`; `Boolean`/`Int64`/`Float64`/plain `Utf8` -> the matching JSON
/// scalar. `None` when the cell is null (nulls are omitted from the
/// attributes object entirely by the caller).
fn attribute_value(col: &AttributeColumn<'_>, row: usize) -> Result<Option<Value>> {
    let name = col.name;
    let array = col.array;
    if array.is_null(row) {
        return Ok(None);
    }

    let value = match col.data_type {
        DataType::Boolean => {
            let a = downcast::<BooleanArray>(array.as_ref(), name)?;
            Value::from(a.value(row))
        }
        DataType::Int64 => {
            let a = downcast::<Int64Array>(array.as_ref(), name)?;
            Value::from(a.value(row))
        }
        DataType::Float64 => {
            let a = downcast::<Float64Array>(array.as_ref(), name)?;
            Value::from(a.value(row))
        }
        DataType::Date32 => {
            let a = downcast::<Date32Array>(array.as_ref(), name)?;
            let date = a
                .value_as_date(row)
                .ok_or_else(|| err(format!("invalid Date32 value in column '{name}'")))?;
            Value::String(date.format("%Y-%m-%d").to_string())
        }
        DataType::Timestamp(unit @ (TimeUnit::Millisecond | TimeUnit::Microsecond), Some(tz))
            if tz.as_ref() == "UTC" =>
        {
            let naive = crate::arrow_compat::timestamp_utc_value(array.as_ref(), *unit, row, name)?
                .ok_or_else(|| err(format!("invalid Timestamp value in column '{name}'")))?;
            let dt = Utc.from_utc_datetime(&naive);
            Value::String(dt.to_rfc3339_opts(SecondsFormat::Millis, true))
        }
        DataType::Utf8 if col.is_json => {
            let a = downcast::<StringArray>(array.as_ref(), name)?;
            serde_json::from_str(a.value(row))?
        }
        DataType::Utf8 => {
            let a = downcast::<StringArray>(array.as_ref(), name)?;
            Value::String(a.value(row).to_string())
        }
        // Tolerant of a dictionary-encoded VARCHAR attribute column (spec
        // "Physical encoding and conformance"): `from_arrow` resolves it to
        // `String`, ambiguous with `Json` exactly like plain `Utf8` above —
        // the field's `arrow.json` tag (metadata, independent of the
        // physical shape) is what upgrades it.
        DataType::Dictionary(_, _) if col.is_json => {
            let view = crate::arrow_compat::string_view(array.as_ref(), name)?;
            serde_json::from_str(view.value(row))?
        }
        DataType::Dictionary(_, _) => {
            let view = crate::arrow_compat::string_view(array.as_ref(), name)?;
            Value::String(view.value(row).to_string())
        }
        DataType::List(item) if item.data_type() == &DataType::Utf8 => {
            let a = downcast::<ListArray>(array.as_ref(), name)?;
            let values = a.value(row);
            let strs = downcast::<StringArray>(values.as_ref(), name)?;
            Value::Array(
                (0..strs.len())
                    .map(|i| Value::String(strs.value(i).to_string()))
                    .collect(),
            )
        }
        other => {
            return Err(err(format!(
                "attribute column '{name}' has an arrow type decode cannot represent: {other:?}"
            )));
        }
    };
    Ok(Some(value))
}

/// Decode every row of `batch` into a [`DecodedObject`], reconstructing
/// attributes from `meta.attributes` against the batch's own (actual)
/// arrow types. See the module docs for what is and is not reassembled into
/// the returned `cjseq::CityObject`.
pub fn decode_batch(batch: &RecordBatch, meta: &CityMetadata) -> Result<Vec<DecodedObject>> {
    let schema = batch.schema();

    let id_col = downcast::<StringArray>(get_column(batch, "id")?.as_ref(), "id")?;
    let feature_id_col =
        downcast::<StringArray>(get_column(batch, "feature_id")?.as_ref(), "feature_id")?;

    let object_type_array = get_column(batch, "object_type")?;
    let object_type_view =
        crate::arrow_compat::string_view(object_type_array.as_ref(), "object_type")?;

    // Every column below is nullable per spec, so its WHOLE column may be
    // absent (duckdb-cityjson omits `children_roles`/`address`/`template`/
    // `other_attributes` outright) — `optional_column` synthesises an
    // all-null fallback of the right shape rather than erroring, per the
    // module docs on `optional_column`.
    let parents_array = optional_column(batch, "parents", &string_list_type());
    let parents_col = downcast::<ListArray>(parents_array.as_ref(), "parents")?;
    let children_array = optional_column(batch, "children", &string_list_type());
    let children_col = downcast::<ListArray>(children_array.as_ref(), "children")?;
    let children_roles_array = optional_column(batch, "children_roles", &string_list_type());
    let children_roles_col =
        downcast::<ListArray>(children_roles_array.as_ref(), "children_roles")?;
    let address_array = optional_column(batch, "address", &address_data_type());
    let address_col = downcast::<ListArray>(address_array.as_ref(), "address")?;
    let other_array = optional_column(batch, "other", &DataType::Utf8);
    let other_col = downcast::<StringArray>(other_array.as_ref(), "other")?;
    let other_attributes_array = optional_column(batch, "other_attributes", &DataType::Utf8);
    let other_attributes_col =
        downcast::<StringArray>(other_attributes_array.as_ref(), "other_attributes")?;

    let template_array = optional_column(batch, "template", &template_data_type());
    let template_col = downcast::<StructArray>(template_array.as_ref(), "template")?;
    let template_id_col = downcast::<Int64Array>(
        crate::arrow_compat::struct_child(template_col, "id")?.as_ref(),
        "template.id",
    )?;
    let template_point_col = downcast::<BinaryArray>(
        crate::arrow_compat::struct_child(template_col, "point")?.as_ref(),
        "template.point",
    )?;
    let template_matrix_col = downcast::<ListArray>(
        crate::arrow_compat::struct_child(template_col, "transformationMatrix")?.as_ref(),
        "template.transformationMatrix",
    )?;

    let geometry_cols = geometry_columns(&schema, meta)?;
    let geometry_arrays: Vec<(Option<Lod>, GeometryColumnArrays<'_>, &StructArray)> = geometry_cols
        .iter()
        .map(|col| {
            let props = downcast::<StructArray>(
                get_column(batch, &col.properties_name)?.as_ref(),
                &col.properties_name,
            )?;
            let arrays = match col.encoding {
                GeometryEncoding::Wkb => GeometryColumnArrays::Wkb(downcast::<BinaryArray>(
                    get_column(batch, &col.geometry_name)?.as_ref(),
                    &col.geometry_name,
                )?),
                GeometryEncoding::ArrowNative => {
                    let geometry = downcast::<ListArray>(
                        get_column(batch, &col.geometry_name)?.as_ref(),
                        &col.geometry_name,
                    )?;
                    let vertices = downcast::<ListArray>(
                        get_column(batch, &col.vertices_name)?.as_ref(),
                        &col.vertices_name,
                    )?;
                    GeometryColumnArrays::ArrowNative { geometry, vertices }
                }
            };
            Ok((col.lod, arrays, props))
        })
        .collect::<Result<_>>()?;

    let attribute_cols = resolve_attribute_columns(batch, &schema, &meta.attributes)?;

    let mut out = Vec::with_capacity(batch.num_rows());
    for row in 0..batch.num_rows() {
        let id = id_col.value(row).to_string();
        let feature_id = if feature_id_col.is_null(row) {
            None
        } else {
            Some(feature_id_col.value(row).to_string())
        };
        // `object_type` stores the CityGML 3.0 class name (spec
        // "object_table-schema" — "object_type vocabulary"); export must
        // restore the CityJSON spelling for the 4 classes that differ.
        // Every other core class, and every extension class (no taxonomy
        // entry), has an identical or unmapped spelling, so the reverse
        // lookup is a no-op for them.
        let stored_object_type = object_type_view.value(row);
        let object_type = cityparquet_schema::cityjson_type_for_citygml_class(stored_object_type)
            .map(str::to_string)
            .unwrap_or_else(|| stored_object_type.to_string());
        let parents = string_list_value(parents_col, row)?;
        let children = string_list_value(children_col, row)?;

        let mut attrs = Map::new();
        for col in &attribute_cols {
            if let Some(value) = attribute_value(col, row)? {
                attrs.insert(col.name.to_string(), value);
            }
        }

        let mut json = Map::new();
        json.insert("type".to_string(), Value::String(object_type));
        if !attrs.is_empty() {
            json.insert("attributes".to_string(), Value::Object(attrs));
        }
        if let Some(parents) = parents {
            json.insert(
                "parents".to_string(),
                Value::Array(parents.into_iter().map(Value::String).collect()),
            );
        }
        if let Some(children) = children {
            json.insert(
                "children".to_string(),
                Value::Array(children.into_iter().map(Value::String).collect()),
            );
        }
        // `children_roles` has no typed cjseq field; placing it in the JSON
        // that builds the CityObject lets it survive in the struct's private
        // `#[serde(flatten)]` member and re-serialise on export (§5.1, G5).
        if let Some(children_roles) = string_list_value(children_roles_col, row)? {
            json.insert(
                "children_roles".to_string(),
                Value::Array(children_roles.into_iter().map(Value::String).collect()),
            );
        }
        let other_cell = (!other_col.is_null(row)).then(|| other_col.value(row));
        merge_other_members(&mut json, other_cell, &id)?;
        let other_attributes_cell =
            (!other_attributes_col.is_null(row)).then(|| other_attributes_col.value(row));
        merge_other_attributes(&mut json, other_attributes_cell, &id)?;
        let object: cjseq::CityObject = serde_json::from_value(Value::Object(json))?;

        let address = decode_address_column(address_col, row)?;

        let mut geometries = Vec::with_capacity(geometry_arrays.len());
        for (lod, arrays, props_arr) in &geometry_arrays {
            if arrays.is_null(row) {
                continue;
            }
            let props = crate::geometry_properties::read_geometry_properties(props_arr, row)?;
            let decoded = match arrays {
                GeometryColumnArrays::Wkb(geom_arr) => {
                    wkb_read::wkb_to_geometry(geom_arr.value(row))?
                }
                GeometryColumnArrays::ArrowNative { geometry, vertices } => {
                    // `decode_row` dispatches on `geometry_properties.type`
                    // to know how to interpret/strip the physical shape's
                    // padding dimensions (design doc "Critical invariant" —
                    // never inferred from nesting depth), so that field must
                    // be present whenever the geometry cell itself is
                    // non-null (checked above) — its absence means a
                    // corrupt/hand-rolled file, an error rather than a panic.
                    let type_name = props
                        .as_ref()
                        .and_then(|p| p.get("type"))
                        .and_then(Value::as_str)
                        .ok_or_else(|| {
                            err(format!(
                                "object '{id}': arrow-native geometry has no \
                                 geometry_properties.type to dispatch decode on"
                            ))
                        })?;
                    crate::arrow_geom_read::decode_row(geometry, vertices, row, type_name)?
                }
            };
            // Every geometry column, including LoD0, is suffixed (spec
            // "Levels of detail") — `lod` already carries the geometry's LoD
            // straight from the column name. The only `None` case left is the
            // genuine zero-analysis-geometry fallback's un-suffixed column,
            // whose cell is always null (skipped above), so `lod` here is
            // never trusted as a footprint fallback.
            geometries.push((*lod, decoded, props));
        }

        let template = if template_col.is_null(row) {
            None
        } else {
            let point = wkb_read::read_point(template_point_col.value(row))?;
            let transformation_matrix = if template_matrix_col.is_null(row) {
                None
            } else {
                let values = template_matrix_col.value(row);
                let floats = downcast::<Float64Array>(
                    values.as_ref(),
                    "template.transformationMatrix item",
                )?;
                // spec "Appearance & templates": exactly 16 values when
                // non-null — a defensive check against a corrupt or foreign
                // file, mirroring the encoder's own write-time validator.
                if floats.len() != 16 {
                    return Err(err(format!(
                        "object '{id}': template.transformationMatrix has {} values, expected \
                         exactly 16",
                        floats.len()
                    )));
                }
                Some(serde_json::to_value(
                    (0..floats.len())
                        .map(|i| floats.value(i))
                        .collect::<Vec<f64>>(),
                )?)
            };
            Some(TemplateInstance {
                id: template_id_col.value(row),
                point,
                transformation_matrix,
            })
        };

        out.push(DecodedObject {
            id,
            feature_id,
            object,
            geometries,
            template,
            address,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use serde_json::json;

    /// `street` and `city` sit at ordinal positions 0 and 4 in the canonical
    /// `address` item struct — both nullable `Utf8`, exactly the shape a
    /// positionally-indexed reader would silently swap for a writer that
    /// emits the two fields in the other order. Built with `city` FIRST and
    /// `street` fifth to force that hazard.
    #[test]
    fn address_column_resolves_street_and_city_by_name_not_position() {
        let street_arr: ArrayRef = Arc::new(StringArray::from(vec!["Main St"]));
        let city_arr: ArrayRef = Arc::new(StringArray::from(vec!["Delft"]));
        let null_str = || -> ArrayRef { new_null_array(&DataType::Utf8, 1) };
        let null_bin: ArrayRef = new_null_array(&DataType::Binary, 1);

        let fields = vec![
            Field::new("city", DataType::Utf8, true),
            Field::new("house_number", DataType::Utf8, true),
            Field::new("po_box", DataType::Utf8, true),
            Field::new("zip_code", DataType::Utf8, true),
            Field::new("street", DataType::Utf8, true),
            Field::new("state", DataType::Utf8, true),
            Field::new("country", DataType::Utf8, true),
            Field::new("free_text", DataType::Utf8, true),
            Field::new("location", DataType::Binary, true),
        ];
        let arrays = vec![
            city_arr,
            null_str(),
            null_str(),
            null_str(),
            street_arr,
            null_str(),
            null_str(),
            null_str(),
            null_bin,
        ];
        let items = StructArray::new(arrow_schema::Fields::from(fields), arrays, None);

        let item_field: Arc<Field> = Field::new("item", items.data_type().clone(), true).into();
        let address_col = ListArray::new(
            item_field,
            arrow_buffer::OffsetBuffer::from_lengths([1usize]),
            Arc::new(items),
            None,
        );

        let entries = decode_address_column(&address_col, 0).unwrap().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].street.as_deref(),
            Some("Main St"),
            "street must resolve by NAME, not ordinal position 0"
        );
        assert_eq!(
            entries[0].city.as_deref(),
            Some("Delft"),
            "city must resolve by NAME, not ordinal position 4"
        );
    }

    // G9 decode guard: the `other`-column merge is a losslessness-critical,
    // corruption-sensitive path, so it is unit-tested directly (building a full
    // reserved-column RecordBatch just to reach it would be disproportionate).

    #[test]
    fn merge_other_members_injects_unmapped_members() {
        let mut json = Map::new();
        json.insert("type".to_string(), json!("Building"));
        merge_other_members(
            &mut json,
            Some(r#"{"unreserved_member":"value"}"#),
            "obj-1",
        )
        .unwrap();
        assert_eq!(json["unreserved_member"], json!("value"));
        assert_eq!(
            json["type"],
            json!("Building"),
            "must not disturb typed members"
        );
    }

    /// `address` has its own reserved column now (gap 10) — a well-formed
    /// `other` cell must never carry it.
    #[test]
    fn merge_other_members_rejects_address() {
        let mut json = Map::new();
        let err = merge_other_members(
            &mut json,
            Some(r#"{"address":[{"locality":"Helsinki"}]}"#),
            "obj-1",
        )
        .expect_err("address in `other` must be an error");
        assert!(
            format!("{err:?}").contains("reserved member 'address'"),
            "error must name the offending member, got: {err:?}"
        );
    }

    #[test]
    fn merge_other_members_null_and_empty_are_no_ops() {
        let mut json = Map::new();
        merge_other_members(&mut json, None, "obj-1").unwrap();
        assert!(json.is_empty(), "a null `other` cell contributes nothing");
        // A foreign writer may emit `{}` instead of null — identical effect.
        merge_other_members(&mut json, Some("{}"), "obj-1").unwrap();
        assert!(json.is_empty(), "an empty `{{}}` cell contributes nothing");
    }

    #[test]
    fn merge_other_members_rejects_a_reserved_member() {
        // A rogue `geometry` in `other` must NOT reach the typed field — decode's
        // assembled JSON never sets `geometry`, so a `contains_key` guard would
        // miss it; the static reserved-set guard catches it.
        let mut json = Map::new();
        let err = merge_other_members(&mut json, Some(r#"{"geometry":[]}"#), "obj-1")
            .expect_err("a reserved member in `other` must be an error");
        assert!(
            format!("{err:?}").contains("reserved member 'geometry'"),
            "error must name the offending member, got: {err:?}"
        );
    }

    #[test]
    fn merge_other_members_rejects_a_non_object_cell() {
        let mut json = Map::new();
        assert!(
            merge_other_members(&mut json, Some("[1,2,3]"), "obj-1").is_err(),
            "a non-object `other` cell must be an error, not a panic"
        );
        assert!(
            merge_other_members(&mut json, Some("not json"), "obj-1").is_err(),
            "a malformed `other` cell must be an error, not a panic"
        );
    }

    #[test]
    fn merge_other_attributes_restores_into_attributes() {
        // G12/gap 14: the `other_attributes` column's map merges into
        // `attributes` (creating it if the row had no column attributes),
        // never the top level.
        let mut json = Map::new();
        merge_other_attributes(&mut json, Some(r#"{"bbox":"x","id":42}"#), "o").unwrap();
        assert_eq!(json["attributes"], json!({"bbox": "x", "id": 42}));
        assert!(
            !json.contains_key("bbox"),
            "diverted attrs must not land at the top level"
        );
    }

    #[test]
    fn merge_other_attributes_null_is_a_no_op() {
        let mut json = Map::new();
        merge_other_attributes(&mut json, None, "o").unwrap();
        assert!(json.is_empty());
    }

    #[test]
    fn merge_other_attributes_guards() {
        // Non-object cell → error.
        let mut json = Map::new();
        assert!(
            merge_other_attributes(&mut json, Some("\"nope\""), "o").is_err(),
            "a non-object other_attributes cell must error"
        );
        // A diverted name duplicating a decoded column attribute → error.
        let mut json = Map::new();
        json.insert("attributes".to_string(), json!({"bbox": "from-column"}));
        assert!(
            merge_other_attributes(&mut json, Some(r#"{"bbox":"x"}"#), "o").is_err(),
            "a diverted attr colliding with a column attr must error"
        );
    }

    #[test]
    fn merge_other_members_rejects_geographical_extent() {
        // `geographicalExtent` is a reserved member carried by `bbox`, not by
        // `other`. A well-formed encoder never places it there; if found, it is
        // a corrupt or foreign file.
        let mut json = Map::new();
        let err = merge_other_members(&mut json, Some(r#"{"geographicalExtent":[0]}"#), "o")
            .expect_err("geographicalExtent in `other` must be an error");
        assert!(
            format!("{err:?}").contains("reserved member 'geographicalExtent'"),
            "error must name the offending member, got: {err:?}"
        );
    }
}
