//! The typed `geometry_properties[_lod*]` `STRUCT` (spec "Geometry
//! properties and semantics"): shared builder/reader machinery for the main
//! object table (`crate::encode`/`crate::decode`) and the geometry-template
//! sidecar (`crate::sidecar`), so both write and read the SAME physical
//! shape rather than two divergent implementations.
//!
//! ```text
//! STRUCT<
//!   type            VARCHAR,          -- non-null
//!   surfaces        JSON,             -- nullable
//!   face_semantics  LIST<INT>,        -- nullable; items nullable
//!   shells          LIST<LIST<INT>>   -- nullable; where non-null, both
//!                                     -- nesting levels (inner LIST<INT>
//!                                     -- and each INT) are non-null
//! >
//! ```
//!
//! There is no `lod` field anywhere in this struct — the column name (main
//! table) or a sibling `lod` column (the template sidecar, which has no
//! per-LoD column name of its own) carries it instead.

use std::sync::Arc;

use arrow_array::builder::{Int32Builder, ListBuilder, StringBuilder};
use arrow_array::{Array, ArrayRef, Int32Array, ListArray, StringArray, StructArray};
use arrow_buffer::NullBufferBuilder;
use arrow_schema::{DataType, Field};
use serde_json::Value;

use cityparquet_schema::model::geometry_properties_data_type;
use cityparquet_schema::{CityParquetError, Result};

fn err(msg: impl Into<String>) -> CityParquetError {
    CityParquetError::Schema(msg.into())
}

/// One geometry's `geometry_properties` value, decoupled from Arrow.
///
/// `shells` is **always nested one inner list per solid** (spec: a `Solid`
/// still gets `[[12]]`, never the flat `[12]`) — the outer `Vec` indexes the
/// solid (exactly one entry for `Solid`, one per member for `MultiSolid`/
/// `CompositeSolid`), the inner `Vec<i32>` is that solid's per-shell face
/// count, outer shell first.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct GeometryProperties {
    pub type_name: String,
    pub surfaces: Option<Value>,
    pub face_semantics: Option<Vec<Option<i32>>>,
    pub shells: Option<Vec<Vec<i32>>>,
}

impl GeometryProperties {
    /// Reassembles the JSON shape earlier code in this crate (export,
    /// decode, the CityGML writer) already consumes: `{"type", "surfaces"?,
    /// "face_semantics"?, "shells"?}`, omitting a key entirely rather than
    /// storing it `null` — the exact convention the old JSON-text encoding
    /// used, so downstream `Value::get(...)` call sites need no change.
    pub(crate) fn to_value(&self) -> Value {
        let mut map = serde_json::Map::new();
        map.insert("type".to_string(), Value::String(self.type_name.clone()));
        if let Some(surfaces) = &self.surfaces {
            map.insert("surfaces".to_string(), surfaces.clone());
        }
        if let Some(face_semantics) = &self.face_semantics {
            let arr = face_semantics
                .iter()
                .map(|v| v.map_or(Value::Null, Value::from))
                .collect();
            map.insert("face_semantics".to_string(), Value::Array(arr));
        }
        if let Some(shells) = &self.shells {
            let arr = shells
                .iter()
                .map(|solid| Value::Array(solid.iter().map(|&n| Value::from(n)).collect()))
                .collect();
            map.insert("shells".to_string(), Value::Array(arr));
        }
        Value::Object(map)
    }

    /// The inverse of [`Self::to_value`]: parses the `{"type", "surfaces"?,
    /// "face_semantics"?, "shells"?}` JSON shape back into a typed
    /// [`GeometryProperties`]. Used only where a `geometry_properties` value
    /// legitimately travels as JSON between this typed form and the shared
    /// [`GeometryPropertiesBuilder`] — the geometry-template sidecar's
    /// `TemplateRow`, whose public field stays `Option<Value>` for
    /// call-site stability.
    pub(crate) fn try_from_value(v: &Value) -> Result<Self> {
        let obj = v
            .as_object()
            .ok_or_else(|| err("geometry_properties must be a JSON object"))?;
        let type_name = obj
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| err("geometry_properties missing a non-null 'type'"))?
            .to_string();
        let surfaces = obj.get("surfaces").cloned();
        let face_semantics = match obj.get("face_semantics") {
            None => None,
            Some(Value::Array(arr)) => Some(
                arr.iter()
                    .map(|v| match v {
                        Value::Null => Ok(None),
                        Value::Number(n) => n
                            .as_i64()
                            .and_then(|x| i32::try_from(x).ok())
                            .map(Some)
                            .ok_or_else(|| err(format!("face_semantics entry {n} is not an i32"))),
                        other => Err(err(format!(
                            "face_semantics entry must be an integer or null, got {other}"
                        ))),
                    })
                    .collect::<Result<Vec<_>>>()?,
            ),
            Some(other) => return Err(err(format!("face_semantics must be an array, got {other}"))),
        };
        let shells = match obj.get("shells") {
            None => None,
            Some(Value::Array(solids)) => Some(
                solids
                    .iter()
                    .map(|solid| {
                        let arr = solid
                            .as_array()
                            .ok_or_else(|| err(format!("shells entry must be an array, got {solid}")))?;
                        arr.iter()
                            .map(|n| {
                                n.as_i64()
                                    .and_then(|x| i32::try_from(x).ok())
                                    .ok_or_else(|| err(format!("shells count {n} is not an i32")))
                            })
                            .collect::<Result<Vec<i32>>>()
                    })
                    .collect::<Result<Vec<Vec<i32>>>>()?,
            ),
            Some(other) => return Err(err(format!("shells must be an array, got {other}"))),
        };
        Ok(Self {
            type_name,
            surfaces,
            face_semantics,
            shells,
        })
    }
}

/// Builder for one `geometry_properties[_lod*]` `STRUCT` column — the SAME
/// machinery the main table's per-LoD slots and the template sidecar's
/// single column both drive, so the two can never diverge in shape.
pub(crate) struct GeometryPropertiesBuilder {
    type_b: StringBuilder,
    surfaces_b: StringBuilder,
    face_semantics_b: ListBuilder<Int32Builder>,
    shells_b: ListBuilder<ListBuilder<Int32Builder>>,
    nulls: NullBufferBuilder,
}

/// The 4 child `Field`s of [`geometry_properties_data_type`], in
/// `type, surfaces, face_semantics, shells` order — extracted once so the
/// builder's own list/struct fields are constructed with EXACTLY the field
/// metadata (name, nullability) the schema declares, never a builder-default
/// that could silently drift from it.
fn child_fields() -> (Arc<Field>, Arc<Field>, Arc<Field>, Arc<Field>) {
    let DataType::Struct(fields) = geometry_properties_data_type() else {
        unreachable!("geometry_properties_data_type always returns Struct")
    };
    let mut it = fields.iter().cloned();
    let type_f = it.next().expect("type field");
    let surfaces_f = it.next().expect("surfaces field");
    let face_semantics_f = it.next().expect("face_semantics field");
    let shells_f = it.next().expect("shells field");
    (type_f, surfaces_f, face_semantics_f, shells_f)
}

impl GeometryPropertiesBuilder {
    pub(crate) fn new() -> Self {
        let (_, _, face_semantics_f, shells_f) = child_fields();

        let DataType::List(fs_item) = face_semantics_f.data_type().clone() else {
            unreachable!("face_semantics is always List")
        };
        let face_semantics_b = ListBuilder::new(Int32Builder::new()).with_field(fs_item);

        let DataType::List(solid_item) = shells_f.data_type().clone() else {
            unreachable!("shells is always List")
        };
        let DataType::List(count_item) = solid_item.data_type().clone() else {
            unreachable!("shells' items are always List")
        };
        let inner = ListBuilder::new(Int32Builder::new()).with_field(count_item);
        let shells_b = ListBuilder::new(inner).with_field(solid_item);

        Self {
            type_b: StringBuilder::new(),
            surfaces_b: StringBuilder::new(),
            face_semantics_b,
            shells_b,
            nulls: NullBufferBuilder::new(0),
        }
    }

    /// Appends one non-null `geometry_properties` cell.
    pub(crate) fn append_value(&mut self, props: &GeometryProperties) -> Result<()> {
        self.type_b.append_value(&props.type_name);
        match &props.surfaces {
            Some(v) => self.surfaces_b.append_value(serde_json::to_string(v)?),
            None => self.surfaces_b.append_null(),
        }
        match &props.face_semantics {
            Some(items) => {
                for item in items {
                    match item {
                        Some(v) => self.face_semantics_b.values().append_value(*v),
                        None => self.face_semantics_b.values().append_null(),
                    }
                }
                self.face_semantics_b.append(true);
            }
            None => self.face_semantics_b.append_null(),
        }
        match &props.shells {
            Some(solids) => {
                for shell_counts in solids {
                    let inner = self.shells_b.values();
                    for &n in shell_counts {
                        inner.values().append_value(n);
                    }
                    inner.append(true);
                }
                self.shells_b.append(true);
            }
            None => self.shells_b.append_null(),
        }
        self.nulls.append(true);
        Ok(())
    }

    /// Appends a whole-cell-null `geometry_properties` (no geometry to
    /// describe at this row/slot). The `type` child is declared non-nullable
    /// (spec), so — matching Arrow's convention that a struct's null bitmap
    /// alone marks the row absent, while children still need SOME physical
    /// value there — an empty string placeholder is appended rather than a
    /// null; it is never read back, since the row is null.
    pub(crate) fn append_null(&mut self) {
        self.type_b.append_value("");
        self.surfaces_b.append_null();
        self.face_semantics_b.append_null();
        self.shells_b.append_null();
        self.nulls.append(false);
    }

    pub(crate) fn finish(&mut self) -> ArrayRef {
        let DataType::Struct(fields) = geometry_properties_data_type() else {
            unreachable!("geometry_properties_data_type always returns Struct")
        };
        let arrays: Vec<ArrayRef> = vec![
            Arc::new(self.type_b.finish()),
            Arc::new(self.surfaces_b.finish()),
            Arc::new(self.face_semantics_b.finish()),
            Arc::new(self.shells_b.finish()),
        ];
        let nulls = self.nulls.finish();
        Arc::new(StructArray::new(fields, arrays, nulls))
    }
}

fn downcast<'a, T: 'static>(array: &'a dyn Array, name: &str) -> Result<&'a T> {
    array
        .as_any()
        .downcast_ref::<T>()
        .ok_or_else(|| err(format!("geometry_properties.{name} has an unexpected array type")))
}

/// Reads one `geometry_properties[_lod*]` cell at `row` of a decoded
/// `StructArray` back into the JSON shape earlier code in this crate already
/// consumes: `{"type", "surfaces"?, "face_semantics"?, "shells"?}` (a key is
/// omitted entirely, never stored `null`, exactly like the struct's own
/// per-field optionality). `None` when the whole cell is null.
pub(crate) fn read_geometry_properties(array: &StructArray, row: usize) -> Result<Option<Value>> {
    if array.is_null(row) {
        return Ok(None);
    }
    let type_col = downcast::<StringArray>(array.column(0).as_ref(), "type")?;
    let surfaces_col = downcast::<StringArray>(array.column(1).as_ref(), "surfaces")?;
    let face_semantics_col = downcast::<ListArray>(array.column(2).as_ref(), "face_semantics")?;
    let shells_col = downcast::<ListArray>(array.column(3).as_ref(), "shells")?;

    let mut map = serde_json::Map::new();
    map.insert(
        "type".to_string(),
        Value::String(type_col.value(row).to_string()),
    );
    if !surfaces_col.is_null(row) {
        map.insert(
            "surfaces".to_string(),
            serde_json::from_str(surfaces_col.value(row))?,
        );
    }
    if !face_semantics_col.is_null(row) {
        let items = face_semantics_col.value(row);
        let ints = downcast::<Int32Array>(items.as_ref(), "face_semantics.item")?;
        let arr: Vec<Value> = (0..ints.len())
            .map(|i| {
                if ints.is_null(i) {
                    Value::Null
                } else {
                    Value::from(ints.value(i))
                }
            })
            .collect();
        map.insert("face_semantics".to_string(), Value::Array(arr));
    }
    if !shells_col.is_null(row) {
        let solids = shells_col.value(row);
        let solids = downcast::<ListArray>(solids.as_ref(), "shells.item")?;
        let mut out = Vec::with_capacity(solids.len());
        for s in 0..solids.len() {
            let counts = solids.value(s);
            let counts = downcast::<Int32Array>(counts.as_ref(), "shells.item.item")?;
            let arr: Vec<Value> = (0..counts.len()).map(|i| Value::from(counts.value(i))).collect();
            out.push(Value::Array(arr));
        }
        map.insert("shells".to_string(), Value::Array(out));
    }
    Ok(Some(Value::Object(map)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(props: &GeometryProperties) -> Value {
        let mut b = GeometryPropertiesBuilder::new();
        b.append_value(props).unwrap();
        let array = b.finish();
        let struct_array = array.as_any().downcast_ref::<StructArray>().unwrap();
        read_geometry_properties(struct_array, 0).unwrap().unwrap()
    }

    #[test]
    fn type_only_round_trips_with_no_other_keys() {
        let props = GeometryProperties {
            type_name: "MultiSurface".to_string(),
            surfaces: None,
            face_semantics: None,
            shells: None,
        };
        let v = roundtrip(&props);
        assert_eq!(v, serde_json::json!({"type": "MultiSurface"}));
    }

    #[test]
    fn surfaces_and_face_semantics_round_trip() {
        let props = GeometryProperties {
            type_name: "MultiSurface".to_string(),
            surfaces: Some(serde_json::json!([{"type": "WallSurface"}])),
            face_semantics: Some(vec![Some(0), None]),
            shells: None,
        };
        let v = roundtrip(&props);
        assert_eq!(
            v,
            serde_json::json!({
                "type": "MultiSurface",
                "surfaces": [{"type": "WallSurface"}],
                "face_semantics": [0, null]
            })
        );
    }

    /// Spec's clarified edge case: `surfaces` non-null/non-empty while every
    /// `face_semantics` entry is null — a same-length all-null LIST, not a
    /// null `face_semantics` cell.
    #[test]
    fn all_null_face_semantics_list_is_distinct_from_a_null_cell() {
        let props = GeometryProperties {
            type_name: "MultiSurface".to_string(),
            surfaces: Some(serde_json::json!([{"type": "WallSurface"}])),
            face_semantics: Some(vec![None, None]),
            shells: None,
        };
        let v = roundtrip(&props);
        assert_eq!(v["face_semantics"], serde_json::json!([null, null]));
        assert!(v.get("face_semantics").is_some());
    }

    #[test]
    fn solid_shells_round_trip_nested_one_list_per_solid() {
        let props = GeometryProperties {
            type_name: "Solid".to_string(),
            surfaces: None,
            face_semantics: None,
            shells: Some(vec![vec![12]]),
        };
        let v = roundtrip(&props);
        assert_eq!(v["shells"], serde_json::json!([[12]]));
    }

    #[test]
    fn multisolid_shells_nest_one_list_per_solid() {
        let props = GeometryProperties {
            type_name: "MultiSolid".to_string(),
            surfaces: None,
            face_semantics: None,
            shells: Some(vec![vec![12, 4], vec![8, 4]]),
        };
        let v = roundtrip(&props);
        assert_eq!(v["shells"], serde_json::json!([[12, 4], [8, 4]]));
    }

    #[test]
    fn null_cell_round_trips_to_none() {
        let mut b = GeometryPropertiesBuilder::new();
        b.append_null();
        let array = b.finish();
        let struct_array = array.as_any().downcast_ref::<StructArray>().unwrap();
        assert_eq!(read_geometry_properties(struct_array, 0).unwrap(), None);
    }

    #[test]
    fn physical_field_shape_matches_spec() {
        let mut b = GeometryPropertiesBuilder::new();
        b.append_value(&GeometryProperties {
            type_name: "Solid".to_string(),
            surfaces: None,
            face_semantics: None,
            shells: Some(vec![vec![6]]),
        })
        .unwrap();
        let array = b.finish();
        let struct_array = array.as_any().downcast_ref::<StructArray>().unwrap();
        assert_eq!(struct_array.data_type(), &geometry_properties_data_type());
    }
}
