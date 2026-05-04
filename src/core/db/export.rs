use duckdb::Connection;
use std::sync::{Arc, Mutex};

use crate::core::interface::repository::RepositoryResult;
use crate::core::interface::types::ExportFormat;

use super::table::validate_identifier;

/// Export a table to a CityJSON format file using the cityjson extension's COPY TO.
pub async fn export_table(
    connection: &Arc<Mutex<Connection>>,
    table_name: &str,
    output_path: &str,
    format: ExportFormat,
) -> RepositoryResult<()> {
    validate_identifier(table_name, "Table name")?;

    let conn = connection
        .lock()
        .map_err(|e| format!("Failed to lock connection: {e}"))?;

    let path_lit = output_path.replace('\'', "''");
    let sql = format!(
        "COPY (SELECT * FROM citylake.{table_name}) TO '{path_lit}' (FORMAT {fmt})",
        fmt = format.as_duckdb_format(),
    );

    conn.execute_batch(&sql)
        .map_err(|e| format!("Failed to export table '{table_name}' to '{output_path}': {e}"))?;

    tracing::info!(
        "Exported table '{table_name}' to '{output_path}' (format: {format})"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::core::interface::repository::CityLakeRepository;
    use crate::core::interface::types::ExportFormat;
    use crate::tests::helpers;

    #[tokio::test]
    async fn test_export_nonexistent_table() {
        let service = helpers::setup();
        let temp_dir = tempfile::tempdir().unwrap();
        let output_path = temp_dir.path().join("export.city.jsonl");
        let output_str = output_path.to_string_lossy().to_string();

        let result = service
            .export_table("nonexistent", &output_str, ExportFormat::CityJsonSeq)
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_export_csv() {
        let service = helpers::setup_with_table("export_csv");
        let temp_dir = tempfile::tempdir().unwrap();
        let output_path = temp_dir.path().join("export.csv");
        let output_str = output_path.to_string_lossy().to_string();

        let conn = service.connection().lock().unwrap();
        conn.execute_batch(&format!(
            "COPY (SELECT * FROM citylake.export_csv) TO '{output_str}' (FORMAT csv, HEADER true)"
        ))
        .unwrap();

        assert!(output_path.exists());
        let contents = std::fs::read_to_string(&output_path).unwrap();
        assert!(contents.contains("building_001"));
    }
}
