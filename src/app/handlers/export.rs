use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde_json::json;
use std::sync::Arc;

use crate::core::interface::repository::CityLakeRepository;
use crate::core::interface::types::ExportRequest;

/// POST /tables/:table_name/export
///
/// Export a table to a CityJSON format file.
pub async fn export_table(
    State(repo): State<Arc<dyn CityLakeRepository>>,
    Path(table_name): Path<String>,
    Json(body): Json<ExportRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    repo.export_table(&table_name, &body.output_path, body.format)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
        })?;

    Ok(Json(json!({
        "message": format!("Exported table '{}' to '{}'", table_name, body.output_path),
        "format": body.format.as_duckdb_format(),
    })))
}
