//! Task 1 (mandatory-by-type-layout plan): characterization safety net.
//!
//! Pins the exact by-module object-table set the by-module layout produces
//! (spec "By-module object-table layout") — this test stayed green across
//! Task 3's removal of the single-file table layout (by-type/by-module is
//! now unconditional), proving the deletion did not perturb output by a
//! single byte. Updated for the ModuleKey-driven by-module split (spec
//! alignment): the file set is keyed by CityGML 3.0 module, not 1st-level
//! CityJSON family, so it is smaller and differently named than the old
//! by-family layout.

use std::collections::BTreeSet;
use std::path::PathBuf;

use cityparquet::package::{ConvertOptions, convert};

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

fn table_files(dir: &std::path::Path) -> BTreeSet<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".parquet"))
        .collect()
}

#[test]
fn railway_by_type_writes_one_file_per_citygml_module() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("pkg");
    let opts = ConvertOptions::new(fixture("lod3_railway.city.json"), out.clone());
    convert(&opts).expect("convert railway");

    let expected: BTreeSet<String> = [
        "bridge.parquet",
        "building.parquet",
        "city_furniture.parquet",
        "generics.parquet",
        "transportation.parquet",
        "vegetation.parquet",
        "relief.parquet",
        "tunnel.parquet",
        "water_body.parquet",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    assert_eq!(
        table_files(&out),
        expected,
        "by-module must write exactly one file per CityGML module; 2nd-level types (and \
         CityObjectGroup, folded per spec) share their module's file"
    );
}

/// `tests/fixtures/empty.city.jsonl` is a single CityJSONSeq header line
/// (`CityObjects: {}`, no feature lines follow) — the minimal input that
/// scans to zero city-object rows (`scan_result.object_count == 0`).
/// `write_package` used to paper over a zero-row by-type run by writing a
/// standalone, empty reserved-name fallback table (the by-type writer opens
/// tables lazily per-family, so zero rows means zero writers ever open);
/// per plan decision (2026-07-21) that fallback is gone and this must be a
/// hard error instead, off `scan_result.object_count` — see
/// `crates/cityparquet/tests/convert_real_data.rs`'s
/// `empty_input_is_rejected` for the same assertion.
#[test]
fn zero_object_input_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("pkg");
    let opts = ConvertOptions::new(fixture("empty.city.jsonl"), out);
    let err = convert(&opts).unwrap_err();
    assert!(
        format!("{err}").contains("no city objects"),
        "a zero-object conversion must fail clearly, got: {err}"
    );
}
