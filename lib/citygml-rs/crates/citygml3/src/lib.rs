//! CityGML 3.0 reader.
//!
//! [`FeatureReader`] streams a CityGML 3.0 document one top-level city object
//! at a time, as a version-neutral [`citygml_core::GmlObject`]. With the
//! `cjseq` feature, [`FeatureReader::features`] turns each into a
//! `cjseq::CityJSONFeature` quantised against a caller-chosen transform.
//! [`scan_preamble`] reads the document's declared CRS and extent, from which
//! a caller chooses that transform. [`read_document`] passes over the whole
//! document once for what its objects need from elsewhere in it, and what a
//! CityJSON header declares: the relative geometries its
//! `ImplicitGeometry`s place (`geometry-templates`) and the ADEs it uses
//! (`extensions`).
//!
//! The reader reads the top-level objects of the modules CityJSON has types
//! for, with the parts, rooms, installations, furniture, constructive
//! elements and subdivisions they hold, and the objects of an ADE's
//! classes; [`ObjectStream::skipped_members`] reports the members it passes
//! over.

pub mod ns;
mod reader;

pub use citygml_core::reader::{Document, ObjectStream, Preamble, scan_preamble};
#[cfg(feature = "cjseq")]
pub use reader::Features;
pub use reader::{FeatureReader, read_document, read_document_without_appearance};
