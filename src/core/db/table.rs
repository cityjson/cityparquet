use duckdb::Connection;
use std::sync::{Arc, Mutex};

use crate::core::interface::repository::RepositoryResult;
use crate::core::interface::types::InputFormat;

/// Validate that a table name contains only safe characters.
fn validate_table_name(name: &str) -> RepositoryResult<()> {
    if name.is_empty() {
        return Err("Table name cannot be empty".into());
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(format!(
            "Table name '{}' contains invalid characters. Only alphanumeric and underscore allowed.",
            name
        )
        .into());
    }
    Ok(())
}

/// Create a new table from a CityJSON source file.
///
/// Uses the cityjson extension's read functions to auto-infer the schema.
pub async fn create_table(
    connection: &Arc<Mutex<Connection>>,
    table_name: &str,
    source_path: &str,
) -> RepositoryResult<()> {
    validate_table_name(table_name)?;

    let format = InputFormat::from_path(source_path)
        .ok_or_else(|| format!("Cannot detect CityJSON format from path: {source_path}"))?;

    let conn = connection
        .lock()
        .map_err(|e| format!("Failed to lock connection: {e}"))?;

    let sql = format!(
        "CREATE TABLE citylake.{table_name} AS SELECT * FROM {read_fn}('{source_path}')",
        read_fn = format.read_function(),
    );

    conn.execute_batch(&sql)
        .map_err(|e| format!("Failed to create table '{table_name}': {e}"))?;

    tracing::info!("Created table '{table_name}' from '{source_path}'");
    Ok(())
}

/// Check if a table exists in the DuckLake catalog.
pub async fn table_exists(
    connection: &Arc<Mutex<Connection>>,
    table_name: &str,
) -> RepositoryResult<bool> {
    validate_table_name(table_name)?;

    let conn = connection
        .lock()
        .map_err(|e| format!("Failed to lock connection: {e}"))?;

    let mut stmt = conn
        .prepare(
            "SELECT COUNT(*) FROM information_schema.tables
             WHERE table_catalog = 'citylake' AND table_name = ?",
        )
        .map_err(|e| format!("Failed to prepare table exists query: {e}"))?;

    let count: i64 = stmt
        .query_row([table_name], |row| row.get(0))
        .map_err(|e| format!("Failed to check table existence: {e}"))?;

    Ok(count > 0)
}
