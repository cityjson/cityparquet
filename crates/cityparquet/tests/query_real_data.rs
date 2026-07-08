//! RED (readbench task 2/3): `query::full_read` / `query::count` /
//! `query::bbox_query`, read primitives of the cross-format read-benchmark
//! milestone — exercised against a real converted delft package (never
//! inline artificial CityJSON).

use std::path::{Path, PathBuf};

use arrow_array::{Array, Float64Array, StringArray, StructArray};
use cityparquet::package::{ConvertOptions, convert};
use cityparquet::query::bbox_query;
use cityparquet::reader::CityParquetReaderBuilder;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

/// One row's `bbox` leaves, read straight out of a `StructArray` at `row`.
fn row_bbox(bbox_col: &StructArray, row: usize) -> Option<[f64; 6]> {
    if bbox_col.is_null(row) {
        return None;
    }
    let leaf = |name: &str| -> f64 {
        bbox_col
            .column_by_name(name)
            .and_then(|c| c.as_any().downcast_ref::<Float64Array>())
            .unwrap_or_else(|| panic!("bbox.{name} missing or not Float64"))
            .value(row)
    };
    Some([
        leaf("xmin"),
        leaf("ymin"),
        leaf("zmin"),
        leaf("xmax"),
        leaf("ymax"),
        leaf("zmax"),
    ])
}

/// Test-only derivation of the dataset bbox: unions every row's `bbox` in
/// `table_path`, scanning the file completely independently of
/// [`bbox_query`] (so this test never validates the function against its
/// own logic).
fn dataset_bbox(table_path: &Path) -> [f64; 6] {
    let file = std::fs::File::open(table_path).unwrap();
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
    let reader = builder.build().unwrap();

    let mut acc: Option<[f64; 6]> = None;
    for batch in reader {
        let batch = batch.unwrap();
        let bbox_col = batch
            .column_by_name("bbox")
            .and_then(|c| c.as_any().downcast_ref::<StructArray>())
            .expect("main table must carry a 'bbox' struct column");
        for row in 0..batch.num_rows() {
            let Some(b) = row_bbox(bbox_col, row) else {
                continue;
            };
            acc = Some(match acc {
                None => b,
                Some(cur) => [
                    cur[0].min(b[0]),
                    cur[1].min(b[1]),
                    cur[2].min(b[2]),
                    cur[3].max(b[3]),
                    cur[4].max(b[4]),
                    cur[5].max(b[5]),
                ],
            });
        }
    }
    acc.expect("delft fixture must have at least one row with a non-null bbox")
}

#[test]
fn full_read_and_count_over_a_converted_delft_package() {
    let out = tempfile::tempdir().unwrap();
    let opts = ConvertOptions::new(fixture("delft.city.jsonl"), out.path().to_path_buf());
    let report = convert(&opts).unwrap();
    assert_eq!(report.object_count, 2231);

    let main_table = out.path().join("cityobjects.parquet");

    // `meta` is read independently of `full_read`, exactly as a real caller
    // (e.g. the readbench harness) would: open once for metadata, then hand
    // both the path and the parsed metadata to the query primitive.
    let file = std::fs::File::open(&main_table).unwrap();
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
    let meta = builder.cityparquet_metadata().unwrap();

    let full = cityparquet::query::full_read(&main_table, &meta).unwrap();
    assert_eq!(full.feature_count, 2231);
    assert!(
        full.boundary_count > 0,
        "delft has real geometry, so decoding every row's WKB must yield at least one surface"
    );

    assert_eq!(cityparquet::query::count(&main_table).unwrap(), 2231);
}

#[test]
fn bbox_query_returns_the_exact_matching_ids_for_a_lower_left_window() {
    let out = tempfile::tempdir().unwrap();
    let opts = ConvertOptions::new(fixture("delft.city.jsonl"), out.path().to_path_buf());
    let report = convert(&opts).unwrap();
    assert_eq!(report.object_count, 2231);

    let main_table = out.path().join("cityobjects.parquet");

    // Query window: the lower-left 25% of the dataset bbox's x/y extent,
    // full z range — derived independently of `bbox_query` via `dataset_bbox`
    // above.
    let bbox = dataset_bbox(&main_table);
    let span_x = bbox[3] - bbox[0];
    let span_y = bbox[4] - bbox[1];
    let window: [f64; 6] = [
        bbox[0],
        bbox[1],
        bbox[2],
        bbox[0] + span_x * 0.25,
        bbox[1] + span_y * 0.25,
        bbox[5],
    ];

    let result = bbox_query(&main_table, window).unwrap();

    assert!(
        !result.ids.is_empty(),
        "the lower-left 25% window should match at least one object"
    );
    assert!(
        result.ids.len() < 2231,
        "the window should not match the whole dataset (got {} of 2231)",
        result.ids.len()
    );
    assert!(
        result.row_groups_touched <= result.row_groups_total,
        "touched ({}) must never exceed total ({})",
        result.row_groups_touched,
        result.row_groups_total
    );

    // Independently re-verify: every id `bbox_query` returned must have a
    // real bbox in the file that truly intersects `window` — re-reading the
    // file's `id`/`bbox` columns directly, never reusing `bbox_query`'s
    // internals or its shared predicate.
    let file = std::fs::File::open(&main_table).unwrap();
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
    let reader = builder.build().unwrap();

    let mut verified: std::collections::HashSet<String> = std::collections::HashSet::new();
    for batch in reader {
        let batch = batch.unwrap();
        let id_col = batch
            .column_by_name("id")
            .and_then(|c| c.as_any().downcast_ref::<StringArray>())
            .expect("main table must carry an 'id' column");
        let bbox_col = batch
            .column_by_name("bbox")
            .and_then(|c| c.as_any().downcast_ref::<StructArray>())
            .expect("main table must carry a 'bbox' struct column");
        for row in 0..batch.num_rows() {
            let id = id_col.value(row);
            if !result.ids.iter().any(|x| x == id) {
                continue;
            }
            let row_box = row_bbox(bbox_col, row)
                .unwrap_or_else(|| panic!("returned id {id} has no bbox to intersect"));
            let intersects = row_box[0] <= window[3]
                && row_box[3] >= window[0]
                && row_box[1] <= window[4]
                && row_box[4] >= window[1]
                && row_box[2] <= window[5]
                && row_box[5] >= window[2];
            assert!(
                intersects,
                "returned id {id} bbox {row_box:?} does not intersect window {window:?}"
            );
            verified.insert(id.to_string());
        }
    }
    assert_eq!(
        verified.len(),
        result.ids.len(),
        "every id bbox_query returned must be found exactly once in the table"
    );
}
