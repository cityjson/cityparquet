//! RED (M5 task 6): `bench::run` called directly (library-shaped, not via
//! `Command::new(binary)`), exercised against the real delft fixture.

use std::path::PathBuf;

use cityparquet_cli::bench::{BenchOptions, run};

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

/// One parsed CSV data row, indexed by position in the header line's
/// comma-split column names (see `columns` at each call site — there is no
/// `CSV_COLUMNS` constant; the column order is read back from the CSV's own
/// header row rather than hard-coded here).
struct Row {
    fields: Vec<String>,
}

impl Row {
    fn get(&self, columns: &[&str], name: &str) -> String {
        let idx = columns
            .iter()
            .position(|c| *c == name)
            .unwrap_or_else(|| panic!("no column named {name}"));
        self.fields[idx].clone()
    }
}

#[test]
fn bench_run_produces_the_default_ten_variant_matrix_for_delft() {
    let out_dir = tempfile::tempdir().unwrap();
    let out_csv = out_dir.path().join("bench.csv");

    let opts = BenchOptions {
        input: fixture("delft.city.jsonl"),
        out_csv: out_csv.clone(),
        repeat: 1,
        variants: Vec::new(),
        window_frac: 0.05,
        skip_roundtrip: false,
    };

    run(&opts).expect("bench::run should succeed against the delft fixture");

    let csv_text = std::fs::read_to_string(&out_csv).unwrap();
    let mut lines = csv_text.lines();

    let header = lines.next().expect("CSV must have a header line");
    assert_eq!(
        header,
        "dataset,variant,object_count,write_s,total_bytes,cityobjects_bytes,sidecar_bytes,\
         full_scan_s,window_query_s,row_groups_total,row_groups_touched,roundtrip_equal",
        "CSV header must match the binding interface exactly"
    );
    let columns: Vec<&str> = header.split(',').collect();

    let rows: Vec<Row> = lines
        .map(|line| Row {
            fields: line.split(',').map(str::to_string).collect(),
        })
        .collect();
    assert_eq!(
        rows.len(),
        10,
        "the default variant set must produce exactly 10 rows (M5 Codex-review fix 4 added \
         `cityparquet+rg4096` / `cityparquet+hilbert+rg4096` to the prior 8), got: {csv_text}"
    );

    let mut by_variant = std::collections::HashMap::new();
    for row in &rows {
        let variant = row.get(&columns, "variant");

        let roundtrip_equal = row.get(&columns, "roundtrip_equal");
        assert_eq!(
            roundtrip_equal, "true",
            "variant {variant}: roundtrip_equal must be true for a lossless real dataset, got: {csv_text}"
        );

        let total_bytes: u64 = row.get(&columns, "total_bytes").parse().unwrap();
        assert!(
            total_bytes > 0,
            "variant {variant}: total_bytes must be > 0, got {total_bytes}"
        );

        let row_groups_total: usize = row.get(&columns, "row_groups_total").parse().unwrap();
        let row_groups_touched: usize = row.get(&columns, "row_groups_touched").parse().unwrap();
        assert!(
            row_groups_touched <= row_groups_total,
            "variant {variant}: row_groups_touched ({row_groups_touched}) must be <= \
             row_groups_total ({row_groups_total})"
        );

        by_variant.insert(variant, row_groups_touched);
    }

    let cityparquet_touched = *by_variant
        .get("cityparquet")
        .expect("the default set must include the plain 'cityparquet' variant");
    let hilbert_touched = *by_variant
        .get("cityparquet+hilbert")
        .expect("the default set must include 'cityparquet+hilbert'");
    assert!(
        hilbert_touched <= cityparquet_touched,
        "cityparquet+hilbert should touch no more row groups than plain cityparquet on delft's \
         window query: hilbert={hilbert_touched} cityparquet={cityparquet_touched}"
    );

    // M5 Codex review (Important finding 4): delft (2231 objects) is still
    // smaller than the 4096-row row-group size, so both `+rg4096` rows land
    // in a single row group here too (`row_groups_touched` == 1 either
    // way) — this only proves the two rows exist and preserve the same
    // ordering invariant as the un-sized pair above; the row-group COUNT
    // actually growing past 1, and Hilbert's pruning effect on top of that,
    // is only observable on the larger pinned 3DBAG tiles (see
    // `bench/README.md`'s pruning numbers).
    let rg4096_touched = *by_variant
        .get("cityparquet+rg4096")
        .expect("the default set must include 'cityparquet+rg4096'");
    let hilbert_rg4096_touched = *by_variant
        .get("cityparquet+hilbert+rg4096")
        .expect("the default set must include 'cityparquet+hilbert+rg4096'");
    assert!(
        hilbert_rg4096_touched <= rg4096_touched,
        "cityparquet+hilbert+rg4096 should touch no more row groups than plain \
         cityparquet+rg4096 on delft's window query: \
         hilbert_rg4096={hilbert_rg4096_touched} rg4096={rg4096_touched}"
    );
}

#[test]
fn bench_run_rejects_an_unknown_variant_with_the_grammar_in_the_message() {
    let out_dir = tempfile::tempdir().unwrap();
    let opts = BenchOptions {
        input: fixture("delft.city.jsonl"),
        out_csv: out_dir.path().join("bench.csv"),
        repeat: 1,
        variants: vec!["not-a-real-preset".to_string()],
        window_frac: 0.05,
        skip_roundtrip: true,
    };

    let err = run(&opts).expect_err("an unknown variant string must error");
    let msg = err.to_string();
    assert!(
        msg.contains("not-a-real-preset"),
        "error should name the offending variant, got: {msg}"
    );
    assert!(
        msg.contains("<preset>[+hilbert][+by-type]"),
        "error should show the variant grammar, got: {msg}"
    );
}

/// M5 final-review fix: `repeat: 0` must error fast, before any conversion
/// runs (the guard is the very first thing `bench::run` checks) — otherwise
/// `run_variant`'s write/full-scan/window-query loops each collect zero
/// samples and `median_secs` panics on an empty `Vec`.
#[test]
fn bench_run_rejects_repeat_zero_before_converting_anything() {
    let out_dir = tempfile::tempdir().unwrap();
    let opts = BenchOptions {
        // A path that does not exist: if the guard did not fire FIRST and
        // conversion were attempted, this would fail for the WRONG reason
        // (missing input) rather than proving the repeat check runs first.
        input: PathBuf::from("/nonexistent/does-not-exist.city.json"),
        out_csv: out_dir.path().join("bench.csv"),
        repeat: 0,
        variants: Vec::new(),
        window_frac: 0.05,
        skip_roundtrip: true,
    };

    let err = run(&opts).expect_err("repeat: 0 must error");
    let msg = err.to_string();
    assert!(
        msg.contains("repeat") && msg.contains(">= 1"),
        "error should name the repeat >= 1 requirement, got: {msg}"
    );
}

/// M5 final-review fix: appending to a CSV that already exists but carries a
/// DIFFERENT header (e.g. left over from an older schema, or hand-edited)
/// must error rather than silently mixing two column schemas into one file.
#[test]
fn bench_run_rejects_appending_to_a_csv_with_a_foreign_header() {
    let out_dir = tempfile::tempdir().unwrap();
    let out_csv = out_dir.path().join("bench.csv");
    std::fs::write(&out_csv, "dataset,variant,some,other,schema\n").unwrap();

    let opts = BenchOptions {
        input: fixture("delft.city.jsonl"),
        out_csv: out_csv.clone(),
        repeat: 1,
        variants: vec!["cityparquet".to_string()],
        window_frac: 0.05,
        skip_roundtrip: true,
    };

    let err = run(&opts).expect_err("a foreign CSV header must error");
    let msg = err.to_string();
    assert!(
        msg.contains(&out_csv.display().to_string()),
        "error should name the offending file, got: {msg}"
    );
    assert!(
        msg.contains("dataset,variant,some,other,schema"),
        "error should quote the mismatched header found on disk, got: {msg}"
    );
}

/// M5 Codex review (Minor finding): `--window-frac` must satisfy
/// `0 < window_frac <= 1` and be finite — checked fast, before any
/// conversion runs, mirroring the `repeat >= 1` guard above. Table-driven
/// over the invalid shapes the window-scaling arithmetic in `run_variant`
/// would otherwise mishandle silently: zero/negative (an inverted or
/// empty window), `> 1` (a window larger than the dataset, e.g. an
/// intended "5%" typed as `5`), and non-finite (`NaN`/`inf`).
#[test]
fn bench_run_rejects_an_invalid_window_frac() {
    for bad in [0.0, -0.1, 1.5, 5.0, f64::NAN, f64::INFINITY] {
        let out_dir = tempfile::tempdir().unwrap();
        let opts = BenchOptions {
            input: PathBuf::from("/nonexistent/does-not-exist.city.json"),
            out_csv: out_dir.path().join("bench.csv"),
            repeat: 1,
            variants: Vec::new(),
            window_frac: bad,
            skip_roundtrip: true,
        };

        let err = run(&opts).expect_err(&format!("window_frac {bad} must error"));
        let msg = err.to_string();
        assert!(
            msg.contains("window_frac") && msg.contains("0 < window_frac <= 1"),
            "window_frac {bad}: error should name the 0 < window_frac <= 1 requirement, got: {msg}"
        );
    }
}

/// M5 Codex review (Minor finding): the variant parser must reject a
/// duplicated suffix (e.g. `+hilbert` or `+by-type` or `+rg<N>` appearing
/// twice) rather than silently accepting it as a distinct-looking label for
/// the same — or, for two conflicting `+rg<N>`s, an ambiguous — writer
/// configuration.
#[test]
fn bench_run_rejects_duplicate_variant_suffixes() {
    for variant in [
        "cityparquet+hilbert+hilbert",
        "cityparquet+by-type+by-type",
        "cityparquet+rg4096+rg8192",
    ] {
        let out_dir = tempfile::tempdir().unwrap();
        let opts = BenchOptions {
            input: fixture("delft.city.jsonl"),
            out_csv: out_dir.path().join("bench.csv"),
            repeat: 1,
            variants: vec![variant.to_string()],
            window_frac: 0.05,
            skip_roundtrip: true,
        };

        let err = run(&opts).expect_err(&format!("duplicate suffix '{variant}' must error"));
        let msg = err.to_string();
        assert!(
            msg.contains(variant),
            "variant '{variant}': error should name the offending variant, got: {msg}"
        );
    }
}

/// M5 Codex review (Important finding 4): the `+rg<N>` suffix's `<N>` must
/// be a positive integer — zero, negative, non-numeric, and empty are all
/// rejected rather than silently accepted as some default/garbage
/// row-group size.
#[test]
fn bench_run_rejects_a_malformed_rg_suffix() {
    for variant in [
        "cityparquet+rg0",
        "cityparquet+rg-1",
        "cityparquet+rgabc",
        "cityparquet+rg",
    ] {
        let out_dir = tempfile::tempdir().unwrap();
        let opts = BenchOptions {
            input: fixture("delft.city.jsonl"),
            out_csv: out_dir.path().join("bench.csv"),
            repeat: 1,
            variants: vec![variant.to_string()],
            window_frac: 0.05,
            skip_roundtrip: true,
        };

        let err = run(&opts).expect_err(&format!("malformed rg suffix '{variant}' must error"));
        let msg = err.to_string();
        assert!(
            msg.contains(variant),
            "variant '{variant}': error should name the offending variant, got: {msg}"
        );
    }
}
