//! CityParquet reader/writer: the native Parquet path for CityJSON models.

mod address;
pub mod appearance;
pub mod citygml;
pub mod compare;
pub mod decode;
pub mod encode;
pub mod export;
mod geometry_properties;
pub mod inputs;
pub mod lod0;
pub mod merge;
pub mod order;
pub mod package;
pub mod partition;
pub mod query;
#[cfg(feature = "object-store")]
pub mod query_async;
pub mod reader;
pub mod recipe;
pub mod scan;
pub mod sidecar;
pub mod source;
pub mod stac;
pub mod wkb_read;
pub mod wkb_write;

pub use cityparquet_schema::{self as schema, CityParquetError, Result};
pub use cjseq;
