//! What the per-version readers share: the parts of reading a CityGML
//! document that both versions spell alike, or that take the few names that
//! differ as parameters.
//!
//! - [`preamble`]: the CRS and envelope ahead of the first object;
//! - [`scan`]: byte searches that let a reader skip its up-front pass;
//! - [`document`]: what that pass finds ([`Document`]);
//! - [`templates`]: the relative geometries `ImplicitGeometry`s place;
//! - [`geometry`]: `lodN…` geometry properties and `ImplicitGeometry`;
//! - [`appearance`]: `app:Appearance`, in either version's spelling;
//! - [`text`] and [`node`]: a property's text, typed, or an element kept
//!   verbatim;
//! - [`ObjectStream`]: what a per-version reader yields;
//! - with the `cjseq` feature, [`Features`]: a reader's objects as CityJSON
//!   features.

pub mod appearance;
pub mod document;
#[cfg(feature = "cjseq")]
mod features;
pub mod geometry;
pub mod node;
pub mod preamble;
pub mod scan;
mod stream;
pub mod templates;
pub mod text;

pub use document::Document;
#[cfg(feature = "cjseq")]
pub use features::Features;
pub use preamble::{Preamble, scan_preamble};
pub use stream::ObjectStream;
