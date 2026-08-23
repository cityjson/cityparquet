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
fn bench_run_produces_the_default_nine_variant_matrix_for_delft() {
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
        9,
        "the default variant set must produce exactly 9 rows (M5 Codex-review fix 4 added \
         `cityparquet+rg512` / `cityparquet+hilbert+rg512` to the prior 8, whose \
         `cityparquet+by-type` was retired 2026-07-21 when by-type became the sole, mandatory \
         layout), got: {csv_text}"
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

        by_variant.insert(variant, (row_groups_total, row_groups_touched));
    }

    let (_, cityparquet_touched) = *by_variant
        .get("cityparquet")
        .expect("the default set must include the plain 'cityparquet' variant");
    let (_, hilbert_touched) = *by_variant
        .get("cityparquet+hilbert")
        .expect("the default set must include 'cityparquet+hilbert'");
    assert!(
        hilbert_touched <= cityparquet_touched,
        "cityparquet+hilbert should touch no more row groups than plain cityparquet on delft's \
         window query: hilbert={hilbert_touched} cityparquet={cityparquet_touched}"
    );

    // M5 Codex review (Important finding 4, revised ruling): the default
    // rg-size rows are `+rg512`, small enough that delft's 2231 objects
    // genuinely split into multiple row groups (ceil(2231/512) = 5) — the
    // original `+rg4096` choice left even the largest committed dataset
    // (2423 objects) in a single group, demonstrating nothing. With real
    // groups to prune, Hilbert ordering must do no worse than source order
    // on the window query (measured: source order touches 2 of 5, Hilbert
    // 1 of 5 — see `benchmark/formats/README.md`'s pruning numbers).
    let (rg512_total, rg512_touched) = *by_variant
        .get("cityparquet+rg512")
        .expect("the default set must include 'cityparquet+rg512'");
    let (hilbert_rg512_total, hilbert_rg512_touched) = *by_variant
        .get("cityparquet+hilbert+rg512")
        .expect("the default set must include 'cityparquet+hilbert+rg512'");
    assert_eq!(
        rg512_total, 5,
        "cityparquet+rg512 on delft (2231 objects) must write ceil(2231/512) = 5 row groups"
    );
    assert_eq!(
        hilbert_rg512_total, 5,
        "cityparquet+hilbert+rg512 on delft (2231 objects) must write ceil(2231/512) = 5 row \
         groups"
    );
    assert!(
        hilbert_rg512_touched <= rg512_touched,
        "cityparquet+hilbert+rg512 should touch no more row groups than plain \
         cityparquet+rg512 on delft's window query: \
         hilbert_rg512={hilbert_rg512_touched} rg512={rg512_touched}"
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
        msg.contains("<preset>[+hilbert][+rg<N>]"),
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
/// duplicated suffix (e.g. `+hilbert` or `+rg<N>` appearing twice) rather
/// than silently accepting it as a distinct-looking label for the same —
/// or, for two conflicting `+rg<N>`s, an ambiguous — writer configuration.
#[test]
fn bench_run_rejects_duplicate_variant_suffixes() {
    for variant in [
        "cityparquet+hilbert+hilbert",
        "cityparquet+rg4096+rg8192",
        "cityparquet+gzip+zstd",
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

/// A `+<codec>` suffix combines with `+rg<N>` (and every other existing
/// suffix) without disturbing the grammar: `cityparquet+gzip+rg512` must
/// parse and run to completion, producing exactly one CSV row.
#[test]
fn bench_run_accepts_a_codec_suffix_combined_with_rg() {
    let out_dir = tempfile::tempdir().unwrap();
    let out_csv = out_dir.path().join("bench.csv");

    let opts = BenchOptions {
        input: fixture("delft.city.jsonl"),
        out_csv: out_csv.clone(),
        repeat: 1,
        variants: vec!["cityparquet+gzip+rg512".to_string()],
        window_frac: 0.05,
        skip_roundtrip: true,
    };

    run(&opts).expect("cityparquet+gzip+rg512 must parse and run");

    let csv_text = std::fs::read_to_string(&out_csv).unwrap();
    let rows: Vec<&str> = csv_text.lines().skip(1).collect();
    assert_eq!(
        rows.len(),
        1,
        "one variant must produce exactly one CSV row, got: {csv_text}"
    );
    assert!(
        rows[0].contains("cityparquet+gzip+rg512"),
        "the row must be labelled with the exact variant id, got: {}",
        rows[0]
    );
}

/// The compression-codec axis: `+uncompressed`/`+gzip`/`+zstd` all override
/// the `cityparquet` preset's default codec, all round-trip losslessly on a
/// real fixture, and — proving the codec actually took effect — the
/// resulting `total_bytes` differ, with uncompressed strictly the largest.
#[test]
fn bench_run_compression_variants_differ_in_total_bytes() {
    let out_dir = tempfile::tempdir().unwrap();
    let out_csv = out_dir.path().join("bench.csv");

    let opts = BenchOptions {
        input: fixture("delft.city.jsonl"),
        out_csv: out_csv.clone(),
        repeat: 1,
        variants: vec![
            "cityparquet+gzip".to_string(),
            "cityparquet+zstd".to_string(),
            "cityparquet+uncompressed".to_string(),
        ],
        window_frac: 0.05,
        skip_roundtrip: false,
    };

    run(&opts).expect("bench::run should succeed across compression-codec variants");

    let csv_text = std::fs::read_to_string(&out_csv).unwrap();
    let mut lines = csv_text.lines();
    let header = lines.next().expect("CSV must have a header line");
    let columns: Vec<&str> = header.split(',').collect();

    let mut total_bytes_by_variant = std::collections::HashMap::new();
    for line in lines {
        let fields: Vec<&str> = line.split(',').collect();
        let get = |name: &str| -> &str {
            let idx = columns.iter().position(|c| *c == name).unwrap();
            fields[idx]
        };
        let variant = get("variant").to_string();
        let roundtrip_equal = get("roundtrip_equal");
        assert_eq!(
            roundtrip_equal, "true",
            "variant {variant}: roundtrip_equal must be true, got: {csv_text}"
        );
        let total_bytes: u64 = get("total_bytes").parse().unwrap();
        total_bytes_by_variant.insert(variant, total_bytes);
    }

    let gzip = total_bytes_by_variant["cityparquet+gzip"];
    let zstd = total_bytes_by_variant["cityparquet+zstd"];
    let uncompressed = total_bytes_by_variant["cityparquet+uncompressed"];

    assert!(
        uncompressed > zstd,
        "uncompressed total_bytes ({uncompressed}) must exceed zstd's ({zstd})"
    );
    assert!(
        uncompressed > gzip,
        "uncompressed total_bytes ({uncompressed}) must exceed gzip's ({gzip})"
    );
    assert_ne!(
        gzip, zstd,
        "gzip and zstd should produce differently-sized packages, both got {gzip} bytes"
    );
}
