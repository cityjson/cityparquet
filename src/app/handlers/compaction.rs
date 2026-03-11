use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde_json::json;
use std::sync::Arc;

use crate::core::interface::repository::CityLakeRepository;

/// POST /tables/:table_name/compact
///
/// Trigger compaction for a table.
pub async fn compact_table(
    State(repo): State<Arc<dyn CityLakeRepository>>,
    Path(table_name): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let stats = repo.compact_table(&table_name).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
    })?;

    Ok(Json(json!({
        "message": format!("Compacted table '{}'", table_name),
        "stats": stats,
    })))
}
