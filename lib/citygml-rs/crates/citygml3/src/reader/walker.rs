//! Read one city object's subtree into an [`ObjectBuilder`].
//!
//! One walker reads every class: [`read_object`]. What it does with each
//! child element depends on the element's namespace and name, never on the
//! class being read, since CityGML 3.0 names its properties uniformly across
//! modules:
//!
//! - the core module's `lodNSolid`, `lodNMultiSurface`, `lodNMultiCurve` and
//!   `lod0Point` are geometry;
//! - `lodNImplicitRepresentation` places one of the document's relative
//!   geometries ([`TemplateIndex`]);
//! - `boundary` holds boundary surfaces, each a thematic surface whose
//!   `fillingSurface`s (a wall's doors and windows) are surfaces of their
//!   own;
//! - a transportation object's `trafficSpace`, `auxiliaryTrafficSpace`,
//!   `hole`, `section` and `intersection` hold spaces whose boundary
//!   surfaces, and `marking`s, are the object's own: CityJSON models them as
//!   semantic surfaces of a road, as CityGML 2.0 does;
//! - `buildingPart`, `buildingRoom`, `buildingSubdivision` and the like nest
//!   sub-features, read as objects of their own when [`NESTED`] names their
//!   class;
//! - `address` is a postal address in xAL 3, each part named as xAL 2 names
//!   it;
//! - a construction's `height` gives its `measuredHeight`;
//! - an `adeOf…` hook holds an ADE's properties of the object, each kept
//!   verbatim as an [`XmlNode`];
//! - a group's `groupMember` holds a role, whose member is an object of its
//!   own when nested, a `gml:id` when referenced by `xlink:href`;
//! - any other text-only property of a CityGML module is an attribute, typed
//!   as [`attributes::property_type`] says;
//! - a property in an ADE's namespace ([`is_ade`]) is kept verbatim, unless
//!   it is an ADE object's own geometry or boundary surfaces, which are read
//!   as a CityGML object's are;
//! - `gml:name`, `gml:description`, `genericAttribute` and `appearance` are
//!   read wherever they appear.
//!
//! An element of an ADE's namespace where a city object belongs (in a
//! `cityObjectMember`, a nesting property or a group's member) is an object
//! of that ADE's class, read by the same walker.

use std::io::BufRead;

use citygml_core::gml::ObjectBuilder;
use citygml_core::gml::read::{self as gml, Collected};
use citygml_core::model::{Address, Attribute, AttributeName, Module, Namespace, QName};
use citygml_core::reader::appearance::read_appearance;
use citygml_core::reader::geometry::{
    ImplicitNames, implicit_lod, lod_suffix, read_implicit, read_lod_solid,
};
use citygml_core::reader::node::{read_xml_node, xml_attributes};
use citygml_core::reader::templates::TemplateIndex;
use citygml_core::reader::text::{AttrType, read_leaf_text, to_value};
use citygml_core::xml::{
    NS_GML, get_attr_local, gml_id, is_ade, member_id, ns_eq, ns_is, ns_of, skip_element,
    unescaped_text, xml_err,
};
use citygml_core::{Error, Result};
use quick_xml::events::Event;
use quick_xml::reader::NsReader;

use super::attributes;
use crate::ns;

/// How CityGML 3.0 spells an `ImplicitGeometry`.
pub(crate) const IMPLICIT: ImplicitNames<'static> = ImplicitNames {
    namespace: ns::CORE,
    relative_geometry: b"relativeGeometry",
};

/// Maximum nesting depth of parts and nested features: a guard against
/// attacker-controlled XML recursion.
const MAX_PART_DEPTH: usize = 32;

/// What every level of an object's subtree is read with.
#[derive(Clone, Copy)]
pub struct Cx<'a> {
    /// How many levels below a top-level object.
    pub depth: usize,
    /// The document's relative geometries, which an `ImplicitGeometry`
    /// instantiates.
    pub templates: &'a TemplateIndex,
}

impl Cx<'_> {
    fn deeper(self) -> Self {
        Self {
            depth: self.depth + 1,
            ..self
        }
    }
}

/// The CityGML 3.0 top-level classes this reader reads.
///
/// A `ReliefFeature` is not an object of its own: CityJSON has no type for
/// it, so the reader passes through it to its components (see
/// [`is_container`]). Of those, only a `TINRelief` maps to CityJSON.
const TOP_LEVEL: &[(&str, Module)] = &[
    ("Building", Module::Building),
    ("Bridge", Module::Bridge),
    ("Tunnel", Module::Tunnel),
    ("OtherConstruction", Module::Construction),
    ("WaterBody", Module::WaterBody),
    ("LandUse", Module::LandUse),
    ("CityFurniture", Module::CityFurniture),
    ("SolitaryVegetationObject", Module::Vegetation),
    ("PlantCover", Module::Vegetation),
    ("GenericOccupiedSpace", Module::Generics),
    ("GenericUnoccupiedSpace", Module::Generics),
    ("GenericLogicalSpace", Module::Generics),
    ("CityObjectGroup", Module::CityObjectGroup),
    ("Road", Module::Transportation),
    ("Railway", Module::Transportation),
    ("Track", Module::Transportation),
    ("Square", Module::Transportation),
    ("Waterway", Module::Transportation),
    ("TINRelief", Module::Relief),
];

/// The class of a top-level object, by its namespace and local name, or
/// `None` for one this reader does not read.
pub fn top_level_class(uri: &[u8], local: &[u8]) -> Option<QName> {
    lookup(TOP_LEVEL, uri, local)
}

/// The class of an ADE object: an element of an ADE's namespace where a
/// city object belongs.
pub fn ade_class(uri: &[u8], local: &[u8]) -> Option<QName> {
    is_ade(uri).then(|| qname(uri, local))
}

/// Whether an element is a feature the reader passes through rather than
/// reads: a `dem:ReliefFeature`, whose `reliefComponent`s are read as
/// top-level objects.
pub fn is_container(uri: &[u8], local: &[u8]) -> bool {
    ns::module(uri) == Some(Module::Relief) && local == b"ReliefFeature"
}

/// Whether an element is a container's member property, whose child is
/// read, or recorded as skipped, as a `cityObjectMember`'s is.
pub fn is_container_member(uri: &[u8], local: &[u8]) -> bool {
    ns::module(uri) == Some(Module::Relief) && local == b"reliefComponent"
}

/// How a nested feature hangs off the object holding it.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Nesting {
    /// A part (`buildingPart`): its polygons are its own.
    Part,
    /// Any other sub-feature: its polygons are also the parent's xlink
    /// targets, since a parent's solid routinely composes faces defined in
    /// one.
    Child,
}

/// The CityGML 3.0 sub-features read as objects of their own.
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

/// The properties of CityGML 3.0 that nest sub-features: the only places a
/// parent's `lodNSolid` can find the polygons it composes by `xlink:href`.
/// Matched as a closed vocabulary rather than by descending into every
/// unknown element, so an ADE subtree is skipped by bytes rather than
/// parsed for geometry nothing will reference.
const NESTING: &[&str] = &[
    "buildingPart",
    "buildingInstallation",
    "buildingRoom",
    "buildingFurniture",
    "buildingConstructiveElement",
    "buildingSubdivision",
    "buildingUnit",
    "storey",
    "bridgePart",
    "bridgeInstallation",
    "bridgeRoom",
    "bridgeFurniture",
    "bridgeConstructiveElement",
    "tunnelPart",
    "tunnelInstallation",
    "hollowSpace",
    "tunnelFurniture",
    "tunnelConstructiveElement",
];

/// A transportation object's properties holding spaces whose boundary
/// surfaces are its own ([`read_spaces`]).
const SPACES: &[&str] = &[
    "trafficSpace",
    "auxiliaryTrafficSpace",
    "hole",
    "section",
    "intersection",
];

fn lookup(table: &[(&str, Module)], uri: &[u8], local: &[u8]) -> Option<QName> {
    let module = ns::module(uri)?;
    table
        .iter()
        .find(|(name, m)| *m == module && name.as_bytes() == local)
        .map(|(name, module)| QName::module(*module, *name))
}

pub(crate) fn nested_class(uri: &[u8], local: &[u8]) -> Option<(QName, Nesting)> {
    let module = ns::module(uri)?;
    NESTED
        .iter()
        .find(|(name, m, _)| *m == module && name.as_bytes() == local)
        .map(|(name, module, nesting)| (QName::module(*module, *name), *nesting))
}

/// Whether a module property nests sub-features (see [`NESTING`]).
pub(crate) fn is_nesting_property(local: &[u8]) -> bool {
    NESTING.iter().any(|p| p.as_bytes() == local)
}

/// Whether a property of module `module` holds boundary surfaces: the core
/// module's `boundary`, and a transportation space's `marking`.
pub(crate) fn is_boundary_property(module: Module, local: &[u8]) -> bool {
    matches!(
        (module, local),
        (Module::Core, b"boundary") | (Module::Transportation, b"marking")
    )
}

/// Whether a property of module `module` holds transportation spaces whose
/// surfaces are the enclosing object's (see [`SPACES`]).
pub(crate) fn is_space_property(module: Module, local: &[u8]) -> bool {
    module == Module::Transportation && SPACES.iter().any(|p| p.as_bytes() == local)
}

/// Whether a module property is an ADE hook (`bldg:adeOfAbstractBuilding`),
/// which holds an ADE's properties of the object.
pub(crate) fn is_ade_hook(local: &[u8]) -> bool {
    local.starts_with(b"adeOf")
}

/// The name of an element: its local name in the namespace it is bound to.
fn qname(uri: &[u8], local: &[u8]) -> QName {
    QName::new(ns::namespace(uri), String::from_utf8_lossy(local))
}

/// Read an object of class `class` (positioned after its `Start`, ending at
/// `End(end_name)`), `cx.depth` levels below a top-level object.
pub fn read_object<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    class: QName,
    id: Option<String>,
    end_name: &[u8],
    cx: Cx<'_>,
) -> Result<ObjectBuilder> {
    if cx.depth > MAX_PART_DEPTH {
        return Err(Error::invalid(format!(
            "CityGML features nested deeper than {MAX_PART_DEPTH}"
        )));
    }
    // The namespace of an ADE object's own properties.
    let own_ade = match &class.ns {
        Namespace::Other(uri) => Some(uri.as_bytes().to_vec()),
        _ => None,
    };
    let mut b = ObjectBuilder::new(id, class);
    let mut tin = Tin::default();
    let mut measured_height: Option<String> = None;
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        match ev {
            Event::Start(e) => {
                let local = e.local_name();
                let name = local.as_ref().to_vec();
                let uri = ns_of(&rr).unwrap_or_default();
                let module = ns::module(&uri);
                if ns_is(&rr, NS_GML) {
                    if matches!(name.as_slice(), b"name" | b"description") {
                        push_leaf(reader, buf, &mut b, &name, AttrType::Str, || {
                            QName::new(Namespace::Gml, String::from_utf8_lossy(&name))
                        })?;
                    } else {
                        skip_element(reader, buf)?;
                    }
                } else if let Some(module) = module {
                    let property = Property {
                        uri: &uri,
                        module,
                        name: &name,
                    };
                    read_module_property(
                        reader,
                        buf,
                        &mut b,
                        property,
                        &mut tin,
                        &mut measured_height,
                        cx,
                    )?;
                } else if is_ade(&uri) {
                    let own = own_ade.as_deref() == Some(uri.as_slice());
                    let attributes = xml_attributes(&e)?;
                    if own && read_geometry_property(reader, buf, &mut b, &name, cx)? {
                        // The ADE object's own geometry, read.
                    } else {
                        let node = read_xml_node(
                            reader,
                            buf,
                            qname(&uri, &name),
                            attributes,
                            ns::namespace,
                        )?;
                        b.push_ade(node);
                    }
                } else {
                    // A standard's namespace other than CityGML's and
                    // GML's (xAL, XLink, …), which names no property of a
                    // city object.
                    skip_element(reader, buf)?;
                }
            }
            Event::End(e) if e.local_name().as_ref() == end_name => break,
            Event::Eof => {
                return Err(Error::invalid(format!(
                    "unexpected end of document inside <{}>",
                    String::from_utf8_lossy(end_name)
                )));
            }
            _ => {}
        }
    }
    if let Some(surfaces) = tin.surfaces {
        // A TIN without its LoD is at LoD 1, the LoD a relief has when its
        // LoD is not otherwise stated.
        b.add_tin(tin.lod.unwrap_or_else(|| "1".into()), surfaces);
    }
    if let Some(height) = measured_height {
        b.push_attribute(Attribute {
            name: AttributeName::Property(QName::module(Module::Construction, "measuredHeight")),
            value: to_value(&height, AttrType::Float),
        });
    }
    Ok(b)
}

/// A `TINRelief`'s `tin` and its `lod`, placed once the whole subtree has
/// been read, since element order is not guaranteed.
#[derive(Default)]
struct Tin {
    lod: Option<String>,
    surfaces: Option<Collected>,
}

/// A property element of a CityGML module: its namespace URI, its module
/// and its local name.
#[derive(Clone, Copy)]
struct Property<'a> {
    uri: &'a [u8],
    module: Module,
    name: &'a [u8],
}

/// A property in a CityGML module namespace (positioned after its `Start`).
fn read_module_property<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    b: &mut ObjectBuilder,
    Property { uri, module, name }: Property<'_>,
    tin: &mut Tin,
    measured_height: &mut Option<String>,
    cx: Cx<'_>,
) -> Result<()> {
    if module == Module::Core && read_geometry_property(reader, buf, b, name, cx)? {
        return Ok(());
    }
    match (module, name) {
        (Module::Core, b"appearance") => {
            b.add_appearance(read_appearance(reader, buf, b"appearance", ns::APPEARANCE)?);
        }
        (Module::Core, b"genericAttribute") => read_generic_attribute(reader, buf, b)?,
        (Module::Relief, b"lod") => {
            // A relief component's LoD is the LoD of its TIN, not an
            // attribute.
            if let Some(text) = read_leaf_text(reader, buf, name)? {
                tin.lod = Some(text.trim().to_string());
            }
        }
        (Module::Relief, b"tin") => tin.surfaces = Some(gml::collect_surfaces(reader, buf)?),
        (Module::Construction, b"height") => {
            let height = read_height(reader, buf)?;
            if measured_height.is_none() {
                *measured_height = height;
            }
        }
        (Module::CityObjectGroup, b"groupMember") => read_group_member(reader, buf, b, cx)?,
        (_, b"address") => b.push_address(read_address(reader, buf)?),
        _ if is_boundary_property(module, name) => read_boundary(reader, buf, b, name)?,
        _ if is_space_property(module, name) => read_spaces(reader, buf, b, name)?,
        _ if is_nesting_property(name) => read_nested(reader, buf, b, name, uri, cx)?,
        _ if is_ade_hook(name) => read_ade_hook(reader, buf, b, name)?,
        _ => {
            let ty = attributes::property_type(module, name);
            push_leaf(reader, buf, b, name, ty, || {
                QName::module(module, String::from_utf8_lossy(name))
            })?;
        }
    }
    Ok(())
}

/// A geometry property (positioned after its `Start`), named the way
/// CityGML 3.0's core module names one: `lodNSolid`, `lodNMultiSurface`,
/// `lodNMultiCurve`, `lod0Point` and `lodNImplicitRepresentation`. `false`,
/// having read nothing, for any other name.
fn read_geometry_property<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    b: &mut ObjectBuilder,
    name: &[u8],
    cx: Cx<'_>,
) -> Result<bool> {
    if let Some(lod) = lod_suffix(name, b"Solid") {
        let geom = read_lod_solid(reader, buf, name)?;
        b.add_solid(lod, geom);
    } else if let Some(lod) = lod_suffix(name, b"MultiSurface") {
        b.add_surfaces(lod, gml::collect_surfaces(reader, buf)?);
    } else if let Some(lod) = implicit_lod(name) {
        if let Some(instance) = read_implicit(reader, buf, name, cx.templates, IMPLICIT)? {
            b.add_instance(lod, instance);
        }
    } else if let Some(lod) = lod_suffix(name, b"MultiCurve") {
        b.add_lines(lod, gml::collect_lines(reader, buf)?);
    } else if let Some(lod) = lod_suffix(name, b"Point") {
        b.add_points(lod, gml::collect_points(reader, buf)?);
    } else {
        return Ok(false);
    }
    Ok(true)
}

/// A text-only property (positioned after its `Start`) as an attribute
/// named `name()`, typed `ty`. An empty property, or one with child
/// elements, adds nothing.
fn push_leaf<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    b: &mut ObjectBuilder,
    end: &[u8],
    ty: AttrType,
    name: impl FnOnce() -> QName,
) -> Result<()> {
    if let Some(text) = read_leaf_text(reader, buf, end)? {
        b.push_attribute(Attribute {
            name: AttributeName::Property(name()),
            value: to_value(&text, ty),
        });
    }
    Ok(())
}

/// A `genericAttribute` property (positioned after its `Start`): its `gen:`
/// attribute, when it is one with a name and a value. A
/// `GenericAttributeSet` is skipped.
fn read_generic_attribute<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    b: &mut ObjectBuilder,
) -> Result<()> {
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        match ev {
            Event::Start(e) => {
                let ty = ns_eq(&rr, ns::GENERICS.as_bytes())
                    .then(|| attributes::generic_attr(e.local_name().as_ref()))
                    .flatten();
                match ty {
                    Some(ty) => {
                        if let Some((name, value)) = attributes::read_generic(reader, buf, ty)? {
                            b.push_attribute(Attribute {
                                name: AttributeName::Generic(name),
                                value,
                            });
                        }
                    }
                    None => skip_element(reader, buf)?,
                }
            }
            Event::End(e) if e.local_name().as_ref() == b"genericAttribute" => break,
            Event::Eof => {
                return Err(Error::invalid(
                    "unexpected end of document inside <genericAttribute>",
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

/// A construction's `height` property (positioned after its `Start`): the
/// `value` of its `con:Height` when that height's `status` is `measured`,
/// the height CityGML 2.0 calls `measuredHeight`.
fn read_height<R: BufRead>(reader: &mut NsReader<R>, buf: &mut Vec<u8>) -> Result<Option<String>> {
    let mut status: Option<String> = None;
    let mut value: Option<String> = None;
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        match ev {
            Event::Start(e) => {
                let local = e.local_name();
                let construction = ns_eq(&rr, ns::CONSTRUCTION.as_bytes());
                match local.as_ref() {
                    b"Height" if construction => {}
                    b"status" if construction => status = read_leaf_text(reader, buf, b"status")?,
                    b"value" if construction => value = read_leaf_text(reader, buf, b"value")?,
                    _ => skip_element(reader, buf)?,
                }
            }
            Event::End(e) if e.local_name().as_ref() == b"height" => break,
            Event::Eof => {
                return Err(Error::invalid("unexpected end of document inside <height>"));
            }
            _ => {}
        }
    }
    Ok(value.filter(|_| status.as_deref().map(str::trim) == Some("measured")))
}

/// The xAL 3 parts whose text xAL 2 spells with an element of its own: the
/// part's element, the element holding the text inside it, and xAL 2's
/// name for it.
const XAL3: &[(&str, &str, &str)] = &[
    ("Country", "NameElement", "CountryName"),
    (
        "AdministrativeArea",
        "NameElement",
        "AdministrativeAreaName",
    ),
    (
        "SubAdministrativeArea",
        "NameElement",
        "SubAdministrativeAreaName",
    ),
    ("Locality", "NameElement", "LocalityName"),
    ("SubLocality", "NameElement", "DependentLocalityName"),
    ("Thoroughfare", "NameElement", "ThoroughfareName"),
    ("Thoroughfare", "Number", "ThoroughfareNumber"),
    ("Premises", "NameElement", "PremiseName"),
    ("Premises", "Number", "PremiseNumber"),
    ("PostCode", "Identifier", "PostalCodeNumber"),
    ("FreeTextAddress", "AddressLine", "AddressLine"),
];

/// An `address` property (positioned after its `Start`): every leaf of its
/// xAL subtree that holds text, named as xAL 2 names it where [`XAL3`]
/// says, and the points of its `multiPoint`.
fn read_address<R: BufRead>(reader: &mut NsReader<R>, buf: &mut Vec<u8>) -> Result<Address> {
    let mut address = Address::default();
    // Open elements below the property: (local name, text, has a child).
    let mut open: Vec<(String, String, bool)> = Vec::new();
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        match ev {
            Event::Start(e) => {
                if let Some(parent) = open.last_mut() {
                    parent.2 = true;
                }
                let local = e.local_name();
                if ns_is(&rr, ns::CORE) && local.as_ref() == b"multiPoint" {
                    address.location.extend(gml::collect_points(reader, buf)?);
                } else {
                    let name = String::from_utf8_lossy(local.as_ref()).into_owned();
                    open.push((name, String::new(), false));
                }
            }
            Event::Text(t) => {
                if let Some(top) = open.last_mut() {
                    top.1.push_str(&unescaped_text(&t)?);
                }
            }
            Event::End(_) => {
                let Some((name, text, has_child)) = open.pop() else {
                    break;
                };
                let text = text.trim();
                if !has_child && !text.is_empty() {
                    let parent = open.last().map(|(p, _, _)| p.as_str());
                    let name = XAL3
                        .iter()
                        .find(|(part, leaf, _)| Some(*part) == parent && *leaf == name)
                        .map_or(name, |(_, _, xal2)| (*xal2).to_string());
                    address.fields.push((name, text.to_string()));
                }
            }
            Event::Eof => {
                return Err(Error::invalid(
                    "unexpected end of document inside <address>",
                ));
            }
            _ => {}
        }
    }
    Ok(address)
}

/// Read a nesting property (positioned after its `Start`, ending at
/// `End(prop_name)`).
///
/// A child [`NESTED`] names, in the property's own namespace, becomes a part
/// or child object of `b`, and so does a child of an ADE's class. Anything
/// else only has its polygons registered as `b`'s xlink targets: a parent's
/// `lodNSolid` may compose its faces from them, and they are never geometry
/// of their own, which would double the object. A property that refers to
/// its sub-feature by `xlink:href` (a storey's `buildingUnit`, a unit's
/// `buildingRoom`) holds nothing: the feature is read where it is defined.
fn read_nested<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    b: &mut ObjectBuilder,
    prop_name: &[u8],
    uri: &[u8],
    cx: Cx<'_>,
) -> Result<()> {
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        match ev {
            Event::Start(e) => {
                let local = e.local_name();
                let child_uri = ns_of(&rr).unwrap_or_default();
                let nested = ns_eq(&rr, uri)
                    .then(|| nested_class(uri, local.as_ref()))
                    .flatten()
                    .or_else(|| {
                        ade_class(&child_uri, local.as_ref()).map(|class| (class, Nesting::Child))
                    });
                if let Some((class, nesting)) = nested {
                    let end = local.as_ref().to_vec();
                    let id = gml_id(&e);
                    let child = read_object(reader, buf, class, id, &end, cx.deeper())?;
                    match nesting {
                        Nesting::Part => b.add_part(child),
                        Nesting::Child => b.add_child(child),
                    }
                } else {
                    b.register(gml::collect_surfaces(reader, buf)?);
                }
            }
            Event::End(e) if e.local_name().as_ref() == prop_name => break,
            Event::Eof => {
                return Err(Error::invalid(format!(
                    "unexpected end of document inside <{}>",
                    String::from_utf8_lossy(prop_name)
                )));
            }
            _ => {}
        }
    }
    Ok(())
}

/// Read a transportation space property (positioned after its `Start`,
/// ending at `End(prop_name)`): the boundary surfaces and markings of the
/// space it holds, and of the spaces that space holds in turn (a section's
/// traffic spaces), as `b`'s own. Anything else a space says (its
/// attributes, its own geometry) is skipped.
fn read_spaces<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    b: &mut ObjectBuilder,
    prop_name: &[u8],
) -> Result<()> {
    // Open spaces and space properties below the property.
    let mut depth = 0usize;
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        match ev {
            Event::Start(e) => {
                let local = e.local_name();
                let name = local.as_ref().to_vec();
                let module = ns_of(&rr).and_then(|uri| ns::module(&uri));
                let inside_property = depth.is_multiple_of(2);
                match module {
                    Some(Module::Transportation) if inside_property => depth += 1,
                    Some(m) if is_boundary_property(m, &name) => {
                        read_boundary(reader, buf, b, &name)?;
                    }
                    Some(m) if is_space_property(m, &name) => depth += 1,
                    _ => skip_element(reader, buf)?,
                }
            }
            Event::End(e) => {
                if depth == 0 {
                    if e.local_name().as_ref() == prop_name {
                        break;
                    }
                } else {
                    depth -= 1;
                }
            }
            Event::Eof => {
                return Err(Error::invalid(format!(
                    "unexpected end of document inside <{}>",
                    String::from_utf8_lossy(prop_name)
                )));
            }
            _ => {}
        }
    }
    Ok(())
}

/// Read an ADE hook (positioned after its `Start`, ending at
/// `End(prop_name)`): the properties of the ADE element it holds
/// (`uro:BuildingProperties`), each kept verbatim as one of `b`'s ADE
/// properties. The element itself, which only gathers them, is not kept.
fn read_ade_hook<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    b: &mut ObjectBuilder,
    prop_name: &[u8],
) -> Result<()> {
    let mut inside = false;
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        match ev {
            Event::Start(e) => {
                let uri = ns_of(&rr).unwrap_or_default();
                if !is_ade(&uri) {
                    skip_element(reader, buf)?;
                } else if inside {
                    let attributes = xml_attributes(&e)?;
                    let name = qname(&uri, e.local_name().as_ref());
                    b.push_ade(read_xml_node(reader, buf, name, attributes, ns::namespace)?);
                } else {
                    inside = true;
                }
            }
            Event::End(e) => {
                if inside {
                    inside = false;
                } else if e.local_name().as_ref() == prop_name {
                    break;
                }
            }
            Event::Eof => {
                return Err(Error::invalid(format!(
                    "unexpected end of document inside <{}>",
                    String::from_utf8_lossy(prop_name)
                )));
            }
            _ => {}
        }
    }
    Ok(())
}

/// Read a group's `grp:groupMember` (positioned after its `Start`): the
/// `grp:Role` it holds, whose `grp:role` names the member's role and whose
/// own `grp:groupMember` is the member, by `xlink:href` or inline. An inline
/// member of any class this reader reads, at any level, or of an ADE's
/// class, is kept as the group's child; one of a class it does not read is
/// skipped.
fn read_group_member<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    b: &mut ObjectBuilder,
    cx: Cx<'_>,
) -> Result<()> {
    let mut role: Option<String> = None;
    // Whether the reader is inside the `grp:Role`, and inside its member.
    let (mut in_role, mut in_member) = (false, false);
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        let group_ns = ns_eq(&rr, ns::CITY_OBJECT_GROUP.as_bytes());
        match ev {
            Event::Start(e) => {
                let local = e.local_name();
                let uri = ns_of(&rr).unwrap_or_default();
                match local.as_ref() {
                    b"Role" if group_ns && !in_role => in_role = true,
                    b"role" if group_ns && in_role && !in_member => {
                        role = read_leaf_text(reader, buf, b"role")?.map(|r| r.trim().to_string());
                    }
                    b"groupMember" if group_ns && in_role && !in_member => {
                        match get_attr_local(&e, b"href") {
                            Some(href) => {
                                skip_element(reader, buf)?;
                                if let Some(id) = member_id(&href) {
                                    b.add_member_ref(id, role.clone());
                                }
                            }
                            None => in_member = true,
                        }
                    }
                    _ if in_member => {
                        let class = top_level_class(&uri, local.as_ref())
                            .or_else(|| nested_class(&uri, local.as_ref()).map(|(class, _)| class))
                            .or_else(|| ade_class(&uri, local.as_ref()));
                        match class {
                            Some(class) => {
                                let end = local.as_ref().to_vec();
                                let id = gml_id(&e);
                                let member =
                                    read_object(reader, buf, class, id, &end, cx.deeper())?;
                                b.add_member(member, role.clone());
                            }
                            None => skip_element(reader, buf)?,
                        }
                    }
                    _ => skip_element(reader, buf)?,
                }
            }
            Event::End(e) => match e.local_name().as_ref() {
                b"groupMember" if in_member => in_member = false,
                b"Role" if in_role => in_role = false,
                b"groupMember" => break,
                _ => {}
            },
            Event::Eof => {
                return Err(Error::invalid(
                    "unexpected end of document inside <groupMember>",
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

/// A boundary surface's geometry property is `lodNMultiSurface`; its LoD.
fn boundary_lod(local: &[u8]) -> Option<String> {
    lod_suffix(local, b"MultiSurface")
}

/// Whether a boundary surface class is CityGML 3.0's generic one, which
/// CityJSON has no semantic surface type for.
fn is_generic_surface(uri: &[u8], local: &[u8]) -> bool {
    ns::module(uri) == Some(Module::Generics) && local == b"GenericThematicSurface"
}

/// A boundary property (positioned after its `Start`, ending at
/// `End(prop_name)`): each child in a CityGML module's namespace, or an
/// ADE's, is a boundary surface, its element name its class. A
/// `gen:GenericThematicSurface`'s geometry is the object's, untyped.
fn read_boundary<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    b: &mut ObjectBuilder,
    prop_name: &[u8],
) -> Result<()> {
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        match ev {
            Event::Start(e) => {
                let local = e.local_name();
                let name = local.as_ref().to_vec();
                let uri = ns_of(&rr).unwrap_or_default();
                if is_generic_surface(&uri, &name) {
                    read_surface(reader, buf, b, None, &name, 0)?;
                } else if ns::module(&uri).is_some() || is_ade(&uri) {
                    let idx = b.push_boundary_surface(qname(&uri, &name), None);
                    read_surface(reader, buf, b, Some(idx), &name, 0)?;
                } else {
                    skip_element(reader, buf)?;
                }
            }
            Event::End(e) if e.local_name().as_ref() == prop_name => break,
            Event::Eof => {
                return Err(Error::invalid(format!(
                    "unexpected end of document inside <{}>",
                    String::from_utf8_lossy(prop_name)
                )));
            }
            _ => {}
        }
    }
    Ok(())
}

/// How deep boundary surfaces may nest in one another: a wall holds a door
/// or window, which holds nothing. A guard against attacker-controlled
/// recursion.
const MAX_SURFACE_DEPTH: usize = 8;

/// One level deeper than `depth`, or an error past [`MAX_SURFACE_DEPTH`].
fn deeper_surface(depth: usize) -> Result<usize> {
    if depth >= MAX_SURFACE_DEPTH {
        return Err(Error::invalid(format!(
            "boundary surfaces nest more than {MAX_SURFACE_DEPTH} levels deep"
        )));
    }
    Ok(depth + 1)
}

/// One boundary surface's subtree (surface `surface`, or the object itself
/// when `None`, ending at `end_name`): the polygons of its `lodN`
/// geometry, and the filling surfaces in its `con:fillingSurface`s, each a
/// surface of its own nested in it.
fn read_surface<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    b: &mut ObjectBuilder,
    surface: Option<usize>,
    end_name: &[u8],
    depth: usize,
) -> Result<()> {
    let depth = deeper_surface(depth)?;
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        match ev {
            Event::Start(e) => {
                let local = e.local_name();
                let name = local.as_ref().to_vec();
                let core = ns_eq(&rr, ns::CORE.as_bytes());
                let construction = ns_eq(&rr, ns::CONSTRUCTION.as_bytes());
                if core && let Some(lod) = boundary_lod(&name) {
                    let collected = gml::collect_surfaces(reader, buf)?;
                    match surface {
                        Some(surface) => b.add_boundary_geometry(surface, &lod, collected),
                        None => b.add_surfaces(lod, collected),
                    }
                } else if construction && name == b"fillingSurface" {
                    read_filling(reader, buf, b, surface, depth)?;
                } else {
                    skip_element(reader, buf)?;
                }
            }
            Event::End(e) if e.local_name().as_ref() == end_name => break,
            Event::Eof => {
                return Err(Error::invalid(
                    "unexpected end of document inside a boundary surface",
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

/// A `con:fillingSurface` (positioned after its `Start`): its door or window
/// surface, nested in `parent`.
fn read_filling<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    b: &mut ObjectBuilder,
    parent: Option<usize>,
    depth: usize,
) -> Result<()> {
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        match ev {
            Event::Start(e) => {
                let local = e.local_name();
                let name = local.as_ref().to_vec();
                let uri = ns_of(&rr).unwrap_or_default();
                if ns::module(&uri).is_some() || is_ade(&uri) {
                    let idx = b.push_boundary_surface(qname(&uri, &name), parent);
                    read_surface(reader, buf, b, Some(idx), &name, depth)?;
                } else {
                    skip_element(reader, buf)?;
                }
            }
            Event::End(e) if e.local_name().as_ref() == b"fillingSurface" => break,
            Event::Eof => {
                return Err(Error::invalid(
                    "unexpected end of document inside <fillingSurface>",
                ));
            }
            _ => {}
        }
    }
    Ok(())
}
