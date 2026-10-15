//! What an ADE's XML Schema says that a conversion uses: the element each
//! of its elements substitutes for, and the type of each element.
//!
//! [`SchemaSet::read`] reads a schema document with every document it
//! includes or imports by a local path; a document named by URL is not
//! fetched, so the CityGML and GML schemas an ADE builds on are never read.
//! A document that is not there is skipped, so a schema whose imports are
//! missing still gives what it declares itself. Only what the lookups below
//! need is kept: global elements with their type and substitution group,
//! named and anonymous types with their base, and the elements a complex
//! type's content model declares. Attributes, facets and `xs:group`s are
//! not, and an included document without a `targetNamespace` keeps its
//! names in no namespace rather than taking the includer's.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use quick_xml::events::{BytesStart, Event};
use quick_xml::name::{QName, ResolveResult};
use quick_xml::reader::NsReader;

use crate::xml::{NS_GML, get_attr, xml_err};
use crate::{Error, Result};

/// XML Schema's own namespace.
pub const NS_XSD: &str = "http://www.w3.org/2001/XMLSchema";
/// GML 3.2's namespace, which CityGML 3.0 builds on.
const NS_GML32: &str = "http://www.opengis.net/gml/3.2";

/// How far a chain of bases or substitution groups is followed: a guard
/// against a cyclic schema.
const MAX_CHAIN: usize = 64;

/// A name in a schema: namespace URI and local name.
pub type Name = (String, String);

/// What the text of an element of a simple type becomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueType {
    /// An `xs:integer` or one of its restrictions.
    Integer,
    /// An `xs:double`, `xs:float` or `xs:decimal`, or a GML measure
    /// (`gml:MeasureType`, `gml:LengthType`, …).
    Number,
    /// An `xs:boolean`.
    Boolean,
    /// Anything else: text.
    Text,
}

/// The schema documents read for one CityGML document's ADEs.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SchemaSet {
    elements: HashMap<Name, Element>,
    types: HashMap<Name, Type>,
    /// The target namespaces of the documents read.
    namespaces: HashSet<String>,
    /// The documents read, so an import cycle reads each once.
    read: HashSet<PathBuf>,
    /// Names given to anonymous types.
    anonymous: usize,
}

#[derive(Debug, Clone, Default, PartialEq)]
struct Element {
    ty: Option<Name>,
    substitutes: Option<Name>,
}

#[derive(Debug, Clone, Default, PartialEq)]
struct Type {
    /// The `base` of its extension or restriction.
    base: Option<Name>,
    /// The elements its content model declares, by name: `Some` with the
    /// declared type, or `None` for a `ref` to a global element.
    children: HashMap<Name, Option<Name>>,
    /// The same elements, in the order the content model declares them.
    order: Vec<Name>,
}

impl SchemaSet {
    /// Whether no schema document has been read.
    pub fn is_empty(&self) -> bool {
        self.namespaces.is_empty()
    }

    /// Whether a document with target namespace `namespace` has been read.
    pub fn has_namespace(&self, namespace: &str) -> bool {
        self.namespaces.contains(namespace)
    }

    /// Read the schema document at `path`, and every document it includes or
    /// imports by a path relative to it. One already read, or not there, is
    /// not read; one that is there but cannot be parsed is an error.
    pub fn read(&mut self, path: &Path) -> Result<()> {
        let mut queue = vec![path.to_path_buf()];
        while let Some(path) = queue.pop() {
            if !path.is_file() {
                continue;
            }
            let key = path.canonicalize().unwrap_or_else(|_| path.clone());
            if !self.read.insert(key) {
                continue;
            }
            let bytes = std::fs::read(&path)
                .map_err(|e| Error::io(format!("cannot read {}", path.display()), e))?;
            let found = self.read_document(&bytes).map_err(|e| {
                Error::invalid(format!("cannot read the schema {}: {e}", path.display()))
            })?;
            let dir = path.parent().unwrap_or(Path::new(""));
            queue.extend(found.into_iter().map(|location| dir.join(location)));
        }
        Ok(())
    }

    /// The type of the global element `name`.
    pub fn element_type(&self, name: &Name) -> Option<&Name> {
        self.elements.get(name)?.ty.as_ref()
    }

    /// The element the global element `name` substitutes for.
    pub fn substitution_group(&self, name: &Name) -> Option<&Name> {
        self.elements.get(name)?.substitutes.as_ref()
    }

    /// The type of an element `child` inside an element of type `parent`:
    /// what the content model of `parent`, or of a type it derives from,
    /// declares for it, else the global element `child`'s (a member of a
    /// substitution group, or an element of unknown context).
    pub fn child_type(&self, parent: Option<&Name>, child: &Name) -> Option<&Name> {
        let mut ty = parent;
        for _ in 0..MAX_CHAIN {
            let Some(def) = ty.and_then(|t| self.types.get(t)) else {
                break;
            };
            match def.children.get(child) {
                Some(Some(declared)) => return Some(declared),
                Some(None) => break,
                None => ty = def.base.as_ref(),
            }
        }
        self.element_type(child)
    }

    /// The elements the content model of `ty` declares, in order, after
    /// those of the types it derives from, as far as the schemas read
    /// define them.
    pub fn content(&self, ty: &Name) -> Vec<Name> {
        let mut chain = Vec::new();
        let mut ty = Some(ty);
        while let Some(def) = ty.and_then(|t| self.types.get(t)) {
            if chain.len() == MAX_CHAIN {
                break;
            }
            chain.push(def);
            ty = def.base.as_ref();
        }
        chain
            .into_iter()
            .rev()
            .flat_map(|def| def.order.iter().cloned())
            .collect()
    }

    /// What the text of an element of type `ty` becomes, following the
    /// type's bases to a type of XML Schema or GML, or `None` when they lead
    /// to a type of a schema not read.
    pub fn value_type(&self, ty: &Name) -> Option<ValueType> {
        let mut ty = ty;
        for _ in 0..MAX_CHAIN {
            if let Some(builtin) = builtin(ty) {
                return Some(builtin);
            }
            match self.types.get(ty) {
                Some(def) => ty = def.base.as_ref()?,
                None => return None,
            }
        }
        None
    }

    /// Read one schema document, returning the locations of the documents
    /// it includes or imports by a relative path.
    fn read_document(&mut self, bytes: &[u8]) -> Result<Vec<String>> {
        let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
        let mut reader = NsReader::from_reader(bytes);
        reader.config_mut().expand_empty_elements = true;
        let mut buf = Vec::new();
        let mut target = String::new();
        let mut qualified = false;
        let mut found = Vec::new();
        let mut stack: Vec<Frame> = Vec::new();
        loop {
            buf.clear();
            let (rr, ev) = reader.read_resolved_event_into(&mut buf).map_err(xml_err)?;
            match ev {
                Event::Start(e) => {
                    let xsd =
                        matches!(&rr, ResolveResult::Bound(ns) if ns.as_ref() == NS_XSD.as_bytes());
                    let frame = if !xsd {
                        Frame::Other
                    } else {
                        match e.local_name().as_ref() {
                            b"schema" => {
                                target = get_attr(&e, b"targetNamespace").unwrap_or_default();
                                qualified = get_attr(&e, b"elementFormDefault").as_deref()
                                    == Some("qualified");
                                self.namespaces.insert(target.clone());
                                Frame::Other
                            }
                            b"include" | b"import" => {
                                if let Some(location) =
                                    get_attr(&e, b"schemaLocation").filter(|l| !l.contains("://"))
                                {
                                    found.push(location);
                                }
                                Frame::Other
                            }
                            b"element" => self.element(&reader, &e, &stack, &target, qualified)?,
                            b"complexType" | b"simpleType" => self.ty(&e, &mut stack, &target),
                            b"extension" | b"restriction" => {
                                if let (Some(base), Some(ty)) =
                                    (resolve(&reader, &e, b"base")?, current_type(&stack))
                                {
                                    self.types.entry(ty.clone()).or_default().base = Some(base);
                                }
                                Frame::Other
                            }
                            _ => Frame::Other,
                        }
                    };
                    stack.push(frame);
                }
                Event::End(_) => {
                    stack.pop();
                }
                Event::Eof => break,
                _ => {}
            }
        }
        Ok(found)
    }

    /// An `xs:element`: global when directly inside the schema, else part of
    /// the content model of the type it is in.
    fn element(
        &mut self,
        reader: &NsReader<&[u8]>,
        e: &BytesStart,
        stack: &[Frame],
        target: &str,
        qualified: bool,
    ) -> Result<Frame> {
        let ty = resolve(reader, e, b"type")?;
        if stack.len() == 1 {
            let Some(local) = get_attr(e, b"name") else {
                return Ok(Frame::Other);
            };
            let name = (target.to_string(), local);
            let element = Element {
                ty,
                substitutes: resolve(reader, e, b"substitutionGroup")?,
            };
            self.elements.insert(name.clone(), element);
            return Ok(Frame::Element(Owner::Global(name)));
        }
        let Some(parent) = current_type(stack).cloned() else {
            return Ok(Frame::Other);
        };
        let (name, declared) = match resolve(reader, e, b"ref")? {
            Some(name) => (name, None),
            None => {
                let Some(local) = get_attr(e, b"name") else {
                    return Ok(Frame::Other);
                };
                let ns = if qualified { target } else { "" };
                ((ns.to_string(), local), ty)
            }
        };
        let def = self.types.entry(parent.clone()).or_default();
        if !def.children.contains_key(&name) {
            def.order.push(name.clone());
        }
        def.children.entry(name.clone()).or_insert(declared);
        Ok(Frame::Element(Owner::Local(parent, name)))
    }

    /// An `xs:complexType` or `xs:simpleType`: named, or anonymous inside an
    /// element, which then has it as its type.
    fn ty(&mut self, e: &BytesStart, stack: &mut [Frame], target: &str) -> Frame {
        let name = match get_attr(e, b"name") {
            Some(local) => (target.to_string(), local),
            None => {
                self.anonymous += 1;
                let name = (target.to_string(), format!("#{}", self.anonymous));
                match stack.last() {
                    Some(Frame::Element(Owner::Global(element))) => {
                        if let Some(el) = self.elements.get_mut(element) {
                            el.ty = Some(name.clone());
                        }
                    }
                    Some(Frame::Element(Owner::Local(parent, element))) => {
                        if let Some(def) = self.types.get_mut(parent) {
                            def.children.insert(element.clone(), Some(name.clone()));
                        }
                    }
                    _ => {}
                }
                name
            }
        };
        self.types.entry(name.clone()).or_default();
        Frame::Type(name)
    }
}

/// What an element of a schema document opened.
enum Frame {
    Type(Name),
    Element(Owner),
    Other,
}

/// An `xs:element` an anonymous type may be the type of.
enum Owner {
    Global(Name),
    /// One in the content model of a type: the type, then the element.
    Local(Name, Name),
}

/// The type the innermost open `xs:complexType` or `xs:simpleType` defines.
fn current_type(stack: &[Frame]) -> Option<&Name> {
    stack.iter().rev().find_map(|f| match f {
        Frame::Type(name) => Some(name),
        _ => None,
    })
}

/// The value of attribute `attr`, a prefixed name, resolved against the
/// namespaces in scope.
fn resolve(reader: &NsReader<&[u8]>, e: &BytesStart, attr: &[u8]) -> Result<Option<Name>> {
    let Some(value) = get_attr(e, attr) else {
        return Ok(None);
    };
    let (rr, local) = reader.resolver().resolve(QName(value.as_bytes()), true);
    let ns = match rr {
        ResolveResult::Bound(ns) => String::from_utf8_lossy(ns.as_ref()).into_owned(),
        ResolveResult::Unbound => String::new(),
        // A name whose prefix is unbound names nothing this reader can find.
        ResolveResult::Unknown(_) => return Ok(None),
    };
    Ok(Some((
        ns,
        String::from_utf8_lossy(local.as_ref()).into_owned(),
    )))
}

/// What a type of XML Schema or GML holds, or `None` for another
/// namespace's.
fn builtin((ns, local): &Name) -> Option<ValueType> {
    if ns == NS_XSD {
        return Some(match local.as_str() {
            "integer" | "int" | "long" | "short" | "byte" | "nonNegativeInteger"
            | "positiveInteger" | "nonPositiveInteger" | "negativeInteger" | "unsignedLong"
            | "unsignedInt" | "unsignedShort" | "unsignedByte" => ValueType::Integer,
            "double" | "float" | "decimal" => ValueType::Number,
            "boolean" => ValueType::Boolean,
            _ => ValueType::Text,
        });
    }
    if ns == NS_GML || ns == NS_GML32 {
        return Some(match local.as_str() {
            "MeasureType" | "LengthType" | "AreaType" | "VolumeType" | "AngleType"
            | "SpeedType" | "ScaleType" | "TimeType" | "GridLengthType" => ValueType::Number,
            _ => ValueType::Text,
        });
    }
    None
}
