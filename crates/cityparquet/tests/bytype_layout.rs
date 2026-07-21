//! Task 1 (mandatory-by-type-layout plan): characterization safety net.
//!
//! Pins the exact by-type object-table set and round-trip behaviour the
//! `TableLayout::ByType` layout produces TODAY, while `TableLayout::Single`
//! still exists. A later task in this plan deletes `TableLayout::Single`
//! (and the `opts.layout = ...` line below, since by-type becomes
//! unconditional); this test must stay green across that removal, proving
//! the deletion did not perturb by-type output by a single byte.
//!
//! The round-trip test below reuses the crate's one true CityJSON-equality
//! routine, `cityparquet::compare::compare_datasets` (the same function
//! `roundtrip_real_data.rs` uses for every one of its round-trip gates,
//! including its own `delft_by_type_round_trips_losslessly`). Rust
//! integration tests are separate binaries, so the small `convert()` +
//! `export()` plumbing helper in `roundtrip_real_data.rs` cannot be
//! imported here directly (no `tests/common` module exists in this crate to
//! share it through); rather than add one for a single call site, this file
//! replicates that minimal plumbing inline and defers all equality
//! judgement to `compare_datasets`, exactly as `roundtrip_real_data.rs`
//! does.

use std::collections::BTreeSet;
use std::path::PathBuf;

use cityparquet::compare::{CompareOptions, compare_datasets};
use cityparquet::export::{ExportOptions, export};
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
fn railway_by_type_writes_one_file_per_first_level_family() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("pkg");
    // NOTE: while Single still exists, set layout explicitly; after the
    // later task removing TableLayout::Single, the field is gone and this
    // line is deleted (by-type is unconditional).
    let mut opts = ConvertOptions::new(fixture("lod3_railway.city.json"), out.clone());
    opts.layout = cityparquet::package::TableLayout::ByType;
    convert(&opts).expect("convert railway");

    let expected: BTreeSet<String> = [
        "bridge.parquet",
        "building.parquet",
        "cityfurniture.parquet",
        "cityobjectgroup.parquet",
        "genericcityobject.parquet",
        "railway.parquet",
        "solitaryvegetationobject.parquet",
        "tinrelief.parquet",
        "tunnel.parquet",
        "waterbody.parquet",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    assert_eq!(
        table_files(&out),
        expected,
        "by-type must write exactly one file per 1st-level family; 2nd-level types share their \
         parent's file"
    );
}

/// delft has Building + BuildingPart, so by-type puts both in
/// `building.parquet`; export must still reconstruct every object. Reuses
/// `compare_datasets` (see module docs above) rather than re-implementing
/// CityJSON equality here — the same exclusion-accounting pattern as
/// `roundtrip_real_data.rs::delft_by_type_round_trips_losslessly`, which
/// this test deliberately duplicates the pinned counts of (16
/// coordinate-degenerate-ring drops, 8 objects x source+export side) so
/// that this standalone file is a self-contained characterization of
/// by-type behaviour, not merely a re-export of that other test.
#[test]
fn delft_by_type_round_trips_to_equivalent_cityjson() {
    let package_dir = tempfile::tempdir().unwrap();
    let mut opts = ConvertOptions::new(
        fixture("delft.city.jsonl"),
        package_dir.path().to_path_buf(),
    );
    opts.layout = cityparquet::package::TableLayout::ByType;
    convert(&opts).expect("convert delft by-type");
    assert!(
        package_dir.path().join("building.parquet").exists(),
        "sanity: this must actually be a split-by-type package"
    );
    assert!(
        !package_dir.path().join("buildingpart.parquet").exists(),
        "BuildingPart is 2nd-level and must share building.parquet, not get its own file"
    );

    let export_dir = tempfile::tempdir().unwrap();
    let output = export_dir.path().join("export.city.jsonl");
    export(&ExportOptions {
        package_dir: package_dir.path().to_path_buf(),
        output: output.clone(),
    })
    .expect("export delft by-type package");

    let report = compare_datasets(
        &fixture("delft.city.jsonl"),
        &output,
        &CompareOptions::default(),
    )
    .unwrap();
    assert!(
        report.equal,
        "ByType-layout delft must round-trip losslessly; differences: {:#?}",
        report.differences
    );
    assert!(report.differences.is_empty());

    let (header_excluded, non_header_excluded): (Vec<&String>, Vec<&String>) = report
        .excluded
        .iter()
        .partition(|e| e.starts_with("header: metadata member"));
    let degenerate = non_header_excluded
        .iter()
        .filter(|e| e.contains("degenerate ring"))
        .count();
    assert_eq!(
        (degenerate, non_header_excluded.len()),
        (16, 16),
        "delft's only non-header exclusions must be the 16 pinned coordinate-degenerate-ring \
         drops (8 objects, source + export side each; see \
         `roundtrip_real_data.rs::delft_round_trips_losslessly` for the full explanation), \
         got: {:#?}",
        non_header_excluded
    );
    assert!(
        !header_excluded.is_empty(),
        "delft's header sets metadata members; expected at least one documented header-metadata \
         exclusion, got none. Full excluded: {:#?}",
        report.excluded
    );
}
