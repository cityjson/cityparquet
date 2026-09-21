//! RED (readbench Task 10): the FlatCityBuf (FCB) `FormatRunner`, exercised
//! only through the BUILT `cityparquet-readbench` binary's `--child`
//! protocol — never calling into the runner's internals directly — against
//! a real `.fcb` generated (via the `fcb` CLI) from `lod3_railway.city.json`
//! (never inline artificial CityJSON).
//!
//! **Granularity — see `formats::flatcitybuf`'s own module doc for the full
//! rationale, empirically confirmed by the assertions below, not merely
//! assumed.** `fcb ser` builds one `CityFeature` per top-level CityObject,
//! so `lod3_railway.city.json` (121 CityObjects in its single CityJSON
//! document) becomes an `.fcb` file with 38 features (confirmed via `fcb
//! info`) — `Count`/`FullRead`/`BBoxQuery` count at that feature level.
//! `AttrFilter`/`AttrStats`/`Project` count at CityObject level instead:
//! FCB's own B+-tree attribute index returns one match per matching
//! CityObject occurrence (not deduplicated by feature), and this runner's
//! own `select_all`-walk fallback deliberately matches that same
//! granularity. Independently verified against the fixture with Python
//! (`python3` over the raw CityJSON): 65 of 121 CityObjects have
//! `function == "1070"`, and 94 of 121 carry a `function` value at all —
//! matched exactly below. None of this tries to reproduce CityParquet's own
//! CityObject-ROW counts (CityParquet counts parents AND children as table
//! rows directly; this fixture's own CityParquet ingestion is out of scope
//! here).
//!
//! Every test shells the real `fcb` CLI (`fcb ser -A`) to build its own
//! `.fcb` fixture in a tempdir — `.fcb` files are never committed. If `fcb`
//! isn't on PATH (optional external tool, not vendored into this repo),
//! every test here SKIPS gracefully (an `eprintln!` + early `return`, never
//! a failure) rather than depending on network/tool availability in CI.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../lib/cityparquet-rs/tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

/// Whether the `fcb` CLI (`fcb_core`'s own conversion tool) is on PATH —
/// checked once per test so every test can skip gracefully rather than
/// fail when the optional external tool (or the network needed to install
/// it) isn't available, mirroring how this milestone's other optional
/// external-tool-dependent tests guard themselves.
fn fcb_cli_missing() -> bool {
    Command::new("fcb")
        .arg("--version")
        .output()
        .map(|o| !o.status.success())
        .unwrap_or(true)
}

/// Converts `fixture_name` (a real CityJSON/CityJSONSeq fixture, never an
/// inline artificial payload) to a `.fcb` file inside `out_dir` via the real
/// `fcb` CLI, with BOTH a spatial R-tree and an all-attribute B+-tree index
/// (`-A`) — the same invocation `just readbench-prepare` uses. Panics if
/// `fcb` is on PATH but the conversion itself fails (a real bug, not a
/// missing-tool skip); callers must check [`fcb_cli_missing`] first.
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

/// Runs the built `cityparquet-readbench` binary's `--child` protocol with
/// `extra_args` appended, asserts it exits successfully, and returns the
/// parsed `result_count` (field 4 of the 4-field stdout line) — identical
/// protocol to `tests/cityjsonseq_runner.rs`'s own `run_child`.
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
fn count_and_full_read_match_fcb_infos_own_feature_count() {
    if fcb_cli_missing() {
        eprintln!("skipping: `fcb` CLI not found on PATH");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let input = generate_fcb("lod3_railway.city.json", tmp.path());

    // Confirmed via `fcb info` on this exact conversion: 38 features (one
    // per top-level CityObject) from the fixture's 121 CityObjects.
    let count = run_child("flatcitybuf", "count", &input, &[]);
    assert_eq!(
        count, 38,
        "fcb info reports 38 features for lod3_railway.city.json; \
         Count must read this from header metadata alone, no scan"
    );

    let full_read = run_child("flatcitybuf", "full-read", &input, &[]);
    assert_eq!(
        full_read, 38,
        "full-read must decode and count the same 38 features"
    );
}

#[test]
fn bbox_query_quarter_window_is_a_proper_nonempty_subset() {
    if fcb_cli_missing() {
        eprintln!("skipping: `fcb` CLI not found on PATH");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let input = generate_fcb("lod3_railway.city.json", tmp.path());

    // Whole-dataset extent (confirmed via `fcb info`): x in [0.56, 12.64],
    // y in [0.64, 7.68]. Every feature must match this.
    let whole = ["--bbox", "0.56,0.64,-1000,12.64,7.68,1000"];
    let all = run_child("flatcitybuf", "bbox-query", &input, &whole);
    assert_eq!(
        all, 38,
        "a query window covering the whole dataset extent must match every feature"
    );

    // A window far outside the dataset extent must match none.
    let far_away = ["--bbox", "1000,1000,-1000,1001,1001,1000"];
    let none = run_child("flatcitybuf", "bbox-query", &input, &far_away);
    assert_eq!(
        none, 0,
        "a query window outside the dataset must match none"
    );

    // A ~25% (bottom-left quadrant) window: internal consistency + sanity
    // bounds only (NOT an exact cross-format count — see this module's own
    // doc comment).
    let quarter = ["--bbox", "0.56,0.64,-1000,6.6,4.16,1000"];
    let partial = run_child("flatcitybuf", "bbox-query", &input, &quarter);
    assert!(
        partial < all,
        "a quarter-area window must match strictly fewer features than the \
         whole-dataset window (got {partial}, whole dataset is {all})"
    );
}

#[test]
fn attr_filter_on_an_indexed_string_column_matches_the_known_cityobject_count() {
    if fcb_cli_missing() {
        eprintln!("skipping: `fcb` CLI not found on PATH");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let input = generate_fcb("lod3_railway.city.json", tmp.path());

    // `function` is one of `fcb info`'s confirmed B+-tree attribute indices
    // for this fixture; "1070" is a real value, independently counted with
    // Python over the raw CityJSON: exactly 65 of the 121 CityObjects
    // (BuildingInstallation/TunnelInstallation/CityFurniture carry it;
    // Railway/Tunnel/Bridge/etc. do not) — CityObject-level, per this
    // module's own doc comment, NOT the 38-feature total.
    let matched = run_child(
        "flatcitybuf",
        "attr-filter",
        &input,
        &["--attr-column", "function", "--attr-eq", "1070"],
    );
    assert_eq!(
        matched, 65,
        "attr-filter on function == '1070' must match the fixture's own \
         known 65 CityObjects exactly (FCB's B+-tree is CityObject-level, \
         not feature-level, for this scenario)"
    );
}

#[test]
fn project_on_the_same_indexed_column_matches_the_known_cityobject_count() {
    if fcb_cli_missing() {
        eprintln!("skipping: `fcb` CLI not found on PATH");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let input = generate_fcb("lod3_railway.city.json", tmp.path());

    // project always takes the full `select_all` walk (no columnar
    // aggregation in FCB); independently counted with Python: 94 of 121
    // CityObjects carry a `function` value at all.
    let projected = run_child(
        "flatcitybuf",
        "project",
        &input,
        &["--attr-column", "function"],
    );
    assert_eq!(
        projected, 94,
        "project on function must match the fixture's own known 94 \
         CityObjects exactly (CityObject-level, matching attr-filter's own \
         granularity)"
    );
}

#[test]
fn id_lookup_finds_a_real_object_id_and_none_for_a_bogus_id() {
    if fcb_cli_missing() {
        eprintln!("skipping: `fcb` CLI not found on PATH");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let input = generate_fcb("lod3_railway.city.json", tmp.path());

    // A real CityObject id lifted directly from the fixture.
    let real_id = "UUID_bd865e62-18de-40ff-85da-883709a86f0f";
    let found = run_child(
        "flatcitybuf",
        "id-lookup",
        &input,
        &["--target-id", real_id],
    );
    assert_eq!(found, 1, "id-lookup must find a real railway object id");

    let missing = run_child(
        "flatcitybuf",
        "id-lookup",
        &input,
        &["--target-id", "no-such-id"],
    );
    assert_eq!(missing, 0, "id-lookup must return 0 for a bogus id");
}
