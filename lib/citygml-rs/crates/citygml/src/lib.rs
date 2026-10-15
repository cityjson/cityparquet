//! CityGML 2.0 and 3.0 to and from CityJSON, whichever version a document is.
//!
//! The reader and writer of each version are its own crate's (`citygml2`,
//! `citygml3`); this crate picks the one a document's version needs:
//!
//! - [`sniff`] tells which version a document is;
//! - [`open`] reads a document's CityJSON header and opens its features,
//!   with one pass over it beforehand (what [`header`] and
//!   [`FeatureReader::open`] would each make);
//! - [`header`] builds its CityJSON header: the transform every feature is
//!   quantised against, chosen from the CRS and envelope it declares, its
//!   reference system and extent, its implicit geometries' templates and its
//!   ADEs' Extensions;
//! - [`FeatureReader`] streams its city objects as `cjseq::CityJSONFeature`s;
//! - [`Writer`] writes CityJSON features as a document of the version it is
//!   given.
//!
//! The CityJSON types are those of the `cjseq` this crate re-exports; a
//! tool that names them uses it, or pins the same version, so that it does
//! not build a second, incompatible copy.

use std::io::{BufRead, Write};
use std::path::Path;
use std::sync::Arc;

use citygml_core::reader::{Features, ObjectStream};
use cjseq::{CityJSON, CityJSONFeature, Transform};

pub use cjseq;

pub use citygml_core::convert::cjseq::{Report, WriterOptions, header_from, header_with};
pub use citygml_core::crs;
pub use citygml_core::reader::{Document, Preamble, scan_preamble};
pub use citygml_core::{Error, Result, Version, sniff, sniff_from};

/// A version no reader or writer here handles, as an error.
fn unsupported(version: &Version) -> Error {
    let version = match version {
        Version::V2_0 => "2.0",
        Version::V3_0 => "3.0",
        Version::Other(version) => version,
    };
    Error::invalid(format!(
        "unsupported CityGML version {version} (only CityGML 2.0 and 3.0 are supported)"
    ))
}

/// The version of the document at `path`, or an error when it is not
/// CityGML.
fn version_of(path: &Path) -> Result<Version> {
    sniff(path)
        .ok_or_else(|| Error::invalid(format!("{} is not a CityGML document", path.display())))
}

/// Read the document at `path`, of `version`, once for what its objects
/// need from elsewhere in it and what a CityJSON header declares: the
/// CityModel-level appearance, the groups' members, the relative geometries
/// its `ImplicitGeometry`s place and the ADEs it uses.
pub fn read_document(path: &Path, version: &Version) -> Result<Document> {
    match version {
        Version::V2_0 => citygml2::read_document(path),
        Version::V3_0 => citygml3::read_document(path),
        other => Err(unsupported(other)),
    }
}

/// [`read_document`] without the CityModel-level appearance and the ADE
/// index, for [`FeatureReader::open_without_appearance`].
pub fn read_document_without_appearance(path: &Path, version: &Version) -> Result<Document> {
    match version {
        Version::V2_0 => citygml2::read_document_without_appearance(path),
        Version::V3_0 => citygml3::read_document_without_appearance(path),
        other => Err(unsupported(other)),
    }
}

/// The CityJSON header of the document at `path` (see [`header_with`]),
/// after a pass over the whole document for its templates and ADEs.
pub fn header(path: &Path) -> Result<CityJSON> {
    header_with(path, &read_document(path, &version_of(path)?)?)
}

/// The CityJSON header of the document at `path` and its features,
/// quantised against the header's transform, with one pass over the
/// document before the features stream.
pub fn open(path: &Path) -> Result<(CityJSON, FeatureReader)> {
    let version = version_of(path)?;
    let document = read_document(path, &version)?;
    let header = header_with(path, &document)?;
    let features =
        FeatureReader::with_document(path, &version, &header.transform, Arc::new(document))?;
    Ok((header, features))
}

/// The city objects of a CityGML document as CityJSON features, read by the
/// reader of the document's version, one top-level object (with its parts,
/// rooms and other children) per feature. It is `Send`, so it can stream
/// on another thread.
pub struct FeatureReader {
    inner: Features<Box<dyn ObjectStream + Send>>,
}

impl FeatureReader {
    /// Open `path`, with the CityModel-level appearance (two passes over the
    /// file unless it has none).
    pub fn open(path: &Path, transform: &Transform) -> Result<Self> {
        let version = version_of(path)?;
        let document = read_document(path, &version)?;
        Self::with_document(path, &version, transform, Arc::new(document))
    }

    /// Open `path` without the CityModel-level appearance: one pass.
    pub fn open_without_appearance(path: &Path, transform: &Transform) -> Result<Self> {
        let version = version_of(path)?;
        let document = read_document_without_appearance(path, &version)?;
        Self::with_document(path, &version, transform, Arc::new(document))
    }

    /// Open `path`, of `version`, with what [`read_document`] read from it
    /// beforehand: one pass over the file.
    pub fn with_document(
        path: &Path,
        version: &Version,
        transform: &Transform,
        document: Arc<Document>,
    ) -> Result<Self> {
        let objects: Box<dyn ObjectStream + Send> = match version {
            Version::V2_0 => Box::new(citygml2::FeatureReader::with_document(path, document)?),
            Version::V3_0 => Box::new(citygml3::FeatureReader::with_document(path, document)?),
            other => return Err(unsupported(other)),
        };
        Ok(Self::wrap(objects, transform))
    }

    /// Stream a document of `version` from any `BufRead` positioned at its
    /// first byte, in one pass and without the CityModel-level appearance,
    /// implicit geometries and the groups that name an object only by
    /// reference. A caller that ran [`sniff_from`] or [`header_from`] on the
    /// same stream replays the bytes they consumed ahead of the rest.
    pub fn from_reader(
        input: Box<dyn BufRead + Send>,
        version: &Version,
        transform: &Transform,
    ) -> Result<Self> {
        let objects: Box<dyn ObjectStream + Send> = match version {
            Version::V2_0 => Box::new(citygml2::FeatureReader::from_reader(input)),
            Version::V3_0 => Box::new(citygml3::FeatureReader::from_reader(input)),
            other => return Err(unsupported(other)),
        };
        Ok(Self::wrap(objects, transform))
    }

    fn wrap(objects: Box<dyn ObjectStream + Send>, transform: &Transform) -> Self {
        Self {
            inner: Features::new(objects, transform.clone()),
        }
    }

    /// The version's reader: what it has emitted, skipped and read of an
    /// ADE so far.
    pub fn objects(&self) -> &(dyn ObjectStream + Send) {
        self.inner.reader().as_ref()
    }
}

impl Iterator for FeatureReader {
    type Item = Result<CityJSONFeature>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }
}

/// CityJSON features written as a CityGML document of the version it was
/// started with, one feature at a time, in any order.
pub struct Writer<W: Write>(Versioned<W>);

enum Versioned<W: Write> {
    V2_0(citygml2::writer::Writer<W>),
    V3_0(citygml3::writer::Writer<W>),
}

impl<W: Write> Writer<W> {
    /// Start a document of `version` on `out` for the features of `header`.
    pub fn new(
        version: &Version,
        out: W,
        header: &CityJSON,
        options: WriterOptions,
    ) -> Result<Self> {
        match version {
            Version::V2_0 => citygml2::writer::Writer::new(out, header, options)
                .map(|w| Self(Versioned::V2_0(w))),
            Version::V3_0 => citygml3::writer::Writer::new(out, header, options)
                .map(|w| Self(Versioned::V3_0(w))),
            other => Err(unsupported(other)),
        }
    }

    /// Write `feature`'s objects.
    pub fn write_feature(&mut self, feature: &CityJSONFeature) -> Result<()> {
        match &mut self.0 {
            Versioned::V2_0(w) => w.write_feature(feature),
            Versioned::V3_0(w) => w.write_feature(feature),
        }
    }

    /// End the document. Gives back what it was written to, and what it
    /// holds and left out.
    pub fn finish(self) -> Result<(W, Report)> {
        match self.0 {
            Versioned::V2_0(w) => w.finish(),
            Versioned::V3_0(w) => w.finish(),
        }
    }
}
