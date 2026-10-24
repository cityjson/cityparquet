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
pub use citygml_core::convert::cjseq::{Report, WriterOptions};
pub use citygml_core::write::document::{Ade, ContentOf, Head, RESERVED_PREFIXES};
#[cfg(feature = "cjseq")]
pub use cityjson::Writer;
pub use document::DocumentWriter;
pub use schema::Vocabulary2;
