//! The package-shaped operations the extension makes cheap: exporting one
//! module to a CityJSON-family file, writing a whole dataset out as a
//! CityParquet package, merging one dataset into another.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;

use crate::app::output_path::resolve_output_path;
use crate::app::server::AppState;
use crate::core::interface::repository::CityLakeRepository;
use crate::core::interface::types::{
    CityLakeError, DatasetName, ExportFormat, ModuleName, PackageFile,
};

#[derive(Debug, Deserialize)]
pub struct ExportBody {
    module: String,
    /// A destination the server writes, replacing an existing file. Resolved
    /// against the configured output root (`CITYLAKE_OUTPUT_ROOT`) before
    /// the dataset is even looked up; see the module-level "What this API
    /// trusts" comment.
    output_path: String,
    format: ExportFormat,
}

/// `POST /datasets/{ds}/export` — export one module to a single
/// CityJSON-family file.
pub async fn export(
    State(state): State<AppState>,
    Path(dataset): Path<String>,
    Json(body): Json<ExportBody>,
) -> Result<StatusCode, CityLakeError> {
    let resolved = resolve_output_path(state.output_root.as_deref(), &body.output_path)
        .map_err(|e| CityLakeError::BadRequest(e.to_string()))?;

    // `resolve_output_path` treats a requested path of "" or "." as the root
    // itself — coherent for `write_package`, which is about to create a
    // directory there, but not for `export`, which writes a single file: the
    // extension would fail deep inside with a confusing error rather than a
    // clear refusal here.
    let root = state
        .output_root
        .as_deref()
        .expect("resolve_output_path only succeeds with a configured root");
    // A fresh `canonicalize` rather than reusing `resolve_output_path`'s: if
    // the root vanished in the interval between the two calls, that is a
    // request-time failure to report, not a logic invariant to panic on.
    let canonical_root = std::fs::canonicalize(root).map_err(|_| {
        CityLakeError::BadRequest(format!("configured output root {root:?} does not exist"))
    })?;
    if resolved == canonical_root {
        return Err(CityLakeError::BadRequest(
            "export needs a file path, not the output root".to_string(),
        ));
    }

    let dataset = DatasetName::new(&dataset)?;
    let module = ModuleName::new(&body.module)?;
    state
        .repo
        .export_module(&dataset, &module, &resolved.to_string_lossy(), body.format)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
pub struct WritePackageBody {
    /// A destination directory the server writes into. Resolved against the
    /// configured output root (`CITYLAKE_OUTPUT_ROOT`) before the dataset is
    /// even looked up; see the module-level "What this API trusts" comment.
    output_dir: String,
}

/// `POST /datasets/{ds}/package` — write the dataset out as a CityParquet
/// package directory.
pub async fn write_package(
    State(state): State<AppState>,
    Path(dataset): Path<String>,
    Json(body): Json<WritePackageBody>,
) -> Result<Json<Vec<PackageFile>>, CityLakeError> {
    let resolved = resolve_output_path(state.output_root.as_deref(), &body.output_dir)
        .map_err(|e| CityLakeError::BadRequest(e.to_string()))?;
    let dataset = DatasetName::new(&dataset)?;
    let files = state
        .repo
        .write_package(&dataset, &resolved.to_string_lossy())
        .await?;
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
