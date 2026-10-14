//! The CityGML feature stream, read by the reader of the document's
//! version: one `cjseq::CityJSONFeature` per top-level city object,
//! quantised against the transform [`super::parse_header`] chose.

use std::collections::BTreeMap;
use std::io::BufRead;
use std::path::Path;
use std::sync::Arc;

use citygml_core::Version as CityGmlVersion;
use citygml_core::reader::{Document, Features, ObjectStream};
use cityparquet_schema::{CityParquetError, Result};
use cjseq::{CityJSONFeature, Transform};

/// A `citygml-core` error as this crate's.
pub(crate) fn error(e: citygml_core::Error) -> CityParquetError {
    match e {
        citygml_core::Error::Invalid(message) => CityParquetError::Schema(message),
        citygml_core::Error::Io { message, source } => CityParquetError::io_source(message, source),
        other => CityParquetError::Schema(other.to_string()),
    }
}

/// Streams the features of a CityGML document, with the reader of its
/// version (see [`citygml2::FeatureReader`] for what it reads and skips).
pub struct FeatureReader {
    inner: Features<Box<dyn ObjectStream + Send>>,
}

/// A CityGML version no reader reads, as an error.
pub(crate) fn unsupported(version: &CityGmlVersion) -> CityParquetError {
    let version = match version {
        CityGmlVersion::V2_0 => "2.0",
        CityGmlVersion::V3_0 => "3.0",
        CityGmlVersion::Other(version) => version,
    };
    CityParquetError::Schema(format!(
        "unsupported CityGML version {version} (only CityGML 2.0 is supported)"
    ))
}

/// The version of the CityGML document at `path`.
fn version(path: &Path) -> Result<CityGmlVersion> {
    citygml_core::sniff(path).ok_or_else(|| {
        CityParquetError::Schema(format!("{} is not a CityGML document", path.display()))
    })
}

/// Read the document at `path`, of `version`, once for what its objects
/// need from elsewhere in it and what a CityJSON header declares: the
/// CityModel-level appearance, the groups' members, the relative geometries
/// its `ImplicitGeometry`s place and the ADEs it uses.
pub fn read_document(path: &Path, version: &CityGmlVersion) -> Result<Document> {
    match version {
        CityGmlVersion::V2_0 => citygml2::read_document(path).map_err(error),
        other => Err(unsupported(other)),
    }
}

impl FeatureReader {
    /// Open `path`, with the CityModel-level appearance (two passes over the
    /// file unless it has none).
    pub fn open(path: &Path, transform: &Transform) -> Result<Self> {
        let version = version(path)?;
        let document = read_document(path, &version)?;
        Self::with_document(path, &version, transform, Arc::new(document))
    }

    /// Open `path` without the CityModel-level appearance: one pass.
    /// Features differ from [`Self::open`]'s only in that appearance. Pinned
    /// by `tests/citygml_reader_profile.rs::open_without_appearance_changes_only_the_appearance`.
    pub fn open_without_appearance(path: &Path, transform: &Transform) -> Result<Self> {
        let version = version(path)?;
        let document = match &version {
            CityGmlVersion::V2_0 => {
                citygml2::read_document_without_appearance(path).map_err(error)?
            }
            other => return Err(unsupported(other)),
        };
        Self::with_document(path, &version, transform, Arc::new(document))
    }

    /// Stream a document of `version` from any `BufRead` positioned at its
    /// first byte (an HTTP body, say), in one pass and without the
    /// CityModel-level appearance. A caller that ran
    /// [`super::sniff_citygml_from`] or [`super::parse_header_from`] on the
    /// same stream replays the bytes they consumed ahead of the rest.
    pub fn from_reader(
        input: Box<dyn BufRead + Send>,
        version: &CityGmlVersion,
        transform: &Transform,
    ) -> Result<Self> {
        let objects: Box<dyn ObjectStream + Send> = match version {
            CityGmlVersion::V2_0 => Box::new(citygml2::FeatureReader::from_reader(input)),
            other => return Err(unsupported(other)),
        };
        Ok(Self::wrap(objects, transform))
    }

    /// Open `path`, of `version`, with the document-wide facts read
    /// beforehand by [`read_document`]: one pass over the file.
    pub fn with_document(
        path: &Path,
        version: &CityGmlVersion,
        transform: &Transform,
        document: Arc<Document>,
    ) -> Result<Self> {
        let objects: Box<dyn ObjectStream + Send> = match version {
            CityGmlVersion::V2_0 => {
                Box::new(citygml2::FeatureReader::with_document(path, document).map_err(error)?)
            }
            other => return Err(unsupported(other)),
        };
        Ok(Self::wrap(objects, transform))
    }

    fn wrap(objects: Box<dyn ObjectStream + Send>, transform: &Transform) -> Self {
        Self {
            inner: Features::new(objects, transform.clone()),
        }
    }

    /// Every member object of a class the reader does not read, by
    /// document-spelled name (`tran:Track` -> 3), so far.
    pub fn skipped_members(&self) -> &BTreeMap<String, usize> {
        self.inner.reader().skipped_members()
    }

    /// Every member object of an ADE's class, by document-spelled name
    /// (`urf:UseDistrict` -> 3), so far.
    pub fn ade_members(&self) -> &BTreeMap<String, usize> {
        self.inner.reader().ade_members()
    }

    /// How many features have been emitted so far.
    pub fn emitted_members(&self) -> usize {
        self.inner.reader().emitted_members()
    }
}

impl Iterator for FeatureReader {
    type Item = Result<CityJSONFeature>;

    fn next(&mut self) -> Option<Self::Item> {
        Some(self.inner.next()?.map_err(error))
    }
}
