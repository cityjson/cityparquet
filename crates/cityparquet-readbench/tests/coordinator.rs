//! RED (readbench Task 11, Commit B): the COORDINATOR — `cityparquet-readbench
//! run ...` — exercised only through the BUILT binary (never calling into
//! `coordinator`'s internals directly), against a real prepared package
//! built from `delft.city.jsonl` (never inline artificial CityJSON).
//!
//! Prepares its own tiny "prepared dir" by converting the real fixture with
//! `cityparquet::package::convert` directly (rather than shelling to
//! `scripts/readbench_prepare.sh`/`fcb`, keeping this test network- and
//! external-tool-independent) — exactly the `<x>.parquet` naming convention
//! the real prep script uses (`readbench_prepare.sh`'s own doc comment), so
//! the coordinator's artefact-location logic is exercised unmodified.
//!
//! Uses `delft.city.jsonl` rather than `lod3_railway.city.json` (2026-07-21,
//! mandatory-by-type-layout): the single-file table layout is gone, so `convert()`
//! now always writes one table per 1st-level CityObject family, and delft
//! (Building + BuildingPart, both mapping to the "Building" family) is the
//! only committed fixture that still by-type-converts to exactly ONE main
//! table — which every test below except
//! `attr_filter_selectivity_uses_the_shared_cityparquet_object_total_as_denominator`
//! (already delft-based) relies on for a single, whole-dataset-queryable
//! `cityparquet` artefact. Nothing asserted below is railway-specific: the
//! CSV contract, `--repeat` validation, and the missing-artefact skip are
//! all generic over which fixture is converted.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use cityparquet::cjseq::{CityJSON, CityJSONFeature, cjseq_to_cj};
use cityparquet::package::{ConvertOptions, convert};

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

/// A committed fixture of `crates/cityparquet`'s own test data — real
/// published CityGML, never a hand-written document.
fn citygml_fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../cityparquet/tests/data")
        .join(name);
    assert!(p.exists(), "missing fixture {name}");
    p
}

/// Copies `input` into `prepared` under the name `Format::CityJsonSeq`
/// resolves to — the same thing `scripts/readbench_prepare.sh` does for a
/// `.city.jsonl` input (its `cp` branch), reproduced here so these tests
/// exercise the real resolution path without shelling out to the script.
fn prepare_seq_artefact(prepared: &Path, input: &Path, base: &str) {
    std::fs::copy(input, prepared.join(format!("{base}.city.jsonl"))).unwrap();
}

/// A whole-document CityJSON collected from the real `delft.city.jsonl`
/// fixture by `cjseq`'s own `cjseq_to_cj` — the very call `cjseq collect`
/// makes, and therefore the same artefact `readbench_prepare.sh` would
/// produce. Written as `<dir>/delft.city.json`, so a coordinator run over it
/// is a run over a `.city.json` INPUT (the shape six of the corpus's
/// datasets have) rather than over the `.city.jsonl` every other test here
/// uses. Never inline hand-written CityJSON.
fn collect_delft_document(dir: &Path) -> PathBuf {
    let text = std::fs::read_to_string(fixture("delft.city.jsonl")).unwrap();
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let header = CityJSON::from_str(lines.next().unwrap()).unwrap();
    let features: Vec<CityJSONFeature> = lines
        .map(|line| CityJSONFeature::from_str(line).unwrap())
        .collect();
    let doc = cjseq_to_cj(header, features);
    let path = dir.join("delft.city.json");
    std::fs::write(&path, serde_json::to_string(&doc).unwrap()).unwrap();
    path
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

const CSV_COLUMNS: [&str; 13] = [
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
    "bytes_read",
    "http_requests",
];

const EXPECTED_HEADER: &str = "dataset,format,scenario,selectivity,result_count,time_s,\
time_mad_s,peak_heap_bytes,peak_rss_bytes,repeat,notes,bytes_read,http_requests";

#[test]
fn run_produces_the_exact_csv_contract_with_medians_and_selectivity_derived_from_real_data() {
    let prepared = tempfile::tempdir().unwrap();
    let input = fixture("delft.city.jsonl");

    // The "prepared" cityparquet package, at the exact path/name convention
    // `scripts/readbench_prepare.sh` produces: `<prepared_dir>/<base>.parquet`
    // where `<base>` strips `.city.jsonl`.
    let package_dir = prepared.path().join("delft.parquet");
    let report = convert(&ConvertOptions::new(input.clone(), package_dir)).unwrap();
    assert_eq!(report.object_count, 2231);
    // …and the `cityjsonseq` artefact beside it, `<base>.city.jsonl`: EVERY
    // measured format reads a prepared artefact, this one included.
    prepare_seq_artefact(prepared.path(), &input, "delft");

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
        assert_eq!(row.field("dataset"), "delft.city.jsonl");
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
        assert_eq!(
            row.field("bytes_read"),
            "",
            "a local-transport row's bytes_read must be empty (no HTTP concept locally)"
        );
        assert_eq!(
            row.field("http_requests"),
            "",
            "a local-transport row's http_requests must be empty (no HTTP concept locally)"
        );

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
            // `[0, 1]`, not `(0, 1]`: delft's real, unpadded geographic bbox
            // means the SMALL lower-left-anchored windows (1pct especially)
            // can legitimately touch zero buildings — a valid selectivity of
            // 0, not a bug in selectivity computation. What this test
            // actually pins is that every row parses as a well-formed
            // fraction in range, never a NaN/negative/>1 value.
            assert!(
                (0.0..=1.0).contains(&selectivity),
                "bbox-query selectivity must be in [0, 1], got {selectivity} \
                 (row: {row_fields:?})",
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
    let input = fixture("delft.city.jsonl");
    let package_dir = prepared.path().join("delft.parquet");
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
    let input = fixture("delft.city.jsonl");
    let package_dir = prepared.path().join("delft.parquet");
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

/// A skipped format is a different kind of event depending on WHO chose it.
/// When the operator names `--formats`, a missing artefact is answered by the
/// per-format skip note above and nothing more. When `--formats` is omitted,
/// the coordinator picked the format-comparison set itself, and a CSV holding
/// only some of it is not the comparison the operator asked for — silently
/// dropping four of five formats would be published as "the format
/// comparison". So a default-set run that resolves fewer formats than the set
/// holds must say so loudly, naming exactly what is missing.
///
/// Prepares only the `cityparquet` package (required regardless, for
/// QueryParams derivation) and the `cityjsonseq` artefact, so those two are
/// all of the default set that can resolve.
#[test]
fn a_default_set_run_says_loudly_when_it_could_not_measure_the_whole_set() {
    let prepared = tempfile::tempdir().unwrap();
    let input = fixture("delft.city.jsonl");
    let package_dir = prepared.path().join("delft.parquet");
    convert(&ConvertOptions::new(input.clone(), package_dir)).unwrap();
    prepare_seq_artefact(prepared.path(), &input, "delft");
    let out_csv = prepared.path().join("out.csv");

    // No `--formats`: the coordinator selects the format-comparison set.
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
    ]);

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not a complete format comparison"),
        "an incomplete default-set run must say the CSV is not a complete format \
         comparison; stderr:\n{stderr}"
    );
    for missing in ["citygml", "cityjson", "flatcitybuf", "cityparquet-hilbert"] {
        assert!(
            stderr.contains(missing),
            "the warning must name the missing format '{missing}'; stderr:\n{stderr}"
        );
    }

    // Only `cityjsonseq` could resolve, and it still ran.
    let csv_text = std::fs::read_to_string(&out_csv).unwrap();
    let rows: Vec<Row> = csv_text.lines().skip(1).map(Row::parse).collect();
    assert_eq!(
        rows.len(),
        1,
        "expected only cityjsonseq's count row: {csv_text}"
    );
    assert_eq!(rows[0].field("format"), "cityjsonseq");
}

/// The mirror of the case above: when the operator NAMED the formats, the
/// per-format skip note is the whole story — no set-level "incomplete
/// comparison" alarm, because there is no set the coordinator chose.
#[test]
fn an_explicitly_requested_skip_does_not_raise_the_incomplete_set_alarm() {
    let prepared = tempfile::tempdir().unwrap();
    let input = fixture("delft.city.jsonl");
    let package_dir = prepared.path().join("delft.parquet");
    convert(&ConvertOptions::new(input.clone(), package_dir)).unwrap();
    let out_csv = prepared.path().join("out.csv");

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
        stderr.contains("flatcitybuf"),
        "the per-format skip note must still appear; stderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("not a complete format comparison"),
        "an explicitly-requested skip is not an incomplete default set; stderr:\n{stderr}"
    );
}

/// Object-level scenarios (`AttrFilter`/`AttrStats`/`Project`/`IdLookup`) are
/// CityObject-level for EVERY format (see `coordinator`'s own module doc),
/// so their selectivity denominator must be the dataset-global CityObject
/// total — the `cityparquet` package's own `Count` — shared across every
/// format, never a per-format total. On `delft.city.jsonl`,
/// `cityjsonseq`'s own `Count` is 1115 (feature-level: one line per
/// top-level `Building`), while `AttrFilter(object_type == "BuildingPart")`
/// is 1116 (CityObject-level: `BuildingPart`s are children flattened out of
/// their parent `Building` features) — dividing the object-level numerator
/// by the feature-level `cityjsonseq` total therefore yields `1116/1115 ≈
/// 1.0009`, a selectivity > 1.0, which is nonsensical and was the pre-fix
/// bug this test pins down as GREEN (it would fail RED against the
/// unfixed coordinator, which used `total_count_for(format, path)` — each
/// format's OWN count — as the denominator for every non-BBoxQuery
/// scenario).
#[test]
fn attr_filter_selectivity_uses_the_shared_cityparquet_object_total_as_denominator() {
    let prepared = tempfile::tempdir().unwrap();
    let input = fixture("delft.city.jsonl");

    let package_dir = prepared.path().join("delft.parquet");
    let report = convert(&ConvertOptions::new(input.clone(), package_dir)).unwrap();
    prepare_seq_artefact(prepared.path(), &input, "delft");
    // The dataset-global CityObject total (parents AND children) —
    // `crates/cityparquet/tests/query_real_data.rs` pins the same known
    // split: `BuildingPart: 1116`, `Building: 1115`, `1116 + 1115 = 2231`.
    assert_eq!(
        report.object_count, 2231,
        "delft.city.jsonl's known CityObject total"
    );

    let out_csv = prepared.path().join("out.csv");

    run_coordinator(&[
        "--input",
        input.to_str().unwrap(),
        "--prepared-dir",
        prepared.path().to_str().unwrap(),
        "--out",
        out_csv.to_str().unwrap(),
        "--repeat",
        "1",
        "--scenarios",
        "attr-filter",
        "--formats",
        "cityparquet,cityjsonseq",
    ]);

    let csv_text = std::fs::read_to_string(&out_csv).expect("coordinator must write the CSV");
    let rows: Vec<Row> = csv_text.lines().skip(1).map(Row::parse).collect();
    assert_eq!(
        rows.len(),
        2,
        "expected exactly one attr-filter row per format, got {}: {csv_text}",
        rows.len()
    );

    let cityparquet_row = rows
        .iter()
        .find(|r| r.field("format") == "cityparquet")
        .expect("missing cityparquet row");
    let cityjsonseq_row = rows
        .iter()
        .find(|r| r.field("format") == "cityjsonseq")
        .expect("missing cityjsonseq row");

    // Both formats agree on the CityObject-level result_count — this
    // scenario's grain is identical across formats (the coordinator's own
    // self-consistency check covers this too, but pin it here as the
    // premise the denominator assertion below depends on).
    let cp_count: u64 = cityparquet_row.field("result_count").parse().unwrap();
    let cjseq_count: u64 = cityjsonseq_row.field("result_count").parse().unwrap();
    assert_eq!(
        cp_count, cjseq_count,
        "AttrFilter(object_type) result_count must match across formats (both CityObject-level)"
    );
    assert_eq!(
        cp_count, 1116,
        "delft.city.jsonl's known BuildingPart count"
    );

    let cp_selectivity: f64 = cityparquet_row.field("selectivity").parse().unwrap();
    let cjseq_selectivity: f64 = cityjsonseq_row.field("selectivity").parse().unwrap();

    for (label, selectivity) in [
        ("cityparquet", cp_selectivity),
        ("cityjsonseq", cjseq_selectivity),
    ] {
        assert!(
            selectivity > 0.0 && selectivity <= 1.0,
            "{label}'s AttrFilter selectivity must be in (0, 1], got {selectivity} \
             (pre-fix cityjsonseq value would be 1116/1115 ≈ 1.0009)"
        );
    }

    // Same numerator (1116) over the SAME shared denominator (2231, the
    // cityparquet package's own object total) must produce identical
    // selectivity for both formats — proving the denominator is no longer
    // per-format.
    assert!(
        (cp_selectivity - cjseq_selectivity).abs() < 1e-6,
        "expected identical selectivity for both formats (same numerator, same shared \
         denominator), got cityparquet={cp_selectivity} cityjsonseq={cjseq_selectivity}"
    );
    let expected = cp_count as f64 / report.object_count as f64;
    assert!(
        (cp_selectivity - expected).abs() < 1e-6,
        "expected selectivity == result_count / cityparquet object total \
         ({cp_count}/{}) = {expected}, got {cp_selectivity}",
        report.object_count
    );
}

async fn spawn_server(dir: PathBuf) -> std::net::SocketAddr {
    let app = axum::Router::new().fallback_service(tower_http::services::ServeDir::new(dir));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    addr
}

/// `--transport http`: the coordinator's own QueryParams derivation (bbox,
/// object_type, numeric attribute, sample id, and the shared CityObject
/// total) always reads the LOCAL `prepared_dir` directly (unchanged by
/// `--transport` — see this module's own doc comment on where `QueryParams`
/// come from); only the actual per-scenario measurement calls the
/// coordinator spawns go over HTTP. Serves the same `parent/delft.parquet/`
/// layout `tests/cityparquet_http_runner.rs` uses (`parent/` is the served
/// root, `delft.parquet/` the package inside it) and asserts the CSV's new
/// `bytes_read`/`http_requests` columns are populated with real positive
/// numbers on the http-transport row.
///
/// `flavor = "multi_thread"`: `run_coordinator` makes a blocking
/// `std::process::Command::output()` call, which would starve a plain
/// current-thread `#[tokio::test]`'s single OS thread and prevent the
/// spawned axum server task from ever being polled — the same gotcha fixed
/// in `tests/cityparquet_http_runner.rs`.
#[tokio::test(flavor = "multi_thread")]
async fn run_with_http_transport_reports_bytes_and_requests_on_the_cityparquet_row() {
    let parent = tempfile::tempdir().unwrap();
    let input = fixture("delft.city.jsonl");
    let package_dir = parent.path().join("delft.parquet");
    convert(&ConvertOptions::new(input.clone(), package_dir)).unwrap();

    let addr = spawn_server(parent.path().to_path_buf()).await;
    let base_url = format!("http://{addr}");

    let out_csv = parent.path().join("out.csv");

    run_coordinator(&[
        "--input",
        input.to_str().unwrap(),
        "--prepared-dir",
        parent.path().to_str().unwrap(),
        "--out",
        out_csv.to_str().unwrap(),
        "--repeat",
        "1",
        "--scenarios",
        "count",
        "--formats",
        "cityparquet",
        "--transport",
        "http",
        "--base-url",
        &base_url,
    ]);

    let csv_text = std::fs::read_to_string(&out_csv).expect("coordinator must write the CSV");
    let mut lines = csv_text.lines();
    assert_eq!(lines.next().unwrap(), EXPECTED_HEADER);
    let rows: Vec<Row> = lines.map(Row::parse).collect();
    assert_eq!(rows.len(), 1, "expected exactly 1 count row: {csv_text}");

    assert_eq!(rows[0].field("format"), "cityparquet");
    let bytes: u64 = rows[0].field("bytes_read").parse().unwrap_or_else(|e| {
        panic!(
            "bytes_read '{}' must parse as u64: {e}",
            rows[0].field("bytes_read")
        )
    });
    let requests: u64 = rows[0].field("http_requests").parse().unwrap_or_else(|e| {
        panic!(
            "http_requests '{}' must parse as u64: {e}",
            rows[0].field("http_requests")
        )
    });
    assert!(bytes > 0, "expected a positive bytes_read, got {bytes}");
    assert!(
        requests >= 1,
        "expected at least 1 http_requests, got {requests}"
    );
}

/// **C1's regression guard.** A CityGML input must never be measured as
/// CityJSONSeq.
///
/// `Format::CityJsonSeq` used to resolve to the `--input` itself, which was
/// correct only while every input WAS a `.city.jsonl`. On the catalogue
/// corpus — `.gml` and `.city.json` — that made the `cityjsonseq` row a
/// measurement of the input's own format under another name: on
/// `plateau_chuo_fld.gml`, `count` was 0.175 s of CityGML parsing published
/// as CityJSONSeq. Nothing caught it, because every coordinator test here
/// used the one input kind for which the old resolution was right.
///
/// So: with no `<base>.city.jsonl` in `--prepared-dir`, `cityjsonseq` must be
/// SKIPPED — the same treatment any other format's missing artefact gets —
/// and the `.gml` must not appear in the CSV under that tag.
#[test]
fn a_citygml_input_is_never_measured_as_cityjsonseq() {
    let prepared = tempfile::tempdir().unwrap();
    let input = citygml_fixture("savenow_ingolstadt_lod2.gml");
    let package_dir = prepared.path().join("savenow_ingolstadt_lod2.parquet");
    convert(&ConvertOptions::new(input.clone(), package_dir)).unwrap();
    let out_csv = prepared.path().join("out.csv");

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
        "cityparquet,cityjsonseq",
    ]);

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("skipping format 'cityjsonseq'")
            && stderr.contains("savenow_ingolstadt_lod2.city.jsonl"),
        "cityjsonseq must be skipped for its own missing prepared artefact; stderr:\n{stderr}"
    );

    let csv_text = std::fs::read_to_string(&out_csv).unwrap();
    let rows: Vec<Row> = csv_text.lines().skip(1).map(Row::parse).collect();
    assert_eq!(
        rows.len(),
        1,
        "only cityparquet may be measured here: {csv_text}"
    );
    assert_eq!(rows[0].field("format"), "cityparquet");
}

/// The same guard for a `.city.json` input — the other half of the catalogue
/// corpus, and the shape where the old resolution was quietest: `cityjson`
/// and `cityjsonseq` read the SAME file, so their counts and timings looked
/// like two formats measured, and the self-consistency check saw nothing
/// wrong.
#[test]
fn a_cityjson_input_is_never_measured_as_cityjsonseq() {
    let prepared = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let input = collect_delft_document(source.path());
    let package_dir = prepared.path().join("delft.parquet");
    convert(&ConvertOptions::new(input.clone(), package_dir)).unwrap();
    let out_csv = prepared.path().join("out.csv");

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
        "cityparquet,cityjsonseq",
    ]);

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("skipping format 'cityjsonseq'") && stderr.contains("delft.city.jsonl"),
        "cityjsonseq must be skipped for its own missing prepared artefact; stderr:\n{stderr}"
    );

    let csv_text = std::fs::read_to_string(&out_csv).unwrap();
    let rows: Vec<Row> = csv_text.lines().skip(1).map(Row::parse).collect();
    assert_eq!(
        rows.len(),
        1,
        "only cityparquet may be measured: {csv_text}"
    );
    assert_eq!(rows[0].field("format"), "cityparquet");
}

/// …and with the artefact PRESENT it is measured, from `--prepared-dir` —
/// so the guard above is a resolution rule, not a way of never measuring
/// CityJSONSeq at all. The prepared seq here is deliberately the only
/// CityJSONSeq in sight: the `--input` is a `.city.json`.
#[test]
fn a_prepared_seq_artefact_is_what_cityjsonseq_measures() {
    let prepared = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let input = collect_delft_document(source.path());
    let package_dir = prepared.path().join("delft.parquet");
    convert(&ConvertOptions::new(input.clone(), package_dir)).unwrap();
    prepare_seq_artefact(prepared.path(), &fixture("delft.city.jsonl"), "delft");
    let out_csv = prepared.path().join("out.csv");

    run_coordinator(&[
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
        "cityjsonseq",
    ]);

    let csv_text = std::fs::read_to_string(&out_csv).unwrap();
    let rows: Vec<Row> = csv_text.lines().skip(1).map(Row::parse).collect();
    assert_eq!(
        rows.len(),
        1,
        "expected one cityjsonseq count row: {csv_text}"
    );
    assert_eq!(rows[0].field("format"), "cityjsonseq");
    // The Seq stream's own natural unit: 1115 feature lines (one per
    // top-level Building). The whole-document `--input` beside it holds 2231
    // CityObjects, so this count is proof of WHICH file was read.
    assert_eq!(
        rows[0].field("result_count"),
        "1115",
        "cityjsonseq must have read the prepared .city.jsonl: {csv_text}"
    );
}

/// The runner's own half of the same guard (the coordinator's is above):
/// pointed straight at a CityGML document, `--format cityjsonseq` must
/// refuse it rather than parse it — `cityparquet::source::Source` sniffs, and
/// would otherwise read the XML quite happily and report its cost as
/// CityJSONSeq. The mirror image of the guards `--format citygml` and
/// `--format cityjson` have carried all along.
#[test]
fn the_cityjsonseq_child_refuses_a_citygml_document() {
    let output = Command::new(env!("CARGO_BIN_EXE_cityparquet-readbench"))
        .args([
            "--child",
            "--format",
            "cityjsonseq",
            "--scenario",
            "count",
            "--input",
        ])
        .arg(citygml_fixture("savenow_ingolstadt_lod2.gml"))
        .output()
        .expect("failed to run the built cityparquet-readbench binary");

    assert!(
        !output.status.success(),
        "a CityGML document must not be measurable as CityJSONSeq; stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("is a CityGML document, not CityJSONSeq"),
        "expected the runner's own refusal; stderr:\n{stderr}"
    );
}
