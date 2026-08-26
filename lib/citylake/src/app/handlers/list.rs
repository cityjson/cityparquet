use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde_json::json;
use std::sync::Arc;

use crate::core::interface::repository::CityLakeRepository;

use super::table::repo_error;

/// GET /tables
///
/// List every table in the citylake catalog. Each entry carries the parsed
/// `(base, lod)` derived from a `_lod_X_Y` suffix when present.
pub async fn list_tables(
    State(repo): State<Arc<dyn CityLakeRepository>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let tables = repo.list_tables().await.map_err(repo_error)?;
    Ok(Json(json!({
        "count": tables.len(),
        "tables": tables,
    })))
}
