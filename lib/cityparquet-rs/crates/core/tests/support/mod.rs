//! Shared helpers for the tests that take a real rs-written package and
//! rewrite one of its files the way a different writer might have written
//! it — the only way to obtain a foreign-shaped (or deliberately
//! non-conformant) CityParquet file without hand-writing CityJSON.

#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use arrow_array::{ArrayRef, RecordBatch, make_array};
use arrow_schema::{DataType, Field, Fields, Schema};
use cityparquet::package::{ConvertOptions, convert};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::arrow::arrow_writer::ArrowWriterOptions;
use parquet::arrow::{ArrowSchemaConverter, ArrowWriter};
use parquet::file::metadata::KeyValue;
use parquet::file::properties::WriterProperties;
use parquet::schema::types::{SchemaDescriptor, Type, TypePtr};

pub fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

/// Convert a real fixture into `<tempdir>/pkg` with the default options.
pub fn convert_fixture(name: &str) -> (tempfile::TempDir, PathBuf) {
    convert_fixture_with(name, |_| {})
}

/// Convert a real fixture into `<tempdir>/pkg`, with `configure` applied to
/// the default options first.
pub fn convert_fixture_with(
    name: &str,
    configure: impl FnOnce(&mut ConvertOptions),
) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("pkg");
    let mut opts = ConvertOptions::new(fixture(name), out.clone());
    configure(&mut opts);
    convert(&opts).unwrap_or_else(|e| panic!("convert {name}: {e}"));
    (dir, out)
}

/// One Parquet file's content, decoded: its record batches (all row groups)
/// and its footer key-value metadata without the Arrow-specific
/// `ARROW:schema` entry.
pub struct FileContent {
    pub schema: Arc<Schema>,
    pub batches: Vec<RecordBatch>,
    pub key_values: Vec<(String, String)>,
}

impl FileContent {
    pub fn read(path: &Path) -> Self {
        let file = fs::File::open(path).unwrap();
        let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
        let key_values = builder
            .metadata()
            .file_metadata()
            .key_value_metadata()
            .map(|kvs| {
                kvs.iter()
                    .filter(|kv| kv.key != "ARROW:schema")
                    .map(|kv| (kv.key.clone(), kv.value.clone().unwrap_or_default()))
                    .collect()
            })
            .unwrap_or_default();
        let schema = builder.schema().clone();
        let batches = builder
            .build()
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();
        Self {
            schema,
            batches,
            key_values,
        }
    }

    pub fn key_value(&self, key: &str) -> Option<&str> {
        self.key_values
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    pub fn set_key_value(&mut self, key: &str, value: String) {
        match self.key_values.iter_mut().find(|(k, _)| k == key) {
            Some((_, v)) => *v = value,
            None => self.key_values.push((key.to_string(), value)),
        }
    }

    pub fn remove_key_value(&mut self, key: &str) {
        self.key_values.retain(|(k, _)| k != key);
    }

    /// The parsed JSON value of the footer key `key`.
    pub fn key_json(&self, key: &str) -> serde_json::Value {
        serde_json::from_str(
            self.key_value(key)
                .unwrap_or_else(|| panic!("no {key} key")),
        )
        .unwrap()
    }

    /// Replace column `name` in every batch by `f(field, column, batch
    /// index)`, which returns the new field and column.
    pub fn map_column(
        &mut self,
        name: &str,
        mut f: impl FnMut(&Field, &ArrayRef, usize) -> (Field, ArrayRef),
    ) {
        let index = self.schema.index_of(name).unwrap();
        let mut new_field = None;
        let mut batches = Vec::with_capacity(self.batches.len());
        for (b, batch) in self.batches.iter().enumerate() {
            let (field, column) = f(self.schema.field(index), batch.column(index), b);
            let mut columns = batch.columns().to_vec();
            columns[index] = column;
            let mut fields: Vec<Field> = self
                .schema
                .fields()
                .iter()
                .map(|f| f.as_ref().clone())
                .collect();
            fields[index] = field.clone();
            let schema = Arc::new(Schema::new(fields));
            batches.push(RecordBatch::try_new(schema, columns).unwrap());
            new_field = Some(field);
        }
        let mut fields: Vec<Field> = self
            .schema
            .fields()
            .iter()
            .map(|f| f.as_ref().clone())
            .collect();
        fields[index] = new_field.unwrap();
        self.schema = Arc::new(Schema::new(fields));
        self.batches = batches;
    }

    /// Re-type column `name` to `data_type` (an Arrow cast of every batch),
    /// keeping the field's name and nullability but none of its metadata.
    pub fn retype(&mut self, name: &str, data_type: &DataType) {
        self.map_column(name, |field, column, _| {
            let cast = arrow_cast::cast(column, data_type)
                .unwrap_or_else(|e| panic!("cannot cast `{name}` to {data_type}: {e}"));
            (
                Field::new(field.name(), data_type.clone(), field.is_nullable()),
                cast,
            )
        });
    }

    /// Store the JSON columns as plain UTF8, the way a writer that does not
    /// annotate them would: every column when `only` is `None`, else just
    /// the top-level column `only`.
    pub fn untag_json(&mut self, only: Option<&str>) {
        let names: Vec<String> = self
            .schema
            .fields()
            .iter()
            .map(|f| f.name().clone())
            .filter(|n| only.is_none_or(|o| o == n))
            .collect();
        for name in names {
            self.map_column(&name, |field, column, _| {
                let field = untag(field);
                let data = column
                    .to_data()
                    .into_builder()
                    .data_type(field.data_type().clone())
                    .build()
                    .unwrap();
                (field, make_array(data))
            });
        }
    }

    /// Write this content to `path`, with no embedded `ARROW:schema`: what
    /// lands in the file is only what the Parquet schema and the footer
    /// key-value pairs say, as it would be for a writer that is not Arrow
    /// based.
    pub fn write(&self, path: &Path) {
        self.write_with_schema(path, |_| {});
    }

    /// [`Self::write`], with `edit` applied to the Parquet schema the Arrow
    /// schema converts to: it receives the root's fields and may replace
    /// any of them, as long as the levels of the replaced column stay those
    /// the Arrow data produces.
    pub fn write_with_schema(&self, path: &Path, edit: impl FnOnce(&mut Vec<TypePtr>)) {
        let converted = ArrowSchemaConverter::new().convert(&self.schema).unwrap();
        let root = converted.root_schema();
        let mut fields = root.get_fields().to_vec();
        edit(&mut fields);
        let root = Type::group_type_builder(root.name())
            .with_fields(fields)
            .build()
            .unwrap();
        let kvs: Vec<KeyValue> = self
            .key_values
            .iter()
            .map(|(k, v)| KeyValue::new(k.clone(), v.clone()))
            .collect();
        let props = WriterProperties::builder()
            .set_key_value_metadata(Some(kvs))
            .build();
        let options = ArrowWriterOptions::new()
            .with_properties(props)
            .with_skip_arrow_metadata(true)
            .with_parquet_schema(SchemaDescriptor::new(Arc::new(root)));
        let file = fs::File::create(path).unwrap();
        let mut writer =
            ArrowWriter::try_new_with_options(file, self.schema.clone(), options).unwrap();
        for batch in &self.batches {
            writer.write(batch).unwrap();
        }
        writer.close().unwrap();
    }
}

/// `field` with any `arrow.json` extension tag removed, recursively through
/// struct children.
fn untag(field: &Field) -> Field {
    let data_type = match field.data_type() {
        DataType::Struct(children) => DataType::Struct(Fields::from(
            children.iter().map(|c| untag(c)).collect::<Vec<_>>(),
        )),
        other => other.clone(),
    };
    let mut metadata = field.metadata().clone();
    if metadata.get("ARROW:extension:name").map(String::as_str) == Some("arrow.json") {
        metadata.remove("ARROW:extension:name");
        metadata.remove("ARROW:extension:metadata");
    }
    Field::new(field.name(), data_type, field.is_nullable()).with_metadata(metadata)
}
