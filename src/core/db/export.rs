use duckdb::Connection;
use std::sync::{Arc, Mutex};

use crate::core::interface::repository::RepositoryResult;
use crate::core::interface::types::ExportFormat;

/// Export a table to a CityJSON format file using the cityjson extension's COPY TO.
pub async fn export_table(
    connection: &Arc<Mutex<Connection>>,
    table_name: &str,
    output_path: &str,
    format: ExportFormat,
) -> RepositoryResult<()> {
    let conn = connection
        .lock()
        .map_err(|e| format!("Failed to lock connection: {e}"))?;

    let sql = format!(
        "COPY (SELECT * FROM citylake.{table_name}) TO '{output_path}' (FORMAT {fmt})",
        fmt = format.as_duckdb_format(),
    );

    conn.execute_batch(&sql)
        .map_err(|e| format!("Failed to export table '{table_name}' to '{output_path}': {e}"))?;

    tracing::info!(
        "Exported table '{table_name}' to '{output_path}' (format: {format})"
    );
    Ok(())
}
