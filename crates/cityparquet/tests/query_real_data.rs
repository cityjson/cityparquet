//! RED (readbench task 2): `query::full_read` / `query::count`, the first
//! read primitives of the cross-format read-benchmark milestone — exercised
//! against a real converted delft package (never inline artificial
//! CityJSON).

use std::path::PathBuf;

use cityparquet::package::{ConvertOptions, convert};
use cityparquet::reader::CityParquetReaderBuilder;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
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
