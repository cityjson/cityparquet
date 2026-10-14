//! CityGML input and output.
//!
//! Reading is the `lib/citygml-rs` crates': this module sniffs the version,
//! synthesises the CityJSON header (CRS and quantisation transform,
//! [`parse_header`]) and adapts the reader of that version's feature stream
//! ([`FeatureReader`]) to this crate's errors. [`writer`] writes a package
//! back out as CityGML 2.0.

pub mod crs;

mod header;
pub mod reader;
pub mod writer;

pub use citygml_core::reader::Document;
pub use citygml_core::{
    Version as CityGmlVersion, sniff as sniff_citygml, sniff_from as sniff_citygml_from,
};
pub use header::{parse_header, parse_header_from, parse_header_with};
pub use reader::{FeatureReader, read_document};
