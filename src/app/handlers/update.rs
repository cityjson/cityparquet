use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde_json::json;
use std::sync::Arc;

use crate::core::interface::repository::CityLakeRepository;
use crate::core::interface::types::UpdateRequest;

/// PUT /tables/:table_name/objects/:id
///
/// Update a CityJSON object by its ID.
pub async fn update_object(
    State(repo): State<Arc<dyn CityLakeRepository>>,
    Path((table_name, id)): Path<(String, String)>,
    Json(body): Json<UpdateRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    repo.update_object(&table_name, &id, &body.cityjson_data)
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
        "message": format!("Updated object '{}' in table '{}'", id, table_name),
    })))
}
