//! The typed `material_lod*` / `texture_lod*` MAP columns (spec "material /
//! texture columns"): the Arrow-decoupled cell values, the builders the main
//! object table and the geometry-template sidecar both write with, and the
//! readers every consumer reads with — one physical shape, one
//! implementation.
//!
//! ```text
//! material  MAP<VARCHAR, LIST<BIGINT>>
//!           -- theme -> one sidecar id (or null) per WKB face
//! texture   MAP<VARCHAR, LIST<LIST<STRUCT<id BIGINT, uv LIST<LIST<DOUBLE>>>>>>
//!           -- theme -> per WKB face -> per ring -> {sidecar id, one [u, v]
//!           --   per distinct ring vertex}
//! ```
//!
//! A cell is flat per WKB face, in WKB face order, keyed by theme — there is
//! no geometry-type-specific nesting to walk. A consumer that needs
//! CityJSON's per-shell nesting re-nests from the same geometry's
//! `geometry_properties_lod*.shells`.
//!
//! The builders' list, struct and map fields are all derived from the schema
//! crate's [`material_data_type`]/[`texture_data_type`], so a builder can
//! never drift from the column type it fills.

use std::sync::Arc;

use arrow_array::builder::{
    Float64Builder, Int64Builder, ListBuilder, MapBuilder, MapFieldNames, StringBuilder,
    StructBuilder,
};
use arrow_array::{Array, ArrayRef, Float64Array, Int64Array, ListArray, MapArray, StringArray};
use arrow_schema::{DataType, Field, Fields};
use serde_json::Value;

use cityparquet_schema::model::{material_data_type, texture_data_type};
use cityparquet_schema::{CityParquetError, Result};

fn err(msg: impl Into<String>) -> CityParquetError {
    CityParquetError::Schema(msg.into())
}

/// One `material_lod*` cell, decoupled from Arrow.
///
/// Insertion order of `themes` is the map's physical entry order; CityJSON's
/// unnamed theme is the empty string. A cell with no themes has no
/// representation — the column is nullable and a geometry with no material in
/// any theme is written as a null cell.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct MaterialCell {
    /// theme -> one sidecar id (or None) per WKB face; insertion order kept.
    pub themes: Vec<(String, Vec<Option<i64>>)>,
}

/// One ring's texture: the sidecar id and the ring's `[u, v]` pairs.
///
/// `id` and `uv` are null together — an untextured ring is `{null, null}`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TextureRing {
    pub id: Option<i64>,
    /// One [u, v] per distinct ring vertex; None exactly when `id` is None.
    pub uv: Option<Vec<[f64; 2]>>,
}

/// One `texture_lod*` cell, decoupled from Arrow.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct TextureCell {
    /// theme -> per WKB face -> per ring.
    pub themes: Vec<(String, Vec<Vec<TextureRing>>)>,
}

/// The `key_value`/`key`/`value` names and the key and value `Field`s of a
/// map type, so a builder is constructed with EXACTLY the field metadata
/// (name, nullability) the schema declares.
fn map_parts(map: &DataType) -> (MapFieldNames, Arc<Field>, Arc<Field>) {
    let DataType::Map(entries, _) = map else {
        unreachable!("appearance columns are always Map")
    };
    let DataType::Struct(kv) = entries.data_type() else {
        unreachable!("map entries are always Struct")
    };
    let names = MapFieldNames {
        entry: entries.name().clone(),
        key: kv[0].name().clone(),
        value: kv[1].name().clone(),
    };
    (names, Arc::clone(&kv[0]), Arc::clone(&kv[1]))
}

/// A list type's item `Field`.
fn list_item(list: &DataType) -> Arc<Field> {
    let DataType::List(item) = list else {
        unreachable!("expected a List")
    };
    Arc::clone(item)
}

/// The `id`/`uv` `Field`s of the texture ring struct.
fn ring_fields(ring: &DataType) -> Fields {
    let DataType::Struct(fields) = ring else {
        unreachable!("a texture ring is always a Struct")
    };
    fields.clone()
}

/// Builder for one `material_lod*` column — the SAME machinery the object
/// table's per-LoD slots and the template sidecar drive.
pub(crate) struct MaterialCellBuilder {
    map: MapBuilder<StringBuilder, ListBuilder<Int64Builder>>,
}

impl MaterialCellBuilder {
    pub(crate) fn new() -> Self {
        let (names, key_f, value_f) = map_parts(&material_data_type());
        let ids = ListBuilder::new(Int64Builder::new()).with_field(list_item(value_f.data_type()));
        let map = MapBuilder::new(Some(names), StringBuilder::new(), ids)
            .with_keys_field(key_f)
            .with_values_field(value_f);
        Self { map }
    }

    /// Appends one non-null cell. The cell is validated in full before any
    /// child builder is touched, so a rejected cell leaves the builder's
    /// keys, values and offsets exactly as they were.
    pub(crate) fn append_value(&mut self, cell: &MaterialCell) -> Result<()> {
        if cell.themes.is_empty() {
            return Err(err(
                "material cell has no themes: write a null cell instead of an empty map",
            ));
        }
        for (theme, ids) in &cell.themes {
            self.map.keys().append_value(theme);
            let list = self.map.values();
            for id in ids {
                list.values().append_option(*id);
            }
            list.append(true);
        }
        self.map
            .append(true)
            .map_err(|e| err(format!("material map: {e}")))
    }

    pub(crate) fn append_null(&mut self) {
        self.map
            .append(false)
            .expect("keys and values stay aligned");
    }

    pub(crate) fn finish(&mut self) -> ArrayRef {
        Arc::new(self.map.finish())
    }
}

/// Builder for one `texture_lod*` column, the material builder's shape one
/// level deeper: theme -> face -> ring struct.
pub(crate) struct TextureCellBuilder {
    map: MapBuilder<StringBuilder, ListBuilder<ListBuilder<StructBuilder>>>,
}

/// The `uv` builder's concrete type, spelled once for `field_builder`.
type UvBuilder = ListBuilder<ListBuilder<Float64Builder>>;

impl TextureCellBuilder {
    pub(crate) fn new() -> Self {
        let (names, key_f, value_f) = map_parts(&texture_data_type());
        let face_f = list_item(value_f.data_type());
        let ring_f = list_item(face_f.data_type());
        let fields = ring_fields(ring_f.data_type());
        let uv_f = Arc::clone(&fields[1]);
        let pair_f = list_item(uv_f.data_type());
        let coord_f = list_item(pair_f.data_type());

        let uv = ListBuilder::new(ListBuilder::new(Float64Builder::new()).with_field(coord_f))
            .with_field(pair_f);
        let ring = StructBuilder::new(fields, vec![Box::new(Int64Builder::new()), Box::new(uv)]);
        let faces = ListBuilder::new(ListBuilder::new(ring).with_field(ring_f)).with_field(face_f);
        let map = MapBuilder::new(Some(names), StringBuilder::new(), faces)
            .with_keys_field(key_f)
            .with_values_field(value_f);
        Self { map }
    }

    /// Appends one non-null cell, validated in full first (see
    /// [`MaterialCellBuilder::append_value`]).
    pub(crate) fn append_value(&mut self, cell: &TextureCell) -> Result<()> {
        validate_texture(cell)?;
        for (theme, faces) in &cell.themes {
            self.map.keys().append_value(theme);
            let face_list = self.map.values();
            for rings in faces {
                let ring_list = face_list.values();
                for ring in rings {
                    let ring_b = ring_list.values();
                    ring_b
                        .field_builder::<Int64Builder>(0)
                        .expect("ring field 0 is the id")
                        .append_option(ring.id);
                    let uv_b = ring_b
                        .field_builder::<UvBuilder>(1)
                        .expect("ring field 1 is the uv list");
                    match &ring.uv {
                        Some(pairs) => {
                            for [u, v] in pairs {
                                let pair = uv_b.values();
                                pair.values().append_value(*u);
                                pair.values().append_value(*v);
                                pair.append(true);
                            }
                            uv_b.append(true);
                        }
                        None => uv_b.append_null(),
                    }
                    ring_b.append(true);
                }
                ring_list.append(true);
            }
            face_list.append(true);
        }
        self.map
            .append(true)
            .map_err(|e| err(format!("texture map: {e}")))
    }

    pub(crate) fn append_null(&mut self) {
        self.map
            .append(false)
            .expect("keys and values stay aligned");
    }

    pub(crate) fn finish(&mut self) -> ArrayRef {
        Arc::new(self.map.finish())
    }
}

/// The invariants a texture cell must satisfy before any of it is written:
/// at least one theme, and within every ring `id` and `uv` null together,
/// a textured ring carrying at least one pair.
fn validate_texture(cell: &TextureCell) -> Result<()> {
    if cell.themes.is_empty() {
        return Err(err(
            "texture cell has no themes: write a null cell instead of an empty map",
        ));
    }
    for (theme, faces) in &cell.themes {
        for (f, rings) in faces.iter().enumerate() {
            for (r, ring) in rings.iter().enumerate() {
                match (&ring.id, &ring.uv) {
                    (Some(_), Some(uv)) if uv.is_empty() => {
                        return Err(err(format!(
                            "texture theme '{theme}' face {f} ring {r} has an id but no [u, v] pair"
                        )));
                    }
                    (Some(_), Some(_)) | (None, None) => {}
                    _ => {
                        return Err(err(format!(
                            "texture theme '{theme}' face {f} ring {r}: id and uv must be null together"
                        )));
                    }
                }
            }
        }
    }
    Ok(())
}

impl MaterialCell {
    /// `{"<theme>": {"values": [id|null, …]}}` — the flat CityJSON-shaped map
    /// every reader re-nests from `shells`.
    pub(crate) fn to_flat_value(&self) -> Value {
        let mut obj = serde_json::Map::new();
        for (theme, ids) in &self.themes {
            let values: Vec<Value> = ids
                .iter()
                .map(|v| v.map_or(Value::Null, Value::from))
                .collect();
            let mut entry = serde_json::Map::new();
            entry.insert("values".to_string(), Value::Array(values));
            obj.insert(theme.clone(), Value::Object(entry));
        }
        Value::Object(obj)
    }
}

impl TextureRing {
    /// `[id, [u, v], …]`, or `[null]` for an untextured ring — CityJSON's
    /// ring form with the UV indices replaced inline by the pairs.
    fn to_flat_value(&self) -> Value {
        match (&self.id, &self.uv) {
            (Some(id), Some(uv)) => {
                let mut ring = Vec::with_capacity(uv.len() + 1);
                ring.push(Value::from(*id));
                ring.extend(
                    uv.iter()
                        .map(|[u, v]| Value::Array(vec![Value::from(*u), Value::from(*v)])),
                );
                Value::Array(ring)
            }
            _ => Value::Array(vec![Value::Null]),
        }
    }
}

impl TextureCell {
    /// `{"<theme>": {"values": [ [ [id, [u,v], …] | [null], … ], … ]}}` — per
    /// face, per ring, the inlined ring form.
    pub(crate) fn to_flat_value(&self) -> Value {
        let mut obj = serde_json::Map::new();
        for (theme, faces) in &self.themes {
            let values: Vec<Value> = faces
                .iter()
                .map(|rings| Value::Array(rings.iter().map(TextureRing::to_flat_value).collect()))
                .collect();
            let mut entry = serde_json::Map::new();
            entry.insert("values".to_string(), Value::Array(values));
            obj.insert(theme.clone(), Value::Object(entry));
        }
        Value::Object(obj)
    }
}

fn downcast<'a, T: 'static>(array: &'a dyn Array, name: &str) -> Result<&'a T> {
    array
        .as_any()
        .downcast_ref::<T>()
        .ok_or_else(|| err(format!("{name} has an unexpected array type")))
}

/// The `key` and `value` columns of one map row's entries. Their order is
/// fixed by Parquet's MAP group, unlike a struct's children, which resolve by
/// name.
fn map_entries<'a>(
    entries: &'a arrow_array::StructArray,
    column: &str,
) -> Result<(&'a StringArray, &'a ArrayRef)> {
    if entries.num_columns() != 2 {
        return Err(err(format!(
            "{column} map entries must have exactly a key and a value column"
        )));
    }
    let keys = downcast::<StringArray>(entries.column(0).as_ref(), &format!("{column} map key"))?;
    Ok((keys, entries.column(1)))
}

/// Reads one `material_lod*` cell at `row`. `None` when the cell is null.
pub(crate) fn read_material_cell(array: &MapArray, row: usize) -> Result<Option<MaterialCell>> {
    if array.is_null(row) {
        return Ok(None);
    }
    let entries = array.value(row);
    let (keys, values) = map_entries(&entries, "material")?;
    let values = downcast::<ListArray>(values.as_ref(), "material map value")?;
    if entries.is_empty() {
        return Err(err(
            "material cell is an empty map: a non-null cell carries at least one theme",
        ));
    }
    let mut themes = Vec::with_capacity(entries.len());
    for i in 0..entries.len() {
        if keys.is_null(i) {
            return Err(err("material theme key is null"));
        }
        let theme = keys.value(i).to_string();
        if values.is_null(i) {
            return Err(err(format!("material theme '{theme}' has a null value")));
        }
        let ids = values.value(i);
        let ids = downcast::<Int64Array>(ids.as_ref(), "material id list")?;
        let per_face = (0..ids.len())
            .map(|f| (!ids.is_null(f)).then(|| ids.value(f)))
            .collect();
        themes.push((theme, per_face));
    }
    Ok(Some(MaterialCell { themes }))
}

/// Reads one `texture_lod*` cell at `row`. `None` when the cell is null.
pub(crate) fn read_texture_cell(array: &MapArray, row: usize) -> Result<Option<TextureCell>> {
    if array.is_null(row) {
        return Ok(None);
    }
    let entries = array.value(row);
    let (keys, values) = map_entries(&entries, "texture")?;
    let values = downcast::<ListArray>(values.as_ref(), "texture map value")?;
    if entries.is_empty() {
        return Err(err(
            "texture cell is an empty map: a non-null cell carries at least one theme",
        ));
    }
    let mut themes = Vec::with_capacity(entries.len());
    for i in 0..entries.len() {
        if keys.is_null(i) {
            return Err(err("texture theme key is null"));
        }
        let theme = keys.value(i).to_string();
        if values.is_null(i) {
            return Err(err(format!("texture theme '{theme}' has a null value")));
        }
        let faces = values.value(i);
        let faces = downcast::<ListArray>(faces.as_ref(), "texture face list")?;
        let mut per_face = Vec::with_capacity(faces.len());
        for f in 0..faces.len() {
            if faces.is_null(f) {
                return Err(err(format!("texture theme '{theme}' face {f} is null")));
            }
            per_face.push(read_face(faces.value(f).as_ref(), &theme, f)?);
        }
        themes.push((theme, per_face));
    }
    Ok(Some(TextureCell { themes }))
}

/// One face's rings, from the `STRUCT<id, uv>` array holding them.
fn read_face(rings: &dyn Array, theme: &str, face: usize) -> Result<Vec<TextureRing>> {
    let rings = downcast::<arrow_array::StructArray>(rings, "texture ring")?;
    let ids = downcast::<Int64Array>(
        crate::arrow_compat::struct_child(rings, "id")?.as_ref(),
        "texture ring id",
    )?;
    let uvs = downcast::<ListArray>(
        crate::arrow_compat::struct_child(rings, "uv")?.as_ref(),
        "texture ring uv",
    )?;
    let mut out = Vec::with_capacity(rings.len());
    for r in 0..rings.len() {
        if rings.is_null(r) {
            return Err(err(format!(
                "texture theme '{theme}' face {face} ring {r} is null"
            )));
        }
        let where_ = || format!("texture theme '{theme}' face {face} ring {r}");
        if ids.is_null(r) != uvs.is_null(r) {
            return Err(err(format!(
                "{}: id and uv must be null together",
                where_()
            )));
        }
        if ids.is_null(r) {
            out.push(TextureRing { id: None, uv: None });
            continue;
        }
        let pairs = uvs.value(r);
        let pairs = downcast::<ListArray>(pairs.as_ref(), "texture uv list")?;
        let mut uv = Vec::with_capacity(pairs.len());
        for p in 0..pairs.len() {
            if pairs.is_null(p) {
                return Err(err(format!("{}: [u, v] pair {p} is null", where_())));
            }
            let coords = pairs.value(p);
            let coords = downcast::<Float64Array>(coords.as_ref(), "texture uv pair")?;
            if coords.len() != 2 || coords.null_count() > 0 {
                return Err(err(format!(
                    "{}: [u, v] pair {p} must hold exactly two non-null values",
                    where_()
                )));
            }
            uv.push([coords.value(0), coords.value(1)]);
        }
        out.push(TextureRing {
            id: Some(ids.value(r)),
            uv: Some(uv),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cityparquet_schema::model::{material_data_type, texture_data_type};

    fn ring(id: i64, uv: &[[f64; 2]]) -> TextureRing {
        TextureRing {
            id: Some(id),
            uv: Some(uv.to_vec()),
        }
    }
    fn bare() -> TextureRing {
        TextureRing { id: None, uv: None }
    }

    #[test]
    fn material_builder_matches_schema_type_and_round_trips() {
        let mut b = MaterialCellBuilder::new();
        let cell = MaterialCell {
            themes: vec![
                ("".into(), vec![Some(3), None, Some(3)]),
                ("night".into(), vec![None, None, None]),
            ],
        };
        b.append_value(&cell).unwrap();
        b.append_null();
        let arr = b.finish();
        assert_eq!(arr.data_type(), &material_data_type());
        let map = arr.as_any().downcast_ref::<MapArray>().unwrap();
        assert_eq!(read_material_cell(map, 0).unwrap(), Some(cell));
        assert_eq!(read_material_cell(map, 1).unwrap(), None);
    }

    #[test]
    fn empty_material_map_is_refused() {
        let mut b = MaterialCellBuilder::new();
        assert!(b.append_value(&MaterialCell::default()).is_err());
    }

    #[test]
    fn texture_builder_matches_schema_type_and_round_trips() {
        let mut b = TextureCellBuilder::new();
        let cell = TextureCell {
            themes: vec![(
                "".into(),
                vec![
                    // face 0: textured exterior, bare hole
                    vec![ring(7, &[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]]), bare()],
                    // face 1: untextured
                    vec![bare()],
                ],
            )],
        };
        b.append_value(&cell).unwrap();
        b.append_null();
        let arr = b.finish();
        assert_eq!(arr.data_type(), &texture_data_type());
        let map = arr.as_any().downcast_ref::<MapArray>().unwrap();
        assert_eq!(read_texture_cell(map, 0).unwrap(), Some(cell));
        assert_eq!(read_texture_cell(map, 1).unwrap(), None);
    }

    #[test]
    fn half_null_ring_is_refused() {
        let mut b = TextureCellBuilder::new();
        let cell = TextureCell {
            themes: vec![(
                "".into(),
                vec![vec![TextureRing {
                    id: Some(1),
                    uv: None,
                }]],
            )],
        };
        assert!(b.append_value(&cell).is_err());
    }

    #[test]
    fn flat_values_take_the_cityjson_shapes_readers_expect() {
        let m = MaterialCell {
            themes: vec![("".into(), vec![Some(3), None])],
        };
        assert_eq!(
            m.to_flat_value(),
            serde_json::json!({"": {"values": [3, null]}})
        );
        let t = TextureCell {
            themes: vec![(
                "".into(),
                vec![
                    vec![ring(7, &[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]]), bare()],
                    vec![bare()],
                ],
            )],
        };
        assert_eq!(
            t.to_flat_value(),
            serde_json::json!({"": {"values": [ [ [7, [0.0, 0.0], [1.0, 0.0], [1.0, 1.0]], [null] ], [ [null] ] ]}})
        );
    }
}
