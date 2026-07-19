//! CityParquet type system: the CityParquet specification as code.
//! Pure schema/metadata layer — no buffers, no parquet.

pub mod attributes;
pub mod crs;
pub mod error;
pub mod metadata;
pub mod model;
pub mod profile;
pub mod types;

pub use attributes::{AttributeInferer, AttributeType};
pub use error::{CityParquetError, Result};
pub use metadata::{CITYPARQUET_VERSION, CityParquetMetadata, SourceFormat};
pub use model::{CityParquetSchema, normalise_attribute_name};
pub use profile::{PackageManifest, Profile};
pub use types::{
    CityGmlModule, ClassInfo, Lod, TAXONOMY, class_info, first_level_type, footprint_lod,
    geometry_column_name, is_extension_type,
};
