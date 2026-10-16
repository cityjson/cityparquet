//! CityParquet reader/writer: the native Parquet path for CityJSON models.

mod address;
pub mod appearance;
pub mod appearance_columns;
mod arrow_compat;
pub mod citygml;
mod cjtypes;
pub mod compare;
#[cfg(feature = "object-store")]
pub mod counting_store;
pub mod decode;
pub mod encode;
pub mod export;
mod faces;
mod family;
pub mod fcb;
mod geometry_encoding;
mod geometry_properties;
mod id_hash;
pub mod inputs;
pub mod lod0;
pub mod merge;
pub mod order;
pub mod package;
pub mod partition;
pub mod query;
#[cfg(feature = "object-store")]
pub mod query_async;
mod query_core;
pub mod reader;
pub mod recipe;
pub mod scan;
pub mod sidecar;
mod sort;
pub mod source;
pub mod stac;
pub mod validate;
pub mod variant;
pub mod visit;
pub mod wkb_read;
pub mod wkb_write;

/// The 3DCityDB v5 reader, whose [`citydb::Config`] [`source::Source::open_citydb`]
/// takes.
#[cfg(feature = "citydb")]
pub use citygml_citydb as citydb;
pub use cityparquet_schema::{self as schema, CityParquetError, Result};
pub use cjseq;
