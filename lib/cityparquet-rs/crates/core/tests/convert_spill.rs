//! The Hilbert sort spills to disk under a memory budget and merges the runs
//! back; the package it writes does not depend on whether it spilled.

mod support;

use std::fs;
use std::path::Path;

use cityparquet::package::{ConvertOptions, ConvertReport, convert};

use support::fixture;

const BATCH: usize = 2048;

fn convert_with_budget(input: &Path, out: &Path, budget: usize) -> ConvertReport {
    let mut opts = ConvertOptions::new(input.to_path_buf(), out.to_path_buf());
    // Delft's rows arrive as several batches, and each spilled run is
    // several of the merge's read batches long, so an output batch draws on
    // a run across its read-batch boundary.
    opts.batch_size = BATCH;
    opts.memory_budget = budget;
    opts.datetime = Some("2026-10-13T00:00:00Z".to_string());
    convert(&opts).unwrap()
}

/// Every Parquet file of `pkg`, by name, with its bytes.
fn parquet_files(pkg: &Path) -> Vec<(String, Vec<u8>)> {
    let mut files: Vec<(String, Vec<u8>)> = fs::read_dir(pkg)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "parquet"))
        .map(|p| {
            (
                p.file_name().unwrap().to_string_lossy().to_string(),
                fs::read(&p).unwrap(),
            )
        })
        .collect();
    files.sort();
    files
}

#[test]
fn a_spilled_hilbert_sort_writes_the_same_package_as_an_in_memory_one() {
    let dir = tempfile::tempdir().unwrap();
    let input = fixture("delft.city.jsonl");

    let in_memory = convert_with_budget(&input, &dir.path().join("memory"), usize::MAX);
    assert_eq!(in_memory.sort_runs, 0);

    // A budget smaller than one batch spills every batch as its own run.
    let spilled = convert_with_budget(&input, &dir.path().join("spilled"), 1);
    assert_eq!(spilled.sort_runs, spilled.object_count.div_ceil(BATCH));
    assert!(spilled.sort_runs > 1, "{} runs", spilled.sort_runs);

    let a = parquet_files(&dir.path().join("memory"));
    let b = parquet_files(&dir.path().join("spilled"));
    assert_eq!(
        a.iter().map(|(n, _)| n).collect::<Vec<_>>(),
        b.iter().map(|(n, _)| n).collect::<Vec<_>>()
    );
    for ((name, x), (_, y)) in a.iter().zip(&b) {
        assert!(
            x == y,
            "{name} differs between the spilled and in-memory sorts"
        );
    }
    assert!(
        !dir.path().join("spilled/.cityparquet-tmp").exists(),
        "the spill files are removed"
    );
}
