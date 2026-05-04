use serde::{Deserialize, Serialize};
use std::fmt;

/// Default base name used when the caller does not supply one.
pub const DEFAULT_BASE_NAME: &str = "city_objects";

/// Name of the shared metadata table that accumulates one row per ingested dataset.
pub const METADATA_TABLE: &str = "cityjson_metadata";

/// Configuration for CityLake service
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CityLakeConfig {
    /// Path where DuckLake stores Parquet data files
    pub storage_path: String,
    /// Path for the DuckLake metadata catalog file (.ducklake)
    pub catalog_path: String,
    /// Whether to auto-compact tables after inserts
    pub auto_compact: bool,
    /// Compaction threshold configuration
    pub compaction_threshold: CompactionThreshold,
    /// Host address for the HTTP server
    pub host: String,
    /// Port for the HTTP server
    pub port: u16,
}

impl Default for CityLakeConfig {
    fn default() -> Self {
        Self {
            storage_path: "data".to_string(),
            catalog_path: "metadata.ducklake".to_string(),
            auto_compact: false,
            compaction_threshold: CompactionThreshold::default(),
            host: "127.0.0.1".to_string(),
            port: 3000,
        }
    }
}

/// Threshold configuration for triggering compaction
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionThreshold {
    /// Minimum number of data files before compaction is considered
    pub min_file_count: usize,
    /// Fragmentation ratio (0.0-1.0) above which compaction is triggered
    pub fragmentation_ratio: f64,
}

impl Default for CompactionThreshold {
    fn default() -> Self {
        Self {
            min_file_count: 10,
            fragmentation_ratio: 0.3,
        }
    }
}

/// Statistics returned after a compaction operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionStats {
    pub files_before: usize,
    pub files_after: usize,
    pub rows_compacted: usize,
}

/// Metadata extracted from a CityJSON source file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CityJsonMetadata {
    pub version: Option<String>,
    pub identifier: Option<String>,
    pub reference_system: Option<String>,
    pub geographical_extent: Option<GeographicalExtent>,
    pub transform_scale: Option<[f64; 3]>,
    pub transform_translate: Option<[f64; 3]>,
}

/// Geographical bounding box
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeographicalExtent {
    pub min_x: f64,
    pub min_y: f64,
    pub min_z: f64,
    pub max_x: f64,
    pub max_y: f64,
    pub max_z: f64,
}

/// Supported CityJSON export formats
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    CityJson,
    CityJsonSeq,
    FlatCityBuf,
}

impl ExportFormat {
    /// Returns the DuckDB FORMAT string for COPY TO
    pub fn as_duckdb_format(&self) -> &'static str {
        match self {
            ExportFormat::CityJson => "cityjson",
            ExportFormat::CityJsonSeq => "cityjsonseq",
            ExportFormat::FlatCityBuf => "flatcitybuf",
        }
    }

    /// Returns the file extension for this format
    pub fn file_extension(&self) -> &'static str {
        match self {
            ExportFormat::CityJson => ".city.json",
            ExportFormat::CityJsonSeq => ".city.jsonl",
            ExportFormat::FlatCityBuf => ".fcb",
        }
    }
}

impl fmt::Display for ExportFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_duckdb_format())
    }
}

/// Supported input file formats (detected by extension)
#[derive(Debug, Clone, Copy)]
pub enum InputFormat {
    CityJson,
    CityJsonSeq,
    FlatCityBuf,
}

impl InputFormat {
    /// Detect format from file path extension
    pub fn from_path(path: &str) -> Option<Self> {
        let lower = path.to_lowercase();
        if lower.ends_with(".city.json") || lower.ends_with(".cityjson") {
            Some(InputFormat::CityJson)
        } else if lower.ends_with(".city.jsonl") || lower.ends_with(".cityjsonl") || lower.ends_with(".jsonl") {
            Some(InputFormat::CityJsonSeq)
        } else if lower.ends_with(".fcb") || lower.ends_with(".flatcitybuf") {
            Some(InputFormat::FlatCityBuf)
        } else {
            None
        }
    }

    /// Returns the DuckDB read function name for this format
    pub fn read_function(&self) -> &'static str {
        match self {
            InputFormat::CityJson => "read_cityjson",
            InputFormat::CityJsonSeq => "read_cityjsonseq",
            InputFormat::FlatCityBuf => "read_flatcitybuf",
        }
    }

    /// Returns the DuckDB metadata function name for this format
    pub fn metadata_function(&self) -> Option<&'static str> {
        match self {
            InputFormat::CityJson => Some("cityjson_metadata"),
            InputFormat::CityJsonSeq => Some("cityjsonseq_metadata"),
            InputFormat::FlatCityBuf => None,
        }
    }
}

/// A validated CityJSON Level-of-Detail identifier (e.g. `"2.2"`, `"1"`).
///
/// LOD strings are accepted in two forms: integer (`"2"`) or `major.minor` decimal
/// (`"2.2"`). Anything else is rejected at construction time. The newtype centralises
/// formatting so call sites cannot accidentally inject an unsanitised value into a
/// SQL string or table name.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LodKey(String);

impl LodKey {
    /// Parse, validate, and canonicalise a LOD string. Accepts `\d+(\.\d+)?`
    /// (e.g. `"2"`, `"2.2"`). Each numeric part is stripped of leading zeros so
    /// that `"02.20"` and `"2.20"` and `"2.2"` collapse to the same key — this
    /// prevents the same logical LOD from mapping to different table names.
    pub fn parse(s: &str) -> Result<Self, String> {
        if s.is_empty() {
            return Err("LOD cannot be empty".to_string());
        }
        let mut saw_dot = false;
        for c in s.chars() {
            match c {
                '0'..='9' => {}
                '.' if !saw_dot => saw_dot = true,
                _ => return Err(format!("Invalid LOD '{s}': only digits and a single '.' allowed")),
            }
        }
        if s.starts_with('.') || s.ends_with('.') {
            return Err(format!("Invalid LOD '{s}': leading or trailing '.' not allowed"));
        }
        let canonical = s
            .split('.')
            .map(strip_leading_zeros)
            .collect::<Vec<_>>()
            .join(".");
        Ok(LodKey(canonical))
    }

    /// Original LOD string, e.g. `"2.2"`.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Suffix used for DuckDB table names, e.g. `"lod_2_2"`. Dots are replaced with
    /// underscores so the result is a valid SQL identifier.
    pub fn as_suffix(&self) -> String {
        format!("lod_{}", self.0.replace('.', "_"))
    }
}

impl fmt::Display for LodKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

fn strip_leading_zeros(part: &str) -> String {
    let trimmed = part.trim_start_matches('0');
    if trimmed.is_empty() {
        "0".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Request body for creating a table
#[derive(Debug, Deserialize)]
pub struct CreateTableRequest {
    /// Path to a CityJSON source file on the server
    pub source_path: Option<String>,
    /// Optional LOD selector. When set, only that LOD is loaded and the suffix is
    /// `_lod_X_Y`. When unset, every LOD found in the source is loaded into its own
    /// table.
    pub lod: Option<String>,
    /// Optional base name for the created tables. Defaults to `city_objects`.
    /// The final table name is `{base}_lod_X_Y` per LOD.
    pub base_name: Option<String>,
}

/// Request body for inserting objects
#[derive(Debug, Deserialize)]
pub struct InsertRequest {
    /// Path to a CityJSON source file on the server
    pub source_path: Option<String>,
    /// Optional LOD selector. When set, only that LOD is read from the source.
    /// When unset, every LOD found in the source is inserted into its matching
    /// `{base}_lod_X_Y` table (which must already exist).
    pub lod: Option<String>,
}

/// Request body for updating an object
#[derive(Debug, Deserialize)]
pub struct UpdateRequest {
    /// CityJSON data as a JSON string
    pub cityjson_data: String,
}

/// Request body for exporting a table
#[derive(Debug, Deserialize)]
pub struct ExportRequest {
    /// Output file path
    pub output_path: String,
    /// Export format
    pub format: ExportFormat,
}

/// Query parameters for querying objects
#[derive(Debug, Deserialize)]
pub struct QueryParams {
    /// Optional SQL WHERE clause filter
    pub filter: Option<String>,
    /// Maximum number of rows to return
    pub limit: Option<usize>,
    /// Offset for pagination
    pub offset: Option<usize>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_city_lake_config_default() {
        let config = CityLakeConfig::default();
        assert_eq!(config.storage_path, "data");
        assert_eq!(config.catalog_path, "metadata.ducklake");
        assert!(!config.auto_compact);
        assert_eq!(config.host, "127.0.0.1");
        assert_eq!(config.port, 3000);
    }

    #[test]
    fn test_compaction_threshold_default() {
        let threshold = CompactionThreshold::default();
        assert_eq!(threshold.min_file_count, 10);
        assert!((threshold.fragmentation_ratio - 0.3).abs() < f64::EPSILON);
    }

    #[test]
    fn test_export_format_duckdb_format() {
        assert_eq!(ExportFormat::CityJson.as_duckdb_format(), "cityjson");
        assert_eq!(ExportFormat::CityJsonSeq.as_duckdb_format(), "cityjsonseq");
        assert_eq!(ExportFormat::FlatCityBuf.as_duckdb_format(), "flatcitybuf");
    }

    #[test]
    fn test_export_format_file_extension() {
        assert_eq!(ExportFormat::CityJson.file_extension(), ".city.json");
        assert_eq!(ExportFormat::CityJsonSeq.file_extension(), ".city.jsonl");
        assert_eq!(ExportFormat::FlatCityBuf.file_extension(), ".fcb");
    }

    #[test]
    fn test_export_format_display() {
        assert_eq!(format!("{}", ExportFormat::CityJson), "cityjson");
        assert_eq!(format!("{}", ExportFormat::CityJsonSeq), "cityjsonseq");
        assert_eq!(format!("{}", ExportFormat::FlatCityBuf), "flatcitybuf");
    }

    #[test]
    fn test_input_format_from_path() {
        assert!(matches!(InputFormat::from_path("test.city.json"), Some(InputFormat::CityJson)));
        assert!(matches!(InputFormat::from_path("test.cityjson"), Some(InputFormat::CityJson)));
        assert!(matches!(InputFormat::from_path("test.city.jsonl"), Some(InputFormat::CityJsonSeq)));
        assert!(matches!(InputFormat::from_path("test.cityjsonl"), Some(InputFormat::CityJsonSeq)));
        assert!(matches!(InputFormat::from_path("test.jsonl"), Some(InputFormat::CityJsonSeq)));
        assert!(matches!(InputFormat::from_path("test.fcb"), Some(InputFormat::FlatCityBuf)));
        assert!(matches!(InputFormat::from_path("test.flatcitybuf"), Some(InputFormat::FlatCityBuf)));
        assert!(InputFormat::from_path("test.csv").is_none());
        assert!(InputFormat::from_path("test.json").is_none());
    }

    #[test]
    fn test_input_format_read_function() {
        assert_eq!(InputFormat::CityJson.read_function(), "read_cityjson");
        assert_eq!(InputFormat::CityJsonSeq.read_function(), "read_cityjsonseq");
        assert_eq!(InputFormat::FlatCityBuf.read_function(), "read_flatcitybuf");
    }

    #[test]
    fn test_input_format_metadata_function() {
        assert_eq!(InputFormat::CityJson.metadata_function(), Some("cityjson_metadata"));
        assert_eq!(InputFormat::CityJsonSeq.metadata_function(), Some("cityjsonseq_metadata"));
        assert_eq!(InputFormat::FlatCityBuf.metadata_function(), None);
    }

    #[test]
    fn test_lod_key_parse_valid() {
        assert_eq!(LodKey::parse("2.2").unwrap().as_str(), "2.2");
        assert_eq!(LodKey::parse("1").unwrap().as_str(), "1");
        assert_eq!(LodKey::parse("0.0").unwrap().as_str(), "0.0");
        assert_eq!(LodKey::parse("12.34").unwrap().as_str(), "12.34");
    }

    #[test]
    fn test_lod_key_parse_invalid() {
        assert!(LodKey::parse("").is_err());
        assert!(LodKey::parse("2.2.2").is_err());
        assert!(LodKey::parse(".2").is_err());
        assert!(LodKey::parse("2.").is_err());
        assert!(LodKey::parse("2,2").is_err());
        assert!(LodKey::parse("2'; DROP TABLE x; --").is_err());
        assert!(LodKey::parse("a").is_err());
    }

    #[test]
    fn test_lod_key_as_suffix() {
        assert_eq!(LodKey::parse("2.2").unwrap().as_suffix(), "lod_2_2");
        assert_eq!(LodKey::parse("1").unwrap().as_suffix(), "lod_1");
        assert_eq!(LodKey::parse("0.0").unwrap().as_suffix(), "lod_0_0");
    }

    #[test]
    fn test_lod_key_canonicalization() {
        // Leading zeros are stripped so semantically equivalent LODs collapse.
        assert_eq!(LodKey::parse("02.2").unwrap(), LodKey::parse("2.2").unwrap());
        assert_eq!(LodKey::parse("2.02").unwrap().as_str(), "2.2");
        assert_eq!(LodKey::parse("002.020").unwrap().as_str(), "2.20");
        // A lone zero stays as "0".
        assert_eq!(LodKey::parse("0").unwrap().as_str(), "0");
        assert_eq!(LodKey::parse("00").unwrap().as_str(), "0");
        assert_eq!(LodKey::parse("0.0").unwrap().as_str(), "0.0");
        assert_eq!(LodKey::parse("00.00").unwrap().as_str(), "0.0");
    }
}
