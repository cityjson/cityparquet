use axum::extract::{Multipart, Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

use crate::core::interface::repository::CityLakeRepository;
use crate::core::interface::types::InsertRequest;

use super::table::{parse_lod, repo_error};

#[derive(Debug, Deserialize, Default)]
pub struct InsertUploadQuery {
    pub lod: Option<String>,
}

/// POST /tables/:base_name/objects
///
/// Insert CityJSON objects from a server-side file path. The path segment is the
/// *base name*; the actual target table is `{base_name}_lod_X_Y` per LOD
/// (filtered by `body.lod` when provided, otherwise all LODs found in the file).
pub async fn insert_objects(
    State(repo): State<Arc<dyn CityLakeRepository>>,
    Path(base_name): Path<String>,
    Json(body): Json<InsertRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let source_path = body.source_path.ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "source_path is required"})),
        )
    })?;

    let lod = parse_lod(body.lod.as_deref())?;

    let count = repo
        .insert_objects(&base_name, &source_path, lod.as_ref())
        .await
        .map_err(repo_error)?;

    Ok(Json(json!({
        "message": format!("Inserted {count} objects under base '{base_name}'"),
        "count": count,
    })))
}

/// POST /tables/:base_name/objects/upload
pub async fn insert_objects_upload(
    State(repo): State<Arc<dyn CityLakeRepository>>,
    Path(base_name): Path<String>,
    Query(qs): Query<InsertUploadQuery>,
    mut multipart: Multipart,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let temp_path = super::table::receive_upload(&mut multipart).await?;

    let lod = parse_lod(qs.lod.as_deref())?;

    let result = repo
        .insert_objects(&base_name, &temp_path, lod.as_ref())
        .await;

    let _ = std::fs::remove_file(&temp_path);

    let count = result.map_err(repo_error)?;

    Ok(Json(json!({
        "message": format!("Inserted {count} objects under base '{base_name}' from upload"),
        "count": count,
    })))
}
