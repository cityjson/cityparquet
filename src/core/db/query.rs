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
