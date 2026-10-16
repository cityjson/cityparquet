//! A reader's objects as `cjseq::CityJSONFeature`s.

use cjseq::{CityJSONFeature, Transform};

use super::ObjectStream;
use crate::Result;
use crate::convert::FromGml;
use crate::convert::cjseq::FeatureContext;

/// The objects of `reader` as `cjseq::CityJSONFeature`s, each with its own
/// vertex pool quantised against a transform.
pub struct Features<R> {
    reader: R,
    transform: Transform,
}

impl<R: ObjectStream> Features<R> {
    pub fn new(reader: R, transform: Transform) -> Self {
        Self { reader, transform }
    }

    /// The reader underneath, for what it reports while streaming.
    pub fn reader(&self) -> &R {
        &self.reader
    }
}

impl<R: ObjectStream> Iterator for Features<R> {
    type Item = Result<CityJSONFeature>;

    fn next(&mut self) -> Option<Self::Item> {
        let object = match self.reader.next()? {
            Ok(object) => object,
            Err(e) => return Some(Err(e)),
        };
        let context = FeatureContext {
            transform: self.transform.clone(),
            index: self.reader.emitted_members(),
            ade: self.reader.document().ade().clone(),
        };
        Some(CityJSONFeature::from_gml(&object, &context))
    }
}
