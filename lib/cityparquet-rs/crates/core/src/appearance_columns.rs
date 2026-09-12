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
    ArrayBuilder, Float64Builder, Int64Builder, ListBuilder, MapBuilder, MapFieldNames,
    StringBuilder, StructBuilder,
};
use arrow_array::{
    Array, ArrayRef, Float64Array, Int64Array, ListArray, MapArray, StringArray, StructArray,
};
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
pub struct MaterialCell {
    /// theme -> one sidecar id (or None) per WKB face; insertion order kept.
    pub themes: Vec<(String, Vec<Option<i64>>)>,
}

/// One ring's texture: the sidecar id and the ring's `[u, v]` pairs.
///
/// `id` and `uv` are null together — an untextured ring is `{null, null}`.
#[derive(Debug, Clone, PartialEq)]
pub struct TextureRing {
    pub id: Option<i64>,
    /// One [u, v] per distinct ring vertex; None exactly when `id` is None.
    pub uv: Option<Vec<[f64; 2]>>,
}

/// One `texture_lod*` cell, decoupled from Arrow.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TextureCell {
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
        validate_material(cell)?;
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
    /// Where `id` and `uv` sit in the ring struct — resolved by NAME once, so
    /// nothing here depends on `id` preceding `uv`.
    id_idx: usize,
    uv_idx: usize,
}

/// The `uv` builder's concrete type, spelled once for `field_builder`.
type UvBuilder = ListBuilder<ListBuilder<Float64Builder>>;

impl TextureCellBuilder {
    pub(crate) fn new() -> Self {
        let (names, key_f, value_f) = map_parts(&texture_data_type());
        let face_f = list_item(value_f.data_type());
        let ring_f = list_item(face_f.data_type());
        let fields = ring_fields(ring_f.data_type());
        let (id_idx, _) = fields.find("id").expect("a texture ring has an id field");
        let (uv_idx, _) = fields.find("uv").expect("a texture ring has a uv field");
        // One child builder per field, in the schema's own field order.
        let builders: Vec<Box<dyn ArrayBuilder>> = fields
            .iter()
            .map(|f| match f.name().as_str() {
                "id" => Box::new(Int64Builder::new()) as Box<dyn ArrayBuilder>,
                "uv" => {
                    let pair_f = list_item(f.data_type());
                    let coord_f = list_item(pair_f.data_type());
                    Box::new(
                        ListBuilder::new(
                            ListBuilder::new(Float64Builder::new()).with_field(coord_f),
                        )
                        .with_field(pair_f),
                    ) as Box<dyn ArrayBuilder>
                }
                other => unreachable!("a texture ring has no '{other}' field"),
            })
            .collect();
        let ring = StructBuilder::new(fields, builders);
        let faces = ListBuilder::new(ListBuilder::new(ring).with_field(ring_f)).with_field(face_f);
        let map = MapBuilder::new(Some(names), StringBuilder::new(), faces)
            .with_keys_field(key_f)
            .with_values_field(value_f);
        Self {
            map,
            id_idx,
            uv_idx,
        }
    }

    /// Appends one non-null cell, validated in full first (see
    /// [`MaterialCellBuilder::append_value`]).
    pub(crate) fn append_value(&mut self, cell: &TextureCell) -> Result<()> {
        validate_texture(cell)?;
        let (id_idx, uv_idx) = (self.id_idx, self.uv_idx);
        for (theme, faces) in &cell.themes {
            self.map.keys().append_value(theme);
            let face_list = self.map.values();
            for rings in faces {
                let ring_list = face_list.values();
                for ring in rings {
                    let ring_b = ring_list.values();
                    ring_b
                        .field_builder::<Int64Builder>(id_idx)
                        .expect("the id builder is an Int64Builder")
                        .append_option(ring.id);
                    let uv_b = ring_b
                        .field_builder::<UvBuilder>(uv_idx)
                        .expect("the uv builder is a list of lists of f64");
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

/// A map key names one theme, so the same key twice is not a representable
/// cell: Parquet's MAP semantics leave duplicate keys undefined, and a reader
/// would silently keep one of the two. The cells carry their themes as a
/// `Vec` to preserve insertion order, which admits the duplicate the type
/// system cannot rule out, so the builders refuse it here.
fn unique_themes<'a>(themes: impl Iterator<Item = &'a String>, column: &str) -> Result<()> {
    let mut seen = std::collections::HashSet::new();
    for theme in themes {
        if !seen.insert(theme) {
            return Err(err(format!(
                "{column} cell has theme '{theme}' more than once: map keys are unique"
            )));
        }
    }
    Ok(())
}

/// The invariants a material cell must satisfy before any of it is written:
/// at least one theme, each named once, each carrying at least one face.
fn validate_material(cell: &MaterialCell) -> Result<()> {
    if cell.themes.is_empty() {
        return Err(err(
            "material cell has no themes: write a null cell instead of an empty map",
        ));
    }
    unique_themes(cell.themes.iter().map(|(theme, _)| theme), "material")?;
    for (theme, ids) in &cell.themes {
        if ids.is_empty() {
            return Err(err(empty_theme("material", theme)));
        }
    }
    Ok(())
}

/// A theme carries one entry per WKB face and a geometry has at least one
/// face, so an empty list is not a representable theme — and neither writer
/// nor reader may pass one off as an all-null theme, which means something
/// else entirely.
fn empty_theme(column: &str, theme: &str) -> String {
    format!("{column} theme '{theme}' has no entries: a theme carries one entry per WKB face")
}

/// The invariants a texture cell must satisfy before any of it is written:
/// at least one theme, each named once and carrying at least one face, every
/// face at least its exterior ring, and within every ring `id` and `uv` null
/// together, a textured ring carrying at least one pair.
fn validate_texture(cell: &TextureCell) -> Result<()> {
    if cell.themes.is_empty() {
        return Err(err(
            "texture cell has no themes: write a null cell instead of an empty map",
        ));
    }
    unique_themes(cell.themes.iter().map(|(theme, _)| theme), "texture")?;
    for (theme, faces) in &cell.themes {
        if faces.is_empty() {
            return Err(err(empty_theme("texture", theme)));
        }
        for (f, rings) in faces.iter().enumerate() {
            if rings.is_empty() {
                return Err(err(no_rings(theme, f)));
            }
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
    pub fn to_flat_value(&self) -> Value {
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
    pub fn to_flat_value(&self) -> Value {
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
    entries: &'a StructArray,
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
pub fn read_material_cell(array: &MapArray, row: usize) -> Result<Option<MaterialCell>> {
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
        if ids.is_empty() {
            return Err(err(empty_theme("material", &theme)));
        }
        let per_face = (0..ids.len())
            .map(|f| (!ids.is_null(f)).then(|| ids.value(f)))
            .collect();
        themes.push((theme, per_face));
    }
    Ok(Some(MaterialCell { themes }))
}

/// Reads one `texture_lod*` cell at `row`. `None` when the cell is null.
pub fn read_texture_cell(array: &MapArray, row: usize) -> Result<Option<TextureCell>> {
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
        if faces.is_empty() {
            return Err(err(empty_theme("texture", &theme)));
        }
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

/// One `material_lod*` cell at `row` as the flat CityJSON-shaped JSON
/// [`MaterialCell::to_flat_value`] produces. `None` when the cell is null.
pub fn material_cell_value(array: &MapArray, row: usize) -> Result<Option<Value>> {
    Ok(read_material_cell(array, row)?.map(|c| c.to_flat_value()))
}

/// One `texture_lod*` cell at `row` as the flat CityJSON-shaped JSON
/// [`TextureCell::to_flat_value`] produces. `None` when the cell is null.
pub fn texture_cell_value(array: &MapArray, row: usize) -> Result<Option<Value>> {
    Ok(read_texture_cell(array, row)?.map(|c| c.to_flat_value()))
}

/// A WKB face always has an exterior ring, so a face with no rings is not a
/// representable face on either side of the column.
fn no_rings(theme: &str, face: usize) -> String {
    format!("texture theme '{theme}' face {face} has no rings")
}

/// One face's rings, from the `STRUCT<id, uv>` array holding them.
fn read_face(rings: &dyn Array, theme: &str, face: usize) -> Result<Vec<TextureRing>> {
    let rings = downcast::<StructArray>(rings, "texture ring")?;
    let ids = downcast::<Int64Array>(
        crate::arrow_compat::struct_child(rings, "id")?.as_ref(),
        "texture ring id",
    )?;
    let uvs = downcast::<ListArray>(
        crate::arrow_compat::struct_child(rings, "uv")?.as_ref(),
        "texture ring uv",
    )?;
    if rings.is_empty() {
        return Err(err(no_rings(theme, face)));
    }
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
        if uv.is_empty() {
            return Err(err(format!("{}: has an id but no [u, v] pair", where_())));
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
    use arrow_buffer::{NullBuffer, OffsetBuffer};
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
    fn a_duplicate_material_theme_is_refused() {
        let mut b = MaterialCellBuilder::new();
        let cell = MaterialCell {
            themes: vec![
                ("visual".into(), vec![Some(1)]),
                ("visual".into(), vec![Some(2)]),
            ],
        };
        let e = b.append_value(&cell).unwrap_err().to_string();
        assert!(e.contains("visual") && e.contains("more than once"), "{e}");
    }

    #[test]
    fn a_duplicate_texture_theme_is_refused() {
        let mut b = TextureCellBuilder::new();
        let cell = TextureCell {
            themes: vec![
                ("visual".into(), vec![vec![bare()]]),
                ("visual".into(), vec![vec![bare()]]),
            ],
        };
        let e = b.append_value(&cell).unwrap_err().to_string();
        assert!(e.contains("visual") && e.contains("more than once"), "{e}");
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

    /// A one-row texture `MapArray` built by hand, so a test can hand the
    /// reader a shape no builder would ever emit. Defaults are the valid
    /// shape: one theme, one face, one ring, one two-value pair.
    struct HandBuilt {
        /// `uv` before `id` in the ring struct — same names, swapped order.
        swapped: bool,
        coords_per_pair: i32,
        pairs_in_uv: i32,
        rings_in_face: i32,
        faces_in_theme: i32,
        /// The theme's first face entry is NULL.
        null_face: bool,
        /// The face's first ring struct is NULL.
        null_ring: bool,
        /// The ring's `id` is null while its `uv` is not — the two are
        /// null together or not at all.
        half_null_ring: bool,
    }

    impl Default for HandBuilt {
        fn default() -> Self {
            Self {
                swapped: false,
                coords_per_pair: 2,
                pairs_in_uv: 1,
                rings_in_face: 1,
                faces_in_theme: 1,
                null_face: false,
                null_ring: false,
                half_null_ring: false,
            }
        }
    }

    fn offsets(ends: Vec<i32>) -> OffsetBuffer<i32> {
        OffsetBuffer::new(ends.into())
    }

    impl HandBuilt {
        fn build(&self) -> MapArray {
            let n = (self.coords_per_pair * self.pairs_in_uv) as usize;
            let coords: ArrayRef = Arc::new(Float64Array::from(vec![0.5; n]));
            let pair_ends = (0..=self.pairs_in_uv)
                .map(|i| i * self.coords_per_pair)
                .collect();
            let pairs = ListArray::new(
                Arc::new(Field::new("item", DataType::Float64, false)),
                offsets(pair_ends),
                coords,
                None,
            );
            let pair_f = Arc::new(Field::new("item", pairs.data_type().clone(), false));
            let uv = ListArray::new(
                pair_f,
                offsets(vec![0, self.pairs_in_uv]),
                Arc::new(pairs),
                None,
            );
            let uv_f = Arc::new(Field::new("uv", uv.data_type().clone(), true));
            let uv: ArrayRef = Arc::new(uv);
            let id_f = Arc::new(Field::new("id", DataType::Int64, true));
            let id: ArrayRef = Arc::new(Int64Array::from(if self.half_null_ring {
                vec![None]
            } else {
                vec![Some(7)]
            }));
            let (fields, arrays) = if self.swapped {
                (vec![uv_f, id_f], vec![uv, id])
            } else {
                (vec![id_f, uv_f], vec![id, uv])
            };
            let ring_nulls = self.null_ring.then(|| NullBuffer::from(vec![false]));
            let ring = StructArray::new(Fields::from(fields), arrays, ring_nulls);
            let ring_f = Arc::new(Field::new("item", ring.data_type().clone(), self.null_ring));
            let face = ListArray::new(
                ring_f,
                offsets(vec![0, self.rings_in_face]),
                Arc::new(ring),
                self.null_face.then(|| NullBuffer::from(vec![false])),
            );
            let face_f = Arc::new(Field::new("item", face.data_type().clone(), self.null_face));
            let value = ListArray::new(
                face_f,
                offsets(vec![0, self.faces_in_theme]),
                Arc::new(face),
                None,
            );
            map_of_one_theme(Arc::new(value))
        }
    }

    /// The same map with its single row emptied — offsets `[0, 0]` over the
    /// same entries, so row 0 carries no theme at all.
    fn without_entries(m: &MapArray) -> MapArray {
        let DataType::Map(entries_f, sorted) = m.data_type().clone() else {
            unreachable!("a MapArray's data type is Map")
        };
        MapArray::new(
            entries_f,
            offsets(vec![0, 0]),
            m.entries().clone(),
            None,
            sorted,
        )
    }

    /// A one-row, one-entry map whose single theme's value is NULL — the
    /// column type forbids it (the map's value field is non-nullable), so
    /// only a foreign writer could produce it.
    fn map_of_null_theme_value() -> MapArray {
        let value: ArrayRef = Arc::new(ListArray::new(
            Arc::new(Field::new("item", DataType::Int64, true)),
            offsets(vec![0, 0]),
            Arc::new(Int64Array::from(Vec::<i64>::new())),
            Some(NullBuffer::from(vec![false])),
        ));
        let entries = StructArray::new(
            Fields::from(vec![
                Field::new("key", DataType::Utf8, false),
                Field::new("value", value.data_type().clone(), true),
            ]),
            vec![Arc::new(StringArray::from(vec![""])), value],
            None,
        );
        let entries_f = Arc::new(Field::new("key_value", entries.data_type().clone(), false));
        MapArray::new(entries_f, offsets(vec![0, 1]), entries, None, false)
    }

    /// A one-row, one-entry `MapArray` over `value`, with the empty theme.
    fn map_of_one_theme(value: ArrayRef) -> MapArray {
        let entries = StructArray::new(
            Fields::from(vec![
                Field::new("key", DataType::Utf8, false),
                Field::new("value", value.data_type().clone(), false),
            ]),
            vec![Arc::new(StringArray::from(vec![""])), value],
            None,
        );
        let entries_f = Arc::new(Field::new("key_value", entries.data_type().clone(), false));
        MapArray::new(entries_f, offsets(vec![0, 1]), entries, None, false)
    }

    /// A one-row material `MapArray` whose single theme holds `ids`.
    fn hand_built_material(ids: Vec<Option<i64>>) -> MapArray {
        let len = ids.len() as i32;
        let value = ListArray::new(
            Arc::new(Field::new("item", DataType::Int64, true)),
            offsets(vec![0, len]),
            Arc::new(Int64Array::from(ids)),
            None,
        );
        map_of_one_theme(Arc::new(value))
    }

    #[test]
    fn empty_texture_map_is_refused() {
        let mut b = TextureCellBuilder::new();
        assert!(b.append_value(&TextureCell::default()).is_err());
    }

    #[test]
    fn a_material_theme_with_no_entries_is_refused() {
        let mut b = MaterialCellBuilder::new();
        let cell = MaterialCell {
            themes: vec![("".into(), vec![])],
        };
        let e = b.append_value(&cell).unwrap_err().to_string();
        assert!(e.contains("no entries"), "{e}");
    }

    #[test]
    fn a_texture_theme_with_no_faces_is_refused() {
        let mut b = TextureCellBuilder::new();
        let cell = TextureCell {
            themes: vec![("".into(), vec![])],
        };
        let e = b.append_value(&cell).unwrap_err().to_string();
        assert!(e.contains("no entries"), "{e}");
    }

    #[test]
    fn a_face_with_no_rings_is_refused() {
        let mut b = TextureCellBuilder::new();
        let cell = TextureCell {
            themes: vec![("".into(), vec![vec![]])],
        };
        let e = b.append_value(&cell).unwrap_err().to_string();
        assert!(e.contains("no rings"), "{e}");
    }

    /// The readers refuse what the builders refuse: a theme with no entries,
    /// a face with no rings, and an `id` with no `[u, v]` pair — none of
    /// which a builder can produce, so each is handed over by hand.
    #[test]
    fn the_readers_refuse_what_the_builders_refuse() {
        let material = hand_built_material(vec![]);
        let e = read_material_cell(&material, 0).unwrap_err().to_string();
        assert!(e.contains("no entries"), "{e}");

        let no_faces = HandBuilt {
            faces_in_theme: 0,
            ..Default::default()
        }
        .build();
        let e = read_texture_cell(&no_faces, 0).unwrap_err().to_string();
        assert!(e.contains("no entries"), "{e}");

        let no_rings = HandBuilt {
            rings_in_face: 0,
            ..Default::default()
        }
        .build();
        let e = read_texture_cell(&no_rings, 0).unwrap_err().to_string();
        assert!(e.contains("no rings"), "{e}");

        let no_pairs = HandBuilt {
            pairs_in_uv: 0,
            ..Default::default()
        }
        .build();
        let e = read_texture_cell(&no_pairs, 0).unwrap_err().to_string();
        assert!(e.contains("no [u, v] pair"), "{e}");
    }

    /// The rest of the reader's refusals, on shapes no builder here can
    /// produce and the column type forbids, but a foreign writer of the same
    /// schema could still emit: an empty map, a null map value, a null face,
    /// a null ring, and a ring whose `id` and `uv` are not null together.
    /// Every one must be an `Err` — a panic fails this test.
    #[test]
    fn the_reader_refuses_every_shape_the_column_type_forbids() {
        let material = hand_built_material(vec![Some(3)]);
        let e = read_material_cell(&without_entries(&material), 0)
            .unwrap_err()
            .to_string();
        assert!(e.contains("empty map"), "{e}");

        let texture = HandBuilt::default().build();
        let e = read_texture_cell(&without_entries(&texture), 0)
            .unwrap_err()
            .to_string();
        assert!(e.contains("empty map"), "{e}");

        let null_value = map_of_null_theme_value();
        let e = read_material_cell(&null_value, 0).unwrap_err().to_string();
        assert!(e.contains("null value"), "{e}");
        let e = read_texture_cell(&null_value, 0).unwrap_err().to_string();
        assert!(e.contains("null value"), "{e}");

        let null_face = HandBuilt {
            null_face: true,
            ..Default::default()
        }
        .build();
        let e = read_texture_cell(&null_face, 0).unwrap_err().to_string();
        assert!(e.contains("face 0 is null"), "{e}");

        let null_ring = HandBuilt {
            null_ring: true,
            ..Default::default()
        }
        .build();
        let e = read_texture_cell(&null_ring, 0).unwrap_err().to_string();
        assert!(e.contains("ring 0 is null"), "{e}");

        let half_null = HandBuilt {
            half_null_ring: true,
            ..Default::default()
        }
        .build();
        let e = read_texture_cell(&half_null, 0).unwrap_err().to_string();
        assert!(e.contains("id and uv must be null together"), "{e}");
    }

    /// A foreign writer that narrowed the material ids to `INT32` is a type
    /// mismatch the reader must report, not downcast-unwrap into a panic.
    #[test]
    fn material_ids_of_the_wrong_child_type_are_an_error_not_a_panic() {
        let value = ListArray::new(
            Arc::new(Field::new("item", DataType::Int32, true)),
            offsets(vec![0, 2]),
            Arc::new(arrow_array::Int32Array::from(vec![3, 4])),
            None,
        );
        let map = map_of_one_theme(Arc::new(value));
        let e = read_material_cell(&map, 0).unwrap_err().to_string();
        assert!(e.contains("material id list"), "{e}");
    }

    /// The hazard `geometry_properties` documents for `type`/`surfaces`, one
    /// column over: a second writer of this shape may lay the ring struct out
    /// as `uv, id`, and the reader must resolve both by NAME.
    #[test]
    fn a_reordered_ring_struct_does_not_transpose_id_and_uv() {
        let map = HandBuilt {
            swapped: true,
            ..Default::default()
        }
        .build();
        let cell = read_texture_cell(&map, 0).unwrap().unwrap();
        assert_eq!(
            cell,
            TextureCell {
                themes: vec![("".into(), vec![vec![ring(7, &[[0.5, 0.5]])]])],
            }
        );
    }

    #[test]
    fn a_pair_with_three_coordinates_is_an_error_not_a_panic() {
        let map = HandBuilt {
            coords_per_pair: 3,
            ..Default::default()
        }
        .build();
        let e = read_texture_cell(&map, 0).unwrap_err().to_string();
        assert!(e.contains("exactly two non-null values"), "{e}");
    }

    /// Every other read here is of row 0; a cell at row 1 exercises the map's
    /// own offsets and the value list's, which a single-row array cannot.
    #[test]
    fn material_reads_a_non_null_cell_at_row_one() {
        let mut b = MaterialCellBuilder::new();
        let first = MaterialCell {
            themes: vec![("".into(), vec![Some(1), None])],
        };
        let second = MaterialCell {
            themes: vec![
                ("day".into(), vec![Some(2)]),
                ("night".into(), vec![Some(3), Some(4), None]),
            ],
        };
        b.append_value(&first).unwrap();
        b.append_value(&second).unwrap();
        let arr = b.finish();
        let map = arr.as_any().downcast_ref::<MapArray>().unwrap();
        assert_eq!(read_material_cell(map, 0).unwrap(), Some(first));
        assert_eq!(read_material_cell(map, 1).unwrap(), Some(second));
    }

    #[test]
    fn texture_reads_a_non_null_cell_at_row_one() {
        let mut b = TextureCellBuilder::new();
        let first = TextureCell {
            themes: vec![("".into(), vec![vec![bare()]])],
        };
        let second = TextureCell {
            themes: vec![
                (
                    "day".into(),
                    vec![
                        vec![ring(9, &[[0.25, 0.5], [0.75, 0.5]]), bare()],
                        vec![ring(9, &[[0.0, 1.0]])],
                    ],
                ),
                ("night".into(), vec![vec![bare()], vec![bare()]]),
            ],
        };
        b.append_value(&first).unwrap();
        b.append_value(&second).unwrap();
        let arr = b.finish();
        let map = arr.as_any().downcast_ref::<MapArray>().unwrap();
        assert_eq!(read_texture_cell(map, 0).unwrap(), Some(first));
        assert_eq!(read_texture_cell(map, 1).unwrap(), Some(second));
    }
}
