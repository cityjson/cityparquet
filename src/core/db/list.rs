//! `list_tables` — enumerate every table in the citylake catalog and decorate
//! each with the parsed `(base, lod)` derived from its `_lod_X_Y` suffix.

use duckdb::Connection;
use std::sync::{Arc, Mutex};

use crate::core::interface::repository::RepositoryResult;
use crate::core::interface::types::TableInfo;

use super::lod::lod_from_table_name;

pub async fn list_tables(
    connection: &Arc<Mutex<Connection>>,
) -> RepositoryResult<Vec<TableInfo>> {
    let conn = connection
        .lock()
        .map_err(|e| format!("Failed to lock connection: {e}"))?;

    let mut stmt = conn
        .prepare(
            "SELECT table_name FROM information_schema.tables \
             WHERE table_catalog = 'citylake' \
             ORDER BY table_name",
        )
        .map_err(|e| format!("Failed to prepare list_tables query: {e}"))?;

    let rows = stmt
        .query_map([], |row| {
            let name: String = row.get(0)?;
            Ok(name)
        })
        .map_err(|e| format!("Failed to execute list_tables query: {e}"))?;

    let mut out = Vec::new();
    for r in rows {
        let name = r.map_err(|e| format!("Failed to read table row: {e}"))?;
        let lod = lod_from_table_name(&name);
        let base = lod.as_ref().and_then(|l| {
            // "buildings_lod_2_2" → strip "_lod_2_2" → "buildings"
            let suffix_len = "_".len() + l.as_suffix().len();
            if name.len() > suffix_len {
                Some(name[..name.len() - suffix_len].to_string())
            } else {
                None
            }
        });
        out.push(TableInfo {
            name,
            base,
            lod: lod.map(|l| l.as_str().to_string()),
        });
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use crate::core::interface::repository::CityLakeRepository;
    use crate::tests::helpers;

    #[tokio::test]
    async fn test_list_tables_includes_lod_metadata() {
        let service = helpers::setup_with_table("buildings_lod_2_2");
        let tables = service.list_tables().await.unwrap();
        let found = tables
            .iter()
            .find(|t| t.name == "buildings_lod_2_2")
            .expect("table not in list");
        assert_eq!(found.base.as_deref(), Some("buildings"));
        assert_eq!(found.lod.as_deref(), Some("2.2"));
    }

    #[tokio::test]
    async fn test_list_tables_handles_non_lod_table() {
        let service = helpers::setup_with_table("plain");
        let tables = service.list_tables().await.unwrap();
        let found = tables.iter().find(|t| t.name == "plain").unwrap();
        assert!(found.base.is_none());
        assert!(found.lod.is_none());
    }
}
