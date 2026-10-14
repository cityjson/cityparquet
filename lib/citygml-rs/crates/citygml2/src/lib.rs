//! CityGML 2.0 reader.
//!
//! [`FeatureReader`] streams a CityGML 2.0 document one top-level city object
//! at a time, as a version-neutral [`citygml_core::GmlObject`]. With the
//! `cjseq` feature, [`FeatureReader::features`] turns each into a
//! `cjseq::CityJSONFeature` quantised against a caller-chosen transform.
//! [`scan_preamble`] reads the document's declared CRS and extent, from which
//! a caller chooses that transform.
//!
//! The reader reads `bldg:Building` (with its parts, installations and
//! boundary surfaces) and the 1st-level objects of the other modules it maps;
//! [`FeatureReader::skipped_members`] reports the members it passes over.

pub mod ns;
mod reader;

pub use reader::FeatureReader;
#[cfg(feature = "cjseq")]
pub use reader::Features;
pub use reader::document::{Preamble, scan_preamble};
