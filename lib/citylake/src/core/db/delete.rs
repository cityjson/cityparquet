use duckdb::Connection;
use std::sync::{Arc, Mutex};

use crate::core::interface::repository::RepositoryResult;

use super::table::validate_identifier;

/// Delete a CityJSON object by its ID.
pub async fn delete_object(
    connection: &Arc<Mutex<Connection>>,
    table_name: &str,
    id: &str,
) -> RepositoryResult<()> {
    validate_identifier(table_name, "Table name")?;

    let conn = connection
        .lock()
        .map_err(|e| format!("Failed to lock connection: {e}"))?;

    let sql = format!("DELETE FROM citylake.{table_name} WHERE id = ?");
    let affected = conn
        .execute(&sql, [id])
        .map_err(|e| format!("Failed to delete object '{id}' from '{table_name}': {e}"))?;

    if affected == 0 {
        return Err(format!("No record found with id '{id}' in table '{table_name}'").into());
    }

    tracing::info!("Deleted object '{id}' from table '{table_name}'");
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::core::interface::repository::CityLakeRepository;
    use crate::core::interface::types::QueryParams;
    use crate::tests::helpers;

    #[tokio::test]
    async fn test_delete_existing_object() {
        let service = helpers::setup_with_table("delete_test");

        // Count before
        let params = QueryParams { filter: None, limit: None, offset: None };
        let before = service.query_objects("delete_test", &params).await.unwrap();
        let before_count = before.len();

        // Delete first object
        service
            .delete_object("delete_test", "building_001")
            .await
            .unwrap();

        // Count after
        let after = service.query_objects("delete_test", &params).await.unwrap();
        assert_eq!(after.len(), before_count - 1);
    }

    #[tokio::test]
    async fn test_delete_nonexistent_object() {
        let service = helpers::setup_with_table("delete_nonexist");
        let result = service
            .delete_object("delete_nonexist", "nonexistent_id")
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No record found"));
    }

    #[tokio::test]
    async fn test_delete_rejects_sql_injection_in_table_name() {
        let service = helpers::setup_with_table("del_inj");
        let malicious = "x'; DROP TABLE del_inj; --";
        let result = service.delete_object(malicious, "id").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("invalid characters"));
    }
}
