use axum::extract::{Multipart, Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde_json::json;
use std::sync::Arc;

use crate::core::interface::repository::CityLakeRepository;
use crate::core::interface::types::InsertRequest;

/// POST /tables/:table_name/objects
///
/// Insert CityJSON objects from a server-side file path.
pub async fn insert_objects(
    State(repo): State<Arc<dyn CityLakeRepository>>,
    Path(table_name): Path<String>,
    Json(body): Json<InsertRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let source_path = body.source_path.ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "source_path is required"})),
        )
    })?;

    let count = repo
        .insert_objects(&table_name, &source_path)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
        })?;

    Ok(Json(json!({
        "message": format!("Inserted {} objects into '{}'", count, table_name),
        "count": count,
    })))
}

/// POST /tables/:table_name/objects/upload
///
/// Insert CityJSON objects from an uploaded file (multipart).
pub async fn insert_objects_upload(
    State(repo): State<Arc<dyn CityLakeRepository>>,
    Path(table_name): Path<String>,
    mut multipart: Multipart,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let temp_path = super::table::receive_upload(&mut multipart).await?;

    let count = repo
        .insert_objects(&table_name, &temp_path)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
        })?;

    let _ = std::fs::remove_file(&temp_path);

    Ok(Json(json!({
        "message": format!("Inserted {} objects into '{}'", count, table_name),
        "count": count,
    })))
}
