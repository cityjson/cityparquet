//! Doc-format (`.city.json`) export round-trip for texture-bearing data.
//!
//! `crate::export::write_output`'s `OutputFormat::Doc` arm is the ONLY export
//! path that calls `cjseq::cjseq_to_cj` to merge the per-feature CityJSONSeq
//! representation into a single document; the Seq (`.city.jsonl`) path
//! (exercised everywhere in `roundtrip_real_data.rs`, including
//! `railway_compatibility_round_trips_losslessly_with_no_exclusions`) writes
//! a header line plus one feature line per object and never touches that
//! merge. This file closes that coverage gap with the Doc-format analogue of
//! the same railway round trip.

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

/// Mirrors `roundtrip_real_data.rs`'s `railway_fixture_with_crs` helper
/// (duplicated rather than shared: integration test binaries in the same
/// crate cannot import each other's free functions). The real
/// `lod3_railway.city.json` fixture carries no `referenceSystem`, so tests
/// that want a georeferenced conversion use a small on-disk copy with a CRS
/// injected via JSON mutation of the real fixture.
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

/// `cjseq` 0.4.1's `CityJSON::add_cjfeature` mis-bases the UV-vertex offset
/// (`t_offset`) it uses to re-home each feature's local
/// `vertices-texture`/texture-ring indices into the merged document's
/// accumulated pool (see `vendor/cjseq/PATCHES.md`). The Seq export path
/// never calls that merge, so it cannot catch this; only the Doc
/// (`.city.json`) path can. This is the M4/M5 railway round-trip gate's Doc
/// analogue: same fixture, same CRS injection, same lossless expectation,
/// different output format.
#[test]
fn railway_doc_export_round_trips_textures_losslessly() {
    let (_crs_dir, railway_path) = railway_fixture_with_crs();

    let package_dir = tempfile::tempdir().unwrap();
    convert(&ConvertOptions::new(
        railway_path.clone(),
        package_dir.path().to_path_buf(),
    ))
    .unwrap();

    let export_dir = tempfile::tempdir().unwrap();
    let output = export_dir.path().join("export.city.json");
    export(&ExportOptions {
        package_dir: package_dir.path().to_path_buf(),
        output: output.clone(),
    })
    .unwrap();

    let report = compare_datasets(&railway_path, &output, &CompareOptions::default()).unwrap();
    assert!(
        report.equal,
        "Doc-format (.city.json) export must round-trip textures losslessly, matching the \
         Seq path's already-proven behaviour in \
         `railway_compatibility_round_trips_losslessly_with_no_exclusions`; differences: {:#?}",
        report.differences
    );
    assert!(
        report.differences.is_empty(),
        "no differences at all are expected; got: {:#?}",
        report.differences
    );
    assert!(
        report.differences.iter().all(|d| !d.contains("texture")),
        "no difference may be texture-related; got: {:#?}",
        report.differences
    );
}
