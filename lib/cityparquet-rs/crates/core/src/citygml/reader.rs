//! The CityGML 2.0 feature stream, read by the `citygml2` crate: one
//! `cjseq::CityJSONFeature` per top-level city object, quantised against the
//! transform [`super::parse_header`] chose.

use std::collections::BTreeMap;
use std::io::BufRead;
use std::path::Path;
use std::sync::Arc;

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

/// Streams the features of a CityGML 2.0 document. See
/// [`citygml2::FeatureReader`] for what it reads and skips.
pub struct FeatureReader {
    inner: citygml2::Features,
}

impl FeatureReader {
    /// Open `path`, with the CityModel-level appearance (two passes over the
    /// file unless it has none).
    pub fn open(path: &Path, transform: &Transform) -> Result<Self> {
        let reader = citygml2::FeatureReader::open(path).map_err(error)?;
        Ok(Self::wrap(reader, transform))
    }

    /// Open `path` without the CityModel-level appearance: one pass.
    /// Features differ from [`Self::open`]'s only in that appearance. Pinned
    /// by `tests/citygml_reader_profile.rs::open_without_appearance_changes_only_the_appearance`.
    pub fn open_without_appearance(path: &Path, transform: &Transform) -> Result<Self> {
        let reader = citygml2::FeatureReader::open_without_appearance(path).map_err(error)?;
        Ok(Self::wrap(reader, transform))
    }

    /// Stream from any `BufRead` positioned at the document's first byte (an
    /// HTTP body, say), in one pass and without the CityModel-level
    /// appearance. A caller that ran [`super::sniff_citygml_from`] or
    /// [`super::parse_header_from`] on the same stream replays the bytes they
    /// consumed ahead of the rest.
    pub fn from_reader(input: Box<dyn BufRead + Send>, transform: &Transform) -> Result<Self> {
        Ok(Self::wrap(
            citygml2::FeatureReader::from_reader(input),
            transform,
        ))
    }

    /// Open `path` with the document-wide facts read beforehand by
    /// [`citygml2::Document::read`]: one pass over the file.
    pub fn with_document(
        path: &Path,
        transform: &Transform,
        document: Arc<citygml2::Document>,
    ) -> Result<Self> {
        let reader = citygml2::FeatureReader::with_document(path, document).map_err(error)?;
        Ok(Self::wrap(reader, transform))
    }

    fn wrap(reader: citygml2::FeatureReader, transform: &Transform) -> Self {
        Self {
            inner: reader.features(transform.clone()),
        }
    }

    /// Every `cityObjectMember` object of a class the reader does not read,
    /// by document-spelled name (`tran:Track` -> 3), so far.
    pub fn skipped_members(&self) -> &BTreeMap<String, usize> {
        self.inner.reader().skipped_members()
    }

    /// Every `cityObjectMember` object of an ADE's class, by
    /// document-spelled name (`urf:UseDistrict` -> 3), so far.
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
