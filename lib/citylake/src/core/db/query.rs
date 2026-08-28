//! Reading a module table's objects back out as a bounded page of JSON rows.
//!
//! Everything here is a `SELECT`: no CityJSON structure is parsed or
//! interpreted, only moved from a DuckDB row to a `serde_json::Value`.

use duckdb::Connection;

use crate::core::db::service::DuckLakeService;
use crate::core::db::sql;
use crate::core::interface::types::{
    CityLakeError, DatasetName, ModuleName, QueryParams, RepositoryResult,
};

impl DuckLakeService {
    pub fn query_objects_impl(
        &self,
        dataset: &DatasetName,
        module: &ModuleName,
        params: &QueryParams,
    ) -> RepositoryResult<Vec<serde_json::Value>> {
        let (name, module_name) = (dataset.as_str(), module.as_str());

        self.with_connection(|conn| {
            if !self.table_exists(conn, name, module_name)? {
                return Err(CityLakeError::ModuleNotFound {
                    dataset: name.to_string(),
                    module: module_name.to_string(),
                });
            }

            // `filter` is a caller-supplied SQL predicate, interpolated as
            // written: cityparquet_delete takes a predicate string by design
            // and the query filter matches it. See the specification's §10 for
            // the trust model this assumes.
            let sql_text = sql::select_objects(
                self.catalog(),
                name,
                module_name,
                params.filter.as_deref(),
                params.limit,
                params.offset,
            );

            let mut stmt = conn.prepare(&sql_text)?;
            let rows = stmt
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;

            rows.into_iter()
                .map(|json| {
                    serde_json::from_str(&json)
                        .map_err(|e| CityLakeError::Internal(format!("row is not JSON: {e}")))
                })
                .collect()
        })
    }

    pub(crate) fn table_exists(
        &self,
        conn: &Connection,
        dataset: &str,
        table: &str,
    ) -> RepositoryResult<bool> {
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM information_schema.tables
             WHERE table_catalog = ? AND table_schema = ? AND table_name = ?",
            [self.catalog(), dataset, table],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }
}
