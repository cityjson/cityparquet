use duckdb::Connection;
use std::io::Write;
use std::sync::{Arc, Mutex};
use tempfile::NamedTempFile;

use crate::core::interface::repository::RepositoryResult;

use super::lod::lod_from_table_name;

/// Update a CityJSON object by its ID in a LOD-suffixed table.
///
/// Implementation: DELETE the existing row, then INSERT a fresh row produced by
/// the cityjson extension reading the new CityJSON snippet at the matching LOD.
///
/// We round-trip the snippet through a temp `.jsonl` file because the extension's
/// `read_cityjsonseq()` requires a path — it cannot ingest a literal SQL string.
/// This is the *only* way to map a user-supplied CityJSONFeature into the
/// extension's per-LOD column shape without re-implementing schema mapping in
/// Rust (which CLAUDE.md forbids). The table name's `_lod_X_Y` suffix is parsed
/// to recover the target LOD so the snippet is read with `lod => 'X.Y'`.
pub async fn update_object(
    connection: &Arc<Mutex<Connection>>,
    table_name: &str,
    id: &str,
    cityjson_data: &str,
) -> RepositoryResult<()> {
    let lod = lod_from_table_name(table_name).ok_or_else(|| {
        format!("Table '{table_name}' is missing a '_lod_X_Y' suffix; cannot determine LOD for update")
    })?;

    let conn = connection
        .lock()
        .map_err(|e| format!("Failed to lock connection: {e}"))?;

    let exists_sql = format!("SELECT COUNT(*) FROM citylake.{table_name} WHERE id = ?");
    let count: i64 = conn
        .prepare(&exists_sql)
        .and_then(|mut stmt| stmt.query_row([id], |row| row.get(0)))
        .map_err(|e| format!("Failed to check if record exists: {e}"))?;

    if count == 0 {
        return Err(format!("No record found with id '{id}' in table '{table_name}'").into());
    }

    let mut temp_file =
        NamedTempFile::with_suffix(".city.jsonl").map_err(|e| format!("Failed to create temp file: {e}"))?;
    writeln!(temp_file, "{cityjson_data}")
        .map_err(|e| format!("Failed to write to temp file: {e}"))?;
    temp_file
        .flush()
        .map_err(|e| format!("Failed to flush temp file: {e}"))?;

    let temp_path = temp_file.path().display().to_string();
    let path_lit = temp_path.replace('\'', "''");
    let lod_lit = lod.as_str();

    let delete_sql = format!("DELETE FROM citylake.{table_name} WHERE id = ?");
    conn.execute(&delete_sql, [id])
        .map_err(|e| format!("Failed to delete old record: {e}"))?;

    let insert_sql = format!(
        "INSERT INTO citylake.{table_name} \
         SELECT * FROM read_cityjsonseq('{path_lit}', lod => '{lod_lit}')"
    );
    conn.execute_batch(&insert_sql)
        .map_err(|e| format!("Failed to insert updated record: {e}"))?;

    tracing::info!("Updated object '{id}' in table '{table_name}' (lod={lod_lit})");
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::core::interface::repository::CityLakeRepository;
    use crate::tests::helpers;

    #[tokio::test]
    async fn test_update_nonexistent_object() {
        let service = helpers::setup_with_table("update_test_lod_2_2");
        let result = service
            .update_object("update_test_lod_2_2", "nonexistent_id", "{}")
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No record found"));
    }

    #[tokio::test]
    async fn test_update_rejects_non_lod_table() {
        let service = helpers::setup_with_table("plain_table");
        let result = service.update_object("plain_table", "id", "{}").await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("missing a '_lod_X_Y' suffix"));
    }
}
