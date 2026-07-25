//! CityParquet type system: the CityParquet specification as code.
//! Pure schema/metadata layer — no buffers, no parquet.

pub mod attributes;
pub mod crs;
pub mod error;
pub mod metadata;
pub mod model;
pub mod sidecar_schemas;
pub mod types;

pub use attributes::{AttributeInferer, AttributeType};
pub use error::{CityParquetError, Result};
pub use metadata::{
    CITYPARQUET_VERSION, CityColumnEntry, CityMetadata, GEOPARQUET_VERSION, GeoColumnEntry,
    GeoMetadata, Orientation3d, SourceFormat,
};
pub use model::{CityParquetSchema, normalise_attribute_name};
pub use types::{
    CityGmlModule, ClassInfo, ExtensionClassDecl, ExtensionRegistry, GeometryEncoding, Lod,
    ModuleKey, ModuleKeyResolver, TAXONOMY, cityjson_type_for_citygml_class, class_info,
    first_level_type, geometry_column_name, is_extension_type, module_file, resolve_module_key,
    strip_plus,
};
