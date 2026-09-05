//! `run --variants`: the configuration-axis run. Per variant, one timed write
//! in a child of its own, the package kept under
//! `<prepared_dir>/<base>.<variant>.parquet`, then the ordinary read
//! children against it. The CSV shape is the read run's, with a `write` row
//! per variant and the variant id in the `format` column.

use std::path::PathBuf;
use std::process::{Command, Output};

use cityparquet::package::{ConvertOptions, convert};

const HEADER: &str = "dataset,format,scenario,selectivity,result_count,time_s,time_mad_s,\
peak_heap_bytes,peak_rss_bytes,repeat,notes,bytes_read,http_requests";

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../lib/cityparquet-rs/tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

/// A prepared dir the way `readbench_prepare.sh` leaves it for a
/// CityJSONSeq input: `<base>.parquet` (the package the query parameters
/// derive from) and `<base>.city.jsonl`.
fn prepared_delft() -> (tempfile::TempDir, PathBuf) {
    let prepared = tempfile::tempdir().unwrap();
    let input = fixture("delft.city.jsonl");
    let mut opts = ConvertOptions::new(input.clone(), prepared.path().join("delft.parquet"));
    opts.generate_lod0 = true;
    convert(&opts).unwrap();
    std::fs::copy(&input, prepared.path().join("delft.city.jsonl")).unwrap();
    (prepared, input)
}

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cityparquet-readbench"))
        .arg("run")
        .args(args)
        .output()
        .expect("failed to run the built cityparquet-readbench binary")
}

fn field(row: &str, i: usize) -> &str {
    row.split(',').nth(i).unwrap()
}

#[test]
fn a_variants_run_writes_reads_keeps_the_packages_and_records_sizes() {
    let (prepared, input) = prepared_delft();
    let out_csv = prepared.path().join("out.csv");
    let output = run(&[
        "--input",
        input.to_str().unwrap(),
        "--prepared-dir",
        prepared.path().to_str().unwrap(),
        "--out",
        out_csv.to_str().unwrap(),
        "--repeat",
        "1",
        "--write-repeat",
        "1",
        "--scenarios",
        "full-read,bbox",
        "--variants",
        "cityparquet,cityparquet+rg512,cityparquet+zstd1",
    ]);
    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let text = std::fs::read_to_string(&out_csv).unwrap();
    let mut lines = text.lines();
    assert_eq!(lines.next().unwrap(), HEADER);
    let rows: Vec<&str> = lines.collect();

    // Grouped per variant, write row first, then the reads in scenario order.
    let expected_order = [
        ("cityparquet", "write"),
        ("cityparquet", "full-read"),
        ("cityparquet", "bbox-query"),
        ("cityparquet", "bbox-query"),
        ("cityparquet", "bbox-query"),
        ("cityparquet+rg512", "write"),
        ("cityparquet+rg512", "full-read"),
        ("cityparquet+rg512", "bbox-query"),
        ("cityparquet+rg512", "bbox-query"),
        ("cityparquet+rg512", "bbox-query"),
        ("cityparquet+zstd1", "write"),
        ("cityparquet+zstd1", "full-read"),
        ("cityparquet+zstd1", "bbox-query"),
        ("cityparquet+zstd1", "bbox-query"),
        ("cityparquet+zstd1", "bbox-query"),
    ];
    assert_eq!(rows.len(), expected_order.len(), "rows:\n{text}");
    for (row, (label, scenario)) in rows.iter().zip(expected_order) {
        assert_eq!(field(row, 0), "delft.city.jsonl");
        assert_eq!(field(row, 1), label, "row: {row}");
        assert_eq!(field(row, 2), scenario, "row: {row}");
    }

    for row in rows.iter().filter(|r| field(r, 2) == "write") {
        assert_eq!(field(row, 3), "", "a write row has no selectivity: {row}");
        assert_eq!(
            field(row, 4),
            "2231",
            "result_count is the object count: {row}"
        );
        assert!(field(row, 5).parse::<f64>().unwrap() > 0.0);
        assert!(
            field(row, 8).parse::<u64>().unwrap() > 0,
            "peak_rss_bytes: {row}"
        );
        assert_eq!(field(row, 9), "1", "repeat is --write-repeat: {row}");
        assert_eq!(field(row, 10), "");
        assert_eq!(field(row, 11), "");
        assert_eq!(field(row, 12), "");
    }
    let full_reads: Vec<&&str> = rows.iter().filter(|r| field(r, 2) == "full-read").collect();
    assert!(full_reads.iter().all(|r| field(r, 4) == "2231"));

    for id in ["cityparquet", "cityparquet+rg512", "cityparquet+zstd1"] {
        let pkg = prepared.path().join(format!("delft.{id}.parquet"));
        assert!(
            pkg.join("metadata.json").is_file(),
            "package kept at {}",
            pkg.display()
        );
    }
    assert!(
        prepared.path().join("delft.parquet").is_dir(),
        "the prepare script's package is untouched"
    );
    let leftovers: Vec<String> = std::fs::read_dir(prepared.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with('.'))
        .collect();
    assert!(
        leftovers.is_empty(),
        "repeat directories were not cleaned up: {leftovers:?}"
    );

    let sizes = std::fs::read_to_string(prepared.path().join("sizes.csv")).unwrap();
    let mut sizes = sizes.lines();
    assert_eq!(
        sizes.next().unwrap(),
        "dataset,format,bytes,mb,ratio_vs_cityjsonseq,baseline_format,ratio_vs_baseline"
    );
    let size_rows: Vec<&str> = sizes.collect();
    assert_eq!(size_rows.len(), 3);
    for (row, id) in size_rows
        .iter()
        .zip(["cityparquet", "cityparquet+rg512", "cityparquet+zstd1"])
    {
        assert_eq!(field(row, 0), "delft");
        assert_eq!(field(row, 1), id);
        assert!(field(row, 2).parse::<u64>().unwrap() > 0);
        assert!(
            field(row, 4).parse::<f64>().unwrap() > 0.0,
            "ratio_vs_cityjsonseq: {row}"
        );
        assert_eq!(field(row, 5), "cityparquet");
    }
    assert_eq!(
        field(size_rows[0], 6),
        "1.000000",
        "the baseline is 1x against itself"
    );
    assert!(prepared.path().join("out.csv.params.json").is_file());
}

#[test]
fn a_rerun_replaces_its_own_sizes_rows_instead_of_appending() {
    let (prepared, input) = prepared_delft();
    let out_csv = prepared.path().join("out.csv");
    let args = [
        "--input",
        input.to_str().unwrap(),
        "--prepared-dir",
        prepared.path().to_str().unwrap(),
        "--out",
        out_csv.to_str().unwrap(),
        "--repeat",
        "1",
        "--write-repeat",
        "1",
        "--scenarios",
        "full-read",
        "--variants",
        "cityparquet,cityparquet+rg512",
    ];
    assert!(run(&args).status.success());
    assert!(run(&args).status.success());
    let sizes = std::fs::read_to_string(prepared.path().join("sizes.csv")).unwrap();
    assert_eq!(
        sizes.lines().count(),
        3,
        "header + two rows, not four:\n{sizes}"
    );
}

fn expect_rejection(args: &[&str], needle: &str) {
    let output = run(args);
    assert!(!output.status.success(), "must be rejected: {args:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(needle), "expected {needle:?} in:\n{stderr}");
}

/// `base` deliberately carries no `--write-repeat`: clap rejects a repeated
/// `--write-repeat` before the coordinator ever sees it, which would make the
/// `--write-repeat 0` case below assert clap's error rather than this run's
/// own validation.
fn with<'a>(base: &[&'a str], extra: &[&'a str]) -> Vec<&'a str> {
    base.iter().copied().chain(extra.iter().copied()).collect()
}

#[test]
fn variants_and_formats_are_exclusive_and_the_list_is_validated() {
    let (prepared, input) = prepared_delft();
    let out = prepared.path().join("out.csv");
    let base: Vec<&str> = vec![
        "--input",
        input.to_str().unwrap(),
        "--prepared-dir",
        prepared.path().to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
        "--repeat",
        "1",
        "--scenarios",
        "count",
    ];

    expect_rejection(
        &with(
            &base,
            &["--variants", "cityparquet", "--formats", "cityjsonseq"],
        ),
        "exclusive",
    );
    expect_rejection(
        &with(&base, &["--variants", "cityparquet+rg512"]),
        "baseline",
    );
    expect_rejection(
        &with(
            &base,
            &[
                "--variants",
                "cityparquet,cityparquet+rg512,cityparquet+rg512",
            ],
        ),
        "duplicate",
    );
    expect_rejection(
        &with(&base, &["--variants", "cityparquet,cityparquet+gzip6"]),
        "only zstd takes a level",
    );
    expect_rejection(
        &with(&base, &["--variants", "cityparquet", "--write-repeat", "0"]),
        "--write-repeat",
    );
    expect_rejection(
        &with(
            &base,
            &["--variants", "cityparquet", "--scenarios", "write"],
        ),
        "unknown scenario 'write'",
    );
    expect_rejection(
        &with(
            &base,
            &[
                "--variants",
                "cityparquet",
                "--transport",
                "http",
                "--base-url",
                "http://localhost:1",
            ],
        ),
        "server-side write",
    );
    assert!(!out.exists(), "a rejected run writes no CSV");
}
