//! Shared setup for the integration tests.
//!
//! There is no offline mode. Every operation in this crate is a pragma, so a
//! service without the extension would exercise nothing.

// Cargo compiles this module separately into every integration-test binary, so
// a helper one test file does not call is dead code *there* even though another
// binary uses it. Without this the crate-wide `-D warnings` gate fails.
#![allow(dead_code)]

use citylake::core::db::service::DuckLakeService;
use citylake::core::interface::types::CityLakeConfig;
use std::path::PathBuf;
use tempfile::TempDir;

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

/// Path to a committed test fixture.
pub fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("data")
        .join(name)
}
