//! RED->GREEN (readbench Task 11, Commit A): `--attr-eq` must always mean
//! STRING equality, and every `FormatRunner` — `cityparquet`, `cityjson`,
//! `cityjsonseq`, `flatcitybuf` — must therefore agree on the same real,
//! numeric-looking string-typed attribute code.
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
//! produces `AttrPred::Eq(Value::String(_))`.
//!
//! **CityParquet leg split off (2026-07-21, mandatory-by-type-layout):**
//! the single-file table layout is gone, so converting
//! `lod3_railway.city.json` (10 1st-level families) now always writes 10
//! separate family tables, and the
//! `cityparquet` `FormatRunner`'s single-file `locate_main_table` correctly
//! rejects that package outright rather than silently reading only one
//! family's rows — see `cityparquet_attr_filter_rejects_a_multi_family_by_type_package`
//! below, which pins that rejection. The CityJSONSeq/FlatCityBuf legs never
//! go through `convert()` at all (they read the raw fixture / a `.fcb`
//! export of it directly), so they are unaffected and keep proving the
//! original `--attr-eq` regression stays fixed, in
//! `cityjson_cityjsonseq_and_flatcitybuf_agree_on_the_string_typed_numeric_attr_code`
//! below — which the plain-CityJSON runner joined when it was added, because
//! THIS file, not any single runner's own test file, is the designated
//! cross-format attribute-semantics guard: a new runner has to satisfy it
//! too. The `citygml` runner joined in
//! `citygml_agrees_on_the_string_typed_numeric_attr_code`, on a CityGML
//! fragment of the SAME upstream Railway dataset — it cannot read the
//! CityJSON file itself (and must refuse it), but it can be held to the same
//! `--attr-eq` semantics on the same `function` codes. The full three-way
//! cross-check
//! (`all_three_runners_agree_on_the_string_typed_numeric_attr_code`) is kept
//! as an `#[ignore]`d, documented gap until a follow-up plan teaches the
//! readbench runners to aggregate across every table in a by-type
//! package's manifest (the same deferral this task's brief makes for
//! `export`).
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use cityparquet::package::{ConvertOptions, convert};

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../lib/cityparquet-rs/tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

/// The real `lod3_railway.city.json` fixture carries no `referenceSystem` at
/// all. Coordinate-bearing input with no resolvable CRS converts to an
/// explicit `city.crs: null` rather than failing (spec "CRS rules": "an
/// unresolvable CRS is declared, not fatal"), so tests below that want a
/// GEOREFERENCED railway conversion (or comparison) use a small on-disk COPY
/// with a CRS injected via JSON mutation of the real fixture — never
/// hand-written CityJSON.
fn railway_fixture_with_crs() -> (tempfile::TempDir, PathBuf) {
    let mut doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(fixture("lod3_railway.city.json")).unwrap())
            .unwrap();
    doc["metadata"]["referenceSystem"] =
        serde_json::json!("https://www.opengis.net/def/crs/EPSG/0/7415");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("railway_with_crs.city.json");
    std::fs::write(&path, serde_json::to_string(&doc).unwrap()).unwrap();
    (dir, path)
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
        .arg(&src)
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

/// Runs the built binary's `--child` protocol exactly like [`run_child`],
/// but asserts the child process FAILS and returns its stderr — for
/// pinning a deliberate rejection rather than a successful `result_count`.
fn run_child_expect_failure(
    format: &str,
    scenario: &str,
    input: &Path,
    extra_args: &[&str],
) -> String {
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
        !output.status.success(),
        "child process (format={format}, scenario={scenario}) was expected to fail but exited \
         successfully"
    );
    String::from_utf8(output.stderr).expect("stderr must be valid UTF-8")
}

/// The non-CityParquet legs of the original regression pin: none of them
/// goes through `convert()`, so all are unaffected by the single-file table
/// layout's removal and keep proving `--attr-eq` always means STRING
/// equality (see this module's own doc comment on the historical bug, which
/// manifested in the CityJSONSeq runner).
///
/// The `cityjson` runner joined this matrix when it was added: this file —
/// not any single runner's own test file — is the designated cross-format
/// attribute-semantics guard, so every new runner has to satisfy it too.
/// `cityjson` and `cityjsonseq` read the very SAME `.city.json` document
/// here, from opposite parse shapes (whole document vs. line-oriented), and
/// must still agree object-for-object.
#[test]
fn cityjson_cityjsonseq_and_flatcitybuf_agree_on_the_string_typed_numeric_attr_code() {
    const EXPECTED: u64 = 65;
    let attr_args: &[&str] = &["--attr-column", "function", "--attr-eq", "1070"];

    // --- plain CityJSON (whole-document parse of the same fixture) ---
    let cityjson_count = run_child(
        "cityjson",
        "attr-filter",
        &fixture("lod3_railway.city.json"),
        attr_args,
    );
    assert_eq!(
        cityjson_count, EXPECTED,
        "cityjson's attr-filter on function == '1070' must match the fixture's \
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
    assert_eq!(
        cityjson_count, cityjsonseq_count,
        "the two JSON runners read the very same document from opposite parse \
         shapes and must agree exactly"
    );

    // --- FlatCityBuf (skips gracefully if `fcb` isn't on PATH) ---
    if fcb_cli_missing() {
        eprintln!("skipping FlatCityBuf leg: `fcb` CLI not found on PATH");
        return;
    }
    let fcb_tmp = tempfile::tempdir().unwrap();
    let fcb_input = generate_fcb("lod3_railway.city.json", fcb_tmp.path());
    let fcb_count = run_child("flatcitybuf", "attr-filter", &fcb_input, attr_args);
    assert_eq!(
        fcb_count, EXPECTED,
        "flatcitybuf's attr-filter on function == '1070' must also match 65"
    );
    assert_eq!(cityjsonseq_count, fcb_count);
    assert_eq!(cityjson_count, fcb_count);
}

/// Single-file table layout removal (2026-07-21, mandatory-by-type-layout):
/// `lod3_railway.city.json` has 10 1st-level families, so `convert()` now
/// always writes 10 separate family tables — there is no longer a single
/// file holding the whole dataset. The `cityparquet` `FormatRunner`'s
/// `locate_main_table` must reject that package with a clear diagnostic
/// (never a panic, and never a silently-wrong count from just one family's
/// table) — pinned here with a real 10-table package, not a hand-rolled one.
#[test]
fn cityparquet_attr_filter_rejects_a_multi_family_by_type_package() {
    let out = tempfile::tempdir().unwrap();
    let (_crs_dir, railway_path) = railway_fixture_with_crs();
    let opts = ConvertOptions::new(railway_path, out.path().to_path_buf());
    let report = convert(&opts).unwrap();
    assert_eq!(report.object_count, 121);

    let attr_args: &[&str] = &["--attr-column", "function", "--attr-eq", "1070"];
    let stderr = run_child_expect_failure("cityparquet", "attr-filter", out.path(), attr_args);
    assert!(
        stderr.contains("tables") && stderr.contains("single-table"),
        "expected a clear multi-table rejection, got stderr:\n{stderr}"
    );
}

/// The `citygml` runner's leg of the same guard.
///
/// It cannot share the other runners' input: `lod3_railway.city.json` is
/// CityJSON, and `--format citygml` must REFUSE it rather than measure another
/// format's cost under this format's name (see `formats::citygml`'s
/// `open_citygml`). What it can share is the DATA: `railway_lod3_fragment.gml`
/// is a fragment of the very same upstream CityGML 2.0 "Railway" reference
/// dataset that `lod3_railway.city.json` was converted from, and it carries
/// the same string-typed, numeric-LOOKING `function` codes — including
/// `"1070"`, the exact value this whole file exists to pin.
///
/// So the count differs (a 4-member fragment, not the whole dataset) while the
/// SEMANTICS asserted are identical: `--attr-eq 1070` is a STRING comparison,
/// and a runner that parsed it as a JSON number would return 0 here just as
/// the CityJSONSeq runner once did.
///
/// The `function` values live on the Building's two
/// `outerBuildingInstallation` children (1070 and 1040), which also proves
/// this runner's attribute scenarios reach NESTED CityObjects — `count` on the
/// same file reports 4 members and never sees them.
#[test]
fn citygml_agrees_on_the_string_typed_numeric_attr_code() {
    let fragment = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../lib/cityparquet-rs/crates/core/tests/data/railway_lod3_fragment.gml");
    assert!(fragment.exists(), "missing committed CityGML fragment");

    let matched = run_child(
        "citygml",
        "attr-filter",
        &fragment,
        &["--attr-column", "function", "--attr-eq", "1070"],
    );
    assert_eq!(
        matched, 1,
        "exactly one BuildingInstallation in the fragment has function == \
         '1070' (the other has '1040'); a numeric-parsed predicate would \
         return 0"
    );

    let other = run_child(
        "citygml",
        "attr-filter",
        &fragment,
        &["--attr-column", "function", "--attr-eq", "1040"],
    );
    assert_eq!(other, 1, "and the sibling installation carries '1040'");

    let absent = run_child(
        "citygml",
        "attr-filter",
        &fragment,
        &["--attr-column", "function", "--attr-eq", "9999"],
    );
    assert_eq!(absent, 0, "a code no object carries must match nothing");
}

/// DEFERRED (2026-07-21, feat/mandatory-bytype-layout): the single-file
/// table layout was removed, so `convert()` on this 10-family fixture now
/// always writes 10 tables, and readbench's single-table `locate_main_table`
/// correctly bails.
/// Re-enable when a follow-up readbench plan teaches the runners to
/// aggregate across every table in the manifest (the same deferral as the
/// task-3 brief's "a later plan rebinds export").
#[test]
#[ignore = "readbench cannot yet query multi-table (by-type) packages; see doc comment"]
fn all_three_runners_agree_on_the_string_typed_numeric_attr_code() {
    const EXPECTED: u64 = 65;
    let attr_args: &[&str] = &["--attr-column", "function", "--attr-eq", "1070"];

    // --- CityParquet ---
    let out = tempfile::tempdir().unwrap();
    let (_crs_dir, railway_path) = railway_fixture_with_crs();
    let opts = ConvertOptions::new(railway_path, out.path().to_path_buf());
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
