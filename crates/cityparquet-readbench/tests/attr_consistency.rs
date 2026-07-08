//! RED->GREEN (readbench Task 11, Commit A): `--attr-eq` must always mean
//! STRING equality, and every `FormatRunner` — `cityparquet`, `cityjsonseq`,
//! `flatcitybuf` — must therefore agree on the same real, numeric-looking
//! string-typed attribute code.
//!
//! `lod3_railway.city.json`'s `function` attribute is a STRING column whose
//! values are numeric-looking codes (e.g. `"1070"`); independently confirmed
//! with Python over the raw fixture: 65 of its 121 CityObjects have
//! `function == "1070"` (see `formats::flatcitybuf`'s own module doc for the
//! same count, cross-checked there against FCB's B+-tree).
//!
//! Before the fix, `main.rs::build_attr_pred` parsed a numeric-looking
//! `--attr-eq` value into a JSON *number*, which the CityJSONSeq runner's
//! `matches_predicate` then silently failed to match against the STRING
//! attribute cell (comparing `value.as_f64()`, always `None` for a JSON
//! string) — returning 0 instead of 65, while the CityParquet runner's
//! `query::attr_filter` rejected the same numeric `Eq` against its Utf8
//! column outright (a schema error, not a silent wrong answer), and the
//! FlatCityBuf runner's own `eq_key` locally re-stringified the number back
//! to `"1070"` and got the right answer anyway — three different behaviours
//! for the one shared `--attr-eq` flag. After the fix, `--attr-eq` always
//! produces `AttrPred::Eq(Value::String(_))`, and all three runners return
//! exactly 65.
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use cityparquet::package::{ConvertOptions, convert};

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

/// Whether the `fcb` CLI is on PATH — mirrors
/// `tests/flatcitybuf_runner.rs`'s own `fcb_cli_missing`, so the FCB leg of
/// this test skips gracefully rather than depending on an optional external
/// tool in every environment.
fn fcb_cli_missing() -> bool {
    Command::new("fcb")
        .arg("--version")
        .output()
        .map(|o| !o.status.success())
        .unwrap_or(true)
}

fn generate_fcb(fixture_name: &str, out_dir: &Path) -> PathBuf {
    let src = fixture(fixture_name);
    let out = out_dir.join(format!("{fixture_name}.fcb"));
    let output = Command::new("fcb")
        .arg("ser")
        .arg("-i")
        .arg(&src)
        .arg("-o")
        .arg(&out)
        .arg("-A")
        .output()
        .expect("failed to run `fcb ser` (PATH availability already checked)");
    assert!(
        output.status.success(),
        "fcb ser failed for {fixture_name}; stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    out
}

/// Runs the built `cityparquet-readbench` binary's `--child` protocol and
/// returns the parsed `result_count` — identical protocol to the other
/// runner test files (`tests/cityjsonseq_runner.rs`, `tests/flatcitybuf_runner.rs`).
fn run_child(format: &str, scenario: &str, input: &Path, extra_args: &[&str]) -> u64 {
    let mut args = vec!["--child", "--format", format, "--scenario", scenario];
    args.push("--input");
    let input_str = input.to_str().unwrap();
    args.push(input_str);
    args.extend_from_slice(extra_args);

    let output: Output = Command::new(env!("CARGO_BIN_EXE_cityparquet-readbench"))
        .args(&args)
        .output()
        .expect("failed to run the built cityparquet-readbench binary");

    assert!(
        output.status.success(),
        "child process exited non-zero (format={format}, scenario={scenario}); stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout must be valid UTF-8");
    let line = stdout.trim();
    let fields: Vec<&str> = line.split_whitespace().collect();
    assert_eq!(
        fields.len(),
        4,
        "expected exactly 4 whitespace-separated fields in '{line}'"
    );
    fields[3].parse().unwrap_or_else(|e| {
        panic!(
            "field 4 (result_count) '{}' did not parse as u64: {e}",
            fields[3]
        )
    })
}

#[test]
fn all_three_runners_agree_on_the_string_typed_numeric_attr_code() {
    const EXPECTED: u64 = 65;
    let attr_args: &[&str] = &["--attr-column", "function", "--attr-eq", "1070"];

    // --- CityParquet ---
    let out = tempfile::tempdir().unwrap();
    let opts = ConvertOptions::new(fixture("lod3_railway.city.json"), out.path().to_path_buf());
    let report = convert(&opts).unwrap();
    assert_eq!(report.object_count, 121);
    let cityparquet_count = run_child("cityparquet", "attr-filter", out.path(), attr_args);
    assert_eq!(
        cityparquet_count, EXPECTED,
        "cityparquet's attr-filter on function == '1070' must match the fixture's \
         known 65 CityObjects"
    );

    // --- CityJSONSeq (the format the bug actually manifested in) ---
    let cityjsonseq_count = run_child(
        "cityjsonseq",
        "attr-filter",
        &fixture("lod3_railway.city.json"),
        attr_args,
    );
    assert_eq!(
        cityjsonseq_count, EXPECTED,
        "cityjsonseq's attr-filter on function == '1070' must also match 65 — \
         before the --attr-eq fix this returned 0 (a JSON-number predicate can \
         never equal a JSON-string cell)"
    );

    // --- FlatCityBuf (skips gracefully if `fcb` isn't on PATH) ---
    let fcb_count = if fcb_cli_missing() {
        eprintln!("skipping FlatCityBuf leg: `fcb` CLI not found on PATH");
        None
    } else {
        let fcb_tmp = tempfile::tempdir().unwrap();
        let fcb_input = generate_fcb("lod3_railway.city.json", fcb_tmp.path());
        let count = run_child("flatcitybuf", "attr-filter", &fcb_input, attr_args);
        assert_eq!(
            count, EXPECTED,
            "flatcitybuf's attr-filter on function == '1070' must also match 65"
        );
        Some(count)
    };

    // Cross-runner agreement, spelled out explicitly rather than merely
    // implied by each runner's own assertion against the constant above.
    assert_eq!(cityparquet_count, cityjsonseq_count);
    if let Some(fcb_count) = fcb_count {
        assert_eq!(cityparquet_count, fcb_count);
        assert_eq!(cityjsonseq_count, fcb_count);
    }
}
