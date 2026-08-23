//! RED (format-comparison Task 3): the plain-CityJSON (`cityjson`)
//! `FormatRunner`, exercised only through the BUILT `cityparquet-readbench`
//! binary's `--child` protocol — never calling into the runner's internals
//! directly — against the real `lod3_railway.city.json` fixture (a genuine
//! whole-document CityJSON, never inline artificial CityJSON).
//!
//! **The counting grain is the point of this file.** A plain CityJSON
//! document's natural unit is a `CityObjects` MAP ENTRY, so `count` and
//! `full-read` report 121 for this fixture — parents AND second-level
//! children (56 `BuildingInstallation`s, 8 `TunnelInstallation`s,
//! 3 `BridgeConstructiveElement`s, 2 `BridgeInstallation`s) each counted in
//! their own right. The `cityjsonseq` runner reading the very SAME file
//! reports 38, because its own grain is the top-level FEATURE. Both numbers
//! are honest; they answer different questions. That difference is asserted
//! here (not merely documented in a doc comment) so the disclosure cannot
//! rot.
//!
//! Every expected count below was derived independently from the raw fixture
//! with Python, not from this runner's own output.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../lib/cityparquet-rs/tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

/// Runs the built `cityparquet-readbench` binary's `--child` protocol with
/// `extra_args` appended, asserts it exits successfully, and returns the
/// parsed `result_count` (field 4 of the 4-field stdout line) — identical
/// protocol to the other runner test files (`tests/cityjsonseq_runner.rs`,
/// `tests/flatcitybuf_runner.rs`).
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

/// The counting-grain disclosure, enforced: `cityjson` counts `CityObjects`
/// map entries (121), while `cityjsonseq` reading the SAME document counts
/// top-level features (38). Asserting both here means the module doc's claim
/// cannot silently become false.
#[test]
fn count_and_full_read_are_cityobject_map_entries_not_top_level_features() {
    let input = fixture("lod3_railway.city.json");

    let count = run_child("cityjson", "count", &input, &[]);
    assert_eq!(
        count, 121,
        "lod3_railway.city.json's CityObjects map has 121 entries — parents \
         AND second-level children (BuildingInstallation, TunnelInstallation, \
         BridgeConstructiveElement, BridgeInstallation) each counted in their \
         own right"
    );

    let full_read = run_child("cityjson", "full-read", &input, &[]);
    assert_eq!(
        full_read, 121,
        "full-read is also CityObject-level for this format"
    );

    let seq_count = run_child("cityjsonseq", "count", &input, &[]);
    assert_eq!(
        seq_count, 38,
        "the cityjsonseq runner reading the very same document counts its 38 \
         TOP-LEVEL features instead — a documented, deliberate cross-format \
         difference in counting grain, not a bug"
    );
    assert_ne!(
        count, seq_count,
        "the two grains must stay genuinely different on this fixture, or this \
         test is no longer testing anything"
    );
}

/// The OTHER disclosure this runner owes the paper, enforced the same way:
/// `full-read` is not the same operation in the two JSON runners.
/// `cityjsonseq`'s walks each geometry's boundary-index tree; this one
/// additionally resolves every leaf through the document-level `vertices`
/// array and `transform` (measurably more work — see the module doc). Both
/// are honest for their own format, but a CSV row labelled `full-read` must
/// not be read as "both formats did the same thing", so the module doc has
/// to say so in as many words. There is no runtime signal a reader of the
/// CSV could check instead, so this asserts against the module's own doc
/// block — the disclosure cannot be deleted without a test going red.
#[test]
fn the_module_doc_discloses_that_full_read_differs_from_cityjsonseqs() {
    const SOURCE: &str = include_str!("../src/formats/cityjson.rs");
    // The leading `//!` block, i.e. everything before the first `use`.
    let module_doc = SOURCE
        .split("\nuse ")
        .next()
        .expect("the module always has a doc block above its first `use`");

    for needle in [
        "not the same operation",
        "resolves every boundary leaf",
        "cityjsonseq",
    ] {
        assert!(
            module_doc.contains(needle),
            "the module doc must disclose that full-read resolves coordinates \
             while cityjsonseq's only traverses boundary indices; missing: \
             '{needle}'"
        );
    }
}

/// Second-level objects are first-class rows for this runner: the fixture's
/// 56 `BuildingInstallation`s are children of its `Building`s and are counted
/// individually.
#[test]
fn attr_filter_on_object_type_counts_second_level_objects() {
    let input = fixture("lod3_railway.city.json");
    let count = run_child(
        "cityjson",
        "attr-filter",
        &input,
        &[
            "--attr-column",
            "object_type",
            "--attr-eq",
            "BuildingInstallation",
        ],
    );
    assert_eq!(
        count, 56,
        "the fixture has 56 BuildingInstallation CityObjects, all of them \
         second-level children — this runner counts them in their own right"
    );
}

/// Cross-format agreement on a real, string-typed but numeric-LOOKING
/// attribute code: the same 65 the `cityjsonseq` and FlatCityBuf runners
/// already pin for this fixture (see `tests/attr_consistency.rs`).
#[test]
fn attr_filter_matches_the_other_runners_on_the_string_typed_numeric_code() {
    let input = fixture("lod3_railway.city.json");
    let attr_args: &[&str] = &["--attr-column", "function", "--attr-eq", "1070"];

    let cityjson_count = run_child("cityjson", "attr-filter", &input, attr_args);
    assert_eq!(
        cityjson_count, 65,
        "65 of the fixture's 121 CityObjects have function == '1070'"
    );

    let cityjsonseq_count = run_child("cityjsonseq", "attr-filter", &input, attr_args);
    assert_eq!(
        cityjson_count, cityjsonseq_count,
        "attr-filter is CityObject-level in BOTH JSON runners, so the two must \
         agree exactly on the same document"
    );
}

/// `project` counts every non-null value of a column; `attr-stats` counts
/// only the NUMERIC ones. `function` is a string column here (its values are
/// numeric-looking codes such as `"1070"`, not numbers), so the two answers
/// legitimately differ — pinned rather than papered over.
#[test]
fn project_counts_non_null_values_and_attr_stats_counts_only_numeric_ones() {
    let input = fixture("lod3_railway.city.json");

    let project_count = run_child(
        "cityjson",
        "project",
        &input,
        &["--attr-column", "function"],
    );
    assert_eq!(
        project_count, 94,
        "94 of the fixture's 121 CityObjects carry a non-null `function`"
    );

    let stats_count = run_child(
        "cityjson",
        "attr-stats",
        &input,
        &["--attr-column", "function"],
    );
    assert_eq!(
        stats_count, 0,
        "attr-stats aggregates NUMERIC values only, and this fixture's \
         `function` is a string column — 0 is the honest answer, matching the \
         cityjsonseq runner's own semantics on the same data"
    );

    let species_count = run_child("cityjson", "project", &input, &["--attr-column", "species"]);
    assert_eq!(
        species_count, 15,
        "the 15 SolitaryVegetationObjects are the only carriers of `species`"
    );
}

#[test]
fn id_lookup_finds_a_real_id_and_none_for_a_bogus_id() {
    let input = fixture("lod3_railway.city.json");
    // A real CityObject id lifted directly from the fixture's own
    // `CityObjects` map.
    let real_id = "UUID_bd865e62-18de-40ff-85da-883709a86f0f";

    let found = run_child("cityjson", "id-lookup", &input, &["--target-id", real_id]);
    assert_eq!(found, 1, "id-lookup must find a real railway object id");

    let missing = run_child(
        "cityjson",
        "id-lookup",
        &input,
        &["--target-id", "no-such-id"],
    );
    assert_eq!(missing, 0, "id-lookup must return 0 for a bogus id");
}

/// Per-object bboxes come from the DOCUMENT-level `vertices` array (a plain
/// CityJSON document shares one vertex list across every object) decoded
/// through the document's own `transform`.
#[test]
fn bbox_query_uses_the_document_level_vertices_and_transform() {
    let input = fixture("lod3_railway.city.json");

    // The full dataset extent, taken straight from the fixture's own
    // `metadata.geographicalExtent`.
    let whole_dataset = ["--bbox", "0.56,0.64,7.579,12.64,7.68,9.103"];
    let all = run_child("cityjson", "bbox-query", &input, &whole_dataset);
    assert_eq!(
        all, 120,
        "a window covering the whole extent matches all 120 geometry-bearing \
         CityObjects; the one CityObjectGroup that carries no geometry at all \
         has no bbox to intersect and is honestly excluded"
    );

    // The western half of the same extent: a genuine sub-selection, so this
    // scenario is proven to filter rather than merely return everything.
    let west_half = ["--bbox", "0.56,0.64,7.579,6.6,7.68,9.103"];
    let west = run_child("cityjson", "bbox-query", &input, &west_half);
    assert_eq!(
        west, 93,
        "93 CityObjects intersect the western half of the extent"
    );

    // A window far outside the dataset must match nothing.
    let far_away = ["--bbox", "100,100,100,101,101,101"];
    let none = run_child("cityjson", "bbox-query", &input, &far_away);
    assert_eq!(none, 0, "a window outside the dataset must match none");
}
