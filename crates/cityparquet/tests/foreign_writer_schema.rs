//! A CityParquet file from a foreign writer must read. The physical conventions
//! here are duckdb-cityjson's: its reserved-column order, plain Utf8 for
//! `object_type`, `element` as the LIST child name, and microsecond
//! timestamps. `address`/`template`/`children_roles`/`other` are present but
//! all-null on every row — `decode_batch` requires the columns to exist, so
//! this exercises them as absent-in-effect without pretending a foreign
//! writer would physically omit them.

use std::sync::Arc;

use arrow_array::types::Int32Type;
use arrow_array::{
    Array, ArrayRef, DictionaryArray, ListArray, RecordBatch, StringArray,
    TimestampMicrosecondArray, new_null_array,
};
use arrow_buffer::OffsetBuffer;
use arrow_schema::{DataType, Field, Schema, TimeUnit};
use parquet::arrow::ArrowWriter;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::file::properties::WriterProperties;

use cityparquet::reader::{CityParquetReaderBuilder, CityParquetRecordBatchReader};
use cityparquet_schema::model::{address_data_type, template_data_type};

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
        // `decode_batch` errors on any of these being absent, so they must
        // be present — all-null is enough to exercise the object-type/
        // timestamp tolerance this file tests without needing real address/
        // template/other data.
        Field::new(
            "children_roles",
            DataType::List(Field::new("item", DataType::Utf8, true).into()),
            true,
        ),
        Field::new("address", address_data_type(), true),
        Field::new("template", template_data_type(), true),
        Field::new("other", DataType::Utf8, true),
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
    let children_roles: ArrayRef = new_null_array(
        &DataType::List(Field::new("item", DataType::Utf8, true).into()),
        1,
    );
    let address: ArrayRef = new_null_array(&address_data_type(), 1);
    let template: ArrayRef = new_null_array(&template_data_type(), 1);
    let other: ArrayRef = new_null_array(&DataType::Utf8, 1);

    let batch = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![
            ids,
            feature_ids,
            types,
            children,
            parents,
            stamps,
            children_roles,
            address,
            template,
            other,
        ],
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
            "tijdstipregistratie",
            "children_roles",
            "address",
            "template",
            "other",
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

#[test]
fn a_foreign_writers_values_decode() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("bridge.parquet");
    foreign_file(&path);

    let file = std::fs::File::open(&path).expect("open");
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).expect("builder");
    let meta = builder.cityparquet_metadata().expect("metadata");
    let schema = builder.cityparquet_arrow_schema().expect("render");
    let reader = cityparquet::reader::CityParquetRecordBatchReader::new(
        builder.build().expect("build"),
        schema,
    );

    // Decoding is where the canonical-type downcasts live: a plain Utf8
    // `object_type` and a MICROS timestamp both render fine and then fail here.
    let mut objects = Vec::new();
    for batch in reader {
        let batch = batch.expect("batch");
        objects.extend(
            cityparquet::decode::decode_batch(&batch, &meta)
                .expect("a foreign writer's values must decode"),
        );
    }
    assert_eq!(objects.len(), 1);
    assert_eq!(objects[0].object.thetype, "Bridge");
    // The DECODED VALUE, not just that decoding didn't error: a unit-
    // confusion bug (a MICROS raw value read through the MILLIS divisor)
    // would still decode without error, just to the wrong instant.
    // 1_600_000_000_000_000 microseconds since the epoch is
    // 2020-09-13T12:26:40Z.
    assert_eq!(
        objects[0]
            .object
            .attributes
            .as_ref()
            .and_then(|a| a.get("tijdstipregistratie"))
            .and_then(|v| v.as_str()),
        Some("2020-09-13T12:26:40.000Z"),
        "a MICROS timestamp must decode through the MICROS divisor, not the MILLIS one"
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

/// A minimal, conformant `city` footer with no attributes at all — pairs
/// with [`minimal_foreign_file`], which carries no attribute columns.
const MINIMAL_CITY_JSON: &str = r#"{
  "version": "0.1.0-draft",
  "attributes": []
}"#;

/// What duckdb-cityjson actually writes: only the non-null reserved
/// columns (`id`, `feature_id`, `object_type`, `parents`, `children`) —
/// `children_roles`, `address`, `template`, and `other` are omitted
/// OUTRIGHT, not present-and-null. Kept separate from [`foreign_file`]
/// rather than folded into it — three existing tests depend on that
/// helper's exact shape.
fn minimal_foreign_file(path: &std::path::Path) {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("feature_id", DataType::Utf8, false),
        // Plain Utf8, matching duckdb-cityjson's own convention.
        Field::new("object_type", DataType::Utf8, false),
        Field::new(
            "parents",
            DataType::List(Field::new("item", DataType::Utf8, true).into()),
            true,
        ),
        Field::new(
            "children",
            DataType::List(Field::new("item", DataType::Utf8, true).into()),
            true,
        ),
    ]));

    let ids: ArrayRef = Arc::new(StringArray::from(vec!["obj-1"]));
    let feature_ids: ArrayRef = Arc::new(StringArray::from(vec!["obj-1"]));
    let types: ArrayRef = Arc::new(StringArray::from(vec!["Bridge"]));
    // A standalone object: no parents, no children — both cells null.
    let parents: ArrayRef = new_null_array(
        &DataType::List(Field::new("item", DataType::Utf8, true).into()),
        1,
    );
    let children: ArrayRef = new_null_array(
        &DataType::List(Field::new("item", DataType::Utf8, true).into()),
        1,
    );

    let batch = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![ids, feature_ids, types, parents, children],
    )
    .expect("minimal foreign batch");

    let props = WriterProperties::builder()
        .set_key_value_metadata(Some(vec![parquet::file::metadata::KeyValue::new(
            "city".to_string(),
            MINIMAL_CITY_JSON.to_string(),
        )]))
        .build();
    let file = std::fs::File::create(path).expect("create");
    let mut writer = ArrowWriter::try_new(file, schema, Some(props)).expect("writer");
    writer.write(&batch).expect("write");
    writer.close().expect("close");
}

#[test]
fn a_table_omitting_the_optional_reserved_columns_decodes() {
    // duckdb-cityjson emits none of address/template/other, and the spec
    // permits an absent `other` outright. An absent nullable column and an
    // all-null one carry the same information, so the reader treats the
    // first as the second rather than refusing the file.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("bridge.parquet");
    minimal_foreign_file(&path);

    let file = std::fs::File::open(&path).expect("open");
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).expect("builder");
    let meta = builder.cityparquet_metadata().expect("metadata");
    let schema = builder.cityparquet_arrow_schema().expect("render");
    let reader = cityparquet::reader::CityParquetRecordBatchReader::new(
        builder.build().expect("build"),
        schema,
    );

    let mut objects = Vec::new();
    for batch in reader {
        let batch = batch.expect("batch");
        objects.extend(
            cityparquet::decode::decode_batch(&batch, &meta)
                .expect("a table without the optional reserved columns must decode"),
        );
    }
    assert_eq!(objects.len(), 1);
    assert_eq!(objects[0].object.thetype, "Bridge");
    assert!(
        objects[0].address.is_none(),
        "an absent column decodes as an absent address, not Some(vec![])"
    );
    assert!(
        objects[0].template.is_none(),
        "an absent template column decodes as no template instance, not a spuriously \
         populated one"
    );
}

/// A minimal, conformant `city` footer with no `columns` entry at all — the
/// case `cityparquet_arrow_schema`'s TAIL projection (the geometry-bearing
/// exit, taken whenever `lods` is non-empty) must handle: every real
/// CityParquet file takes this path, yet [`foreign_file`]/[`minimal_foreign_file`]
/// above carry no geometry column, so neither exercises it — only the EARLY
/// return (the geometry-less table shape). `encoding_from_physical_shape`
/// falls back `Binary -> Wkb` when a footer declares no `city.columns` entry
/// for a column, so this fixture needs no encoding-token gymnastics.
const GEOMETRY_CITY_JSON: &str = r#"{
  "version": "0.1.0-draft",
  "attributes": []
}"#;

/// A geometry-bearing foreign file: `geometry_lod1_0` (`Binary`, WKB by the
/// footer-less fallback) plus its `geometry_properties_lod1_0` pair, both
/// all-null — this test only calls `cityparquet_arrow_schema`, never
/// `decode_batch`, so no real WKB/struct payload is needed, only the right
/// physical shape.
fn geometry_bearing_foreign_file(path: &std::path::Path) {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("feature_id", DataType::Utf8, false),
        Field::new("object_type", DataType::Utf8, false),
        Field::new(
            "parents",
            DataType::List(Field::new("item", DataType::Utf8, true).into()),
            true,
        ),
        Field::new(
            "children",
            DataType::List(Field::new("item", DataType::Utf8, true).into()),
            true,
        ),
        Field::new("geometry_lod1_0", DataType::Binary, true),
        Field::new(
            "geometry_properties_lod1_0",
            cityparquet_schema::model::geometry_properties_data_type(),
            true,
        ),
    ]));

    let ids: ArrayRef = Arc::new(StringArray::from(vec!["obj-1"]));
    let feature_ids: ArrayRef = Arc::new(StringArray::from(vec!["obj-1"]));
    let types: ArrayRef = Arc::new(StringArray::from(vec!["Bridge"]));
    let parents: ArrayRef = new_null_array(
        &DataType::List(Field::new("item", DataType::Utf8, true).into()),
        1,
    );
    let children: ArrayRef = new_null_array(
        &DataType::List(Field::new("item", DataType::Utf8, true).into()),
        1,
    );
    let geometry: ArrayRef = new_null_array(&DataType::Binary, 1);
    let geometry_properties: ArrayRef = new_null_array(
        &cityparquet_schema::model::geometry_properties_data_type(),
        1,
    );

    let batch = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![
            ids,
            feature_ids,
            types,
            parents,
            children,
            geometry,
            geometry_properties,
        ],
    )
    .expect("geometry-bearing foreign batch");

    let props = WriterProperties::builder()
        .set_key_value_metadata(Some(vec![parquet::file::metadata::KeyValue::new(
            "city".to_string(),
            GEOMETRY_CITY_JSON.to_string(),
        )]))
        .build();
    let file = std::fs::File::create(path).expect("create");
    let mut writer = ArrowWriter::try_new(file, schema, Some(props)).expect("writer");
    writer.write(&batch).expect("write");
    writer.close().expect("close");
}

/// The tail projection path (`cityparquet_arrow_schema`'s second, geometry-
/// bearing exit) must carry the CANONICAL schema's field metadata onto the
/// file's OWN fields, exactly like the early-return path already proven by
/// [`renders_the_foreign_writers_own_fields_not_the_canonical_set`]. Reverting
/// only the tail's `project_metadata_onto` call (returning the bare
/// `canonical` schema, or the bare `actual` schema) breaks NO other test in
/// this crate — the reviewed gap this test closes.
#[test]
fn tail_projection_carries_canonical_metadata_onto_a_geometry_bearing_foreign_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("geometry_bridge.parquet");
    geometry_bearing_foreign_file(&path);

    let file = std::fs::File::open(&path).expect("open");
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).expect("builder");
    let rendered = builder
        .cityparquet_arrow_schema()
        .expect("a conformant geometry-bearing foreign file must render");

    // File identity preserved: field order is the file's own, and
    // `object_type` stays the file's plain Utf8, not the canonical
    // Dictionary — proves this is not the bare canonical schema.
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
            "parents",
            "children",
            "geometry_lod1_0",
            "geometry_properties_lod1_0",
        ],
        "the rendered schema must be the file's own fields, in the file's order"
    );
    assert_eq!(
        rendered.field_with_name("object_type").unwrap().data_type(),
        &DataType::Utf8,
        "physical types are the file's own, not the canonical Dictionary choice"
    );

    // Canonical metadata carried onto the file's own fields: this fixture's
    // fields are built with plain `Field::new` (no metadata at all), so
    // `geoarrow.wkb` / `cityparquet:role` / `cityparquet:lod` can only have
    // arrived via `project_metadata_onto`.
    let geom = rendered.field_with_name("geometry_lod1_0").unwrap();
    assert_eq!(
        geom.metadata()
            .get(arrow_schema::extension::EXTENSION_TYPE_NAME_KEY)
            .map(String::as_str),
        Some("geoarrow.wkb"),
        "the tail must tag the geometry column geoarrow.wkb, same as the early-return path"
    );
    assert_eq!(
        geom.metadata().get("cityparquet:role").map(String::as_str),
        Some("reserved")
    );
    assert_eq!(
        geom.metadata().get("cityparquet:lod").map(String::as_str),
        Some("1.0")
    );
}

/// A minimal, conformant `city` footer with one dictionary-encoded VARCHAR
/// attribute.
const DICT_ATTR_CITY_JSON: &str = r#"{
  "version": "0.1.0-draft",
  "attributes": ["street_name"]
}"#;

/// An attribute column dictionary-encoded `Dictionary(Int32, Utf8)` — a
/// writer's physical-encoding choice (spec "Physical encoding and
/// conformance": "a reader MUST NOT require a particular in-memory
/// representation of a VARCHAR column, dictionary-encoded or not"). This
/// crate's own writer never dictionary-encodes an ATTRIBUTE column (only
/// `object_type`), so this shape only arises from a foreign writer.
fn dictionary_attribute_foreign_file(path: &std::path::Path) {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("feature_id", DataType::Utf8, false),
        Field::new("object_type", DataType::Utf8, false),
        Field::new(
            "street_name",
            DataType::Dictionary(Box::new(DataType::Int32), Box::new(DataType::Utf8)),
            true,
        ),
    ]));

    let ids: ArrayRef = Arc::new(StringArray::from(vec!["obj-1"]));
    let feature_ids: ArrayRef = Arc::new(StringArray::from(vec!["obj-1"]));
    let types: ArrayRef = Arc::new(StringArray::from(vec!["Bridge"]));
    let street_name: ArrayRef = Arc::new(DictionaryArray::<Int32Type>::from_iter(vec![Some(
        "Main St",
    )]));

    let batch = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![ids, feature_ids, types, street_name],
    )
    .expect("dictionary-attribute foreign batch");

    let props = WriterProperties::builder()
        .set_key_value_metadata(Some(vec![parquet::file::metadata::KeyValue::new(
            "city".to_string(),
            DICT_ATTR_CITY_JSON.to_string(),
        )]))
        .build();
    let file = std::fs::File::create(path).expect("create");
    let mut writer = ArrowWriter::try_new(file, schema, Some(props)).expect("writer");
    writer.write(&batch).expect("write");
    writer.close().expect("close");
}

#[test]
fn a_dictionary_encoded_varchar_attribute_column_is_not_rejected() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("dict_attr.parquet");
    dictionary_attribute_foreign_file(&path);

    let file = std::fs::File::open(&path).expect("open");
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).expect("builder");
    let meta = builder.cityparquet_metadata().expect("metadata");
    let schema = builder
        .cityparquet_arrow_schema()
        .expect("a Dictionary(Int32, Utf8) attribute column must not be rejected outright");
    let reader = CityParquetRecordBatchReader::new(builder.build().expect("build"), schema);

    let mut objects = Vec::new();
    for batch in reader {
        let batch = batch.expect("batch");
        objects.extend(
            cityparquet::decode::decode_batch(&batch, &meta)
                .expect("a dictionary-encoded attribute must decode"),
        );
    }
    assert_eq!(objects.len(), 1);
    assert_eq!(
        objects[0]
            .object
            .attributes
            .as_ref()
            .and_then(|a| a.get("street_name"))
            .and_then(|v| v.as_str()),
        Some("Main St")
    );
}
