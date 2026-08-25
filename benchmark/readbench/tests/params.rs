//! `cityparquet_readbench::params` against a real converted CityParquet
//! package, built here with `cityparquet::package::convert` from a committed
//! fixture — no network, no external tool, no prepared corpus.

use std::path::PathBuf;

use cityparquet::package::{ConvertOptions, convert};
use cityparquet_readbench::params::scan_row_bboxes;

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../lib/cityparquet-rs/tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

/// Converts `delft.city.jsonl` into a package in a fresh temp dir and returns
/// its single main table. Delft is the fixture that by-type-converts to
/// exactly ONE table (Building + BuildingPart both map to the "Building"
/// family), which is what these tests need.
fn delft_table() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("temp dir");
    let out = dir.path().join("delft.parquet");
    convert(&ConvertOptions::new(
        fixture("delft.city.jsonl"),
        out.clone(),
    ))
    .expect("converting delft");
    let table = out.join("building.parquet");
    assert!(table.exists(), "expected {} to exist", table.display());
    (dir, table)
}

#[test]
fn scan_row_bboxes_returns_one_box_per_row_and_their_union() {
    let (_dir, table) = delft_table();
    let scanned = scan_row_bboxes(&table).expect("scanning row bboxes");

    assert!(
        !scanned.boxes.is_empty(),
        "delft's table has rows, so it has row bboxes"
    );

    // The union must contain every row box, on every axis.
    for row in &scanned.boxes {
        for axis in 0..3 {
            assert!(
                scanned.dataset[axis] <= row[axis],
                "dataset min on axis {axis} must not exceed a row's min"
            );
            assert!(
                scanned.dataset[axis + 3] >= row[axis + 3],
                "dataset max on axis {axis} must not be below a row's max"
            );
        }
    }
}
