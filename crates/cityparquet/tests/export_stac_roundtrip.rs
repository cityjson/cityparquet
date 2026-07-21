//! Plan 2b task 2: characterisation safety net, taken BEFORE `metadata.json`
//! is swapped from a flat `PackageManifest` to a STAC Item (Plan 2b's whole
//! point). These tests pin what `export` reconstructs TODAY, on both
//! profiles, while `metadata.json` is still a manifest — so once later tasks
//! rebind export to resolve tables/sidecars from STAC asset roles instead,
//! re-running this same file proves the swap changed nothing observable.
//!
//! They are expected to PASS on first run: that is the correct outcome for a
//! characterisation test (it pins current behaviour, it doesn't assert a not-
//! yet-built one). The helper bodies below are copied from
//! `roundtrip_real_data.rs` — integration test files are separate binaries
//! and cannot import each other's `fn`s, so the small, already-established
//! `fixture` / `convert_and_export` / `convert_and_export_with_profile`
//! pattern is duplicated verbatim here rather than reinvented.

use std::path::PathBuf;

use cityparquet::compare::{CompareOptions, compare_datasets};
use cityparquet::export::{ExportOptions, export};
use cityparquet::package::{ConvertOptions, convert};
use cityparquet::schema::Profile;

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

/// Same as [`convert_and_export`] but lets the caller pick the profile —
/// needed for the Compatibility-profile case below, where
/// materials.parquet/textures.parquet sidecars must actually be written and
/// resolved, not merely absent.
fn convert_and_export_with_profile(
    input: &str,
    profile: Profile,
) -> (PathBuf, tempfile::TempDir, tempfile::TempDir) {
    let package_dir = tempfile::tempdir().unwrap();
    let mut opts = ConvertOptions::new(fixture(input), package_dir.path().to_path_buf());
    opts.profile = profile;
    convert(&opts).unwrap();

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

/// Characterisation pin (Compatibility profile, sidecars): railway converts
/// with `Profile::Compatibility`, which writes materials.parquet/
/// textures.parquet sidecars for appearance, and exports back losslessly. The
/// sidecar-presence assertion proves this test exercises sidecar
/// round-tripping, not merely its absence — export's resolution of those
/// sidecars from `metadata.json` today is exactly the behaviour Plan 2b's
/// later tasks rebind onto STAC asset roles, and this pin is what proves that
/// rebind changed nothing observable.
#[test]
fn railway_compatibility_round_trips_with_sidecars() {
    let (exported, package_dir, _export_dir) =
        convert_and_export_with_profile("lod3_railway.city.json", Profile::Compatibility);

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
