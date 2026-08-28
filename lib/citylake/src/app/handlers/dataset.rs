//! Dataset lifecycle: create (from a source path or an upload), list,
//! describe, drop.

use std::sync::Arc;

use axum::extract::{Multipart, Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;

use crate::core::interface::repository::CityLakeRepository;
use crate::core::interface::types::{CityLakeError, DatasetInfo, DatasetName};

use super::receive_upload;

#[derive(Debug, Deserialize)]
pub struct CreateDatasetBody {
    source_path: String,
}

/// `POST /datasets/{ds}` — bootstrap a dataset from a server-side source path.
pub async fn create(
    State(repo): State<Arc<dyn CityLakeRepository>>,
    Path(dataset): Path<String>,
    Json(body): Json<CreateDatasetBody>,
) -> Result<(StatusCode, Json<DatasetInfo>), CityLakeError> {
    let dataset = DatasetName::new(&dataset)?;
    let info = repo.create_dataset(&dataset, &body.source_path).await?;
    Ok((StatusCode::CREATED, Json(info)))
}

/// `POST /datasets/{ds}/upload` — the multipart variant of [`create`].
pub async fn create_upload(
    State(repo): State<Arc<dyn CityLakeRepository>>,
    Path(dataset): Path<String>,
    multipart: Multipart,
) -> Result<(StatusCode, Json<DatasetInfo>), CityLakeError> {
    let dataset = DatasetName::new(&dataset)?;
    let temp = receive_upload(multipart).await?;
    let source_path = temp
        .path()
        .to_str()
        .ok_or_else(|| CityLakeError::Internal("temp file path is not valid UTF-8".to_string()))?;
    let info = repo.create_dataset(&dataset, source_path).await?;
    Ok((StatusCode::CREATED, Json(info)))
}

/// `GET /datasets` — every dataset's name.
pub async fn list(
    State(repo): State<Arc<dyn CityLakeRepository>>,
) -> Result<Json<Vec<String>>, CityLakeError> {
    Ok(Json(repo.list_datasets().await?))
}

/// `GET /datasets/{ds}` — a dataset's modules and CRS.
pub async fn describe(
    State(repo): State<Arc<dyn CityLakeRepository>>,
    Path(dataset): Path<String>,
) -> Result<Json<DatasetInfo>, CityLakeError> {
    let dataset = DatasetName::new(&dataset)?;
    Ok(Json(repo.describe_dataset(&dataset).await?))
}

/// `DELETE /datasets/{ds}`.
pub async fn drop_dataset(
    State(repo): State<Arc<dyn CityLakeRepository>>,
    Path(dataset): Path<String>,
) -> Result<StatusCode, CityLakeError> {
    let dataset = DatasetName::new(&dataset)?;
    repo.drop_dataset(&dataset).await?;
    Ok(StatusCode::NO_CONTENT)
}
