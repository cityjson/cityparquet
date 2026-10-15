//! CityJSON to a CityGML document: a header, then its features one at a
//! time, in any order. What both versions' writers share; each names its
//! classes with its own [`Vocabulary`] and writes with its own document
//! writer ([`Document`]).

use std::collections::BTreeMap;
use std::io::Write;
use std::marker::PhantomData;
use std::sync::Arc;

use cjseq::{CityJSON, CityJSONFeature};

use super::{Extensions, GmlContext, template_lods, templates_to_gml};
use crate::Result;
use crate::convert::{Losses, ToGml, Vocabulary};
use crate::model::{GmlObject, QName, Template};
use crate::vocabulary::module_of_prefix;
use crate::write::document::{Ade, ContentOf, Head, RESERVED_PREFIXES};

/// A CityGML document the model's objects are written to.
pub trait Document<W: Write>: Sized {
    /// Start a document on `out` with `head`. `templates` are the relative
    /// geometries the objects' implicit geometries place, by position;
    /// `content_of` gives an ADE class's CityGML class.
    fn start(out: W, head: Head, templates: Vec<Template>, content_of: ContentOf) -> Result<Self>;
    /// Write a top-level object, with its descendants.
    fn write(&mut self, object: GmlObject) -> Result<()>;
    /// How many top-level objects have been written.
    fn objects(&self) -> usize;
    /// End the document. Gives back what it was written to, and what was
    /// left out.
    fn finish(self) -> Result<(W, Losses)>;
}

/// How a document is written.
#[derive(Debug, Clone, Default)]
pub struct WriterOptions {
    /// The document's `srsName`. Without one, the header's
    /// `referenceSystem` is written, an EPSG CRS as its
    /// `urn:ogc:def:crs:EPSG::<code>` URN.
    pub srs_name: Option<String>,
    /// The XML namespace of a CityJSON Extension, by its name in the
    /// header, where its declaration gives none or a different one. A
    /// document using an Extension with no namespace is refused.
    pub ade_xmlns: BTreeMap<String, String>,
    /// Where the XML Schema of an ADE is, by its namespace URI, for the
    /// document's `xsi:schemaLocation`, where its Extension's `url` names
    /// none or another. A reader finds a schema named by a relative path or
    /// a local file, and fetches none named by URL.
    pub ade_schemas: BTreeMap<String, String>,
}

/// What a finished document holds, and what its version could not.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    /// Features written.
    pub features: usize,
    /// Top-level city objects written: a feature's root, and any object its
    /// parent's class could not hold.
    pub objects: usize,
    /// What was left out, by a description of it.
    pub losses: Losses,
}

/// A CityGML document written by `D` from a CityJSON header and its
/// features.
///
/// A feature's group links to objects of other features become
/// `xlink:href`s; the header's geometry templates are the relative
/// geometries the features' `GeometryInstance`s place, coloured by the
/// header's appearance.
pub struct Writer<W: Write, D: Document<W>> {
    document: D,
    context: GmlContext,
    features: usize,
    losses: Losses,
    out: PhantomData<W>,
}

impl<W: Write, D: Document<W>> Writer<W, D> {
    /// Start a document on `out` for the features of `header`, its classes
    /// named by `vocabulary`.
    pub fn new(
        out: W,
        header: &CityJSON,
        options: WriterOptions,
        vocabulary: Arc<dyn Vocabulary>,
    ) -> Result<Self> {
        let extensions = Arc::new(Extensions::new(
            header.extensions.iter().flatten(),
            &options.ade_xmlns,
        ));
        let mut losses = Losses::default();
        let (templates, lods) = match &header.geometry_templates {
            Some(t) => {
                let (templates, template_losses) = templates_to_gml(t, header.appearance.as_ref())?;
                losses.merge(template_losses);
                (templates, template_lods(t))
            }
            None => (Vec::new(), Arc::from(Vec::new())),
        };
        let metadata = header.metadata.as_ref();
        let srs_name = options.srs_name.clone().or_else(|| {
            let crs = metadata?.reference_system.as_ref()?;
            Some(match (crs.authority(), crs.code()) {
                (Some("EPSG"), Some(code)) => format!("urn:ogc:def:crs:EPSG::{code}"),
                _ => crs.to_url(),
            })
        });
        let head = Head {
            srs_name,
            envelope: metadata.and_then(|m| m.geographical_extent),
            ades: ades(&extensions, &options.ade_schemas),
            layout: extensions.layout(),
        };
        let content_of = {
            let extensions = Arc::clone(&extensions);
            let vocabulary = Arc::clone(&vocabulary);
            Box::new(move |class: &QName| {
                let base = extensions.base_type(&format!("+{}", class.local))?;
                vocabulary.class(base, &|_| None).map(|c| c.name)
            })
        };
        Ok(Self {
            document: D::start(out, head, templates, content_of)?,
            context: GmlContext {
                transform: header.transform.clone(),
                index: 0,
                vocabulary,
                extensions,
                template_lods: lods,
            },
            features: 0,
            losses,
            out: PhantomData,
        })
    }

    /// Write `feature`'s objects.
    pub fn write_feature(&mut self, feature: &CityJSONFeature) -> Result<()> {
        self.features += 1;
        self.context.index = self.features;
        let model = feature.to_gml(&self.context)?;
        self.losses.merge(model.losses);
        for object in model.objects {
            self.document.write(object)?;
        }
        Ok(())
    }

    /// End the document. Gives back what it was written to, and what it
    /// holds.
    pub fn finish(self) -> Result<(W, Report)> {
        let objects = self.document.objects();
        let (out, document_losses) = self.document.finish()?;
        let mut losses = self.losses;
        losses.merge(document_losses);
        Ok((
            out,
            Report {
                features: self.features,
                objects,
                losses,
            },
        ))
    }
}

/// The ADEs of `extensions` that have an XML namespace, each bound to the
/// Extension's name when that is a prefix the document leaves free, and
/// with its schema: the one `schemas` gives, else the Extension's `url`
/// when that is an XML Schema.
fn ades(extensions: &Extensions, schemas: &BTreeMap<String, String>) -> Vec<Ade> {
    let mut out: Vec<Ade> = Vec::new();
    for e in extensions.iter() {
        let Some(uri) = &e.xmlns else {
            continue;
        };
        if out.iter().any(|a| a.uri == *uri) {
            continue;
        }
        let free = |p: &str| {
            is_prefix(p)
                && !RESERVED_PREFIXES.contains(&p)
                && module_of_prefix(p).is_none()
                && !out.iter().any(|a| a.prefix == p)
        };
        let prefix = if free(&e.prefix) {
            e.prefix.clone()
        } else {
            (1..)
                .map(|n| format!("ade{n}"))
                .find(|p| free(p))
                .expect("a free prefix")
        };
        let schema = schemas.get(uri).cloned().or_else(|| {
            e.url
                .to_ascii_lowercase()
                .ends_with(".xsd")
                .then(|| e.url.clone())
        });
        out.push(Ade {
            prefix,
            uri: uri.clone(),
            schema,
        });
    }
    out
}

/// Whether `p` can be an XML namespace prefix: an NCName of ASCII letters,
/// digits, `-`, `_` and `.`, not beginning with a digit, `-` or `.`, nor
/// with `xml`.
fn is_prefix(p: &str) -> bool {
    let mut chars = p.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && !p.to_ascii_lowercase().starts_with("xml")
}
