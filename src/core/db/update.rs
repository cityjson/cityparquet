use duckdb::Connection;
use std::io::Write;
use std::sync::{Arc, Mutex};
use tempfile::NamedTempFile;

use crate::core::interface::repository::RepositoryResult;

/// Update a CityJSON object by its ID.
///
/// Writes the new CityJSON data to a temp file, reads it via the cityjson extension,
/// and uses UPDATE ... SET to replace the matching row.
pub async fn update_object(
    connection: &Arc<Mutex<Connection>>,
    table_name: &str,
    id: &str,
    cityjson_data: &str,
) -> RepositoryResult<()> {
    let conn = connection
        .lock()
        .map_err(|e| format!("Failed to lock connection: {e}"))?;

    // Verify the record exists
    let exists_sql = format!("SELECT COUNT(*) FROM citylake.{table_name} WHERE id = ?");
    let count: i64 = conn
        .prepare(&exists_sql)
        .and_then(|mut stmt| stmt.query_row([id], |row| row.get(0)))
        .map_err(|e| format!("Failed to check if record exists: {e}"))?;

    if count == 0 {
        return Err(format!("No record found with id '{id}' in table '{table_name}'").into());
    }

    // Write the CityJSON data to a temp file as CityJSONSeq
    let mut temp_file =
        NamedTempFile::new().map_err(|e| format!("Failed to create temp file: {e}"))?;
    writeln!(temp_file, "{cityjson_data}")
        .map_err(|e| format!("Failed to write to temp file: {e}"))?;
    temp_file
        .flush()
        .map_err(|e| format!("Failed to flush temp file: {e}"))?;

    let temp_path = temp_file.path().display().to_string();

    // Delete old row and insert new one from the temp file
    let delete_sql = format!("DELETE FROM citylake.{table_name} WHERE id = ?");
    conn.execute(&delete_sql, [id])
        .map_err(|e| format!("Failed to delete old record: {e}"))?;

    let insert_sql = format!(
        "INSERT INTO citylake.{table_name} SELECT * FROM read_cityjsonseq('{temp_path}')"
    );
    conn.execute_batch(&insert_sql)
        .map_err(|e| format!("Failed to insert updated record: {e}"))?;

    tracing::info!("Updated object '{id}' in table '{table_name}'");
    Ok(())
}
