//! FlatCityBuf HTTP runner: serves a real `.fcb` file (generated via the
//! `fcb` CLI, like `tests/flatcitybuf_runner.rs`'s own local tests) from an
//! in-test axum+tower-http Range server, then drives the BUILT
//! `cityparquet-readbench --child` binary with `--transport http` against
//! it, asserting `result_count` parity with the equivalent `--transport
//! local` call and that a bytes/requests pair is reported.
//!
//! Skips gracefully (never fails) when the optional external `fcb` CLI
//! isn't on PATH — mirrors `tests/flatcitybuf_runner.rs`'s own convention.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Command;

use tower_http::services::ServeDir;

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../lib/cityparquet-rs/tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

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

// `flavor = "multi_thread"`: `run_child` makes a blocking
// `std::process::Command::output()` call, which would starve a plain
// current-thread `#[tokio::test]`'s single OS thread and prevent the
// spawned axum server task from ever being polled — the same gotcha fixed
// in `tests/cityparquet_http_runner.rs`.
#[tokio::test(flavor = "multi_thread")]
async fn http_count_matches_local_count_and_reports_bytes_and_requests() {
    if fcb_cli_missing() {
        eprintln!("skipping: `fcb` CLI not found on PATH");
        return;
    }

    let parent = tempfile::tempdir().unwrap();
    let fcb_path = generate_fcb("lod3_railway.city.json", parent.path());
    let fcb_name = fcb_path.file_name().unwrap().to_str().unwrap().to_string();

    let addr = spawn_server(parent.path().to_path_buf()).await;
    let base_url = format!("http://{addr}");

    let (local_ok, local_out, local_err) = run_child(&[
        "--child",
        "--format",
        "flatcitybuf",
        "--scenario",
        "count",
        "--input",
        fcb_path.to_str().unwrap(),
    ]);
    assert!(local_ok, "local child failed: {local_err}");
    let local_fields: Vec<&str> = local_out.split_whitespace().collect();
    assert_eq!(local_fields.len(), 4, "local line: {local_out}");
    // fcb info reports 38 features for lod3_railway.city.json (one per
    // top-level CityObject); same value `tests/flatcitybuf_runner.rs`
    // asserts for its own local-transport `count` test.
    assert_eq!(local_fields[3], "38");

    let (http_ok, http_out, http_err) = run_child(&[
        "--child",
        "--format",
        "flatcitybuf",
        "--scenario",
        "count",
        "--transport",
        "http",
        "--base-url",
        &base_url,
        "--input",
        &fcb_name,
    ]);
    assert!(http_ok, "http child failed: {http_err}");
    let http_fields: Vec<&str> = http_out.split_whitespace().collect();
    assert_eq!(http_fields.len(), 6, "http line: {http_out}");
    assert_eq!(
        http_fields[3], local_fields[3],
        "result_count must match between local and http transports"
    );
    let bytes: u64 = http_fields[4].parse().unwrap();
    let requests: u64 = http_fields[5].parse().unwrap();
    assert!(requests >= 1);
    assert!(bytes > 0);
}

/// The last whitespace-separated line of `stdout` — the child's own result
/// line. Taking the LAST one rather than the whole capture keeps this
/// robust against a dependency printing progress of its own before it
/// (`fcb_core` 0.7.6 does, on a numeric-range index query).
fn result_line(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .rfind(|l| !l.trim().is_empty())
        .unwrap_or_default()
        .split_whitespace()
        .map(str::to_string)
        .collect()
}

/// Every FCB scenario that walks features — not just the metadata-only
/// `count` the test above covers — driven over BOTH transports and
/// asserted to agree.
///
/// The runner's local and HTTP paths are two separate implementations of
/// the same five walks (`formats::flatcitybuf`'s `*_http` mirrors), so
/// "they read the same thing" is a claim that has to be executed, not
/// assumed from the fact that they were written together. The absolute
/// numbers are `tests/flatcitybuf_runner.rs`'s own, independently derived
/// from the fixture; asserting parity against the local run as well is what
/// makes a drift in either path fail here.
#[tokio::test(flavor = "multi_thread")]
async fn http_feature_walks_match_the_local_transport() {
    if fcb_cli_missing() {
        eprintln!("skipping: `fcb` CLI not found on PATH");
        return;
    }

    let parent = tempfile::tempdir().unwrap();
    let fcb_path = generate_fcb("lod3_railway.city.json", parent.path());
    let fcb_name = fcb_path.file_name().unwrap().to_str().unwrap().to_string();
    let local_input = fcb_path.to_str().unwrap().to_string();

    let addr = spawn_server(parent.path().to_path_buf()).await;
    let base_url = format!("http://{addr}");

    // (scenario, extra args, the count `tests/flatcitybuf_runner.rs`
    // asserts for the same call on the local transport).
    let cases: [(&str, Vec<&str>, &str); 6] = [
        ("full-read", vec![], "38"),
        (
            "attr-filter",
            vec!["--attr-column", "function", "--attr-eq", "1070"],
            "65",
        ),
        // `object_type` is never in FCB's attribute schema, so this one
        // takes the full raw walk on both transports (`no-attr-index`).
        // 10, counted independently with Python over the raw CityJSON: 10
        // of the fixture's 121 CityObjects are `Railway` (CityObject
        // level, not the 38-feature total — see `formats::flatcitybuf`'s
        // own module doc on the counting grain).
        (
            "attr-filter",
            vec!["--attr-column", "object_type", "--attr-eq", "Railway"],
            "10",
        ),
        (
            "id-lookup",
            vec!["--target-id", "UUID_bd865e62-18de-40ff-85da-883709a86f0f"],
            "1",
        ),
        ("id-lookup", vec!["--target-id", "no-such-id"], "0"),
        // This fixture carries no numeric attribute at all (`function` is
        // a string code), so `attr-stats` is 0 on both transports — the
        // assertion here is that the two walks AGREE and that the HTTP one
        // runs at all, not a magnitude.
        ("attr-stats", vec!["--attr-column", "function"], "0"),
    ];

    for (scenario, extra, expected) in &cases {
        let mut local_args = vec![
            "--child",
            "--format",
            "flatcitybuf",
            "--scenario",
            scenario,
            "--input",
            &local_input,
        ];
        local_args.extend_from_slice(extra);
        let (ok, out, err) = run_child(&local_args);
        assert!(ok, "local child failed ({scenario}): {err}");
        let local = result_line(&out);
        assert_eq!(local.len(), 4, "local line ({scenario}): {out}");
        assert_eq!(
            local[3], *expected,
            "the local transport's own {scenario} count changed"
        );

        let mut http_args = vec![
            "--child",
            "--format",
            "flatcitybuf",
            "--scenario",
            scenario,
            "--transport",
            "http",
            "--base-url",
            &base_url,
            "--input",
            &fcb_name,
        ];
        http_args.extend_from_slice(extra);
        let (ok, out, err) = run_child(&http_args);
        assert!(ok, "http child failed ({scenario}): {err}");
        let http = result_line(&out);
        assert_eq!(http.len(), 6, "http line ({scenario}): {out}");
        assert_eq!(
            http[3], local[3],
            "the http and local transports must agree on {scenario}'s \
             result_count (http {}, local {})",
            http[3], local[3]
        );
        assert!(
            http[5].parse::<u64>().unwrap() >= 1,
            "{scenario} over http must report at least one range request"
        );
    }
}

/// The first line of `stderr` that starts with `marker`, split into the
/// fields after it.
fn marker(stderr: &str, marker: &str) -> Option<Vec<String>> {
    stderr
        .lines()
        .find_map(|l| l.strip_prefix(marker))
        .map(|rest| rest.split_whitespace().map(str::to_string).collect())
}

/// A string-equality attribute filter on the committed Tokyo cut
/// (`usage = "401"`: 11 of its 50 CityObjects, counted with Python over the
/// CityJSONSeq) returns the same objects over HTTP as locally — the same
/// count AND the same identifier digest. On the full Tokyo file the HTTP
/// arm once published 0 objects for this predicate while the local arm
/// found 12,348.
#[tokio::test(flavor = "multi_thread")]
async fn http_string_attr_filter_matches_the_local_transport_on_tokyo() {
    if fcb_cli_missing() {
        eprintln!("skipping: `fcb` CLI not found on PATH");
        return;
    }
    const DIGEST: &str = "cityparquet-readbench: id-digest";

    let parent = tempfile::tempdir().unwrap();
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/tokyo_chiyoda_40.city.jsonl");
    let fcb_path = parent.path().join("tokyo_chiyoda_40.fcb");
    let output = Command::new("fcb")
        .arg("ser")
        .arg(&src)
        .arg(&fcb_path)
        .arg("-A")
        .output()
        .expect("failed to run `fcb ser`");
    assert!(
        output.status.success(),
        "fcb ser failed; stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let local_input = fcb_path.to_str().unwrap().to_string();
    let addr = spawn_server(parent.path().to_path_buf()).await;
    let base_url = format!("http://{addr}");
    let filter = ["--attr-column", "usage", "--attr-eq", "401"];

    let mut local_args = vec![
        "--child",
        "--format",
        "flatcitybuf",
        "--scenario",
        "attr-filter",
        "--input",
        &local_input,
    ];
    local_args.extend_from_slice(&filter);
    let (ok, out, local_err) = run_child(&local_args);
    assert!(ok, "local child failed: {local_err}");
    let local = result_line(&out);
    assert_eq!(local[3], "11", "the local transport's own count changed");

    let mut http_args = vec![
        "--child",
        "--format",
        "flatcitybuf",
        "--scenario",
        "attr-filter",
        "--transport",
        "http",
        "--base-url",
        &base_url,
        "--input",
        "tokyo_chiyoda_40.fcb",
    ];
    http_args.extend_from_slice(&filter);
    let (ok, out, http_err) = run_child(&http_args);
    assert!(ok, "http child failed: {http_err}");
    let http = result_line(&out);
    assert_eq!(
        http[3], local[3],
        "the http and local transports must agree on the string attr-filter \
         (http {}, local {}); http stderr:\n{http_err}",
        http[3], local[3]
    );
    let local_digest = marker(&local_err, DIGEST).expect("local id-digest marker");
    assert_eq!(local_digest[0], "11");
    assert_eq!(
        marker(&http_err, DIGEST).expect("http id-digest marker"),
        local_digest,
        "the http and local transports must return the same identifiers"
    );
}

/// An attribute filter the HTTP index answers with no hits is verified by
/// the full walk, never published as 0 on the index's word: `fcb_core`
/// 0.7.6's HTTP reader also answers 0 for a key naming 4,096 or more
/// features (on Tokyo, `usage = "401"`: 12,348 objects locally, 0 over
/// HTTP). A value absent from the cut (`usage = "999"`) drives that path:
/// the HTTP arm discloses `attr-index-failed`, reads the whole file and
/// returns the same verified 0 as the local arm, which trusts its index.
#[tokio::test(flavor = "multi_thread")]
async fn an_empty_http_attr_index_answer_is_verified_by_the_full_walk() {
    if fcb_cli_missing() {
        eprintln!("skipping: `fcb` CLI not found on PATH");
        return;
    }
    let parent = tempfile::tempdir().unwrap();
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/tokyo_chiyoda_40.city.jsonl");
    let fcb_path = parent.path().join("tokyo_chiyoda_40.fcb");
    let output = Command::new("fcb")
        .arg("ser")
        .arg(&src)
        .arg(&fcb_path)
        .arg("-A")
        .output()
        .expect("failed to run `fcb ser`");
    assert!(output.status.success(), "fcb ser failed");
    let fcb_len = std::fs::metadata(&fcb_path).unwrap().len();
    let local_input = fcb_path.to_str().unwrap().to_string();
    let addr = spawn_server(parent.path().to_path_buf()).await;
    let base_url = format!("http://{addr}");
    let filter = ["--attr-column", "usage", "--attr-eq", "999"];

    let mut local_args = vec![
        "--child",
        "--format",
        "flatcitybuf",
        "--scenario",
        "attr-filter",
        "--input",
        &local_input,
    ];
    local_args.extend_from_slice(&filter);
    let (ok, out, local_err) = run_child(&local_args);
    assert!(ok, "local child failed: {local_err}");
    assert_eq!(result_line(&out)[3], "0");
    assert!(
        !local_err.contains("attr-index-failed"),
        "the local arm trusts its index's empty answer: {local_err}"
    );

    let mut http_args = vec![
        "--child",
        "--format",
        "flatcitybuf",
        "--scenario",
        "attr-filter",
        "--transport",
        "http",
        "--base-url",
        &base_url,
        "--input",
        "tokyo_chiyoda_40.fcb",
    ];
    http_args.extend_from_slice(&filter);
    let (ok, out, http_err) = run_child(&http_args);
    assert!(ok, "http child failed: {http_err}");
    let http = result_line(&out);
    assert_eq!(http[3], "0", "the verified answer is still empty");
    assert!(
        http_err.contains("(attr-index-failed)"),
        "an empty HTTP index answer must be disclosed as a fallback: {http_err}"
    );
    assert!(
        http[4].parse::<u64>().unwrap() >= fcb_len.saturating_sub(64 * 1024),
        "the verifying walk reads the whole file ({} bytes of {fcb_len})",
        http[4]
    );
}
