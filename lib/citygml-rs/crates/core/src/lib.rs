//! The version-neutral half of the CityGML crates.
//!
//! A per-version reader (`citygml2`, `citygml3`) walks the XML and fills
//! a [`GmlObject`] for each top-level city object. Nothing in the model names
//! a target format: [`convert::FromGml`] turns it into one, and the `cjseq`
//! feature provides the CityJSON conversion.
//!
//! What lives here:
//! - [`sniff`]: which CityGML version a document is;
//! - [`xml`]: quick-xml helpers every reader uses;
//! - [`gml`]: GML 3.1/3.2 geometry, and the assembly of one object's geometry
//!   with its `xlink:href`s resolved ([`gml::ObjectBuilder`]);
//! - [`model`]: [`GmlObject`] and what it holds;
//! - [`reader`]: the parts of reading a document both versions share;
//! - [`vocabulary`]: CityGML class names and their CityJSON counterparts;
//! - [`xsd`]: what an ADE's XML Schema says about its elements;
//! - [`convert`]: the conversions out of the model.

pub mod convert;
mod error;
pub mod gml;
pub mod model;
pub mod reader;
pub mod sniff;
pub mod vocabulary;
pub mod xml;
pub mod xsd;

pub use error::{Error, Result};
pub use model::GmlObject;
pub use sniff::{Version, sniff, sniff_from};
