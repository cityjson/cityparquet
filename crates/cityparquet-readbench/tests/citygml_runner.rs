//! RED (format-comparison Task 4): the CityGML (`citygml`) `FormatRunner`,
//! exercised only through the BUILT `cityparquet-readbench` binary's `--child`
//! protocol — never calling into the runner's internals directly — against
//! real CityGML 2.0 fixtures (never inline hand-written GML).
//!
//! **What the `citygml` row means.** CityGML carries no index of any kind, so
//! every scenario below is a full XML parse plus an in-memory filter. That is
//! the point of the row, not a shortcoming of it: it answers "what does it
//! cost to answer this query against the format the data actually ships in,
//! using the same codebase as every other row?". It is NOT a claim about the
//! format's theoretical ceiling — a different parser would give different
//! numbers.
//!
//! **The counting grain is asserted here, not merely documented.** This
//! runner's grain is `cityjsonseq`'s: `count`/`full-read`/`bbox-query` count
//! top-level `cityObjectMember`s (one per 1st-level CityObject the reader
//! supports), while `attr-filter`/`attr-stats`/`project`/`id-lookup` are
//! CityOBJECT-level and therefore also see nested children (BuildingParts,
//! BuildingInstallations). `railway_lod3_fragment.gml` proves the two grains
//! genuinely differ: 4 members, but 2 of its CityObjects (both
//! `BuildingInstallation`s) live INSIDE one of those members and are never
//! counted by `count`.
//!
//! Every expected number below was derived independently from the raw XML
//! with Python (`ElementTree`), never snapshotted from this runner's own
//! output.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A fixture fetched by `just fixtures` into the workspace's own
/// `tests/fixtures/`.
fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

/// A committed (in-repo) CityGML fixture under `crates/cityparquet/tests/data/`
/// — small REAL fragments carrying their own provenance/licence headers, the
/// same helper `crates/cityparquet/tests/citygml_real_data.rs` already uses.
///
/// `just fixtures` only fetches two CityGML 2.0 files, both a single
/// attribute-less, id-less building; neither can express a counting-grain
/// difference or a real attribute query. Reaching across to the reader crate's
/// own committed real fragments is deliberate: they are real published data
/// (SAVeNoW Ingolstadt LoD2, CC BY 4.0; the KIT/IAI CityGML 2.0 "Railway"
/// reference dataset), which the house rule demands, where an inline
/// hand-written document would not be.
fn data_fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../cityparquet/tests/data")
        .join(name);
    assert!(p.exists(), "missing committed fixture {name}");
    p
}

/// Runs the built `cityparquet-readbench` binary's `--child` protocol with
/// `extra_args` appended, asserts it exits successfully, and returns the
/// parsed `result_count` (field 4 of the 4-field stdout line) — identical
/// protocol to the other runner test files (`tests/cityjson_runner.rs`,
/// `tests/cityjsonseq_runner.rs`, `tests/flatcitybuf_runner.rs`).
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

/// Like [`run_child`], but asserts the child FAILS and returns its stderr —
/// for pinning a deliberate refusal rather than a `result_count`.
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
         successfully with stdout:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );
    String::from_utf8(output.stderr).expect("stderr must be valid UTF-8")
}

// ---------------------------------------------------------------------------
// The brief's named fixture: all seven scenarios against one real building.
// ---------------------------------------------------------------------------

/// `b1_lod2_cs_w_sem.gml` is one `cityObjectMember` holding one
/// `bldg:Building` (a lod2 `gml:CompositeSolid` plus nine `bldg:boundedBy`
/// semantic surfaces). Semantic surfaces are geometry semantics, NOT
/// CityObjects, so the whole document is exactly one object at every grain.
#[test]
fn count_and_full_read_are_top_level_city_object_members() {
    let input = fixture("b1_lod2_cs_w_sem.gml");
    assert_eq!(
        run_child("citygml", "count", &input, &[]),
        1,
        "the fixture holds exactly one cityObjectMember (one bldg:Building); \
         its nine boundedBy semantic surfaces are geometry semantics, not \
         CityObjects"
    );
    assert_eq!(
        run_child("citygml", "full-read", &input, &[]),
        1,
        "full-read is member-level too — it parses every coordinate but reports \
         the same unit as count"
    );
}

/// The remaining five scenarios on the same fixture. It carries no attributes
/// and no `gml:id` at all (an upstream property of this hand-published
/// libcitygml sample, not an oversight here), so the attribute scenarios
/// honestly report 0 and the id is the reader's own deterministic synthesis.
#[test]
fn every_scenario_answers_the_single_building_fixture() {
    let input = fixture("b1_lod2_cs_w_sem.gml");

    // attr-filter: the reserved `object_type` column is always present.
    assert_eq!(
        run_child(
            "citygml",
            "attr-filter",
            &input,
            &["--attr-column", "object_type", "--attr-eq", "Building"],
        ),
        1,
        "the fixture's lone CityObject is a Building"
    );
    assert_eq!(
        run_child(
            "citygml",
            "attr-filter",
            &input,
            &["--attr-column", "object_type", "--attr-eq", "Bridge"],
        ),
        0,
        "there is no Bridge in this fixture"
    );

    // attr-stats / project: the fixture declares no attributes whatsoever, so
    // 0 is the honest answer rather than a skipped scenario.
    assert_eq!(
        run_child(
            "citygml",
            "attr-stats",
            &input,
            &["--attr-column", "measuredHeight"],
        ),
        0,
        "the fixture declares no bldg:measuredHeight (nor any other attribute)"
    );
    assert_eq!(
        run_child("citygml", "project", &input, &["--attr-column", "function"]),
        0,
        "the fixture declares no bldg:function"
    );

    // id-lookup: no `gml:id` on the building, so the reader synthesises the
    // deterministic `Building_<index>` (1-based) id.
    assert_eq!(
        run_child(
            "citygml",
            "id-lookup",
            &input,
            &["--target-id", "Building_1"],
        ),
        1,
        "an id-less CityGML building is named Building_<index> by the reader"
    );
    assert_eq!(
        run_child(
            "citygml",
            "id-lookup",
            &input,
            &["--target-id", "no-such-id"]
        ),
        0,
        "id-lookup must return 0 for a bogus id"
    );

    // bbox-query: the fixture's gml:pos coordinates span (0,0,0)-(100,100,150)
    // and it declares no gml:Envelope, so the reader quantises against a
    // [0,0,0] origin and the window is in raw coordinates.
    // (A negative lower corner would be swallowed by clap as a flag, so the
    // window starts at the fixture's own origin — which its geometry touches.)
    assert_eq!(
        run_child(
            "citygml",
            "bbox-query",
            &input,
            &["--bbox", "0,0,0,101,101,151"],
        ),
        1,
        "a window covering the fixture's whole coordinate span matches it"
    );
    assert_eq!(
        run_child(
            "citygml",
            "bbox-query",
            &input,
            &["--bbox", "1000,1000,1000,1001,1001,1001"],
        ),
        0,
        "a window far outside the fixture must match nothing"
    );
}

// ---------------------------------------------------------------------------
// Counting grain: members vs. CityObjects, on a real multi-module fragment.
// ---------------------------------------------------------------------------

/// `railway_lod3_fragment.gml` has four `cityObjectMember`s — a
/// `bldg:Building`, a `brid:Bridge`, a `veg:SolitaryVegetationObject` and a
/// `grp:CityObjectGroup` — and the Building holds two
/// `bldg:outerBuildingInstallation`s. So `count` reports 4 while the
/// object-level `attr-filter` sees the 2 nested `BuildingInstallation`s that
/// `count` never counted. Asserting BOTH is what stops the module doc's
/// grain disclosure from silently rotting.
#[test]
fn count_is_member_level_while_attr_scenarios_reach_nested_city_objects() {
    let input = data_fixture("railway_lod3_fragment.gml");

    let members = run_child("citygml", "count", &input, &[]);
    assert_eq!(
        members, 4,
        "four cityObjectMembers: Building, Bridge, SolitaryVegetationObject, \
         CityObjectGroup"
    );
    assert_eq!(
        run_child("citygml", "full-read", &input, &[]),
        4,
        "full-read shares count's member-level grain"
    );

    let installations = run_child(
        "citygml",
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
        installations, 2,
        "the Building's two outerBuildingInstallations are CityObjects in their \
         own right for the object-level scenarios"
    );
    assert_eq!(
        run_child(
            "citygml",
            "attr-filter",
            &input,
            &["--attr-column", "object_type", "--attr-eq", "Building"],
        ),
        1,
        "exactly one of the four members is a Building"
    );

    // The grain difference itself: those two installations are inside ONE of
    // the four members, so the object-level total (6) exceeds `count` (4).
    // Stated as an assertion so the disclosure cannot rot into a lie.
    assert_eq!(
        members + installations,
        6,
        "4 members + 2 nested installations = 6 CityObjects; the two grains are \
         deliberately different numbers"
    );
}

// ---------------------------------------------------------------------------
// Real attribute queries, against a real national-export fragment.
// ---------------------------------------------------------------------------

/// `savenow_ingolstadt_lod2.gml` is the first three buildings of the SAVeNoW
/// Ingolstadt LoD2 export, verbatim. Two of the three declare
/// `bldg:roofType` 3100; all three declare a numeric `bldg:measuredHeight`;
/// exactly one declares `bldg:storeysAboveGround`.
#[test]
fn attribute_scenarios_answer_a_real_export_fragment() {
    let input = data_fixture("savenow_ingolstadt_lod2.gml");

    assert_eq!(
        run_child("citygml", "count", &input, &[]),
        3,
        "three cityObjectMembers, each one bldg:Building"
    );

    assert_eq!(
        run_child(
            "citygml",
            "attr-filter",
            &input,
            &["--attr-column", "roofType", "--attr-eq", "3100"],
        ),
        2,
        "DEBY_LOD2_107777354 and DEBY_LOD2_4392636 have roofType 3100; \
         DEBY_LOD2_51985910 has 1000"
    );

    assert_eq!(
        run_child(
            "citygml",
            "attr-stats",
            &input,
            &["--attr-column", "measuredHeight"],
        ),
        3,
        "bldg:measuredHeight is a NUMERIC CityGML element, so all three count"
    );
    assert_eq!(
        run_child(
            "citygml",
            "attr-stats",
            &input,
            &["--attr-column", "roofType"]
        ),
        0,
        "bldg:roofType is a STRING element whose values merely look numeric — \
         attr-stats aggregates numeric values only, matching every other \
         runner's semantics"
    );

    assert_eq!(
        run_child(
            "citygml",
            "project",
            &input,
            &["--attr-column", "measuredHeight"],
        ),
        3,
        "all three buildings carry a measuredHeight"
    );
    assert_eq!(
        run_child(
            "citygml",
            "project",
            &input,
            &["--attr-column", "storeysAboveGround"],
        ),
        1,
        "only DEBY_LOD2_107777354 declares storeysAboveGround"
    );

    assert_eq!(
        run_child(
            "citygml",
            "id-lookup",
            &input,
            &["--target-id", "DEBY_LOD2_4392636"],
        ),
        1,
        "a real gml:id from the fragment"
    );
    assert_eq!(
        run_child(
            "citygml",
            "id-lookup",
            &input,
            &["--target-id", "DEBY_LOD2_00000000"],
        ),
        0,
        "a plausible-looking but absent id must return 0"
    );
}

/// Per-member bboxes come from the feature's own (feature-local, quantised)
/// vertices, dequantised through the header transform the reader derives from
/// the document's `gml:Envelope` lower corner. The three buildings are far
/// apart in EPSG:25832 easting, so a window can select a genuine subset.
#[test]
fn bbox_query_selects_a_real_subset_of_the_export_fragment() {
    let input = data_fixture("savenow_ingolstadt_lod2.gml");

    // Covers all three buildings (their union spans easting 676298..677525,
    // northing 5403198..5403605, height 367..379).
    assert_eq!(
        run_child(
            "citygml",
            "bbox-query",
            &input,
            &["--bbox", "676290,5403190,360,677530,5403610,380"],
        ),
        3,
        "a window over the union of all three buildings matches all three"
    );

    // Excludes DEBY_LOD2_4392636 (easting 677507..677524) only.
    assert_eq!(
        run_child(
            "citygml",
            "bbox-query",
            &input,
            &["--bbox", "676290,5403190,360,676880,5403290,380"],
        ),
        2,
        "the western window keeps DEBY_LOD2_51985910 and DEBY_LOD2_107777354 \
         and drops DEBY_LOD2_4392636 — a genuine sub-selection, so this \
         scenario is proven to filter"
    );

    assert_eq!(
        run_child("citygml", "bbox-query", &input, &["--bbox", "0,0,0,1,1,1"],),
        0,
        "a window outside the dataset must match nothing"
    );
}

// ---------------------------------------------------------------------------
// Refusals: an unreadable document must fail loudly, never return a number.
// ---------------------------------------------------------------------------

/// The reader supports CityGML **2.0** only. A 1.0 document must FAIL with a
/// clear version message: a benchmark row that silently reported 0 objects
/// for a file it could not read would be worse than no row at all.
#[test]
fn a_citygml_1_0_document_fails_with_a_clear_version_error() {
    let input = fixture("berlin_citygml1.gml");
    let stderr = run_child_expect_failure("citygml", "count", &input, &[]);
    assert!(
        stderr.contains("unsupported CityGML version 1.0"),
        "expected a clear CityGML version refusal, got stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("2.0"),
        "the refusal must name the version that IS supported, got stderr:\n{stderr}"
    );
}

/// Pointing `--format citygml` at a CityJSON document must fail too. The
/// underlying `Source` would happily read it as CityJSON — and the CSV would
/// then publish another format's cost under this format's name.
#[test]
fn a_non_citygml_input_is_refused_rather_than_measured_as_citygml() {
    let input = fixture("lod3_railway.city.json");
    let stderr = run_child_expect_failure("citygml", "count", &input, &[]);
    assert!(
        stderr.contains("not a CityGML"),
        "expected a clear not-CityGML refusal, got stderr:\n{stderr}"
    );
}

// ---------------------------------------------------------------------------
// The disclosure the paper depends on.
// ---------------------------------------------------------------------------

/// The single most important thing this runner owes the paper is a statement
/// of what its row does and does NOT claim. There is no runtime signal a
/// reader of the CSV could check instead, so this asserts against the
/// module's own doc block — the disclosure cannot be deleted without a test
/// going red.
#[test]
fn the_module_doc_discloses_the_full_parse_and_disclaims_a_format_ceiling() {
    const SOURCE: &str = include_str!("../src/formats/citygml.rs");
    // The leading `//!` block, i.e. everything before the first `use`.
    let module_doc = SOURCE
        .split("\nuse ")
        .next()
        .expect("the module always has a doc block above its first `use`");

    // Short, wrap-safe needles: the doc block is hard-wrapped, so a long
    // sentence would match nothing however faithfully it were written.
    for needle in [
        "no index",
        "full parse",
        "theoretical ceiling",
        "different parser",
        "different numbers",
        "cityObjectMember",
    ] {
        assert!(
            module_doc.contains(needle),
            "the module doc must disclose the full-parse cost, the counting \
             grain, and that the row is not a claim about the format's ceiling; \
             missing: '{needle}'"
        );
    }
}
