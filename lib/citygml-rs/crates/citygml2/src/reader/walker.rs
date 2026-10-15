//! Read one city object's subtree into an [`ObjectBuilder`].
//!
//! One walker reads every class: [`read_object`]. What it does with each
//! child element depends on the element's namespace and name, never on the
//! class being read, since CityGML 2.0 names its properties uniformly across
//! modules:
//!
//! - `lodNSolid`, `lodNMultiSurface`, `lodNGeometry`, and a building's
//!   `lod0FootPrint` / `lod0RoofEdge`, are geometry;
//! - `lodNImplicitRepresentation` places one of the document's relative
//!   geometries ([`TemplateIndex`]);
//! - `boundedBy`, and a transportation object's `trafficArea` /
//!   `auxiliaryTrafficArea`, hold boundary surfaces;
//! - `consistsOf…Part`, `outer…` / `interior…` and the room installation
//!   properties nest sub-features, read as objects of their own when
//!   [`NESTED`] names their class;
//! - `address` is a postal address;
//! - a group's `groupMember` is a member: an object of its own when nested,
//!   a `gml:id` when referenced by `xlink:href`;
//! - any other text-only property of a CityGML module is an attribute, typed
//!   as [`attributes::property_type`] says;
//! - a property in an ADE's namespace ([`is_ade`]) is kept verbatim as
//!   an [`XmlNode`], unless it is an ADE object's own geometry or boundary
//!   surfaces, which are read as a CityGML object's are;
//! - `gml:name`, `gml:description`, `gen:` generic attributes and
//!   `app:appearance` are read wherever they appear.
//!
//! An element of an ADE's namespace where a city object belongs (in a
//! `cityObjectMember`, a nesting property or a group's `groupMember`) is an
//! object of that ADE's class, read by the same walker.

use std::io::BufRead;

use citygml_core::gml::ObjectBuilder;
use citygml_core::gml::read::{self as gml, Collected};
use citygml_core::model::{Address, Attribute, AttributeName, Module, Namespace, QName};
use citygml_core::reader::appearance::read_appearance;
use citygml_core::reader::geometry::{
    ImplicitNames, add_lod_geometry, implicit_lod, lod_suffix, read_implicit, read_lod_geometry,
    read_lod_solid,
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

/// How CityGML 2.0 spells an `ImplicitGeometry`.
pub(crate) const IMPLICIT: ImplicitNames<'static> = ImplicitNames {
    namespace: ns::CORE,
    relative_geometry: b"relativeGMLGeometry",
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

/// The CityGML 2.0 top-level classes this reader reads.
///
/// A `ReliefFeature` is not an object of its own: CityJSON has no type for
/// it, so the reader passes through it to its components (see
/// [`is_container`]). Of those, only a `TINRelief` maps to CityJSON.
const TOP_LEVEL: &[(&str, Module)] = &[
    ("Building", Module::Building),
    ("Bridge", Module::Bridge),
    ("Tunnel", Module::Tunnel),
    ("WaterBody", Module::WaterBody),
    ("LandUse", Module::LandUse),
    ("CityFurniture", Module::CityFurniture),
    ("SolitaryVegetationObject", Module::Vegetation),
    ("PlantCover", Module::Vegetation),
    ("GenericCityObject", Module::Generics),
    ("CityObjectGroup", Module::CityObjectGroup),
    ("Road", Module::Transportation),
    ("Railway", Module::Transportation),
    ("Track", Module::Transportation),
    ("Square", Module::Transportation),
    ("TransportationComplex", Module::Transportation),
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
    /// A part (`consistsOf…Part`): its polygons are its own.
    Part,
    /// Any other sub-feature: its polygons are also the parent's xlink
    /// targets, since a parent's solid routinely composes faces defined in
    /// one.
    Child,
}

/// The CityGML 2.0 sub-features read as objects of their own.
const NESTED: &[(&str, Module, Nesting)] = &[
    ("BuildingPart", Module::Building, Nesting::Part),
    ("BuildingInstallation", Module::Building, Nesting::Child),
    ("IntBuildingInstallation", Module::Building, Nesting::Child),
    ("Room", Module::Building, Nesting::Child),
    ("BuildingFurniture", Module::Building, Nesting::Child),
    ("BridgePart", Module::Bridge, Nesting::Part),
    ("BridgeInstallation", Module::Bridge, Nesting::Child),
    ("IntBridgeInstallation", Module::Bridge, Nesting::Child),
    ("BridgeConstructionElement", Module::Bridge, Nesting::Child),
    ("BridgeRoom", Module::Bridge, Nesting::Child),
    ("BridgeFurniture", Module::Bridge, Nesting::Child),
    ("TunnelPart", Module::Tunnel, Nesting::Part),
    ("TunnelInstallation", Module::Tunnel, Nesting::Child),
    ("IntTunnelInstallation", Module::Tunnel, Nesting::Child),
    ("HollowSpace", Module::Tunnel, Nesting::Child),
    ("TunnelFurniture", Module::Tunnel, Nesting::Child),
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
    let relation = attributes::relation_to_construction(&class);
    let mut b = ObjectBuilder::new(id, class);
    let mut pending = Pending::default();
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        match ev {
            Event::Start(e) => {
                let local = e.local_name();
                let name = local.as_ref().to_vec();
                let uri = ns_of(&rr).unwrap_or_default();
                let module = ns::module(&uri);
                if module == Some(Module::Appearance) && name == b"appearance" {
                    b.add_appearance(read_appearance(reader, buf, b"appearance", ns::APPEARANCE)?);
                } else if let (Some(Module::Generics), Some(ty)) =
                    (module, attributes::generic_attr(&name))
                {
                    let attr_name = get_attr_local(&e, b"name");
                    read_generic_attribute(reader, buf, &mut b, attr_name, ty)?;
                } else if ns_is(&rr, NS_GML) {
                    if matches!(name.as_slice(), b"name" | b"description") {
                        push_leaf(reader, buf, &mut b, &name, AttrType::Str, || {
                            QName::new(Namespace::Gml, String::from_utf8_lossy(&name))
                        })?;
                    } else if name == b"boundedBy" {
                        if let Some(extent) = gml::read_envelope(reader, buf)? {
                            b.set_extent(extent);
                        }
                    } else {
                        skip_element(reader, buf)?;
                    }
                } else if module == Some(Module::CityObjectGroup) && name == b"groupMember" {
                    let role = get_attr_local(&e, b"role");
                    match get_attr_local(&e, b"href") {
                        Some(href) => {
                            skip_element(reader, buf)?;
                            if let Some(id) = member_id(&href) {
                                b.add_member_ref(id, role);
                            }
                        }
                        None => read_group_member(reader, buf, &mut b, role, cx)?,
                    }
                } else if let Some(module) = module {
                    read_module_property(
                        reader,
                        buf,
                        &mut b,
                        &mut pending,
                        (&uri, module),
                        &name,
                        cx,
                    )?;
                } else if is_ade(&uri) {
                    let own = own_ade.as_deref() == Some(uri.as_slice());
                    let attributes = xml_attributes(&e)?;
                    if own && read_geometry_property(reader, buf, &mut b, &mut pending, &name, cx)?
                    {
                        // The ADE object's own geometry, read.
                    } else if own && is_semantic_surface_property(&name) {
                        read_bounded_by(reader, buf, &mut b, &uri, &name)?;
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
    if let Some((lod, collected)) = pending.roof_edge
        && !b.has_plain_surfaces(&lod)
    {
        b.add_surfaces(lod, collected);
    }
    if let Some(surfaces) = pending.tin.surfaces {
        // A TIN without its LoD is at LoD 1, the LoD a relief has when its
        // LoD is not otherwise stated.
        b.add_tin(pending.tin.lod.unwrap_or_else(|| "1".into()), surfaces);
    }
    if let Some(relation) = relation {
        b.push_attribute(relation);
    }
    Ok(b)
}

/// What an object's subtree holds that is only placed once the whole
/// subtree has been read, since element order is not guaranteed.
#[derive(Default)]
struct Pending {
    /// A building's `lod0RoofEdge`.
    roof_edge: Option<(String, Collected)>,
    /// A `TINRelief`'s `tin` and its `lod`.
    tin: Tin,
}

#[derive(Default)]
struct Tin {
    lod: Option<String>,
    surfaces: Option<Collected>,
}

/// A property in a CityGML module namespace `uri` (positioned after its
/// `Start`).
fn read_module_property<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    b: &mut ObjectBuilder,
    pending: &mut Pending,
    (uri, module): (&[u8], Module),
    name: &[u8],
    cx: Cx<'_>,
) -> Result<()> {
    if read_geometry_property(reader, buf, b, pending, name, cx)? {
        return Ok(());
    }
    if module == Module::Relief && name == b"lod" {
        // A relief component's LoD is the LoD of its TIN, not an attribute.
        if let Some(text) = read_leaf_text(reader, buf, name)? {
            pending.tin.lod = Some(text.trim().to_string());
        }
    } else if module == Module::Relief && name == b"tin" {
        pending.tin.surfaces = Some(gml::collect_surfaces(reader, buf)?);
    } else if is_semantic_surface_property(name) {
        read_bounded_by(reader, buf, b, uri, name)?;
    } else if name == b"address" {
        b.push_address(read_address(reader, buf)?);
    } else if is_nesting_property(name) {
        read_nested(reader, buf, b, name, uri, cx)?;
    } else {
        let ty = attributes::property_type(module, name);
        push_leaf(reader, buf, b, name, ty, || {
            QName::module(module, String::from_utf8_lossy(name))
        })?;
    }
    Ok(())
}

/// A geometry property (positioned after its `Start`), named the way every
/// CityGML 2.0 module names one: `lodNSolid`, `lodNMultiSurface`,
/// `lodNGeometry`, `lodNMultiCurve`, `lodNImplicitRepresentation`, a
/// transportation object's `lodNNetwork` and a building's `lod0FootPrint` /
/// `lod0RoofEdge`. `false`, having read nothing, for any other name.
fn read_geometry_property<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    b: &mut ObjectBuilder,
    pending: &mut Pending,
    name: &[u8],
    cx: Cx<'_>,
) -> Result<bool> {
    if let Some(lod) = lod_suffix(name, b"Solid") {
        let geom = read_lod_solid(reader, buf, name)?;
        b.add_solid(lod, geom);
    } else if let Some(lod) =
        lod_suffix(name, b"MultiSurface").or_else(|| lod_suffix(name, b"FootPrint"))
    {
        // CityGML 2.0 spells a building's LoD0 with two dedicated elements
        // rather than `lod0MultiSurface`: `lod0FootPrint` (the ground
        // outline) and `lod0RoofEdge` (the roof outline). They are different
        // geometry, and CityJSON cannot say which a LoD0 MultiSurface is, so
        // they are never merged: the footprint is the LoD0 geometry, and a
        // roof edge is kept only for a building with no footprint (held back
        // until the subtree is read, since element order is not guaranteed).
        b.add_surfaces(lod, gml::collect_surfaces(reader, buf)?);
    } else if let Some(lod) = lod_suffix(name, b"RoofEdge") {
        let collected = gml::collect_surfaces(reader, buf)?;
        if pending.roof_edge.is_none() {
            pending.roof_edge = Some((lod, collected));
        }
    } else if let Some(lod) = lod_suffix(name, b"Geometry") {
        // `lodNGeometry` may wrap a solid or a surface collection.
        let (_, geom) = read_lod_geometry(reader, buf, name)?;
        add_lod_geometry(b, lod, geom);
    } else if let Some(lod) = implicit_lod(name) {
        if let Some(instance) = read_implicit(reader, buf, name, cx.templates, IMPLICIT)? {
            b.add_instance(lod, instance);
        }
    } else if let Some(lod) =
        lod_suffix(name, b"MultiCurve").or_else(|| lod_suffix(name, b"Network"))
    {
        // `lodNMultiCurve`, and a transportation object's `lodNNetwork`
        // (a `gml:GeometricComplex` of curves).
        b.add_lines(lod, gml::collect_lines(reader, buf)?);
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

/// What [`read_surface_attribute`] needs of a property's start tag.
struct SurfaceProperty {
    uri: Vec<u8>,
    /// Whether the property is in GML's namespace.
    gml: bool,
    /// Its `name` attribute, which names a generic attribute.
    generic_name: Option<String>,
    /// Whether it is in the module of the surface's owner.
    own_module: bool,
}

/// A boundary surface's property (positioned after its `Start`, named
/// `name`) as an attribute: `gml:name` and `gml:description`, a generic
/// attribute, a `core:` property, or one of its own module's (a
/// `TrafficArea`'s `function`). Anything else, an ADE's property included,
/// is skipped; so is a property holding elements.
fn read_surface_attribute<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    name: &[u8],
    property: SurfaceProperty,
) -> Result<Option<Attribute>> {
    let module = ns::module(&property.uri);
    let leaf = |reader: &mut NsReader<R>, buf: &mut Vec<u8>, ty, qname: QName| {
        Ok::<_, Error>(read_leaf_text(reader, buf, name)?.map(|text| Attribute {
            name: AttributeName::Property(qname),
            value: to_value(&text, ty),
        }))
    };
    if property.gml && matches!(name, b"name" | b"description") {
        let qname = QName::new(Namespace::Gml, String::from_utf8_lossy(name));
        return leaf(reader, buf, AttrType::Str, qname);
    }
    if let (Some(Module::Generics), Some(ty)) = (module, attributes::generic_attr(name)) {
        let value = attributes::read_generic(reader, buf, ty)?;
        return Ok(value
            .zip(property.generic_name)
            .map(|(value, name)| Attribute {
                name: AttributeName::Generic(name),
                value,
            }));
    }
    match module {
        Some(module) if property.own_module || module == Module::Core => {
            let qname = QName::module(module, String::from_utf8_lossy(name));
            leaf(reader, buf, attributes::property_type(module, name), qname)
        }
        _ => {
            skip_element(reader, buf)?;
            Ok(None)
        }
    }
}

/// A `gen:` generic attribute (positioned after its `Start`). One without a
/// `name` cannot be named, and is dropped.
fn read_generic_attribute<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    b: &mut ObjectBuilder,
    name: Option<String>,
    ty: AttrType,
) -> Result<()> {
    if let Some(value) = attributes::read_generic(reader, buf, ty)?
        && let Some(name) = name
    {
        b.push_attribute(Attribute {
            name: AttributeName::Generic(name),
            value,
        });
    }
    Ok(())
}

/// An `address` property (positioned after its `Start`): every leaf of its
/// xAL subtree that holds text, and the points of its `multiPoint`.
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
/// else only has its polygons registered as
/// `b`'s xlink targets: a parent's `lodNSolid` may compose its faces from
/// them, and they are never geometry of their own, which would double the
/// object.
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

/// Read a `grp:groupMember` holding its member inline (positioned after its
/// `Start`): an object of any class this reader reads, at any level, or of
/// an ADE's class, kept as the group's child. A member of a class it does not read is skipped.
fn read_group_member<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    b: &mut ObjectBuilder,
    role: Option<String>,
    cx: Cx<'_>,
) -> Result<()> {
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        match ev {
            Event::Start(e) => {
                let local = e.local_name();
                let uri = ns_of(&rr).unwrap_or_default();
                let class = top_level_class(&uri, local.as_ref())
                    .or_else(|| nested_class(&uri, local.as_ref()).map(|(class, _)| class))
                    .or_else(|| ade_class(&uri, local.as_ref()));
                match class {
                    Some(class) => {
                        let end = local.as_ref().to_vec();
                        let id = gml_id(&e);
                        let member = read_object(reader, buf, class, id, &end, cx.deeper())?;
                        b.add_member(member, role.clone());
                    }
                    None => skip_element(reader, buf)?,
                }
            }
            Event::End(e) if e.local_name().as_ref() == b"groupMember" => break,
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

/// Whether a module property holds the object's boundary surfaces:
/// `boundedBy` for the modules whose features have them, and a
/// transportation object's `trafficArea` / `auxiliaryTrafficArea`. CityJSON
/// models traffic areas as semantic surfaces of the object's own geometry,
/// the same shape, so they are read the same way. A traffic area's own
/// attributes are not carried onto the surface, exactly as a `WallSurface`'s
/// are not.
pub(crate) fn is_semantic_surface_property(local: &[u8]) -> bool {
    matches!(
        local,
        b"boundedBy" | b"trafficArea" | b"auxiliaryTrafficArea"
    )
}

/// Whether a module property is one of CityGML 2.0's nesting properties: the
/// places the schema puts a city object's sub-features (`consistsOf…Part`,
/// `outer…` and `interior…`, and a room's or hollow space's installations),
/// and so the only places a parent's `lodNSolid` can find the polygons it
/// composes by `xlink:href`. Matched as a closed vocabulary rather than by
/// descending into every unknown element, so an ADE subtree is skipped by
/// bytes rather than parsed for geometry nothing will reference.
pub(crate) fn is_nesting_property(local: &[u8]) -> bool {
    let Ok(name) = std::str::from_utf8(local) else {
        return false;
    };
    name.starts_with("outer")
        || name.starts_with("interior")
        || name.starts_with("consistsOf")
        || name.ends_with("RoomInstallation")
        || name == "roomInstallation"
        || name == "hollowSpaceInstallation"
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

/// The boundary surface classes a semantic-surface subtree may nest
/// (`Door` and `Window` inside a wall's `opening` become surfaces of their
/// own).
const SEMANTIC_SURFACES: &[&[u8]] = &[
    b"WallSurface",
    b"RoofSurface",
    b"GroundSurface",
    b"ClosureSurface",
    b"OuterCeilingSurface",
    b"OuterFloorSurface",
    b"CeilingSurface",
    b"FloorSurface",
    b"InteriorWallSurface",
    b"Door",
    b"Window",
];

/// A boundary surface's geometry property is `lodN{MultiSurface,Geometry,
/// Surface}`; its LoD. `MultiSurface` is tried first so its trailing
/// `Surface` is not stripped wrongly.
fn boundary_lod(local: &[u8]) -> Option<String> {
    lod_suffix(local, b"MultiSurface")
        .or_else(|| lod_suffix(local, b"Geometry"))
        .or_else(|| lod_suffix(local, b"Surface"))
}

/// A semantic-surface property (positioned after its `Start`, ending at
/// `End(prop_name)`): each child in the owning feature's module namespace
/// `ns`, or in an ADE's namespace, is a boundary surface, its element name
/// its class.
///
/// `ns` comes from the property element, which the schema puts in the owning
/// feature's module: a bridge's boundary surfaces are `brid:`, a building's
/// `bldg:`. Matching the CityGML family instead would admit a surface of
/// another module. An ADE's surface class substitutes for the module's
/// abstract boundary surface, so its geometry property may be in either
/// namespace.
fn read_bounded_by<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    b: &mut ObjectBuilder,
    ns: &[u8],
    prop_name: &[u8],
) -> Result<()> {
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        let own_module = ns_eq(&rr, ns);
        match ev {
            Event::Start(e) => {
                let local = e.local_name();
                let uri = ns_of(&rr).unwrap_or_default();
                if own_module || is_ade(&uri) {
                    let name = local.as_ref().to_vec();
                    let idx = b.push_boundary_surface(qname(&uri, &name), None);
                    read_semantic_surface(reader, buf, b, idx, &name, (ns, &uri), 0)?;
                } else {
                    skip_element(reader, buf)?;
                }
            }
            Event::End(e) if e.local_name().as_ref() == prop_name => break,
            Event::Eof => {
                return Err(Error::invalid(
                    "unexpected end of document inside <boundedBy>",
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

/// One boundary surface's subtree (surface `surface`, ending at `end_name`):
/// the polygons of its `lodN` geometry, and the surfaces nested in it. An
/// `opening` is descended into, so its `Door` / `Window` becomes a surface of
/// its own. `(ns, own)` are the module namespace of the feature and the
/// surface element's own namespace, which differ for an ADE's surface.
fn read_semantic_surface<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    b: &mut ObjectBuilder,
    surface: usize,
    end_name: &[u8],
    (ns, own): (&[u8], &[u8]),
    depth: usize,
) -> Result<()> {
    let depth = deeper_surface(depth)?;
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        let own_module = ns_eq(&rr, ns);
        let geometry_ns = own_module || ns_eq(&rr, own);
        match ev {
            Event::Start(e) => {
                let local = e.local_name();
                let name = local.as_ref().to_vec();
                if geometry_ns && let Some(lod) = boundary_lod(&name) {
                    let collected = gml::collect_surfaces(reader, buf)?;
                    b.add_boundary_geometry(surface, &lod, collected);
                } else if own_module && SEMANTIC_SURFACES.contains(&name.as_slice()) {
                    let idx = b.push_boundary_surface(qname(ns, &name), Some(surface));
                    read_semantic_surface(reader, buf, b, idx, &name, (ns, ns), depth)?;
                } else if own_module && name == b"opening" {
                    read_semantic_surface(reader, buf, b, surface, &name, (ns, own), depth)?;
                } else if let Some(attribute) = {
                    let property = SurfaceProperty {
                        uri: ns_of(&rr).unwrap_or_default(),
                        gml: ns_is(&rr, NS_GML),
                        generic_name: get_attr_local(&e, b"name"),
                        own_module,
                    };
                    read_surface_attribute(reader, buf, &name, property)?
                } {
                    b.push_surface_attribute(surface, attribute);
                }
            }
            Event::End(e) if e.local_name().as_ref() == end_name => break,
            Event::Eof => {
                return Err(Error::invalid(
                    "unexpected end of document inside a boundedBy semantic surface",
                ));
            }
            _ => {}
        }
    }
    Ok(())
}
