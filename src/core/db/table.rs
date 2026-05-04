use duckdb::Connection;
use std::sync::{Arc, Mutex};

use crate::core::interface::repository::RepositoryResult;
use crate::core::interface::types::{InputFormat, LodKey, DEFAULT_BASE_NAME};

use super::lod::{derive_table_name, discover_lods};
use super::metadata_table::persist_metadata;

/// Validate that a SQL identifier contains only safe characters.
pub(super) fn validate_identifier(name: &str, kind: &str) -> RepositoryResult<()> {
    if name.is_empty() {
        return Err(format!("{kind} cannot be empty").into());
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(format!(
            "{kind} '{name}' contains invalid characters. Only alphanumeric and underscore allowed."
        )
        .into());
    }
    Ok(())
}

/// Create one table per LOD from a CityJSON source file, plus a shared metadata row.
///
/// See [`CityLakeRepository::create_table`] for the full contract.
pub async fn create_table(
    connection: &Arc<Mutex<Connection>>,
    base_name: Option<&str>,
    source_path: &str,
    lod: Option<&LodKey>,
) -> RepositoryResult<Vec<String>> {
    let base = base_name.unwrap_or(DEFAULT_BASE_NAME);
    validate_identifier(base, "Base name")?;

    let format = InputFormat::from_path(source_path)
        .ok_or_else(|| format!("Cannot detect CityJSON format from path: {source_path}"))?;

    let lods: Vec<LodKey> = match lod {
        Some(l) => vec![l.clone()],
        None => discover_lods(connection, source_path, format)?,
    };

    let mut created = Vec::with_capacity(lods.len());
    {
        let conn = connection
            .lock()
            .map_err(|e| format!("Failed to lock connection: {e}"))?;

        for lod_key in &lods {
            let table_name = derive_table_name(base, lod_key);
            validate_identifier(&table_name, "Table name")?;

            let path_lit = source_path.replace('\'', "''");
            let lod_lit = lod_key.as_str();
            let sql = format!(
                "CREATE TABLE citylake.{table_name} AS \
                 SELECT * FROM {read_fn}('{path_lit}', lod => '{lod_lit}')",
                read_fn = format.read_function(),
            );

            conn.execute_batch(&sql)
                .map_err(|e| format!("Failed to create table '{table_name}': {e}"))?;

            tracing::info!("Created table '{table_name}' from '{source_path}' (lod={lod_lit})");
            created.push(table_name);
        }
    }

    // Metadata table: shared across datasets, keyed by `dataset` (= base name).
    let persisted = persist_metadata(connection, base, source_path, format)?;
    if !persisted {
        tracing::debug!(
            "Skipping metadata persistence for '{source_path}': format does not expose a metadata function"
        );
    }

    Ok(created)
}

/// Check if a table exists in the DuckLake catalog.
pub async fn table_exists(
    connection: &Arc<Mutex<Connection>>,
    table_name: &str,
) -> RepositoryResult<bool> {
    validate_identifier(table_name, "Table name")?;

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

#[cfg(test)]
mod tests {
    use crate::core::interface::repository::CityLakeRepository;
    use crate::core::interface::types::LodKey;
    use crate::tests::helpers;

    #[test]
    fn test_validate_identifier_valid() {
        assert!(super::validate_identifier("buildings", "x").is_ok());
        assert!(super::validate_identifier("my_table_123", "x").is_ok());
    }

    #[test]
    fn test_validate_identifier_invalid() {
        let result = super::validate_identifier("bad-name!", "Table name");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("invalid characters"));
    }

    #[test]
    fn test_validate_identifier_empty() {
        let result = super::validate_identifier("", "Table name");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("cannot be empty"));
    }

    #[tokio::test]
    async fn test_table_exists_true() {
        let service = helpers::setup_with_table("test_exists_lod_2_2");
        let exists = service.table_exists("test_exists_lod_2_2").await.unwrap();
        assert!(exists);
    }

    #[tokio::test]
    async fn test_table_exists_false() {
        let service = helpers::setup();
        let exists = service.table_exists("nonexistent").await.unwrap();
        assert!(!exists);
    }

    #[tokio::test]
    async fn test_create_table_invalid_base_rejected() {
        let service = helpers::setup();
        let lod = LodKey::parse("2.2").unwrap();
        let result = service
            .create_table(Some("bad-name!"), "test.city.jsonl", Some(&lod))
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("invalid characters"));
    }

    #[tokio::test]
    async fn test_create_table_unknown_format_rejected() {
        let service = helpers::setup();
        let lod = LodKey::parse("2.2").unwrap();
        let result = service
            .create_table(Some("test"), "test.csv", Some(&lod))
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Cannot detect"));
    }
}
