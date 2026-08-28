//! The package-shaped operations the extension makes cheap: exporting one
//! module to a CityJSON-family file, writing a whole dataset out as a
//! CityParquet package, merging one dataset into another.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;

use crate::core::interface::repository::CityLakeRepository;
use crate::core::interface::types::{
    CityLakeError, DatasetName, ExportFormat, ModuleName, PackageFile,
};

#[derive(Debug, Deserialize)]
pub struct ExportBody {
    module: String,
    output_path: String,
    format: ExportFormat,
}

/// `POST /datasets/{ds}/export` — export one module to a single
/// CityJSON-family file.
pub async fn export(
    State(repo): State<Arc<dyn CityLakeRepository>>,
    Path(dataset): Path<String>,
    Json(body): Json<ExportBody>,
) -> Result<StatusCode, CityLakeError> {
    let dataset = DatasetName::new(&dataset)?;
    let module = ModuleName::new(&body.module)?;
    repo.export_module(&dataset, &module, &body.output_path, body.format)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
pub struct WritePackageBody {
    output_dir: String,
}

/// `POST /datasets/{ds}/package` — write the dataset out as a CityParquet
/// package directory.
pub async fn write_package(
    State(repo): State<Arc<dyn CityLakeRepository>>,
    Path(dataset): Path<String>,
    Json(body): Json<WritePackageBody>,
) -> Result<Json<Vec<PackageFile>>, CityLakeError> {
    let dataset = DatasetName::new(&dataset)?;
    let files = repo.write_package(&dataset, &body.output_dir).await?;
    Ok(Json(files))
}

#[derive(Debug, Deserialize)]
pub struct MergeBody {
    source: String,
}

/// `POST /datasets/{ds}/merge` — merge the named source dataset into `{ds}`.
pub async fn merge(
    State(repo): State<Arc<dyn CityLakeRepository>>,
    Path(destination): Path<String>,
    Json(body): Json<MergeBody>,
) -> Result<StatusCode, CityLakeError> {
    let destination = DatasetName::new(&destination)?;
    let source = DatasetName::new(&body.source)?;
    repo.merge(&destination, &source).await?;
    Ok(StatusCode::NO_CONTENT)
}
