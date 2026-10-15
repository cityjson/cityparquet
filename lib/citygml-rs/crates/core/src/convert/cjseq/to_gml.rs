//! [`cjseq::CityJSONFeature`] to [`GmlObject`]s: the way back.
//!
//! A feature's root object, and any other object of it with no parent in
//! the feature (a group member the feature holds for a group elsewhere),
//! each become a top-level object; the rest nest under their first parent
//! in the feature, as [`super`]'s conversion puts them. What the version
//! calls each thing comes from its [`Vocabulary`]:
//!
//! - a City Object type becomes the class the vocabulary names, or, when
//!   the version has none, is left out and its children take its place;
//! - an attribute becomes the class's property of that name when the value
//!   fits the property, else a generic attribute (a JSON object a set of
//!   them, an array one per item);
//! - a semantic surface becomes a boundary surface of the class the
//!   vocabulary names for the owner, its members that class's properties
//!   or generic attributes as an object's are, and a face whose surface has
//!   no class is kept untagged;
//! - a `geographicalExtent` becomes the object's envelope;
//! - vertices are dequantised with the feature's transform;
//! - a geometry's materials and textures become the object's own
//!   appearance, one `X3DMaterial` or `ParameterizedTexture` per theme and
//!   palette entry, targeting the faces and rings they cover by generated
//!   `gml:id`s.
//!
//! What an Extension adds goes back into the ADE whose namespace the
//! Extension's declaration names ([`Extensions`]): a `+` City Object type
//! becomes a class of that namespace, a `+` semantic surface a boundary
//! surface, and a `+` attribute an XML property rebuilt from its JSON (the
//! reverse of [`super`]'s mapping, with the XML attributes the declaration
//! states once restored on every element). An object of an ADE class keeps
//! the properties every city object has as such, and every other attribute
//! becomes a property in its own namespace.
//!
//! Generated `gml:id`s are `<prefix>p<n>` for polygons and `<prefix>r<n>`
//! for rings, the prefix naming the feature (`_f3`) or template (`_t0`), so
//! they are unique in a document whose features are numbered. Ids that are
//! not XML names are made into one ([`ncname`]).

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use cjseq::{
    Appearance as CjAppearance, BorderColor, CityJSONFeature, CityObject, CityObjectAddress,
    Extension, Geometry as CjGeometry, GeometryTemplates, MaterialObject, SemanticsSurface,
    TextureFormat, TextureObject, TexturedRing, Transform, VertexIndex,
};
use serde_json::{Map, Value as Json};

use crate::convert::{Class, GmlFeature, Losses, Property, PropertyKind, ToGml, Vocabulary};
use crate::model::{
    Address, Appearance, Attribute, AttributeName, BoundarySurface, Face, Geometry, GmlObject,
    GroupMember, Instance, Material, MemberTarget, Namespace, Polygon, QName, RingUvs, Shape,
    Template, Texture, Value, XmlAttribute, XmlContent, XmlNode,
};
use crate::vocabulary::module_of_prefix;
use crate::{Error, Result};

/// How deep objects may nest in one feature, and JSON in one attribute.
const MAX_DEPTH: usize = 128;

/// What the conversion of one feature needs besides the feature.
#[derive(Clone)]
pub struct GmlContext {
    /// The transform the feature's vertices are quantised against.
    pub transform: Transform,
    /// The feature's position in the document, which names the `gml:id`s
    /// the conversion generates.
    pub index: usize,
    /// What the target version names.
    pub vocabulary: Arc<dyn Vocabulary>,
    /// The namespaces of the Extensions the document declares.
    pub extensions: Arc<Extensions>,
    /// The LoD of each geometry template: a GeometryInstance takes its
    /// template's.
    pub template_lods: Arc<[String]>,
}

impl ToGml for CityJSONFeature {
    type Context = GmlContext;

    fn to_gml(&self, cx: &GmlContext) -> Result<GmlFeature> {
        // An object's structural parent is its first parent in the feature:
        // [`super`]'s conversion puts a group that references it after it.
        let parent: HashMap<&str, &str> = self
            .city_objects
            .iter()
            .filter_map(|(id, co)| {
                let p = co
                    .parents
                    .iter()
                    .flatten()
                    .find(|p| *p != id && self.city_objects.contains_key(p.as_str()))?;
                Some((id.as_str(), p.as_str()))
            })
            .collect();
        let mut conv = Conv {
            vocabulary: cx.vocabulary.as_ref(),
            extensions: &cx.extensions,
            pool: Pool::Feature {
                vertices: &self.vertices,
                transform: &cx.transform,
            },
            palette: self.appearance.as_ref(),
            template_lods: &cx.template_lods,
            prefix: format!("_f{}", cx.index),
            polygons: 0,
            rings: 0,
            losses: Losses::default(),
            parent,
            visiting: HashSet::new(),
        };
        let roots = std::iter::once(self.id.as_str())
            .filter(|id| self.city_objects.contains_key(*id))
            .chain(
                self.city_objects
                    .keys()
                    .map(String::as_str)
                    .filter(|id| *id != self.id && !conv.parent.contains_key(id)),
            )
            .collect::<Vec<_>>();
        let mut objects = Vec::new();
        for id in roots {
            objects.extend(conv.object(self, id, 0)?.objects);
        }
        Ok(GmlFeature {
            objects,
            losses: conv.losses,
        })
    }
}

/// A document's geometry templates as the relative geometries
/// `ImplicitGeometry`s place, coloured by the header `appearance` their
/// material and texture values index. A template's semantics are left out:
/// no class owns them.
pub fn templates_to_gml(
    templates: &GeometryTemplates,
    appearance: Option<&CjAppearance>,
) -> Result<(Vec<Template>, Losses)> {
    let mut losses = Losses::default();
    let mut out = Vec::with_capacity(templates.templates.len());
    let no_templates: Arc<[String]> = Arc::from(Vec::new());
    let extensions = Extensions::default();
    for (i, g) in templates.templates.iter().enumerate() {
        let mut conv = Conv {
            vocabulary: &NoVocabulary,
            extensions: &extensions,
            pool: Pool::Template(&templates.vertices_templates),
            palette: appearance,
            template_lods: &no_templates,
            prefix: format!("_t{i}"),
            polygons: 0,
            rings: 0,
            losses: Losses::default(),
            parent: HashMap::new(),
            visiting: HashSet::new(),
        };
        let mut surfaces = Vec::new();
        let mut app = AppearanceBuilder::default();
        let geometry = conv
            .geometry(g, None, &mut surfaces, &mut app)?
            .ok_or_else(|| Error::invalid("a geometry template is itself a GeometryInstance"))?;
        let appearance = app.finish(conv.palette)?;
        losses.merge(conv.losses);
        out.push(Template {
            id: Some(format!("_t{i}")),
            geometry,
            appearance,
        });
    }
    Ok((out, losses))
}

/// The LoD of each geometry template, for [`GmlContext::template_lods`].
pub fn template_lods(templates: &GeometryTemplates) -> Arc<[String]> {
    templates
        .templates
        .iter()
        .map(|g| lod_of(g).unwrap_or_default())
        .collect()
}

fn lod_of(g: &CjGeometry) -> Option<String> {
    Some(match g {
        CjGeometry::MultiPoint { lod, .. }
        | CjGeometry::MultiLineString { lod, .. }
        | CjGeometry::MultiSurface { lod, .. }
        | CjGeometry::CompositeSurface { lod, .. }
        | CjGeometry::Solid { lod, .. }
        | CjGeometry::MultiSolid { lod, .. }
        | CjGeometry::CompositeSolid { lod, .. } => lod.to_string(),
        _ => return None,
    })
}

/// The vocabulary of a template, which belongs to no class.
struct NoVocabulary;

impl Vocabulary for NoVocabulary {
    fn class(&self, _: &str, _: &dyn Fn(&str) -> Option<String>) -> Option<Class> {
        None
    }
    fn surface(&self, _: &str, _: &QName) -> Option<QName> {
        None
    }
    fn property(&self, _: &QName, _: &str) -> Option<Property> {
        None
    }
}

/// `id` as an XML name (an `NCName`), which `gml:id` must be: each
/// character a name may not hold becomes `_`, and one that may not start a
/// name is preceded by `_`.
pub fn ncname(id: &str) -> Cow<'_, str> {
    let start = |c: char| c == '_' || c.is_alphabetic();
    let inner = |c: char| start(c) || c == '-' || c == '.' || c.is_alphanumeric();
    let valid = id.chars().next().is_some_and(start) && id.chars().all(inner);
    if valid {
        return Cow::Borrowed(id);
    }
    let mut out = String::with_capacity(id.len() + 1);
    if !id.chars().next().is_some_and(start) {
        out.push('_');
    }
    out.extend(id.chars().map(|c| if inner(c) { c } else { '_' }));
    Cow::Owned(out)
}

/// The XML namespaces of a document's CityJSON Extensions, and what each
/// declares: what a `+` name goes back into.
#[derive(Debug, Clone, Default)]
pub struct Extensions {
    list: Vec<ExtensionNs>,
}

/// One Extension, as CityGML output needs it.
#[derive(Debug, Clone)]
pub struct ExtensionNs {
    /// The Extension's name in the document, which becomes its namespace's
    /// prefix.
    pub prefix: String,
    /// Its XML namespace, when the declaration or the caller gives one.
    pub xmlns: Option<String>,
    /// Its `url`, the schema a CityGML document names for the namespace
    /// when it is an XML Schema.
    pub url: String,
    /// The XML attributes every occurrence of an element carries with one
    /// value, by element and attribute name.
    xml_attributes: BTreeMap<String, BTreeMap<String, String>>,
    classes: BTreeSet<String>,
    /// The definition of CityJSON's schema each class specialises
    /// (`_AbstractBuilding`), as its declaration's `allOf` refers to it.
    parents: BTreeMap<String, String>,
    surfaces: BTreeSet<String>,
    /// Attribute names, by City Object type.
    attributes: BTreeMap<String, BTreeSet<String>>,
}

impl Extensions {
    /// The Extensions a CityJSON header declares. `xmlns` gives the XML
    /// namespace of an Extension, by name, where the declaration does not
    /// (a CityJSON Extension never does; one read from CityGML has
    /// `xmlns`), or overrides it.
    pub fn new<'a>(
        declared: impl IntoIterator<Item = (&'a String, &'a Extension)>,
        xmlns: &BTreeMap<String, String>,
    ) -> Self {
        let names = |v: Option<&Json>| -> BTreeSet<String> {
            v.and_then(Json::as_object)
                .map(|m| m.keys().cloned().collect())
                .unwrap_or_default()
        };
        let list = declared
            .into_iter()
            .map(|(name, ext)| ExtensionNs {
                prefix: name.clone(),
                xmlns: xmlns.get(name).cloned().or_else(|| {
                    ext.other
                        .get("xmlns")
                        .and_then(Json::as_str)
                        .map(str::to_string)
                }),
                url: ext.url.clone(),
                xml_attributes: ext
                    .other
                    .get("xmlAttributes")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default(),
                classes: names(ext.other.get("extraCityObjects")),
                parents: ext
                    .other
                    .get("extraCityObjects")
                    .and_then(Json::as_object)
                    .map(|classes| {
                        classes
                            .iter()
                            .filter_map(|(name, schema)| {
                                let reference = schema["allOf"][0]["$ref"].as_str()?;
                                let (_, definition) = reference.split_once("#/")?;
                                Some((name.clone(), definition.to_string()))
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
                surfaces: names(ext.other.get("extraSemanticSurfaces")),
                attributes: ext
                    .other
                    .get("extraAttributes")
                    .and_then(Json::as_object)
                    .map(|by_type| {
                        by_type
                            .iter()
                            .map(|(ty, v)| (ty.clone(), names(Some(v))))
                            .collect()
                    })
                    .unwrap_or_default(),
            })
            .collect();
        Self { list }
    }

    /// Every Extension, in the order the header declares them.
    pub fn iter(&self) -> impl Iterator<Item = &ExtensionNs> {
        self.list.iter()
    }

    /// The Extension whose XML namespace is `xmlns`.
    pub fn by_xmlns(&self, xmlns: &str) -> Option<&ExtensionNs> {
        self.list.iter().find(|e| e.xmlns.as_deref() == Some(xmlns))
    }

    /// The XML attributes the Extension of namespace `xmlns` states once for
    /// every `element`.
    fn constant_attributes(&self, xmlns: &str, element: &str) -> Option<&BTreeMap<String, String>> {
        self.by_xmlns(xmlns)?.xml_attributes.get(element)
    }

    /// The namespace of the `+` name `name` (with its `+`), declared by the
    /// Extension `declares` picks; with none, the only Extension's.
    fn namespace(&self, name: &str, declares: impl Fn(&ExtensionNs) -> bool) -> Result<String> {
        let ext = match self.list.iter().find(|e| declares(e)) {
            Some(ext) => ext,
            None if self.list.len() == 1 => &self.list[0],
            None => {
                return Err(Error::invalid(format!(
                    "no CityJSON Extension declares {name}, so it has no XML namespace"
                )));
            }
        };
        ext.xmlns.clone().ok_or_else(|| {
            Error::invalid(format!(
                "the CityJSON Extension {:?} has no XML namespace for CityGML: give it one \
                 (an `xmlns` member of its declaration, or the writer's namespace option)",
                ext.prefix
            ))
        })
    }

    /// The CityJSON City Object type whose CityGML class the Extension
    /// class `name` (with its `+`) specialises, as its declaration says:
    /// `Building` for one that refers to `_AbstractBuilding`. `None` for
    /// one that refers to no CityGML class.
    pub fn base_type(&self, name: &str) -> Option<&str> {
        let definition = self.list.iter().find_map(|e| e.parents.get(name))?;
        match definition.as_str() {
            "_AbstractBuilding" => Some("Building"),
            "_AbstractTransportationComplex" => Some("Road"),
            "_AbstractCityObject" => None,
            other if other.starts_with('_') => None,
            other => Some(other),
        }
    }

    fn class_ns(&self, name: &str) -> Result<String> {
        self.namespace(name, |e| e.classes.contains(name))
    }

    fn surface_ns(&self, name: &str) -> Result<String> {
        self.namespace(name, |e| e.surfaces.contains(name))
    }

    fn attribute_ns(&self, object_type: &str, name: &str) -> Result<String> {
        let on_type = self.list.iter().any(|e| {
            e.attributes
                .get(object_type)
                .is_some_and(|s| s.contains(name))
        });
        self.namespace(name, |e| {
            if on_type {
                e.attributes
                    .get(object_type)
                    .is_some_and(|s| s.contains(name))
            } else {
                e.attributes.values().any(|s| s.contains(name))
            }
        })
    }

    /// The namespace a prefix names inside an ADE property: an Extension's
    /// name, a CityGML module's usual prefix, or `gml`.
    fn prefixed(&self, prefix: &str) -> Option<Namespace> {
        if prefix == "gml" {
            return Some(Namespace::Gml);
        }
        if let Some(ns) = self
            .list
            .iter()
            .find(|e| e.prefix == prefix)
            .and_then(|e| e.xmlns.clone())
        {
            return Some(Namespace::Other(ns));
        }
        module_of_prefix(prefix).map(Namespace::Module)
    }
}

/// Where a geometry's vertex indices point.
enum Pool<'a> {
    /// A feature's quantised vertices.
    Feature {
        vertices: &'a [[i64; 3]],
        transform: &'a Transform,
    },
    /// The templates' real coordinates.
    Template(&'a [[f64; 3]]),
}

impl Pool<'_> {
    fn get(&self, i: VertexIndex) -> Result<[f64; 3]> {
        let missing = || Error::invalid(format!("vertex index {} out of range", i.0));
        Ok(match self {
            Pool::Feature {
                vertices,
                transform,
            } => {
                let v = vertices.get(i.0).ok_or_else(missing)?;
                std::array::from_fn(|k| v[k] as f64 * transform.scale[k] + transform.translate[k])
            }
            Pool::Template(vertices) => *vertices.get(i.0).ok_or_else(missing)?,
        })
    }
}

/// One feature's (or template's) conversion.
struct Conv<'a> {
    vocabulary: &'a dyn Vocabulary,
    extensions: &'a Extensions,
    pool: Pool<'a>,
    /// The appearance the geometries' material and texture values index.
    palette: Option<&'a CjAppearance>,
    template_lods: &'a [String],
    /// What generated `gml:id`s begin with.
    prefix: String,
    polygons: usize,
    rings: usize,
    losses: Losses,
    /// Each object's structural parent in the feature.
    parent: HashMap<&'a str, &'a str>,
    /// The objects being converted, nearest last: a guard against cycles.
    visiting: HashSet<&'a str>,
}

/// What one City Object became: itself, or, when its type has no class,
/// its children in its place.
struct Built {
    objects: Vec<GmlObject>,
    /// Whether `objects` is the object itself.
    own: bool,
}

impl<'a> Conv<'a> {
    fn object(&mut self, feature: &'a CityJSONFeature, id: &'a str, depth: usize) -> Result<Built> {
        if depth > MAX_DEPTH {
            return Err(Error::invalid(format!(
                "City Objects nested deeper than {MAX_DEPTH}"
            )));
        }
        if !self.visiting.insert(id) {
            return Err(Error::invalid(format!(
                "City Object {id:?} is its own ancestor"
            )));
        }
        let co = &feature.city_objects[id];
        let ty = String::from(co.thetype.clone());
        let class = self.class(&ty, co)?;

        let mut children = Vec::new();
        // Where each child that stayed itself landed in `children`.
        let mut child_at: HashMap<&str, usize> = HashMap::new();
        for cid in co.children.iter().flatten() {
            if self.parent.get(cid.as_str()) != Some(&id) {
                continue;
            }
            let built = self.object(feature, cid, depth + 1)?;
            if built.own {
                child_at.insert(cid, children.len());
            }
            children.extend(built.objects);
        }
        let Some(class) = class else {
            self.visiting.remove(id);
            return Ok(Built {
                objects: children,
                own: false,
            });
        };

        let own_ade = match &class.name.ns {
            Namespace::Other(ns) => Some(ns.clone()),
            _ => None,
        };
        // The CityGML class whose properties and boundary surfaces the
        // object has: its own, or the one an ADE class specialises.
        let content = match own_ade {
            None => Some(class.name.clone()),
            Some(_) => self
                .extensions
                .base_type(&ty)
                .and_then(|base| self.vocabulary.class(base, &|_| None))
                .map(|c| c.name),
        };
        // The parents after the structural one are the groups that
        // reference the object.
        let groups = co
            .parents
            .iter()
            .flatten()
            .filter(|p| self.parent.get(id) != Some(&p.as_str()))
            .map(|p| ncname(p).into_owned())
            .collect();
        let mut object = GmlObject {
            id: Some(ncname(id).into_owned()),
            class: class.name.clone(),
            attributes: Vec::new(),
            ade: Vec::new(),
            addresses: Vec::new(),
            extent: co.geographical_extent,
            geometries: Vec::new(),
            boundary_surfaces: Vec::new(),
            appearance: Appearance::default(),
            children: Vec::new(),
            members: Vec::new(),
            groups,
        };
        self.attributes(
            &class,
            content.as_ref(),
            &ty,
            co,
            own_ade.as_deref(),
            &mut object,
        )?;
        for address in co.address.iter().flatten() {
            object.addresses.push(self.address(address)?);
        }
        let mut app = AppearanceBuilder::default();
        for g in co.geometry.iter().flatten() {
            if let Some(geometry) = self.geometry(
                g,
                Some(content.as_ref().unwrap_or(&class.name)),
                &mut object.boundary_surfaces,
                &mut app,
            )? {
                object.geometries.push(geometry);
            }
        }
        object.appearance = app.finish(self.palette)?;

        if ty == "CityObjectGroup" {
            let roles = co.children_roles.as_deref().unwrap_or_default();
            for (i, cid) in co.children.iter().flatten().enumerate() {
                let target = match child_at.get(cid.as_str()) {
                    Some(&k) => MemberTarget::Child(k),
                    None => MemberTarget::Id(ncname(cid).into_owned()),
                };
                object.members.push(GroupMember {
                    target,
                    role: roles.get(i).cloned().flatten(),
                });
            }
        } else {
            let elsewhere = co
                .children
                .iter()
                .flatten()
                .filter(|c| self.parent.get(c.as_str()) != Some(&id))
                .count();
            for _ in 0..elsewhere {
                self.losses
                    .note("links to children outside their parent's feature");
            }
        }
        object.children = children;
        self.visiting.remove(id);
        Ok(Built {
            objects: vec![object],
            own: true,
        })
    }

    /// The class of an object of CityJSON type `ty`, or `None` (noted as a
    /// loss) when the version has none.
    fn class(&mut self, ty: &str, co: &CityObject) -> Result<Option<Class>> {
        if let Some(local) = ty.strip_prefix('+') {
            let ns = self.extensions.class_ns(ty)?;
            return Ok(Some(Class {
                name: QName::new(Namespace::Other(ns), local),
                implied: &[],
            }));
        }
        let attribute = |name: &str| {
            co.attributes
                .as_ref()
                .and_then(|a| a.get(name))
                .and_then(Json::as_str)
                .map(str::to_string)
        };
        let class = self.vocabulary.class(ty, &attribute);
        if class.is_none() {
            self.losses.note(format!(
                "{ty} objects, which have no class in this CityGML version (their children are \
                 kept)"
            ));
        }
        Ok(class)
    }

    /// `content` is the class whose properties the object has, which for an
    /// ADE class is the CityGML class it specialises, if any.
    fn attributes(
        &mut self,
        class: &Class,
        content: Option<&QName>,
        ty: &str,
        co: &CityObject,
        own_ade: Option<&str>,
        object: &mut GmlObject,
    ) -> Result<()> {
        for (key, value) in co.attributes.iter().flatten() {
            if class.implied.contains(&key.as_str()) {
                continue;
            }
            if value.is_null() {
                self.losses.note("attributes whose value is null");
                continue;
            }
            if key.strip_prefix('+').is_some_and(|n| !n.is_empty()) {
                let ns = self.extensions.attribute_ns(ty, key)?;
                let name = QName::new(Namespace::Other(ns), &key[1..]);
                object.ade.extend(self.xml_nodes(name, value, 0)?);
                continue;
            }
            let property = match content {
                Some(content) => self.vocabulary.property(content, key),
                None => self.vocabulary.property(&class.name, key),
            };
            if let Some(property) = property
                && let Some(values) = fit(&property, value)
            {
                object
                    .attributes
                    .extend(values.into_iter().map(|value| Attribute {
                        name: AttributeName::Property(property.qname()),
                        value,
                    }));
                continue;
            }
            if let Some(ns) = own_ade {
                let name = QName::new(Namespace::Other(ns.to_string()), key.as_str());
                object.ade.extend(self.xml_nodes(name, value, 0)?);
                continue;
            }
            object.attributes.extend(self.generic(key, value, 0)?);
        }
        Ok(())
    }

    /// An attribute that is no property of the class: one generic attribute,
    /// one per item of an array, or a set for an object.
    fn generic(&mut self, key: &str, value: &Json, depth: usize) -> Result<Vec<Attribute>> {
        if depth > MAX_DEPTH {
            return Err(Error::invalid(format!(
                "attribute {key:?} nested deeper than {MAX_DEPTH}"
            )));
        }
        let name = || AttributeName::Generic(key.to_string());
        Ok(match value {
            Json::Null => {
                self.losses.note("attributes whose value is null");
                Vec::new()
            }
            Json::Array(items) => {
                let mut out = Vec::new();
                for item in items {
                    if item.is_array() {
                        self.losses
                            .note("attribute values that are arrays of arrays");
                        continue;
                    }
                    out.extend(self.generic(key, item, depth + 1)?);
                }
                out
            }
            Json::Object(map) => {
                let mut members = Vec::new();
                for (k, v) in map {
                    members.extend(self.generic(k, v, depth + 1)?);
                }
                vec![Attribute {
                    name: name(),
                    value: Value::Set(members),
                }]
            }
            scalar => vec![Attribute {
                name: name(),
                value: scalar_value(scalar).expect("a scalar"),
            }],
        })
    }

    /// An Extension attribute's JSON as the XML elements it came from: one
    /// per item of an array.
    fn xml_nodes(&mut self, name: QName, value: &Json, depth: usize) -> Result<Vec<XmlNode>> {
        if depth > MAX_DEPTH {
            return Err(Error::invalid(format!(
                "Extension attribute nested deeper than {MAX_DEPTH}"
            )));
        }
        match value {
            Json::Null => Ok(Vec::new()),
            Json::Array(items) => {
                let mut out = Vec::new();
                for item in items {
                    out.extend(self.xml_nodes(name.clone(), item, depth + 1)?);
                }
                Ok(out)
            }
            Json::Object(map) => {
                let mut attributes = Vec::new();
                let mut text = None;
                let mut children = Vec::new();
                for (key, v) in map {
                    if let Some(attr) = key.strip_prefix('@') {
                        attributes.push(XmlAttribute {
                            name: attr.to_string(),
                            value: scalar_text(v).unwrap_or_default(),
                        });
                    } else if key == "$" {
                        text = scalar_text(v);
                    } else {
                        let child = self.child_name(key, &name.ns);
                        children.extend(self.xml_nodes(child, v, depth + 1)?);
                    }
                }
                if text.is_some() && !children.is_empty() {
                    self.losses
                        .note("Extension attribute text beside child elements");
                }
                let content = if !children.is_empty() {
                    XmlContent::Children(children)
                } else {
                    text.map_or(XmlContent::Empty, XmlContent::Text)
                };
                Ok(vec![self.node(name, attributes, content)])
            }
            scalar => {
                let content = XmlContent::Text(scalar_text(scalar).unwrap_or_default());
                Ok(vec![self.node(name, Vec::new(), content)])
            }
        }
    }

    /// An element, with the XML attributes its Extension states once put
    /// back ahead of its own.
    fn node(&self, name: QName, mut attributes: Vec<XmlAttribute>, content: XmlContent) -> XmlNode {
        if let Namespace::Other(ns) = &name.ns
            && let Some(constant) = self.extensions.constant_attributes(ns, &name.local)
        {
            let mut all: Vec<XmlAttribute> = constant
                .iter()
                .filter(|(n, _)| !attributes.iter().any(|a| &a.name == *n))
                .map(|(n, v)| XmlAttribute {
                    name: n.clone(),
                    value: v.clone(),
                })
                .collect();
            all.append(&mut attributes);
            attributes = all;
        }
        XmlNode {
            name,
            attributes,
            content,
        }
    }

    /// The element a key inside an Extension attribute names: `local` in
    /// its parent's namespace, `prefix:local` in the namespace the prefix
    /// names, or `{uri}local`.
    fn child_name(&self, key: &str, parent: &Namespace) -> QName {
        if let Some(rest) = key.strip_prefix('{')
            && let Some((uri, local)) = rest.split_once('}')
        {
            return QName::new(Namespace::Other(uri.to_string()), local);
        }
        if let Some((prefix, local)) = key.split_once(':')
            && let Some(ns) = self.extensions.prefixed(prefix)
        {
            return QName::new(ns, local);
        }
        QName::new(parent.clone(), key)
    }

    fn address(&mut self, a: &CityObjectAddress) -> Result<Address> {
        let mut address = Address::default();
        for (key, value) in &a.other {
            let leaf = xal_name(key);
            let items: Vec<&Json> = match value {
                Json::Array(items) => items.iter().collect(),
                other => vec![other],
            };
            for item in items {
                match scalar_text(item) {
                    Some(text) => address.fields.push((leaf.clone(), text)),
                    None => self.losses.note("address members that are not text"),
                }
            }
        }
        match &a.location {
            None => {}
            Some(CjGeometry::MultiPoint { boundaries, .. }) => {
                for &i in boundaries {
                    address.location.push(self.pool.get(i)?);
                }
            }
            Some(_) => self
                .losses
                .note("address locations that are not a MultiPoint"),
        }
        Ok(address)
    }

    fn next_polygon(&mut self) -> String {
        self.polygons += 1;
        format!("{}p{}", self.prefix, self.polygons)
    }

    fn next_ring(&mut self) -> String {
        self.rings += 1;
        format!("{}r{}", self.prefix, self.rings)
    }

    fn face(&mut self, rings: &[Vec<VertexIndex>], identified: bool) -> Result<Face> {
        let mut coords = rings
            .iter()
            .map(|ring| ring.iter().map(|&i| self.pool.get(i)).collect())
            .collect::<Result<Vec<Vec<[f64; 3]>>>>()?;
        if coords.is_empty() {
            return Err(Error::invalid("a surface with no ring"));
        }
        let exterior = coords.remove(0);
        let id = identified.then(|| self.next_polygon());
        Ok(Face {
            polygon: Polygon {
                id: id.clone(),
                exterior,
                ring_ids: vec![None; coords.len() + 1],
                interiors: coords,
                groups: Vec::new(),
            },
            id,
            reversed: false,
            surface: None,
        })
    }

    /// One geometry, its semantic surfaces added to `surfaces` and its
    /// materials and textures to `app`; `None` for a GeometryInstance of a
    /// template (which has no owner) and a geometry type the model lacks.
    fn geometry(
        &mut self,
        g: &CjGeometry,
        owner: Option<&QName>,
        surfaces: &mut Vec<BoundarySurface>,
        app: &mut AppearanceBuilder,
    ) -> Result<Option<Geometry>> {
        macro_rules! faces {
            ($boundaries:expr, $identified:expr) => {
                $boundaries
                    .iter()
                    .map(|s| self.face(s, $identified))
                    .collect::<Result<Vec<Face>>>()?
            };
        }
        let (lod, mut shape, semantics, materials, textures) = match g {
            CjGeometry::MultiPoint {
                lod, boundaries, ..
            } => {
                let points = boundaries
                    .iter()
                    .map(|&i| self.pool.get(i))
                    .collect::<Result<_>>()?;
                return Ok(Some(Geometry {
                    lod: lod.to_string(),
                    shape: Shape::MultiPoint(points),
                }));
            }
            CjGeometry::MultiLineString {
                lod, boundaries, ..
            } => {
                let lines = boundaries
                    .iter()
                    .map(|l| l.iter().map(|&i| self.pool.get(i)).collect())
                    .collect::<Result<_>>()?;
                return Ok(Some(Geometry {
                    lod: lod.to_string(),
                    shape: Shape::MultiLineString(lines),
                }));
            }
            CjGeometry::MultiSurface {
                lod,
                boundaries,
                semantics,
                material,
                texture,
            }
            | CjGeometry::CompositeSurface {
                lod,
                boundaries,
                semantics,
                material,
                texture,
            } => {
                let identified = semantics.is_some() || material.is_some() || texture.is_some();
                let faces = faces!(boundaries, identified);
                let shape = if matches!(g, CjGeometry::MultiSurface { .. }) {
                    Shape::MultiSurface(faces)
                } else {
                    Shape::CompositeSurface(faces)
                };
                (
                    lod,
                    shape,
                    semantics
                        .as_ref()
                        .map(|s| (&s.surfaces, flat(s.values.as_ref()))),
                    themes(material, |m| {
                        m.value
                            .map_or_else(|| flat(m.values.clone().flatten().as_ref()), Uniform::All)
                    }),
                    themes(texture, |t| t.values.clone()),
                )
            }
            CjGeometry::Solid {
                lod,
                boundaries,
                semantics,
                material,
                texture,
            } => {
                let identified = semantics.is_some() || material.is_some() || texture.is_some();
                let mut shells = Vec::with_capacity(boundaries.len());
                for shell in boundaries {
                    shells.push(faces!(shell, identified));
                }
                let sizes: Vec<usize> = boundaries.iter().map(Vec::len).collect();
                (
                    lod,
                    Shape::Solid(shells),
                    semantics
                        .as_ref()
                        .map(|s| (&s.surfaces, flat_solid(s.values.as_ref(), &sizes))),
                    themes(material, |m| {
                        m.value.map_or_else(
                            || flat_solid(m.values.clone().flatten().as_ref(), &sizes),
                            Uniform::All,
                        )
                    }),
                    themes(texture, |t| t.values.clone().map(|v| v.concat())),
                )
            }
            CjGeometry::MultiSolid {
                lod,
                boundaries,
                semantics,
                material,
                texture,
            }
            | CjGeometry::CompositeSolid {
                lod,
                boundaries,
                semantics,
                material,
                texture,
            } => {
                let identified = semantics.is_some() || material.is_some() || texture.is_some();
                let mut solids = Vec::with_capacity(boundaries.len());
                for solid in boundaries {
                    let mut shells = Vec::with_capacity(solid.len());
                    for shell in solid {
                        shells.push(faces!(shell, identified));
                    }
                    solids.push(shells);
                }
                let shape = if matches!(g, CjGeometry::MultiSolid { .. }) {
                    Shape::MultiSolid(solids)
                } else {
                    Shape::CompositeSolid(solids)
                };
                let sizes: Vec<Vec<usize>> = boundaries
                    .iter()
                    .map(|solid| solid.iter().map(Vec::len).collect())
                    .collect();
                (
                    lod,
                    shape,
                    semantics
                        .as_ref()
                        .map(|s| (&s.surfaces, flat_multi_solid(s.values.as_ref(), &sizes))),
                    themes(material, |m| {
                        m.value.map_or_else(
                            || flat_multi_solid(m.values.clone().flatten().as_ref(), &sizes),
                            Uniform::All,
                        )
                    }),
                    themes(texture, |t| {
                        t.values
                            .clone()
                            .map(|v| v.into_iter().flatten().flatten().collect())
                    }),
                )
            }
            CjGeometry::GeometryInstance {
                boundaries,
                template,
                transformation_matrix,
            } => {
                let Some(lod) = self.template_lods.get(template.0) else {
                    return Err(Error::invalid(format!(
                        "GeometryInstance of template {}, which the document does not have",
                        template.0
                    )));
                };
                return Ok(Some(Geometry {
                    lod: lod.clone(),
                    shape: Shape::Instance(Instance {
                        template: template.0,
                        matrix: *transformation_matrix,
                        reference_point: self.pool.get(boundaries[0])?,
                    }),
                }));
            }
            _ => {
                self.losses.note("geometries of a type the model lacks");
                return Ok(None);
            }
        };
        let mut faces = faces_mut(&mut shape);
        if let Some((declared, values)) = semantics {
            let map = self.surfaces(declared, owner, surfaces)?;
            let values = values.expand(faces.len(), "semantics")?;
            for (face, value) in faces.iter_mut().zip(values) {
                face.surface = value.and_then(|v| map.get(v).copied().flatten());
            }
        }
        for (theme, values) in materials {
            let values = values.expand(faces.len(), "material")?;
            for (face, value) in faces.iter().zip(values) {
                if let (Some(m), Some(id)) = (value, &face.id) {
                    app.material(&theme, m, id.clone());
                }
            }
        }
        for (theme, per_face) in textures {
            let Some(per_face) = per_face else {
                continue;
            };
            if per_face.len() != faces.len() {
                return Err(Error::invalid(format!(
                    "texture values for {} surfaces on a geometry of {}",
                    per_face.len(),
                    faces.len()
                )));
            }
            for (face, rings) in faces.iter_mut().zip(per_face) {
                for (r, ring) in rings.into_iter().enumerate() {
                    let TexturedRing::Textured { texture, uvs } = ring else {
                        continue;
                    };
                    let length = match r {
                        0 => face.polygon.exterior.len(),
                        _ => face
                            .polygon
                            .interiors
                            .get(r - 1)
                            .map_or(usize::MAX, Vec::len),
                    };
                    if uvs.len() != length {
                        return Err(Error::invalid(format!(
                            "{} texture coordinates for a ring of {length} vertices",
                            uvs.len()
                        )));
                    }
                    let uv = |i: cjseq::UvIndex| {
                        self.palette
                            .and_then(|p| p.vertices_texture.as_ref())
                            .and_then(|v| v.get(i.0))
                            .copied()
                            .ok_or_else(|| {
                                Error::invalid(format!("texture vertex index {} out of range", i.0))
                            })
                    };
                    let coords = uvs.into_iter().map(uv).collect::<Result<Vec<_>>>()?;
                    let ring_id = match &face.polygon.ring_ids[r] {
                        Some(id) => id.clone(),
                        None => {
                            let id = self.next_ring();
                            face.polygon.ring_ids[r] = Some(id.clone());
                            id
                        }
                    };
                    app.texture(&theme, texture.0, (ring_id, coords));
                }
            }
        }
        Ok(Some(Geometry {
            lod: lod.to_string(),
            shape,
        }))
    }

    /// Add a geometry's semantic surfaces to the object's, and say where
    /// each landed: `None` for one this version has no class for, whose
    /// faces stay untagged.
    fn surfaces(
        &mut self,
        declared: &[SemanticsSurface],
        owner: Option<&QName>,
        surfaces: &mut Vec<BoundarySurface>,
    ) -> Result<Vec<Option<usize>>> {
        let mut map = Vec::with_capacity(declared.len());
        for s in declared {
            let ty = String::from(s.thetype.clone());
            let class = match (ty.strip_prefix('+'), owner) {
                (Some(local), Some(_)) => match self.extensions.surface_ns(&ty) {
                    Ok(ns) => Some(QName::new(Namespace::Other(ns), local)),
                    Err(_) => None,
                },
                (None, Some(owner)) => self.vocabulary.surface(&ty, owner),
                (_, None) => None,
            };
            match class {
                Some(class) => {
                    map.push(Some(surfaces.len()));
                    let attributes = self.surface_attributes(&class, &s.other)?;
                    surfaces.push(BoundarySurface {
                        class,
                        parent: None,
                        attributes,
                    });
                }
                None => {
                    self.losses.note(format!(
                        "{ty} semantic surfaces, which have no class here (their faces are kept)"
                    ));
                    map.push(None);
                }
            }
        }
        for (s, me) in declared.iter().zip(&map) {
            if let (Some(me), Some(parent)) = (me, s.parent) {
                surfaces[*me].parent = map.get(parent.0).copied().flatten();
            }
        }
        Ok(map)
    }

    /// A semantic surface's attributes on boundary surface class `class`:
    /// a property of the class where the value fits it, else a generic
    /// attribute. An Extension attribute is left out, since the model keeps
    /// no ADE element on a surface.
    fn surface_attributes(
        &mut self,
        class: &QName,
        other: &Map<String, Json>,
    ) -> Result<Vec<Attribute>> {
        let mut out = Vec::new();
        for (key, value) in other {
            if value.is_null() {
                self.losses.note("attributes whose value is null");
            } else if key.starts_with('+') {
                self.losses
                    .note("Extension attributes of semantic surfaces");
            } else if let Some(property) = self.vocabulary.property(class, key)
                && let Some(values) = fit(&property, value)
            {
                out.extend(values.into_iter().map(|value| Attribute {
                    name: AttributeName::Property(property.qname()),
                    value,
                }));
            } else {
                out.extend(self.generic(key, value, 0)?);
            }
        }
        Ok(out)
    }
}

/// A geometry's values (semantics or one theme's materials) for each face
/// in order: given per face, or one for every face.
enum Uniform {
    Faces(Option<Vec<Option<usize>>>),
    All(cjseq::MaterialIndex),
}

impl Uniform {
    fn expand(self, faces: usize, what: &str) -> Result<Vec<Option<usize>>> {
        match self {
            Uniform::All(m) => Ok(vec![Some(m.0); faces]),
            Uniform::Faces(None) => Ok(vec![None; faces]),
            Uniform::Faces(Some(values)) if values.len() == faces => Ok(values),
            Uniform::Faces(Some(values)) => Err(Error::invalid(format!(
                "{what} values for {} surfaces on a geometry of {faces}",
                values.len()
            ))),
        }
    }
}

trait Index: Copy {
    fn index(self) -> usize;
}
impl Index for cjseq::SurfaceIndex {
    fn index(self) -> usize {
        self.0
    }
}
impl Index for cjseq::MaterialIndex {
    fn index(self) -> usize {
        self.0
    }
}

fn flat<I: Index>(values: Option<&Vec<Option<I>>>) -> Uniform {
    Uniform::Faces(values.map(|v| v.iter().map(|i| i.map(Index::index)).collect()))
}

/// A solid's values in face order, a `null` shell standing for no value on
/// each of its faces; `sizes` gives each shell's face count.
fn flat_solid<I: Index>(values: Option<&Vec<Option<Vec<Option<I>>>>>, sizes: &[usize]) -> Uniform {
    Uniform::Faces(values.map(|shells| flat_shells(shells, sizes)))
}

fn flat_shells<I: Index>(shells: &[Option<Vec<Option<I>>>], sizes: &[usize]) -> Vec<Option<usize>> {
    let mut out = Vec::new();
    for (k, shell) in shells.iter().enumerate() {
        match shell {
            Some(faces) => out.extend(faces.iter().map(|i| i.map(Index::index))),
            None => out.extend(std::iter::repeat_n(
                None,
                sizes.get(k).copied().unwrap_or(0),
            )),
        }
    }
    out
}

/// A Solid's values: per shell, per face.
type ShellValues<I> = Vec<Option<Vec<Option<I>>>>;

/// A MultiSolid's or CompositeSolid's values in face order; `sizes` gives
/// each solid's shells' face counts.
fn flat_multi_solid<I: Index>(
    values: Option<&Vec<Option<ShellValues<I>>>>,
    sizes: &[Vec<usize>],
) -> Uniform {
    Uniform::Faces(values.map(|solids| {
        let mut out = Vec::new();
        for (k, solid) in solids.iter().enumerate() {
            let shells = sizes.get(k).map_or(&[][..], Vec::as_slice);
            match solid {
                Some(solid) => out.extend(flat_shells(solid, shells)),
                None => out.extend(std::iter::repeat_n(None, shells.iter().sum())),
            }
        }
        out
    }))
}

/// Each theme of a geometry's `material` or `texture` with its values.
fn themes<R, T>(map: &Option<cjseq::Themes<R>>, values: impl Fn(&R) -> T) -> Vec<(String, T)> {
    map.iter()
        .flatten()
        .map(|(theme, r)| (theme.clone(), values(r)))
        .collect()
}

/// Every face of a shape, in order.
fn faces_mut(shape: &mut Shape) -> Vec<&mut Face> {
    match shape {
        Shape::MultiSurface(faces) | Shape::CompositeSurface(faces) => faces.iter_mut().collect(),
        Shape::Solid(shells) => shells.iter_mut().flatten().collect(),
        Shape::CompositeSolid(solids) | Shape::MultiSolid(solids) => {
            solids.iter_mut().flatten().flatten().collect()
        }
        Shape::MultiPoint(_) | Shape::MultiLineString(_) | Shape::Instance(_) => Vec::new(),
    }
}

/// The values a property is written with, or `None` when `value` does not
/// fit it and is written as a generic attribute instead.
fn fit(property: &Property, value: &Json) -> Option<Vec<Value>> {
    match (value, property.kind) {
        (Json::Array(items), PropertyKind::MeasureList) => {
            let numbers = items
                .iter()
                .map(|v| v.as_f64().map(Value::Double))
                .collect::<Option<Vec<_>>>()?;
            Some(vec![Value::List(numbers)])
        }
        (Json::Array(items), _) if property.repeated && !items.is_empty() => {
            items.iter().map(|v| fit_scalar(property.kind, v)).collect()
        }
        (Json::Array(_), _) => None,
        (scalar, kind) => fit_scalar(kind, scalar).map(|v| vec![v]),
    }
}

fn fit_scalar(kind: PropertyKind, value: &Json) -> Option<Value> {
    match (kind, value) {
        (PropertyKind::Text, Json::String(s)) => Some(Value::String(s.clone())),
        (PropertyKind::Date, Json::String(s)) if is_date(s) => Some(Value::String(s.clone())),
        (PropertyKind::Year, Json::String(s)) if is_year(s) => Some(Value::String(s.clone())),
        (PropertyKind::Integer { non_negative }, Json::Number(n)) => n
            .as_i64()
            .filter(|i| !non_negative || *i >= 0)
            .map(Value::Integer),
        (PropertyKind::Number { .. }, Json::Number(n)) => n.as_f64().map(Value::Double),
        (PropertyKind::Boolean, Json::Bool(b)) => Some(Value::Boolean(*b)),
        (PropertyKind::Enumeration(words), Json::String(s)) if words.contains(&s.as_str()) => {
            Some(Value::String(s.clone()))
        }
        _ => None,
    }
}

/// `xs:date`: `YYYY-MM-DD`, with an optional time zone.
fn is_date(s: &str) -> bool {
    let (body, _) = split_zone(s);
    let mut parts = body.splitn(3, '-');
    let digits = |p: Option<&str>, n: usize| p.is_some_and(|p| p.len() == n && all_digits(p));
    let year = parts.next().is_some_and(|y| y.len() >= 4 && all_digits(y));
    year && digits(parts.next(), 2) && digits(parts.next(), 2)
}

/// `xs:gYear`: a year of four or more digits, with an optional time zone.
fn is_year(s: &str) -> bool {
    let (body, _) = split_zone(s);
    body.len() >= 4 && all_digits(body)
}

fn all_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// A lexical date or year and its time zone (`Z`, `+09:00`).
fn split_zone(s: &str) -> (&str, &str) {
    if let Some(body) = s.strip_suffix('Z') {
        return (body, "Z");
    }
    if s.len() > 6 {
        let (body, zone) = s.split_at(s.len() - 6);
        let z = zone.as_bytes();
        if (z[0] == b'+' || z[0] == b'-') && z[3] == b':' {
            return (body, zone);
        }
    }
    (s, "")
}

/// A JSON scalar as a typed value.
fn scalar_value(v: &Json) -> Option<Value> {
    Some(match v {
        Json::String(s) => Value::String(s.clone()),
        Json::Bool(b) => Value::Boolean(*b),
        Json::Number(n) => match n.as_i64() {
            Some(i) => Value::Integer(i),
            None => Value::Double(n.as_f64()?),
        },
        _ => return None,
    })
}

/// A JSON scalar as XML text.
fn scalar_text(v: &Json) -> Option<String> {
    match v {
        Json::String(s) => Some(s.clone()),
        Json::Bool(b) => Some(b.to_string()),
        Json::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// The xAL leaf a CityJSON address member names: the reverse of
/// [`super`]'s naming, and otherwise the member with its first letter
/// raised.
fn xal_name(key: &str) -> String {
    let known = match key {
        "country" => "CountryName",
        "administrativeArea" => "AdministrativeAreaName",
        "subAdministrativeArea" => "SubAdministrativeAreaName",
        "locality" => "LocalityName",
        "dependentLocality" => "DependentLocalityName",
        "thoroughfareName" => "ThoroughfareName",
        "thoroughfareNumber" => "ThoroughfareNumber",
        "postcode" => "PostalCodeNumber",
        "postBox" => "PostBoxNumber",
        "buildingName" => "BuildingName",
        "premiseName" => "PremiseName",
        "premiseNumber" => "PremiseNumber",
        "freeText" => "AddressLine",
        _ => {
            let mut chars = key.chars();
            return chars
                .next()
                .map_or_else(String::new, |c| c.to_uppercase().chain(chars).collect());
        }
    };
    known.to_string()
}

/// An object's materials and textures as geometries name them: one entry
/// per theme and palette index, in the order first met, with what it
/// covers.
#[derive(Default)]
struct AppearanceBuilder {
    materials: Vec<(String, usize, Vec<String>)>,
    material_at: HashMap<(String, usize), usize>,
    textures: Vec<(String, usize, Vec<RingUvs>)>,
    texture_at: HashMap<(String, usize), usize>,
}

impl AppearanceBuilder {
    fn material(&mut self, theme: &str, index: usize, target: String) {
        let key = (theme.to_string(), index);
        let at = *self.material_at.entry(key).or_insert_with(|| {
            self.materials.push((theme.to_string(), index, Vec::new()));
            self.materials.len() - 1
        });
        self.materials[at].2.push(target);
    }

    fn texture(&mut self, theme: &str, index: usize, ring: RingUvs) {
        let key = (theme.to_string(), index);
        let at = *self.texture_at.entry(key).or_insert_with(|| {
            self.textures.push((theme.to_string(), index, Vec::new()));
            self.textures.len() - 1
        });
        self.textures[at].2.push(ring);
    }

    fn finish(self, palette: Option<&CjAppearance>) -> Result<Appearance> {
        let materials = self
            .materials
            .into_iter()
            .map(|(theme, index, targets)| {
                let m = palette
                    .and_then(|p| p.materials.as_ref())
                    .and_then(|m| m.get(index))
                    .ok_or_else(|| Error::invalid(format!("material {index} out of range")))?;
                Ok(material(m, theme, targets))
            })
            .collect::<Result<_>>()?;
        let textures = self
            .textures
            .into_iter()
            .map(|(theme, index, rings)| {
                let t = palette
                    .and_then(|p| p.textures.as_ref())
                    .and_then(|t| t.get(index))
                    .ok_or_else(|| Error::invalid(format!("texture {index} out of range")))?;
                Ok(texture(t, theme, rings))
            })
            .collect::<Result<_>>()?;
        Ok(Appearance {
            materials,
            textures,
        })
    }
}

fn material(m: &MaterialObject, theme: String, targets: Vec<String>) -> Material {
    Material {
        theme,
        targets,
        id: None,
        name: Some(m.name.clone()),
        ambient_intensity: m.ambient_intensity,
        diffuse_color: m.diffuse_color.map(Vec::from),
        emissive_color: m.emissive_color.map(Vec::from),
        specular_color: m.specular_color.map(Vec::from),
        shininess: m.shininess,
        transparency: m.transparency,
        is_smooth: m.is_smooth,
    }
}

fn texture(t: &TextureObject, theme: String, rings: Vec<RingUvs>) -> Texture {
    let word = |v: Json| v.as_str().map(str::to_string);
    Texture {
        theme,
        rings,
        image_uri: t.image.clone(),
        mime_type: t.thetype.and_then(|f| {
            Some(
                match f {
                    TextureFormat::PNG => "image/png",
                    TextureFormat::JPG => "image/jpeg",
                    _ => return None,
                }
                .to_string(),
            )
        }),
        texture_type: t
            .texture_type
            .and_then(|v| serde_json::to_value(v).ok())
            .and_then(word),
        wrap_mode: t
            .wrap_mode
            .and_then(|v| serde_json::to_value(v).ok())
            .and_then(word),
        border_color: t.border_color.as_ref().map(|c| match c {
            BorderColor::Rgb(c) => c.to_vec(),
            BorderColor::Rgba(c) => c.to_vec(),
        }),
    }
}
