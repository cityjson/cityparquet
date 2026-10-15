//! CityGML 2.0 writer.
//!
//! [`DocumentWriter`] writes [`citygml_core::GmlObject`]s as a CityGML 2.0
//! document. With the `cjseq` feature, [`Writer`] writes a CityJSON header
//! and its features, converted to the model with [`Vocabulary2`]'s names.

#[cfg(feature = "cjseq")]
mod cityjson;
mod document;
mod schema;

#[cfg(feature = "cjseq")]
pub use cityjson::{Report, Writer, WriterOptions};
pub use document::{Ade, ContentOf, DocumentWriter, Head, RESERVED_PREFIXES};
pub use schema::Vocabulary2;
