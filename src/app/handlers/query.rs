use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde_json::json;
use std::sync::Arc;

use crate::core::interface::repository::CityLakeRepository;
use crate::core::interface::types::QueryParams;

use super::table::repo_error;

/// GET /tables/:table_name/objects
///
/// Query objects from a table. Supports optional query parameters:
/// - `filter` — SQL WHERE clause (e.g., `object_type = 'Building'`)
/// - `limit` — max rows to return
/// - `offset` — pagination offset
pub async fn query_objects(
    State(repo): State<Arc<dyn CityLakeRepository>>,
    Path(table_name): Path<String>,
    Query(params): Query<QueryParams>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let results = repo
        .query_objects(&table_name, &params)
        .await
        .map_err(repo_error)?;

    Ok(Json(json!({
        "table": table_name,
        "count": results.len(),
        "objects": results,
    })))
}
