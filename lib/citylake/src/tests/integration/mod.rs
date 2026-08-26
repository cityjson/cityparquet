//! Integration tests against the real cityjson + ducklake DuckDB extensions.
//!
//! These tests are gated with `#[ignore]` so they do not run during the default
//! `cargo test` invocation: they download the cityjson extension from the
//! community repo on first run (and the ducklake extension), and then read
//! sample CityJSON datasets directly from public HTTPS URLs via the extension's
//! built-in `httpfs` support. Run them explicitly with:
//!
//! ```text
//! cargo test --lib -- --ignored
//! ```
//!
//! Each test creates an isolated `DuckLakeService` backed by a per-test
//! `tempfile::TempDir` so storage and catalog state never leak between tests
//! or between runs.

mod round_trip;
