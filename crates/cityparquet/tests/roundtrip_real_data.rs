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
    assert!(
        report.excluded.is_empty(),
        "delft has no appearance, no GeometryInstances, and no degenerate rings: \
         nothing should be excluded, got: {:#?}",
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
    assert!(
        !report.excluded.is_empty(),
        "railway carries GeometryInstances, appearance, and degenerate rings — all must be logged"
    );

    let joined = report.excluded.join("\n");
    assert!(
        joined.contains("GeometryInstance"),
        "expected at least one excluded GeometryInstance entry, got: {joined}"
    );
    assert!(
        joined.contains("exclusions.appearance"),
        "expected at least one excluded material/texture entry, got: {joined}"
    );
    assert!(
        joined.contains("degenerate ring"),
        "expected at least one excluded degenerate-ring normalisation entry, got: {joined}"
    );
}
