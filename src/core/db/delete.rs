use duckdb::Connection;
use std::sync::{Arc, Mutex};

use crate::core::interface::repository::RepositoryResult;

/// Delete a CityJSON object by its ID.
pub async fn delete_object(
    connection: &Arc<Mutex<Connection>>,
    table_name: &str,
    id: &str,
) -> RepositoryResult<()> {
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
