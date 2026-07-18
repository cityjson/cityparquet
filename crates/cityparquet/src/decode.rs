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

use arrow_array::types::Int32Type;
use arrow_array::{
    Array, ArrayAccessor, BinaryArray, BooleanArray, Date32Array, DictionaryArray, Float64Array,
    Int64Array, ListArray, RecordBatch, StringArray, StructArray, TimestampMillisecondArray,
};
use arrow_schema::extension::EXTENSION_TYPE_NAME_KEY;
use arrow_schema::{DataType, Schema, TimeUnit};
use chrono::{SecondsFormat, TimeZone, Utc};
use serde_json::{Map, Value};

use cityparquet_schema::{CityParquetError, CityParquetMetadata, Lod, Result};

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
    pub id: String,
    pub point: [f64; 3],
    pub transformation_matrix: Option<Value>,
}

/// One decoded row: the reassembled `cjseq::CityObject` (attributes,
/// parents/children, `type`; geometry deliberately excluded, see the module
/// docs), its per-LoD geometries decoded from WKB alongside their
/// `geometry_properties`, and its `template` reference if any.
#[derive(Debug, Clone)]
pub struct DecodedObject {
    pub id: String,
    pub feature_id: Option<String>,
    pub object: cjseq::CityObject,
    /// `(lod, decoded WKB geometry, geometry_properties)`, one entry per
    /// non-null geometry cell on this row, ascending by LoD. `None` LoD means
    /// the dataset's single unsuffixed `geometry` column — the
    /// zero-analysis-geometry fallback (a dataset with only GeometryInstances,
    /// or none; see [`cityparquet_schema::model`]'s lods-empty branch). In
    /// that dataset the column is all-null, so this variant does not arise in
    /// practice.
    pub geometries: Vec<(Option<Lod>, DecodedGeometry, Option<Value>)>,
    pub template: Option<TemplateInstance>,
}

fn err(msg: impl Into<String>) -> CityParquetError {
    CityParquetError::Metadata(msg.into())
}

/// Merge the `other` column's unmapped members into the JSON that rebuilds a
/// CityObject (§5.1, G9): members with no dedicated column — a Building's
/// `address`, a per-object `geographicalExtent`, Extension `+members`. Typed
/// fields route home; the rest ride cjseq's private flatten and re-serialise on
/// export. A `None`/empty-`{}` cell contributes nothing.
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
        // Attributes diverted here because their name collides with a column
        // (§5.2, G12) are merged back into `attributes`, not the top level.
        if key == crate::encode::DIVERTED_ATTRS_KEY {
            merge_diverted_attributes(json, value, id)?;
            continue;
        }
        if crate::encode::OTHER_RESERVED_MEMBERS.contains(&key.as_str()) {
            return Err(err(format!(
                "object '{id}': 'other' column carries reserved member '{key}'"
            )));
        }
        // `geographicalExtent` routes into cjseq's typed `Vec<f64>`, so a
        // corrupt cell (wrong length, null, non-numbers) would decode and then
        // export as invalid CityJSON. CityJSON 2.0.1 §2 fixes it at exactly six
        // numbers.
        if key == "geographicalExtent" && !is_geographical_extent(&value) {
            return Err(err(format!(
                "object '{id}': 'other' geographicalExtent must be an array of exactly six \
                 numbers, got: {value}"
            )));
        }
        json.insert(key, value);
    }
    Ok(())
}

/// Merge diverted attributes (the `other` cell's `cityparquet:diverted_attributes`
/// value, §5.2/G12) back into the object's `attributes`, creating that object if
/// the row had no column attributes. Errors if the value is not an object, or if
/// a diverted name duplicates a decoded column attribute — both mean a corrupt
/// or foreign file, and silently dropping either would mask it.
fn merge_diverted_attributes(json: &mut Map<String, Value>, value: Value, id: &str) -> Result<()> {
    let Value::Object(diverted) = value else {
        return Err(err(format!(
            "object '{id}': '{}' must be a JSON object",
            crate::encode::DIVERTED_ATTRS_KEY
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

/// A CityJSON `geographicalExtent`: exactly six finite numbers
/// `[minx, miny, minz, maxx, maxy, maxz]` (CityJSON 2.0.1 §2).
fn is_geographical_extent(value: &Value) -> bool {
    value
        .as_array()
        .is_some_and(|a| a.len() == 6 && a.iter().all(|n| n.as_f64().is_some_and(f64::is_finite)))
}

fn get_column<'a>(batch: &'a RecordBatch, name: &str) -> Result<&'a arrow_array::ArrayRef> {
    batch
        .column_by_name(name)
        .ok_or_else(|| err(format!("record batch missing expected column '{name}'")))
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

/// `(lod, geometry column name, geometry_properties column name)` triples
/// present in `schema`, ascending by LoD. Mirrors
/// `CityParquetReaderBuilder::cityparquet_arrow_schema`'s LoD derivation:
/// only `geometry_lod*` names parse as a LoD suffix — `geometry_properties_lod*`
/// also starts with `geometry_` but is excluded because `"properties_lod1"`
/// does not parse as one. A zero-analysis-geometry dataset instead has the
/// single unsuffixed `geometry`/`geometry_properties` pair ([`cityparquet_schema::model`]'s
/// lods-empty branch), returned as a `None`-LoD entry; the two shapes are
/// mutually exclusive by construction, but both are checked unconditionally
/// so a file carrying both would still decode every geometry column.
fn geometry_columns(schema: &Schema) -> Vec<(Option<Lod>, String, String)> {
    let mut cols: Vec<(Option<Lod>, String, String)> = schema
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
            ))
        })
        .collect();
    cols.sort_by_key(|(lod, _, _)| *lod);
    if schema.field_with_name("geometry").is_ok() {
        cols.push((
            None,
            "geometry".to_string(),
            "geometry_properties".to_string(),
        ));
    }
    cols
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

/// One reconstructed attribute value at `row`, per the binding rules: `Date32`
/// -> `"%Y-%m-%d"` string; `Timestamp(ms, UTC)` -> RFC3339 `Z` string;
/// `List<Utf8>` -> JSON array; a Utf8 column tagged `arrow.json` -> the parsed
/// `Value`; `Boolean`/`Int64`/`Float64`/plain `Utf8` -> the matching JSON
/// scalar. `None` when the cell is null (nulls are omitted from the
/// attributes object entirely by the caller).
fn attribute_value(
    batch: &RecordBatch,
    schema: &Schema,
    name: &str,
    row: usize,
) -> Result<Option<Value>> {
    let field = schema.field_with_name(name).map_err(|_| {
        err(format!(
            "attribute column '{name}' missing from batch schema"
        ))
    })?;
    let array = get_column(batch, name)?;
    if array.is_null(row) {
        return Ok(None);
    }
    let is_json = field
        .metadata()
        .get(EXTENSION_TYPE_NAME_KEY)
        .map(String::as_str)
        == Some(ARROW_JSON_EXTENSION);

    let value = match field.data_type() {
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
        DataType::Timestamp(TimeUnit::Millisecond, Some(tz)) if tz.as_ref() == "UTC" => {
            let a = downcast::<TimestampMillisecondArray>(array.as_ref(), name)?;
            let naive = a
                .value_as_datetime(row)
                .ok_or_else(|| err(format!("invalid Timestamp value in column '{name}'")))?;
            let dt = Utc.from_utc_datetime(&naive);
            Value::String(dt.to_rfc3339_opts(SecondsFormat::Millis, true))
        }
        DataType::Utf8 if is_json => {
            let a = downcast::<StringArray>(array.as_ref(), name)?;
            serde_json::from_str(a.value(row))?
        }
        DataType::Utf8 => {
            let a = downcast::<StringArray>(array.as_ref(), name)?;
            Value::String(a.value(row).to_string())
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
/// attributes from `meta.attribute_columns` against the batch's own (actual)
/// arrow types. See the module docs for what is and is not reassembled into
/// the returned `cjseq::CityObject`.
pub fn decode_batch(batch: &RecordBatch, meta: &CityParquetMetadata) -> Result<Vec<DecodedObject>> {
    let schema = batch.schema();

    let id_col = downcast::<StringArray>(get_column(batch, "id")?.as_ref(), "id")?;
    let feature_id_col =
        downcast::<StringArray>(get_column(batch, "feature_id")?.as_ref(), "feature_id")?;

    let object_type_array = get_column(batch, "object_type")?;
    let object_type_dict =
        downcast::<DictionaryArray<Int32Type>>(object_type_array.as_ref(), "object_type")?;
    let object_type_values = object_type_dict
        .downcast_dict::<StringArray>()
        .ok_or_else(|| err("'object_type' dictionary values are not Utf8"))?;

    let parents_col = downcast::<ListArray>(get_column(batch, "parents")?.as_ref(), "parents")?;
    let children_col = downcast::<ListArray>(get_column(batch, "children")?.as_ref(), "children")?;
    let children_roles_col = downcast::<ListArray>(
        get_column(batch, "children_roles")?.as_ref(),
        "children_roles",
    )?;
    let other_col = downcast::<StringArray>(get_column(batch, "other")?.as_ref(), "other")?;

    let template_col =
        downcast::<StructArray>(get_column(batch, "template")?.as_ref(), "template")?;
    let template_id_col = downcast::<StringArray>(template_col.column(0).as_ref(), "template.id")?;
    let template_point_col =
        downcast::<BinaryArray>(template_col.column(1).as_ref(), "template.point")?;
    let template_matrix_col = downcast::<StringArray>(
        template_col.column(2).as_ref(),
        "template.transformationMatrix",
    )?;

    let geometry_cols = geometry_columns(&schema);
    let geometry_arrays: Vec<(Option<Lod>, &BinaryArray, &StringArray)> = geometry_cols
        .iter()
        .map(|(lod, geom_name, props_name)| {
            let geom = downcast::<BinaryArray>(get_column(batch, geom_name)?.as_ref(), geom_name)?;
            let props =
                downcast::<StringArray>(get_column(batch, props_name)?.as_ref(), props_name)?;
            Ok((*lod, geom, props))
        })
        .collect::<Result<_>>()?;

    let mut out = Vec::with_capacity(batch.num_rows());
    for row in 0..batch.num_rows() {
        let id = id_col.value(row).to_string();
        let feature_id = if feature_id_col.is_null(row) {
            None
        } else {
            Some(feature_id_col.value(row).to_string())
        };
        let object_type = object_type_values.value(row).to_string();
        let parents = string_list_value(parents_col, row)?;
        let children = string_list_value(children_col, row)?;

        let mut attrs = Map::new();
        for name in &meta.attribute_columns {
            if let Some(value) = attribute_value(batch, &schema, name, row)? {
                attrs.insert(name.clone(), value);
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
        let object: cjseq::CityObject = serde_json::from_value(Value::Object(json))?;

        let mut geometries = Vec::with_capacity(geometry_arrays.len());
        for (lod, geom_arr, props_arr) in &geometry_arrays {
            if geom_arr.is_null(row) {
                continue;
            }
            let decoded = wkb_read::wkb_to_geometry(geom_arr.value(row))?;
            let props = if props_arr.is_null(row) {
                None
            } else {
                Some(serde_json::from_str(props_arr.value(row))?)
            };
            geometries.push((*lod, decoded, props));
        }

        let template = if template_col.is_null(row) {
            None
        } else {
            let point = wkb_read::read_point(template_point_col.value(row))?;
            let transformation_matrix = if template_matrix_col.is_null(row) {
                None
            } else {
                Some(serde_json::from_str(template_matrix_col.value(row))?)
            };
            Some(TemplateInstance {
                id: template_id_col.value(row).to_string(),
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
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // G9 decode guard: the `other`-column merge is a losslessness-critical,
    // corruption-sensitive path, so it is unit-tested directly (building a full
    // reserved-column RecordBatch just to reach it would be disproportionate).

    #[test]
    fn merge_other_members_injects_unmapped_members() {
        let mut json = Map::new();
        json.insert("type".to_string(), json!("Building"));
        merge_other_members(
            &mut json,
            Some(r#"{"address":[{"Locality":"Helsinki"}],"geographicalExtent":[0,0,0,1,1,1]}"#),
            "obj-1",
        )
        .unwrap();
        assert_eq!(json["address"], json!([{"Locality": "Helsinki"}]));
        assert_eq!(json["geographicalExtent"], json!([0, 0, 0, 1, 1, 1]));
        assert_eq!(
            json["type"],
            json!("Building"),
            "must not disturb typed members"
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
    fn merge_diverted_attributes_restores_into_attributes() {
        // G12: the diverted map merges into `attributes` (creating it if the
        // row had no column attributes), never the top level.
        let mut json = Map::new();
        merge_other_members(
            &mut json,
            Some(r#"{"cityparquet:diverted_attributes":{"bbox":"x","id":42}}"#),
            "o",
        )
        .unwrap();
        assert_eq!(json["attributes"], json!({"bbox": "x", "id": 42}));
        assert!(
            !json.contains_key("bbox"),
            "diverted attrs must not land at the top level"
        );
    }

    #[test]
    fn merge_diverted_attributes_guards() {
        // Non-object value → error.
        let mut json = Map::new();
        assert!(
            merge_other_members(
                &mut json,
                Some(r#"{"cityparquet:diverted_attributes":"nope"}"#),
                "o"
            )
            .is_err(),
            "a non-object diverted value must error"
        );
        // A diverted name duplicating a decoded column attribute → error.
        let mut json = Map::new();
        json.insert("attributes".to_string(), json!({"bbox": "from-column"}));
        assert!(
            merge_other_members(
                &mut json,
                Some(r#"{"cityparquet:diverted_attributes":{"bbox":"x"}}"#),
                "o"
            )
            .is_err(),
            "a diverted attr colliding with a column attr must error"
        );
    }

    #[test]
    fn merge_other_members_validates_geographical_extent_shape() {
        // G9 sol-review Finding 3: a corrupt `geographicalExtent` in `other`
        // routes into cjseq's typed `Vec<f64>` and would export as invalid
        // CityJSON, so it must be rejected before merging.
        let mut json = Map::new();
        assert!(
            merge_other_members(&mut json, Some(r#"{"geographicalExtent":[0]}"#), "o").is_err(),
            "fewer than six numbers must be rejected"
        );
        assert!(
            merge_other_members(&mut json, Some(r#"{"geographicalExtent":null}"#), "o").is_err(),
            "a null extent must be rejected, not silently dropped"
        );
        // A valid six-number extent passes.
        let mut json = Map::new();
        merge_other_members(
            &mut json,
            Some(r#"{"geographicalExtent":[0,0,0,1,1,1]}"#),
            "o",
        )
        .unwrap();
        assert_eq!(json["geographicalExtent"], json!([0, 0, 0, 1, 1, 1]));
    }
}
