//! One batch's rows rebuilt as the objects the CityGML 3.0 reader reads
//! from the same model in XML.
//!
//! A feature's properties are a tree of rows. What the reader does with
//! each top-level property depends on its datatype, namespace and name,
//! as the XML walker's does on the element's:
//!
//! - a `GeometryProperty` is geometry at the LoD the row states:
//!   `lodNSolid`, `lodNMultiSurface`, `lodNMultiCurve`, `lod0Point`, and the
//!   `depr` namespace's CityGML 2.0 slots (`lod4Solid`, `lod0RoofEdge`, …),
//!   each placed by what the geometry is; a terrain intersection curve is
//!   left out, as the XML readers leave it out;
//! - an `ImplicitGeometryProperty` places one of the database's templates;
//! - `boundary` (and a transportation space's `marking`) contains boundary
//!   surface features, each a [`BoundarySurface`] whose `fillingSurface`s
//!   (a wall's doors and windows) are surfaces of their own; a
//!   `gen:GenericThematicSurface`'s geometry is the object's, untyped;
//! - a transportation object's `trafficSpace`, `auxiliaryTrafficSpace`,
//!   `hole`, `section` and `intersection` contain spaces whose boundary
//!   surfaces are the object's own, as CityJSON models them;
//! - any other property containing a feature of a class [`NESTED`] names
//!   (`buildingPart`, `buildingInstallation`, …) nests it as a part or
//!   child object; one containing an ADE's feature, as a child;
//! - a group's `groupMember` role holds its member, contained (a child) or
//!   referenced (by its `objectid`);
//! - `con:height` gives `measuredHeight` when its status is `measured`;
//! - an address is the object's address;
//! - a property in an ADE's namespace is kept as an [`XmlNode`] tree;
//! - any other property with a value is an attribute: `core:name` and
//!   `core:description` as GML's, a `gen:` property as a generic
//!   attribute, a `GenericAttributeSet` as a set.
//!
//! What has no place in the model is counted by name, never dropped
//! silently.
//!
//! [`BoundarySurface`]: citygml_core::model::BoundarySurface

use std::collections::{BTreeMap, HashMap, HashSet};

use citygml_core::gml::ObjectBuilder;
use citygml_core::gml::read::Collected;
use citygml_core::model::{
    Attribute, AttributeName, Instance, Module, Namespace, Polygon, QName, Value, XmlContent,
    XmlNode,
};
use citygml_core::vocabulary::cityjson_type;
use citygml_core::{Error, Result};
use serde_json::Value as Json;

use crate::appearance::appearance;
use crate::catalogue::{Catalogue, Class, Ns};
use crate::geometry::{self, Built, Surfaces};
use crate::rows::{Batch, FeatureRow, PropertyRow};

/// How deep features may nest in one another: a guard against a cycle of
/// containment.
const MAX_DEPTH: usize = 32;

/// How deep boundary surfaces may nest in one another (a wall holds a door,
/// which holds nothing).
const MAX_SURFACE_DEPTH: usize = 8;

/// How a nested feature hangs off the object holding it.
#[derive(Clone, Copy, PartialEq)]
enum Nesting {
    /// A part (`BuildingPart`): an object of its own.
    Part,
    /// Any other sub-feature: also an object of its own.
    Child,
}

/// The CityGML 3.0 sub-features read as objects of their own, as the
/// CityGML 3.0 reader reads them.
const NESTED: &[(&str, Module, Nesting)] = &[
    ("BuildingPart", Module::Building, Nesting::Part),
    ("BuildingInstallation", Module::Building, Nesting::Child),
    ("BuildingRoom", Module::Building, Nesting::Child),
    ("BuildingFurniture", Module::Building, Nesting::Child),
    (
        "BuildingConstructiveElement",
        Module::Building,
        Nesting::Child,
    ),
    ("Storey", Module::Building, Nesting::Child),
    ("BuildingUnit", Module::Building, Nesting::Child),
    ("BridgePart", Module::Bridge, Nesting::Part),
    ("BridgeInstallation", Module::Bridge, Nesting::Child),
    ("BridgeRoom", Module::Bridge, Nesting::Child),
    ("BridgeFurniture", Module::Bridge, Nesting::Child),
    ("BridgeConstructiveElement", Module::Bridge, Nesting::Child),
    ("TunnelPart", Module::Tunnel, Nesting::Part),
    ("TunnelInstallation", Module::Tunnel, Nesting::Child),
    ("HollowSpace", Module::Tunnel, Nesting::Child),
    ("TunnelFurniture", Module::Tunnel, Nesting::Child),
    ("TunnelConstructiveElement", Module::Tunnel, Nesting::Child),
];

/// A transportation object's properties holding spaces whose boundary
/// surfaces are its own.
const SPACES: &[&str] = &[
    "trafficSpace",
    "auxiliaryTrafficSpace",
    "hole",
    "section",
    "intersection",
];

/// What the reader has passed over, by name.
#[derive(Debug, Default)]
pub(crate) struct Tally {
    /// Features of a class the reader does not read (`dem:RasterRelief`).
    pub(crate) skipped_members: BTreeMap<String, usize>,
    /// Features of an ADE's class, read.
    pub(crate) ade_members: BTreeMap<String, usize>,
    /// Properties, appearances and geometries with no place in the model
    /// (`core:lod2TerrainIntersectionCurve`, `app:GeoreferencedTexture`).
    pub(crate) dropped: BTreeMap<String, usize>,
}

impl Tally {
    fn drop(&mut self, what: String) {
        *self.dropped.entry(what).or_insert(0) += 1;
    }
}

/// Rebuilds a batch's features.
pub(crate) struct Assembler<'a> {
    pub(crate) batch: &'a Batch,
    pub(crate) catalogue: &'a Catalogue,
    /// `implicit_geometry.id` -> the template's position in the document's
    /// templates.
    pub(crate) templates: &'a HashMap<i64, usize>,
    pub(crate) tally: &'a mut Tally,
    /// The features already read below the current top-level feature. A
    /// feature two properties contain (a room inside its building, and
    /// also listed by its building unit) is read where it is first met,
    /// as the XML readers read it where it is defined, not where an
    /// `xlink:href` refers to it.
    pub(crate) seen: HashSet<i64>,
}

/// What an object's properties leave to place once all have been read.
#[derive(Default)]
struct Pending {
    /// The `geometry_data` rows the object and its surfaces use, whose
    /// mapped appearance is the object's.
    geometries: Vec<i64>,
    tin_lod: Option<String>,
    tin: Option<Collected>,
    roof_edge: Option<(String, Surfaces)>,
    measured_height: Option<f64>,
}

impl<'a> Assembler<'a> {
    /// The objects a top-level feature `id` yields: itself, or a relief
    /// feature's components. A feature of a class the reader does not
    /// read yields nothing and is counted.
    pub(crate) fn top_level(&mut self, id: i64) -> Result<Vec<ObjectBuilder>> {
        self.seen.clear();
        self.seen.insert(id);
        let Some(row) = self.batch.features.get(&id) else {
            return Ok(Vec::new());
        };
        let Some(class) = self.catalogue.class(row.class).cloned() else {
            self.tally
                .skipped_members
                .entry(format!("objectclass {}", row.class))
                .and_modify(|n| *n += 1)
                .or_insert(1);
            return Ok(Vec::new());
        };
        if class.name == QName::module(Module::Relief, "ReliefFeature") {
            // Passed through: its components are what is read.
            let mut out = Vec::new();
            for p in self.properties(id) {
                if p.name != "reliefComponent" || p.relation != Some(1) {
                    continue;
                }
                let Some(component) = p.feature else { continue };
                if self.seen.insert(component) {
                    out.extend(self.emitted(component)?);
                }
            }
            return Ok(out);
        }
        Ok(self.emitted(id)?.into_iter().collect())
    }

    /// Feature `id` as an object the reader yields, with its extent, or
    /// nothing, counted, for a class with no CityJSON type.
    fn emitted(&mut self, id: i64) -> Result<Option<ObjectBuilder>> {
        let Some(row) = self.batch.features.get(&id) else {
            return Ok(None);
        };
        let Some(class) = self.catalogue.class(row.class).cloned() else {
            return Ok(None);
        };
        if !self.reads(&class) {
            *self
                .tally
                .skipped_members
                .entry(class.prefixed.clone())
                .or_insert(0) += 1;
            return Ok(None);
        }
        let mut b = self.object(id, &class, 0)?;
        if let Some(envelope) = row.envelope {
            b.set_extent(envelope);
        }
        Ok(Some(b))
    }

    /// Whether the reader reads features of `class` as objects: a class
    /// CityJSON has a type for, or an ADE's.
    fn reads(&self, class: &Class) -> bool {
        cityjson_type(&class.name).is_some() || matches!(class.name.ns, Namespace::Other(_))
    }

    fn properties(&self, feature: i64) -> &'a [PropertyRow] {
        self.batch
            .properties
            .get(&feature)
            .map_or(&[][..], Vec::as_slice)
    }

    fn children(&self, property: i64) -> &'a [PropertyRow] {
        self.batch
            .children
            .get(&property)
            .map_or(&[][..], Vec::as_slice)
    }

    /// The name a count gives a property: `bldg:function`.
    fn prefixed(&self, p: &PropertyRow) -> String {
        match self.catalogue.namespace(p.namespace) {
            Some(Ns::Module(module)) => {
                format!(
                    "{}:{}",
                    citygml_core::vocabulary::module_prefix(*module),
                    p.name
                )
            }
            Some(Ns::Deprecated) => format!("depr:{}", p.name),
            _ => p.name.clone(),
        }
    }

    /// Feature `id` of class `class`, `depth` levels below a top-level
    /// object.
    fn object(&mut self, id: i64, class: &Class, depth: usize) -> Result<ObjectBuilder> {
        if depth > MAX_DEPTH {
            return Err(Error::invalid(format!(
                "features nested deeper than {MAX_DEPTH} below a top-level feature"
            )));
        }
        if matches!(class.name.ns, Namespace::Other(_)) {
            *self
                .tally
                .ade_members
                .entry(class.prefixed.clone())
                .or_insert(0) += 1;
        }
        let batch = self.batch;
        let row = &batch.features[&id];
        let mut b = ObjectBuilder::new(row.objectid.clone(), class.name.clone());
        for attribute in stamps(row) {
            b.push_attribute(attribute);
        }
        let mut pending = Pending::default();
        for p in self.properties(id) {
            self.property(&mut b, &mut pending, p, depth)?;
        }
        if let Some(surfaces) = pending.tin {
            // A TIN without its LoD is at LoD 1, as the XML readers place it.
            b.add_tin(pending.tin_lod.unwrap_or_else(|| "1".into()), surfaces);
        }
        if let Some((lod, edge)) = pending.roof_edge
            && !b.has_plain_surfaces(&lod)
        {
            // A roof edge is a building's LoD 0 geometry only when it has no
            // footprint, as the CityGML 2.0 reader reads it.
            add_surfaces(&mut b, lod, edge);
        }
        if let Some(height) = pending.measured_height {
            b.push_attribute(Attribute {
                name: AttributeName::Property(QName::module(
                    Module::Construction,
                    "measuredHeight",
                )),
                value: Value::Double(height),
            });
        }
        let mappings = pending
            .geometries
            .iter()
            .filter_map(|g| self.batch.mappings.get(g).map(Vec::as_slice));
        b.add_appearance(appearance(
            mappings,
            &self.batch.surface_data,
            &mut self.tally.dropped,
        ));
        Ok(b)
    }

    fn property(
        &mut self,
        b: &mut ObjectBuilder,
        pending: &mut Pending,
        p: &PropertyRow,
        depth: usize,
    ) -> Result<()> {
        let ns = self.catalogue.namespace(p.namespace).cloned();
        if let Some(Ns::Ade(uri)) = &ns {
            let node = self.xml_node(p, uri);
            b.push_ade(node);
            return Ok(());
        }
        let module = match &ns {
            Some(Ns::Module(m)) => Some(*m),
            _ => None,
        };
        match self.catalogue.datatype(p.datatype) {
            "GeometryProperty" => self.geometry_property(b, pending, p, module)?,
            "ImplicitGeometryProperty" => self.instance(b, p),
            // Appearances are read from what they map onto the object's
            // geometry.
            "AppearanceProperty" => {}
            "AddressProperty" => match p.address.and_then(|a| self.batch.addresses.get(&a)) {
                Some(address) => b.push_address(address.clone()),
                None => self.tally.drop(self.prefixed(p)),
            },
            "FeatureProperty" => self.feature_property(b, pending, p, module, depth)?,
            "Role" => self.group_member(b, p, depth)?,
            "Height" => self.height(pending, p),
            _ if module == Some(Module::Relief) && p.name == "lod" => {
                pending.tin_lod = scalar(p).map(|v| v.trim().to_string());
            }
            _ => match self.attribute(p, &ns) {
                Some(attribute) => b.push_attribute(attribute),
                None => self.tally.drop(self.prefixed(p)),
            },
        }
        Ok(())
    }

    fn geometry_property(
        &mut self,
        b: &mut ObjectBuilder,
        pending: &mut Pending,
        p: &PropertyRow,
        module: Option<Module>,
    ) -> Result<()> {
        if p.name.contains("TerrainIntersectionCurve") {
            self.tally.drop(self.prefixed(p));
            return Ok(());
        }
        let Some((id, built)) = self.geometry(p)? else {
            self.tally.drop(self.prefixed(p));
            return Ok(());
        };
        pending.geometries.push(id);
        if module == Some(Module::Relief) && p.name == "tin" {
            match built {
                Built::Surfaces(surfaces) => pending.tin = Some(surfaces.into_faces()),
                _ => self.tally.drop(self.prefixed(p)),
            }
            return Ok(());
        }
        let Some(lod) = lod(p) else {
            self.tally.drop(self.prefixed(p));
            return Ok(());
        };
        match built {
            Built::Surfaces(surfaces) if p.name.ends_with("RoofEdge") => {
                if pending.roof_edge.is_none() {
                    pending.roof_edge = Some((lod, surfaces));
                }
            }
            Built::Solid(solid) => b.add_solid(lod, solid),
            Built::Surfaces(surfaces) => add_surfaces(b, lod, surfaces),
            Built::Lines(lines) => b.add_lines(lod, lines),
            Built::Points(points) => b.add_points(lod, points),
            Built::Unsupported(kind) => self.tally.drop(format!("gml:{kind}")),
        }
        Ok(())
    }

    /// The geometry a property points at, rebuilt, with its row id.
    fn geometry(&self, p: &PropertyRow) -> Result<Option<(i64, Built)>> {
        let Some(id) = p.geometry else {
            return Ok(None);
        };
        let Some(row) = self.batch.geometries.get(&id) else {
            return Ok(None);
        };
        let built = geometry::build(&row.wkb, &row.properties, &self.batch.textured(id))
            .map_err(|e| Error::invalid(format!("geometry {id}: {e}")))?;
        Ok(Some((id, built)))
    }

    fn instance(&mut self, b: &mut ObjectBuilder, p: &PropertyRow) {
        let template = p.implicit.and_then(|t| self.templates.get(&t)).copied();
        let matrix = p.array.as_ref().and_then(matrix).unwrap_or(IDENTITY);
        match (template, p.reference_point, lod(p)) {
            (Some(template), Some(reference_point), Some(lod)) => b.add_instance(
                lod,
                Instance {
                    template,
                    matrix,
                    reference_point,
                },
            ),
            _ => self.tally.drop(self.prefixed(p)),
        }
    }

    fn feature_property(
        &mut self,
        b: &mut ObjectBuilder,
        pending: &mut Pending,
        p: &PropertyRow,
        module: Option<Module>,
        depth: usize,
    ) -> Result<()> {
        let Some(target) = p.feature else {
            return Ok(());
        };
        if p.relation != Some(1) {
            // A reference to a feature held elsewhere (`core:relatedTo`).
            self.tally.drop(self.prefixed(p));
            return Ok(());
        }
        if !self.seen.insert(target) {
            return Ok(());
        }
        let Some(row) = self.batch.features.get(&target) else {
            // A terminated feature.
            return Ok(());
        };
        let Some(class) = self.catalogue.class(row.class).cloned() else {
            return Ok(());
        };
        let name = p.name.as_str();
        match module {
            Some(m) if is_boundary_property(m, name) => {
                self.boundary(b, pending, target, None, 0)?;
            }
            Some(Module::Transportation) if SPACES.contains(&name) => {
                self.spaces(b, pending, target, 0)?;
            }
            _ => match nesting(&class) {
                Some(nesting) => {
                    let child = self.object(target, &class, depth + 1)?;
                    match nesting {
                        Nesting::Part => b.add_part(child),
                        Nesting::Child => b.add_child(child),
                    }
                }
                None => {
                    *self
                        .tally
                        .skipped_members
                        .entry(class.prefixed.clone())
                        .or_insert(0) += 1;
                }
            },
        }
        Ok(())
    }

    /// Boundary surface feature `id`, nested in surface `parent` if any.
    fn boundary(
        &mut self,
        b: &mut ObjectBuilder,
        pending: &mut Pending,
        id: i64,
        parent: Option<usize>,
        depth: usize,
    ) -> Result<()> {
        if depth >= MAX_SURFACE_DEPTH {
            return Err(Error::invalid(format!(
                "boundary surfaces nest more than {MAX_SURFACE_DEPTH} levels deep"
            )));
        }
        let Some(row) = self.batch.features.get(&id) else {
            return Ok(());
        };
        let Some(class) = self.catalogue.class(row.class).cloned() else {
            return Ok(());
        };
        let generic = class.name == QName::module(Module::Generics, "GenericThematicSurface");
        let surface = (!generic).then(|| b.push_boundary_surface(class.name.clone(), parent));
        if let Some(s) = surface {
            for attribute in stamps(row) {
                b.push_surface_attribute(s, attribute);
            }
        }
        for p in self.properties(id) {
            let ns = self.catalogue.namespace(p.namespace).cloned();
            match self.catalogue.datatype(p.datatype) {
                "GeometryProperty" => {
                    let lod = lod(p);
                    match (self.geometry(p)?, lod) {
                        (Some((g, Built::Surfaces(surfaces))), Some(lod)) => {
                            pending.geometries.push(g);
                            match surface {
                                Some(s) => {
                                    let (bases, collected) = surfaces.by_reference();
                                    register(b, bases);
                                    b.add_boundary_geometry(s, &lod, collected);
                                }
                                None => add_surfaces(b, lod, surfaces),
                            }
                        }
                        _ => self.tally.drop(self.prefixed(p)),
                    }
                }
                "FeatureProperty" if p.name == "fillingSurface" && p.relation == Some(1) => {
                    if let Some(filling) = p.feature
                        && self.seen.insert(filling)
                    {
                        self.boundary(b, pending, filling, surface, depth + 1)?;
                    }
                }
                "AppearanceProperty" => {}
                _ => match (surface, self.attribute(p, &ns)) {
                    (Some(s), Some(attribute)) => b.push_surface_attribute(s, attribute),
                    _ => self.tally.drop(self.prefixed(p)),
                },
            }
        }
        Ok(())
    }

    /// Transportation space `id`: its boundary surfaces and markings, and
    /// those of the spaces it holds, as the enclosing object's own.
    /// Anything else a space says is counted and left out.
    fn spaces(
        &mut self,
        b: &mut ObjectBuilder,
        pending: &mut Pending,
        id: i64,
        depth: usize,
    ) -> Result<()> {
        if depth > MAX_DEPTH {
            return Err(Error::invalid(format!(
                "transportation spaces nested deeper than {MAX_DEPTH}"
            )));
        }
        for p in self.properties(id) {
            let module = match self.catalogue.namespace(p.namespace) {
                Some(Ns::Module(m)) => Some(*m),
                _ => None,
            };
            let contained = p.relation == Some(1) && p.feature.is_some();
            match module {
                Some(m) if contained && is_boundary_property(m, &p.name) => {
                    let target = p.feature.unwrap_or_default();
                    if self.seen.insert(target) {
                        self.boundary(b, pending, target, None, 0)?;
                    }
                }
                Some(Module::Transportation) if contained && SPACES.contains(&p.name.as_str()) => {
                    let target = p.feature.unwrap_or_default();
                    if self.seen.insert(target) {
                        self.spaces(b, pending, target, depth + 1)?;
                    }
                }
                _ => self.tally.drop(format!("space {}", self.prefixed(p))),
            }
        }
        Ok(())
    }

    /// A group's `groupMember` role: its member, contained or referenced,
    /// with the role it plays.
    fn group_member(&mut self, b: &mut ObjectBuilder, p: &PropertyRow, depth: usize) -> Result<()> {
        let children = self.children(p.id);
        let role = children
            .iter()
            .find(|c| c.name == "role")
            .and_then(|c| c.string.clone());
        let Some(member) = children.iter().find(|c| c.name == "groupMember") else {
            return Ok(());
        };
        let Some(target) = member.feature else {
            return Ok(());
        };
        if member.relation == Some(1) {
            if !self.seen.insert(target) {
                return Ok(());
            }
            let Some(row) = self.batch.features.get(&target) else {
                return Ok(());
            };
            let Some(class) = self.catalogue.class(row.class).cloned() else {
                return Ok(());
            };
            if self.reads(&class) || nesting(&class).is_some() {
                let object = self.object(target, &class, depth + 1)?;
                b.add_member(object, role);
            } else {
                *self
                    .tally
                    .skipped_members
                    .entry(class.prefixed.clone())
                    .or_insert(0) += 1;
            }
        } else if let Some(Some(id)) = self.batch.referenced.get(&target) {
            b.add_member_ref(id.clone(), role);
        }
        Ok(())
    }

    /// A construction's `con:height`: its value when its status is
    /// `measured`, the height CityGML 2.0 calls `measuredHeight`.
    fn height(&mut self, pending: &mut Pending, p: &PropertyRow) {
        let children = self.children(p.id);
        let status = children
            .iter()
            .find(|c| c.name == "status")
            .and_then(|c| c.string.as_deref());
        let value = children
            .iter()
            .find(|c| c.name == "value")
            .and_then(|c| c.double);
        match (status.map(str::trim), value) {
            (Some("measured"), Some(value)) if pending.measured_height.is_none() => {
                pending.measured_height = Some(value);
            }
            _ => self.tally.drop("con:height".into()),
        }
    }

    /// A property with a value, or a generic attribute set, as an
    /// attribute.
    fn attribute(&self, p: &PropertyRow, ns: &Option<Ns>) -> Option<Attribute> {
        let module = match ns {
            Some(Ns::Module(m)) => *m,
            _ => return None,
        };
        let value = if self.catalogue.datatype(p.datatype) == "GenericAttributeSet" {
            let members: Vec<Attribute> = self
                .children(p.id)
                .iter()
                .filter_map(|c| {
                    let ns = self.catalogue.namespace(c.namespace).cloned();
                    self.attribute(c, &ns)
                })
                .collect();
            Value::Set(members)
        } else {
            self.value(p)?
        };
        let name = match (module, p.name.as_str()) {
            (Module::Core, "name" | "description") => {
                AttributeName::Property(QName::new(Namespace::Gml, p.name.clone()))
            }
            (Module::Generics, _) => AttributeName::Generic(p.name.clone()),
            _ => AttributeName::Property(QName::module(module, p.name.clone())),
        };
        Some(Attribute { name, value })
    }

    /// A property's value, typed as its datatype types it. A complex
    /// datatype (one with child properties) has none.
    fn value(&self, p: &PropertyRow) -> Option<Value> {
        let value = match self.catalogue.datatype(p.datatype) {
            "Integer" => Value::Integer(p.int?),
            "Double" | "Measure" => Value::Double(p.double?),
            "Boolean" => Value::Boolean(p.int? != 0),
            "String" | "StringOrRef" | "Code" | "URI" | "Timestamp" | "TimePosition" => {
                Value::String(scalar(p)?)
            }
            _ => match &p.array {
                Some(Json::Array(items)) => {
                    Value::List(items.iter().filter_map(json_value).collect())
                }
                _ if self.children(p.id).is_empty() => Value::String(scalar(p)?),
                _ => return None,
            },
        };
        Some(value)
    }

    /// An ADE's property, and the properties below it, as XML.
    fn xml_node(&self, p: &PropertyRow, uri: &str) -> XmlNode {
        let children = self.children(p.id);
        let content = if !children.is_empty() {
            XmlContent::Children(
                children
                    .iter()
                    .map(|c| {
                        let uri = match self.catalogue.namespace(c.namespace) {
                            Some(Ns::Ade(u)) => u.clone(),
                            _ => uri.to_string(),
                        };
                        self.xml_node(c, &uri)
                    })
                    .collect(),
            )
        } else {
            match scalar(p)
                .or_else(|| p.int.map(|i| i.to_string()))
                .or_else(|| p.double.map(|d| d.to_string()))
            {
                Some(text) => XmlContent::Text(text),
                None => XmlContent::Empty,
            }
        };
        XmlNode {
            name: QName::new(Namespace::Other(uri.to_string()), p.name.clone()),
            attributes: Vec::new(),
            content,
        }
    }
}

/// Whether a property of module `module` holds boundary surfaces: the core
/// module's `boundary`, and a transportation space's `marking`.
fn is_boundary_property(module: Module, name: &str) -> bool {
    matches!(
        (module, name),
        (Module::Core, "boundary") | (Module::Transportation, "marking")
    )
}

/// How a feature of `class` nests in the feature containing it, or `None`
/// for a class the reader does not nest.
fn nesting(class: &Class) -> Option<Nesting> {
    if matches!(class.name.ns, Namespace::Other(_)) {
        return Some(Nesting::Child);
    }
    let Namespace::Module(module) = class.name.ns else {
        return None;
    };
    NESTED
        .iter()
        .find(|(name, m, _)| *m == module && *name == class.name.local)
        .map(|(_, _, nesting)| *nesting)
}

/// The LoD a geometry property states, else the digit its name spells
/// (`lod2MultiSurface`).
fn lod(p: &PropertyRow) -> Option<String> {
    p.lod.clone().or_else(|| {
        p.name
            .strip_prefix("lod")
            .and_then(|rest| rest.chars().next())
            .filter(char::is_ascii_digit)
            .map(String::from)
    })
}

/// A property's value as text: a string, a URI or a timestamp. Text that
/// is only whitespace is none, as the XML readers read an empty element.
fn scalar(p: &PropertyRow) -> Option<String> {
    p.string
        .clone()
        .or_else(|| p.uri.clone())
        .or_else(|| p.timestamp.clone())
        .or_else(|| p.int.map(|i| i.to_string()))
        .filter(|text| !text.trim().is_empty())
}

/// Register the base polygons `bases` of reversed surfaces as xlink
/// targets.
/// A feature's creation date and validity, which the database keeps in
/// columns of its own, as the attributes the XML readers read them as.
fn stamps(row: &FeatureRow) -> impl Iterator<Item = Attribute> + '_ {
    [
        ("creationDate", &row.creation_date),
        ("validFrom", &row.valid_from),
        ("validTo", &row.valid_to),
    ]
    .into_iter()
    .filter_map(|(name, value)| {
        Some(Attribute {
            name: AttributeName::Property(QName::module(Module::Core, name)),
            value: Value::String(value.clone()?),
        })
    })
}

fn register(b: &mut ObjectBuilder, bases: Vec<(Option<String>, Polygon)>) {
    if !bases.is_empty() {
        b.register(Collected {
            polygons: bases,
            ..Collected::default()
        });
    }
}

/// Add a standalone surface collection at `lod`.
fn add_surfaces(b: &mut ObjectBuilder, lod: String, surfaces: Surfaces) {
    b.add_surfaces(lod, surfaces.into_faces());
}

fn json_value(json: &Json) -> Option<Value> {
    Some(match json {
        Json::Bool(b) => Value::Boolean(*b),
        Json::Number(n) => match n.as_i64() {
            Some(i) => Value::Integer(i),
            None => Value::Double(n.as_f64()?),
        },
        Json::String(s) => Value::String(s.clone()),
        _ => return None,
    })
}

const IDENTITY: [f64; 16] = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];

/// A transformation matrix 3DCityDB stores as a JSON array of sixteen
/// numbers, row by row.
fn matrix(json: &Json) -> Option<[f64; 16]> {
    let values: Vec<f64> = json.as_array()?.iter().filter_map(Json::as_f64).collect();
    values.try_into().ok()
}
