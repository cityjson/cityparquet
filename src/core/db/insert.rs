use duckdb::Connection;
use std::sync::{Arc, Mutex};

use crate::core::interface::repository::RepositoryResult;
use crate::core::interface::types::{CityLakeConfig, InputFormat};

/// Insert CityJSON objects from a file into an existing table.
///
/// The cityjson extension handles all parsing and schema mapping.
/// Returns the number of rows inserted.
pub async fn insert_objects(
    connection: &Arc<Mutex<Connection>>,
    table_name: &str,
    file_path: &str,
    config: &CityLakeConfig,
) -> RepositoryResult<usize> {
    let format = InputFormat::from_path(file_path)
        .ok_or_else(|| format!("Cannot detect CityJSON format from path: {file_path}"))?;

    let conn = connection
        .lock()
        .map_err(|e| format!("Failed to lock connection: {e}"))?;

    // Count rows before insert
    let count_sql = format!("SELECT COUNT(*) FROM citylake.{table_name}");
    let before: i64 = conn
        .prepare(&count_sql)
        .and_then(|mut stmt| stmt.query_row([], |row| row.get(0)))
        .map_err(|e| format!("Failed to count rows: {e}"))?;

    // Insert using the cityjson extension's read function
    let insert_sql = format!(
        "INSERT INTO citylake.{table_name} SELECT * FROM {read_fn}('{file_path}')",
        read_fn = format.read_function(),
    );

    conn.execute_batch(&insert_sql)
        .map_err(|e| format!("Failed to insert objects into '{table_name}': {e}"))?;

    // Count rows after insert
    let after: i64 = conn
        .prepare(&count_sql)
        .and_then(|mut stmt| stmt.query_row([], |row| row.get(0)))
        .map_err(|e| format!("Failed to count rows after insert: {e}"))?;

    let inserted = (after - before) as usize;
    tracing::info!("Inserted {inserted} objects into '{table_name}' from '{file_path}'");

    // Check if auto-compaction should be triggered
    if config.auto_compact {
        tracing::debug!("Auto-compaction check for '{table_name}' (not yet implemented)");
    }

    Ok(inserted)
}

#[cfg(test)]
mod tests {
    use crate::core::interface::repository::CityLakeRepository;
    use crate::tests::helpers;

    #[tokio::test]
    async fn test_insert_invalid_format() {
        let service = helpers::setup_with_table("insert_fmt");
        let result = service
            .insert_objects("insert_fmt", "/tmp/bad_file.csv")
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Cannot detect"));
    }

    #[test]
    fn test_input_format_detection() {
        use crate::core::interface::types::InputFormat;
        assert!(InputFormat::from_path("data.city.jsonl").is_some());
        assert!(InputFormat::from_path("data.csv").is_none());
    }
}
