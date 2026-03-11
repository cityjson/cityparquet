use axum::extract::{Multipart, Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde_json::json;
use std::sync::Arc;

use crate::core::interface::repository::CityLakeRepository;
use crate::core::interface::types::CreateTableRequest;

/// POST /tables/:table_name
///
/// Create a new table from a CityJSON source. Accepts either:
/// - JSON body with `source_path` (server-side file)
/// - Multipart file upload
pub async fn create_table(
    State(repo): State<Arc<dyn CityLakeRepository>>,
    Path(table_name): Path<String>,
    Json(body): Json<CreateTableRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let source_path = body.source_path.ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "source_path is required"})),
        )
    })?;

    repo.create_table(&table_name, &source_path)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
        })?;

    Ok(Json(json!({
        "message": format!("Table '{}' created successfully", table_name),
        "table_name": table_name,
    })))
}

/// POST /tables/:table_name/upload
///
/// Create a new table from an uploaded CityJSON file (multipart).
pub async fn create_table_upload(
    State(repo): State<Arc<dyn CityLakeRepository>>,
    Path(table_name): Path<String>,
    mut multipart: Multipart,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let temp_path = receive_upload(&mut multipart).await?;

    repo.create_table(&table_name, &temp_path)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
        })?;

    // Clean up temp file (best effort)
    let _ = std::fs::remove_file(&temp_path);

    Ok(Json(json!({
        "message": format!("Table '{}' created successfully from upload", table_name),
        "table_name": table_name,
    })))
}

/// Receive a multipart file upload and save to a temp file.
/// Returns the path to the temp file.
pub(crate) async fn receive_upload(
    multipart: &mut Multipart,
) -> Result<String, (StatusCode, Json<serde_json::Value>)> {
    while let Some(field) = multipart.next_field().await.map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": format!("Failed to read multipart field: {e}")})),
        )
    })? {
        let file_name = field
            .file_name()
            .unwrap_or("upload.city.jsonl")
            .to_string();

        let data = field.bytes().await.map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": format!("Failed to read file data: {e}")})),
            )
        })?;

        // Determine extension from original filename
        let extension = if file_name.ends_with(".city.json") {
            ".city.json"
        } else if file_name.ends_with(".city.jsonl") || file_name.ends_with(".cityjsonl") {
            ".city.jsonl"
        } else if file_name.ends_with(".fcb") {
            ".fcb"
        } else {
            ".city.jsonl" // default
        };

        let temp_file = tempfile::Builder::new()
            .suffix(extension)
            .tempfile()
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": format!("Failed to create temp file: {e}")})),
                )
            })?;

        let temp_path = temp_file.into_temp_path();
        std::fs::write(&temp_path, &data).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("Failed to write temp file: {e}")})),
            )
        })?;

        return Ok(temp_path.to_string_lossy().to_string());
    }

    Err((
        StatusCode::BAD_REQUEST,
        Json(json!({"error": "No file found in multipart upload"})),
    ))
}
