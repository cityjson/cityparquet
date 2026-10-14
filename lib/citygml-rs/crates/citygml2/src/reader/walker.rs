//! Read one city object's subtree into an [`ObjectBuilder`].
//!
//! Two walkers, one per content model this reader knows:
//!
//! - [`read_building`] reads a `bldg:Building` or `bldg:BuildingPart`:
//!   `lodNSolid`, `lod0FootPrint` / `lod0RoofEdge`, `lodNMultiSurface` (kept
//!   as xlink targets), `boundedBy`, `consistsOfBuildingPart`, the two
//!   installation properties, `app:appearance`, typed `bldg:` attributes and
//!   `gen:` generic attributes.
//! - [`read_generic_object`] reads every other mapped object: `lodN{Solid,
//!   MultiSurface,Geometry}`, boundary surfaces (`boundedBy`, and a
//!   transportation object's `trafficArea` / `auxiliaryTrafficArea`), nested
//!   features in `outer*` / `interior*` / `consistsOf*` properties, `gen:`
//!   attributes, and any other text-only module property as a string.

use std::io::BufRead;

use citygml_core::gml::ObjectBuilder;
use citygml_core::gml::read::{self as gml, SolidGeom};
use citygml_core::model::{Attribute, AttributeName, Module, Polygon, QName, Value};
use citygml_core::xml::{
    NS_GML, get_attr_local, gml_id, ns_eq, ns_is, ns_of, skip_element, unescaped_text, xml_err,
};
use citygml_core::{Error, Result};
use quick_xml::events::Event;
use quick_xml::reader::NsReader;

use super::appearance::read_appearance;
use super::attributes::{self, AttrType};
use crate::ns;

/// Maximum nesting depth of parts and nested features: a guard against
/// attacker-controlled XML recursion.
const MAX_PART_DEPTH: usize = 32;

/// The CityGML 2.0 top-level classes this reader reads besides `Building`,
/// by local name (unique across the 2.0 modules).
///
/// `ReliefFeature` is deliberately absent: a relief may be raster,
/// breaklines, mass points or a TIN, and only a TIN maps cleanly to CityJSON.
/// `Track` is absent too.
const TOP_LEVEL: &[(&str, Module)] = &[
    ("WaterBody", Module::WaterBody),
    ("LandUse", Module::LandUse),
    ("CityFurniture", Module::CityFurniture),
    ("SolitaryVegetationObject", Module::Vegetation),
    ("PlantCover", Module::Vegetation),
    ("Bridge", Module::Bridge),
    ("Tunnel", Module::Tunnel),
    ("GenericCityObject", Module::Generics),
    ("CityObjectGroup", Module::CityObjectGroup),
    ("Road", Module::Transportation),
    ("Railway", Module::Transportation),
    ("Square", Module::Transportation),
];

/// The class of a top-level object other than a `Building`, by its local
/// name, or `None` for one this reader does not read.
pub fn top_level_class(local: &[u8]) -> Option<QName> {
    lookup(TOP_LEVEL, local)
}

/// The CityGML 2.0 nested features read as objects of their own, by local
/// name. Tunnel's counterparts are absent: no fixture exercises them.
const NESTED: &[(&str, Module)] = &[
    ("BuildingInstallation", Module::Building),
    ("IntBuildingInstallation", Module::Building),
    ("BridgeInstallation", Module::Bridge),
    ("IntBridgeInstallation", Module::Bridge),
    ("BridgeConstructionElement", Module::Bridge),
    ("BridgePart", Module::Bridge),
];

fn lookup(table: &[(&str, Module)], local: &[u8]) -> Option<QName> {
    table
        .iter()
        .find(|(name, _)| name.as_bytes() == local)
        .map(|(name, module)| QName::module(*module, *name))
}

/// The name of an element: its local name in the namespace it is bound to.
fn qname(uri: &[u8], local: &[u8]) -> QName {
    QName::new(ns::namespace(uri), String::from_utf8_lossy(local))
}

/// Read a `bldg:Building` subtree (positioned after its `Start`).
pub fn read_building<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    id: Option<String>,
) -> Result<ObjectBuilder> {
    read_abstract_building(reader, buf, id, b"Building", 0)
}

/// Read an `_AbstractBuilding` subtree (a `bldg:Building` or
/// `bldg:BuildingPart`, which share their content model), ending at
/// `End(end_name)`.
fn read_abstract_building<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    id: Option<String>,
    end_name: &[u8],
    depth: usize,
) -> Result<ObjectBuilder> {
    if depth > MAX_PART_DEPTH {
        return Err(Error::invalid(format!(
            "bldg:consistsOfBuildingPart nested deeper than {MAX_PART_DEPTH}"
        )));
    }
    let class = QName::module(Module::Building, String::from_utf8_lossy(end_name));
    let mut b = ObjectBuilder::new(id, class);
    // A `lod0RoofEdge`, kept aside until the whole subtree has been read.
    let mut roof_edge: Option<(String, Vec<Polygon>)> = None;

    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        let bldg = ns_is(&rr, ns::BUILDING);
        match ev {
            Event::Start(e) => {
                let local = e.local_name();
                let name = local.as_ref().to_vec();
                if bldg && let Some(lod) = lod_suffix(&name, b"Solid") {
                    let geom = read_lod_solid(reader, buf, name)?;
                    b.add_solid(lod, geom);
                } else if bldg && let Some(lod) = lod_suffix(&name, b"FootPrint") {
                    // CityGML 2.0 spells a building's LoD0 with two dedicated
                    // elements rather than `lod0MultiSurface`: `lod0FootPrint`
                    // (the ground outline) and `lod0RoofEdge` (the roof
                    // outline). They are different geometry, and CityJSON
                    // cannot say which a LoD0 MultiSurface is, so they are
                    // never merged: the footprint is the LoD0 geometry, and a
                    // roof edge is kept only for a building with no footprint
                    // (held back until the subtree is read, since element
                    // order is not guaranteed).
                    let polys = surface_polygons(reader, buf)?;
                    b.add_plain_surfaces(lod, polys);
                } else if bldg && let Some(lod) = lod_suffix(&name, b"RoofEdge") {
                    let polys = surface_polygons(reader, buf)?;
                    if roof_edge.is_none() {
                        roof_edge = Some((lod, polys));
                    }
                } else if bldg && lod_suffix(&name, b"MultiSurface").is_some() {
                    // A building's standalone lodNMultiSurface: its polygons
                    // may be xlink targets (an internal ceiling, say).
                    b.register(gml::collect_surfaces(reader, buf)?);
                } else if bldg && name == b"boundedBy" {
                    read_bounded_by(reader, buf, &mut b, ns::BUILDING.as_bytes(), b"boundedBy")?;
                } else if bldg && name == b"consistsOfBuildingPart" {
                    read_consists_of_part(reader, buf, &mut b, depth)?;
                } else if bldg
                    && (name == b"outerBuildingInstallation"
                        || name == b"interiorBuildingInstallation")
                {
                    read_child_object(reader, buf, &mut b, &name, ns::BUILDING.as_bytes(), depth)?;
                } else if ns_is(&rr, ns::APPEARANCE) && name == b"appearance" {
                    b.add_appearance(read_appearance(reader, buf, b"appearance")?);
                } else if bldg && let Some(ty) = attributes::typed_building_attr(&name) {
                    if let Some(value) = attributes::read_typed(reader, buf, ty)? {
                        b.push_attribute(Attribute {
                            name: AttributeName::Property(QName::module(
                                Module::Building,
                                String::from_utf8_lossy(&name),
                            )),
                            value,
                        });
                    }
                } else if let (true, Some(ty)) =
                    (ns_is(&rr, ns::GENERICS), attributes::generic_attr(&name))
                {
                    let attr_name = get_attr_local(&e, b"name");
                    read_generic_attribute(reader, buf, &mut b, attr_name, ty)?;
                } else {
                    skip_element(reader, buf)?;
                }
            }
            Event::End(e) if e.local_name().as_ref() == end_name => break,
            Event::Eof => {
                return Err(Error::invalid(
                    "unexpected end of document inside a bldg:Building/BuildingPart",
                ));
            }
            _ => {}
        }
    }
    if let Some((lod, polys)) = roof_edge
        && !b.has_plain_surfaces(&lod)
    {
        b.add_plain_surfaces(lod, polys);
    }
    Ok(b)
}

/// Read any other mapped object's subtree (positioned after its `Start`,
/// ending at `End(end_name)`) as `class`.
pub fn read_generic_object<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    class: QName,
    id: Option<String>,
    end_name: &[u8],
    depth: usize,
) -> Result<ObjectBuilder> {
    let mut b = ObjectBuilder::new(id, class);
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        match ev {
            Event::Start(e) => {
                let local = e.local_name();
                let name = local.as_ref().to_vec();
                // Geometry properties are in the object's own module
                // namespace (never gml:), so they are matched by local name.
                if let Some(lod) = lod_suffix(&name, b"Solid") {
                    let geom = read_lod_solid(reader, buf, name)?;
                    b.add_solid(lod, geom);
                } else if let Some(lod) = lod_suffix(&name, b"MultiSurface") {
                    let polys = surface_polygons(reader, buf)?;
                    b.add_plain_surfaces(lod, polys);
                } else if let Some(lod) = lod_suffix(&name, b"Geometry") {
                    // `lodNGeometry` may wrap a solid or a surface collection.
                    match read_lod_geometry(reader, buf, name)? {
                        Some(LodGeom::Solid(geom)) => b.add_solid(lod, geom),
                        Some(LodGeom::Surfaces(polys)) => b.add_plain_surfaces(lod, polys),
                        None => {}
                    }
                } else if let (true, Some(ty)) =
                    (ns_is(&rr, ns::GENERICS), attributes::generic_attr(&name))
                {
                    let attr_name = get_attr_local(&e, b"name");
                    read_generic_attribute(reader, buf, &mut b, attr_name, ty)?;
                } else if ns_is(&rr, ns::APPEARANCE) && name == b"appearance" {
                    b.add_appearance(read_appearance(reader, buf, b"appearance")?);
                } else if !ns_is(&rr, NS_GML) && is_semantic_surface_property(&name) {
                    // The object's own boundary surfaces, in the module
                    // namespace the property itself is in.
                    let Some(ns) = ns_of(&rr) else {
                        skip_element(reader, buf)?;
                        continue;
                    };
                    read_bounded_by(reader, buf, &mut b, &ns, &name)?;
                } else if !ns_is(&rr, NS_GML) && is_nesting_property(&name) {
                    let ns = ns_of(&rr).unwrap_or_default();
                    read_child_object(reader, buf, &mut b, &name, &ns, depth)?;
                } else if !ns_is(&rr, NS_GML) {
                    // Any other module property: a text-only leaf is kept as
                    // a string attribute; a structured one is dropped.
                    let uri = ns_of(&rr).unwrap_or_default();
                    if let Some(text) = read_leaf_text(reader, buf, &name)? {
                        b.push_attribute(Attribute {
                            name: AttributeName::Property(qname(&uri, &name)),
                            value: Value::String(text),
                        });
                    }
                } else {
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
    Ok(b)
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

/// The polygons of a surface-collection property, without their xlinks.
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
/// A child the [`NESTED`] table names, in the property's own namespace,
/// becomes a child object of `b`. Anything else only has its polygons
/// registered as `b`'s xlink targets: a parent's `lodNSolid` may compose its
/// faces from them, and they are never geometry of their own, which would
/// double the object.
fn read_child_object<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    b: &mut ObjectBuilder,
    prop_name: &[u8],
    ns: &[u8],
    depth: usize,
) -> Result<()> {
    if depth > MAX_PART_DEPTH {
        return Err(Error::invalid(format!(
            "CityGML features nested deeper than {MAX_PART_DEPTH}"
        )));
    }
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        match ev {
            Event::Start(e) => {
                let local = e.local_name();
                let class = ns_eq(&rr, ns)
                    .then(|| lookup(NESTED, local.as_ref()))
                    .flatten();
                if let Some(class) = class {
                    let end = local.as_ref().to_vec();
                    let id = gml_id(&e);
                    let child = read_generic_object(reader, buf, class, id, &end, depth + 1)?;
                    b.add_child(child);
                } else {
                    b.register(gml::collect_surfaces(reader, buf)?);
                }
            }
            Event::End(e) if e.local_name().as_ref() == prop_name => break,
            Event::Eof => {
                return Err(Error::invalid(
                    "unexpected end of document inside a BuildingInstallation property",
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

/// A `bldg:consistsOfBuildingPart` (positioned after its `Start`): its
/// `bldg:BuildingPart`, read as a part of `b`. An empty or xlink-only property
/// adds nothing.
fn read_consists_of_part<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    b: &mut ObjectBuilder,
    depth: usize,
) -> Result<()> {
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        match ev {
            Event::Start(e) => {
                if ns_is(&rr, ns::BUILDING) && e.local_name().as_ref() == b"BuildingPart" {
                    let id = gml_id(&e);
                    let part = read_abstract_building(reader, buf, id, b"BuildingPart", depth + 1)?;
                    b.add_part(part);
                } else {
                    skip_element(reader, buf)?;
                }
            }
            Event::End(e) if e.local_name().as_ref() == b"consistsOfBuildingPart" => break,
            Event::Eof => {
                return Err(Error::invalid(
                    "unexpected end of document inside <bldg:consistsOfBuildingPart>",
                ));
            }
            _ => {}
        }
    }
    Ok(())
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
/// places the schema puts a city object's sub-features, and so the only
/// places a parent's `lodNSolid` can find the polygons it composes by
/// `xlink:href`. Matched as a closed vocabulary rather than by descending
/// into every unknown element, so an ADE subtree is skipped by bytes rather
/// than parsed for geometry nothing will reference.
fn is_nesting_property(local: &[u8]) -> bool {
    let Ok(name) = std::str::from_utf8(local) else {
        return false;
    };
    name == "boundedBy"
        || name.starts_with("outer")
        || name.starts_with("interior")
        || name.starts_with("consistsOf")
}

/// The geometry a `lodNGeometry` property wraps.
enum LodGeom {
    Solid(SolidGeom),
    Surfaces(Vec<Polygon>),
}

/// Inside a `lodNGeometry` (positioned after its `Start`, ending at
/// `End(end)`): a `gml:Solid` / `CompositeSolid` is a solid, a
/// `gml:MultiSurface` / `CompositeSurface` / `Polygon` a surface list. Points
/// and curves are skipped, so they give `None`.
fn read_lod_geometry<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    end: Vec<u8>,
) -> Result<Option<LodGeom>> {
    let mut solid: Option<SolidGeom> = None;
    let mut polys: Vec<Polygon> = Vec::new();
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        let is_gml = ns_is(&rr, NS_GML);
        match ev {
            Event::Start(e) => {
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
                        polys.extend(surface_polygons(reader, buf)?);
                    }
                    (true, b"Polygon") => {
                        let id = gml_id(&e);
                        let mut poly = gml::read_polygon(reader, buf)?;
                        poly.id = id;
                        polys.push(poly);
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
    Ok(solid
        .map(LodGeom::Solid)
        .or_else(|| (!polys.is_empty()).then_some(LodGeom::Surfaces(polys))))
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
                    "unexpected end of document inside <bldg:lodNSolid>",
                ));
            }
            _ => {}
        }
    }
    geom.ok_or_else(|| Error::invalid("bldg:lodNSolid without gml:Solid or gml:CompositeSolid"))
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
/// `End(end)`): the trimmed text of a text-only leaf, or `None` for an empty
/// one or one with child elements.
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
    if has_child {
        return Ok(None);
    }
    let t = text.trim();
    Ok((!t.is_empty()).then(|| t.to_string()))
}
