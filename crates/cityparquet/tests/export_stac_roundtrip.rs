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
    convert_and_export_path(fixture(input))
}

/// [`convert_and_export`] taking an already-resolved input path (so a
/// CRS-injected derivative can be passed directly).
fn convert_and_export_path(input: PathBuf) -> (PathBuf, tempfile::TempDir, tempfile::TempDir) {
    let package_dir = tempfile::tempdir().unwrap();
    convert(&ConvertOptions::new(
        input,
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

/// The real `lod3_railway.city.json` fixture carries no `referenceSystem` at
/// all. Since `scan` now hard-fails on coordinate-bearing input with no
/// resolvable CRS (spec "CRS rules"), writes a small on-disk COPY with a CRS
/// injected via JSON mutation of the real fixture — never hand-written
/// CityJSON.
fn railway_fixture_with_crs() -> (tempfile::TempDir, PathBuf) {
    let mut doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(fixture("lod3_railway.city.json")).unwrap())
            .unwrap();
    doc["metadata"]["referenceSystem"] =
        serde_json::json!("https://www.opengis.net/def/crs/EPSG/0/7415");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("railway_with_crs.city.json");
    std::fs::write(&path, serde_json::to_string(&doc).unwrap()).unwrap();
    (dir, path)
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
    let (_crs_dir, railway_path) = railway_fixture_with_crs();
    let (exported, package_dir, _export_dir) = convert_and_export_path(railway_path.clone());

    let has_materials = package_dir.path().join("materials.parquet").exists();
    let has_textures = package_dir.path().join("textures.parquet").exists();
    assert!(
        has_materials || has_textures,
        "Compatibility-profile railway must write at least one appearance sidecar \
         (materials.parquet or textures.parquet); package dir: {:?}",
        package_dir.path()
    );

    let report = compare_datasets(&railway_path, &exported, &CompareOptions::default()).unwrap();
    assert!(
        report.equal,
        "railway must round-trip losslessly through export under the Compatibility profile \
         today (characterisation pin for the upcoming metadata.json -> STAC Item swap); \
         differences: {:#?}",
        report.differences
    );
}
