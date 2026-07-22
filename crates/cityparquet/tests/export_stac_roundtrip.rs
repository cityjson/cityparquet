//! Plan 2b task 2: characterisation safety net, written BEFORE `metadata.json`
//! was swapped from a flat package manifest to a STAC Item (Plan 2b's whole
//! point). These tests pin what `export` reconstructs, on both profiles, now
//! that `export` resolves tables/sidecars from STAC asset roles instead of a
//! manifest — re-running this same file after the swap proves it changed
//! nothing observable.
//!
//! They passed before the swap and must keep passing after it: that is the
//! correct outcome for a characterisation test (it pins observable behaviour
//! across a refactor, it doesn't assert a not-yet-built one). The helper
//! bodies below are copied from
//! `roundtrip_real_data.rs` — integration test files are separate binaries
//! and cannot import each other's `fn`s, so the small, already-established
//! `fixture` / `convert_and_export` pattern is duplicated verbatim here
//! rather than reinvented. `convert_and_export_with_profile` is gone
//! (spec-alignment gap 19 dropped `Profile`): sidecars are now written
//! whenever the source has content for them, so `convert_and_export` alone
//! covers both the old Core and Compatibility cases.

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

/// Converts `input` into a fresh tempdir package under the Core profile
/// (`ConvertOptions::new`'s default — which is also by-type, the only
/// object-table layout since `feat/mandatory-bytype-layout`), exports it back
/// to `.city.jsonl`, and returns the export's path alongside the tempdirs
/// that back both (kept alive so the caller can still read the file).
fn convert_and_export(input: &str) -> (PathBuf, tempfile::TempDir, tempfile::TempDir) {
    let package_dir = tempfile::tempdir().unwrap();
    convert(&ConvertOptions::new(
        fixture(input),
        package_dir.path().to_path_buf(),
    ))
    .unwrap();

    let export_dir = tempfile::tempdir().unwrap();
    let output = export_dir.path().join("export.city.jsonl");
    export(&ExportOptions {
        package_dir: package_dir.path().to_path_buf(),
        output: output.clone(),
    })
    .unwrap();

    (output, package_dir, export_dir)
}


/// Characterisation pin (Core profile, by-type default): delft converts into
/// a package and exports back out losslessly (up to the documented
/// degenerate-ring/header-metadata exclusions already established by
/// `roundtrip_real_data.rs::delft_round_trips_losslessly`). This is the
/// baseline Plan 2b's `metadata.json` -> STAC Item swap must not disturb.
#[test]
fn delft_core_round_trips_through_export() {
    let (exported, _package_dir, _export_dir) = convert_and_export("delft.city.jsonl");
    let report = compare_datasets(
        &fixture("delft.city.jsonl"),
        &exported,
        &CompareOptions::default(),
    )
    .unwrap();
    assert!(
        report.equal,
        "delft must round-trip losslessly through export today (characterisation pin for the \
         upcoming metadata.json -> STAC Item swap); differences: {:#?}",
        report.differences
    );
}

/// Characterisation pin (sidecars): railway carries materials/textures, which
/// are now written unconditionally whenever the source has them (no `Profile`
/// choice to make), and exports back losslessly. The sidecar-presence
/// assertion proves this test exercises sidecar round-tripping, not merely
/// its absence — export's resolution of those sidecars from `metadata.json`
/// today is exactly the behaviour Plan 2b's later tasks rebind onto STAC
/// asset roles, and this pin is what proves that rebind changed nothing
/// observable.
#[test]
fn railway_compatibility_round_trips_with_sidecars() {
    let (exported, package_dir, _export_dir) = convert_and_export("lod3_railway.city.json");

    let has_materials = package_dir.path().join("materials.parquet").exists();
    let has_textures = package_dir.path().join("textures.parquet").exists();
    assert!(
        has_materials || has_textures,
        "Compatibility-profile railway must write at least one appearance sidecar \
         (materials.parquet or textures.parquet); package dir: {:?}",
        package_dir.path()
    );

    let report = compare_datasets(
        &fixture("lod3_railway.city.json"),
        &exported,
        &CompareOptions::default(),
    )
    .unwrap();
    assert!(
        report.equal,
        "railway must round-trip losslessly through export under the Compatibility profile \
         today (characterisation pin for the upcoming metadata.json -> STAC Item swap); \
         differences: {:#?}",
        report.differences
    );
}
