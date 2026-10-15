//! CityJSON to a CityGML document: a header, then its features one at a
//! time, in any order. What both versions' writers share; each names its
//! classes with its own [`Vocabulary`] and writes with its own document
//! writer ([`Document`]).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Write;
use std::marker::PhantomData;
use std::sync::Arc;

use cjseq::{CityJSON, CityJSONFeature};

use super::{Extensions, GmlContext, ncname, template_lods, templates_to_gml};
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
///
/// An object whose parent is in another feature is written inside that
/// parent, whichever comes first: a parent waits for the children it names
/// in other features, and a part (an object of a type CityJSON gives a
/// parent, such as a `BuildingPart`) for its parent. A part whose parent
/// has no class in the version takes the parent's place, as within a
/// feature. Only these wait, so a writer holds nothing for features that
/// keep their parts, and a part whose parent never comes until the end.
/// A feature is otherwise written as it comes.
pub struct Writer<W: Write, D: Document<W>> {
    document: D,
    context: GmlContext,
    features: usize,
    losses: Losses,
    /// Top-level objects waiting, in the order they came.
    held: BTreeMap<usize, Held>,
    next: usize,
    /// The children named by an object in a held tree that have not come,
    /// by id: their parent's id and the tree it is in.
    awaited: HashMap<String, (String, usize)>,
    /// The parts no parent has claimed, by id: the held trees standing for
    /// each, which are the part, or for a part with no class here, its
    /// children.
    strays: HashMap<String, Vec<usize>>,
    /// The children in other features of a part with no class here, by
    /// the part's id, for the parent that claims it.
    proxies: HashMap<String, Vec<String>>,
    /// The extension objects written at the top level whose parent is in
    /// another feature, which a parent coming later cannot claim.
    gone: HashSet<String>,
    out: PhantomData<W>,
}

/// A top-level object waiting to be written.
struct Held {
    object: GmlObject,
    /// How many children named in it have not come.
    awaiting: usize,
    /// Whether it is a part whose parent has not come.
    stray: bool,
}

/// CityJSON's 1st-level City Object types: those that need no parent.
const FIRST_LEVEL: &[&str] = &[
    "Bridge",
    "Building",
    "CityFurniture",
    "CityObjectGroup",
    "GenericCityObject",
    "LandUse",
    "OtherConstruction",
    "PlantCover",
    "Railway",
    "Road",
    "SolitaryVegetationObject",
    "TINRelief",
    "TransportSquare",
    "Tunnel",
    "WaterBody",
    "Waterway",
];

/// Map the ids of `tree`'s objects to `key`.
fn ids_of(tree: &GmlObject, key: usize, out: &mut HashMap<String, usize>) {
    if let Some(id) = &tree.id {
        out.insert(id.clone(), key);
    }
    for c in &tree.children {
        ids_of(c, key, out);
    }
}

/// The topmost ancestor of `id` in its feature, by `up`.
fn place_top<'a>(id: &'a str, up: &impl Fn(&str) -> Option<&'a str>) -> &'a str {
    let mut at = id;
    let mut steps = 0;
    while let Some(p) = up(at) {
        at = p;
        steps += 1;
        if steps > 1024 {
            break;
        }
    }
    at
}

/// The object of `tree` whose id is `id`.
fn find<'a>(tree: &'a mut GmlObject, id: &str) -> Option<&'a mut GmlObject> {
    if tree.id.as_deref() == Some(id) {
        return Some(tree);
    }
    tree.children.iter_mut().find_map(|c| find(c, id))
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
            held: BTreeMap::new(),
            next: 0,
            awaited: HashMap::new(),
            strays: HashMap::new(),
            proxies: HashMap::new(),
            gone: HashSet::new(),
            out: PhantomData,
        })
    }

    /// Write `feature`'s objects.
    pub fn write_feature(&mut self, feature: &CityJSONFeature) -> Result<()> {
        self.features += 1;
        self.context.index = self.features;
        let model = feature.to_gml(&self.context)?;
        self.losses.merge(model.losses);
        let objects = &feature.city_objects;
        let is_group = |id: &str| {
            objects
                .get(id)
                .is_some_and(|co| String::from(co.thetype.clone()) == "CityObjectGroup")
        };
        // The parts with a parent in another feature, by id (whether each
        // is of a CityGML type, rather than an extension's), and the
        // children in other features that objects other than groups name.
        let mut outside = HashMap::new();
        let mut links = Vec::new();
        for (id, co) in objects {
            let ty = String::from(co.thetype.clone());
            if !FIRST_LEVEL.contains(&ty.as_str())
                && co
                    .parents
                    .iter()
                    .flatten()
                    .any(|p| !objects.contains_key(p))
            {
                outside.insert(ncname(id).into_owned(), !ty.starts_with('+'));
            }
            if ty != "CityObjectGroup" {
                let mut seen = HashSet::new();
                for c in co.children.iter().flatten() {
                    if !objects.contains_key(c) && seen.insert(c) {
                        links.push((id.as_str(), ncname(c).into_owned()));
                    }
                }
            }
        }
        if outside.is_empty()
            && links.is_empty()
            && (self.awaited.is_empty()
                || !objects
                    .keys()
                    .any(|id| self.awaited.contains_key(&*ncname(id))))
        {
            for object in model.objects {
                self.document.write(object)?;
            }
            return Ok(());
        }
        // An object's parent in the feature: one other than a group that
        // lists it, as the conversion nests it.
        let up = |id: &str| -> Option<&str> {
            objects[id]
                .parents
                .iter()
                .flatten()
                .map(String::as_str)
                .find(|p| {
                    !is_group(p)
                        && objects
                            .get(*p)
                            .and_then(|co| co.children.as_ref())
                            .is_some_and(|c| c.iter().any(|c| c == id))
                })
        };
        // The object an object is written in or in place of: itself, or
        // its nearest ancestor with a class here (`Ok`), or the topmost
        // ancestor, which has none (`Err`).
        let mut tree_of = HashMap::new();
        let mut came = Vec::new();
        for object in model.objects {
            let key = self.next;
            self.next += 1;
            ids_of(&object, key, &mut tree_of);
            came.push((key, object.id.clone()));
            self.held.insert(
                key,
                Held {
                    object,
                    awaiting: 0,
                    stray: false,
                },
            );
        }
        let place = |id: &str| -> std::result::Result<(String, usize), String> {
            let mut at = id;
            for _ in 0..objects.len() {
                let name = ncname(at).into_owned();
                if let Some(&key) = tree_of.get(&name) {
                    return Ok((name, key));
                }
                match up(at) {
                    Some(p) => at = p,
                    None => return Err(name),
                }
            }
            Err(ncname(at).into_owned())
        };
        let mut touched: Vec<usize> = came.iter().map(|(k, _)| *k).collect();
        // The children in other features of objects with no class here.
        let mut deferred: HashMap<String, Vec<String>> = HashMap::new();
        for (parent, child) in links {
            match place(parent) {
                Ok((parent, tree)) => self.link(tree, &parent, child),
                Err(top) => deferred.entry(top).or_default().push(child),
            }
        }
        // The objects with no class here and no parent in the feature:
        // the roots their children became stand in for them.
        let named: HashMap<String, &str> = objects
            .keys()
            .map(|k| (ncname(k).into_owned(), k.as_str()))
            .collect();
        let mut taken = HashSet::new();
        for id in objects.keys() {
            let name = ncname(id).into_owned();
            if tree_of.contains_key(&name) || up(id).is_some() {
                continue;
            }
            let stand: Vec<usize> = came
                .iter()
                .filter(|(_, root)| {
                    root.as_ref()
                        .and_then(|root| named.get(root.as_str()))
                        .is_some_and(|k| place_top(k, &up) == id.as_str())
                })
                .map(|(k, _)| *k)
                .collect();
            taken.extend(stand.iter().copied());
            let children = deferred.remove(&name).unwrap_or_default();
            if let Some((parent, tree)) = self.awaited.remove(&name) {
                self.held.get_mut(&tree).expect("held").awaiting -= 1;
                for key in stand {
                    self.claim(tree, &parent, key);
                }
                for child in children {
                    self.link(tree, &parent, child);
                }
                touched.push(tree);
            } else if outside.get(&name) == Some(&true) {
                for key in &stand {
                    self.held.get_mut(key).expect("held").stray = true;
                }
                self.strays.insert(name.clone(), stand);
                self.proxies.insert(name, children);
            } else {
                for _ in children {
                    self.losses
                        .note("links to children outside their parent's feature");
                }
            }
        }
        for (key, id) in came {
            let Some(id) = id else { continue };
            if taken.contains(&key) || !self.held.contains_key(&key) {
                continue;
            }
            match self.awaited.remove(&id) {
                Some((parent, tree)) if tree != key => {
                    self.held.get_mut(&tree).expect("held").awaiting -= 1;
                    self.claim(tree, &parent, key);
                    touched.push(tree);
                }
                Some(link) => {
                    self.awaited.insert(id, link);
                }
                None => match outside.get(&id) {
                    Some(true) => {
                        self.held.get_mut(&key).expect("held").stray = true;
                        self.strays.insert(id, vec![key]);
                    }
                    Some(false) => {
                        self.gone.insert(id);
                    }
                    None => {}
                },
            }
        }
        for key in touched {
            if self
                .held
                .get(&key)
                .is_some_and(|h| h.awaiting == 0 && !h.stray)
            {
                let held = self.held.remove(&key).expect("held");
                self.document.write(held.object)?;
            }
        }
        Ok(())
    }

    /// `child`, named by `parent` in the held tree `tree`, goes in it if it
    /// has come and is unclaimed, or is awaited.
    fn link(&mut self, tree: usize, parent: &str, child: String) {
        if let Some(keys) = self.strays.remove(&child) {
            for key in keys {
                self.claim(tree, parent, key);
            }
            for grandchild in self.proxies.remove(&child).unwrap_or_default() {
                self.link(tree, parent, grandchild);
            }
        } else if self.gone.contains(&child) || self.awaited.contains_key(&child) {
            self.losses
                .note("links to children outside their parent's feature");
        } else {
            self.held.get_mut(&tree).expect("held").awaiting += 1;
            self.awaited.insert(child, (parent.to_string(), tree));
        }
    }

    /// Put the held tree `part` in `parent`, an object of the held tree
    /// `tree`, which takes on the children `part` awaits.
    fn claim(&mut self, tree: usize, parent: &str, part: usize) {
        let mut moved = 0;
        for (_, at) in self.awaited.values_mut() {
            if *at == part {
                *at = tree;
                moved += 1;
            }
        }
        let part = self.held.remove(&part).expect("held").object;
        let held = self.held.get_mut(&tree).expect("held");
        held.awaiting += moved;
        find(&mut held.object, parent)
            .expect("the parent is in its tree")
            .children
            .push(part);
    }

    /// End the document. Gives back what it was written to, and what it
    /// holds.
    pub fn finish(mut self) -> Result<(W, Report)> {
        let unmet = self
            .proxies
            .values()
            .flatten()
            .filter(|c| !self.strays.contains_key(*c))
            .count();
        for _ in 0..unmet {
            self.losses
                .note("links to children outside their parent's feature");
        }
        for (_, held) in std::mem::take(&mut self.held) {
            if held.stray {
                self.losses.note(
                    "parts whose parent was not written as their parent (written on their own)",
                );
            }
            self.document.write(held.object)?;
        }
        for _ in self.awaited.drain() {
            self.losses
                .note("links to children outside their parent's feature");
        }
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
