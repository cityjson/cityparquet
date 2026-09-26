//! The CityGML 2.0 Relief module: `dem:ReliefFeature` → CityJSON `TINRelief`.
//!
//! A `dem:ReliefFeature` aggregates one or more `dem:reliefComponent`s, each a
//! `dem:TINRelief`, `dem:RasterRelief`, `dem:MassPointRelief` or
//! `dem:BreaklineRelief`. CityJSON keeps only the TIN, as a 1st-level
//! `TINRelief` whose geometry is a `CompositeSurface` of triangles (CityJSON
//! 2.0 §2.11), so each TIN component becomes one `TINRelief` and the
//! `ReliefFeature` itself — a container with no geometry of its own — does
//! not survive as an object. The other three component kinds have no CityJSON
//! counterpart; they are reported back by name so the caller can tally them
//! as skipped members.
//!
//! A TIN's triangles are the `gml:Triangle` patches of its `dem:tin`, which
//! holds a `gml:TriangulatedSurface` or a `gml:Tin`. A `gml:Tin` given only by
//! control points (no patches) yields a `TINRelief` without geometry: the
//! reader does not triangulate.

use std::io::BufRead;

use cityparquet_schema::{CityParquetError, Result};
use quick_xml::events::Event;
use quick_xml::reader::NsReader;
use serde_json::Value;

use super::attributes;
use super::building::RawBuilding;
use super::geometry::{self, Polygon};
use super::xml::{
    NS_DEM, NS_GEN, NS_GML, get_attr_local, gml_id, ns_is, read_text, skip_element, xml_err,
};

/// What one `dem:ReliefFeature` yields.
pub struct ReadRelief {
    /// One `TINRelief` per TIN component, in document order.
    pub tins: Vec<RawBuilding>,
    /// The element names, as the document spells them, of the components that
    /// have no CityJSON counterpart (`dem:RasterRelief`, …).
    pub unmapped_components: Vec<String>,
}

/// Read a `dem:ReliefFeature` subtree, positioned after its `Start`; consumes
/// through its matching `End`. `id` is the feature's own `gml:id`.
pub fn read_relief_feature<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    id: Option<String>,
) -> Result<ReadRelief> {
    let mut feature_lod: Option<String> = None;
    let mut feature_attributes = serde_json::Map::new();
    let mut components: Vec<TinComponent> = Vec::new();
    let mut unmapped_components = Vec::new();
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        match ev {
            Event::Start(e) => {
                let local = e.local_name().as_ref().to_vec();
                if ns_is(&rr, NS_DEM) && local == b"lod" {
                    feature_lod = non_empty(read_text(reader, buf)?);
                } else if ns_is(&rr, NS_DEM) && local == b"reliefComponent" {
                    read_relief_component(reader, buf, &mut components, &mut unmapped_components)?;
                } else if let (true, Some(ty)) =
                    (ns_is(&rr, NS_GEN), attributes::generic_attr(&local))
                {
                    let name = get_attr_local(&e, b"name");
                    if let Some((k, v)) = attributes::read_generic_attribute(reader, buf, name, ty)?
                    {
                        attributes::accumulate(&mut feature_attributes, k, v);
                    }
                } else {
                    skip_element(reader, buf)?;
                }
            }
            Event::End(e) if e.local_name().as_ref() == b"ReliefFeature" => break,
            Event::Eof => return Err(eof("dem:ReliefFeature")),
            _ => {}
        }
    }

    // A lone TIN without an id of its own takes the feature's: it is the
    // feature's only representation in CityJSON.
    let lone = components.len() == 1;
    let tins = components
        .into_iter()
        .map(|c| {
            let id = c.id.or_else(|| if lone { id.clone() } else { None });
            // The feature's generic attributes describe every component; a
            // component's own attribute of the same name is more specific.
            let mut attrs = feature_attributes.clone();
            attrs.extend(c.attributes);
            RawBuilding::tin_relief(
                id,
                c.lod.or_else(|| feature_lod.clone()),
                c.triangles,
                attrs,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(ReadRelief {
        tins,
        unmapped_components,
    })
}

struct TinComponent {
    id: Option<String>,
    lod: Option<String>,
    triangles: Vec<Polygon>,
    attributes: serde_json::Map<String, Value>,
}

/// A `dem:reliefComponent` property, positioned after its `Start`.
fn read_relief_component<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    components: &mut Vec<TinComponent>,
    unmapped: &mut Vec<String>,
) -> Result<()> {
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        match ev {
            Event::Start(e) => {
                if ns_is(&rr, NS_DEM) && e.local_name().as_ref() == b"TINRelief" {
                    let id = gml_id(&e);
                    components.push(read_tin_relief(reader, buf, id)?);
                } else {
                    unmapped.push(String::from_utf8_lossy(e.name().as_ref()).into_owned());
                    skip_element(reader, buf)?;
                }
            }
            Event::End(e) if e.local_name().as_ref() == b"reliefComponent" => return Ok(()),
            Event::Eof => return Err(eof("dem:reliefComponent")),
            _ => {}
        }
    }
}

/// A `dem:TINRelief`, positioned after its `Start`.
fn read_tin_relief<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    id: Option<String>,
) -> Result<TinComponent> {
    let mut tin = TinComponent {
        id,
        lod: None,
        triangles: Vec::new(),
        attributes: serde_json::Map::new(),
    };
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        match ev {
            Event::Start(e) => {
                let local = e.local_name().as_ref().to_vec();
                if ns_is(&rr, NS_DEM) && local == b"lod" {
                    tin.lod = non_empty(read_text(reader, buf)?);
                } else if ns_is(&rr, NS_DEM) && local == b"tin" {
                    tin.triangles.extend(read_tin_property(reader, buf)?);
                } else if let (true, Some(ty)) =
                    (ns_is(&rr, NS_GEN), attributes::generic_attr(&local))
                {
                    let name = get_attr_local(&e, b"name");
                    if let Some((k, v)) = attributes::read_generic_attribute(reader, buf, name, ty)?
                    {
                        attributes::accumulate(&mut tin.attributes, k, v);
                    }
                } else {
                    // `dem:extent` (a 2D validity polygon) and anything else:
                    // not geometry of the TIN.
                    skip_element(reader, buf)?;
                }
            }
            Event::End(e) if e.local_name().as_ref() == b"TINRelief" => return Ok(tin),
            Event::Eof => return Err(eof("dem:TINRelief")),
            _ => {}
        }
    }
}

/// A `dem:tin` property, positioned after its `Start`: the triangles of the
/// `gml:TriangulatedSurface` or `gml:Tin` it wraps.
fn read_tin_property<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
) -> Result<Vec<Polygon>> {
    let mut triangles = Vec::new();
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        match ev {
            Event::Start(e) => {
                let local = e.local_name();
                if ns_is(&rr, NS_GML) && matches!(local.as_ref(), b"TriangulatedSurface" | b"Tin") {
                    triangles.extend(geometry::collect_triangles(reader, buf)?);
                } else {
                    skip_element(reader, buf)?;
                }
            }
            Event::End(e) if e.local_name().as_ref() == b"tin" => return Ok(triangles),
            Event::Eof => return Err(eof("dem:tin")),
            _ => {}
        }
    }
}

fn non_empty(text: String) -> Option<String> {
    let t = text.trim();
    (!t.is_empty()).then(|| t.to_string())
}

fn eof(ctx: &str) -> CityParquetError {
    CityParquetError::Schema(format!("unexpected end of CityGML document inside <{ctx}>"))
}
