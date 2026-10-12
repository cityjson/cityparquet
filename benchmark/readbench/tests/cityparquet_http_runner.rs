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

/// The fields after `marker` on the child's stderr marker line.
fn marker_fields(stderr: &str, marker: &str) -> Vec<String> {
    let line = stderr
        .lines()
        .find_map(|l| l.strip_prefix(marker))
        .unwrap_or_else(|| panic!("no '{marker}' line in stderr:\n{stderr}"));
    line.split_whitespace().map(str::to_string).collect()
}

/// FNV-1a 64 count/sum/xor digest — the runner's identifier digest,
/// restated here so the test checks the protocol, not a shared function.
fn digest(ids: &[String]) -> Vec<String> {
    let (mut sum, mut xor) = (0u64, 0u64);
    for id in ids {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for &b in id.as_bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        sum = sum.wrapping_add(h);
        xor ^= h;
    }
    vec![ids.len().to_string(), sum.to_string(), xor.to_string()]
}

/// Every changed scenario's markers, locally and over HTTP, against the
/// library's own answer on the real delft fixture.
#[tokio::test(flavor = "multi_thread")]
async fn the_returned_markers_match_the_library_locally_and_over_http() {
    use cityparquet::query;
    const DIGEST: &str = "cityparquet-readbench: id-digest";
    const TOTALS: &str = "cityparquet-readbench: totals";
    const GEOMETRY: &str = "cityparquet-readbench: returned-geometry";
    const LOOKUP: &str = "cityparquet-readbench: lookup-stats";

    let parent = tempfile::tempdir().unwrap();
    let package_dir = parent.path().join("delft.parquet");
    convert(&ConvertOptions::new(
        fixture("delft.city.jsonl"),
        package_dir.clone(),
    ))
    .unwrap();
    let table = std::fs::read_dir(&package_dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.extension().is_some_and(|x| x == "parquet"))
        .expect("a table file in the package");
    let addr = spawn_server(parent.path().to_path_buf()).await;
    let base_url = format!("http://{addr}");

    let full = query::full_read_visit(&table).unwrap();
    let ext = full.extent;
    let half = [
        ext[0],
        ext[1],
        ext[2],
        (ext[0] + ext[3]) / 2.0,
        ext[4],
        ext[5],
    ];
    let window = query::bbox_query_geometry(&table, half).unwrap();
    assert!(!window.ids.is_empty() && window.geometries > 0);
    let pred = query::AttrPredicate::Eq(serde_json::Value::String("BuildingPart".into()));
    let (matched, _) = query::attr_filter_ids(&table, "object_type", &pred).unwrap();
    let target = window.ids[0].clone();
    let (one, stats) = query::id_lookup_visit(&table, &target).unwrap();

    let bbox = half.map(|v| v.to_string()).join(",");
    let local = package_dir.to_str().unwrap().to_string();
    for http in [false, true] {
        let run = |scenario: &str, extra: &[&str]| {
            let mut args = vec!["--child", "--format", "cityparquet", "--scenario", scenario];
            if http {
                args.extend(["--transport", "http", "--base-url", &base_url]);
                args.extend(["--input", "delft.parquet"]);
            } else {
                args.extend(["--input", &local]);
            }
            args.extend(extra);
            let (ok, out, err) = run_child(&args);
            assert!(ok, "{scenario} (http={http}) failed: {err}");
            (out, err)
        };

        let (_, err) = run("full-read", &[]);
        let t = marker_fields(&err, TOTALS);
        assert_eq!(
            t[..3],
            [full.objects, full.geometries, full.semantic_faces].map(|v| v.to_string())
        );

        let (out, err) = run("bbox-query", &["--bbox", &bbox]);
        assert_eq!(
            out.split_whitespace().nth(3).unwrap(),
            window.ids.len().to_string()
        );
        assert_eq!(marker_fields(&err, DIGEST), digest(&window.ids));
        let g = marker_fields(&err, GEOMETRY);
        assert_eq!(g[0], window.geometries.to_string());
        let visited: Vec<f64> = g[1..].iter().map(|v| v.parse().unwrap()).collect();
        assert_eq!(visited, window.extent.to_vec());

        let (_, err) = run(
            "attr-filter",
            &["--attr-column", "object_type", "--attr-eq", "BuildingPart"],
        );
        assert_eq!(marker_fields(&err, DIGEST), digest(&matched));

        let (_, err) = run("id-lookup", &["--target-id", &target]);
        let t = marker_fields(&err, TOTALS);
        assert_eq!(
            t[..3],
            [one.objects, one.geometries, one.semantic_faces].map(|v| v.to_string())
        );
        assert_eq!(one.objects, 1);
        let l = marker_fields(&err, LOOKUP);
        assert_eq!(
            l.len(),
            4,
            "row_groups_total bloom_pruned filter_bytes stats_pruned"
        );
        assert_eq!(l[3], stats.stats_pruned.to_string());
    }
}
