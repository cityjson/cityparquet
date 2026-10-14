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
//! - `gml:name`, `gml:description`, `gen:` generic attributes and
//!   `app:appearance` are read wherever they appear.

use std::io::BufRead;

use citygml_core::gml::ObjectBuilder;
use citygml_core::gml::read::{self as gml, Collected, SolidGeom};
use citygml_core::model::{
    Address, Attribute, AttributeName, Instance, Line, Module, Namespace, Polygon, QName,
};
use citygml_core::xml::{
    NS_GML, get_attr_local, gml_id, ns_eq, ns_is, ns_of, skip_element, unescaped_text, xml_err,
};
use citygml_core::{Error, Result};
use quick_xml::events::Event;
use quick_xml::reader::NsReader;

use super::appearance::read_appearance;
use super::attributes::{self, AttrType};
use super::templates::TemplateIndex;
use crate::ns;

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
enum Nesting {
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

fn nested_class(uri: &[u8], local: &[u8]) -> Option<(QName, Nesting)> {
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
                    b.add_appearance(read_appearance(reader, buf, b"appearance")?);
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
                } else {
                    // A property of another namespace (an ADE's): a text-only
                    // one is kept as a string attribute; a structured one is
                    // dropped.
                    push_leaf(reader, buf, &mut b, &name, AttrType::Str, || {
                        qname(&uri, &name)
                    })?;
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
    if let Some(lod) = lod_suffix(name, b"Solid") {
        let geom = read_lod_solid(reader, buf, name.to_vec())?;
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
        if let Some(instance) = read_implicit(reader, buf, name, cx.templates)? {
            b.add_instance(lod, instance);
        }
    } else if let Some(lod) =
        lod_suffix(name, b"MultiCurve").or_else(|| lod_suffix(name, b"Network"))
    {
        // `lodNMultiCurve`, and a transportation object's `lodNNetwork`
        // (a `gml:GeometricComplex` of curves).
        b.add_lines(lod, gml::collect_lines(reader, buf)?);
    } else if module == Module::Relief && name == b"lod" {
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
            value: attributes::to_value(&text, ty),
        });
    }
    Ok(())
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

/// The polygons of a surface-collection subtree, without their xlinks.
fn surface_polygons<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
) -> Result<Vec<Polygon>> {
    Ok(gml::collect_polygons(reader, buf)?
        .into_iter()
        .map(|(_, p)| p)
        .collect())
}

/// Read a nesting property (positioned after its `Start`, ending at
/// `End(prop_name)`).
///
/// A child [`NESTED`] names, in the property's own namespace, becomes a part
/// or child object of `b`. Anything else only has its polygons registered as
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
                let nested = ns_eq(&rr, uri)
                    .then(|| nested_class(uri, local.as_ref()))
                    .flatten();
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

/// The `gml:id` a group member's `xlink:href` names in this document
/// (`#id`). A member in another document cannot be linked, so it is `None`.
pub fn member_id(href: &str) -> Option<String> {
    let id = href.strip_prefix('#')?;
    (!id.is_empty() && !id.contains('#')).then(|| id.to_string())
}

/// Read a `grp:groupMember` holding its member inline (positioned after its
/// `Start`): an object of any class this reader reads, at any level, kept
/// as the group's child. A member of a class it does not read is skipped.
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
                    .or_else(|| nested_class(&uri, local.as_ref()).map(|(class, _)| class));
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

/// `lodNImplicitRepresentation` -> the LoD `"N"`, else `None`.
pub(crate) fn implicit_lod(local: &[u8]) -> Option<String> {
    lod_suffix(local, b"ImplicitRepresentation")
}

/// `lodN<suffix>` -> the LoD `"N"`, else `None`.
fn lod_suffix(local: &[u8], suffix: &[u8]) -> Option<String> {
    let s = std::str::from_utf8(local).ok()?;
    let suffix = std::str::from_utf8(suffix).ok()?;
    let digits = s.strip_prefix("lod")?.strip_suffix(suffix)?;
    if !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) {
        Some(digits.to_string())
    } else {
        None
    }
}

/// Whether a module property holds the object's boundary surfaces:
/// `boundedBy` for the modules whose features have them, and a
/// transportation object's `trafficArea` / `auxiliaryTrafficArea`. CityJSON
/// models traffic areas as semantic surfaces of the object's own geometry,
/// the same shape, so they are read the same way. A traffic area's own
/// attributes are not carried onto the surface, exactly as a `WallSurface`'s
/// are not.
fn is_semantic_surface_property(local: &[u8]) -> bool {
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
fn is_nesting_property(local: &[u8]) -> bool {
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

/// The geometry a `lodNGeometry` property wraps.
pub(crate) enum LodGeom {
    Solid(SolidGeom),
    Surfaces(Vec<Polygon>),
    Lines(Vec<Line>),
    Points(Vec<[f64; 3]>),
}

/// Inside a `lodNGeometry` or a `relativeGMLGeometry` (positioned after its
/// `Start`, ending at `End(end)`): a `gml:Solid` / `CompositeSolid` is a
/// solid, a `gml:MultiSurface` / `CompositeSurface` / `Polygon` a surface
/// list, a curve or curve aggregate lines, and a `gml:Point` / `MultiPoint`
/// points. Also returns the `gml:id` of the first geometry element.
pub(crate) fn read_lod_geometry<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    end: &[u8],
) -> Result<(Option<String>, Option<LodGeom>)> {
    let end = end.to_vec();
    let mut id: Option<String> = None;
    let mut first = true;
    let mut solid: Option<SolidGeom> = None;
    let mut polys: Vec<Polygon> = Vec::new();
    let mut lines: Vec<Line> = Vec::new();
    let mut points: Vec<[f64; 3]> = Vec::new();
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        let is_gml = ns_is(&rr, NS_GML);
        match ev {
            Event::Start(e) => {
                if std::mem::take(&mut first) {
                    id = gml_id(&e);
                }
                let local = e.local_name();
                match (is_gml, local.as_ref()) {
                    (true, b"Solid") => {
                        solid = Some(SolidGeom::Solid(gml::read_solid(reader, buf)?))
                    }
                    (true, b"CompositeSolid") => {
                        solid = Some(SolidGeom::Composite(gml::read_composite_solid(
                            reader, buf,
                        )?))
                    }
                    (true, b"MultiSurface") | (true, b"CompositeSurface") => {
                        // A material or texture may target the aggregate.
                        let aggregate = gml_id(&e);
                        polys.extend(surface_polygons(reader, buf)?.into_iter().map(|mut p| {
                            p.groups.extend(aggregate.clone());
                            p
                        }));
                    }
                    (true, b"Polygon") => {
                        let id = gml_id(&e);
                        let mut poly = gml::read_polygon(reader, buf)?;
                        poly.id = id;
                        polys.push(poly);
                    }
                    (
                        true,
                        b"LineString" | b"Curve" | b"MultiCurve" | b"CompositeCurve"
                        | b"OrientableCurve",
                    ) => {
                        let local = local.as_ref().to_vec();
                        lines.extend(gml::read_curve(reader, buf, &local)?);
                    }
                    (true, b"Point" | b"MultiPoint") => {
                        points.extend(gml::collect_points(reader, buf)?);
                    }
                    _ => skip_element(reader, buf)?,
                }
            }
            Event::End(e) if e.local_name().as_ref() == end.as_slice() => break,
            Event::Eof => {
                return Err(Error::invalid(
                    "unexpected end of document inside a lodNGeometry",
                ));
            }
            _ => {}
        }
    }
    let geom = solid
        .map(LodGeom::Solid)
        .or_else(|| (!polys.is_empty()).then_some(LodGeom::Surfaces(polys)))
        .or_else(|| (!lines.is_empty()).then_some(LodGeom::Lines(lines)))
        .or_else(|| (!points.is_empty()).then_some(LodGeom::Points(points)));
    Ok((id, geom))
}

/// Record what [`read_lod_geometry`] read as `b`'s geometry at `lod`.
pub(crate) fn add_lod_geometry(b: &mut ObjectBuilder, lod: String, geom: Option<LodGeom>) {
    match geom {
        Some(LodGeom::Solid(geom)) => b.add_solid(lod, geom),
        Some(LodGeom::Surfaces(polys)) => b.add_plain_surfaces(lod, polys),
        Some(LodGeom::Lines(lines)) => b.add_lines(lod, lines),
        Some(LodGeom::Points(points)) => b.add_points(lod, points),
        None => {}
    }
}

/// An `lodNImplicitRepresentation` (positioned after its `Start`, ending at
/// `End(end)`) as an [`Instance`] of one of `templates`. `None` when it
/// names no relative geometry of this document (a `libraryObject` only, or
/// an `xlink:href` to another document or to the `ImplicitGeometry` itself),
/// or has no reference point.
fn read_implicit<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    end: &[u8],
    templates: &TemplateIndex,
) -> Result<Option<Instance>> {
    let mut matrix = IDENTITY;
    let mut template: Option<usize> = None;
    let mut reference_point: Option<[f64; 3]> = None;
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        match ev {
            Event::Start(e) => {
                let local = e.local_name();
                match (ns_is(&rr, ns::CORE), local.as_ref()) {
                    (true, b"ImplicitGeometry") => {}
                    (true, b"transformationMatrix") => {
                        let text = read_leaf_text(reader, buf, b"transformationMatrix")?;
                        matrix = parse_matrix(text.as_deref().unwrap_or_default())?;
                    }
                    (true, b"relativeGMLGeometry") => {
                        template = match get_attr_local(&e, b"href") {
                            Some(href) => member_id(&href).and_then(|id| templates.by_id(&id)),
                            None => templates.at(reader.buffer_position()),
                        };
                        skip_element(reader, buf)?;
                    }
                    (true, b"referencePoint") => {
                        reference_point = gml::collect_points(reader, buf)?.first().copied();
                    }
                    _ => skip_element(reader, buf)?,
                }
            }
            Event::End(e) if e.local_name().as_ref() == end => break,
            Event::Eof => {
                return Err(Error::invalid(
                    "unexpected end of document inside an lodNImplicitRepresentation",
                ));
            }
            _ => {}
        }
    }
    Ok(template
        .zip(reference_point)
        .map(|(template, reference_point)| Instance {
            template,
            matrix,
            reference_point,
        }))
}

const IDENTITY: [f64; 16] = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];

/// A `transformationMatrix`: sixteen numbers, row by row.
fn parse_matrix(text: &str) -> Result<[f64; 16]> {
    let values = text
        .split_whitespace()
        .map(|t| t.parse::<f64>().ok().filter(|v| v.is_finite()))
        .collect::<Option<Vec<f64>>>();
    values
        .and_then(|v| <[f64; 16]>::try_from(v).ok())
        .ok_or_else(|| {
            Error::invalid(format!(
                "a transformationMatrix is sixteen finite numbers, not {text:?}"
            ))
        })
}

/// Inside a `lodNSolid`: the wrapped `gml:Solid` or `gml:CompositeSolid`.
fn read_lod_solid<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    end: Vec<u8>,
) -> Result<SolidGeom> {
    let mut geom: Option<SolidGeom> = None;
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        let is_gml = ns_is(&rr, NS_GML);
        match ev {
            Event::Start(e) => {
                let local = e.local_name();
                match (is_gml, local.as_ref()) {
                    (true, b"Solid") => {
                        geom = Some(SolidGeom::Solid(gml::read_solid(reader, buf)?))
                    }
                    (true, b"CompositeSolid") => {
                        geom = Some(SolidGeom::Composite(gml::read_composite_solid(
                            reader, buf,
                        )?))
                    }
                    _ => skip_element(reader, buf)?,
                }
            }
            Event::End(e) if e.local_name().as_ref() == end.as_slice() => break,
            Event::Eof => {
                return Err(Error::invalid(
                    "unexpected end of document inside <lodNSolid>",
                ));
            }
            _ => {}
        }
    }
    geom.ok_or_else(|| Error::invalid("lodNSolid without gml:Solid or gml:CompositeSolid"))
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
/// `ns` is a boundary surface, its element name its class.
///
/// `ns` comes from the property element, which the schema puts in the owning
/// feature's module: a bridge's boundary surfaces are `brid:`, a building's
/// `bldg:`. Matching the CityGML family instead would admit a
/// `_GenericApplicationPropertyOf…` ADE hook.
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
                if own_module {
                    let name = local.as_ref().to_vec();
                    let idx = b.push_boundary_surface(qname(ns, &name), None);
                    read_semantic_surface(reader, buf, b, idx, &name, ns)?;
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
/// its own.
fn read_semantic_surface<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    b: &mut ObjectBuilder,
    surface: usize,
    end_name: &[u8],
    ns: &[u8],
) -> Result<()> {
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        let own_module = ns_eq(&rr, ns);
        match ev {
            Event::Start(e) => {
                let local = e.local_name();
                let name = local.as_ref().to_vec();
                if own_module && let Some(lod) = boundary_lod(&name) {
                    let collected = gml::collect_surfaces(reader, buf)?;
                    b.add_boundary_geometry(surface, &lod, collected);
                } else if own_module && SEMANTIC_SURFACES.contains(&name.as_slice()) {
                    let idx = b.push_boundary_surface(qname(ns, &name), Some(surface));
                    read_semantic_surface(reader, buf, b, idx, &name, ns)?;
                } else if own_module && name == b"opening" {
                    read_semantic_surface(reader, buf, b, surface, &name, ns)?;
                } else {
                    skip_element(reader, buf)?;
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

/// A leaf element's text (positioned after its `Start`, ending at
/// `End(end)`), verbatim: `None` for an empty or blank one, or one with
/// child elements.
fn read_leaf_text<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    end: &[u8],
) -> Result<Option<String>> {
    let mut text = String::new();
    let mut has_child = false;
    loop {
        buf.clear();
        match reader.read_event_into(buf).map_err(xml_err)? {
            Event::Text(t) => text.push_str(&unescaped_text(&t)?),
            Event::CData(t) => text.push_str(&String::from_utf8_lossy(&t)),
            Event::Start(_) => {
                has_child = true;
                skip_element(reader, buf)?;
            }
            Event::End(e) if e.local_name().as_ref() == end => break,
            Event::Eof => {
                return Err(Error::invalid(
                    "unexpected end of document inside a typed property",
                ));
            }
            _ => {}
        }
    }
    Ok((!has_child && !text.trim().is_empty()).then_some(text))
}
