//! RED (readbench Task 9): the CityJSONSeq (+ gzipped-CityJSONSeq)
//! `FormatRunner`, exercised only through the BUILT `cityparquet-readbench`
//! binary's `--child` protocol — never calling into the runner's internals
//! directly — against the real `delft.city.jsonl` fixture (never inline
//! artificial CityJSON).
//!
//! The critical cross-format assertions here are the CityOBJECT-level
//! scenarios (`attr-filter`, `attr-stats`, `project`, `id-lookup`): they must
//! return EXACTLY the same `result_count` the CityParquet runner returns for
//! the same fixture (`crates/cityparquet/tests/query_real_data.rs` pins
//! delft's known split at `BuildingPart: 1116`, `Building: 1115`, and
//! `oorspronkelijkbouwjaar` present on exactly the 1115 `Building` rows).
//! `count`/`full-read` are deliberately FEATURE-level for this format (1115
//! top-level features in delft.city.jsonl — one per `Building`, each
//! carrying its `BuildingPart` child inline) and so intentionally differ
//! from CityParquet's 2231 (one row per CityObject, parents AND children).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

/// Gzip-compresses a real fixture into `out_dir` (never an inline artificial
/// CityJSON payload) and returns the compressed file's path.
fn gzip_fixture(name: &str, out_dir: &Path) -> PathBuf {
    let src = fixture(name);
    let data = std::fs::read(&src).expect("reading fixture to gzip");
    let out_path = out_dir.join(format!("{name}.gz"));
    let file = std::fs::File::create(&out_path).expect("creating gz output file");
    let mut encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
    encoder.write_all(&data).expect("writing gzip payload");
    encoder.finish().expect("finishing gzip stream");
    out_path
}

/// Runs the built `cityparquet-readbench` binary's `--child` protocol with
/// `extra_args` appended, asserts it exits successfully, and returns the
/// parsed `result_count` (field 4 of the 4-field stdout line).
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
fn count_and_full_read_are_feature_level_not_cityobject_level() {
    let input = fixture("delft.city.jsonl");
    let count = run_child("cityjsonseq", "count", &input, &[]);
    assert_eq!(
        count, 1115,
        "delft.city.jsonl has 1115 top-level features (one per Building); \
         CityParquet's own count for the same data is 2231 (one row per \
         CityObject, parents AND children) — a documented, deliberate \
         cross-format difference, not a bug"
    );

    let full_read = run_child("cityjsonseq", "full-read", &input, &[]);
    assert_eq!(
        full_read, 1115,
        "full-read is also feature-level for this format"
    );
}

#[test]
fn attr_filter_object_type_matches_cityparquets_known_buildingpart_count() {
    let input = fixture("delft.city.jsonl");
    let count = run_child(
        "cityjsonseq",
        "attr-filter",
        &input,
        &["--attr-column", "object_type", "--attr-eq", "BuildingPart"],
    );
    assert_eq!(
        count, 1116,
        "attr-filter is CityObject-level: must match CityParquet's own known \
         BuildingPart count (1116) exactly on the same fixture"
    );
}

#[test]
fn project_and_attr_stats_oorspronkelijkbouwjaar_match_cityparquets_known_count() {
    let input = fixture("delft.city.jsonl");

    let project_count = run_child(
        "cityjsonseq",
        "project",
        &input,
        &["--attr-column", "oorspronkelijkbouwjaar"],
    );
    assert_eq!(
        project_count, 1115,
        "project is CityObject-level: oorspronkelijkbouwjaar is present on \
         exactly delft's 1115 Building rows, matching CityParquet exactly"
    );

    let stats_count = run_child(
        "cityjsonseq",
        "attr-stats",
        &input,
        &["--attr-column", "oorspronkelijkbouwjaar"],
    );
    assert_eq!(
        stats_count, 1115,
        "attr-stats' non-null count must also match CityParquet's 1115"
    );
}

#[test]
fn id_lookup_finds_a_real_id_and_none_for_a_bogus_id() {
    let input = fixture("delft.city.jsonl");
    // A real Building CityObject id lifted directly from the fixture's first
    // feature line.
    let real_id = "NL.IMBAG.Pand.0503100000012869";

    let found = run_child(
        "cityjsonseq",
        "id-lookup",
        &input,
        &["--target-id", real_id],
    );
    assert_eq!(found, 1, "id-lookup must find a real delft object id");

    let missing = run_child(
        "cityjsonseq",
        "id-lookup",
        &input,
        &["--target-id", "no-such-id"],
    );
    assert_eq!(missing, 0, "id-lookup must return 0 for a bogus id");
}

#[test]
fn bbox_query_is_feature_level_and_uses_the_header_transform() {
    let input = fixture("delft.city.jsonl");
    // The full dataset extent, taken straight from delft.city.jsonl's header
    // `metadata.geographicalExtent` — every feature must intersect it.
    let whole_dataset = [
        "--bbox",
        "84501.5546875,445805.03125,-3.746997833251953,85675.234375,446983.46875,95.04200744628906",
    ];
    let all = run_child("cityjsonseq", "bbox-query", &input, &whole_dataset);
    assert_eq!(
        all, 1115,
        "a query window covering the whole dataset extent must match every \
         one of delft's 1115 features"
    );

    // A window far outside the dataset (middle of the North Sea) must match
    // nothing.
    let far_away = ["--bbox", "0,0,0,1,1,1"];
    let none = run_child("cityjsonseq", "bbox-query", &input, &far_away);
    assert_eq!(
        none, 0,
        "a query window outside the dataset must match none"
    );
}

#[test]
fn gzip_variant_matches_the_plain_variant_on_the_same_cityobject_level_scenarios() {
    let tmp = tempfile::tempdir().unwrap();
    let gz_input = gzip_fixture("delft.city.jsonl", tmp.path());

    let count = run_child("cityjsonseq-gz", "count", &gz_input, &[]);
    assert_eq!(
        count, 1115,
        "gzip variant must match the plain variant's feature count"
    );

    let attr_filter_count = run_child(
        "cityjsonseq-gz",
        "attr-filter",
        &gz_input,
        &["--attr-column", "object_type", "--attr-eq", "BuildingPart"],
    );
    assert_eq!(
        attr_filter_count, 1116,
        "gzip variant's attr-filter must match CityParquet's known BuildingPart count"
    );

    let project_count = run_child(
        "cityjsonseq-gz",
        "project",
        &gz_input,
        &["--attr-column", "oorspronkelijkbouwjaar"],
    );
    assert_eq!(
        project_count, 1115,
        "gzip variant's project count must match CityParquet's known count"
    );

    let id_found = run_child(
        "cityjsonseq-gz",
        "id-lookup",
        &gz_input,
        &["--target-id", "NL.IMBAG.Pand.0503100000012869"],
    );
    assert_eq!(
        id_found, 1,
        "gzip variant's id-lookup must find the real id"
    );
}
