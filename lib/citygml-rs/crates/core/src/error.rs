/// What can go wrong reading or converting CityGML.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The document is malformed, or holds something these crates cannot
    /// represent. The message names the object where one is known.
    #[error("{0}")]
    Invalid(String),
    /// The input could not be read.
    #[error("{message}")]
    Io {
        message: String,
        #[source]
        source: std::io::Error,
    },
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::Invalid(message.into())
    }

    /// An I/O failure, with what was being done when it happened.
    pub fn io(message: impl Into<String>, source: std::io::Error) -> Self {
        Self::Io {
            message: message.into(),
            source,
        }
    }
}
