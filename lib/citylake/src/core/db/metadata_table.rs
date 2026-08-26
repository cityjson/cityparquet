//! Shared `cityjson_metadata` table that accumulates one row per ingested dataset.
//!
//! The schema is determined by the cityjson extension's `*_metadata()` table
//! function plus two leading bookkeeping columns: `dataset` and `source_path`.
//! On the first ingest the table is created via `CREATE TABLE … AS SELECT`; on
//! subsequent ingests we `INSERT INTO` the same table.

use duckdb::Connection;
use std::sync::{Arc, Mutex};

use crate::core::interface::repository::RepositoryResult;
use crate::core::interface::types::{InputFormat, METADATA_TABLE};

/// Persist the source file's CityJSON metadata into the shared metadata table.
///
/// The table is created on first call (CTAS over `{format}_metadata(path)` plus
/// `dataset` and `source_path` literal columns). Returns `Ok(false)` when the
/// format does not expose a metadata function (FlatCityBuf without the FCB
/// build feature) — caller decides whether that is fatal.
pub fn persist_metadata(
    connection: &Arc<Mutex<Connection>>,
    dataset: &str,
    source_path: &str,
    format: InputFormat,
) -> RepositoryResult<bool> {
    let Some(metadata_fn) = format.metadata_function() else {
        return Ok(false);
    };

    let conn = connection
        .lock()
        .map_err(|e| format!("Failed to lock connection: {e}"))?;

    let exists = metadata_table_exists(&conn)?;

    let dataset_lit = dataset.replace('\'', "''");
    let source_lit = source_path.replace('\'', "''");

    let sql = if exists {
        format!(
            "INSERT INTO citylake.{METADATA_TABLE} \
             SELECT '{dataset_lit}' AS dataset, '{source_lit}' AS source_path, m.* \
             FROM {metadata_fn}('{source_lit}') m"
        )
    } else {
        format!(
            "CREATE TABLE citylake.{METADATA_TABLE} AS \
             SELECT '{dataset_lit}' AS dataset, '{source_lit}' AS source_path, m.* \
             FROM {metadata_fn}('{source_lit}') m"
        )
    };

    conn.execute_batch(&sql)
        .map_err(|e| format!("Failed to persist metadata for '{dataset}': {e}"))?;

    Ok(true)
}

fn metadata_table_exists(conn: &Connection) -> RepositoryResult<bool> {
    let mut stmt = conn
        .prepare(
            "SELECT COUNT(*) FROM information_schema.tables \
             WHERE table_catalog = 'citylake' AND table_name = ?",
        )
        .map_err(|e| format!("Failed to prepare metadata-table check: {e}"))?;
    let count: i64 = stmt
        .query_row([METADATA_TABLE], |row| row.get(0))
        .map_err(|e| format!("Failed to check metadata table: {e}"))?;
    Ok(count > 0)
}
