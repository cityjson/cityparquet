use async_trait::async_trait;
use duckdb::Connection;
use std::sync::{Arc, Mutex};

use crate::core::interface::repository::{CityLakeRepository, RepositoryResult};
use crate::core::interface::types::{
    CityJsonMetadata, CityLakeConfig, CompactionStats, ExportFormat, LodKey, QueryParams,
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

    /// Create a DuckLakeService for testing without requiring external extensions.
    ///
    /// Uses a plain in-memory DuckDB with a `citylake` schema instead of
    /// the ducklake extension. Does not load cityjson or ducklake extensions.
    #[cfg(test)]
    pub fn new_for_testing() -> RepositoryResult<Self> {
        let conn = Connection::open_in_memory()
            .map_err(|e| format!("Failed to open DuckDB connection: {e}"))?;

        // Disable auto-install/load of extensions to prevent network timeouts in tests.
        // Then explicitly load the bundled json extension (needed for to_json()).
        conn.execute_batch(
            "SET autoinstall_known_extensions=false; SET autoload_known_extensions=false; LOAD json;",
        )
        .map_err(|e| format!("Failed to configure extensions: {e}"))?;

        // Attach a second in-memory database as "citylake" to mimic the
        // DuckLake catalog attachment used in production.
        conn.execute_batch("ATTACH ':memory:' AS citylake;")
            .map_err(|e| format!("Failed to attach citylake catalog: {e}"))?;

        let config = CityLakeConfig {
            storage_path: String::new(),
            catalog_path: String::new(),
            auto_compact: false,
            ..Default::default()
        };

        Ok(Self {
            connection: Arc::new(Mutex::new(conn)),
            config,
        })
    }

    /// Get a reference to the underlying connection (for test setup).
    #[cfg(test)]
    pub fn connection(&self) -> &Arc<Mutex<Connection>> {
        &self.connection
    }
}

#[async_trait]
impl CityLakeRepository for DuckLakeService {
    async fn create_table(
        &self,
        base_name: Option<&str>,
        source_path: &str,
        lod: Option<&LodKey>,
    ) -> RepositoryResult<Vec<String>> {
        super::table::create_table(&self.connection, base_name, source_path, lod).await
    }

    async fn insert_objects(
        &self,
        base_name: &str,
        file_path: &str,
        lod: Option<&LodKey>,
    ) -> RepositoryResult<usize> {
        super::insert::insert_objects(&self.connection, base_name, file_path, lod, &self.config)
            .await
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

#[cfg(test)]
mod tests {
    use crate::tests::helpers;

    #[test]
    fn test_new_service_success() {
        let service = helpers::setup();
        // Service created with empty storage_path in test mode
        assert_eq!(service.config().storage_path, "");
    }

    #[test]
    fn test_new_for_testing_creates_schema() {
        let service = helpers::setup();
        let conn = service.connection().lock().unwrap();
        // Verify citylake schema exists by creating a table in it
        conn.execute_batch("CREATE TABLE citylake.test_schema_check (id INTEGER);")
            .expect("citylake schema should exist");
    }

    #[test]
    fn test_config_accessor() {
        let service = helpers::setup();
        let config = service.config();
        assert!(!config.auto_compact);
        assert_eq!(config.host, "127.0.0.1");
    }
}
