//! Conversions out of the model.
//!
//! [`FromGml`] is the one contract a target model implements, so a reader
//! never learns what its objects become. The `cjseq` feature implements it
//! for [`cjseq::CityJSONFeature`].

#[cfg(feature = "cjseq")]
pub mod cjseq;

use crate::{GmlObject, Result};

/// A model that can be built from one top-level [`GmlObject`].
pub trait FromGml: Sized {
    /// What the conversion needs beyond the object itself (for CityJSON, the
    /// transform its vertices are quantised against).
    type Context;

    fn from_gml(object: &GmlObject, context: &Self::Context) -> Result<Self>;
}
