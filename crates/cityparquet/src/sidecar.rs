//! Compatibility-profile sidecar tables: `materials.parquet` and
//! `textures.parquet`. Each row is one dataset-global appearance definition
//! (see [`crate::appearance::AppearanceInterner`]), `id` is its row index,
//! and the well-known CityJSON members are split into typed columns per
//! [`cityparquet_schema::profile::materials_schema`] /
//! [`cityparquet_schema::profile::textures_schema`]; anything else on the
//! definition is preserved verbatim under the `other` JSON column.
//!
//! `write_materials`/`write_textures` and `read_materials`/`read_textures`
//! are VALUE-exact inverses of each other (value-equality up to JSON member
//! order): every field the writer peels off into its own column, the reader
//! puts back under the same key; every leftover field the writer stashes in
//! `other`, the reader merges back in. One deliberate normalisation: the
//! numeric scalar columns (`ambientIntensity`/`transparency`/`shininess`)
//! round-trip through Arrow `Float64`, so a definition written with an
//! integer-literal JSON number (e.g. `"shininess": 1`) reads back in float
//! form (`"shininess": 1.0`) — same value, different literal, and NOT `==`
//! under `serde_json::Value`'s `PartialEq`. All other members (including
//! anything routed through a JSON column or `other`) round-trip literally.
//! The dataset comparator ([`crate::compare`]) already treats mixed
//! int/float JSON numbers as equal within tolerance, so this normalisation
//! can never break round-trip equality at the dataset level.

use std::fs::File;
use std::path::Path;
use std::sync::Arc;

use arrow_array::builder::{
    BinaryBuilder, BooleanBuilder, Float64Builder, Int64Builder, StringBuilder,
};
use arrow_array::{
    Array, ArrayRef, BinaryArray, BooleanArray, Float64Array, Int64Array, RecordBatch, StringArray,
};
use arrow_schema::Schema;
use parquet::arrow::ArrowWriter;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::basic::{Compression, ZstdLevel};
use parquet::file::properties::WriterProperties;
use serde_json::Value;

use cityparquet_schema::{CityParquetError, Result, profile};

fn schema_err(msg: impl Into<String>) -> CityParquetError {
    CityParquetError::Schema(msg.into())
}

fn io_err(msg: impl Into<String>) -> CityParquetError {
    CityParquetError::Io(msg.into())
}

fn parquet_err(msg: impl Into<String>) -> CityParquetError {
    CityParquetError::Parquet(msg.into())
}

/// zstd level 3, no per-column tuning — sidecars are small, dictionary-poor
/// tables (a few dozen to a few thousand appearance definitions), unlike the
/// main table's [`crate::recipe::WriterRecipe`] which is the paper's actual
/// benchmark variable.
fn sidecar_writer_properties() -> Result<WriterProperties> {
    let level = ZstdLevel::try_new(3)
        .map_err(|e| schema_err(format!("invalid sidecar zstd level: {e}")))?;
    Ok(WriterProperties::builder()
        .set_compression(Compression::ZSTD(level))
        .build())
}

fn write_batch(path: &Path, schema: Arc<Schema>, batch: RecordBatch) -> Result<()> {
    let file =
        File::create(path).map_err(|e| io_err(format!("cannot create {}: {e}", path.display())))?;
    let props = sidecar_writer_properties()?;
    let mut writer = ArrowWriter::try_new(file, schema, Some(props))
        .map_err(|e| parquet_err(format!("cannot open parquet writer: {e}")))?;
    writer
        .write(&batch)
        .map_err(|e| parquet_err(format!("parquet write error: {e}")))?;
    writer
        .close()
        .map_err(|e| parquet_err(format!("cannot finalise parquet file: {e}")))?;
    Ok(())
}

fn push_opt_str(b: &mut StringBuilder, v: Option<&Value>, field: &str) -> Result<()> {
    match v {
        None | Some(Value::Null) => b.append_null(),
        Some(Value::String(s)) => b.append_value(s),
        Some(other) => {
            return Err(schema_err(format!(
                "'{field}' must be a string, got {other}"
            )));
        }
    }
    Ok(())
}

fn push_opt_f64(b: &mut Float64Builder, v: Option<&Value>, field: &str) -> Result<()> {
    match v {
        None | Some(Value::Null) => b.append_null(),
        Some(Value::Number(n)) => {
            let x = n
                .as_f64()
                .ok_or_else(|| schema_err(format!("'{field}' number not representable as f64")))?;
            b.append_value(x);
        }
        Some(other) => {
            return Err(schema_err(format!(
                "'{field}' must be a number, got {other}"
            )));
        }
    }
    Ok(())
}

fn push_opt_bool(b: &mut BooleanBuilder, v: Option<&Value>, field: &str) -> Result<()> {
    match v {
        None | Some(Value::Null) => b.append_null(),
        Some(Value::Bool(x)) => b.append_value(*x),
        Some(other) => {
            return Err(schema_err(format!(
                "'{field}' must be a boolean, got {other}"
            )));
        }
    }
    Ok(())
}

fn push_opt_json(b: &mut StringBuilder, v: Option<&Value>) -> Result<()> {
    match v {
        None | Some(Value::Null) => b.append_null(),
        Some(val) => b.append_value(serde_json::to_string(val)?),
    }
    Ok(())
}

/// The remaining members of `obj` not in `known`, as a JSON object (`None`
/// when nothing is left over).
fn other_members(obj: &serde_json::Map<String, Value>, known: &[&str]) -> Option<String> {
    let rest: serde_json::Map<String, Value> = obj
        .iter()
        .filter(|(k, _)| !known.contains(&k.as_str()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    if rest.is_empty() {
        None
    } else {
        Some(
            serde_json::to_string(&Value::Object(rest))
                .expect("Map<String, Value> always serialises"),
        )
    }
}

const MATERIAL_KNOWN_FIELDS: [&str; 8] = [
    "name",
    "ambientIntensity",
    "diffuseColor",
    "specularColor",
    "emissiveColor",
    "transparency",
    "shininess",
    "isSmooth",
];

/// Write one row per `defs[i]` to `path` (`id` = `i` as `Int64`), per
/// [`cityparquet_schema::profile::materials_schema`]'s column mapping.
/// Writes nothing and returns `0` when `defs` is empty. [`read_materials`]
/// is the value-exact inverse (numeric scalar columns normalise
/// integer-literal JSON numbers to float form; see the module docs).
pub fn write_materials(path: &Path, defs: &[Value]) -> Result<usize> {
    if defs.is_empty() {
        return Ok(0);
    }
    let schema = Arc::new(profile::materials_schema());

    let mut id = Int64Builder::with_capacity(defs.len());
    let mut name = StringBuilder::new();
    let mut ambient_intensity = Float64Builder::new();
    let mut diffuse_color = StringBuilder::new();
    let mut specular_color = StringBuilder::new();
    let mut emissive_color = StringBuilder::new();
    let mut transparency = Float64Builder::new();
    let mut shininess = Float64Builder::new();
    let mut is_smooth = BooleanBuilder::new();
    let mut other = StringBuilder::new();

    for (idx, def) in defs.iter().enumerate() {
        let obj = def
            .as_object()
            .ok_or_else(|| schema_err(format!("material def {idx} is not a JSON object")))?;
        id.append_value(idx as i64);
        push_opt_str(&mut name, obj.get("name"), "name")?;
        push_opt_f64(
            &mut ambient_intensity,
            obj.get("ambientIntensity"),
            "ambientIntensity",
        )?;
        push_opt_json(&mut diffuse_color, obj.get("diffuseColor"))?;
        push_opt_json(&mut specular_color, obj.get("specularColor"))?;
        push_opt_json(&mut emissive_color, obj.get("emissiveColor"))?;
        push_opt_f64(&mut transparency, obj.get("transparency"), "transparency")?;
        push_opt_f64(&mut shininess, obj.get("shininess"), "shininess")?;
        push_opt_bool(&mut is_smooth, obj.get("isSmooth"), "isSmooth")?;
        match other_members(obj, &MATERIAL_KNOWN_FIELDS) {
            Some(json) => other.append_value(json),
            None => other.append_null(),
        }
    }

    let arrays: Vec<ArrayRef> = vec![
        Arc::new(id.finish()),
        Arc::new(name.finish()),
        Arc::new(ambient_intensity.finish()),
        Arc::new(diffuse_color.finish()),
        Arc::new(specular_color.finish()),
        Arc::new(emissive_color.finish()),
        Arc::new(transparency.finish()),
        Arc::new(shininess.finish()),
        Arc::new(is_smooth.finish()),
        Arc::new(other.finish()),
    ];
    let batch = RecordBatch::try_new(schema.clone(), arrays)?;
    write_batch(path, schema, batch)?;
    Ok(defs.len())
}

/// `type`/`image`/`wrapMode`/`textureType`/`borderColor` are the only
/// members the writer peels into their own column (see the brief's Design
/// ruling 4); a texture object's `name` member, if present, is NOT a known
/// field here — it lands in `other` like anything else, and the `name`
/// column (kept for schema parity with materials) is always null.
const TEXTURE_KNOWN_FIELDS: [&str; 5] = ["type", "image", "wrapMode", "textureType", "borderColor"];

/// Write one row per `defs[i]` to `path` (`id` = `i` as `Int64`), per
/// [`cityparquet_schema::profile::textures_schema`]'s column mapping.
/// Writes nothing and returns `0` when `defs` is empty. [`read_textures`]
/// is the value-exact inverse — literally exact for textures, in fact,
/// since no texture column is numeric (the module docs' float
/// normalisation only applies to the materials table).
pub fn write_textures(path: &Path, defs: &[Value]) -> Result<usize> {
    if defs.is_empty() {
        return Ok(0);
    }
    let schema = Arc::new(profile::textures_schema());

    let mut id = Int64Builder::with_capacity(defs.len());
    let mut image_uri = StringBuilder::new();
    let mut mime_type = StringBuilder::new();
    let mut wrap_mode = StringBuilder::new();
    let mut texture_type = StringBuilder::new();
    let mut border_color = StringBuilder::new();
    let mut other = StringBuilder::new();

    for (idx, def) in defs.iter().enumerate() {
        let obj = def
            .as_object()
            .ok_or_else(|| schema_err(format!("texture def {idx} is not a JSON object")))?;
        id.append_value(idx as i64);
        push_opt_str(&mut image_uri, obj.get("image"), "image")?;
        push_opt_str(&mut mime_type, obj.get("type"), "type")?;
        push_opt_str(&mut wrap_mode, obj.get("wrapMode"), "wrapMode")?;
        push_opt_str(&mut texture_type, obj.get("textureType"), "textureType")?;
        push_opt_json(&mut border_color, obj.get("borderColor"))?;
        match other_members(obj, &TEXTURE_KNOWN_FIELDS) {
            Some(json) => other.append_value(json),
            None => other.append_null(),
        }
    }

    // image_data is never populated from a CityJSON texture definition (no
    // source field maps to it); an all-null Binary column of the right
    // length still round-trips cleanly through Arrow/Parquet.
    let mut image_data = arrow_array::builder::BinaryBuilder::new();
    for _ in 0..defs.len() {
        image_data.append_null();
    }

    let arrays: Vec<ArrayRef> = vec![
        Arc::new(id.finish()),
        Arc::new(image_uri.finish()),
        Arc::new(image_data.finish()),
        Arc::new(mime_type.finish()),
        Arc::new(wrap_mode.finish()),
        Arc::new(texture_type.finish()),
        Arc::new(border_color.finish()),
        Arc::new(other.finish()),
    ];
    let batch = RecordBatch::try_new(schema.clone(), arrays)?;
    write_batch(path, schema, batch)?;
    Ok(defs.len())
}

/// One row of `geometry_templates.parquet`: a geometry template's WKB (in
/// its own template-local coordinate space, via [`crate::wkb_write::VertexPool::raw`]),
/// its `geometry_properties` (mirrors the main-table column: `type`, `lod`,
/// `semantics`, `dropped_degenerate` if the writer dropped anything), and
/// its `material`/`texture` maps already rewritten to dataset-global ids by
/// the same [`crate::appearance::AppearanceInterner`] the main table and the
/// materials/textures sidecars use. `other` is reserved for any Geometry
/// member the schema doesn't otherwise carry (cjseq's `Geometry` is a fully
/// typed struct with no catch-all, so in practice this is always `None`).
#[derive(Debug, Clone, PartialEq)]
pub struct TemplateRow {
    pub id: String,
    pub wkb: Vec<u8>,
    pub geometry_properties: Option<Value>,
    pub material: Option<Value>,
    pub texture: Option<Value>,
    pub other: Option<Value>,
}

/// Write one row per `rows[i]` to `path`, per
/// [`cityparquet_schema::profile::geometry_templates_schema`]'s column
/// mapping (`id`/`geometry` non-null, everything else an optional JSON
/// column). Writes nothing and returns `0` when `rows` is empty. `id` is
/// written verbatim (the caller assigns it — the main-table `template.id`
/// column stores the template's position as a string, and this sidecar's
/// `id` must match it so a reader can join the two). [`read_templates`] is
/// the value-exact inverse.
pub fn write_templates(path: &Path, rows: &[TemplateRow]) -> Result<usize> {
    if rows.is_empty() {
        return Ok(0);
    }
    let schema = Arc::new(profile::geometry_templates_schema());

    let mut id = StringBuilder::new();
    let mut geometry = BinaryBuilder::new();
    let mut geometry_properties = StringBuilder::new();
    let mut material = StringBuilder::new();
    let mut texture = StringBuilder::new();
    let mut other = StringBuilder::new();

    for row in rows {
        id.append_value(&row.id);
        geometry.append_value(&row.wkb);
        push_opt_json(&mut geometry_properties, row.geometry_properties.as_ref())?;
        push_opt_json(&mut material, row.material.as_ref())?;
        push_opt_json(&mut texture, row.texture.as_ref())?;
        push_opt_json(&mut other, row.other.as_ref())?;
    }

    let arrays: Vec<ArrayRef> = vec![
        Arc::new(id.finish()),
        Arc::new(geometry.finish()),
        Arc::new(geometry_properties.finish()),
        Arc::new(material.finish()),
        Arc::new(texture.finish()),
        Arc::new(other.finish()),
    ];
    let batch = RecordBatch::try_new(schema.clone(), arrays)?;
    write_batch(path, schema, batch)?;
    Ok(rows.len())
}

/// Read `geometry_templates.parquet` at `path` back into one [`TemplateRow`]
/// per row, in file order. Missing file reads as empty (a dataset with no
/// geometry templates never gets a sidecar written; whether an ABSENT-BUT-
/// MANIFEST-LISTED file is instead an error is `export`'s call to make, not
/// this function's — see `crate::export`'s module doc for the M4 Codex-review
/// Finding 1 gating).
///
/// The join from a main-table `template.id` string to a row here is
/// POSITIONAL (row `i`'s id is `i.to_string()` — this crate's own
/// [`write_templates`]/`crate::package::build_template_rows`'s dense
/// contract), so `id` is read back and validated against row position
/// exactly like [`read_materials`]/[`read_textures`]: a row whose `id` does
/// not equal its position as a string (a corrupted/hand-rolled file, e.g. a
/// duplicated or reordered id) is a `Schema` error. This single check rules
/// out BOTH duplicate ids (two rows can't each equal both their own,
/// different positions) and gaps (a missing position would make some later
/// row's `id` disagree with its position) in one pass — matching the
/// materials/textures readers' rationale (M4 Codex-review Finding 2).
pub fn read_templates(path: &Path) -> Result<Vec<TemplateRow>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file =
        File::open(path).map_err(|e| io_err(format!("cannot open {}: {e}", path.display())))?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)
        .map_err(|e| parquet_err(format!("cannot open parquet reader: {e}")))?;
    let reader = builder
        .build()
        .map_err(|e| parquet_err(format!("cannot build parquet reader: {e}")))?;

    let mut out = Vec::new();
    let mut next_pos = 0usize;
    for batch in reader {
        let batch = batch.map_err(|e| parquet_err(format!("parquet read error: {e}")))?;
        let id: &StringArray = downcast(get_column(&batch, "id")?.as_ref(), "id")?;
        let geometry: &BinaryArray =
            downcast(get_column(&batch, "geometry")?.as_ref(), "geometry")?;
        let geometry_properties: &StringArray = downcast(
            get_column(&batch, "geometry_properties")?.as_ref(),
            "geometry_properties",
        )?;
        let material: &StringArray =
            downcast(get_column(&batch, "material")?.as_ref(), "material")?;
        let texture: &StringArray = downcast(get_column(&batch, "texture")?.as_ref(), "texture")?;
        let other: &StringArray = downcast(get_column(&batch, "other")?.as_ref(), "other")?;

        for row in 0..batch.num_rows() {
            let expected = next_pos.to_string();
            if id.value(row) != expected {
                return Err(schema_err(format!(
                    "geometry_templates.parquet: row at position {next_pos} has id {:?}, expected {:?} \
                     — ids must be dense '0'..'n' in row order",
                    id.value(row),
                    expected
                )));
            }
            next_pos += 1;

            out.push(TemplateRow {
                id: id.value(row).to_string(),
                wkb: geometry.value(row).to_vec(),
                geometry_properties: opt_json(geometry_properties, row)?,
                material: opt_json(material, row)?,
                texture: opt_json(texture, row)?,
                other: opt_json(other, row)?,
            });
        }
    }
    Ok(out)
}

fn get_column<'a>(batch: &'a RecordBatch, name: &str) -> Result<&'a ArrayRef> {
    batch.column_by_name(name).ok_or_else(|| {
        schema_err(format!(
            "sidecar record batch missing expected column '{name}'"
        ))
    })
}

fn downcast<'a, T: 'static>(array: &'a dyn Array, name: &str) -> Result<&'a T> {
    array.as_any().downcast_ref::<T>().ok_or_else(|| {
        schema_err(format!(
            "sidecar column '{name}' has an unexpected array type"
        ))
    })
}

fn opt_str(arr: &StringArray, row: usize) -> Option<String> {
    (!arr.is_null(row)).then(|| arr.value(row).to_string())
}

fn opt_f64(arr: &Float64Array, row: usize) -> Option<f64> {
    (!arr.is_null(row)).then(|| arr.value(row))
}

fn opt_bool(arr: &BooleanArray, row: usize) -> Option<bool> {
    (!arr.is_null(row)).then(|| arr.value(row))
}

/// Parse `col`'s row `row` (a JSON-text column) into a `Value`, `None` if
/// null.
fn opt_json(col: &StringArray, row: usize) -> Result<Option<Value>> {
    match opt_str(col, row) {
        None => Ok(None),
        Some(s) => Ok(Some(serde_json::from_str(&s)?)),
    }
}

/// Merge `other`'s parsed JSON object members into `map` (a no-op if `other`
/// is `None`; a `Schema` error if it parses to anything but a JSON object).
fn merge_other(map: &mut serde_json::Map<String, Value>, other: Option<Value>) -> Result<()> {
    let Some(other) = other else { return Ok(()) };
    let Value::Object(obj) = other else {
        return Err(schema_err(format!(
            "sidecar 'other' column must decode to a JSON object, got {other}"
        )));
    };
    for (k, v) in obj {
        map.insert(k, v);
    }
    Ok(())
}

/// Read `materials.parquet` at `path` back into one CityJSON material
/// definition per row, ordered by `id` (rows are written in `id` order and
/// read back batch-by-batch in file order, so no explicit sort is needed
/// unless a future writer reorders rows). Missing file reads as empty (a
/// dataset with no materials never gets a sidecar written). Value-exact
/// inverse of [`write_materials`]: `ambientIntensity`/`transparency`/
/// `shininess` come back in float-literal form regardless of how the source
/// wrote them (see the module docs).
///
/// The join back to a geometry's material index is POSITIONAL (row `i` is
/// material `i`), so the `id` column — this crate's own writer always sets
/// it to the row's position — is read back and validated: a row whose `id`
/// does not equal its position (a spec-conformant but row-reordered
/// third-party file, or a `materials.parquet` hand-edited/regenerated out of
/// order) is a `Schema` error rather than a silent mis-attribution of every
/// definition after that point. Ids are therefore required to be dense
/// `0..n` in row order; a future reader could instead sort by `id` and drop
/// this restriction, but no writer this crate produces needs that.
pub fn read_materials(path: &Path) -> Result<Vec<Value>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file =
        File::open(path).map_err(|e| io_err(format!("cannot open {}: {e}", path.display())))?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)
        .map_err(|e| parquet_err(format!("cannot open parquet reader: {e}")))?;
    let reader = builder
        .build()
        .map_err(|e| parquet_err(format!("cannot build parquet reader: {e}")))?;

    let mut out = Vec::new();
    let mut next_id = 0i64;
    for batch in reader {
        let batch = batch.map_err(|e| parquet_err(format!("parquet read error: {e}")))?;
        let id: &Int64Array = downcast(get_column(&batch, "id")?.as_ref(), "id")?;
        let name: &StringArray = downcast(get_column(&batch, "name")?.as_ref(), "name")?;
        let ambient_intensity: &Float64Array = downcast(
            get_column(&batch, "ambientIntensity")?.as_ref(),
            "ambientIntensity",
        )?;
        let diffuse_color: &StringArray =
            downcast(get_column(&batch, "diffuseColor")?.as_ref(), "diffuseColor")?;
        let specular_color: &StringArray = downcast(
            get_column(&batch, "specularColor")?.as_ref(),
            "specularColor",
        )?;
        let emissive_color: &StringArray = downcast(
            get_column(&batch, "emissiveColor")?.as_ref(),
            "emissiveColor",
        )?;
        let transparency: &Float64Array =
            downcast(get_column(&batch, "transparency")?.as_ref(), "transparency")?;
        let shininess: &Float64Array =
            downcast(get_column(&batch, "shininess")?.as_ref(), "shininess")?;
        let is_smooth: &BooleanArray =
            downcast(get_column(&batch, "isSmooth")?.as_ref(), "isSmooth")?;
        let other: &StringArray = downcast(get_column(&batch, "other")?.as_ref(), "other")?;

        for row in 0..batch.num_rows() {
            if id.value(row) != next_id {
                return Err(schema_err(format!(
                    "materials.parquet: row at position {next_id} has id {} — ids must be dense 0..n in row order",
                    id.value(row)
                )));
            }
            next_id += 1;

            let mut map = serde_json::Map::new();
            if let Some(v) = opt_str(name, row) {
                map.insert("name".to_string(), Value::String(v));
            }
            if let Some(v) = opt_f64(ambient_intensity, row) {
                map.insert("ambientIntensity".to_string(), serde_json::json!(v));
            }
            if let Some(v) = opt_json(diffuse_color, row)? {
                map.insert("diffuseColor".to_string(), v);
            }
            if let Some(v) = opt_json(specular_color, row)? {
                map.insert("specularColor".to_string(), v);
            }
            if let Some(v) = opt_json(emissive_color, row)? {
                map.insert("emissiveColor".to_string(), v);
            }
            if let Some(v) = opt_f64(transparency, row) {
                map.insert("transparency".to_string(), serde_json::json!(v));
            }
            if let Some(v) = opt_f64(shininess, row) {
                map.insert("shininess".to_string(), serde_json::json!(v));
            }
            if let Some(v) = opt_bool(is_smooth, row) {
                map.insert("isSmooth".to_string(), Value::Bool(v));
            }
            merge_other(&mut map, opt_json(other, row)?)?;
            out.push(Value::Object(map));
        }
    }
    Ok(out)
}

/// Read `textures.parquet` at `path` back into one CityJSON texture
/// definition per row (see [`read_materials`] for the ordering/missing-file
/// contract, and its docs for why the `id` column is read back and
/// validated against row position rather than trusted implicitly).
///
/// This crate's own [`write_textures`] never populates `name` (it always
/// routes a source texture's `name` member to `other` instead — see its
/// doc) or `image_data` (always null), but a third-party/hand-rolled file
/// might: a non-null `name` is restored as the definition's `"name"` member
/// (an extra member CityJSON tolerates; a re-convert of the exported file
/// would route it back to `other`, same as any writer-unknown member) and a
/// non-null `image_data` (embedded image bytes, which have no JSON
/// representation to restore them into) is a `Schema` error naming the row —
/// honest rejection beats silently losing the bytes.
pub fn read_textures(path: &Path) -> Result<Vec<Value>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file =
        File::open(path).map_err(|e| io_err(format!("cannot open {}: {e}", path.display())))?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)
        .map_err(|e| parquet_err(format!("cannot open parquet reader: {e}")))?;
    let reader = builder
        .build()
        .map_err(|e| parquet_err(format!("cannot build parquet reader: {e}")))?;

    let mut out = Vec::new();
    let mut next_id = 0i64;
    for batch in reader {
        let batch = batch.map_err(|e| parquet_err(format!("parquet read error: {e}")))?;
        let id: &Int64Array = downcast(get_column(&batch, "id")?.as_ref(), "id")?;
        // This crate no longer writes a `name` column (§11.3, G16), but a
        // third-party file may still carry one; read it only when present so a
        // populated foreign `name` is still restored (M4 tolerance).
        let name: Option<&StringArray> = match batch.schema().field_with_name("name") {
            Ok(_) => Some(downcast(get_column(&batch, "name")?.as_ref(), "name")?),
            Err(_) => None,
        };
        let image_uri: &StringArray =
            downcast(get_column(&batch, "image_uri")?.as_ref(), "image_uri")?;
        let image_data: &BinaryArray =
            downcast(get_column(&batch, "image_data")?.as_ref(), "image_data")?;
        let mime_type: &StringArray =
            downcast(get_column(&batch, "mime_type")?.as_ref(), "mime_type")?;
        let wrap_mode: &StringArray =
            downcast(get_column(&batch, "wrapMode")?.as_ref(), "wrapMode")?;
        let texture_type: &StringArray =
            downcast(get_column(&batch, "textureType")?.as_ref(), "textureType")?;
        let border_color: &StringArray =
            downcast(get_column(&batch, "borderColor")?.as_ref(), "borderColor")?;
        let other: &StringArray = downcast(get_column(&batch, "other")?.as_ref(), "other")?;

        for row in 0..batch.num_rows() {
            if id.value(row) != next_id {
                return Err(schema_err(format!(
                    "textures.parquet: row at position {next_id} has id {} — ids must be dense 0..n in row order",
                    id.value(row)
                )));
            }
            next_id += 1;

            // Embedded image bytes have no JSON representation to restore
            // them into: honest rejection beats silently dropping them (see
            // this function's doc).
            if !image_data.is_null(row) {
                return Err(schema_err(format!(
                    "textures.parquet row {row}: embedded 'image_data' is not supported by this \
                     reader (no JSON representation to restore it into)"
                )));
            }

            let mut map = serde_json::Map::new();
            if let Some(v) = opt_str(mime_type, row) {
                map.insert("type".to_string(), Value::String(v));
            }
            if let Some(v) = opt_str(image_uri, row) {
                map.insert("image".to_string(), Value::String(v));
            }
            if let Some(v) = opt_str(wrap_mode, row) {
                map.insert("wrapMode".to_string(), Value::String(v));
            }
            if let Some(v) = opt_str(texture_type, row) {
                map.insert("textureType".to_string(), Value::String(v));
            }
            if let Some(v) = opt_json(border_color, row)? {
                map.insert("borderColor".to_string(), v);
            }
            // Our own writer no longer emits this column (§11.3, G16), but a
            // third-party file might carry a populated one: restore it as a
            // plain extra member rather than discard it silently.
            if let Some(v) = name.and_then(|n| opt_str(n, row)) {
                map.insert("name".to_string(), Value::String(v));
            }
            merge_other(&mut map, opt_json(other, row)?)?;
            out.push(Value::Object(map));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    use cjseq::CityJSON;

    fn fixture(name: &str) -> PathBuf {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures")
            .join(name);
        assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
        p
    }

    fn canonical(v: &Value) -> String {
        // Key-sorted (recursively) serialization: a value-equality compare
        // independent of member order, and independent of serde_json's
        // object-map type (see `crate::appearance::canonical_json_string`).
        crate::appearance::canonical_json_string(v)
    }

    fn assert_defs_equal(actual: &[Value], expected: &[Value]) {
        assert_eq!(actual.len(), expected.len());
        for (i, (a, e)) in actual.iter().zip(expected).enumerate() {
            assert_eq!(canonical(a), canonical(e), "def {i} mismatch");
        }
    }

    #[test]
    fn empty_defs_write_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("materials.parquet");
        assert_eq!(write_materials(&path, &[]).unwrap(), 0);
        assert!(!path.exists());
        assert_eq!(read_materials(&path).unwrap(), Vec::<Value>::new());

        let path = dir.path().join("textures.parquet");
        assert_eq!(write_textures(&path, &[]).unwrap(), 0);
        assert!(!path.exists());
        assert_eq!(read_textures(&path).unwrap(), Vec::<Value>::new());
    }

    /// Real CityJSON data: railway's own header `appearance.materials` /
    /// `appearance.textures` (85 / 34 definitions), round-tripped through
    /// write -> read with no interner involved (that wiring is Task 7's
    /// encode-side test, this is the sidecar codec on its own).
    #[test]
    fn railway_materials_and_textures_round_trip() {
        let raw_text = std::fs::read_to_string(fixture("lod3_railway.city.json")).unwrap();
        let doc = CityJSON::from_str(&raw_text).unwrap();
        let appearance = doc.appearance.as_ref().expect("railway has appearance");
        let materials = appearance.materials.clone().unwrap_or_default();
        let textures = appearance.textures.clone().unwrap_or_default();
        assert_eq!(materials.len(), 85);
        assert_eq!(textures.len(), 34);

        let dir = tempfile::tempdir().unwrap();
        let materials_path = dir.path().join("materials.parquet");
        let textures_path = dir.path().join("textures.parquet");

        let m_written = write_materials(&materials_path, &materials).unwrap();
        let t_written = write_textures(&textures_path, &textures).unwrap();
        assert_eq!(m_written, 85);
        assert_eq!(t_written, 34);

        let m_back = read_materials(&materials_path).unwrap();
        let t_back = read_textures(&textures_path).unwrap();
        assert_defs_equal(&m_back, &materials);
        assert_defs_equal(&t_back, &textures);

        // Concrete mapping assertion: row 0's texture columns.
        let file = File::open(&textures_path).unwrap();
        let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
        let mut reader = builder.build().unwrap();
        let batch = reader.next().unwrap().unwrap();
        let mime_type: &StringArray = batch
            .column_by_name("mime_type")
            .unwrap()
            .as_any()
            .downcast_ref()
            .unwrap();
        let image_uri: &StringArray = batch
            .column_by_name("image_uri")
            .unwrap()
            .as_any()
            .downcast_ref()
            .unwrap();
        assert_eq!(mime_type.value(0), "JPG");
        assert_eq!(image_uri.value(0), "appearances/Vegetation_Juniper2.jpg");

        // G16 (§11.3): a CityJSON `Texture` has no `name` member, so
        // `textures.parquet` carries no `name` column.
        assert!(
            batch.schema().field_with_name("name").is_err(),
            "textures.parquet must not declare a `name` column (§11.3)"
        );
    }

    #[test]
    fn materials_write_rejects_non_object_defs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("materials.parquet");
        let err = write_materials(&path, &[Value::String("not an object".into())]).unwrap_err();
        assert!(matches!(err, CityParquetError::Schema(_)));
    }

    #[test]
    fn textures_write_rejects_non_object_defs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("textures.parquet");
        let err = write_textures(&path, &[Value::String("not an object".into())]).unwrap_err();
        assert!(matches!(err, CityParquetError::Schema(_)));
    }

    /// Reads back `path`'s first (only, for these small test files) batch,
    /// rewrites it with the `id` column shifted by `+1` — a corrupted file
    /// whose `id` no longer matches row position, standing in for a
    /// spec-conformant but row-reordered third-party file (see
    /// [`read_materials`]'s docs for why the acceptable alternative from the
    /// M4 final-review brief is used here instead of physically reordering
    /// rows: shifting `id` alone is enough to prove the reader actually
    /// looks at the column instead of trusting position).
    fn corrupt_id_column_by_shifting(path: &Path, schema: Arc<Schema>) {
        let file = File::open(path).unwrap();
        let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
        let mut reader = builder.build().unwrap();
        let batch = reader.next().unwrap().unwrap();
        assert!(
            reader.next().is_none(),
            "test assumes a single record batch"
        );

        let old_id: &Int64Array = batch
            .column_by_name("id")
            .unwrap()
            .as_any()
            .downcast_ref()
            .unwrap();
        let mut shifted = Int64Builder::with_capacity(old_id.len());
        for i in 0..old_id.len() {
            shifted.append_value(old_id.value(i) + 1);
        }
        let mut arrays: Vec<ArrayRef> = batch.columns().to_vec();
        let id_pos = batch.schema().index_of("id").unwrap();
        arrays[id_pos] = Arc::new(shifted.finish());

        let corrupted = RecordBatch::try_new(schema.clone(), arrays).unwrap();
        write_batch(path, schema, corrupted).unwrap();
    }

    /// Reviewer follow-up (M4 final-review Fix 2): the join from a
    /// `materials.parquet` row to the geometry `material` index it backs is
    /// positional, so a file whose `id` column disagrees with row position
    /// (a spec-conformant but reordered/corrupted third-party file) must be
    /// rejected rather than silently mis-attributing every definition from
    /// that point on. Derived from real railway material definitions.
    #[test]
    fn read_materials_rejects_id_column_disagreeing_with_row_position() {
        let raw_text = std::fs::read_to_string(fixture("lod3_railway.city.json")).unwrap();
        let doc = CityJSON::from_str(&raw_text).unwrap();
        let materials = doc
            .appearance
            .as_ref()
            .and_then(|a| a.materials.clone())
            .expect("railway has materials");

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("materials.parquet");
        write_materials(&path, &materials).unwrap();

        // Precondition: the honest file reads back fine.
        assert_eq!(read_materials(&path).unwrap().len(), materials.len());

        corrupt_id_column_by_shifting(&path, Arc::new(profile::materials_schema()));

        let err = read_materials(&path).unwrap_err();
        assert!(
            matches!(err, CityParquetError::Schema(_)),
            "expected a Schema error, got {err:?}"
        );
        assert!(
            err.to_string().contains("id"),
            "the error must mention the id mismatch, got: {err}"
        );
    }

    /// [`read_materials_rejects_id_column_disagreeing_with_row_position`]'s
    /// counterpart for `read_textures`.
    #[test]
    fn read_textures_rejects_id_column_disagreeing_with_row_position() {
        let raw_text = std::fs::read_to_string(fixture("lod3_railway.city.json")).unwrap();
        let doc = CityJSON::from_str(&raw_text).unwrap();
        let textures = doc
            .appearance
            .as_ref()
            .and_then(|a| a.textures.clone())
            .expect("railway has textures");

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("textures.parquet");
        write_textures(&path, &textures).unwrap();

        assert_eq!(read_textures(&path).unwrap().len(), textures.len());

        corrupt_id_column_by_shifting(&path, Arc::new(profile::textures_schema()));

        let err = read_textures(&path).unwrap_err();
        assert!(
            matches!(err, CityParquetError::Schema(_)),
            "expected a Schema error, got {err:?}"
        );
        assert!(
            err.to_string().contains("id"),
            "the error must mention the id mismatch, got: {err}"
        );
    }

    /// Derived-from-real-fixture: railway's material numerics all happen to
    /// be float literals (`0.0`), so the fixture never exercises the
    /// documented normalisation — a numeric scalar column written from an
    /// INTEGER-literal JSON number reads back in float form (same value,
    /// different literal; see the module docs). Pin it: take a real railway
    /// material def, set `"shininess": 1` as a bare integer, round-trip.
    #[test]
    fn material_numeric_columns_are_value_exact_not_literal_exact() {
        let raw_text = std::fs::read_to_string(fixture("lod3_railway.city.json")).unwrap();
        let doc = CityJSON::from_str(&raw_text).unwrap();
        let materials = doc
            .appearance
            .as_ref()
            .and_then(|a| a.materials.clone())
            .expect("railway has materials");

        let mut def = materials[0].clone();
        def.as_object_mut()
            .unwrap()
            .insert("shininess".to_string(), serde_json::json!(1));
        assert!(
            def["shininess"].is_i64(),
            "precondition: shininess starts as an integer-literal JSON number"
        );

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("materials.parquet");
        assert_eq!(
            write_materials(&path, std::slice::from_ref(&def)).unwrap(),
            1
        );
        let back = read_materials(&path).unwrap();
        assert_eq!(back.len(), 1);

        // Value-exact: same number...
        assert_eq!(back[0]["shininess"].as_f64(), Some(1.0));
        // ...but the documented normalisation to float-literal form: `1.0`
        // comes back, NOT the original bare `1` (which serde_json's
        // `PartialEq` treats as a different Value).
        assert_eq!(back[0]["shininess"], serde_json::json!(1.0));
        assert_ne!(back[0]["shininess"], serde_json::json!(1));
        // The dataset comparator treats mixed int/float as equal within
        // tolerance, so this normalisation cannot break dataset-level
        // round-trip equality; here only `shininess` differs in literal
        // form — every other member must still round-trip literally.
        let mut expected = def.clone();
        expected
            .as_object_mut()
            .unwrap()
            .insert("shininess".to_string(), serde_json::json!(1.0));
        assert_eq!(canonical(&back[0]), canonical(&expected));
    }

    /// Real railway geometry-templates (3 templates): build one
    /// [`TemplateRow`] per template built by the PRODUCTION builder
    /// (`crate::package::build_template_rows`, exercised directly so this
    /// test cannot drift from what convert actually writes — a hand-built
    /// duplicate of its logic previously masked a missing `"lod"`),
    /// write/read round-trip.
    #[test]
    fn railway_templates_round_trip() {
        use crate::appearance::AppearanceInterner;
        use crate::package::build_template_rows;
        use crate::source::Source;
        use crate::wkb_read::wkb_to_geometry;

        let source = Source::open(&fixture("lod3_railway.city.json")).unwrap();
        let templates = source
            .header()
            .geometry_templates
            .clone()
            .expect("railway has geometry-templates");
        assert_eq!(templates.templates.len(), 3);

        let mut interner = AppearanceInterner::new();
        let rows = build_template_rows(&templates, &source, &mut interner).unwrap();
        assert_eq!(rows.len(), 3);
        for (i, (row, tpl)) in rows.iter().zip(&templates.templates).enumerate() {
            let props = row
                .geometry_properties
                .as_ref()
                .expect("template rows carry geometry_properties");
            assert!(props.get("type").is_some(), "template {i} missing type");
            assert_eq!(
                props.get("lod").and_then(|v| v.as_str()),
                tpl.lod.as_deref(),
                "template {i}: geometry_properties must carry the source lod"
            );
        }

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("geometry_templates.parquet");
        let written = write_templates(&path, &rows).unwrap();
        assert_eq!(written, 3);

        let back = read_templates(&path).unwrap();
        assert_eq!(back.len(), 3);
        for (i, row) in back.iter().enumerate() {
            assert_eq!(row.id, i.to_string());
            wkb_to_geometry(&row.wkb).expect("sidecar WKB must be accepted by the hardened reader");
        }
        assert_eq!(back[0].material, rows[0].material);
        assert_eq!(back[0].texture, rows[0].texture);
        assert_eq!(back[0].geometry_properties, rows[0].geometry_properties);
    }

    /// M4 Codex-review Finding 2: `read_templates` must validate the dense
    /// `id == position` contract, exactly like `read_materials`/
    /// `read_textures` already do — this single check rules out both
    /// duplicate ids and gaps. Derived from the real railway templates
    /// sidecar (3 rows, ids `"0"`, `"1"`, `"2"`): row 1's id is corrupted to
    /// `"0"` (a duplicate of row 0's), which is simultaneously a gap at
    /// position 1 — either framing is valid, one check catches both.
    #[test]
    fn read_templates_rejects_a_duplicate_id() {
        use crate::appearance::AppearanceInterner;
        use crate::package::build_template_rows;
        use crate::source::Source;

        let source = Source::open(&fixture("lod3_railway.city.json")).unwrap();
        let templates = source
            .header()
            .geometry_templates
            .clone()
            .expect("railway has geometry-templates");
        let mut interner = AppearanceInterner::new();
        let mut rows = build_template_rows(&templates, &source, &mut interner).unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[1].id, "1", "precondition: row 1 starts as id \"1\"");
        rows[1].id = "0".to_string();

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("geometry_templates.parquet");
        write_templates(&path, &rows).unwrap();

        let err = read_templates(&path).unwrap_err();
        assert!(
            matches!(err, CityParquetError::Schema(_)),
            "expected a Schema error, got {err:?}"
        );
        assert!(
            err.to_string().contains("id"),
            "the error must mention the id mismatch, got: {err}"
        );
    }

    /// Builds a one-row `textures.parquet`-shaped batch directly via Arrow
    /// (this crate's own [`write_textures`] never sets `name`/`image_data`,
    /// so a hand-built batch is the only way to exercise a third-party file
    /// that does — sanctioned per the M4 Codex-review brief for Finding 4).
    fn write_one_texture_row(path: &Path, name: Option<&str>, image_data: Option<&[u8]>) {
        // A foreign file that carries a `name` column (this crate no longer
        // writes one, §11.3/G16): the real schema plus a `name` field after
        // `id`, matching the array order below.
        let mut fields: Vec<arrow_schema::Field> = profile::textures_schema()
            .fields()
            .iter()
            .map(|f| f.as_ref().clone())
            .collect();
        fields.insert(
            1,
            arrow_schema::Field::new("name", arrow_schema::DataType::Utf8, true),
        );
        let schema = Arc::new(Schema::new(fields));
        let mut id = Int64Builder::new();
        let mut name_b = StringBuilder::new();
        let mut image_uri = StringBuilder::new();
        let mut image_data_b = BinaryBuilder::new();
        let mut mime_type = StringBuilder::new();
        let mut wrap_mode = StringBuilder::new();
        let mut texture_type = StringBuilder::new();
        let mut border_color = StringBuilder::new();
        let mut other = StringBuilder::new();

        id.append_value(0);
        match name {
            Some(n) => name_b.append_value(n),
            None => name_b.append_null(),
        }
        image_uri.append_value("appearances/foo.jpg");
        match image_data {
            Some(bytes) => image_data_b.append_value(bytes),
            None => image_data_b.append_null(),
        }
        mime_type.append_value("JPG");
        wrap_mode.append_null();
        texture_type.append_null();
        border_color.append_null();
        other.append_null();

        let arrays: Vec<ArrayRef> = vec![
            Arc::new(id.finish()),
            Arc::new(name_b.finish()),
            Arc::new(image_uri.finish()),
            Arc::new(image_data_b.finish()),
            Arc::new(mime_type.finish()),
            Arc::new(wrap_mode.finish()),
            Arc::new(texture_type.finish()),
            Arc::new(border_color.finish()),
            Arc::new(other.finish()),
        ];
        let batch = RecordBatch::try_new(schema.clone(), arrays).unwrap();
        write_batch(path, schema, batch).unwrap();
    }

    /// M4 Codex-review Finding 4: a non-null `name` column (never written by
    /// this crate's own [`write_textures`], but legal for a third-party file)
    /// must be restored as the definition's `"name"` member, not silently
    /// discarded.
    #[test]
    fn read_textures_restores_a_non_null_name_column() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("textures.parquet");
        write_one_texture_row(&path, Some("my-texture"), None);

        let defs = read_textures(&path).unwrap();
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0]["name"], serde_json::json!("my-texture"));
    }

    /// M4 Codex-review Finding 4: a non-null `image_data` column (embedded
    /// image bytes, which have no JSON representation to restore them into)
    /// must be a `Schema` error, not a silent drop of the bytes.
    #[test]
    fn read_textures_rejects_non_null_image_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("textures.parquet");
        write_one_texture_row(&path, None, Some(&[0xDE, 0xAD, 0xBE, 0xEF]));

        let err = read_textures(&path).unwrap_err();
        assert!(
            matches!(err, CityParquetError::Schema(_)),
            "expected a Schema error, got {err:?}"
        );
        assert!(
            err.to_string().contains("image_data"),
            "error must name image_data, got: {err}"
        );
    }
}
