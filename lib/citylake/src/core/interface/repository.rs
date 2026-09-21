//! The repository trait: every database operation CityLake exposes, keyed on
//! validated dataset and module names. Handlers hold `Arc<dyn
//! CityLakeRepository>`, so the trait must stay object-safe.

use async_trait::async_trait;

use super::types::{
    CompactionStats, DatasetInfo, DatasetName, ExportFormat, ModuleName, PackageFile, QueryParams,
    RepositoryResult, ValidationFinding,
};

#[async_trait]
pub trait CityLakeRepository: Send + Sync {
    /// Create a dataset from a source. A CityJSON / CityJSONSeq / FlatCityBuf
    /// file is bootstrapped and ingested; a CityParquet package directory is
    /// loaded through `cityparquet_read`, footers and all.
    async fn create_dataset(
        &self,
        dataset: &DatasetName,
        source_path: &str,
    ) -> RepositoryResult<DatasetInfo>;

    async fn list_datasets(&self) -> RepositoryResult<Vec<String>>;

    async fn describe_dataset(&self, dataset: &DatasetName) -> RepositoryResult<DatasetInfo>;

    async fn drop_dataset(&self, dataset: &DatasetName) -> RepositoryResult<()>;

    /// Ingest a further source into an existing dataset. Routing, sidecar
    /// renumbering and re-derivation are the extension's.
    async fn ingest(&self, dataset: &DatasetName, source_path: &str) -> RepositoryResult<usize>;

    async fn query_objects(
        &self,
        dataset: &DatasetName,
        module: &ModuleName,
        params: &QueryParams,
    ) -> RepositoryResult<Vec<serde_json::Value>>;

    /// Update attributes of one object, then re-derive what the edit
    /// invalidated. `attributes` is a JSON object of column name to value.
    async fn update_object(
        &self,
        dataset: &DatasetName,
        id: &str,
        attributes: &serde_json::Map<String, serde_json::Value>,
    ) -> RepositoryResult<()>;

    /// Delete by id, cascading transitively through `children`.
    async fn delete_object(&self, dataset: &DatasetName, id: &str) -> RepositoryResult<usize>;

    /// Delete by predicate, cascading transitively through `children`.
    async fn delete_where(&self, dataset: &DatasetName, predicate: &str)
        -> RepositoryResult<usize>;

    async fn reconcile(&self, dataset: &DatasetName) -> RepositoryResult<()>;

    async fn validate(&self, dataset: &DatasetName) -> RepositoryResult<Vec<ValidationFinding>>;

    async fn vacuum(&self, dataset: &DatasetName) -> RepositoryResult<usize>;

    async fn merge(&self, destination: &DatasetName, source: &DatasetName) -> RepositoryResult<()>;

    /// Write the dataset out as a CityParquet package directory.
    async fn write_package(
        &self,
        dataset: &DatasetName,
        output_dir: &str,
    ) -> RepositoryResult<Vec<PackageFile>>;

    /// Export one module to a single CityJSON-family file.
    async fn export_module(
        &self,
        dataset: &DatasetName,
        module: &ModuleName,
        output_path: &str,
        format: ExportFormat,
    ) -> RepositoryResult<()>;

    async fn compact(&self, dataset: &DatasetName) -> RepositoryResult<CompactionStats>;
}
