//! Ingesting a further source into an existing dataset.
//!
//! One pragma does all of it. What is worth knowing is what the pragma
//! guarantees, because CityLake must not second-guess any of it: routing is by
//! CityGML module and is total, ids are identity so a duplicate refuses the
//! whole insert, the CRS must match and is never reprojected, and derived
//! state is re-derived afterwards.

use duckdb::Connection;

use crate::core::db::service::DuckLakeService;
use crate::core::db::sql;
use crate::core::interface::types::{CityLakeError, DatasetName, RepositoryResult};

impl DuckLakeService {
    pub fn ingest_impl(&self, dataset: &DatasetName, source_path: &str) -> RepositoryResult<usize> {
        let name = dataset.as_str();
        let format = sql::reader_for(source_path);

        self.with_connection(|conn| {
            if !self.schema_exists(conn, name)? {
                return Err(CityLakeError::DatasetNotFound(name.to_string()));
            }
            let before = self.total_object_rows(conn, name)?;

            let path = format!("{}.{name}", self.catalog());
            self.in_transaction(conn, |conn| {
                self.with_search_path(conn, &path, |conn| {
                    // create_tables = true so a source spanning a module this
                    // dataset has not seen yet brings its table with it.
                    conn.execute_batch(&sql::insert_pragma(name, source_path, format, true))?;
                    Ok(())
                })
            })?;

            Ok(self.total_object_rows(conn, name)? - before)
        })
    }

    fn total_object_rows(&self, conn: &Connection, dataset: &str) -> RepositoryResult<usize> {
        let mut total = 0usize;
        for table in self.object_tables(conn, dataset)? {
            let rows: i64 = conn.query_row(
                &format!(
                    "SELECT COUNT(*) FROM {}",
                    sql::qualified(&[self.catalog(), dataset, &table])
                ),
                [],
                |row| row.get(0),
            )?;
            total += rows as usize;
        }
        Ok(total)
    }
}
