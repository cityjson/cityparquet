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

use cityparquet_readbench::params::{citygml_ids, seq_feature_ids};

#[test]
fn seq_feature_ids_reads_the_stream_in_order_and_skips_the_metadata_line() {
    let ids = seq_feature_ids(&fixture("delft.city.jsonl")).expect("reading seq ids");
    assert!(!ids.is_empty(), "delft has features");
    assert!(
        !ids.iter().any(|id| id.is_empty()),
        "no feature id may be empty"
    );
    // The first line is the CityJSON metadata object, not a feature, so the
    // count is the feature count rather than the line count.
    let lines = std::fs::read_to_string(fixture("delft.city.jsonl"))
        .expect("reading the fixture")
        .lines()
        .filter(|l| !l.trim().is_empty())
        .count();
    assert_eq!(
        ids.len(),
        lines - 1,
        "one id per feature line, the metadata line excluded"
    );
}

/// `b1_lod2_cs_w_sem.gml` is one of the two CityGML 2.0 files `just
/// fixtures` fetches — a single semantically-decomposed building.
#[test]
fn citygml_ids_collects_every_city_object_key() {
    let ids = citygml_ids(&fixture("b1_lod2_cs_w_sem.gml")).expect("reading citygml ids");
    assert!(!ids.is_empty(), "the CityGML fixture has city objects");
}

use cityparquet_readbench::params::resolve;
use std::path::Path;

#[test]
fn resolve_produces_three_populated_windows_and_four_id_probes() {
    let (_dir, table) = delft_table();
    let resolved = resolve(
        "delft.city.jsonl",
        &table,
        Some(&fixture("delft.city.jsonl")),
        None,
    )
    .expect("resolving params");

    assert_eq!(resolved.windows.len(), 3, "three bbox windows");
    for window in &resolved.windows {
        assert!(
            window.achieved > 0.0,
            "{} selected no rows — the defect this replaces",
            window.tag
        );
    }

    assert_eq!(resolved.id_probes.len(), 4, "three deciles plus a miss");
    assert!(
        !resolved.object_type.is_empty(),
        "an object_type was chosen"
    );
    assert!(resolved.cp_object_total > 0, "a non-zero denominator");
}

#[test]
fn resolve_fails_loudly_when_the_seq_artefact_is_missing() {
    let (_dir, table) = delft_table();
    let err = resolve(
        "delft.city.jsonl",
        &table,
        Some(Path::new("/nonexistent/delft.city.jsonl")),
        None,
    )
    .expect_err("a seq artefact that is named but unreadable must be a hard failure");
    let message = format!("{err:#}");
    assert!(
        message.contains("delft.city.jsonl"),
        "the error must name the missing artefact, got: {message}"
    );
}

/// No seq artefact in the prepared directory means no canonical order to cut
/// deciles from. Deriving them from some other order would quietly redefine
/// what a probe's position means, so the scenario is skipped instead — the
/// same treatment a dataset with no numeric attribute gets.
#[test]
fn resolve_yields_no_id_probes_when_there_is_no_seq_artefact() {
    let (_dir, table) = delft_table();
    let resolved =
        resolve("delft.city.jsonl", &table, None, None).expect("resolving without a seq artefact");

    assert!(
        resolved.id_probes.is_empty(),
        "no seq artefact means no id probes, got {:?}",
        resolved.id_probes
    );
    // Everything the cityparquet package alone can answer still resolves.
    assert_eq!(resolved.windows.len(), 3, "windows need only the package");
    assert!(resolved.cp_object_total > 0);
}
