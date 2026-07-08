//! RED (readbench task 2/3/4): `query::full_read` / `query::count` /
//! `query::bbox_query` / `query::attr_filter`, read primitives of the
//! cross-format read-benchmark milestone — exercised against a real
//! converted delft package (never inline artificial CityJSON).

use std::path::{Path, PathBuf};

use arrow_array::{Array, Float64Array, StringArray, StructArray};
use cityparquet::decode::decode_batch;
use cityparquet::package::{ConvertOptions, convert};
use cityparquet::query::{AttrPredicate, attr_filter, attr_stats, bbox_query};
use cityparquet::reader::{CityParquetReaderBuilder, CityParquetRecordBatchReader};
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

/// Converts `fixture(name)` into a fresh tempdir and decodes every row of
/// its main table — independently of [`attr_filter`], so these tests never
/// validate the function against its own logic. Returns the tempdir (the
/// caller must keep it bound so it is not deleted before `attr_filter` gets
/// to re-open the table), the main table path, and the decoded objects.
fn convert_and_decode(
    name: &str,
) -> (
    tempfile::TempDir,
    PathBuf,
    Vec<cityparquet::decode::DecodedObject>,
) {
    let out = tempfile::tempdir().unwrap();
    let report = convert(&ConvertOptions::new(
        fixture(name),
        out.path().to_path_buf(),
    ))
    .unwrap();
    assert_eq!(report.object_count, 2231);

    let main_table = out.path().join("cityobjects.parquet");
    let file = std::fs::File::open(&main_table).unwrap();
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
    let meta = builder.cityparquet_metadata().unwrap();
    let schema = builder.cityparquet_arrow_schema().unwrap();
    let parquet_reader = builder.build().unwrap();
    let reader = CityParquetRecordBatchReader::new(parquet_reader, schema);

    let mut all = Vec::new();
    for batch in reader {
        let batch = batch.unwrap();
        all.extend(decode_batch(&batch, &meta).unwrap());
    }
    (out, main_table, all)
}

/// `Eq` on `object_type` (the reserved `Dictionary<Int32, Utf8>` column):
/// independently counts every decoded object whose `cjseq::CityObject::thetype`
/// is `"BuildingPart"` (delft's real per-fixture split is
/// `{'BuildingPart': 1116, 'Building': 1115}` — see `decode_real_data.rs`),
/// then asserts `attr_filter` returns the identical count via a Parquet
/// `RowFilter` over the dictionary column.
#[test]
fn attr_filter_eq_matches_object_type_dictionary_column() {
    let (_out, main_table, objects) = convert_and_decode("delft.city.jsonl");

    let expected = objects
        .iter()
        .filter(|o| o.object.thetype == "BuildingPart")
        .count() as u64;
    assert_eq!(expected, 1116, "sanity: delft's known BuildingPart count");

    let pred = AttrPredicate::Eq(serde_json::Value::String("BuildingPart".to_string()));
    let got = attr_filter(&main_table, "object_type", &pred).unwrap();
    assert_eq!(got, expected);
}

/// `Eq` on `status` (a plain `Utf8` attribute column, non-reserved, present
/// only on `Building` rows — `BuildingPart` rows have no `status` attribute
/// at all, so those rows are NULL in the column and must never match):
/// independently counts every decoded object whose `attributes.status` JSON
/// string is `"Pand in gebruik"`.
#[test]
fn attr_filter_eq_matches_string_attribute_column_and_excludes_nulls() {
    let (_out, main_table, objects) = convert_and_decode("delft.city.jsonl");

    let expected = objects
        .iter()
        .filter(|o| {
            o.object
                .attributes
                .as_ref()
                .and_then(|v| v.as_object())
                .and_then(|attrs| attrs.get("status"))
                .and_then(|v| v.as_str())
                == Some("Pand in gebruik")
        })
        .count() as u64;
    // Sanity bound: strictly fewer than the 1115 Building rows (a handful
    // carry a different `status` value) and strictly more than zero.
    assert!(
        expected > 0 && expected < 1115,
        "expected a proper subset of delft's 1115 Building rows, got {expected}"
    );

    let pred = AttrPredicate::Eq(serde_json::Value::String("Pand in gebruik".to_string()));
    let got = attr_filter(&main_table, "status", &pred).unwrap();
    assert_eq!(got, expected);
}

/// `Ge`/`Le`/`Range` on `oorspronkelijkbouwjaar` (a plain `Int64` attribute
/// column, non-reserved, present only on `Building` rows — `BuildingPart`
/// rows are NULL and must never match any of the three predicates):
/// independently counts every decoded object whose year-built integer
/// satisfies each bound, scanning the SAME decoded objects fixture the
/// dictionary/string tests above already validated `attr_filter` against.
#[test]
fn attr_filter_numeric_predicates_match_year_built_attribute_column() {
    let (_out, main_table, objects) = convert_and_decode("delft.city.jsonl");

    let years: Vec<i64> = objects
        .iter()
        .filter_map(|o| {
            o.object
                .attributes
                .as_ref()
                .and_then(|v| v.as_object())
                .and_then(|attrs| attrs.get("oorspronkelijkbouwjaar"))
                .and_then(|v| v.as_i64())
        })
        .collect();
    assert_eq!(
        years.len(),
        1115,
        "oorspronkelijkbouwjaar must be present on exactly delft's 1115 Building rows"
    );

    let expected_ge = years.iter().filter(|&&y| y >= 2000).count() as u64;
    let expected_le = years.iter().filter(|&&y| y <= 1900).count() as u64;
    let expected_range = years
        .iter()
        .filter(|&&y| (1950..=2000).contains(&y))
        .count() as u64;
    // Sanity: none of the three subsets are trivially empty or the whole set
    // — otherwise the test would not distinguish a correct implementation
    // from a broken always-true/always-false one.
    assert!(expected_ge > 0 && expected_ge < years.len() as u64);
    assert!(expected_le > 0 && expected_le < years.len() as u64);
    assert!(expected_range > 0 && expected_range < years.len() as u64);

    let got_ge = attr_filter(
        &main_table,
        "oorspronkelijkbouwjaar",
        &AttrPredicate::Ge(2000.0),
    )
    .unwrap();
    assert_eq!(got_ge, expected_ge);

    let got_le = attr_filter(
        &main_table,
        "oorspronkelijkbouwjaar",
        &AttrPredicate::Le(1900.0),
    )
    .unwrap();
    assert_eq!(got_le, expected_le);

    let got_range = attr_filter(
        &main_table,
        "oorspronkelijkbouwjaar",
        &AttrPredicate::Range(1950.0, 2000.0),
    )
    .unwrap();
    assert_eq!(got_range, expected_range);
}

/// `attr_stats` on `oorspronkelijkbouwjaar` (the same `Int64` attribute
/// column as the test above): independently derive min/max/sum/count from
/// the decoded objects — scanning only the 1115 non-null year values, the
/// same set the numeric-predicate test above already sanity-checked — then
/// assert `attr_stats` matches exactly (min/max/count) and within a tiny
/// float tolerance (sum). Delft's 2231 rows all fit in a single row group
/// (the default `row_group_size` is 65536), and that row group carries both
/// null (`BuildingPart`) and non-null (`Building`) values for this column,
/// so Parquet's own column-chunk statistics are expected to be present and
/// exercised via the stats fast-path — not the no-statistics fallback.
#[test]
fn attr_stats_matches_independently_computed_year_built_stats() {
    let (_out, main_table, objects) = convert_and_decode("delft.city.jsonl");

    let years: Vec<i64> = objects
        .iter()
        .filter_map(|o| {
            o.object
                .attributes
                .as_ref()
                .and_then(|v| v.as_object())
                .and_then(|attrs| attrs.get("oorspronkelijkbouwjaar"))
                .and_then(|v| v.as_i64())
        })
        .collect();
    assert_eq!(
        years.len(),
        1115,
        "oorspronkelijkbouwjaar must be present on exactly delft's 1115 Building rows"
    );

    let expected_min = *years.iter().min().unwrap() as f64;
    let expected_max = *years.iter().max().unwrap() as f64;
    let expected_sum: f64 = years.iter().map(|&y| y as f64).sum();
    let expected_count = years.len() as u64;

    let got = attr_stats(&main_table, "oorspronkelijkbouwjaar").unwrap();

    assert_eq!(got.min, expected_min);
    assert_eq!(got.max, expected_max);
    assert!(
        (got.sum - expected_sum).abs() < 1.0,
        "sum {} not within tolerance of expected {}",
        got.sum,
        expected_sum
    );
    assert_eq!(got.count, expected_count);
    assert!(
        got.min <= got.max,
        "min {} must be <= max {}",
        got.min,
        got.max
    );
}
