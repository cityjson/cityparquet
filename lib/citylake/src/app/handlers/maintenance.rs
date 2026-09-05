//! Housekeeping over an existing dataset: validate its invariants, reconcile
//! derived state, reclaim vacuumed rows, compact its Parquet files.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;

use crate::core::interface::repository::CityLakeRepository;
use crate::core::interface::types::{
    CityLakeError, CompactionStats, DatasetName, ValidationFinding,
};

/// `POST /datasets/{ds}/validate`.
pub async fn validate(
    State(repo): State<Arc<dyn CityLakeRepository>>,
    Path(dataset): Path<String>,
) -> Result<Json<Vec<ValidationFinding>>, CityLakeError> {
    let dataset = DatasetName::new(&dataset)?;
    Ok(Json(repo.validate(&dataset).await?))
}

/// `POST /datasets/{ds}/reconcile`.
pub async fn reconcile(
    State(repo): State<Arc<dyn CityLakeRepository>>,
    Path(dataset): Path<String>,
) -> Result<StatusCode, CityLakeError> {
    let dataset = DatasetName::new(&dataset)?;
    repo.reconcile(&dataset).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /datasets/{ds}/vacuum`.
pub async fn vacuum(
    State(repo): State<Arc<dyn CityLakeRepository>>,
    Path(dataset): Path<String>,
) -> Result<Json<serde_json::Value>, CityLakeError> {
    let dataset = DatasetName::new(&dataset)?;
    let vacuumed = repo.vacuum(&dataset).await?;
    Ok(Json(serde_json::json!({ "vacuumed": vacuumed })))
}

/// `POST /datasets/{ds}/compact`.
pub async fn compact(
    State(repo): State<Arc<dyn CityLakeRepository>>,
    Path(dataset): Path<String>,
) -> Result<Json<CompactionStats>, CityLakeError> {
    let dataset = DatasetName::new(&dataset)?;
    Ok(Json(repo.compact(&dataset).await?))
}
