use async_trait::async_trait;

use super::types::{
    CityJsonMetadata, CompactionStats, ExportFormat, LodKey, QueryParams,
};

/// Result type for repository operations
pub type RepositoryResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Repository trait defining all database operations for CityLake.
///
/// This is the core abstraction layer. All database access goes through this trait,
/// allowing the implementation to be swapped (e.g., for testing).
#[async_trait]
pub trait CityLakeRepository: Send + Sync {
    /// Create LOD-suffixed table(s) from a CityJSON source file.
    ///
    /// - `base_name` defaults to `"city_objects"` when `None`. Each created table is
    ///   named `{base}_lod_X_Y` (e.g. `buildings_lod_2_2`).
    /// - When `lod` is `Some(...)` only that LOD is loaded. When `None`, every LOD
    ///   present in the source is discovered and a table is created for each.
    /// - Source-level metadata is persisted into the shared `cityjson_metadata`
    ///   table.
    ///
    /// Returns the names of the data tables that were created (in creation order).
    async fn create_table(
        &self,
        base_name: Option<&str>,
        source_path: &str,
        lod: Option<&LodKey>,
    ) -> RepositoryResult<Vec<String>>;

    /// Insert CityJSON objects into existing per-LOD table(s).
    ///
    /// - When `lod` is `Some(...)`, only that LOD is read from the source and
    ///   inserted into `{base_name}_lod_X_Y`.
    /// - When `lod` is `None`, every LOD found in the source is inserted into its
    ///   matching `{base_name}_lod_X_Y` table. All target tables must already
    ///   exist.
    ///
    /// Returns the total number of rows inserted across all targeted tables.
    async fn insert_objects(
        &self,
        base_name: &str,
        source_path: &str,
        lod: Option<&LodKey>,
    ) -> RepositoryResult<usize>;

    /// Update a CityJSON object by its ID in a specific LOD-suffixed table.
    ///
    /// The `table_name` must end with a `_lod_X_Y` suffix; the LOD is recovered
    /// from the suffix and used when re-reading the new CityJSON data through the
    /// extension.
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

    /// Export a single LOD-suffixed table to a CityJSON format file.
    ///
    /// Multi-LOD round-trip export (rejoining LOD tables back into a unified
    /// CityJSON) is not supported. See `tasks.md` for the deferred follow-up.
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
