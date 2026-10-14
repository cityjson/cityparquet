//! What an Application Domain Extension (ADE) adds to a document, kept as
//! the document wrote it, and what a reader learns about the document's ADEs
//! before it reads an object.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use super::{Module, QName};
use crate::Result;
use crate::xsd::{Name, SchemaSet};

/// An element the model has no field for, with everything inside it: an ADE
/// property of a city object, kept verbatim so that a conversion, not the
/// reader, decides what a target format can hold.
#[derive(Debug, Clone, PartialEq)]
pub struct XmlNode {
    pub name: QName,
    /// Its XML attributes in document order, each named as the document
    /// spells it (`codeSpace`, `uom`, `xlink:href`). Namespace declarations
    /// are not attributes.
    pub attributes: Vec<XmlAttribute>,
    pub content: XmlContent,
}

/// One XML attribute of an [`XmlNode`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XmlAttribute {
    pub name: String,
    pub value: String,
}

/// What an [`XmlNode`] holds.
#[derive(Debug, Clone, PartialEq)]
pub enum XmlContent {
    /// Nothing, or only whitespace.
    Empty,
    /// Text, verbatim.
    Text(String),
    /// Child elements, in document order. Text between them is dropped.
    Children(Vec<XmlNode>),
}

impl XmlNode {
    /// The value of the attribute named `name` as the document spells it.
    pub fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|a| a.name == name)
            .map(|a| a.value.as_str())
    }
}

/// One ADE a document uses: an XML namespace outside CityGML and GML in
/// which at least one element occurs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ade {
    /// The ADE's prefix: the one the document first binds to its namespace
    /// (`uro`), lower-cased to ASCII letters and digits, `ade` when that
    /// leaves nothing or begins with a digit, and numbered from 2 (`uro2`)
    /// when an ADE used earlier has it. So every ADE of a document has its
    /// own, and it is a valid CityParquet namespace.
    pub prefix: String,
    /// The namespace URI (`https://www.geospatial.jp/iur/uro/3.2`).
    pub namespace: String,
    /// The schema document `xsi:schemaLocation` names for the namespace, as
    /// written (a relative path is relative to the document).
    pub schema_location: Option<String>,
    /// Its classes read as city objects: the elements directly inside a
    /// `cityObjectMember`, a group's `groupMember` or a nesting property.
    pub classes: BTreeSet<String>,
    /// Its classes read as boundary surfaces: the elements directly inside a
    /// `boundedBy`.
    pub surfaces: BTreeSet<String>,
    /// Its properties of city objects of a CityGML class, by the CityJSON
    /// type of the object they are found on.
    pub properties: BTreeMap<String, BTreeSet<String>>,
    /// The CityGML class each of its [`Self::classes`] specialises, as its
    /// schema's substitution groups say (`UndergroundBuilding` substitutes
    /// for `bldg:_AbstractBuilding`), through any classes of its own. A
    /// class whose schema was not read is not listed.
    pub parents: BTreeMap<String, QName>,
    /// The unprefixed XML attributes of its elements (`codeSpace`, `uom`)
    /// that every occurrence of the element carries with one value, by
    /// element local name, then attribute name: what a
    /// conversion may state once for the document instead of on every
    /// element.
    pub constant_attributes: BTreeMap<String, BTreeMap<String, String>>,
}

/// The ADEs of one document, read before its objects.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AdeIndex {
    /// In the order the document first uses them.
    ades: Vec<Ade>,
    /// Every namespace URI the document binds, to the first prefix it binds
    /// to it, or for an ADE to [`Ade::prefix`].
    prefixes: HashMap<String, String>,
    /// The ADEs' schemas, as far as they were found.
    schemas: SchemaSet,
}

impl AdeIndex {
    /// The ADEs the document uses, in the order it first uses them.
    pub fn ades(&self) -> &[Ade] {
        &self.ades
    }

    pub fn is_empty(&self) -> bool {
        self.ades.is_empty()
    }

    /// The ADE whose namespace URI is `namespace`.
    pub fn ade(&self, namespace: &str) -> Option<&Ade> {
        self.ades.iter().find(|a| a.namespace == namespace)
    }

    /// The prefix of `namespace`: [`Ade::prefix`] for an ADE's, else the
    /// first the document binds to it, if it binds one.
    pub fn prefix(&self, namespace: &str) -> Option<&str> {
        self.prefixes.get(namespace).map(String::as_str)
    }

    /// The ADEs' schemas read by [`Self::read_schemas`].
    pub fn schemas(&self) -> &SchemaSet {
        &self.schemas
    }

    /// Read the schema of each ADE that `xsi:schemaLocation` names by a path
    /// relative to `document` (PLATEAU's `../../schemas/iur/uro/3.2/…`),
    /// when that file exists, and record the CityGML class each ADE class
    /// specialises. `module` tells a CityGML module's namespace URI from
    /// another. A schema named by URL is not fetched, so an ADE whose schema
    /// is not at hand is read by position alone.
    pub fn read_schemas(
        &mut self,
        document: &Path,
        module: impl Fn(&str) -> Option<Module>,
    ) -> Result<()> {
        let dir = document.parent().unwrap_or(Path::new(""));
        for ade in &self.ades {
            if self.schemas.has_namespace(&ade.namespace) {
                continue;
            }
            let Some(location) = ade
                .schema_location
                .as_deref()
                .filter(|l| !l.contains("://"))
            else {
                continue;
            };
            let path = dir.join(location);
            if path.is_file() {
                self.schemas.read(&path)?;
            }
        }
        for ade in &mut self.ades {
            ade.parents = ade
                .classes
                .iter()
                .filter_map(|class| {
                    let parent = specialises(&self.schemas, &ade.namespace, class, &module)?;
                    Some((class.clone(), parent))
                })
                .collect();
        }
        Ok(())
    }

    /// The attributes of `element` (an ADE element, by namespace URI and
    /// local name) that have one value throughout the document.
    pub fn constant_attributes(
        &self,
        namespace: &str,
        element: &str,
    ) -> Option<&BTreeMap<String, String>> {
        self.ade(namespace)?.constant_attributes.get(element)
    }
}

/// Collects an [`AdeIndex`] while a reader passes over a document.
#[derive(Debug, Default)]
pub struct AdeIndexBuilder {
    prefixes: HashMap<String, String>,
    schema_locations: HashMap<String, String>,
    /// Namespace URIs in the order of their first element.
    order: Vec<String>,
    ades: HashMap<String, Used>,
}

#[derive(Debug, Default)]
struct Used {
    classes: BTreeSet<String>,
    surfaces: BTreeSet<String>,
    properties: BTreeMap<String, BTreeSet<String>>,
    elements: BTreeMap<String, Occurrences>,
}

/// How often an element occurs, and the value of each of its attributes
/// while that value has not varied.
#[derive(Debug, Default)]
struct Occurrences {
    count: usize,
    attributes: BTreeMap<String, Seen>,
}

#[derive(Debug)]
struct Seen {
    value: String,
    count: usize,
    varies: bool,
}

impl AdeIndexBuilder {
    /// The document binds `prefix` to `namespace`. The first binding of a
    /// namespace is the one kept.
    pub fn bind(&mut self, prefix: &str, namespace: &str) {
        self.prefixes
            .entry(namespace.to_string())
            .or_insert_with(|| prefix.to_string());
    }

    /// The document's `xsi:schemaLocation` names `location` for `namespace`.
    pub fn schema_location(&mut self, namespace: &str, location: &str) {
        self.schema_locations
            .entry(namespace.to_string())
            .or_insert_with(|| location.to_string());
    }

    /// An element of the ADE `namespace` occurs, with `attributes` (named as
    /// the document spells them, namespace declarations left out).
    pub fn element<'a>(
        &mut self,
        namespace: &str,
        local: &str,
        attributes: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) {
        let used = self.used(namespace);
        let occurrences = used.elements.entry(local.to_string()).or_default();
        occurrences.count += 1;
        // Only an attribute without a prefix (`codeSpace`, `uom`) is the
        // element's own; `gml:id` and `xlink:href` identify and link it.
        for (name, value) in attributes.into_iter().filter(|(n, _)| !n.contains(':')) {
            match occurrences.attributes.get_mut(name) {
                Some(seen) => {
                    seen.count += 1;
                    seen.varies |= seen.value != value;
                }
                None => {
                    occurrences.attributes.insert(
                        name.to_string(),
                        Seen {
                            value: value.to_string(),
                            count: 1,
                            varies: false,
                        },
                    );
                }
            }
        }
    }

    /// An element of the ADE `namespace` is read as a city object.
    pub fn class(&mut self, namespace: &str, local: &str) {
        self.used(namespace).classes.insert(local.to_string());
    }

    /// An element of the ADE `namespace` is read as a boundary surface.
    pub fn surface(&mut self, namespace: &str, local: &str) {
        self.used(namespace).surfaces.insert(local.to_string());
    }

    /// An element of the ADE `namespace` is a property of a city object of
    /// CityJSON type `object_type`.
    pub fn property(&mut self, namespace: &str, object_type: &str, local: &str) {
        self.used(namespace)
            .properties
            .entry(object_type.to_string())
            .or_default()
            .insert(local.to_string());
    }

    fn used(&mut self, namespace: &str) -> &mut Used {
        if !self.ades.contains_key(namespace) {
            self.order.push(namespace.to_string());
        }
        self.ades.entry(namespace.to_string()).or_default()
    }

    pub fn finish(mut self) -> AdeIndex {
        let mut taken: BTreeSet<String> = BTreeSet::new();
        let ades: Vec<Ade> = self
            .order
            .iter()
            .map(|namespace| {
                let bound: String = self
                    .prefixes
                    .get(namespace)
                    .map(String::as_str)
                    .unwrap_or_default()
                    .chars()
                    .filter(char::is_ascii_alphanumeric)
                    .map(|c| c.to_ascii_lowercase())
                    .collect();
                let base = if bound.starts_with(|c: char| c.is_ascii_lowercase()) {
                    bound
                } else {
                    "ade".to_string()
                };
                let prefix = (1..)
                    .map(|n| {
                        if n == 1 {
                            base.clone()
                        } else {
                            format!("{base}{n}")
                        }
                    })
                    .find(|p| !taken.contains(p))
                    .expect("an unbounded range has a free name");
                taken.insert(prefix.clone());
                let used = self.ades.remove(namespace).unwrap_or_default();
                let constant_attributes = used
                    .elements
                    .into_iter()
                    .filter_map(|(element, occurrences)| {
                        let constant: BTreeMap<String, String> = occurrences
                            .attributes
                            .into_iter()
                            .filter(|(_, seen)| !seen.varies && seen.count == occurrences.count)
                            .map(|(name, seen)| (name, seen.value))
                            .collect();
                        (!constant.is_empty()).then_some((element, constant))
                    })
                    .collect();
                Ade {
                    prefix,
                    namespace: namespace.clone(),
                    schema_location: self.schema_locations.remove(namespace),
                    classes: used.classes,
                    surfaces: used.surfaces,
                    properties: used.properties,
                    parents: BTreeMap::new(),
                    constant_attributes,
                }
            })
            .collect();
        for a in &ades {
            self.prefixes.insert(a.namespace.clone(), a.prefix.clone());
        }
        AdeIndex {
            ades,
            prefixes: self.prefixes,
            schemas: SchemaSet::default(),
        }
    }
}

/// The CityGML class the ADE class `local` of `namespace` specialises: the
/// first element of a CityGML module along its chain of substitution
/// groups, or `None` when the chain leaves the schemas read or reaches
/// another namespace first (`gml:_Feature`).
fn specialises(
    schemas: &SchemaSet,
    namespace: &str,
    local: &str,
    module: impl Fn(&str) -> Option<Module>,
) -> Option<QName> {
    let mut name: Name = (namespace.to_string(), local.to_string());
    for _ in 0..64 {
        let head = schemas.substitution_group(&name)?;
        if let Some(m) = module(&head.0) {
            return Some(QName::module(m, head.1.clone()));
        }
        if !schemas.has_namespace(&head.0) {
            return None;
        }
        name = head.clone();
    }
    None
}
