//! Update, delete, reconcile.
//!
//! There is deliberately no `cityparquet_update` to call: attribute edits are
//! an ordinary UPDATE and need no wrapper. What structural edits invalidate is
//! derived state — feature_id, the reciprocal hierarchy, bbox — and
//! `cityparquet_reconcile` re-derives exactly that. Delete is different: it
//! has to cascade through `children`, so it goes through the pragma.

use duckdb::{Connection, ToSql};

use crate::core::db::service::DuckLakeService;
use crate::core::db::sql;
use crate::core::interface::types::{CityLakeError, DatasetName, RepositoryResult};

impl DuckLakeService {
    /// Update `id`'s attributes with an ordinary `UPDATE`. The row's module
    /// table is not asked of the caller — an id is unique across the whole
    /// package, so it is found by searching the object tables.
    pub fn update_object_impl(
        &self,
        dataset: &DatasetName,
        id: &str,
        attributes: &serde_json::Map<String, serde_json::Value>,
    ) -> RepositoryResult<()> {
        if attributes.is_empty() {
            return Ok(());
        }
        let name = dataset.as_str();

        self.with_connection(|conn| {
            if !self.schema_exists(conn, name)? {
                return Err(CityLakeError::DatasetNotFound(name.to_string()));
            }
            let Some(module) = self.module_holding(conn, name, id)? else {
                return Err(CityLakeError::Internal(format!(
                    "no object with id {id} in dataset {name}"
                )));
            };

            self.in_transaction(conn, |conn| {
                // Column names are identifiers and cannot be parameterised —
                // they are quoted through `sql::ident`, same as every other
                // identifier this module handles.
                let assignments = attributes
                    .keys()
                    .map(|column| format!("{} = ?", sql::ident(column)))
                    .collect::<Vec<_>>()
                    .join(", ");
                let statement = format!(
                    "UPDATE {} SET {assignments} WHERE id = ?",
                    sql::qualified(&[self.catalog(), name, &module])
                );

                // Values are bound, not interpolated — only identifiers cannot
                // be parameterised.
                let mut params: Vec<Box<dyn ToSql>> =
                    attributes.values().map(value_to_sql).collect();
                params.push(Box::new(id.to_string()));
                let refs: Vec<&dyn ToSql> = params.iter().map(|p| p.as_ref()).collect();
                conn.execute(&statement, refs.as_slice())?;

                // An attribute edit alone cannot invalidate derived state, but
                // nothing here stops a caller sending a geometry column among
                // the attributes, and reconciling an already-correct package
                // is a no-op — see `reconciling_an_untouched_dataset_changes_
                // nothing` in tests/mutate.rs.
                let path = format!("{}.{name}", self.catalog());
                self.with_search_path(conn, &path, |conn| {
                    conn.execute_batch(&sql::reconcile_pragma(name))?;
                    Ok(())
                })
            })
        })
    }

    /// Delete one object by id — the predicate that selects exactly it.
    pub fn delete_object_impl(&self, dataset: &DatasetName, id: &str) -> RepositoryResult<usize> {
        self.delete_where_impl(dataset, &format!("id = {}", sql::literal(id)))
    }

    /// Delete every object `predicate` selects, cascading through `children`.
    ///
    /// `predicate` is caller-supplied SQL text and is interpolated as
    /// written, never bound as a parameter: `cityparquet_delete` takes its
    /// predicate as a SQL fragment by design (it has to appear inside a
    /// `WHERE` clause the pragma builds), so there is nothing to bind it to.
    /// This is the one deliberate exception to "bind, don't interpolate" in
    /// this module, and it matches the trust model `select_objects`'s
    /// `filter` already uses — see the specification's §10.
    pub fn delete_where_impl(
        &self,
        dataset: &DatasetName,
        predicate: &str,
    ) -> RepositoryResult<usize> {
        let name = dataset.as_str();

        self.with_connection(|conn| {
            if !self.schema_exists(conn, name)? {
                return Err(CityLakeError::DatasetNotFound(name.to_string()));
            }
            let before = self.total_object_rows(conn, name)?;

            let path = format!("{}.{name}", self.catalog());
            self.in_transaction(conn, |conn| {
                self.with_search_path(conn, &path, |conn| {
                    // Cascade is the default and walks `children`
                    // transitively, never `feature_id` equality — deleting a
                    // BuildingPart must not take out the Building sharing its
                    // feature_id.
                    conn.execute_batch(&sql::delete_pragma(name, predicate, true))?;
                    Ok(())
                })
            })?;

            Ok(before - self.total_object_rows(conn, name)?)
        })
    }

    /// Re-derive the structural state a structural edit invalidates:
    /// feature_id, the reciprocal hierarchy, bbox.
    pub fn reconcile_impl(&self, dataset: &DatasetName) -> RepositoryResult<()> {
        let name = dataset.as_str();
        self.with_connection(|conn| {
            if !self.schema_exists(conn, name)? {
                return Err(CityLakeError::DatasetNotFound(name.to_string()));
            }
            let path = format!("{}.{name}", self.catalog());
            self.in_transaction(conn, |conn| {
                self.with_search_path(conn, &path, |conn| {
                    conn.execute_batch(&sql::reconcile_pragma(name))?;
                    Ok(())
                })
            })
        })
    }

    /// Which object table holds `id`. Ids are unique across the whole
    /// package, so at most one table answers.
    fn module_holding(
        &self,
        conn: &Connection,
        dataset: &str,
        id: &str,
    ) -> RepositoryResult<Option<String>> {
        for table in self.object_tables(conn, dataset)? {
            let found: bool = conn.query_row(
                &format!(
                    "SELECT COUNT(*) > 0 FROM {} WHERE id = ?",
                    sql::qualified(&[self.catalog(), dataset, &table])
                ),
                [id],
                |row| row.get(0),
            )?;
            if found {
                return Ok(Some(table));
            }
        }
        Ok(None)
    }
}

/// Bind a JSON value as the DuckDB parameter it names. `Null` becomes a real
/// SQL `NULL` rather than the four-character string `"null"`; a JSON array or
/// object — not expected among a CityParquet object's attributes, but not
/// ruled out by the type either — falls back to its JSON text so a caller
/// sending one gets a clear column-type error instead of a silently dropped
/// value.
fn value_to_sql(value: &serde_json::Value) -> Box<dyn ToSql> {
    match value {
        serde_json::Value::Null => Box::new(None::<String>),
        serde_json::Value::String(s) => Box::new(s.clone()),
        serde_json::Value::Number(n) if n.is_i64() => Box::new(n.as_i64().unwrap()),
        // Above i64::MAX but still an exact non-negative integer — CityJSON
        // attributes carry identifiers and codes that can be this large, and
        // duckdb-rs's `ToSql` covers `u64` directly, so it is bound exactly
        // rather than narrowed through `f64`, which starts losing integers
        // past 2^53.
        serde_json::Value::Number(n) if n.is_u64() => Box::new(n.as_u64().unwrap()),
        serde_json::Value::Number(n) => Box::new(n.as_f64().unwrap_or_default()),
        serde_json::Value::Bool(b) => Box::new(*b),
        other => Box::new(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use duckdb::types::{ToSqlOutput, Value};

    #[test]
    fn a_number_above_i64_max_binds_as_an_exact_u64_not_a_lossy_f64() {
        // 2^63 — one past i64::MAX, the smallest value that forces the u64
        // branch. Round-tripped through f64 this loses precision (f64 only
        // represents integers exactly up to 2^53); bound as u64 it does not.
        let huge = serde_json::json!(9_223_372_036_854_775_808u64);
        let bound = value_to_sql(&huge);
        assert_eq!(
            bound.to_sql().unwrap(),
            ToSqlOutput::Owned(Value::UBigInt(9_223_372_036_854_775_808u64))
        );
    }

    #[test]
    fn an_ordinary_integer_still_binds_as_i64() {
        let small = serde_json::json!(42i64);
        let bound = value_to_sql(&small);
        assert_eq!(
            bound.to_sql().unwrap(),
            ToSqlOutput::Owned(Value::BigInt(42))
        );
    }

    #[test]
    fn null_binds_as_a_real_sql_null_not_the_text_null() {
        let bound = value_to_sql(&serde_json::Value::Null);
        assert_eq!(bound.to_sql().unwrap(), ToSqlOutput::Owned(Value::Null));
    }
}
