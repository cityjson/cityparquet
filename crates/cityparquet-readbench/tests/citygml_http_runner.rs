//! CityGML HTTP runner: serves real CityGML 2.0 documents from an in-test
//! axum+tower-http server, then drives the BUILT `cityparquet-readbench
//! --child` binary with `--transport http` against them, asserting
//! `result_count` parity with the equivalent `--transport local` call and that
//! the reported `bytes`/`requests` are genuinely MEASURED.
//!
//! CityGML has no index of any kind, so "the whole object, in one request" is
//! the honest — and only — thing this transport can do for ANY scenario: there
//! is no way to know which byte range holds the answer, and the reader
//! re-streams the document from the start regardless. Both a whole-document
//! scenario (`count`) and the most selective one (`id-lookup`) are checked, so
//! the CSV can never suggest this format range-reads its way to a cheap point
//! lookup.
//!
//! **Why two differently-sized documents are served.** `bytes_read` and
//! `http_requests` are published CSV columns, so the test must be able to tell
//! a real `CountingObjectStore` tally from a fabricated one. Against a single
//! document, `requests == 1` and `bytes == <that file's length>` are exactly
//! what a hardcoded `IoStats` would report, and replacing the tally with
//! literals leaves such a test green. Serving TWO documents of different sizes
//! and asserting each run reports its own file's length removes that escape:
//! one hardcoded pair cannot be right for both. (Verified by mutation: with
//! `IoStats` replaced by literals, `io_stats_track_each_documents_own_size…`
//! below fails.)

use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::Command;

use tower_http::services::ServeDir;

/// A fixture fetched by `just fixtures` into the workspace's `tests/fixtures/`.
fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

/// A committed real CityGML fragment under `crates/cityparquet/tests/data/` —
/// see `tests/citygml_runner.rs`'s own helper for why this crate reaches
/// across to the reader crate's fixtures.
fn data_fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../cityparquet/tests/data")
        .join(name);
    assert!(p.exists(), "missing committed fixture {name}");
    p
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
        "citygml",
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

/// Copies both real fixtures into one served directory and returns it.
fn served_dir() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::copy(
        data_fixture("savenow_ingolstadt_lod2.gml"),
        tmp.path().join("ingolstadt.gml"),
    )
    .unwrap();
    std::fs::copy(fixture("b1_lod2_cs_w_sem.gml"), tmp.path().join("b1.gml")).unwrap();
    tmp
}

// `flavor = "multi_thread"`: `run_child` makes a blocking
// `std::process::Command::output()` call, which would starve a plain
// current-thread `#[tokio::test]`'s single OS thread and prevent the spawned
// axum server task from ever being polled — the same gotcha handled in
// `tests/cityjson_http_runner.rs`.
#[tokio::test(flavor = "multi_thread")]
async fn http_matches_local_and_is_exactly_one_whole_object_get_per_scenario() {
    let tmp = served_dir();
    let local_path = tmp.path().join("ingolstadt.gml");
    let expected_bytes = std::fs::metadata(&local_path).unwrap().len();

    let addr = spawn_server(tmp.path().to_path_buf()).await;
    let base_url = format!("http://{addr}");

    // (scenario, extra args) — a whole-document scenario and the most
    // selective one; both must cost the same single whole-object GET.
    let cases: [(&str, Vec<&str>); 2] = [
        ("count", vec![]),
        ("id-lookup", vec!["--target-id", "DEBY_LOD2_4392636"]),
    ];

    for (scenario, extra) in cases {
        let mut local_args = vec![
            "--child",
            "--format",
            "citygml",
            "--scenario",
            scenario,
            "--input",
            local_path.to_str().unwrap(),
        ];
        local_args.extend_from_slice(&extra);
        let (local_ok, local_out, local_err) = run_child(&local_args);
        assert!(local_ok, "local child failed ({scenario}): {local_err}");
        let local_fields: Vec<&str> = local_out.split_whitespace().collect();
        assert_eq!(local_fields.len(), 4, "local line: {local_out}");
        let local_count: u64 = local_fields[3].parse().unwrap();

        let (http_count, bytes, requests) = http_run(&base_url, "ingolstadt.gml", scenario, &extra);
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
            "an unindexed CityGML document transfers its whole byte length, even \
             for the most selective scenario ({scenario})"
        );
    }
}

/// `bytes_read`/`http_requests` are published CSV columns, so they must be
/// MEASURED by the `CountingObjectStore`, not asserted into existence.
///
/// Two REAL CityGML 2.0 documents of genuinely different sizes are served from
/// the same directory and each is asked the same question; each run must
/// report its OWN file's byte length AND its own object count. A hardcoded
/// `IoStats` — the exact mutation that survives a single-document test —
/// cannot be right for both.
#[tokio::test(flavor = "multi_thread")]
async fn io_stats_track_each_documents_own_size_so_a_hardcoded_tally_cannot_pass() {
    let tmp = served_dir();
    let big_bytes = std::fs::metadata(tmp.path().join("ingolstadt.gml"))
        .unwrap()
        .len();
    let small_bytes = std::fs::metadata(tmp.path().join("b1.gml")).unwrap().len();
    assert_ne!(
        big_bytes, small_bytes,
        "this test's whole premise is that the two served documents differ in \
         size; if they ever stop differing it proves nothing"
    );

    let addr = spawn_server(tmp.path().to_path_buf()).await;
    let base_url = format!("http://{addr}");

    let (big_count, big_tally, big_requests) = http_run(&base_url, "ingolstadt.gml", "count", &[]);
    let (small_count, small_tally, small_requests) = http_run(&base_url, "b1.gml", "count", &[]);

    assert_eq!(
        big_tally, big_bytes,
        "the Ingolstadt fragment's tally must be its own byte length"
    );
    assert_eq!(
        small_tally, small_bytes,
        "the single-building document's tally must be ITS own byte length — a \
         literal that satisfied the other file cannot also satisfy this"
    );
    assert_ne!(
        big_tally, small_tally,
        "two differently-sized documents must not report the same tally"
    );
    assert_eq!(big_requests, 1);
    assert_eq!(small_requests, 1);

    // The same argument at the `result_count` level: both runs answered the
    // document actually fetched, not a remembered one.
    assert_eq!(
        big_count, 3,
        "the Ingolstadt fragment's three cityObjectMembers"
    );
    assert_eq!(small_count, 1, "the single-building document's one member");
}
