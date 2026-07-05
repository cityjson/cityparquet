//! CityParquet reader/writer: the native Parquet path for CityJSON models.

pub mod source;

pub use cityparquet_schema::{self as schema, CityParquetError, Result};
pub use cjseq;
