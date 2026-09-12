//! The trait implementation.
//!
//! Every method hands its work to a blocking thread: the DuckDB connection is
//! behind a std::sync::Mutex and its operations are CPU-bound, so running them
//! on the async executor would let one slow ingest stall every other request.
//! `handle()` shares the connection Arc rather than moving `self`, so the
//! caller keeps its own reference and the blocking task sees the same
//! catalog.

use async_trait::async_trait;

use crate::core::db::service::DuckLakeService;
use crate::core::interface::repository::CityLakeRepository;
use crate::core::interface::types::{
    CityLakeError, CompactionStats, DatasetInfo, DatasetName, ExportFormat, ModuleName,
    PackageFile, QueryParams, RepositoryResult, ValidationFinding,
};

/// A `JoinError` means the blocking closure panicked or was cancelled at
/// runtime shutdown — never a DuckDB failure the caller can act on, so it is
/// folded into `CityLakeError::Internal` rather than propagated as one of the
/// pragma error variants.
fn join_err(e: tokio::task::JoinError) -> CityLakeError {
    CityLakeError::Internal(format!("blocking task failed: {e}"))
}

#[async_trait]
impl CityLakeRepository for DuckLakeService {
    async fn create_dataset(
        &self,
        dataset: &DatasetName,
        source_path: &str,
    ) -> RepositoryResult<DatasetInfo> {
        let (service, dataset, source) = (self.handle(), dataset.clone(), source_path.to_string());
        tokio::task::spawn_blocking(move || service.create_dataset_impl(&dataset, &source))
            .await
            .map_err(join_err)?
    }

    async fn list_datasets(&self) -> RepositoryResult<Vec<String>> {
        let service = self.handle();
        tokio::task::spawn_blocking(move || service.list_datasets_impl())
            .await
            .map_err(join_err)?
    }

    async fn describe_dataset(&self, dataset: &DatasetName) -> RepositoryResult<DatasetInfo> {
        let (service, dataset) = (self.handle(), dataset.clone());
        tokio::task::spawn_blocking(move || service.describe_dataset_impl(&dataset))
            .await
            .map_err(join_err)?
    }

    async fn drop_dataset(&self, dataset: &DatasetName) -> RepositoryResult<()> {
        let (service, dataset) = (self.handle(), dataset.clone());
        tokio::task::spawn_blocking(move || service.drop_dataset_impl(&dataset))
            .await
            .map_err(join_err)?
    }

    async fn ingest(&self, dataset: &DatasetName, source_path: &str) -> RepositoryResult<usize> {
        let (service, dataset, source) = (self.handle(), dataset.clone(), source_path.to_string());
        tokio::task::spawn_blocking(move || service.ingest_impl(&dataset, &source))
            .await
            .map_err(join_err)?
    }

    async fn query_objects(
        &self,
        dataset: &DatasetName,
        module: &ModuleName,
        params: &QueryParams,
    ) -> RepositoryResult<Vec<serde_json::Value>> {
        let (service, dataset, module, params) = (
            self.handle(),
            dataset.clone(),
            module.clone(),
            params.clone(),
        );
        tokio::task::spawn_blocking(move || service.query_objects_impl(&dataset, &module, &params))
            .await
            .map_err(join_err)?
    }

    async fn update_object(
        &self,
        dataset: &DatasetName,
        id: &str,
        attributes: &serde_json::Map<String, serde_json::Value>,
    ) -> RepositoryResult<()> {
        let (service, dataset, id, attributes) = (
            self.handle(),
            dataset.clone(),
            id.to_string(),
            attributes.clone(),
        );
        tokio::task::spawn_blocking(move || service.update_object_impl(&dataset, &id, &attributes))
            .await
            .map_err(join_err)?
    }

    async fn delete_object(&self, dataset: &DatasetName, id: &str) -> RepositoryResult<usize> {
        let (service, dataset, id) = (self.handle(), dataset.clone(), id.to_string());
        tokio::task::spawn_blocking(move || service.delete_object_impl(&dataset, &id))
            .await
            .map_err(join_err)?
    }

    async fn delete_where(
        &self,
        dataset: &DatasetName,
        predicate: &str,
    ) -> RepositoryResult<usize> {
        let (service, dataset, predicate) = (self.handle(), dataset.clone(), predicate.to_string());
        tokio::task::spawn_blocking(move || service.delete_where_impl(&dataset, &predicate))
            .await
            .map_err(join_err)?
    }

    async fn reconcile(&self, dataset: &DatasetName) -> RepositoryResult<()> {
        let (service, dataset) = (self.handle(), dataset.clone());
        tokio::task::spawn_blocking(move || service.reconcile_impl(&dataset))
            .await
            .map_err(join_err)?
    }

    async fn validate(&self, dataset: &DatasetName) -> RepositoryResult<Vec<ValidationFinding>> {
        let (service, dataset) = (self.handle(), dataset.clone());
        tokio::task::spawn_blocking(move || service.validate_impl(&dataset))
            .await
            .map_err(join_err)?
    }

    async fn vacuum(&self, dataset: &DatasetName) -> RepositoryResult<usize> {
        let (service, dataset) = (self.handle(), dataset.clone());
        tokio::task::spawn_blocking(move || service.vacuum_impl(&dataset))
            .await
            .map_err(join_err)?
    }

    async fn merge(&self, destination: &DatasetName, source: &DatasetName) -> RepositoryResult<()> {
        let (service, destination, source) = (self.handle(), destination.clone(), source.clone());
        tokio::task::spawn_blocking(move || service.merge_impl(&destination, &source))
            .await
            .map_err(join_err)?
    }

    async fn write_package(
        &self,
        dataset: &DatasetName,
        output_dir: &str,
    ) -> RepositoryResult<Vec<PackageFile>> {
        let (service, dataset, output_dir) =
            (self.handle(), dataset.clone(), output_dir.to_string());
        tokio::task::spawn_blocking(move || service.write_package_impl(&dataset, &output_dir))
            .await
            .map_err(join_err)?
    }

    async fn export_module(
        &self,
        dataset: &DatasetName,
        module: &ModuleName,
        output_path: &str,
        format: ExportFormat,
    ) -> RepositoryResult<()> {
        let (service, dataset, module, output_path) = (
            self.handle(),
            dataset.clone(),
            module.clone(),
            output_path.to_string(),
        );
        tokio::task::spawn_blocking(move || {
            service.export_module_impl(&dataset, &module, &output_path, format)
        })
        .await
        .map_err(join_err)?
    }

    async fn compact(&self, dataset: &DatasetName) -> RepositoryResult<CompactionStats> {
        let (service, dataset) = (self.handle(), dataset.clone());
        tokio::task::spawn_blocking(move || service.compact_impl(&dataset))
            .await
            .map_err(join_err)?
    }
}
