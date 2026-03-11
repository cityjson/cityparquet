use async_trait::async_trait;

use super::types::{
    CityJsonMetadata, CompactionStats, ExportFormat, QueryParams,
};

/// Result type for repository operations
pub type RepositoryResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Repository trait defining all database operations for CityLake.
///
/// This is the core abstraction layer. All database access goes through this trait,
/// allowing the implementation to be swapped (e.g., for testing).
#[async_trait]
pub trait CityLakeRepository: Send + Sync {
    /// Create a new table from a CityJSON source file.
    ///
    /// The cityjson extension auto-infers the schema from the source file.
    async fn create_table(&self, table_name: &str, source_path: &str) -> RepositoryResult<()>;

    /// Insert CityJSON objects from a file into an existing table.
    ///
    /// The file format is detected from the extension (.city.json, .city.jsonl, .fcb).
    /// Returns the number of objects inserted.
    async fn insert_objects(&self, table_name: &str, file_path: &str) -> RepositoryResult<usize>;

    /// Update a CityJSON object by its ID.
    ///
    /// The cityjson_data should be a valid CityJSON object as a JSON string.
    async fn update_object(
        &self,
        table_name: &str,
        id: &str,
        cityjson_data: &str,
    ) -> RepositoryResult<()>;

    /// Delete a CityJSON object by its ID.
    async fn delete_object(&self, table_name: &str, id: &str) -> RepositoryResult<()>;

    /// Check if a table exists in the DuckLake catalog.
    async fn table_exists(&self, table_name: &str) -> RepositoryResult<bool>;

    /// Compact a table to optimize storage (merge small Parquet files).
    async fn compact_table(&self, table_name: &str) -> RepositoryResult<CompactionStats>;

    /// Get metadata from a CityJSON source file.
    async fn get_metadata(&self, file_path: &str) -> RepositoryResult<CityJsonMetadata>;

    /// Export a table to a CityJSON format file.
    async fn export_table(
        &self,
        table_name: &str,
        output_path: &str,
        format: ExportFormat,
    ) -> RepositoryResult<()>;

    /// Query objects from a table with optional filters and pagination.
    async fn query_objects(
        &self,
        table_name: &str,
        params: &QueryParams,
    ) -> RepositoryResult<Vec<serde_json::Value>>;
}
