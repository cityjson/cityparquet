//! Dataset lifecycle: create, list, describe, drop.
//!
//! A dataset is a CityParquet package living as a schema in the DuckLake
//! catalog. Creating one from a CityJSON-family source has to bootstrap the
//! package first — there is no pragma that makes one from nothing — and then
//! give it a CRS the extension's own guard can check against.

use duckdb::{Connection, OptionalExt};

use crate::core::db::service::DuckLakeService;
use crate::core::db::sql;
use crate::core::interface::types::{
    CityLakeError, DatasetInfo, DatasetName, ModuleInfo, RepositoryResult,
};

/// DuckLake's own default schema in the attached catalog. It is not a dataset,
/// so it never appears in a listing.
const CATALOG_DEFAULT_SCHEMA: &str = "main";

impl DuckLakeService {
    pub fn create_dataset_impl(
        &self,
        dataset: &DatasetName,
        source_path: &str,
    ) -> RepositoryResult<DatasetInfo> {
        // A directory is an existing package: cityparquet_read loads it and
        // recovers each file's Parquet footer, so its CRS arrives with it and
        // none of the file bootstrap applies.
        if std::path::Path::new(source_path).is_dir() {
            return self.import_package(dataset, source_path);
        }

        let name = dataset.as_str();
        let format = sql::reader_for(source_path);
        let catalog = self.catalog().to_string();
        let path = format!("{catalog}.{name}");

        self.with_connection(|conn| {
            if self.schema_exists(conn, name)? {
                return Err(CityLakeError::DatasetExists(name.to_string()));
            }

            // Phase 1 — the ingest, one unit against the lake catalog. A
            // create that fails partway must not leave an addressable
            // half-built dataset behind.
            self.in_transaction(conn, |conn| {
                conn.execute_batch(&sql::create_schema(&catalog, name))?;
                conn.execute_batch(&sql::seed_table(&catalog, name, source_path, format))?;

                self.with_search_path(conn, &path, |conn| {
                    // One pragma per statement: DuckDB expands every pragma in
                    // a script before running any of it, so a batched pair
                    // would each see pre-batch state.
                    conn.execute_batch(&sql::init_pragma(name))?;
                    conn.execute_batch(&sql::insert_pragma(name, source_path, format, true))?;
                    Ok(())
                })
            })?;

            // Phases 2 and 3 — outside that transaction, and they must be:
            // cityparquet_write sees committed state only, and a transaction
            // that has written to `lake` may not also write to `memory`.
            if let Err(e) = self.mint_crs_footer(conn, name, source_path, format) {
                // The ingest is already committed, so unwinding is explicit.
                // A dataset whose CRS guard is silently off is worse than none.
                // The mint failure is the one worth returning, so a drop
                // failure on top of it is logged rather than propagated.
                if let Err(drop_err) = conn.execute_batch(&format!(
                    "DROP SCHEMA {} CASCADE",
                    sql::qualified(&[&catalog, name])
                )) {
                    tracing::error!(%drop_err, "dropping the failed dataset's schema failed");
                }
                return Err(e);
            }

            self.describe_locked(conn, name)
        })
    }

    pub fn list_datasets_impl(&self) -> RepositoryResult<Vec<String>> {
        self.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT schema_name FROM information_schema.schemata
                 WHERE catalog_name = ? AND schema_name != ?
                 ORDER BY schema_name",
            )?;
            let names = stmt
                .query_map([self.catalog(), CATALOG_DEFAULT_SCHEMA], |row| row.get(0))?
                .collect::<Result<Vec<String>, _>>()?;
            Ok(names)
        })
    }

    pub fn describe_dataset_impl(&self, dataset: &DatasetName) -> RepositoryResult<DatasetInfo> {
        let name = dataset.as_str();
        self.with_connection(|conn| {
            if !self.schema_exists(conn, name)? {
                return Err(CityLakeError::DatasetNotFound(name.to_string()));
            }
            self.describe_locked(conn, name)
        })
    }

    pub fn drop_dataset_impl(&self, dataset: &DatasetName) -> RepositoryResult<()> {
        let name = dataset.as_str();
        self.with_connection(|conn| {
            if !self.schema_exists(conn, name)? {
                return Err(CityLakeError::DatasetNotFound(name.to_string()));
            }
            // CASCADE, because a package's tables are the package: a schema
            // emptied of them is not a state CityLake has a name for.
            conn.execute_batch(&format!(
                "DROP SCHEMA {} CASCADE",
                sql::qualified(&[self.catalog(), name])
            ))?;
            Ok(())
        })
    }

    /// The `crs` field of any object-table footer.
    ///
    /// Every object table in a package states the same CRS — the extension
    /// refuses a package whose footers disagree — so one row answers for all
    /// of them. `None` means the package states no CRS, which is a state
    /// rather than a failure.
    ///
    /// A package with no footer row at all already comes back as `Ok(None)`
    /// through `optional`, so only a real failure reaches the error arm. It
    /// must propagate: a write and an export both choose what CRS to state
    /// from this value, and a swallowed error would silently produce a
    /// CRS-less package instead of failing.
    pub(crate) fn dataset_crs(
        &self,
        conn: &Connection,
        dataset: &str,
    ) -> RepositoryResult<Option<String>> {
        Ok(conn
            .query_row(
                &format!(
                    "SELECT cityparquet_city_field(city, 'crs') FROM {}
                     WHERE role = 'object' AND city IS NOT NULL LIMIT 1",
                    sql::qualified(&[self.catalog(), dataset, "__cityparquet"])
                ),
                [],
                |row| row.get(0),
            )
            .optional()?
            .flatten())
    }

    /// The same CRS as the OGC URI that CityJSON's `metadata.referenceSystem`
    /// expects — `https://www.opengis.net/def/crs/EPSG/0/7415` — or `None`
    /// when the footer carries no identifier to build one from.
    ///
    /// This resolves nothing. `id.authority` and `id.code` are values the
    /// extension itself resolved and wrote into the footer; reading them back
    /// and formatting them into the standard template is string assembly, not
    /// CRS logic, so it stays on the right side of the rule that every CRS
    /// question belongs to the extension. The concatenation is NULL-propagating,
    /// so a PROJJSON without an `id` yields `None` rather than a URI with
    /// holes in it.
    pub(crate) fn dataset_crs_uri(
        &self,
        conn: &Connection,
        dataset: &str,
    ) -> RepositoryResult<Option<String>> {
        Ok(conn
            .query_row(
                &format!(
                    "SELECT 'https://www.opengis.net/def/crs/'
                            || json_extract_string(crs, '$.id.authority')
                            || '/0/'
                            || json_extract_string(crs, '$.id.code')
                     FROM (SELECT cityparquet_city_field(city, 'crs') AS crs FROM {}
                           WHERE role = 'object' AND city IS NOT NULL LIMIT 1)",
                    sql::qualified(&[self.catalog(), dataset, "__cityparquet"])
                ),
                [],
                |row| row.get(0),
            )
            .optional()?
            .flatten())
    }

    /// The package's object tables, as the extension's own registry records
    /// them. The sidecars are excluded: they hold appearance, not objects.
    pub(crate) fn object_tables(
        &self,
        conn: &Connection,
        dataset: &str,
    ) -> RepositoryResult<Vec<String>> {
        let mut stmt = conn.prepare(&format!(
            "SELECT table_name FROM {} WHERE role = 'object' ORDER BY table_name",
            sql::qualified(&[self.catalog(), dataset, "__cityparquet"])
        ))?;
        let tables = stmt
            .query_map([], |row| row.get(0))?
            .collect::<Result<Vec<String>, _>>()?;
        Ok(tables)
    }

    /// Describe a dataset on a connection the caller already holds, so a
    /// create can report what it built without releasing the lock.
    ///
    /// `pub(crate)`, not private: `package.rs` is a sibling module, and an
    /// import reports what it loaded the same way a create does.
    pub(crate) fn describe_locked(
        &self,
        conn: &Connection,
        dataset: &str,
    ) -> RepositoryResult<DatasetInfo> {
        let mut stmt = conn.prepare(&format!(
            "SELECT table_name, role FROM {} ORDER BY role, table_name",
            sql::qualified(&[self.catalog(), dataset, "__cityparquet"])
        ))?;
        let registered = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let mut modules = Vec::with_capacity(registered.len());
        for (table, role) in registered {
            let rows: i64 = conn.query_row(
                &format!(
                    "SELECT COUNT(*) FROM {}",
                    sql::qualified(&[self.catalog(), dataset, &table])
                ),
                [],
                |row| row.get(0),
            )?;
            modules.push(ModuleInfo {
                name: table,
                role,
                rows: rows as usize,
            });
        }

        Ok(DatasetInfo {
            name: dataset.to_string(),
            modules,
            crs: self.dataset_crs(conn, dataset)?,
        })
    }

    /// Give the package a CRS the extension can check against.
    ///
    /// A DuckLake table has no Parquet footer, so `__cityparquet.city` is NULL
    /// and the CRS guard is silent. The footer's `crs` is canonical PROJJSON
    /// minted by the extension's resolver, so CityLake cannot write one — it
    /// asks the extension to, by writing a single row to a throwaway package
    /// and reading the footer back. Only `crs` is kept: the guard reads that
    /// field alone, and a minimal value carries no stale probe inventory.
    ///
    /// A source that declares no referenceSystem leaves the footer NULL, which
    /// is the correct "CRS unknown" state rather than a guess.
    ///
    /// Every statement here runs **outside** the ingest transaction, and must:
    /// `cityparquet_write` sees committed state only, and a transaction that
    /// has written to the lake catalog may not also write to `memory`, where
    /// the probe lives.
    fn mint_crs_footer(
        &self,
        conn: &Connection,
        dataset: &str,
        source_path: &str,
        format: sql::SourceFormat,
    ) -> RepositoryResult<()> {
        // `reference_system` is a struct — struct(base_url, authority,
        // version, code) — so the authority:code spelling the writer wants is
        // assembled in SQL. Rust never inspects or resolves a CRS.
        let reference_system: Option<String> = conn
            .query_row(
                &format!(
                    "SELECT reference_system.authority || ':' || reference_system.code
                     FROM {}({})",
                    match format {
                        sql::SourceFormat::CityJson => "cityjson_metadata",
                        sql::SourceFormat::CityJsonSeq => "cityjsonseq_metadata",
                        sql::SourceFormat::FlatCityBuf => "flatcitybuf_metadata",
                    },
                    sql::literal(source_path)
                ),
                [],
                |row| row.get(0),
            )
            // A source that declares nothing already comes back as Ok(None),
            // so only a real failure reaches the error arm — and swallowing
            // one here would hand back a package whose CRS guard is silently
            // off, which is the outcome this whole sequence exists to avoid.
            .optional()?
            .flatten();

        let Some(reference_system) = reference_system else {
            tracing::info!(dataset, "source declares no CRS; the package states none");
            return Ok(());
        };

        // A non-empty object table is required: cityparquet_write emits one
        // file per non-empty table, and an empty probe would produce no footer.
        // The COUNT(*) query's own failure must propagate rather than read as
        // "empty" — swallowing it here would surface later as a misleading
        // NoObjectTable instead of the real cause.
        let mut module = None;
        for table in self.object_tables(conn, dataset)? {
            let non_empty: bool = conn.query_row(
                &format!(
                    "SELECT COUNT(*) > 0 FROM {}",
                    sql::qualified(&[self.catalog(), dataset, &table])
                ),
                [],
                |row| row.get(0),
            )?;
            if non_empty {
                module = Some(table);
                break;
            }
        }
        let Some(module) = module else {
            return Err(CityLakeError::NoObjectTable(dataset.to_string()));
        };

        let probe = format!("__citylake_crs_{dataset}");
        let probe_dir = tempfile::tempdir()?;
        let probe_path = probe_dir.path().to_string_lossy().into_owned();

        // The probe lives in the default catalog: one row, no dependency on
        // the attached-catalog write path.
        conn.execute_batch(&format!("CREATE SCHEMA {}", sql::ident(&probe)))?;
        let minted = self.run_crs_probe(
            conn,
            dataset,
            &probe,
            &probe_path,
            &module,
            &reference_system,
        );
        if let Err(drop_err) =
            conn.execute_batch(&format!("DROP SCHEMA {} CASCADE", sql::ident(&probe)))
        {
            // The probe's own failure is the one worth reporting, so a drop
            // failure on top of it is logged rather than returned. The schema
            // is a throwaway in the default catalog: leaking one costs a name,
            // not data.
            tracing::error!(%drop_err, "dropping the CRS probe schema failed");
        }

        // The source stated a CRS, so failing to mint is fatal: the caller
        // would otherwise get a package whose guard is silently off.
        let footer = minted?.ok_or_else(|| {
            CityLakeError::Internal(format!(
                "could not mint a CRS footer for {dataset}: the source declares \
                 {reference_system} but no footer came back from the probe"
            ))
        })?;

        // Every object table in a package states the same CRS.
        conn.execute(
            &format!(
                "UPDATE {} SET city = ? WHERE role = 'object'",
                sql::qualified(&[self.catalog(), dataset, "__cityparquet"])
            ),
            [&footer],
        )?;
        Ok(())
    }

    /// Write one row out as a package and read the footer the extension put on
    /// it. Split from [`Self::mint_crs_footer`] so the probe schema is dropped
    /// on the way out whether this succeeds or fails.
    fn run_crs_probe(
        &self,
        conn: &Connection,
        dataset: &str,
        probe: &str,
        probe_path: &str,
        module: &str,
        reference_system: &str,
    ) -> RepositoryResult<Option<String>> {
        conn.execute_batch(&format!(
            "CREATE TABLE {}.{} AS SELECT * FROM {} LIMIT 1",
            sql::ident(probe),
            sql::ident(module),
            sql::qualified(&[self.catalog(), dataset, module])
        ))?;
        conn.execute_batch(&sql::init_pragma(probe))?;
        conn.execute_batch(&sql::write_package(
            probe,
            probe_path,
            Some(reference_system),
        ))?;

        let footer: Option<String> = conn
            .query_row(
                &format!(
                    "SELECT json_object('crs', json_extract(decode(value), '$.crs'))::VARCHAR
                     FROM parquet_kv_metadata({})
                     WHERE decode(key) = 'city'",
                    sql::literal(&format!("{probe_path}/{module}.parquet"))
                ),
                [],
                |row| row.get(0),
            )
            .ok();
        Ok(footer)
    }
}
