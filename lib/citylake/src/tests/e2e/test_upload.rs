//! HTTP-level e2e tests for the multipart upload pipeline.
//!
//! Earlier e2e tests (`test_api.rs`) ran against `setup_with_table()` — a
//! synthetic VARCHAR schema that never invokes the cityjson extension. The
//! multipart parser, tempfile handoff, LOD discovery, and DuckLake-backed
//! `CREATE TABLE` were therefore untested at the HTTP boundary.
//!
//! These tests drive the same axum router used in production, against a real
//! `DuckLakeService` with the cityjson + ducklake extensions loaded. The
//! cityjson community extension is published only for DuckDB v1.5.0/v1.5.1, so
//! `cargo` must be able to fetch (or have cached) the matching binary.
//!
//! Tagged `#[ignore]` to keep `just test` offline-by-default. Run with
//! `just test-integration` (or `cargo test --lib -- --ignored`).

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use tower::ServiceExt;

use crate::app::server::build_router as build_production_router;
use crate::core::db::service::DuckLakeService;
use crate::core::interface::repository::CityLakeRepository;
use crate::core::interface::types::CityLakeConfig;

/// Path to the bundled 4-line Delft sample (LOD 2.2 only).
fn delft_jsonl_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/tests/data/delft.city.jsonl")
}

/// Build a fresh service rooted at a temp directory. The bundled .ducklake
/// catalog plus its data dir live under the tempdir, so each test is isolated.
fn fresh_service() -> (DuckLakeService, tempfile::TempDir) {
    let tmp = tempfile::tempdir().expect("create tempdir");
    let storage = tmp.path().join("data");
    let catalog = tmp.path().join("metadata.ducklake");
    let config = CityLakeConfig {
        storage_path: storage.to_string_lossy().to_string(),
        catalog_path: catalog.to_string_lossy().to_string(),
        ..Default::default()
    };
    let svc = DuckLakeService::new(config).expect("init DuckLakeService with extensions");
    (svc, tmp)
}

/// Reuse the same router we serve in production so body-limit and middleware
/// behaviour match what the browser sees.
fn build_router(repo: Arc<dyn CityLakeRepository>) -> Router {
    build_production_router(repo)
}

/// Hand-built multipart body. We avoid pulling in `reqwest::multipart` here:
/// `oneshot` consumes a `Body`, and tower's plain bytes path is more reliable
/// for unit tests than spawning an actual HTTP server.
fn multipart_body(file_name: &str, file_bytes: &[u8]) -> (String, Vec<u8>) {
    let boundary = "citylakeboundary7f9";
    let content_type = format!("multipart/form-data; boundary={boundary}");

    let mut body: Vec<u8> = Vec::new();
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        format!(
            "Content-Disposition: form-data; name=\"file\"; filename=\"{file_name}\"\r\n\
             Content-Type: application/octet-stream\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(file_bytes);
    body.extend_from_slice(b"\r\n");
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());

    (content_type, body)
}

async fn body_json(body: Body) -> Value {
    let bytes = body.collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes)
        .unwrap_or_else(|e| panic!("response body was not JSON: {e} (body={:?})", &bytes[..]))
}

// ---- /tables/{base}/upload --------------------------------------------------

#[tokio::test]
#[ignore = "loads cityjson + ducklake extensions; opt-in with --ignored"]
async fn upload_create_table_creates_lod_table_for_bundled_delft() {
    let (svc, _tmp) = fresh_service();
    let repo: Arc<dyn CityLakeRepository> = Arc::new(svc);
    let app = build_router(repo.clone());

    let bytes = std::fs::read(delft_jsonl_path()).expect("read bundled delft jsonl");
    let (ct, body) = multipart_body("delft.city.jsonl", &bytes);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/tables/buildings/upload")
                .header("content-type", ct)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    let status = response.status();
    let payload = body_json(response.into_body()).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "upload should succeed; got {status} body={payload}"
    );
    assert_eq!(payload["base_name"], "buildings");

    let tables = payload["tables"].as_array().expect("tables array");
    assert!(!tables.is_empty(), "expected at least one LOD table");
    assert!(
        tables.iter().any(|t| t == "buildings_lod_2_2"),
        "expected buildings_lod_2_2 in {tables:?}"
    );

    // Each created table must have rows.
    for t in tables {
        let name = t.as_str().unwrap();
        let resp = build_router(repo.clone())
            .oneshot(
                Request::builder()
                    .uri(format!("/tables/{name}/objects"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let q = body_json(resp.into_body()).await;
        let count = q["count"].as_u64().expect("count number");
        assert!(count > 0, "table {name} should have rows");
    }
}

#[tokio::test]
#[ignore = "loads cityjson + ducklake extensions; opt-in with --ignored"]
async fn upload_create_table_pins_lod_via_querystring() {
    let (svc, _tmp) = fresh_service();
    let repo: Arc<dyn CityLakeRepository> = Arc::new(svc);
    let app = build_router(repo);

    let bytes = std::fs::read(delft_jsonl_path()).unwrap();
    let (ct, body) = multipart_body("delft.city.jsonl", &bytes);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/tables/pinned/upload?lod=2.2")
                .header("content-type", ct)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let payload = body_json(response.into_body()).await;
    let tables = payload["tables"].as_array().unwrap();
    assert_eq!(tables.len(), 1, "single LOD pin should make exactly one table");
    assert_eq!(tables[0], "pinned_lod_2_2");
}

#[tokio::test]
#[ignore = "loads cityjson + ducklake extensions; opt-in with --ignored"]
async fn upload_create_table_overrides_base_via_querystring() {
    let (svc, _tmp) = fresh_service();
    let repo: Arc<dyn CityLakeRepository> = Arc::new(svc);
    let app = build_router(repo);

    let bytes = std::fs::read(delft_jsonl_path()).unwrap();
    let (ct, body) = multipart_body("delft.city.jsonl", &bytes);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                // path segment is "ignored" but query base_name should win
                .uri("/tables/ignored/upload?base_name=delft_buildings")
                .header("content-type", ct)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let payload = body_json(response.into_body()).await;
    assert_eq!(payload["base_name"], "delft_buildings");
    let tables = payload["tables"].as_array().unwrap();
    assert!(tables.iter().any(|t| t == "delft_buildings_lod_2_2"));
}

#[tokio::test]
#[ignore = "loads cityjson + ducklake extensions; opt-in with --ignored"]
async fn upload_rejects_base_name_starting_with_digit() {
    // Regression: a filename like "9_508_648.city.jsonl" yielded a base name
    // beginning with a digit, which DuckDB then rejected mid-CREATE TABLE with
    // a SQL parse error. The validator now stops it at 400.
    let (svc, _tmp) = fresh_service();
    let repo: Arc<dyn CityLakeRepository> = Arc::new(svc);
    let app = build_router(repo);

    let bytes = std::fs::read(delft_jsonl_path()).unwrap();
    let (ct, body) = multipart_body("delft.city.jsonl", &bytes);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/tables/9_508_648/upload")
                .header("content-type", ct)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let payload = body_json(response.into_body()).await;
    let err = payload["error"].as_str().unwrap();
    assert!(
        err.contains("must start with a letter or underscore"),
        "error should explain the SQL identifier rule: {err}"
    );
}

#[tokio::test]
#[ignore = "loads cityjson + ducklake extensions; opt-in with --ignored"]
async fn upload_rejects_invalid_base_name() {
    let (svc, _tmp) = fresh_service();
    let repo: Arc<dyn CityLakeRepository> = Arc::new(svc);
    let app = build_router(repo);

    let bytes = std::fs::read(delft_jsonl_path()).unwrap();
    let (ct, body) = multipart_body("delft.city.jsonl", &bytes);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/tables/bad-name!/upload")
                .header("content-type", ct)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let payload = body_json(response.into_body()).await;
    let err = payload["error"].as_str().unwrap();
    assert!(
        err.contains("invalid characters"),
        "error should call out invalid characters: {err}"
    );
}

#[tokio::test]
#[ignore = "loads cityjson + ducklake extensions; opt-in with --ignored"]
async fn upload_rejects_unknown_extension() {
    let (svc, _tmp) = fresh_service();
    let repo: Arc<dyn CityLakeRepository> = Arc::new(svc);
    let app = build_router(repo);

    // Bytes are valid JSONL but we present an extension the format detector
    // doesn't recognise — the multipart receiver assumes .city.jsonl when the
    // extension is unknown, so this currently *succeeds*. We document that
    // behaviour by asserting a 200 with an LOD table; if the receiver is
    // tightened later, flip this test's assertion.
    let bytes = std::fs::read(delft_jsonl_path()).unwrap();
    let (ct, body) = multipart_body("mystery.bin", &bytes);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/tables/mystery/upload")
                .header("content-type", ct)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "unknown extension currently falls back to .city.jsonl"
    );
}

#[tokio::test]
#[ignore = "loads cityjson + ducklake extensions; opt-in with --ignored"]
async fn upload_persists_metadata_row() {
    let (svc, _tmp) = fresh_service();
    let repo: Arc<dyn CityLakeRepository> = Arc::new(svc);
    let app = build_router(repo);

    let bytes = std::fs::read(delft_jsonl_path()).unwrap();
    let (ct, body) = multipart_body("delft.city.jsonl", &bytes);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/tables/delft/upload")
                .header("content-type", ct)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // The shared metadata table should now hold one row keyed to "delft".
    let response = app
        .oneshot(
            Request::builder()
                .uri("/tables/cityjson_metadata/objects")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let payload = body_json(response.into_body()).await;
    assert_eq!(payload["count"], 1);
    let row = &payload["objects"][0];
    assert_eq!(row["dataset"], "delft");
    // For multipart uploads the recorded `source_path` is the server-side
    // tempfile path. We don't currently round-trip the original filename, so we
    // only verify the metadata row exists and the format suffix survives.
    assert!(
        row["source_path"]
            .as_str()
            .unwrap()
            .ends_with(".city.jsonl"),
        "source_path should still carry the .city.jsonl suffix"
    );
}

// ---- /tables/{base}/objects/upload ------------------------------------------

#[tokio::test]
#[ignore = "loads cityjson + ducklake extensions; opt-in with --ignored"]
async fn upload_insert_into_existing_lod_table_doubles_count() {
    let (svc, _tmp) = fresh_service();
    let repo: Arc<dyn CityLakeRepository> = Arc::new(svc);
    let app = build_router(repo.clone());

    let bytes = std::fs::read(delft_jsonl_path()).unwrap();

    // First upload: create the table.
    let (ct, body) = multipart_body("delft.city.jsonl", &bytes);
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/tables/delft/upload")
                .header("content-type", ct)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Read the row count before the duplicate insert.
    let before_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/tables/delft_lod_2_2/objects")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let before = body_json(before_resp.into_body()).await["count"]
        .as_u64()
        .unwrap();
    assert!(before > 0);

    // Second upload via the /objects/upload endpoint (insert path).
    let (ct, body) = multipart_body("delft.city.jsonl", &bytes);
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/tables/delft/objects/upload")
                .header("content-type", ct)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Row count should have doubled.
    let after_resp = app
        .oneshot(
            Request::builder()
                .uri("/tables/delft_lod_2_2/objects")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let after = body_json(after_resp.into_body()).await["count"]
        .as_u64()
        .unwrap();
    assert_eq!(after, before * 2, "insert path should double the row count");
}

// ---- body-size guardrail ----------------------------------------------------

/// Axum's default body limit is 2 MiB. Real CityJSON files can comfortably blow
/// past that. This test confirms the dev configuration accepts at least a 5 MiB
/// upload — if we ever introduce `DefaultBodyLimit::disable()` or set a higher
/// cap, this test guards the behaviour.
#[tokio::test]
#[ignore = "loads cityjson + ducklake extensions; opt-in with --ignored"]
async fn upload_accepts_payload_larger_than_default_2mib_limit() {
    let (svc, _tmp) = fresh_service();
    let repo: Arc<dyn CityLakeRepository> = Arc::new(svc);
    let app = build_router(repo);

    // Pad the bundled file by repeating the JSONL feature lines until the body
    // crosses 5 MiB. Each repeated feature line is a self-contained JSONL row,
    // so the cityjson extension can still parse the result.
    let original = std::fs::read_to_string(delft_jsonl_path()).unwrap();
    let mut lines: Vec<&str> = original.lines().collect();
    let header = lines.remove(0);
    let features = lines.join("\n");
    let mut padded = String::with_capacity(5 * 1024 * 1024 + features.len());
    padded.push_str(header);
    padded.push('\n');
    while padded.len() < 5 * 1024 * 1024 {
        padded.push_str(&features);
        padded.push('\n');
    }

    let (ct, body) = multipart_body("big.city.jsonl", padded.as_bytes());

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/tables/big/upload")
                .header("content-type", ct)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    let status = response.status();
    let payload = body_json(response.into_body()).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "5 MiB upload should be accepted; got {status}, body={payload}"
    );
}
