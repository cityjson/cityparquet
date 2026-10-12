use arrow_schema::ArrowError;

/// Error type shared across the cityparquet crate family.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CityParquetError {
    #[error(transparent)]
    Arrow(#[from] ArrowError),
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
    #[error("io error: {message}")]
    Io {
        message: String,
        /// The underlying OS/filesystem error, when one exists — a real
        /// `#[source]` so error chains keep the errno/kind instead of a
        /// flattened string (review P7).
        #[source]
        source: Option<std::io::Error>,
    },
    #[error("geometry error: {0}")]
    Geometry(String),
    #[error("parquet error: {message}")]
    Parquet {
        message: String,
        /// Boxed because this crate must stay free of the `parquet` crate
        /// (`just isolation`); at every current construction site the
        /// concrete type is `parquet::errors::ParquetError` or the reader's
        /// `ArrowError`.
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync + 'static>>,
    },
}

impl CityParquetError {
    /// A message-only io error (no OS-level source exists, e.g. "no input
    /// files resolved").
    pub fn io(message: impl Into<String>) -> Self {
        Self::Io {
            message: message.into(),
            source: None,
        }
    }

    /// An io error that keeps its `std::io::Error` on the chain; `message`
    /// carries the caller's context ("cannot open <path>"), the source
    /// carries the errno/kind.
    pub fn io_source(message: impl Into<String>, source: std::io::Error) -> Self {
        Self::Io {
            message: message.into(),
            source: Some(source),
        }
    }

    /// A message-only parquet error.
    pub fn parquet(message: impl Into<String>) -> Self {
        Self::Parquet {
            message: message.into(),
            source: None,
        }
    }

    /// A parquet error that keeps its source on the chain.
    pub fn parquet_source(
        message: impl Into<String>,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self::Parquet {
            message: message.into(),
            source: Some(Box::new(source)),
        }
    }

    /// A parquet error with no extra context: the message IS the source's
    /// Display (kept so `{0}`-era call sites lose nothing), and the source
    /// still rides the chain.
    pub fn parquet_from(source: impl std::error::Error + Send + Sync + 'static) -> Self {
        let message = source.to_string();
        Self::Parquet {
            message,
            source: Some(Box::new(source)),
        }
    }
}

impl From<std::io::Error> for CityParquetError {
    fn from(source: std::io::Error) -> Self {
        let message = source.to_string();
        Self::Io {
            message,
            source: Some(source),
        }
    }
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

    /// Every source-carrying constructor must keep its `#[source]` on the
    /// chain and surface its message in Display. This guards the review-P7
    /// fix: a dropped `#[source]` attribute would silently flatten the chain.
    #[test]
    fn source_carrying_constructors_keep_their_source() {
        let cases: Vec<(&str, CityParquetError)> = vec![
            (
                "cannot open /tmp/x",
                CityParquetError::io_source(
                    "cannot open /tmp/x",
                    std::io::Error::new(std::io::ErrorKind::NotFound, "gone"),
                ),
            ),
            (
                "cannot open parquet reader",
                CityParquetError::parquet_source(
                    "cannot open parquet reader",
                    std::io::Error::other("inner parquet failure"),
                ),
            ),
            (
                "disk fell off",
                std::io::Error::other("disk fell off").into(),
            ),
            (
                "column not found",
                CityParquetError::parquet_from(std::io::Error::other("column not found")),
            ),
        ];
        for (message, e) in cases {
            assert!(
                e.to_string().contains(message),
                "Display must contain {message:?}, got: {e}"
            );
            assert!(
                std::error::Error::source(&e).is_some(),
                "a source-carrying constructor must keep its #[source]: {e}"
            );
        }
    }
}
