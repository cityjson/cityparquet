//! What the up-front pass learns about a document's ADEs: an [`AdeIndex`].
//!
//! The pass sees every element, so it follows where each one sits without
//! reading objects: an ADE element where a city object belongs is a class,
//! one directly inside a city object a property, one directly inside a
//! `boundedBy` a boundary surface. These are the positions the walker reads
//! them in.

use citygml_core::model::{AdeIndex, AdeIndexBuilder, Module, QName};
use citygml_core::vocabulary::cityjson_type;
use citygml_core::xml::xml_err;
use citygml_core::{Error, Result};
use quick_xml::events::BytesStart;

use super::walker;
use crate::ns;

/// Where an element sits.
enum Frame {
    /// A property that holds a city object (`cityObjectMember`, a nesting
    /// property, `groupMember`).
    Member,
    /// A city object.
    Object(Owner),
    /// A property that holds boundary surfaces.
    Surfaces,
    /// Anything else.
    Other,
}

enum Owner {
    /// An object of a CityGML class, with its CityJSON type when it has one.
    CityGml(Option<&'static str>),
    /// An object of an ADE's class, by its namespace URI and local name.
    Ade(Vec<u8>, String),
}

#[derive(Default)]
pub(crate) struct AdeScan {
    builder: AdeIndexBuilder,
    frames: Vec<Frame>,
}

impl AdeScan {
    /// An element starts, in the namespace `uri`. Every `start` is matched
    /// by an [`Self::end`], so the caller calls neither for a subtree it
    /// consumes itself.
    pub(crate) fn start(&mut self, uri: &[u8], e: &BytesStart) -> Result<()> {
        let ade = ns::is_ade(uri);
        let mut attributes = Vec::new();
        for attribute in e.attributes() {
            let attribute = attribute.map_err(xml_err)?;
            let key = std::str::from_utf8(attribute.key.as_ref())
                .map_err(|_| Error::invalid("an XML attribute name is not UTF-8"))?;
            let value = attribute.unescape_value().map_err(xml_err)?;
            if let Some(prefix) = key.strip_prefix("xmlns:") {
                self.builder.bind(prefix, &value);
            } else if (key == "schemaLocation" || key.ends_with(":schemaLocation"))
                && self.frames.is_empty()
            {
                let words: Vec<&str> = value.split_whitespace().collect();
                for pair in words.chunks_exact(2) {
                    self.builder.schema_location(pair[0], pair[1]);
                }
            } else if ade && key != "xmlns" {
                attributes.push((key.to_string(), value.into_owned()));
            }
        }
        let local = std::str::from_utf8(e.local_name().into_inner())
            .map_err(|_| Error::invalid("an XML element name is not UTF-8"))?
            .to_string();
        let uri_text = String::from_utf8_lossy(uri);
        if ade {
            self.builder.element(
                &uri_text,
                &local,
                attributes.iter().map(|(k, v)| (k.as_str(), v.as_str())),
            );
        }
        let module = ns::module(uri);
        let frame = if local == "cityObjectMember" && module == Some(Module::Core)
            || walker::is_container_member(uri, local.as_bytes())
        {
            Frame::Member
        } else {
            match self.frames.last() {
                Some(Frame::Member) => self.object(uri, &local, ade),
                Some(Frame::Object(owner)) => {
                    let owner_type = match owner {
                        Owner::CityGml(ty) => ty.map(str::to_string),
                        Owner::Ade(_, class) => Some(format!("+{class}")),
                    };
                    let own = matches!(owner, Owner::Ade(own_ns, _) if own_ns == uri);
                    if ade {
                        // An ADE object's own properties are not extension
                        // attributes; another ADE's are.
                        if let Some(ty) = owner_type.filter(|_| !own) {
                            self.builder.property(&uri_text, &ty, &local);
                        }
                        if own && walker::is_semantic_surface_property(local.as_bytes()) {
                            Frame::Surfaces
                        } else {
                            Frame::Other
                        }
                    } else if module.is_some()
                        && walker::is_semantic_surface_property(local.as_bytes())
                    {
                        Frame::Surfaces
                    } else if module.is_some() && walker::is_nesting_property(local.as_bytes())
                        || module == Some(Module::CityObjectGroup) && local == "groupMember"
                    {
                        Frame::Member
                    } else {
                        Frame::Other
                    }
                }
                Some(Frame::Surfaces) => {
                    if ade {
                        self.builder.surface(&uri_text, &local);
                    }
                    Frame::Other
                }
                Some(Frame::Other) | None => Frame::Other,
            }
        };
        self.frames.push(frame);
        Ok(())
    }

    /// The element where a city object belongs.
    fn object(&mut self, uri: &[u8], local: &str, ade: bool) -> Frame {
        let class: Option<QName> = walker::top_level_class(uri, local.as_bytes())
            .or_else(|| walker::nested_class(uri, local.as_bytes()).map(|(class, _)| class));
        if let Some(class) = class {
            Frame::Object(Owner::CityGml(cityjson_type(&class)))
        } else if walker::is_container(uri, local.as_bytes()) {
            Frame::Object(Owner::CityGml(None))
        } else if ade {
            self.builder.class(&String::from_utf8_lossy(uri), local);
            Frame::Object(Owner::Ade(uri.to_vec(), local.to_string()))
        } else {
            Frame::Other
        }
    }

    /// The element the last unmatched [`Self::start`] opened ends.
    pub(crate) fn end(&mut self) {
        self.frames.pop();
    }

    pub(crate) fn finish(self) -> AdeIndex {
        self.builder.finish()
    }
}
