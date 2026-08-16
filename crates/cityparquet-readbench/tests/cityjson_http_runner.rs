//! Plain-CityJSON HTTP runner: serves real CityJSON documents from an
//! in-test axum+tower-http Range server, then drives the BUILT
//! `cityparquet-readbench --child` binary with `--transport http` against
//! them, asserting `result_count` parity with the equivalent `--transport
//! local` call and that the reported `bytes`/`requests` are genuinely
//! MEASURED.
//!
//! A plain CityJSON document has no index and cannot be parsed in pieces, so
//! "the whole object, in one request" is the honest — and only — thing this
//! transport can do for ANY scenario. Both a whole-document scenario
//! (`count`) and the most selective one (`id-lookup`) are checked, so the CSV
//! can never suggest this format range-reads its way to a cheap point lookup.
//!
//! **Why two differently-sized documents are served.** `bytes_read` and
//! `http_requests` are published CSV columns, so the test must be able to
//! tell a real `CountingObjectStore` tally from a fabricated one. Against a
//! single document, `requests == 1` and `bytes == <that file's length>` are
//! exactly what a hardcoded `IoStats` would report, and replacing the tally
//! with literals leaves such a test green. Serving TWO documents of different
//! sizes and asserting each run reports its own file's length removes that
//! escape: one hardcoded pair cannot be right for both.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Command;

use tower_http::services::ServeDir;

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

/// Writes a SECOND, deliberately differently-sized CityJSON document into
/// `dir`: the real `lod3_railway.city.json` with its `CityObjects` map
/// reduced to the 15 `SolitaryVegetationObject`s. The document-level
/// `vertices` array is kept intact, so every retained geometry still
/// resolves — a CityJSON document may carry vertices no object references.
///
/// A JSON mutation of the real fixture (the same house pattern as
/// `tests/attr_consistency.rs`'s `railway_fixture_with_crs`), never
/// hand-written CityJSON. Its whole purpose is to have a different byte
/// length and a different object count from the original.
fn write_vegetation_subset(dir: &Path) -> PathBuf {
    let mut doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(fixture("lod3_railway.city.json")).unwrap())
            .unwrap();
    let objects = doc["CityObjects"].as_object().unwrap().clone();
    let kept: serde_json::Map<String, serde_json::Value> = objects
        .into_iter()
        .filter(|(_, co)| co["type"] == "SolitaryVegetationObject")
        .collect();
    assert_eq!(
        kept.len(),
        15,
        "the fixture's 15 SolitaryVegetationObjects are the subset this test relies on"
    );
    doc["CityObjects"] = serde_json::Value::Object(kept);

    let path = dir.join("railway_vegetation.city.json");
    std::fs::write(&path, serde_json::to_string(&doc).unwrap()).unwrap();
    path
}

async fn spawn_server(dir: PathBuf) -> SocketAddr {
    let app = axum::Router::new().fallback_service(ServeDir::new(dir));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    addr
}

fn run_child(args: &[&str]) -> (bool, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_cityparquet-readbench"))
        .args(args)
        .output()
        .expect("failed to run the built binary");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).trim().to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
    )
}

/// One `--transport http` `--child` run: returns `(result_count, bytes,
/// requests)` parsed from the 6-field stdout line.
fn http_run(base_url: &str, key: &str, scenario: &str, extra: &[&str]) -> (u64, u64, u64) {
    let mut args = vec![
        "--child",
        "--format",
        "cityjson",
        "--scenario",
        scenario,
        "--transport",
        "http",
        "--base-url",
        base_url,
        "--input",
        key,
    ];
    args.extend_from_slice(extra);
    let (ok, out, err) = run_child(&args);
    assert!(ok, "http child failed ({key}, {scenario}): {err}");
    let fields: Vec<&str> = out.split_whitespace().collect();
    assert_eq!(fields.len(), 6, "http line: {out}");
    (
        fields[3].parse().unwrap(),
        fields[4].parse().unwrap(),
        fields[5].parse().unwrap(),
    )
}

// `flavor = "multi_thread"`: `run_child` makes a blocking
// `std::process::Command::output()` call, which would starve a plain
// current-thread `#[tokio::test]`'s single OS thread and prevent the spawned
// axum server task from ever being polled — the same gotcha handled in
// `tests/cityjsonseq_http_runner.rs`.
#[tokio::test(flavor = "multi_thread")]
async fn http_matches_local_and_is_exactly_one_whole_object_get_per_scenario() {
    let fixture_path = fixture("lod3_railway.city.json");
    let fixture_dir = fixture_path.parent().unwrap().to_path_buf();
    let fixture_name = fixture_path.file_name().unwrap().to_str().unwrap();
    let expected_bytes = std::fs::metadata(&fixture_path).unwrap().len();

    let addr = spawn_server(fixture_dir).await;
    let base_url = format!("http://{addr}");

    // (scenario, extra args) — a whole-document scenario and the most
    // selective one; both must cost the same single whole-object GET.
    let cases: [(&str, Vec<&str>); 2] = [
        ("count", vec![]),
        (
            "id-lookup",
            vec!["--target-id", "UUID_bd865e62-18de-40ff-85da-883709a86f0f"],
        ),
    ];

    for (scenario, extra) in cases {
        let mut local_args = vec![
            "--child",
            "--format",
            "cityjson",
            "--scenario",
            scenario,
            "--input",
            fixture_path.to_str().unwrap(),
        ];
        local_args.extend_from_slice(&extra);
        let (local_ok, local_out, local_err) = run_child(&local_args);
        assert!(local_ok, "local child failed ({scenario}): {local_err}");
        let local_fields: Vec<&str> = local_out.split_whitespace().collect();
        assert_eq!(local_fields.len(), 4, "local line: {local_out}");
        let local_count: u64 = local_fields[3].parse().unwrap();

        let (http_count, bytes, requests) = http_run(&base_url, fixture_name, scenario, &extra);
        assert_eq!(
            http_count, local_count,
            "result_count must match between local and http transports ({scenario})"
        );
        assert_eq!(
            requests, 1,
            "a whole-object GET is exactly one request by construction ({scenario})"
        );
        assert_eq!(
            bytes, expected_bytes,
            "an unindexed CityJSON document transfers its whole byte length, \
             even for the most selective scenario ({scenario})"
        );
    }
}

/// `bytes_read`/`http_requests` are published CSV columns, so they must be
/// MEASURED by the `CountingObjectStore`, not asserted into existence.
///
/// Two documents of genuinely different sizes are served from the same
/// directory and each is asked the same question; each run must report its
/// OWN file's byte length. A hardcoded `IoStats` — the exact mutation that
/// survives a single-document test — cannot be right for both, so this test
/// fails the moment the tally stops being real.
#[tokio::test(flavor = "multi_thread")]
async fn io_stats_track_each_documents_own_size_so_a_hardcoded_tally_cannot_pass() {
    let tmp = tempfile::tempdir().unwrap();
    let whole = tmp.path().join("railway_whole.city.json");
    std::fs::copy(fixture("lod3_railway.city.json"), &whole).unwrap();
    let subset = write_vegetation_subset(tmp.path());

    let whole_bytes = std::fs::metadata(&whole).unwrap().len();
    let subset_bytes = std::fs::metadata(&subset).unwrap().len();
    assert_ne!(
        whole_bytes, subset_bytes,
        "this test's whole premise is that the two served documents differ in \
         size; if they ever stop differing it proves nothing"
    );

    let addr = spawn_server(tmp.path().to_path_buf()).await;
    let base_url = format!("http://{addr}");

    let (whole_count, whole_tally, whole_requests) =
        http_run(&base_url, "railway_whole.city.json", "count", &[]);
    let (subset_count, subset_tally, subset_requests) =
        http_run(&base_url, "railway_vegetation.city.json", "count", &[]);

    assert_eq!(
        whole_tally, whole_bytes,
        "the whole document's tally must be its own byte length"
    );
    assert_eq!(
        subset_tally, subset_bytes,
        "the subset document's tally must be ITS own byte length — a literal \
         that satisfied the whole document cannot also satisfy this"
    );
    assert_ne!(
        whole_tally, subset_tally,
        "two differently-sized documents must not report the same tally"
    );
    assert_eq!(whole_requests, 1);
    assert_eq!(subset_requests, 1);

    // The same argument at the `result_count` level: both runs answered the
    // document actually fetched, not a remembered one.
    assert_eq!(whole_count, 121, "the whole fixture's CityObjects map size");
    assert_eq!(
        subset_count, 15,
        "the vegetation subset's own CityObjects map size"
    );
}
