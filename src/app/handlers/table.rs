use axum::extract::{Multipart, Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

use crate::core::interface::repository::CityLakeRepository;
use crate::core::interface::types::{CreateTableRequest, LodKey};

/// Query parameters supported alongside multipart uploads (which can't carry a JSON body).
#[derive(Debug, Deserialize, Default)]
pub struct CreateTableUploadQuery {
    pub lod: Option<String>,
    pub base_name: Option<String>,
}

/// POST /tables/:base_name
///
/// Create LOD-suffixed tables from a CityJSON source. The path segment is used as
/// the *base* name for derived tables (e.g. `buildings_lod_2_2`). Body fields:
/// - `source_path` (required): server-side file path
/// - `lod` (optional): single LOD to load; otherwise every LOD in the file is loaded
/// - `base_name` (optional): override the path-derived base name
pub async fn create_table(
    State(repo): State<Arc<dyn CityLakeRepository>>,
    Path(path_base): Path<String>,
    Json(body): Json<CreateTableRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let source_path = body.source_path.ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "source_path is required"})),
        )
    })?;

    let lod = parse_lod(body.lod.as_deref())?;
    let base = body.base_name.as_deref().unwrap_or(path_base.as_str());

    let created = repo
        .create_table(Some(base), &source_path, lod.as_ref())
        .await
        .map_err(repo_error)?;

    Ok(Json(json!({
        "message": format!("Created {} table(s) for base '{}'", created.len(), base),
        "base_name": base,
        "tables": created,
    })))
}

/// POST /tables/:base_name/upload
///
/// Multipart upload variant of `create_table`. `?lod=` and `?base_name=` query
/// parameters carry the same data as the JSON body would.
pub async fn create_table_upload(
    State(repo): State<Arc<dyn CityLakeRepository>>,
    Path(path_base): Path<String>,
    Query(qs): Query<CreateTableUploadQuery>,
    mut multipart: Multipart,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let temp_path = receive_upload(&mut multipart).await?;

    let lod = parse_lod(qs.lod.as_deref())?;
    let base = qs.base_name.as_deref().unwrap_or(path_base.as_str());

    let result = repo
        .create_table(Some(base), &temp_path, lod.as_ref())
        .await;

    let _ = std::fs::remove_file(&temp_path);

    let created = result.map_err(repo_error)?;

    Ok(Json(json!({
        "message": format!("Created {} table(s) for base '{}' from upload", created.len(), base),
        "base_name": base,
        "tables": created,
    })))
}

/// Receive a multipart file upload and save to a temp file.
/// Returns the path to the temp file.
pub(crate) async fn receive_upload(
    multipart: &mut Multipart,
) -> Result<String, (StatusCode, Json<serde_json::Value>)> {
    if let Some(field) = multipart.next_field().await.map_err(|e| {
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

        let extension = if file_name.ends_with(".city.json") {
            ".city.json"
        } else if file_name.ends_with(".city.jsonl") || file_name.ends_with(".cityjsonl") {
            ".city.jsonl"
        } else if file_name.ends_with(".fcb") {
            ".fcb"
        } else {
            ".city.jsonl"
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

        // Disarm the TempPath's drop-time cleanup. Otherwise the file is
        // deleted the moment this function returns, and the cityjson
        // extension's `read_*` opens a path that is gone. The caller is
        // responsible for `std::fs::remove_file` once it's done with the path.
        let kept = temp_path.keep().map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("Failed to persist temp file: {e}")})),
            )
        })?;
        return Ok(kept.to_string_lossy().to_string());
    }

    Err((
        StatusCode::BAD_REQUEST,
        Json(json!({"error": "No file found in multipart upload"})),
    ))
}

pub(crate) fn parse_lod(
    raw: Option<&str>,
) -> Result<Option<LodKey>, (StatusCode, Json<serde_json::Value>)> {
    match raw {
        None => Ok(None),
        Some(s) => LodKey::parse(s)
            .map(Some)
            .map_err(|e| (StatusCode::BAD_REQUEST, Json(json!({"error": e})))),
    }
}

/// Map a repository error string to an HTTP error response. Validation-style
/// errors that originate from user input are surfaced as 400; the rest become
/// 500 (genuinely internal). Markers are kept conservative — anything we don't
/// recognise stays 500 so we don't accidentally leak internals as user errors.
pub(crate) fn repo_error(err: impl std::fmt::Display) -> (StatusCode, Json<serde_json::Value>) {
    let msg = err.to_string();
    let user_input_markers = [
        "invalid characters",
        "must start with a letter or underscore",
        "cannot be empty",
        "Cannot detect",
        "missing a '_lod_X_Y' suffix",
        "No LOD geometry columns",
        "Invalid LOD",
        "LOD cannot be empty",
        "Metadata extraction not supported",
    ];
    let status = if user_input_markers.iter().any(|m| msg.contains(m)) {
        StatusCode::BAD_REQUEST
    } else if msg.contains("No record found") {
        StatusCode::NOT_FOUND
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    };
    (status, Json(json!({"error": msg})))
}

