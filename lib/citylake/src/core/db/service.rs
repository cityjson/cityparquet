//! The DuckDB connection and the rules for using it.
//!
//! DuckDB's Connection is not Send, so it lives behind Arc<Mutex<_>>. Every
//! operation borrows it through [`DuckLakeService::with_connection`] or
//! [`DuckLakeService::scoped`], which is what keeps the search-path discipline
//! in one place instead of spread across nine operation modules.

use duckdb::{Config, Connection};
use std::sync::{Arc, Mutex};

use crate::core::db::sql;
use crate::core::interface::types::{CityLakeConfig, CityLakeError, RepositoryResult};

pub struct DuckLakeService {
    connection: Arc<Mutex<Connection>>,
    config: CityLakeConfig,
}

impl DuckLakeService {
    pub fn new(config: CityLakeConfig) -> RepositoryResult<Self> {
        std::fs::create_dir_all(&config.storage_path)?;

        // A locally built extension is loaded by path; otherwise the community
        // build. The extension is a hard dependency — nothing in this crate
        // works without it, so a failure here is fatal rather than deferred.
        //
        // `allow_unsigned_extensions` is a startup-only option: it goes on the
        // Config the connection is opened with, because `SET` on a running
        // database is refused.
        let conn = match std::env::var("CITYLAKE_CITYJSON_EXTENSION") {
            Ok(path) => {
                let config = Config::default().allow_unsigned_extensions()?;
                let conn = Connection::open_in_memory_with_flags(config)?;
                conn.execute_batch(&format!("LOAD {};", sql::literal(&path)))?;
                conn
            }
            Err(_) => {
                let conn = Connection::open_in_memory()?;
                conn.execute_batch("INSTALL cityjson FROM community; LOAD cityjson;")?;
                conn
            }
        };
        conn.execute_batch("INSTALL ducklake; LOAD ducklake;")?;
        // `json` backs to_json() on the query path and json_object() when the
        // CRS footer is minted.
        conn.execute_batch("INSTALL json; LOAD json;")?;

        conn.execute_batch(&format!(
            "ATTACH {} AS {} (DATA_PATH {})",
            sql::literal(&format!("ducklake:{}", config.catalog_path)),
            sql::ident(&config.catalog_name),
            sql::literal(&config.storage_path),
        ))?;

        tracing::info!(
            catalog = %config.catalog_path,
            storage = %config.storage_path,
            "CityLake ready"
        );

        Ok(Self {
            connection: Arc::new(Mutex::new(conn)),
            config,
        })
    }

    pub fn config(&self) -> &CityLakeConfig {
        &self.config
    }

    pub fn catalog(&self) -> &str {
        &self.config.catalog_name
    }

    pub fn with_connection<T>(
        &self,
        f: impl FnOnce(&Connection) -> RepositoryResult<T>,
    ) -> RepositoryResult<T> {
        let guard = self
            .connection
            .lock()
            .map_err(|e| CityLakeError::Internal(format!("connection mutex poisoned: {e}")))?;
        f(&guard)
    }

    /// Run `f` with the search path set to `path`, on a connection the caller
    /// already holds — which is what lets it nest inside a transaction, where
    /// [`scoped`] cannot go because that takes a connection of its own.
    ///
    /// The path is reset whether `f` succeeds or fails: leaving it set would
    /// silently resolve the next operation against this dataset. A reset
    /// failure never masks the body's error.
    pub fn with_search_path<T>(
        &self,
        conn: &Connection,
        path: &str,
        f: impl FnOnce(&Connection) -> RepositoryResult<T>,
    ) -> RepositoryResult<T> {
        conn.execute_batch(&sql::set_search_path(path))?;
        let result = f(conn);
        let reset = conn.execute_batch("RESET search_path");
        match (result, reset) {
            (Ok(value), Ok(())) => Ok(value),
            (Ok(_), Err(e)) => Err(e.into()),
            (Err(e), Ok(())) => Err(e),
            (Err(e), Err(reset_err)) => {
                // The reset failure never masks the body's error, but it must
                // not vanish silently either — a dying connection is harder
                // to diagnose without a trace of both failures.
                tracing::error!(%reset_err, "search_path reset failed after {e}");
                Err(e)
            }
        }
    }

    /// Point the search path at one dataset, so the package pragmas resolve
    /// their bare schema argument inside the attached catalog.
    pub fn scoped<T>(
        &self,
        dataset: &str,
        f: impl FnOnce(&Connection) -> RepositoryResult<T>,
    ) -> RepositoryResult<T> {
        let path = format!("{}.{dataset}", self.catalog());
        self.with_connection(|conn| self.with_search_path(conn, &path, f))
    }

    /// Run `f` inside a transaction, committing on success and rolling back on
    /// failure. Pragma effects roll back with everything else — a delete's
    /// cascade, survivor cleanup and re-derivation are one unit.
    pub fn in_transaction<T>(
        &self,
        conn: &Connection,
        f: impl FnOnce(&Connection) -> RepositoryResult<T>,
    ) -> RepositoryResult<T> {
        conn.execute_batch("BEGIN")?;
        match f(conn) {
            Ok(value) => {
                conn.execute_batch("COMMIT")?;
                Ok(value)
            }
            Err(e) => {
                // A rollback failure must not hide why we are rolling back.
                if let Err(rollback) = conn.execute_batch("ROLLBACK") {
                    tracing::error!(%rollback, "rollback failed after {e}");
                }
                Err(e)
            }
        }
    }

    pub fn schema_exists(&self, conn: &Connection, dataset: &str) -> RepositoryResult<bool> {
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM information_schema.schemata
             WHERE catalog_name = ? AND schema_name = ?",
            [self.catalog(), dataset],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// A handle sharing this service's connection, for moving into a blocking
    /// task. The connection is already behind `Arc<Mutex<_>>`; this shares it
    /// rather than opening a second one, so DuckLake sees one writer.
    pub(crate) fn handle(&self) -> Self {
        Self {
            connection: Arc::clone(&self.connection),
            config: self.config.clone(),
        }
    }
}
