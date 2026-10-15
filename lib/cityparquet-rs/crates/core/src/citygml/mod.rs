//! CityGML input and output, through the `citygml` crate of
//! `lib/citygml-rs`: it sniffs the version, builds the CityJSON header (the
//! CRS and the quantisation transform, [`parse_header`]) and reads the
//! version's feature stream ([`FeatureReader`]); this module gives them
//! this crate's errors. Writing CityGML is [`crate::export`]'s, through the
//! same crate's [`citygml::Writer`].

pub mod reader;

pub use citygml::crs;
pub use citygml::{
    Document, Version as CityGmlVersion, sniff as sniff_citygml, sniff_from as sniff_citygml_from,
};
pub use reader::{FeatureReader, parse_header, parse_header_from, parse_header_with, read_document};
