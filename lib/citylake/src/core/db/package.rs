//! The boundary between the lake and the file format.
//!
//! DuckLake stores its own Parquet plus manifests, which is not a CityParquet
//! package — so writing one out is a real conversion, not a formality. Loading
//! one back is the inverse, and merging two of them is the extension's, not
//! CityLake's: every rule about identity, CRS and sidecar renumbering lives in
//! the pragma this module calls.

use crate::core::db::service::DuckLakeService;
use crate::core::db::sql;
use crate::core::interface::types::{
    CityLakeError, DatasetInfo, DatasetName, ExportFormat, ModuleName, PackageFile,
    RepositoryResult,
};

impl DuckLakeService {
    /// Load an existing package directory. `cityparquet_read` creates the
    /// schema inside the attached catalog and recovers each file's Parquet
    /// footer — the one thing a hand-rolled `read_parquet` load throws away,
    /// and with it the CRS. Nothing needs minting here.
    pub fn import_package(
        &self,
        dataset: &DatasetName,
        directory: &str,
    ) -> RepositoryResult<DatasetInfo> {
        let name = dataset.as_str();
        let catalog = self.catalog().to_string();

        self.with_connection(|conn| {
            if self.schema_exists(conn, name)? {
                return Err(CityLakeError::DatasetExists(name.to_string()));
            }
            self.in_transaction(conn, |conn| {
                // The pragma issues an unqualified `CREATE SCHEMA`, so the
                // search path is what puts the schema inside the DuckLake
                // catalog rather than the default one. It carries the catalog
                // alone: there is no schema to name yet.
                self.with_search_path(conn, &catalog, |conn| {
                    conn.execute_batch(&sql::read_package_pragma(directory, name))?;
                    Ok(())
                })
            })?;
            self.describe_locked(conn, name)
        })
    }

    /// Write the dataset out as a CityParquet package directory.
    pub fn write_package_impl(
        &self,
        dataset: &DatasetName,
        output_dir: &str,
    ) -> RepositoryResult<Vec<PackageFile>> {
        let name = dataset.as_str();

        self.with_connection(|conn| {
            if !self.schema_exists(conn, name)? {
                return Err(CityLakeError::DatasetNotFound(name.to_string()));
            }
            // `cityparquet_write` runs on a connection of the extension's own
            // and sees committed state only, so there is deliberately no
            // transaction around it: mutate, commit, then write.
            let crs = self.dataset_crs(conn, name)?;
            let statement = sql::write_package(name, output_dir, crs.as_deref());
            let path = format!("{}.{name}", self.catalog());

            self.with_search_path(conn, &path, |conn| {
                let mut stmt = conn.prepare(&statement)?;
                let written = stmt
                    .query_map([], |row| {
                        Ok(PackageFile {
                            file: row.get(0)?,
                            action: row.get(1)?,
                            rows: row.get(2)?,
                            bytes: row.get(3)?,
                        })
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(written)
            })
        })
    }

    /// Export one module table to a single CityJSON-family file.
    pub fn export_module_impl(
        &self,
        dataset: &DatasetName,
        module: &ModuleName,
        output_path: &str,
        format: ExportFormat,
    ) -> RepositoryResult<()> {
        let (name, module_name) = (dataset.as_str(), module.as_str());

        self.with_connection(|conn| {
            if !self.table_exists(conn, name, module_name)? {
                // The two faults lead a caller to look in different places, so
                // they are told apart — as `query_objects_impl` does, and only
                // on this path, so the common case pays nothing for it.
                if !self.schema_exists(conn, name)? {
                    return Err(CityLakeError::DatasetNotFound(name.to_string()));
                }
                return Err(CityLakeError::ModuleNotFound {
                    dataset: name.to_string(),
                    module: module_name.to_string(),
                });
            }

            // COPY inherits a source's metadata only when the SELECT names
            // exactly one reader. A table is not statically discoverable, so
            // the CRS has to be stated or the output would declare none.
            //
            // What it is stated *as* matters here in a way it does not for a
            // package write: this value lands verbatim in the output's
            // `metadata.referenceSystem`, which CityJSON defines as a URI. So
            // the URI form is preferred, and the footer's PROJJSON is only the
            // fallback for a CRS whose footer carries no identifier — stating
            // something imperfect beats stating nothing.
            let crs = match self.dataset_crs_uri(conn, name)? {
                uri @ Some(_) => uri,
                None => self.dataset_crs(conn, name)?,
            };
            let mut options = format!("FORMAT {}", format.as_duckdb_format());
            if let Some(crs) = crs {
                options.push_str(&format!(", crs {}", sql::literal(&crs)));
            }
            conn.execute_batch(&format!(
                "COPY (SELECT * FROM {}) TO {} ({options})",
                sql::qualified(&[self.catalog(), name, module_name]),
                sql::literal(output_path)
            ))?;
            Ok(())
        })
    }

    /// Fold `source` into `destination`. Identity, the one-CRS rule and
    /// sidecar renumbering are all the pragma's; CityLake only makes both
    /// schemas reachable and gives the whole thing a transaction to fail in.
    pub fn merge_impl(
        &self,
        destination: &DatasetName,
        source: &DatasetName,
    ) -> RepositoryResult<()> {
        let (dst, src) = (destination.as_str(), source.as_str());

        self.with_connection(|conn| {
            for name in [dst, src] {
                if !self.schema_exists(conn, name)? {
                    return Err(CityLakeError::DatasetNotFound(name.to_string()));
                }
            }
            // The pragma names both schemas explicitly but qualifies neither
            // with a catalog, so both must resolve through the search path: an
            // empty one fails with "Schema with name dst does not exist". Two
            // entries say that requirement outright. One would in fact do —
            // resolution walks the *catalogs* on the path, and either entry
            // brings the whole lake catalog with it — but that is incidental,
            // and a path naming both schemas cannot quietly stop being enough.
            let path = format!("{0}.{dst},{0}.{src}", self.catalog());
            self.in_transaction(conn, |conn| {
                self.with_search_path(conn, &path, |conn| {
                    conn.execute_batch(&sql::merge_pragma(dst, src))?;
                    Ok(())
                })
            })
        })
    }
}
