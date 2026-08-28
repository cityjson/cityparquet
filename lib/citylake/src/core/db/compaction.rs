//! DuckLake maintenance.
//!
//! Compaction here is merging a table's small Parquet files via DuckLake's own
//! `ducklake_merge_adjacent_files` — not CTAS, DROP and RENAME, which would
//! rewrite the table behind DuckLake's back and lose its snapshots.

use crate::core::db::service::DuckLakeService;
use crate::core::db::sql;
use crate::core::interface::types::{
    CityLakeError, CompactionStats, DatasetName, RepositoryResult,
};

impl DuckLakeService {
    /// Merge each object table's small Parquet files. Sidecars (`materials`,
    /// `textures`, `geometry_templates`) are deliberately out of scope: they
    /// are not among `object_tables`, which only lists the rows the
    /// extension's own registry marks `role = 'object'`.
    pub fn compact_impl(&self, dataset: &DatasetName) -> RepositoryResult<CompactionStats> {
        let name = dataset.as_str();
        self.with_connection(|conn| {
            if !self.schema_exists(conn, name)? {
                return Err(CityLakeError::DatasetNotFound(name.to_string()));
            }

            let mut stats = CompactionStats {
                files_processed: 0,
                files_created: 0,
            };
            for table in self.object_tables(conn, name)? {
                let mut stmt = conn.prepare(&sql::compact(self.catalog(), name, &table))?;
                let rows = stmt
                    .query_map([], |row| {
                        Ok((
                            row.get::<_, i64>("files_processed")?,
                            row.get::<_, i64>("files_created")?,
                        ))
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                for (processed, created) in rows {
                    stats.files_processed += processed as usize;
                    stats.files_created += created as usize;
                }
            }
            Ok(stats)
        })
    }
}
