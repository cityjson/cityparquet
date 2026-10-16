//! The CityGML feature stream and header, with this crate's errors.

use std::collections::BTreeMap;
use std::io::BufRead;
use std::path::Path;
use std::sync::Arc;

use citygml::{Document, Version};
use cityparquet_schema::{CityParquetError, Result};
use cjseq::{CityJSON, CityJSONFeature, Transform};

/// A `citygml` error as this crate's.
pub(crate) fn error(e: citygml::Error) -> CityParquetError {
    match e {
        citygml::Error::Invalid(message) => CityParquetError::Schema(message),
        citygml::Error::Io { message, source } => CityParquetError::io_source(message, source),
        other => CityParquetError::Schema(other.to_string()),
    }
}

/// The CityJSON header of the document at `path`: transform and metadata,
/// the relative geometries its `ImplicitGeometry`s place as
/// `geometry-templates` with the appearance they index, and the Extensions
/// its ADEs are declared as (see [`citygml::header_with`]). Reads the whole
/// document once for the templates and ADEs; [`parse_header_with`] takes a
/// [`Document`] already read.
pub fn parse_header(path: &Path) -> Result<CityJSON> {
    citygml::header(path).map_err(error)
}

/// [`parse_header`] with the document-wide facts read beforehand, so that
/// the same [`Document`] can be handed to [`FeatureReader::with_document`].
pub fn parse_header_with(path: &Path, document: &Document) -> Result<CityJSON> {
    citygml::header_with(path, document).map_err(error)
}

/// [`parse_header`] over any `BufRead`, from the preamble alone (see
/// [`citygml::header_from`]).
pub fn parse_header_from<R: BufRead>(input: R) -> Result<CityJSON> {
    citygml::header_from(input).map_err(error)
}

/// Read the document at `path`, of `version`, once for what its objects
/// need from elsewhere in it and what a CityJSON header declares.
pub fn read_document(path: &Path, version: &Version) -> Result<Document> {
    citygml::read_document(path, version).map_err(error)
}

/// Streams the features of a CityGML document, with the reader of its
/// version (see [`citygml::FeatureReader`]).
pub struct FeatureReader(citygml::FeatureReader);

impl FeatureReader {
    /// Open `path`, with the CityModel-level appearance (two passes over the
    /// file unless it has none).
    pub fn open(path: &Path, transform: &Transform) -> Result<Self> {
        citygml::FeatureReader::open(path, transform)
            .map(Self)
            .map_err(error)
    }

    /// Open `path` without the CityModel-level appearance: one pass.
    /// Features differ from [`Self::open`]'s only in that appearance. Pinned
    /// by `tests/citygml_reader_profile.rs::open_without_appearance_changes_only_the_appearance`.
    pub fn open_without_appearance(path: &Path, transform: &Transform) -> Result<Self> {
        citygml::FeatureReader::open_without_appearance(path, transform)
            .map(Self)
            .map_err(error)
    }

    /// Stream a document of `version` from any `BufRead` positioned at its
    /// first byte (an HTTP body, say), in one pass and without the
    /// CityModel-level appearance.
    pub fn from_reader(
        input: Box<dyn BufRead + Send>,
        version: &Version,
        transform: &Transform,
    ) -> Result<Self> {
        citygml::FeatureReader::from_reader(input, version, transform)
            .map(Self)
            .map_err(error)
    }

    /// Open `path`, of `version`, with the document-wide facts read
    /// beforehand by [`read_document`]: one pass over the file.
    pub fn with_document(
        path: &Path,
        version: &Version,
        transform: &Transform,
        document: Arc<Document>,
    ) -> Result<Self> {
        citygml::FeatureReader::with_document(path, version, transform, document)
            .map(Self)
            .map_err(error)
    }

    /// Every member object of a class the reader does not read, by
    /// document-spelled name (`tran:Track` -> 3), so far.
    pub fn skipped_members(&self) -> &BTreeMap<String, usize> {
        self.0.objects().skipped_members()
    }

    /// Every member object of an ADE's class, by document-spelled name
    /// (`urf:UseDistrict` -> 3), so far.
    pub fn ade_members(&self) -> &BTreeMap<String, usize> {
        self.0.objects().ade_members()
    }

    /// How many features have been emitted so far.
    pub fn emitted_members(&self) -> usize {
        self.0.objects().emitted_members()
    }
}

impl Iterator for FeatureReader {
    type Item = Result<CityJSONFeature>;

    fn next(&mut self) -> Option<Self::Item> {
        Some(self.0.next()?.map_err(error))
    }
}
