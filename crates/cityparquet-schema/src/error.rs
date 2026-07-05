use arrow_schema::ArrowError;

/// Error type shared across the cityparquet crate family.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CityParquetError {
    #[error(transparent)]
    Arrow(#[from] ArrowError),
    #[error(transparent)]
    GeoArrow(#[from] geoarrow_schema::error::GeoArrowError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("metadata error: {0}")]
    Metadata(String),
    #[error("schema error: {0}")]
    Schema(String),
    #[error("attribute inference error: {0}")]
    Attribute(String),
    #[error("invalid LoD: {0}")]
    Lod(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("geometry error: {0}")]
    Geometry(String),
    #[error("parquet error: {0}")]
    Parquet(String),
}

pub type Result<T> = std::result::Result<T, CityParquetError>;

impl From<CityParquetError> for ArrowError {
    fn from(value: CityParquetError) -> Self {
        match value {
            CityParquetError::Arrow(e) => e,
            other => ArrowError::ExternalError(Box::new(other)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow_schema::ArrowError;

    #[test]
    fn arrow_error_converts_both_ways() {
        let e: CityParquetError = ArrowError::SchemaError("bad".into()).into();
        assert!(matches!(e, CityParquetError::Arrow(_)));
        let back: ArrowError = e.into();
        assert!(back.to_string().contains("bad"));
    }

    #[test]
    fn metadata_error_displays_context() {
        let e = CityParquetError::Metadata("missing key cityparquet_version".into());
        assert!(e.to_string().contains("cityparquet_version"));
    }

    #[test]
    fn io_error_displays_context() {
        let e = CityParquetError::Io("x".into());
        assert!(e.to_string().contains("io error"));
    }
}
