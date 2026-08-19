//! A CityParquet file from a foreign writer must read. The physical conventions
//! here are duckdb-cityjson's: its reserved-column order, plain Utf8 for
//! `object_type`, `element` as the LIST child name, microsecond timestamps, and
//! no `address`/`template`/`other_attributes` columns.

use std::sync::Arc;

use arrow_array::{
    Array, ArrayRef, ListArray, RecordBatch, StringArray, TimestampMicrosecondArray,
};
use arrow_buffer::OffsetBuffer;
use arrow_schema::{DataType, Field, Schema, TimeUnit};
use parquet::arrow::ArrowWriter;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::file::properties::WriterProperties;

use cityparquet::reader::{CityParquetReaderBuilder, CityParquetRecordBatchReader};

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
    // Distinguishable, non-null values: a real transposition of these two
    // LIST<VARCHAR> columns must be visible in the DATA, not just provable by
    // reasoning about column order — a `new_null_array` here would make a
    // transposition value-invisible.
    let element_field: Arc<Field> = Field::new("element", DataType::Utf8, true).into();
    let children: ArrayRef = Arc::new(ListArray::new(
        Arc::clone(&element_field),
        OffsetBuffer::from_lengths([1usize]),
        Arc::new(StringArray::from(vec!["child-of-obj-1"])),
        None,
    ));
    let parents: ArrayRef = Arc::new(ListArray::new(
        Arc::clone(&element_field),
        OffsetBuffer::from_lengths([1usize]),
        Arc::new(StringArray::from(vec!["parent-of-obj-1"])),
        None,
    ));
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

    // The projection's entire purpose is carrying canonical METADATA onto the
    // file's own fields — a `children` field with no `cityparquet:role`
    // metadata would mean `project_metadata_onto` silently dropped it, and
    // this test would not otherwise catch that.
    assert_eq!(
        rendered
            .field_with_name("children")
            .unwrap()
            .metadata()
            .get("cityparquet:role")
            .map(String::as_str),
        Some("reserved"),
    );
}

/// The first element of a `LIST<VARCHAR>` cell (row 0), as an owned `String`.
fn first_string(array: &ArrayRef) -> String {
    let list = array
        .as_any()
        .downcast_ref::<ListArray>()
        .expect("a LIST<VARCHAR> array");
    let values = list.value(0);
    let strings = values
        .as_any()
        .downcast_ref::<StringArray>()
        .expect("a VARCHAR list element");
    strings.value(0).to_string()
}

#[test]
fn a_canonically_ordered_schema_does_not_transpose_the_hierarchy_columns() {
    // `parents` and `children` are both LIST<VARCHAR>. A caller holding the
    // spec's normative column order reading a file that orders them the other
    // way round is exactly the case a positional restamp corrupts silently.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bridge.parquet");
    foreign_file(&path);

    let builder =
        ParquetRecordBatchReaderBuilder::try_new(std::fs::File::open(&path).unwrap()).unwrap();
    let rendered = builder.cityparquet_arrow_schema().unwrap();

    // Same names, same DataTypes, spec order: parents before children.
    let canonical_order = Arc::new(Schema::new(vec![
        rendered.field(0).clone(),
        rendered.field(1).clone(),
        rendered.field(2).clone(),
        rendered.field(4).clone(), // parents
        rendered.field(3).clone(), // children
        rendered.field(5).clone(),
    ]));

    let reader = CityParquetRecordBatchReader::new(builder.build().unwrap(), canonical_order);
    for batch in reader {
        let batch = batch.expect("batch");
        let s = batch.schema();
        assert_eq!(s.field(3).name(), "children", "the file's order survives");
        assert_eq!(s.field(4).name(), "parents");
        assert_eq!(
            first_string(batch.column_by_name("children").unwrap()),
            "child-of-obj-1"
        );
        assert_eq!(
            first_string(batch.column_by_name("parents").unwrap()),
            "parent-of-obj-1"
        );
    }
}
