use duckdb::Connection;
use std::sync::{Arc, Mutex};

use crate::core::interface::repository::RepositoryResult;
use crate::core::interface::types::QueryParams;

/// Query objects from a table with optional filters and pagination.
///
/// Returns rows as JSON values. Each row is converted to a JSON object
/// using DuckDB's `to_json()` function.
pub async fn query_objects(
    connection: &Arc<Mutex<Connection>>,
    table_name: &str,
    params: &QueryParams,
) -> RepositoryResult<Vec<serde_json::Value>> {
    let conn = connection
        .lock()
        .map_err(|e| format!("Failed to lock connection: {e}"))?;

    // Build query with optional WHERE, LIMIT, OFFSET
    let mut sql = format!("SELECT to_json(t) AS json_row FROM citylake.{table_name} t");

    if let Some(filter) = &params.filter {
        sql.push_str(&format!(" WHERE {filter}"));
    }

    if let Some(limit) = params.limit {
        sql.push_str(&format!(" LIMIT {limit}"));
    }

    if let Some(offset) = params.offset {
        sql.push_str(&format!(" OFFSET {offset}"));
    }

    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| format!("Failed to prepare query: {e}"))?;

    let rows = stmt
        .query_map([], |row| {
            let json_str: String = row.get(0)?;
            Ok(json_str)
        })
        .map_err(|e| format!("Failed to execute query: {e}"))?;

    let mut results = Vec::new();
    for row in rows {
        let json_str = row.map_err(|e| format!("Failed to read row: {e}"))?;
        let value: serde_json::Value = serde_json::from_str(&json_str)
            .map_err(|e| format!("Failed to parse JSON row: {e}"))?;
        results.push(value);
    }

    Ok(results)
}

#[cfg(test)]
mod tests {
    use crate::core::interface::repository::CityLakeRepository;
    use crate::core::interface::types::QueryParams;
    use crate::tests::helpers;

    fn empty_params() -> QueryParams {
        QueryParams {
            filter: None,
            limit: None,
            offset: None,
        }
    }

    #[tokio::test]
    async fn test_query_all_objects() {
        let service = helpers::setup_with_table("query_all");
        let results = service
            .query_objects("query_all", &empty_params())
            .await
            .unwrap();
        assert_eq!(results.len(), 3, "Expected 3 objects from test data");
    }

    #[tokio::test]
    async fn test_query_with_limit() {
        let service = helpers::setup_with_table("query_limit");
        let params = QueryParams {
            limit: Some(1),
            ..empty_params()
        };
        let results = service
            .query_objects("query_limit", &params)
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
    }

    #[tokio::test]
    async fn test_query_with_offset() {
        let service = helpers::setup_with_table("query_offset");
        let params = QueryParams {
            offset: Some(2),
            ..empty_params()
        };
        let results = service
            .query_objects("query_offset", &params)
            .await
            .unwrap();
        assert_eq!(results.len(), 1, "Expected 1 object after offset=2 of 3 total");
    }

    #[tokio::test]
    async fn test_query_with_filter() {
        let service = helpers::setup_with_table("query_filter");
        let params = QueryParams {
            filter: Some("id = 'building_001'".to_string()),
            ..empty_params()
        };
        let results = service
            .query_objects("query_filter", &params)
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
    }

    #[tokio::test]
    async fn test_query_returns_json() {
        let service = helpers::setup_with_table("query_json");
        let results = service
            .query_objects("query_json", &empty_params())
            .await
            .unwrap();
        assert!(!results.is_empty());
        for val in &results {
            assert!(val.is_object(), "Expected JSON object, got: {val}");
        }
    }
}
