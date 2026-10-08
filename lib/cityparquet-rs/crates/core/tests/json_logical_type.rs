//! The spec types the object-table `other` column, `geometry_properties_lod*`'s
//! `surfaces` field and the sidecars' `other` columns as `JSON` — a Parquet
//! logical type, since CityParquet "is defined at the level of Parquet logical
//! types" (spec "Physical encoding and conformance"). The writer must emit
//! that logical type; a reader must still accept the same columns written as
//! plain UTF8 by a writer that does not annotate them.

mod support;

use std::path::Path;
use std::sync::Arc;

use arrow_array::{RecordBatch, make_array};
use arrow_schema::{DataType, Field, Fields, Schema};
use cityparquet::compare::{CompareOptions, compare_datasets};
use cityparquet::export::{ExportOptions, export};
use parquet::basic::LogicalType;
use parquet::file::reader::{FileReader, SerializedFileReader};

use support::{FileContent, convert_fixture, fixture};

/// `(column path, logical type)` of every leaf of `path` whose spec type is
/// `JSON`: a top-level `other`, or the `surfaces` field of a
/// `geometry_properties_lod*` struct.
fn json_leaves(path: &Path) -> Vec<(String, Option<LogicalType>)> {
    let reader = SerializedFileReader::new(std::fs::File::open(path).unwrap()).unwrap();
    let descr = reader.metadata().file_metadata().schema_descr_ptr();
    (0..descr.num_columns())
        .map(|i| descr.column(i))
        .filter(|col| {
            let parts = col.path().parts();
            matches!(parts, [only] if only == "other")
                || matches!(parts, [root, leaf]
                    if root.starts_with("geometry_properties_lod") && leaf == "surfaces")
        })
        .map(|col| (col.path().string(), col.logical_type_ref().cloned()))
        .collect()
}

#[test]
fn every_column_the_spec_types_json_is_written_as_the_parquet_json_logical_type() {
    let mut files_checked = 0;
    for name in ["lod3_railway.city.json", "delft.city.jsonl"] {
        let (_dir, pkg) = convert_fixture(name);
        for entry in std::fs::read_dir(&pkg).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) != Some("parquet") {
                continue;
            }
            let leaves = json_leaves(&path);
            assert!(
                !leaves.is_empty(),
                "{}: every CityParquet file has at least one JSON-typed column",
                path.display()
            );
            for (column, logical) in leaves {
                assert_eq!(
                    logical,
                    Some(LogicalType::Json),
                    "{name} {}: `{column}` must carry the JSON logical type",
                    path.file_name().unwrap().to_string_lossy()
                );
            }
            files_checked += 1;
        }
    }
    // railway: nine object tables + materials, textures and
    // implicit_geometries; delft: building.
    assert_eq!(files_checked, 13);
}

/// `field` with any `arrow.json` extension tag removed, recursively through
/// struct children — the schema of a writer that stores JSON as plain UTF8.
fn untag(field: &Field) -> Field {
    let data_type = match field.data_type() {
        DataType::Struct(children) => DataType::Struct(Fields::from(
            children.iter().map(|c| untag(c)).collect::<Vec<_>>(),
        )),
        other => other.clone(),
    };
    let mut metadata = field.metadata().clone();
    metadata.remove("ARROW:extension:name");
    metadata.remove("ARROW:extension:metadata");
    Field::new(field.name(), data_type, field.is_nullable()).with_metadata(metadata)
}

fn as_plain_utf8(content: &mut FileContent) {
    let fields: Vec<Field> = content.schema.fields().iter().map(|f| untag(f)).collect();
    let schema = Arc::new(Schema::new(fields));
    content.batches = content
        .batches
        .iter()
        .map(|batch| {
            let columns = batch
                .columns()
                .iter()
                .zip(schema.fields())
                .map(|(column, field)| {
                    let data = column
                        .to_data()
                        .into_builder()
                        .data_type(field.data_type().clone())
                        .build()
                        .unwrap();
                    make_array(data)
                })
                .collect();
            RecordBatch::try_new(schema.clone(), columns).unwrap()
        })
        .collect();
    content.schema = schema;
}

#[test]
fn json_columns_written_as_plain_utf8_by_another_writer_read_back_the_same_model() {
    let (dir, pkg) = convert_fixture("delft.city.jsonl");
    let table = pkg.join("building.parquet");
    let mut content = FileContent::read(&table);
    as_plain_utf8(&mut content);
    content.write(&table);

    // The rewrite really is plain UTF8 — otherwise this test proves nothing.
    let leaves = json_leaves(&table);
    assert!(
        leaves
            .iter()
            .any(|(column, _)| column.starts_with("geometry_properties_lod")),
        "delft carries semantic surfaces"
    );
    for (column, logical) in leaves {
        assert_eq!(logical, Some(LogicalType::String), "{column}");
    }

    let exported = dir.path().join("delft.city.jsonl");
    export(&ExportOptions {
        package_dir: pkg,
        output: exported.clone(),
    })
    .expect("a plain-UTF8 JSON column must read");
    let report = compare_datasets(
        &fixture("delft.city.jsonl"),
        &exported,
        &CompareOptions::default(),
    )
    .unwrap();
    assert!(report.equal, "{:#?}", report.differences);
}
