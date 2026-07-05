//! RED (M3 task 4): the reader extension trait, exercised against a real
//! converted delft package — metadata/schema round-trip plus bbox row-group
//! pruning.

use std::path::PathBuf;

use arrow_schema::extension::EXTENSION_TYPE_NAME_KEY;
use cityparquet::package::{ConvertOptions, convert};
use cityparquet::reader::{CityParquetReaderBuilder, CityParquetRecordBatchReader};
use cityparquet::recipe::WriterRecipe;
use cityparquet::scan::scan;
use cityparquet::source::Source;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

/// Field metadata key CityParquet tags every reserved/attribute/extension
/// column with (`cityparquet_schema::model::ROLE_KEY`, not re-exported at the
/// crate root).
const ROLE_KEY: &str = "cityparquet:role";

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

/// Convert delft with a small row_group_size (256) so the file has enough
/// row groups for pruning to be observable, and return the output dir plus
/// the writer-side scan/metadata for spot-checking against the reader.
fn convert_delft_small_row_groups() -> tempfile::TempDir {
    let out = tempfile::tempdir().unwrap();
    let mut opts = ConvertOptions::new(fixture("delft.city.jsonl"), out.path().to_path_buf());
    opts.recipe = WriterRecipe {
        row_group_size: 256,
        ..WriterRecipe::default()
    };
    let report = convert(&opts).unwrap();
    assert_eq!(report.object_count, 2231);
    out
}

#[test]
fn cityparquet_metadata_matches_the_writer_side_scan() {
    let out = convert_delft_small_row_groups();

    // Writer side, independently: what the scan pass says the metadata is.
    let src = Source::open(&fixture("delft.city.jsonl")).unwrap();
    let scan_result = scan(&src).unwrap();
    let writer_meta = scan_result.metadata(&[]).unwrap();

    let file = std::fs::File::open(out.path().join("cityobjects.parquet")).unwrap();
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
    let read_meta = builder.cityparquet_metadata().unwrap();

    assert_eq!(read_meta.default_geometry, writer_meta.default_geometry);
    assert_eq!(read_meta.default_geometry, "geometry_lod2_2");
    assert_eq!(read_meta.attribute_columns.len(), 50);
    assert_eq!(
        read_meta.attribute_columns.len(),
        writer_meta.attribute_columns.len()
    );
    assert_eq!(read_meta.bbox_column, writer_meta.bbox_column);
    assert_eq!(read_meta.reserved_columns, writer_meta.reserved_columns);
    assert_eq!(
        read_meta.cityparquet_version,
        writer_meta.cityparquet_version
    );
}

#[test]
fn cityparquet_arrow_schema_matches_the_writers_rendered_schema() {
    let out = convert_delft_small_row_groups();

    let src = Source::open(&fixture("delft.city.jsonl")).unwrap();
    let scan_result = scan(&src).unwrap();
    let writer_schema = scan_result.schema.to_arrow_schema().unwrap();

    let file = std::fs::File::open(out.path().join("cityobjects.parquet")).unwrap();
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
    let read_schema = builder.cityparquet_arrow_schema().unwrap();

    let writer_names: Vec<&str> = writer_schema
        .fields()
        .iter()
        .map(|f| f.name().as_str())
        .collect();
    let read_names: Vec<&str> = read_schema
        .fields()
        .iter()
        .map(|f| f.name().as_str())
        .collect();
    assert_eq!(read_names, writer_names);

    for (writer_field, read_field) in writer_schema
        .fields()
        .iter()
        .zip(read_schema.fields().iter())
    {
        assert_eq!(
            writer_field.data_type(),
            read_field.data_type(),
            "field {} data type mismatch",
            writer_field.name()
        );
    }

    // Extension/role metadata present on geometry and JSON columns.
    let geom = read_schema.field_with_name("geometry_lod2_2").unwrap();
    assert_eq!(
        geom.metadata()
            .get(EXTENSION_TYPE_NAME_KEY)
            .map(String::as_str),
        Some("geoarrow.wkb")
    );
    assert_eq!(
        geom.metadata().get(ROLE_KEY).map(String::as_str),
        Some("reserved")
    );

    let props = read_schema
        .field_with_name("geometry_properties_lod2_2")
        .unwrap();
    assert_eq!(
        props
            .metadata()
            .get(EXTENSION_TYPE_NAME_KEY)
            .map(String::as_str),
        Some("arrow.json")
    );

    // An inferred attribute keeps its role metadata too.
    let some_attr = read_schema
        .fields()
        .iter()
        .find(|f| f.metadata().get(ROLE_KEY).map(String::as_str) == Some("attribute"))
        .expect("at least one attribute column");
    assert_eq!(
        some_attr.metadata().get(ROLE_KEY).map(String::as_str),
        Some("attribute")
    );
}

#[test]
fn record_batch_reader_yields_all_rows_with_schema_metadata_preserved() {
    let out = convert_delft_small_row_groups();

    let file = std::fs::File::open(out.path().join("cityobjects.parquet")).unwrap();
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
    let rendered_schema = builder.cityparquet_arrow_schema().unwrap();
    let parquet_reader = builder.build().unwrap();
    let reader = CityParquetRecordBatchReader::new(parquet_reader, rendered_schema.clone());

    assert_eq!(
        reader
            .schema()
            .field_with_name("geometry_lod2_2")
            .unwrap()
            .metadata()
            .get(EXTENSION_TYPE_NAME_KEY)
            .map(String::as_str),
        Some("geoarrow.wkb")
    );

    let mut total_rows = 0usize;
    for batch in reader {
        let batch = batch.unwrap();
        assert_eq!(
            batch
                .schema()
                .field_with_name("geometry_lod2_2")
                .unwrap()
                .metadata()
                .get(EXTENSION_TYPE_NAME_KEY)
                .map(String::as_str),
            Some("geoarrow.wkb"),
            "every emitted batch must carry the rendered schema's field metadata"
        );
        total_rows += batch.num_rows();
    }
    assert_eq!(total_rows, 2231);
}

#[test]
fn with_bbox_row_groups_prunes_a_tight_corner_query_but_keeps_the_whole_extent() {
    let out = convert_delft_small_row_groups();

    let src = Source::open(&fixture("delft.city.jsonl")).unwrap();
    let scan_result = scan(&src).unwrap();
    let dataset_bbox = scan_result
        .dataset_bbox
        .expect("delft has geometry, so a dataset bbox");

    let file = std::fs::File::open(out.path().join("cityobjects.parquet")).unwrap();
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
    let num_row_groups = builder.metadata().num_row_groups();
    assert!(
        num_row_groups > 1,
        "expected multiple row groups at row_group_size 256 for 2231 rows, got {num_row_groups}"
    );

    // A tiny box pinned to the dataset's minimum corner: strictly smaller
    // than the whole extent on every axis, so most row groups should not
    // intersect it.
    let span = [
        dataset_bbox[3] - dataset_bbox[0],
        dataset_bbox[4] - dataset_bbox[1],
        dataset_bbox[5] - dataset_bbox[2],
    ];
    let corner_bbox: [f64; 6] = [
        dataset_bbox[0],
        dataset_bbox[1],
        dataset_bbox[2],
        dataset_bbox[0] + span[0] * 0.01,
        dataset_bbox[1] + span[1] * 0.01,
        dataset_bbox[2] + span[2] * 0.01,
    ];

    let corner_builder = builder.with_bbox_row_groups(corner_bbox).unwrap();
    let corner_rows: usize = corner_builder
        .build()
        .unwrap()
        .map(|b| b.unwrap().num_rows())
        .sum();
    assert!(
        corner_rows < 2231,
        "corner bbox query should prune at least one row group, got {corner_rows} rows"
    );

    let file2 = std::fs::File::open(out.path().join("cityobjects.parquet")).unwrap();
    let builder2 = ParquetRecordBatchReaderBuilder::try_new(file2).unwrap();
    let whole_builder = builder2.with_bbox_row_groups(dataset_bbox).unwrap();
    let whole_rows: usize = whole_builder
        .build()
        .unwrap()
        .map(|b| b.unwrap().num_rows())
        .sum();
    assert_eq!(
        whole_rows, 2231,
        "a bbox covering the whole dataset extent must keep every row"
    );
}
