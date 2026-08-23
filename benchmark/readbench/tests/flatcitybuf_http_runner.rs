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
