//! RED (readbench Task 11, Commit B): the COORDINATOR — `cityparquet-readbench
//! run ...` — exercised only through the BUILT binary (never calling into
//! `coordinator`'s internals directly), against a real prepared package
//! built from `lod3_railway.city.json` (never inline artificial CityJSON).
//!
//! Prepares its own tiny "prepared dir" by converting the real fixture with
//! `cityparquet::package::convert` directly (rather than shelling to
//! `scripts/readbench_prepare.sh`/`fcb`, keeping this test network- and
//! external-tool-independent) — exactly the `<x>.parquet` naming convention
//! the real prep script uses (`readbench_prepare.sh`'s own doc comment), so
//! the coordinator's artefact-location logic is exercised unmodified.

use std::path::PathBuf;
use std::process::{Command, Output};

use cityparquet::package::{ConvertOptions, convert};

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

/// Runs the built `cityparquet-readbench run ...` coordinator with `args`
/// appended, asserting it exits successfully.
fn run_coordinator(args: &[&str]) -> Output {
    let output = Command::new(env!("CARGO_BIN_EXE_cityparquet-readbench"))
        .arg("run")
        .args(args)
        .output()
        .expect("failed to run the built cityparquet-readbench binary");
    assert!(
        output.status.success(),
        "coordinator exited non-zero; stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

/// One parsed CSV data row (never the header), split on `,` — every field
/// used by this test is a simple identifier/number with no embedded comma,
/// so a naive split is sufficient.
struct Row {
    fields: Vec<String>,
}

impl Row {
    fn parse(line: &str) -> Self {
        Self {
            fields: line.split(',').map(str::to_string).collect(),
        }
    }
    fn field(&self, name: &str) -> &str {
        let index = CSV_COLUMNS
            .iter()
            .position(|c| *c == name)
            .unwrap_or_else(|| panic!("unknown column '{name}'"));
        &self.fields[index]
    }
}

const CSV_COLUMNS: [&str; 11] = [
    "dataset",
    "format",
    "scenario",
    "selectivity",
    "result_count",
    "time_s",
    "time_mad_s",
    "peak_heap_bytes",
    "peak_rss_bytes",
    "repeat",
    "notes",
];

const EXPECTED_HEADER: &str = "dataset,format,scenario,selectivity,result_count,time_s,\
time_mad_s,peak_heap_bytes,peak_rss_bytes,repeat,notes";

#[test]
fn run_produces_the_exact_csv_contract_with_medians_and_selectivity_derived_from_real_data() {
    let prepared = tempfile::tempdir().unwrap();
    let input = fixture("lod3_railway.city.json");

    // The "prepared" cityparquet package, at the exact path/name convention
    // `scripts/readbench_prepare.sh` produces: `<prepared_dir>/<base>.parquet`
    // where `<base>` strips `.city.json`.
    let package_dir = prepared.path().join("lod3_railway.parquet");
    let report = convert(&ConvertOptions::new(input.clone(), package_dir)).unwrap();
    assert_eq!(report.object_count, 121);

    let out_csv = prepared.path().join("out.csv");

    run_coordinator(&[
        "--input",
        input.to_str().unwrap(),
        "--prepared-dir",
        prepared.path().to_str().unwrap(),
        "--out",
        out_csv.to_str().unwrap(),
        "--repeat",
        "2",
        "--scenarios",
        "count,bbox",
        "--formats",
        "cityparquet,cityjsonseq",
    ]);

    let csv_text = std::fs::read_to_string(&out_csv).expect("coordinator must write the CSV");
    let mut lines = csv_text.lines();

    assert_eq!(
        lines.next().unwrap(),
        EXPECTED_HEADER,
        "the coordinator must write the exact CSV column contract"
    );

    let rows: Vec<Row> = lines.map(Row::parse).collect();
    // 2 formats x (1 count row + 3 bbox-selectivity rows) = 8 data rows.
    assert_eq!(
        rows.len(),
        8,
        "expected 8 data rows (2 formats x (count + 3 bbox windows)), got {}: {csv_text}",
        rows.len()
    );

    for row in &rows {
        assert_eq!(row.field("dataset"), "lod3_railway.city.json");
        assert!(
            ["cityparquet", "cityjsonseq"].contains(&row.field("format")),
            "unexpected format in row: {row_fields:?}",
            row_fields = row.fields
        );
        assert!(
            ["count", "bbox-query"].contains(&row.field("scenario")),
            "unexpected scenario in row: {row_fields:?}",
            row_fields = row.fields
        );

        let time_s: f64 = row
            .field("time_s")
            .parse()
            .unwrap_or_else(|e| panic!("time_s '{}' must parse as f64: {e}", row.field("time_s")));
        assert!(time_s >= 0.0, "time_s must be non-negative, got {time_s}");
        let _time_mad_s: f64 = row.field("time_mad_s").parse().unwrap_or_else(|e| {
            panic!(
                "time_mad_s '{}' must parse as f64: {e}",
                row.field("time_mad_s")
            )
        });
        let _peak_heap_bytes: u64 = row.field("peak_heap_bytes").parse().unwrap_or_else(|e| {
            panic!(
                "peak_heap_bytes '{}' must parse as u64: {e}",
                row.field("peak_heap_bytes")
            )
        });
        let _peak_rss_bytes: u64 = row.field("peak_rss_bytes").parse().unwrap_or_else(|e| {
            panic!(
                "peak_rss_bytes '{}' must parse as u64: {e}",
                row.field("peak_rss_bytes")
            )
        });
        assert_eq!(row.field("repeat"), "2");

        if row.field("scenario") == "count" {
            assert_eq!(
                row.field("selectivity"),
                "",
                "count's selectivity must be empty (N/A), got '{}'",
                row.field("selectivity")
            );
        } else {
            let selectivity: f64 = row.field("selectivity").parse().unwrap_or_else(|e| {
                panic!(
                    "bbox-query selectivity '{}' must parse as f64: {e}",
                    row.field("selectivity")
                )
            });
            assert!(
                selectivity > 0.0 && selectivity <= 1.0,
                "bbox-query selectivity must be in (0, 1] for this fixture/window set, got \
                 {selectivity} (row: {row_fields:?})",
                row_fields = row.fields
            );
        }
    }

    // Exactly one `count` row per format, three `bbox-query` rows per format
    // (1%/5%/25%), each tagged in `notes`.
    for format in ["cityparquet", "cityjsonseq"] {
        let count_rows: Vec<&Row> = rows
            .iter()
            .filter(|r| r.field("format") == format && r.field("scenario") == "count")
            .collect();
        assert_eq!(
            count_rows.len(),
            1,
            "expected exactly 1 count row for {format}"
        );

        let bbox_rows: Vec<&Row> = rows
            .iter()
            .filter(|r| r.field("format") == format && r.field("scenario") == "bbox-query")
            .collect();
        assert_eq!(
            bbox_rows.len(),
            3,
            "expected exactly 3 bbox-query rows (1pct/5pct/25pct) for {format}"
        );
        let tags: Vec<&str> = bbox_rows.iter().map(|r| r.field("notes")).collect();
        assert!(
            tags.contains(&"bbox-1pct"),
            "missing bbox-1pct row: {tags:?}"
        );
        assert!(
            tags.contains(&"bbox-5pct"),
            "missing bbox-5pct row: {tags:?}"
        );
        assert!(
            tags.contains(&"bbox-25pct"),
            "missing bbox-25pct row: {tags:?}"
        );
    }
}

#[test]
fn run_requires_repeat_at_least_one() {
    let prepared = tempfile::tempdir().unwrap();
    let input = fixture("lod3_railway.city.json");
    let package_dir = prepared.path().join("lod3_railway.parquet");
    convert(&ConvertOptions::new(input.clone(), package_dir)).unwrap();
    let out_csv = prepared.path().join("out.csv");

    let output = Command::new(env!("CARGO_BIN_EXE_cityparquet-readbench"))
        .arg("run")
        .args([
            "--input",
            input.to_str().unwrap(),
            "--prepared-dir",
            prepared.path().to_str().unwrap(),
            "--out",
            out_csv.to_str().unwrap(),
            "--repeat",
            "0",
            "--scenarios",
            "count",
            "--formats",
            "cityparquet",
        ])
        .output()
        .expect("failed to run the built cityparquet-readbench binary");

    assert!(
        !output.status.success(),
        "--repeat 0 must be rejected, not silently accepted"
    );
}

#[test]
fn run_skips_a_format_with_no_prepared_artefact_and_still_produces_the_other() {
    let prepared = tempfile::tempdir().unwrap();
    let input = fixture("lod3_railway.city.json");
    let package_dir = prepared.path().join("lod3_railway.parquet");
    convert(&ConvertOptions::new(input.clone(), package_dir)).unwrap();
    let out_csv = prepared.path().join("out.csv");

    // `flatcitybuf`'s .fcb artefact was never prepared in this tempdir.
    let output = run_coordinator(&[
        "--input",
        input.to_str().unwrap(),
        "--prepared-dir",
        prepared.path().to_str().unwrap(),
        "--out",
        out_csv.to_str().unwrap(),
        "--repeat",
        "1",
        "--scenarios",
        "count",
        "--formats",
        "cityparquet,flatcitybuf",
    ]);

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("flatcitybuf") && stderr.contains("readbench-prepare"),
        "expected a clear skip note for the missing flatcitybuf artefact; stderr:\n{stderr}"
    );

    let csv_text = std::fs::read_to_string(&out_csv).unwrap();
    let rows: Vec<Row> = csv_text.lines().skip(1).map(Row::parse).collect();
    assert_eq!(
        rows.len(),
        1,
        "only cityparquet's count row should be present when flatcitybuf's artefact is missing"
    );
    assert_eq!(rows[0].field("format"), "cityparquet");
}
