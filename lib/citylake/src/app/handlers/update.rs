use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde_json::json;
use std::sync::Arc;

use crate::core::interface::repository::CityLakeRepository;
use crate::core::interface::types::UpdateRequest;

use super::table::repo_error;

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
        .map_err(repo_error)?;

    Ok(Json(json!({
        "message": format!("Updated object '{}' in table '{}'", id, table_name),
    })))
}
