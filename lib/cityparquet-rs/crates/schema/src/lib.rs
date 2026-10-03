//! CityParquet type system: the CityParquet specification as code.
//! Pure schema/metadata layer — no buffers, no parquet.

pub mod attributes;
pub mod crs;
pub mod error;
pub mod extensions;
pub mod metadata;
pub mod model;
pub mod sidecar_schemas;
pub mod types;

pub use attributes::{AttributeInferer, AttributeType};
pub use error::{CityParquetError, Result};
pub use extensions::ExtensionNaming;
pub use metadata::{
    CITYPARQUET_VERSION, CityColumnEntry, CityMetadata, CrsState, ExtensionDeclaration,
    ExtensionDeclarations, GEOPARQUET_VERSION, GeoColumnEntry, GeoMetadata, Orientation3d,
    SourceFormat,
};
pub use model::CityParquetSchema;
pub use types::{
    CityGmlModule, ClassInfo, ExtensionClassDecl, ExtensionRegistry, GeometryEncoding, Lod,
    ModuleKey, ModuleKeyResolver, TAXONOMY, cityjson_type_for_citygml_class, class_info,
    geometry_column_name, is_extension_type, module_file, resolve_module_key, strip_plus,
};
