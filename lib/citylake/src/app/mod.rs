//! The HTTP layer: an axum router and handlers translating between HTTP and
//! [`CityLakeRepository`](crate::core::interface::repository::CityLakeRepository).
//!
//! Handlers hold no SQL, no DuckDB connection and no CityJSON knowledge — they
//! parse a request, call the trait, and shape the response. Error-to-status
//! mapping happens exactly once, below, which is what the repository's error
//! type being an enum (rather than a boxed trait object) is for: a handler
//! cannot classify a string, but a `match` can classify an enum.

pub mod handlers;
pub mod server;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

use crate::core::interface::types::CityLakeError;

/// Turning a CityLakeError into a response.
///
/// Doing this once is why the error type is an enum rather than a boxed trait
/// object: a handler cannot classify a string.
impl IntoResponse for CityLakeError {
    fn into_response(self) -> Response {
        let status = match &self {
            CityLakeError::DatasetNotFound(_) | CityLakeError::ModuleNotFound { .. } => {
                StatusCode::NOT_FOUND
            }
            CityLakeError::DatasetExists(_) => StatusCode::CONFLICT,
            CityLakeError::Sql(_) => StatusCode::BAD_REQUEST,
            // A rejected pragma — a duplicate id, a CRS mismatch — is the
            // caller's input being refused, not the server failing.
            CityLakeError::Duckdb(e) if is_refusal(e) => StatusCode::UNPROCESSABLE_ENTITY,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (
            status,
            Json(serde_json::json!({ "error": self.to_string() })),
        )
            .into_response()
    }
}

/// The extension refuses bad input by raising, so its own refusals read as
/// database errors. These are the caller's fault, not ours.
fn is_refusal(error: &duckdb::Error) -> bool {
    let text = error.to_string();
    [
        "duplicate id",
        "CRS mismatch",
        "unresolved parent",
        "reprojection is not performed",
    ]
    .iter()
    .any(|marker| text.contains(marker))
}
