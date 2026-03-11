use async_trait::async_trait;
use duckdb::Connection;
use std::sync::{Arc, Mutex};

use crate::core::interface::repository::{CityLakeRepository, RepositoryResult};
use crate::core::interface::types::{
    CityJsonMetadata, CityLakeConfig, CompactionStats, ExportFormat, QueryParams,
};

/// DuckLake-backed implementation of [CityLakeRepository].
///
/// Wraps a DuckDB connection with the cityjson and ducklake extensions loaded.
/// DuckDB's Connection is not Send, so it is wrapped in Arc<Mutex<>>.
pub struct DuckLakeService {
    connection: Arc<Mutex<Connection>>,
    config: CityLakeConfig,
}

impl DuckLakeService {
    /// Create a new DuckLakeService, initializing DuckDB with required extensions.
    pub fn new(config: CityLakeConfig) -> RepositoryResult<Self> {
        // Create storage directory if it doesn't exist
        std::fs::create_dir_all(&config.storage_path)
            .map_err(|e| format!("Failed to create storage directory: {e}"))?;

        let conn = Connection::open_in_memory()
            .map_err(|e| format!("Failed to open DuckDB connection: {e}"))?;

        // Install and load the cityjson extension
        conn.execute_batch(
            "INSTALL cityjson FROM community; LOAD cityjson;",
        )
        .map_err(|e| format!("Failed to load cityjson extension: {e}"))?;

        // Install and load the ducklake extension
        conn.execute_batch("INSTALL ducklake; LOAD ducklake;")
            .map_err(|e| format!("Failed to load ducklake extension: {e}"))?;

        // Attach the DuckLake catalog
        let attach_sql = format!(
            "ATTACH 'ducklake:{}' AS citylake (DATA_PATH '{}')",
            config.catalog_path, config.storage_path,
        );
        conn.execute_batch(&attach_sql)
            .map_err(|e| format!("Failed to attach DuckLake catalog: {e}"))?;

        tracing::info!(
            "DuckLakeService initialized (catalog={}, storage={})",
            config.catalog_path,
            config.storage_path
        );

        Ok(Self {
            connection: Arc::new(Mutex::new(conn)),
            config,
        })
    }

    /// Get a reference to the config
    pub fn config(&self) -> &CityLakeConfig {
        &self.config
    }
}

#[async_trait]
impl CityLakeRepository for DuckLakeService {
    async fn create_table(&self, table_name: &str, source_path: &str) -> RepositoryResult<()> {
        super::table::create_table(&self.connection, table_name, source_path).await
    }

    async fn insert_objects(&self, table_name: &str, file_path: &str) -> RepositoryResult<usize> {
        let count =
            super::insert::insert_objects(&self.connection, table_name, file_path, &self.config)
                .await?;
        Ok(count)
    }

    async fn update_object(
        &self,
        table_name: &str,
        id: &str,
        cityjson_data: &str,
    ) -> RepositoryResult<()> {
        super::update::update_object(&self.connection, table_name, id, cityjson_data).await
    }

    async fn delete_object(&self, table_name: &str, id: &str) -> RepositoryResult<()> {
        super::delete::delete_object(&self.connection, table_name, id).await
    }

    async fn table_exists(&self, table_name: &str) -> RepositoryResult<bool> {
        super::table::table_exists(&self.connection, table_name).await
    }

    async fn compact_table(&self, table_name: &str) -> RepositoryResult<CompactionStats> {
        super::compaction::compact_table(&self.connection, table_name).await
    }

    async fn get_metadata(&self, file_path: &str) -> RepositoryResult<CityJsonMetadata> {
        super::metadata::get_metadata(&self.connection, file_path).await
    }

    async fn export_table(
        &self,
        table_name: &str,
        output_path: &str,
        format: ExportFormat,
    ) -> RepositoryResult<()> {
        super::export::export_table(&self.connection, table_name, output_path, format).await
    }

    async fn query_objects(
        &self,
        table_name: &str,
        params: &QueryParams,
    ) -> RepositoryResult<Vec<serde_json::Value>> {
        super::query::query_objects(&self.connection, table_name, params).await
    }
}
