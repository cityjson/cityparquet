//! The domain model: a package-shaped dataset, its validated names, and the
//! error type every repository method returns.

use serde::Serialize;
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
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

    /// The file extension for this format.
    pub fn file_extension(&self) -> &'static str {
        match self {
            ExportFormat::CityJson => ".city.json",
            ExportFormat::CityJsonSeq => ".city.jsonl",
            ExportFormat::FlatCityBuf => ".fcb",
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
    fn export_formats_map_to_duckdb_and_to_file_extensions() {
        assert_eq!(ExportFormat::CityJsonSeq.as_duckdb_format(), "cityjsonseq");
        assert_eq!(ExportFormat::CityJsonSeq.file_extension(), ".city.jsonl");
        assert_eq!(ExportFormat::CityJson.file_extension(), ".city.json");
        assert_eq!(ExportFormat::FlatCityBuf.file_extension(), ".fcb");
    }
}
