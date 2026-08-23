use duckdb::Connection;
use std::sync::{Arc, Mutex};

use crate::core::interface::repository::RepositoryResult;
use crate::core::interface::types::{CityLakeConfig, InputFormat, LodKey};

use super::lod::{derive_table_name, discover_lods};
use super::table::validate_identifier;

/// Insert CityJSON objects from a file into per-LOD table(s).
///
/// See [`CityLakeRepository::insert_objects`] for the full contract.
pub async fn insert_objects(
    connection: &Arc<Mutex<Connection>>,
    base_name: &str,
    file_path: &str,
    lod: Option<&LodKey>,
    config: &CityLakeConfig,
) -> RepositoryResult<usize> {
    validate_identifier(base_name, "Base name")?;

    let format = InputFormat::from_path(file_path)
        .ok_or_else(|| format!("Cannot detect CityJSON format from path: {file_path}"))?;

    let lods: Vec<LodKey> = match lod {
        Some(l) => vec![l.clone()],
        None => discover_lods(connection, file_path, format)?,
    };

    let conn = connection
        .lock()
        .map_err(|e| format!("Failed to lock connection: {e}"))?;

    let mut total_inserted = 0usize;
    for lod_key in &lods {
        let table_name = derive_table_name(base_name, lod_key);
        validate_identifier(&table_name, "Table name")?;

        let count_sql = format!("SELECT COUNT(*) FROM citylake.{table_name}");
        let before: i64 = conn
            .prepare(&count_sql)
            .and_then(|mut stmt| stmt.query_row([], |row| row.get(0)))
            .map_err(|e| format!("Failed to count rows in '{table_name}': {e}"))?;

        let path_lit = file_path.replace('\'', "''");
        let lod_lit = lod_key.as_str();
        let insert_sql = format!(
            "INSERT INTO citylake.{table_name} \
             SELECT * FROM {read_fn}('{path_lit}', lod => '{lod_lit}')",
            read_fn = format.read_function(),
        );

        conn.execute_batch(&insert_sql)
            .map_err(|e| format!("Failed to insert objects into '{table_name}': {e}"))?;

        let after: i64 = conn
            .prepare(&count_sql)
            .and_then(|mut stmt| stmt.query_row([], |row| row.get(0)))
            .map_err(|e| format!("Failed to count rows in '{table_name}' after insert: {e}"))?;

        let inserted = (after - before) as usize;
        tracing::info!(
            "Inserted {inserted} objects into '{table_name}' from '{file_path}' (lod={lod_lit})"
        );
        total_inserted += inserted;
    }

    if config.auto_compact {
        tracing::debug!("Auto-compaction check for base '{base_name}' (not yet implemented)");
    }

    Ok(total_inserted)
}

#[cfg(test)]
mod tests {
    use crate::core::interface::repository::CityLakeRepository;
    use crate::core::interface::types::LodKey;
    use crate::tests::helpers;

    #[tokio::test]
    async fn test_insert_invalid_format() {
        let service = helpers::setup_with_table("insert_fmt_lod_2_2");
        let lod = LodKey::parse("2.2").unwrap();
        let result = service
            .insert_objects("insert_fmt", "/tmp/bad_file.csv", Some(&lod))
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
