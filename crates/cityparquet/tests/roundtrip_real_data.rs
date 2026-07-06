//! M3 task 7 (the milestone claim): full pipeline round-trip proof —
//! convert the real fixture into a package, export it back to CityJSON/Seq,
//! and prove the two are semantically equal via `compare::compare_datasets`.

use std::path::PathBuf;

use cityparquet::compare::{CompareOptions, Exclusions, compare_datasets};
use cityparquet::export::{ExportOptions, export};
use cityparquet::package::{ConvertOptions, convert};

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

/// Converts `input` into a fresh tempdir package, exports it back to
/// `.city.jsonl`, and returns the export's path alongside the tempdirs that
/// back both (kept alive so the caller can still read the file).
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

#[test]
fn delft_round_trips_losslessly() {
    let (exported, _package_dir, _export_dir) = convert_and_export("delft.city.jsonl");
    let report = compare_datasets(
        &fixture("delft.city.jsonl"),
        &exported,
        &CompareOptions::default(),
    )
    .unwrap();
    assert!(
        report.equal,
        "delft must round-trip losslessly; differences: {:#?}",
        report.differences
    );
    assert!(report.differences.is_empty());
    // delft has no appearance, no GeometryInstances, and no degenerate rings,
    // but its header DOES set metadata members (title, geographicalExtent,
    // pointOfContact) that are documented exclusions, not comparisons: every
    // excluded line must be one of those, and nothing else — a non-header
    // exclusion here would mean something real (appearance, an instance, a
    // degenerate ring) slipped through undetected.
    assert!(
        report
            .excluded
            .iter()
            .all(|e| e.starts_with("header: metadata member")),
        "delft's only exclusions must be documented header metadata members, got: {:#?}",
        report.excluded
    );
}

#[test]
fn railway_round_trips_losslessly_modulo_documented_drops() {
    let (exported, _package_dir, _export_dir) = convert_and_export("lod3_railway.city.json");
    let opts = CompareOptions {
        coord_tolerance: [0.0; 3],
        exclusions: Exclusions {
            appearance: true,
            geometry_instances: true,
        },
    };
    let report = compare_datasets(&fixture("lod3_railway.city.json"), &exported, &opts).unwrap();
    assert!(
        report.equal,
        "railway must round-trip losslessly modulo the documented appearance/instance drops; \
         differences: {:#?}",
        report.differences
    );
    assert!(report.differences.is_empty());

    // Split header-metadata exclusions (documented, unbounded — whatever
    // metadata members railway's header happens to set) from everything
    // else, and pin the non-header set exactly as before: any non-header,
    // non-pinned exclusion must still fail this test.
    let (header_excluded, non_header_excluded): (Vec<&String>, Vec<&String>) = report
        .excluded
        .iter()
        .partition(|e| e.starts_with("header: metadata member"));

    // The exact exclusion breakdown, recounted by category. The totals are
    // pinned against counts already proven elsewhere: 105 stored geometries
    // carry material or texture (export_real_data.rs's
    // appearance_refs_dropped), 15 objects carry a GeometryInstance
    // (instance_geometries_dropped), and 3 source geometries carry
    // degenerate rings (wkb_roundtrip_real_data.rs's geometries_with_drops).
    let appearance = non_header_excluded
        .iter()
        .filter(|e| e.contains("exclusions.appearance"))
        .count();
    let instances = non_header_excluded
        .iter()
        .filter(|e| e.contains("exclusions.geometry_instances"))
        .count();
    let degenerate = non_header_excluded
        .iter()
        .filter(|e| e.contains("degenerate ring"))
        .count();
    assert_eq!(
        (appearance, instances, degenerate),
        (105, 15, 3),
        "exclusion breakdown must match the pinned pipeline counts, got: {:#?}",
        non_header_excluded
    );
    assert_eq!(
        non_header_excluded.len(),
        123,
        "105 appearance + 15 instances + 3 degenerate = 123 total non-header exclusions, \
         nothing else, got: {:#?}",
        non_header_excluded
    );

    // Railway's header sets `metadata.geographicalExtent`, a documented
    // exclusion: it must be logged, never silently dropped.
    assert!(
        !header_excluded.is_empty(),
        "railway's header sets metadata members; expected at least one documented \
         header-metadata exclusion, got none. Full excluded: {:#?}",
        report.excluded
    );
}
