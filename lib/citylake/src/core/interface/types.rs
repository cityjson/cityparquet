//! The domain model: a package-shaped dataset, its validated names, and the
//! error type every repository method returns.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::core::db::sql::{self, SqlError};

/// Configuration for a running CityLake service.
#[derive(Debug, Clone, Serialize)]
pub struct CityLakeConfig {
    /// Path where DuckLake stores the Parquet data files.
    pub storage_path: String,
    /// Path for the DuckLake metadata catalog file (`.ducklake`).
    pub catalog_path: String,
    /// Name DuckLake attaches the catalog under.
    pub catalog_name: String,
    /// Host address for the HTTP server.
    pub host: String,
    /// Port for the HTTP server.
    pub port: u16,
}

impl Default for CityLakeConfig {
    fn default() -> Self {
        Self {
            storage_path: "data".to_string(),
            catalog_path: "metadata.ducklake".to_string(),
            catalog_name: "lake".to_string(),
            host: "127.0.0.1".to_string(),
            port: 3000,
        }
    }
}

impl CityLakeConfig {
    /// The configuration, with each field taken from its environment variable
    /// when set and left at its default when not.
    ///
    /// An unset variable means "use the default"; a variable set to something
    /// unusable is an error, because an operator who set it meant it. That
    /// covers both an unparseable `CITYLAKE_PORT` and any variable set to
    /// bytes that are not valid UTF-8 — `std::env::var` reports the latter as
    /// an error distinct from "unset", and treating it the same as unset
    /// would silently use the default instead of reporting the mistake.
    pub fn from_env() -> Self {
        let default = Self::default();
        Self {
            host: Self::var_or("CITYLAKE_HOST", default.host),
            port: Self::port_from(&Self::var_or("CITYLAKE_PORT", default.port.to_string()))
                .expect("CITYLAKE_PORT must be a port number"),
            catalog_name: Self::var_or("CITYLAKE_CATALOG_NAME", default.catalog_name),
            catalog_path: Self::var_or("CITYLAKE_CATALOG_PATH", default.catalog_path),
            storage_path: Self::var_or("CITYLAKE_STORAGE_PATH", default.storage_path),
        }
    }

    /// Read one variable: absent means "use the default", present and
    /// unusable is an error. A value that is set but not valid UTF-8 is a
    /// mistake worth reporting, not a reason to quietly use something else.
    fn var_or(name: &str, default: String) -> String {
        Self::resolve_var(name, std::env::var(name), default)
    }

    /// The decision `var_or` applies, pulled out of the actual environment
    /// read so it is testable without touching the process environment — the
    /// isolation hazard `from_env`'s own tests are written to avoid: unset
    /// uses the default, set uses the value, and set-but-not-UTF-8 panics
    /// naming the variable rather than falling back.
    fn resolve_var(
        name: &str,
        result: Result<String, std::env::VarError>,
        default: String,
    ) -> String {
        match result {
            Ok(value) => value,
            Err(std::env::VarError::NotPresent) => default,
            Err(std::env::VarError::NotUnicode(_)) => {
                panic!("{name} is set to a value that is not valid UTF-8")
            }
        }
    }

    /// Parse a port, so the parsing is testable without touching the process
    /// environment — a test that sets a variable can be seen by every other
    /// test in the binary.
    pub fn port_from(raw: &str) -> Result<u16, std::num::ParseIntError> {
        raw.parse()
    }
}

/// A validated dataset name. It becomes a schema name, so validation happens
/// once here and every consumer downstream can assume it.
///
/// `new` is the only way to construct one — there is no public field, no
/// `From<String>`, and no `Deserialize` impl, so an unvalidated string cannot
/// reach the SQL builder through this type. A handler that receives a dataset
/// name from a request validates it at the boundary by calling `new`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DatasetName(String);

impl DatasetName {
    pub fn new(name: &str) -> Result<Self, CityLakeError> {
        sql::validate_dataset(name)?;
        Ok(Self(name.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A validated module name — one of the CityGML object modules or sidecar
/// tables the specification defines. Same construction discipline as
/// [`DatasetName`]: `new` is the only way in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModuleName(String);

impl ModuleName {
    pub fn new(name: &str) -> Result<Self, CityLakeError> {
        sql::validate_module(name)?;
        Ok(Self(name.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A dataset's shape: which modules it carries and what CRS it declares.
#[derive(Debug, Clone, Serialize)]
pub struct DatasetInfo {
    pub name: String,
    pub modules: Vec<ModuleInfo>,
    pub crs: Option<String>,
}

/// One module table within a dataset.
#[derive(Debug, Clone, Serialize)]
pub struct ModuleInfo {
    pub name: String,
    pub role: String,
    pub rows: usize,
}

/// One file written (or rewritten) by a package write.
#[derive(Debug, Clone, Serialize)]
pub struct PackageFile {
    pub file: String,
    pub action: String,
    pub rows: i64,
    pub bytes: i64,
}

/// One finding from `cityparquet_validate`.
#[derive(Debug, Clone, Serialize)]
pub struct ValidationFinding {
    pub check_name: String,
    pub severity: String,
    pub table_name: String,
    pub object_id: Option<String>,
    pub message: String,
}

/// Statistics returned after a compaction operation.
#[derive(Debug, Clone, Serialize)]
pub struct CompactionStats {
    pub files_processed: usize,
    pub files_created: usize,
}

/// Query parameters for paginated object reads.
///
/// The default bounds the page at 100 rows: an unbounded default would let
/// one request pull a national dataset into memory.
#[derive(Debug, Clone)]
pub struct QueryParams {
    pub filter: Option<String>,
    pub limit: usize,
    pub offset: usize,
}

impl Default for QueryParams {
    fn default() -> Self {
        Self {
            filter: None,
            limit: 100,
            offset: 0,
        }
    }
}

/// Supported CityJSON-family export formats.
///
/// A closed, fieldless enum — unlike the validated newtypes above, there is
/// no invariant `Deserialize` could let a caller bypass. Deserialisation
/// itself is the validation: an unrecognised string simply fails to parse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    CityJson,
    CityJsonSeq,
    FlatCityBuf,
}

impl ExportFormat {
    /// The DuckDB `FORMAT` string for `COPY TO`.
    pub fn as_duckdb_format(&self) -> &'static str {
        match self {
            ExportFormat::CityJson => "cityjson",
            ExportFormat::CityJsonSeq => "cityjsonseq",
            ExportFormat::FlatCityBuf => "flatcitybuf",
        }
    }
}

/// Every error a repository method can return. A handler turns each variant
/// into a status code; `Box<dyn Error>` could not do that.
#[derive(Debug, Error)]
pub enum CityLakeError {
    #[error(transparent)]
    Sql(#[from] SqlError),

    #[error(transparent)]
    Duckdb(#[from] duckdb::Error),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error("dataset {0:?} does not exist")]
    DatasetNotFound(String),

    #[error("dataset {0:?} already exists")]
    DatasetExists(String),

    #[error("module {module:?} not found in dataset {dataset:?}")]
    ModuleNotFound { dataset: String, module: String },

    #[error("object {id:?} not found in dataset {dataset:?}")]
    ObjectNotFound { dataset: String, id: String },

    #[error("dataset {0:?} has no object table")]
    NoObjectTable(String),

    #[error("{0}")]
    Internal(String),
}

/// Result type every repository method returns.
pub type RepositoryResult<T> = Result<T, CityLakeError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dataset_names_are_validated_at_construction() {
        assert_eq!(DatasetName::new("delft").unwrap().as_str(), "delft");
        // Constructing the newtype is the only way in, so an invalid name
        // cannot reach the SQL builder at all.
        assert!(DatasetName::new("delft; DROP SCHEMA public").is_err());
        assert!(DatasetName::new("").is_err());
    }

    #[test]
    fn module_names_are_validated_at_construction() {
        assert_eq!(
            ModuleName::new("water_body").unwrap().as_str(),
            "water_body"
        );
        assert!(ModuleName::new("buildings").is_err());
    }

    #[test]
    fn the_default_catalog_is_named_lake() {
        let config = CityLakeConfig::default();
        assert_eq!(config.catalog_name, "lake");
        assert_eq!(config.catalog_path, "metadata.ducklake");
        assert_eq!(config.storage_path, "data");
        assert_eq!(config.port, 3000);
    }

    #[test]
    fn query_params_default_to_a_bounded_page() {
        // An unbounded default would let one request pull a national dataset
        // into memory.
        let params = QueryParams::default();
        assert_eq!(params.limit, 100);
        assert_eq!(params.offset, 0);
        assert!(params.filter.is_none());
    }

    #[test]
    fn the_config_falls_back_to_its_defaults() {
        // Nothing set: from_env must agree with Default in every field, so an
        // unconfigured run behaves exactly as it did before it could be configured.
        let from_env = CityLakeConfig::from_env();
        let default = CityLakeConfig::default();
        assert_eq!(from_env.host, default.host);
        assert_eq!(from_env.port, default.port);
        assert_eq!(from_env.catalog_name, default.catalog_name);
        assert_eq!(from_env.catalog_path, default.catalog_path);
        assert_eq!(from_env.storage_path, default.storage_path);
    }

    #[test]
    fn an_unparseable_port_is_an_error_not_a_silent_default() {
        // A typo in CITYLAKE_PORT must not quietly serve on 3000 — the operator
        // asked for something specific and deserves to be told it was not honoured.
        assert!(CityLakeConfig::port_from("not-a-number").is_err());
        assert_eq!(CityLakeConfig::port_from("3100").unwrap(), 3100);
    }

    #[test]
    fn var_resolution_uses_the_value_when_present() {
        // Exercises resolve_var's decision table directly, on a constructed
        // Result rather than a real environment variable — the isolation
        // hazard named above applies here too.
        assert_eq!(
            CityLakeConfig::resolve_var("X", Ok("set".to_string()), "default".to_string()),
            "set"
        );
    }

    #[test]
    fn var_resolution_uses_the_default_when_unset() {
        assert_eq!(
            CityLakeConfig::resolve_var(
                "X",
                Err(std::env::VarError::NotPresent),
                "default".to_string()
            ),
            "default"
        );
    }

    #[test]
    fn var_resolution_panics_naming_the_variable_when_not_utf8() {
        // A value that is set but not valid UTF-8 must not be treated like
        // "unset" — that would silently fall back exactly where the operator
        // set the variable on purpose. The OsString's content is irrelevant:
        // resolve_var panics on the VarError::NotUnicode variant itself, so
        // no real invalid-UTF-8 bytes are needed to exercise this arm.
        let outcome = std::panic::catch_unwind(|| {
            CityLakeConfig::resolve_var(
                "CITYLAKE_PORT",
                Err(std::env::VarError::NotUnicode(std::ffi::OsString::from(
                    "irrelevant",
                ))),
                "default".to_string(),
            )
        });
        let err = outcome.expect_err("resolve_var must panic on a non-UTF-8 value");
        let message = err
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| err.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_default();
        assert!(message.contains("CITYLAKE_PORT"));
        assert!(message.contains("not valid UTF-8"));
    }
}
