//! A CityParquet file from a foreign writer must read. The physical conventions
//! here are duckdb-cityjson's: its reserved-column order, plain Utf8 for
//! `object_type`, `element` as the LIST child name, microsecond timestamps, and
//! no `address`/`template`/`other_attributes` columns.

use std::sync::Arc;

use arrow_array::{ArrayRef, RecordBatch, StringArray, TimestampMicrosecondArray};
use arrow_schema::{DataType, Field, Schema, TimeUnit};
use parquet::arrow::ArrowWriter;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::file::properties::WriterProperties;

use cityparquet::reader::CityParquetReaderBuilder;

/// A minimal but conformant `city` footer for a one-attribute object table.
const CITY_JSON: &str = r#"{
  "version": "0.1.0-draft",
  "attributes": ["tijdstipregistratie"]
}"#;

fn foreign_file(path: &std::path::Path) {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("feature_id", DataType::Utf8, false),
        // Plain Utf8, not Dictionary — the spec's dictionary encoding is a SHOULD
        // about Parquet encoding, not an Arrow logical type.
        Field::new("object_type", DataType::Utf8, false),
        // duckdb's order: children before parents. The spec's order is normative
        // for writers; a reader must not depend on it.
        Field::new(
            "children",
            DataType::List(Field::new("element", DataType::Utf8, true).into()),
            true,
        ),
        Field::new(
            "parents",
            DataType::List(Field::new("element", DataType::Utf8, true).into()),
            true,
        ),
        Field::new(
            "tijdstipregistratie",
            DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
            true,
        ),
    ]));

    let ids: ArrayRef = Arc::new(StringArray::from(vec!["obj-1"]));
    let feature_ids: ArrayRef = Arc::new(StringArray::from(vec!["obj-1"]));
    let types: ArrayRef = Arc::new(StringArray::from(vec!["Bridge"]));
    let children: ArrayRef = arrow_array::array::new_null_array(
        &DataType::List(Field::new("element", DataType::Utf8, true).into()),
        1,
    );
    let parents: ArrayRef = arrow_array::array::new_null_array(
        &DataType::List(Field::new("element", DataType::Utf8, true).into()),
        1,
    );
    let stamps: ArrayRef = Arc::new(
        TimestampMicrosecondArray::from(vec![1_600_000_000_000_000i64]).with_timezone("UTC"),
    );

    let batch = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![ids, feature_ids, types, children, parents, stamps],
    )
    .expect("foreign batch");

    let props = WriterProperties::builder()
        .set_key_value_metadata(Some(vec![parquet::file::metadata::KeyValue::new(
            "city".to_string(),
            CITY_JSON.to_string(),
        )]))
        .build();
    let file = std::fs::File::create(path).expect("create");
    let mut writer = ArrowWriter::try_new(file, schema, Some(props)).expect("writer");
    writer.write(&batch).expect("write");
    writer.close().expect("close");
}

#[test]
fn renders_the_foreign_writers_own_fields_not_the_canonical_set() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("bridge.parquet");
    foreign_file(&path);

    let file = std::fs::File::open(&path).expect("open");
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).expect("builder");
    let rendered = builder
        .cityparquet_arrow_schema()
        .expect("a conformant foreign file must render");

    let names: Vec<&str> = rendered
        .fields()
        .iter()
        .map(|f| f.name().as_str())
        .collect();
    assert_eq!(
        names,
        vec![
            "id",
            "feature_id",
            "object_type",
            "children",
            "parents",
            "tijdstipregistratie"
        ],
        "the rendered schema must be the file's own fields, in the file's order"
    );

    // Physical types are the file's, not this crate's canonical choices.
    assert_eq!(
        rendered.field_with_name("object_type").unwrap().data_type(),
        &DataType::Utf8
    );
    assert_eq!(
        rendered
            .field_with_name("tijdstipregistratie")
            .unwrap()
            .data_type(),
        &DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into()))
    );
}

#[test]
fn hierarchy_columns_keep_their_own_names_through_the_restamp() {
    // `parents` and `children` are both LIST<VARCHAR>, so a positional restamp
    // against a schema that orders them the other way round transposes them
    // silently — no error, corrupted hierarchy. Pin that it cannot happen.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("bridge.parquet");
    foreign_file(&path);

    let file = std::fs::File::open(&path).expect("open");
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).expect("builder");
    let schema = builder.cityparquet_arrow_schema().expect("render");
    let reader = cityparquet::reader::CityParquetRecordBatchReader::new(
        builder.build().expect("build"),
        schema,
    );

    for batch in reader {
        let batch = batch.expect("batch");
        let batch_schema = batch.schema();
        let names: Vec<&str> = batch_schema
            .fields()
            .iter()
            .map(|f| f.name().as_str())
            .collect();
        assert_eq!(
            names.iter().position(|n| *n == "children"),
            Some(3),
            "children stays where the file put it"
        );
        assert_eq!(
            names.iter().position(|n| *n == "parents"),
            Some(4),
            "parents stays where the file put it"
        );
    }
}
