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

/// This row group's `bbox.<leaf>` Double statistic: `min` when `want_min`,
/// else `max`. Panics if the chunk or statistic is missing — the recipe
/// guarantees chunk statistics on every bbox leaf, so absence here is a
/// writer regression the test should surface, not silently keep.
fn bbox_stat(rg: &parquet::file::metadata::RowGroupMetaData, leaf: &str, want_min: bool) -> f64 {
    use parquet::file::statistics::Statistics;
    use parquet::schema::types::ColumnPath;
    let path = ColumnPath::new(vec!["bbox".to_string(), leaf.to_string()]);
    let stats = rg
        .columns()
        .iter()
        .find(|c| c.column_path() == &path)
        .unwrap_or_else(|| panic!("no column chunk for bbox.{leaf}"))
        .statistics()
        .unwrap_or_else(|| panic!("no statistics on bbox.{leaf}"));
    let Statistics::Double(v) = stats else {
        panic!("bbox.{leaf} statistics are not Double: {stats:?}");
    };
    let value = if want_min { v.min_opt() } else { v.max_opt() };
    *value.unwrap_or_else(|| {
        panic!(
            "no {} value on bbox.{leaf}",
            if want_min { "min" } else { "max" }
        )
    })
}

#[test]
fn with_bbox_row_groups_selects_a_strict_subset_for_a_partial_band_query() {
    // The corner/whole-extent test above only pins the extremes (0 kept /
    // all kept); a contains-vs-intersects or axis-swap regression could slip
    // through it. This test derives — from the file's own row-group
    // statistics, never hardcoded coordinates — a y-band that the stats say
    // intersects SOME but not ALL row groups, and asserts the reader keeps
    // exactly the rows of that strict subset.
    let out = convert_delft_small_row_groups();

    let src = Source::open(&fixture("delft.city.jsonl")).unwrap();
    let scan_result = scan(&src).unwrap();
    let dataset_bbox = scan_result.dataset_bbox.unwrap();

    let file = std::fs::File::open(out.path().join("cityobjects.parquet")).unwrap();
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
    let metadata = builder.metadata().clone();
    let num_row_groups = metadata.num_row_groups();
    assert!(
        num_row_groups > 1,
        "need multiple row groups to discriminate"
    );

    // Each row group's y interval per its own statistics.
    let y_intervals: Vec<(f64, f64)> = (0..num_row_groups)
        .map(|i| {
            let rg = metadata.row_group(i);
            (bbox_stat(rg, "ymin", true), bbox_stat(rg, "ymax", false))
        })
        .collect();

    // Band = [global y-min, halfway to the second-lowest row-group ymin]:
    // by construction it intersects the group(s) starting at the global
    // minimum and, per the stats, excludes every group starting above the
    // midpoint. Purely derived from the file, so it survives fixture and
    // layout changes as long as the groups do not all share one ymin.
    let mut starts: Vec<f64> = y_intervals.iter().map(|(lo, _)| *lo).collect();
    starts.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let global_lo = starts[0];
    let next_lo = starts
        .iter()
        .copied()
        .find(|&lo| lo > global_lo)
        .expect("all row groups share the same bbox.ymin; no y-band can discriminate — fixture/layout no longer supports this test");
    let band_hi = global_lo + (next_lo - global_lo) / 2.0;
    let band: [f64; 6] = [
        dataset_bbox[0],
        global_lo,
        dataset_bbox[2],
        dataset_bbox[3],
        band_hi,
        dataset_bbox[5],
    ];

    // What the stats say the band should keep.
    let expected_kept: Vec<usize> = (0..num_row_groups)
        .filter(|&i| {
            let (lo, hi) = y_intervals[i];
            hi >= band[1] && lo <= band[4]
        })
        .collect();
    let expected_rows: usize = expected_kept
        .iter()
        .map(|&i| metadata.row_group(i).num_rows() as usize)
        .sum();
    assert!(
        !expected_kept.is_empty() && expected_kept.len() < num_row_groups,
        "derived band must select a strict, non-empty subset per the stats; \
         got {}/{num_row_groups} groups — fixture/layout no longer supports this test",
        expected_kept.len()
    );
    eprintln!(
        "partial y-band [{}, {}] keeps groups {expected_kept:?} of {num_row_groups} ({expected_rows} rows)",
        band[1], band[4]
    );

    let band_builder = builder.with_bbox_row_groups(band).unwrap();
    let band_rows: usize = band_builder
        .build()
        .unwrap()
        .map(|b| b.unwrap().num_rows())
        .sum();
    assert!(
        band_rows > 0 && band_rows < 2231,
        "partial band should keep some but not all rows, got {band_rows}"
    );
    assert_eq!(
        band_rows, expected_rows,
        "reader kept different row groups than the statistics predict \
         (expected groups {expected_kept:?} = {expected_rows} rows)"
    );
}
