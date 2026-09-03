//! Shared setup for the integration tests.
//!
//! There is no offline mode. Every operation in this crate is a pragma, so a
//! service without the extension would exercise nothing.

// Cargo compiles this module separately into every integration-test binary, so
// a helper one test file does not call is dead code *there* even though another
// binary uses it. Without this the crate-wide `-D warnings` gate fails.
#![allow(dead_code)]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use citylake::core::db::service::DuckLakeService;
use citylake::core::interface::types::CityLakeConfig;
use http_body_util::BodyExt;
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;
use tower::ServiceExt;

/// A service over a throwaway DuckLake catalog. The returned TempDir must stay
/// alive for the test's duration — dropping it removes the catalog.
pub fn test_service() -> (DuckLakeService, TempDir) {
    let dir = TempDir::new().expect("create a temporary directory");
    let config = CityLakeConfig {
        storage_path: dir.path().join("data").to_string_lossy().into_owned(),
        catalog_path: dir
            .path()
            .join("meta.ducklake")
            .to_string_lossy()
            .into_owned(),
        ..Default::default()
    };
    let service = DuckLakeService::new(config).expect("start a service");
    (service, dir)
}

/// A router whose output root is a real, existing directory — the case
/// where `export`/`package` requests inside it are allowed and requests
/// outside it are refused.
pub fn app_with_output_root() -> (axum::Router, TempDir) {
    let (service, dir) = test_service();
    let output_root = dir.path().join("output-root");
    std::fs::create_dir_all(&output_root).expect("create the output root");
    let app = citylake::app::server::router(
        Arc::new(service),
        Some(output_root.to_string_lossy().into_owned()),
    );
    (app, dir)
}

/// A router with no output root configured — every write endpoint refuses.
pub fn app_without_output_root() -> (axum::Router, TempDir) {
    let (service, dir) = test_service();
    let app = citylake::app::server::router(Arc::new(service), None);
    (app, dir)
}

/// Path to a committed test fixture.
pub fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("data")
        .join(name)
}

/// Drive one request through a router and collect its status and JSON body.
pub async fn send(app: &axum::Router, request: Request<Body>) -> (StatusCode, serde_json::Value) {
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}
