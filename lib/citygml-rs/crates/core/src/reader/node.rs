//! An element kept verbatim, as an [`XmlNode`]: what a reader does with an
//! ADE property.

use std::io::BufRead;

use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::NsReader;

use crate::model::{Namespace, QName, XmlAttribute, XmlContent, XmlNode};
use crate::xml::{ns_of, unescaped_text, xml_err};
use crate::{Error, Result};

/// An element's XML attributes, namespace declarations left out.
pub fn xml_attributes(e: &BytesStart) -> Result<Vec<XmlAttribute>> {
    let mut attributes = Vec::new();
    for attribute in e.attributes() {
        let attribute = attribute.map_err(xml_err)?;
        let key = attribute.key.as_ref();
        if key == b"xmlns" || key.starts_with(b"xmlns:") {
            continue;
        }
        attributes.push(XmlAttribute {
            name: String::from_utf8_lossy(key).into_owned(),
            value: attribute.unescape_value().map_err(xml_err)?.into_owned(),
        });
    }
    Ok(attributes)
}

/// The subtree of an element (positioned after its `Start`, which gave its
/// `name` and `attributes`) as an [`XmlNode`], verbatim, each element named
/// in the model namespace `namespace` gives its URI. Read without
/// recursion, so a deep subtree cannot exhaust the stack.
pub fn read_xml_node<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    name: QName,
    attributes: Vec<XmlAttribute>,
    namespace: impl Fn(&[u8]) -> Namespace,
) -> Result<XmlNode> {
    struct Open {
        name: QName,
        attributes: Vec<XmlAttribute>,
        text: String,
        children: Vec<XmlNode>,
    }
    impl Open {
        fn close(self) -> XmlNode {
            let content = if !self.children.is_empty() {
                XmlContent::Children(self.children)
            } else if self.text.trim().is_empty() {
                XmlContent::Empty
            } else {
                XmlContent::Text(self.text)
            };
            XmlNode {
                name: self.name,
                attributes: self.attributes,
                content,
            }
        }
    }
    let mut open = vec![Open {
        name,
        attributes,
        text: String::new(),
        children: Vec::new(),
    }];
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        match ev {
            Event::Start(e) => {
                let uri = ns_of(&rr).unwrap_or_default();
                open.push(Open {
                    name: QName::new(
                        namespace(&uri),
                        String::from_utf8_lossy(e.local_name().as_ref()),
                    ),
                    attributes: xml_attributes(&e)?,
                    text: String::new(),
                    children: Vec::new(),
                });
            }
            Event::Text(t) => {
                if let Some(top) = open.last_mut() {
                    top.text.push_str(&unescaped_text(&t)?);
                }
            }
            Event::CData(t) => {
                if let Some(top) = open.last_mut() {
                    top.text.push_str(&String::from_utf8_lossy(&t));
                }
            }
            Event::End(_) => {
                let Some(top) = open.pop() else {
                    unreachable!("an End always closes an open element");
                };
                let node = top.close();
                match open.last_mut() {
                    Some(parent) => parent.children.push(node),
                    None => return Ok(node),
                }
            }
            Event::Eof => {
                return Err(Error::invalid(
                    "unexpected end of document inside an ADE property",
                ));
            }
            _ => {}
        }
    }
}
