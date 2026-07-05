//! CityParquet reader/writer: the native Parquet path for CityJSON models.

pub mod decode;
pub mod encode;
pub mod package;
pub mod reader;
pub mod recipe;
pub mod scan;
pub mod source;
pub mod wkb_read;
pub mod wkb_write;

pub use cityparquet_schema::{self as schema, CityParquetError, Result};
pub use cjseq;
