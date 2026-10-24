//! Geometry properties both CityGML versions spell alike: `lodN…` names, the
//! GML they wrap, and an `ImplicitGeometry`.

use std::io::BufRead;

use quick_xml::events::Event;
use quick_xml::reader::NsReader;

use super::templates::TemplateIndex;
use super::text::read_leaf_text;
use crate::gml::ObjectBuilder;
use crate::gml::read::{self as gml, SolidGeom};
use crate::model::{Instance, Line, Polygon};
use crate::xml::{NS_GML, get_attr_local, gml_id, member_id, ns_is, skip_element, xml_err};
use crate::{Error, Result};

/// `lodN<suffix>` -> the LoD `"N"`, else `None`.
pub fn lod_suffix(local: &[u8], suffix: &[u8]) -> Option<String> {
    let s = std::str::from_utf8(local).ok()?;
    let suffix = std::str::from_utf8(suffix).ok()?;
    let digits = s.strip_prefix("lod")?.strip_suffix(suffix)?;
    if !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) {
        Some(digits.to_string())
    } else {
        None
    }
}

/// `lodNImplicitRepresentation` -> the LoD `"N"`, else `None`.
pub fn implicit_lod(local: &[u8]) -> Option<String> {
    lod_suffix(local, b"ImplicitRepresentation")
}

/// The geometry a `lodNGeometry` property wraps.
pub enum LodGeom {
    Solid(SolidGeom),
    Surfaces(Vec<Polygon>),
    Lines(Vec<Line>),
    Points(Vec<[f64; 3]>),
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

/// Inside a property holding any GML geometry (a `lodNGeometry`, a relative
/// geometry), positioned after its `Start` and ending at `End(end)`: a
/// `gml:Solid` / `CompositeSolid` is a solid, a `gml:MultiSurface` /
/// `CompositeSurface` / `Polygon` a surface list, a curve or curve aggregate
/// lines, and a `gml:Point` / `MultiPoint` points. Also returns the `gml:id`
/// of the first geometry element.
pub fn read_lod_geometry<R: BufRead>(
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
pub fn add_lod_geometry(b: &mut ObjectBuilder, lod: String, geom: Option<LodGeom>) {
    match geom {
        Some(LodGeom::Solid(geom)) => b.add_solid(lod, geom),
        Some(LodGeom::Surfaces(polys)) => b.add_plain_surfaces(lod, polys),
        Some(LodGeom::Lines(lines)) => b.add_lines(lod, lines),
        Some(LodGeom::Points(points)) => b.add_points(lod, points),
        None => {}
    }
}

/// Inside a `lodNSolid` (ending at `End(end)`): the wrapped `gml:Solid` or
/// `gml:CompositeSolid`.
pub fn read_lod_solid<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    end: &[u8],
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
            Event::End(e) if e.local_name().as_ref() == end => break,
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

/// How a version spells an `ImplicitGeometry`: its namespace, and the name
/// of the property holding the relative geometry (`relativeGMLGeometry` in
/// CityGML 2.0, `relativeGeometry` in 3.0).
#[derive(Debug, Clone, Copy)]
pub struct ImplicitNames<'a> {
    pub namespace: &'a str,
    pub relative_geometry: &'a [u8],
}

/// An `lodNImplicitRepresentation` (positioned after its `Start`, ending at
/// `End(end)`) as an [`Instance`] of one of `templates`. `None` when it
/// names no relative geometry of this document (a `libraryObject` only, or
/// an `xlink:href` to another document or to the `ImplicitGeometry` itself),
/// or has no reference point.
pub fn read_implicit<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    end: &[u8],
    templates: &TemplateIndex,
    names: ImplicitNames<'_>,
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
                let local = local.as_ref();
                if !ns_is(&rr, names.namespace) {
                    skip_element(reader, buf)?;
                } else if local == b"ImplicitGeometry" {
                } else if local == b"transformationMatrix" {
                    let text = read_leaf_text(reader, buf, b"transformationMatrix")?;
                    matrix = parse_matrix(text.as_deref().unwrap_or_default())?;
                } else if local == names.relative_geometry {
                    template = match get_attr_local(&e, b"href") {
                        Some(href) => member_id(&href).and_then(|id| templates.by_id(&id)),
                        None => templates.at(reader.buffer_position()),
                    };
                    skip_element(reader, buf)?;
                } else if local == b"referencePoint" {
                    reference_point = gml::collect_points(reader, buf)?.first().copied();
                } else {
                    skip_element(reader, buf)?;
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
