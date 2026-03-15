use duckdb::Connection;
use std::sync::{Arc, Mutex};

use crate::core::interface::repository::RepositoryResult;
use crate::core::interface::types::CompactionStats;

/// Compact a DuckLake table by merging small Parquet files.
///
/// DuckLake stores data as Parquet files and may accumulate many small files
/// after frequent inserts. Compaction merges them for better query performance.
pub async fn compact_table(
    connection: &Arc<Mutex<Connection>>,
    table_name: &str,
) -> RepositoryResult<CompactionStats> {
    let conn = connection
        .lock()
        .map_err(|e| format!("Failed to lock connection: {e}"))?;

    // Get file count before compaction
    let files_before = get_file_count(&conn, table_name)?;
    let rows_count = get_row_count(&conn, table_name)?;

    // Compact by creating a new copy of the table and swapping
    // DuckLake supports ALTER TABLE ... COMPACT, or we can do CTAS + DROP + RENAME
    let compact_sql = format!(
        "CREATE OR REPLACE TABLE citylake.{table_name}_compact AS SELECT * FROM citylake.{table_name}"
    );
    conn.execute_batch(&compact_sql)
        .map_err(|e| format!("Failed to compact table '{table_name}': {e}"))?;

    let drop_sql = format!("DROP TABLE citylake.{table_name}");
    conn.execute_batch(&drop_sql)
        .map_err(|e| format!("Failed to drop old table during compaction: {e}"))?;

    let rename_sql = format!(
        "ALTER TABLE citylake.{table_name}_compact RENAME TO {table_name}"
    );
    conn.execute_batch(&rename_sql)
        .map_err(|e| format!("Failed to rename compacted table: {e}"))?;

    let files_after = get_file_count(&conn, table_name)?;

    let stats = CompactionStats {
        files_before,
        files_after,
        rows_compacted: rows_count,
    };

    tracing::info!(
        "Compacted table '{table_name}': {} files -> {} files ({} rows)",
        stats.files_before,
        stats.files_after,
        stats.rows_compacted,
    );

    Ok(stats)
}

fn get_row_count(conn: &Connection, table_name: &str) -> RepositoryResult<usize> {
    let sql = format!("SELECT COUNT(*) FROM citylake.{table_name}");
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| format!("Failed to count rows: {e}"))?;
    let count: i64 = stmt
        .query_row([], |row| row.get(0))
        .map_err(|e| format!("Failed to count rows: {e}"))?;
    Ok(count as usize)
}

fn get_file_count(conn: &Connection, _table_name: &str) -> RepositoryResult<usize> {
    // DuckLake tracks files internally; for now return a placeholder
    // TODO: Query DuckLake metadata for actual file count
    let _ = conn;
    Ok(1)
}

#[cfg(test)]
mod tests {
    use crate::core::interface::repository::CityLakeRepository;
    use crate::tests::helpers;

    #[tokio::test]
    async fn test_compact_table() {
        let service = helpers::setup_with_table("compact_test");
        let stats = service.compact_table("compact_test").await.unwrap();
        assert_eq!(stats.rows_compacted, 3, "Expected 3 rows compacted from test data");
        assert!(stats.files_after >= 1);
    }

    #[tokio::test]
    async fn test_compact_preserves_data() {
        let service = helpers::setup_with_table("compact_preserve");
        use crate::core::interface::types::QueryParams;

        let params = QueryParams { filter: None, limit: None, offset: None };
        let before = service.query_objects("compact_preserve", &params).await.unwrap();

        service.compact_table("compact_preserve").await.unwrap();

        let after = service.query_objects("compact_preserve", &params).await.unwrap();
        assert_eq!(before.len(), after.len(), "Compaction should preserve all rows");
    }
}
