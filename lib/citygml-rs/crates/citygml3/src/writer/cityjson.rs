//! CityJSON to a CityGML 3.0 document: a header, then its features one at
//! a time, in any order.

use std::io::Write;
use std::sync::Arc;

use citygml_core::Result;
use citygml_core::convert::Losses;
use citygml_core::convert::cjseq::{Document, Report, WriterOptions};
use citygml_core::model::{GmlObject, Template};
use cjseq::{CityJSON, CityJSONFeature};

type Shared<W> = citygml_core::convert::cjseq::Writer<W, DocumentWriter<W>>;

use super::{ContentOf, DocumentWriter, Head, Vocabulary3};

/// A CityGML 3.0 document written from a CityJSON header and its features.
///
/// A feature's group links to objects of other features become
/// `xlink:href`s; the header's geometry templates are the relative
/// geometries the features' `GeometryInstance`s place, coloured by the
/// header's appearance.
pub struct Writer<W: Write>(Shared<W>);

impl<W: Write> Writer<W> {
    /// Start a document on `out` for the features of `header`.
    pub fn new(out: W, header: &CityJSON, options: WriterOptions) -> Result<Self> {
        Shared::new(out, header, options, Arc::new(Vocabulary3)).map(Self)
    }

    /// Write `feature`'s objects.
    pub fn write_feature(&mut self, feature: &CityJSONFeature) -> Result<()> {
        self.0.write_feature(feature)
    }

    /// End the document. Gives back what it was written to, and what it
    /// holds.
    pub fn finish(self) -> Result<(W, Report)> {
        self.0.finish()
    }
}

impl<W: Write> Document<W> for DocumentWriter<W> {
    fn start(out: W, head: Head, templates: Vec<Template>, content_of: ContentOf) -> Result<Self> {
        Self::new(out, head, templates, content_of)
    }

    fn write(&mut self, object: GmlObject) -> Result<()> {
        DocumentWriter::write(self, object)
    }

    fn objects(&self) -> usize {
        DocumentWriter::objects(self)
    }

    fn finish(self) -> Result<(W, Losses)> {
        DocumentWriter::finish(self)
    }
}
