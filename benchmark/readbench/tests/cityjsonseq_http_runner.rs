//! CityJSONSeq HTTP runner: serves `delft.city.jsonl` itself (the
//! `cityjsonseq` format's own artefact IS the original input, per
//! `coordinator::resolve_format_artefact`'s existing local mapping) from an
//! in-test axum+tower-http Range server, then drives the BUILT
//! `cityparquet-readbench --child` binary with `--transport http` against
//! it, asserting `result_count` parity with the equivalent `--transport
//! local` call, and that the whole-object GET reports exactly 1 request and
//! the fixture's exact byte length — a whole-object GET is precisely that by
//! construction, not merely "some positive number".

use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::Command;

use tower_http::services::ServeDir;

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../lib/cityparquet-rs/tests/fixtures")
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

// `flavor = "multi_thread"`: `run_child` makes a blocking
// `std::process::Command::output()` call, which would starve a plain
// current-thread `#[tokio::test]`'s single OS thread and prevent the
// spawned axum server task from ever being polled — the same gotcha fixed
// in `tests/cityparquet_http_runner.rs`/`tests/flatcitybuf_http_runner.rs`.
#[tokio::test(flavor = "multi_thread")]
async fn http_count_matches_local_count_and_is_exactly_one_whole_object_get() {
    let fixture_path = fixture("delft.city.jsonl");
    let fixture_dir = fixture_path.parent().unwrap().to_path_buf();
    let fixture_name = fixture_path.file_name().unwrap().to_str().unwrap();
    let expected_bytes = std::fs::metadata(&fixture_path).unwrap().len();

    let addr = spawn_server(fixture_dir).await;
    let base_url = format!("http://{addr}");

    let (local_ok, local_out, local_err) = run_child(&[
        "--child",
        "--format",
        "cityjsonseq",
        "--scenario",
        "count",
        "--input",
        fixture_path.to_str().unwrap(),
    ]);
    assert!(local_ok, "local child failed: {local_err}");
    let local_fields: Vec<&str> = local_out.split_whitespace().collect();
    assert_eq!(local_fields.len(), 4, "local line: {local_out}");

    let (http_ok, http_out, http_err) = run_child(&[
        "--child",
        "--format",
        "cityjsonseq",
        "--scenario",
        "count",
        "--transport",
        "http",
        "--base-url",
        &base_url,
        "--input",
        fixture_name,
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
    assert_eq!(
        requests, 1,
        "a whole-object GET is exactly one request by construction"
    );
    assert_eq!(
        bytes, expected_bytes,
        "a whole-object GET transfers exactly the file's own byte length"
    );
}
