//! CityParquet type system: the CityParquet specification as code.
//! Pure schema/metadata layer — no buffers, no parquet.

pub mod attributes;
pub mod error;
pub mod metadata;
pub mod model;
pub mod profile;
pub mod types;

pub use attributes::{AttributeInferer, AttributeType};
pub use error::{CityParquetError, Result};
pub use metadata::{CITYPARQUET_VERSION, CityParquetMetadata, SourceFormat};
pub use model::{CityParquetSchema, normalise_attribute_name};
pub use profile::Profile;
pub use types::{CityGmlModule, ClassInfo, Lod, TAXONOMY, class_info, is_extension_type};
