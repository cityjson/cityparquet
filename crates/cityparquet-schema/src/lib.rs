//! CityParquet type system: the CityParquet specification as code.
//! Pure schema/metadata layer — no buffers, no parquet.

pub mod attributes;
pub mod error;
pub mod types;

pub use error::{CityParquetError, Result};
