use serde::{Deserialize, Serialize};
use std::fmt;

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

/// Request body for creating a table
#[derive(Debug, Deserialize)]
pub struct CreateTableRequest {
    /// Path to a CityJSON source file on the server
    pub source_path: Option<String>,
}

/// Request body for inserting objects
#[derive(Debug, Deserialize)]
pub struct InsertRequest {
    /// Path to a CityJSON source file on the server
    pub source_path: Option<String>,
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
