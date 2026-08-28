//! Validation and housekeeping.
//!
//! A PRAGMA cannot be a subquery, so both `cityparquet_validate` and
//! `cityparquet_orphans` materialise their findings into a temp table which is
//! then selected from -- that is what keeps the results filterable rather
//! than fixed at the call.

use crate::core::db::service::DuckLakeService;
use crate::core::db::sql;
use crate::core::interface::types::{
    CityLakeError, DatasetName, RepositoryResult, ValidationFinding,
};

impl DuckLakeService {
    /// Run every structural check `cityparquet_validate` knows and report what
    /// it found. Read-only: this diagnoses, it does not repair. Empirically
    /// confirmed columns and order (`check_name`, `severity`, `table_name`,
    /// `object_id`, `message`) via `DESCRIBE SELECT * FROM
    /// cityparquet_validation` against a DuckLake-attached catalog.
    pub fn validate_impl(&self, dataset: &DatasetName) -> RepositoryResult<Vec<ValidationFinding>> {
        let name = dataset.as_str();
        self.with_connection(|conn| {
            if !self.schema_exists(conn, name)? {
                return Err(CityLakeError::DatasetNotFound(name.to_string()));
            }
            let path = format!("{}.{name}", self.catalog());
            self.with_search_path(conn, &path, |conn| {
                conn.execute_batch(&sql::validate_pragma(name))?;
                let mut stmt = conn.prepare(
                    "SELECT check_name, severity, table_name, object_id, message
                     FROM cityparquet_validation",
                )?;
                let findings = stmt
                    .query_map([], |row| {
                        Ok(ValidationFinding {
                            check_name: row.get(0)?,
                            severity: row.get(1)?,
                            table_name: row.get(2)?,
                            object_id: row.get(3)?,
                            message: row.get(4)?,
                        })
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(findings)
            })
        })
    }

    /// Reclaim unreferenced sidecar rows: `cityparquet_orphans` first, so the
    /// returned count reports what vacuum is about to take, then
    /// `cityparquet_vacuum` itself. Both run inside one transaction --
    /// confirmed empirically (see the task report) that DuckDB permits this
    /// even though the pragmas' temp table lives in the `temp` catalog while
    /// the deletes land in the attached `lake` catalog: unlike Task 6's CRS
    /// probe, which wrote a *second attached* database, `temp` is not subject
    /// to the "one attached database per transaction" rule DuckDB enforces.
    /// If a future DuckDB version tightens that and this transaction starts
    /// failing, the fallback is to drop `in_transaction` here and run the two
    /// pragmas as separate statements: `cityparquet_vacuum` is idempotent, so
    /// a failure between them leaves the package consistent, merely
    /// un-vacuumed.
    ///
    /// A known, undocumented-elsewhere limitation: `cityparquet_validate.cpp`'s
    /// `HasNonNullTemplateReference` (line 32) probes for template references
    /// on its own connection using two-part names, which do not resolve under
    /// an attached catalog's search path. That failure is fail-safe by
    /// construction -- a failed probe contributes no term and the
    /// "undeterminable" fallback fires -- so the effect is that
    /// `geometry_templates` orphans are **not** vacuumed from a
    /// DuckLake-backed package. Missed cleanup, never data loss. This is the
    /// extension's limitation, not fixed here.
    pub fn vacuum_impl(&self, dataset: &DatasetName) -> RepositoryResult<usize> {
        let name = dataset.as_str();
        self.with_connection(|conn| {
            if !self.schema_exists(conn, name)? {
                return Err(CityLakeError::DatasetNotFound(name.to_string()));
            }
            let path = format!("{}.{name}", self.catalog());
            self.in_transaction(conn, |conn| {
                self.with_search_path(conn, &path, |conn| {
                    // Orphans first, so the count reports what vacuum is about
                    // to take.
                    conn.execute_batch(&sql::orphans_pragma(name))?;
                    let count: i64 = conn.query_row(
                        "SELECT COUNT(*) FROM cityparquet_orphan_rows",
                        [],
                        |row| row.get(0),
                    )?;
                    conn.execute_batch(&sql::vacuum_pragma(name))?;
                    Ok(count as usize)
                })
            })
        })
    }
}
