//! Validation and housekeeping.
//!
//! A PRAGMA cannot be a subquery, so both `cityparquet_validate` and
//! `cityparquet_orphans` materialise their findings into a temp table which is
//! then selected from — that is what keeps the results filterable rather
//! than fixed at the call.

use crate::core::db::service::DuckLakeService;
use crate::core::db::sql;
use crate::core::interface::types::{
    CityLakeError, DatasetName, RepositoryResult, ValidationFinding,
};

impl DuckLakeService {
    /// Run every structural check `cityparquet_validate` knows and report what
    /// it found. Read-only: this diagnoses, it does not repair.
    /// `cityparquet_validation` has replace semantics — each call's findings
    /// are the whole result, never appended to a previous call's — and its
    /// columns are `check_name`, `severity`, `table_name`, `object_id`,
    /// `message`, in that order.
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
    /// `cityparquet_vacuum` itself. Both run inside one transaction. Both
    /// pragmas materialise their findings into a temp table, which lives in
    /// the `temp` catalog, while the deletes land in the attached `lake`
    /// catalog — and DuckDB permits `temp` alongside one attached database in
    /// a single transaction. This differs from the CRS probe in `dataset.rs`,
    /// which writes to a *second attached* database and for that reason has
    /// to run outside its ingest transaction. If a future DuckDB version
    /// tightens the rule and this transaction starts failing, the fallback is
    /// to drop `in_transaction` here and run the two pragmas as separate
    /// statements: `cityparquet_vacuum` is idempotent, so a failure between
    /// them leaves the package consistent, merely un-vacuumed.
    ///
    /// A known limitation: `cityparquet_validate.cpp`'s
    /// `HasNonNullTemplateReference` (line 32) probes for template references
    /// on its own connection using two-part names, which do not resolve under
    /// an attached catalog's search path. That failure is fail-safe by
    /// construction — a failed probe contributes no term and the
    /// "undeterminable" fallback fires — so the effect is that
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
