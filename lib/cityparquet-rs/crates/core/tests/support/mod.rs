//! Shared helpers for the tests that take a real rs-written package and
//! rewrite one of its files the way a different writer might have written
//! it — the only way to obtain a foreign-shaped (or deliberately
//! non-conformant) CityParquet file without hand-writing CityJSON.

#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use arrow_array::RecordBatch;
use arrow_schema::Schema;
use cityparquet::package::{ConvertOptions, convert};
use parquet::arrow::ArrowWriter;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::arrow::arrow_writer::ArrowWriterOptions;
use parquet::file::metadata::KeyValue;
use parquet::file::properties::WriterProperties;

pub fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

/// Convert a real fixture into `<tempdir>/pkg` with the default options.
pub fn convert_fixture(name: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("pkg");
    convert(&ConvertOptions::new(fixture(name), out.clone()))
        .unwrap_or_else(|e| panic!("convert {name}: {e}"));
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

    /// Write this content to `path`, with no embedded `ARROW:schema`: what
    /// lands in the file is only what the Parquet schema and the footer
    /// key-value pairs say, as it would be for a writer that is not Arrow
    /// based.
    pub fn write(&self, path: &Path) {
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
            .with_skip_arrow_metadata(true);
        let file = fs::File::create(path).unwrap();
        let mut writer =
            ArrowWriter::try_new_with_options(file, self.schema.clone(), options).unwrap();
        for batch in &self.batches {
            writer.write(batch).unwrap();
        }
        writer.close().unwrap();
    }
}
