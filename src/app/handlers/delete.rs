use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde_json::json;
use std::sync::Arc;

use crate::core::interface::repository::CityLakeRepository;

/// DELETE /tables/:table_name/objects/:id
///
/// Delete a CityJSON object by its ID.
pub async fn delete_object(
    State(repo): State<Arc<dyn CityLakeRepository>>,
    Path((table_name, id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    repo.delete_object(&table_name, &id)
        .await
        .map_err(|e| {
            let status = if e.to_string().contains("No record found") {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            (status, Json(json!({"error": e.to_string()})))
        })?;

    Ok(Json(json!({
        "message": format!("Deleted object '{}' from table '{}'", id, table_name),
    })))
}
