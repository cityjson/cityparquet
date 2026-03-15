use duckdb::Connection;
use std::sync::{Arc, Mutex};

use crate::core::interface::repository::RepositoryResult;
use crate::core::interface::types::{CityJsonMetadata, InputFormat};

/// Get metadata from a CityJSON source file using the cityjson extension.
pub async fn get_metadata(
    connection: &Arc<Mutex<Connection>>,
    file_path: &str,
) -> RepositoryResult<CityJsonMetadata> {
    let format = InputFormat::from_path(file_path)
        .ok_or_else(|| format!("Cannot detect CityJSON format from path: {file_path}"))?;

    let metadata_fn = format
        .metadata_function()
        .ok_or_else(|| format!("Metadata extraction not supported for this format"))?;

    let conn = connection
        .lock()
        .map_err(|e| format!("Failed to lock connection: {e}"))?;

    let sql = format!("SELECT to_json(m) AS json_row FROM {metadata_fn}('{file_path}') m");

    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| format!("Failed to prepare metadata query: {e}"))?;

    let json_str: String = stmt
        .query_row([], |row| row.get(0))
        .map_err(|e| format!("Failed to read metadata: {e}"))?;

    let metadata: CityJsonMetadata = serde_json::from_str(&json_str)
        .map_err(|e| format!("Failed to parse metadata JSON: {e}"))?;

    Ok(metadata)
}

#[cfg(test)]
mod tests {
    use crate::core::interface::repository::CityLakeRepository;
    use crate::tests::helpers;

    #[tokio::test]
    async fn test_get_metadata_unsupported_format() {
        let service = helpers::setup();
        let result = service.get_metadata("/tmp/test.fcb").await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Metadata extraction not supported"));
    }

    #[tokio::test]
    async fn test_get_metadata_unknown_format() {
        let service = helpers::setup();
        let result = service.get_metadata("/tmp/test.csv").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Cannot detect"));
    }
}
