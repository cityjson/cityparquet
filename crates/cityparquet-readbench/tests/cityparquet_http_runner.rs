//! CityParquet HTTP runner: serves a real converted package directory from
//! an in-test axum+tower-http Range server, then drives the BUILT
//! `cityparquet-readbench --child` binary with `--transport http` against
//! it, asserting `result_count` parity with the equivalent `--transport
//! local` call and that a bytes/requests pair is reported.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::Command;

use cityparquet::package::{ConvertOptions, convert};
use tower_http::services::ServeDir;

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
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

// `flavor = "multi_thread"` is required: `run_child` below makes a BLOCKING
// `std::process::Command::output()` call, which would otherwise starve the
// single OS thread a plain `#[tokio::test]` (current-thread runtime) gives
// this test, so the `tokio::spawn`'d axum server task would never actually
// get polled/accept connections — confirmed by reproducing the hang (even
// `curl` against the server's port blocked) before adding this flavor.
#[tokio::test(flavor = "multi_thread")]
async fn http_count_matches_local_count_and_reports_bytes_and_requests() {
    // `ConvertOptions::new(input, output_dir)` writes the package DIRECTLY
    // into `output_dir` (no auto-created nested subdirectory — that's
    // `readbench_prepare.sh`'s own convention, not this library call's), so
    // the nested `"delft.parquet"` package directory is constructed
    // explicitly here to mirror the real prepared-dir layout: `parent/`
    // served over HTTP, `parent/delft.parquet/` the package itself.
    let parent = tempfile::tempdir().unwrap();
    let package_dir_name = "delft.parquet";
    let package_dir = parent.path().join(package_dir_name);
    let opts = ConvertOptions::new(fixture("delft.city.jsonl"), package_dir.clone());
    convert(&opts).unwrap();

    let addr = spawn_server(parent.path().to_path_buf()).await;
    let base_url = format!("http://{addr}");

    let local_input = package_dir;
    let (local_ok, local_out, local_err) = run_child(&[
        "--child",
        "--format",
        "cityparquet",
        "--scenario",
        "count",
        "--input",
        local_input.to_str().unwrap(),
    ]);
    assert!(local_ok, "local child failed: {local_err}");
    let local_fields: Vec<&str> = local_out.split_whitespace().collect();
    assert_eq!(local_fields.len(), 4, "local line: {local_out}");

    let (http_ok, http_out, http_err) = run_child(&[
        "--child",
        "--format",
        "cityparquet",
        "--scenario",
        "count",
        "--transport",
        "http",
        "--base-url",
        &base_url,
        "--input",
        package_dir_name,
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
