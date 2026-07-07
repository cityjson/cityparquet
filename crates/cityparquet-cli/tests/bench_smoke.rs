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

/// One parsed CSV data row, indexed by [`CSV_COLUMNS`] position.
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
fn bench_run_produces_the_default_eight_variant_matrix_for_delft() {
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
        8,
        "the default variant set must produce exactly 8 rows, got: {csv_text}"
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
