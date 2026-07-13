//! Streaming GML geometry parsing into float intermediates.
//!
//! Parses `gml:Solid` / `gml:CompositeSurface` / `gml:MultiSurface` /
//! `gml:Polygon` / `gml:LinearRing` with coordinates in either `gml:posList`
//! or per-point `gml:pos`. Each function is called just after its element's
//! `Start` event and consumes through the matching `End`. GML's closing
//! duplicate ring point is dropped (CityJSON rings are not closed).

use std::io::BufRead;

use cityparquet_schema::{CityParquetError, Result};
use quick_xml::events::Event;
use quick_xml::reader::NsReader;

use super::xml::{NS_GML, get_attr, ns_is, read_text, skip_element, xml_err};

pub type Ring = Vec<[f64; 3]>;

/// A CityJSON "surface": an exterior ring plus optional interior (hole) rings.
#[derive(Debug, Clone)]
pub struct Polygon {
    pub exterior: Ring,
    pub interiors: Vec<Ring>,
}

/// A `gml:Solid`: one or more shells (exterior first), each a list of surfaces.
#[derive(Debug, Clone)]
pub struct Solid {
    pub shells: Vec<Vec<Polygon>>,
}

/// Parse a `gml:Solid` (positioned after its `Start`).
pub fn read_solid<R: BufRead>(reader: &mut NsReader<R>, buf: &mut Vec<u8>) -> Result<Solid> {
    let mut exterior: Option<Vec<Polygon>> = None;
    let mut interiors: Vec<Vec<Polygon>> = Vec::new();
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        let gml = ns_is(&rr, NS_GML);
        match ev {
            Event::Start(e) => {
                let local = e.local_name();
                match (gml, local.as_ref()) {
                    (true, b"exterior") => exterior = Some(read_shell(reader, buf, b"exterior")?),
                    (true, b"interior") => interiors.push(read_shell(reader, buf, b"interior")?),
                    _ => skip_element(reader, buf)?,
                }
            }
            Event::End(e) if e.local_name().as_ref() == b"Solid" => break,
            Event::Eof => return Err(eof("Solid")),
            _ => {}
        }
    }
    let mut shells = Vec::new();
    if let Some(ext) = exterior {
        shells.push(ext);
    }
    shells.extend(interiors);
    Ok(Solid { shells })
}

/// A shell (the `gml:exterior`/`gml:interior` of a Solid), which wraps a
/// `gml:CompositeSurface`.
fn read_shell<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    end: &[u8],
) -> Result<Vec<Polygon>> {
    let mut polys = Vec::new();
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        let gml = ns_is(&rr, NS_GML);
        match ev {
            Event::Start(e) => {
                let local = e.local_name();
                match (gml, local.as_ref()) {
                    (true, b"CompositeSurface") | (true, b"Surface") => {
                        let end = local.as_ref().to_vec();
                        polys = read_surface_collection(reader, buf, end)?
                    }
                    _ => skip_element(reader, buf)?,
                }
            }
            Event::End(e) if e.local_name().as_ref() == end => break,
            Event::Eof => return Err(eof("shell")),
            _ => {}
        }
    }
    Ok(polys)
}

/// A `gml:CompositeSurface` / `gml:MultiSurface` (positioned after its `Start`):
/// a sequence of `gml:surfaceMember`s.
pub fn read_surface_collection<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    end: Vec<u8>,
) -> Result<Vec<Polygon>> {
    let mut polys = Vec::new();
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        let gml = ns_is(&rr, NS_GML);
        match ev {
            Event::Start(e) => {
                let local = e.local_name();
                if gml && local.as_ref() == b"surfaceMember" {
                    polys.append(&mut read_surface_member(reader, buf)?);
                } else {
                    skip_element(reader, buf)?;
                }
            }
            Event::End(e) if e.local_name().as_ref() == end.as_slice() => break,
            Event::Eof => return Err(eof("surface collection")),
            _ => {}
        }
    }
    Ok(polys)
}

/// A `gml:surfaceMember`: a `gml:Polygon`, or a nested surface collection that
/// is flattened into polygons.
fn read_surface_member<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
) -> Result<Vec<Polygon>> {
    let mut polys = Vec::new();
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        let gml = ns_is(&rr, NS_GML);
        match ev {
            Event::Start(e) => {
                let local = e.local_name();
                match (gml, local.as_ref()) {
                    (true, b"Polygon") => polys.push(read_polygon(reader, buf)?),
                    (true, b"CompositeSurface") | (true, b"MultiSurface") => {
                        let end = local.as_ref().to_vec();
                        polys.append(&mut read_surface_collection(reader, buf, end)?)
                    }
                    _ => skip_element(reader, buf)?,
                }
            }
            Event::End(e) if e.local_name().as_ref() == b"surfaceMember" => break,
            Event::Eof => return Err(eof("surfaceMember")),
            _ => {}
        }
    }
    Ok(polys)
}

/// A `gml:Polygon` (positioned after its `Start`): exterior ring + holes.
pub fn read_polygon<R: BufRead>(reader: &mut NsReader<R>, buf: &mut Vec<u8>) -> Result<Polygon> {
    let mut exterior: Option<Ring> = None;
    let mut interiors: Vec<Ring> = Vec::new();
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        let gml = ns_is(&rr, NS_GML);
        match ev {
            Event::Start(e) => {
                let local = e.local_name();
                match (gml, local.as_ref()) {
                    (true, b"exterior") => {
                        exterior = Some(read_ring_container(reader, buf, b"exterior")?)
                    }
                    (true, b"interior") => {
                        interiors.push(read_ring_container(reader, buf, b"interior")?)
                    }
                    _ => skip_element(reader, buf)?,
                }
            }
            Event::End(e) if e.local_name().as_ref() == b"Polygon" => break,
            Event::Eof => return Err(eof("Polygon")),
            _ => {}
        }
    }
    let exterior = exterior
        .ok_or_else(|| CityParquetError::Schema("gml:Polygon without exterior ring".to_string()))?;
    Ok(Polygon {
        exterior,
        interiors,
    })
}

/// The `gml:exterior`/`gml:interior` of a Polygon, wrapping a `gml:LinearRing`.
fn read_ring_container<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    end: &[u8],
) -> Result<Ring> {
    let mut ring: Option<Ring> = None;
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        let gml = ns_is(&rr, NS_GML);
        match ev {
            Event::Start(e) => {
                let local = e.local_name();
                if gml && local.as_ref() == b"LinearRing" {
                    ring = Some(read_linear_ring(reader, buf)?);
                } else {
                    skip_element(reader, buf)?;
                }
            }
            Event::End(e) if e.local_name().as_ref() == end => break,
            Event::Eof => return Err(eof("ring container")),
            _ => {}
        }
    }
    ring.ok_or_else(|| CityParquetError::Schema("gml ring without LinearRing".to_string()))
}

/// A `gml:LinearRing`: points from `gml:posList` or per-point `gml:pos`. Drops
/// the GML closing duplicate point.
fn read_linear_ring<R: BufRead>(reader: &mut NsReader<R>, buf: &mut Vec<u8>) -> Result<Ring> {
    let mut ring: Ring = Vec::new();
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        let gml = ns_is(&rr, NS_GML);
        match ev {
            Event::Start(e) => {
                let local = e.local_name();
                match (gml, local.as_ref()) {
                    (true, b"pos") => {
                        let dim = srs_dimension(&e).unwrap_or(3);
                        let text = read_text(reader, buf)?;
                        ring.extend(parse_coords(&text, dim)?);
                    }
                    (true, b"posList") => {
                        let dim = srs_dimension(&e).unwrap_or(3);
                        let text = read_text(reader, buf)?;
                        ring.extend(parse_coords(&text, dim)?);
                    }
                    _ => skip_element(reader, buf)?,
                }
            }
            Event::End(e) if e.local_name().as_ref() == b"LinearRing" => break,
            Event::Eof => return Err(eof("LinearRing")),
            _ => {}
        }
    }
    if ring.len() >= 2 && ring.first() == ring.last() {
        ring.pop();
    }
    Ok(ring)
}

fn srs_dimension(e: &quick_xml::events::BytesStart) -> Option<usize> {
    get_attr(e, b"srsDimension").and_then(|s| s.trim().parse().ok())
}

/// Parse a whitespace-separated coordinate string into `[x, y, z]` points,
/// `stride` values per point (z defaults to 0 when 2D).
fn parse_coords(text: &str, stride: usize) -> Result<Vec<[f64; 3]>> {
    if stride < 2 {
        return Err(CityParquetError::Schema(format!(
            "invalid srsDimension {stride} in CityGML geometry"
        )));
    }
    let nums: std::result::Result<Vec<f64>, _> =
        text.split_whitespace().map(|t| t.parse::<f64>()).collect();
    let nums = nums.map_err(|e| {
        CityParquetError::Schema(format!("invalid coordinate in CityGML posList/pos: {e}"))
    })?;
    if nums.is_empty() || nums.len() % stride != 0 {
        return Err(CityParquetError::Schema(format!(
            "CityGML coordinate list length {} not a multiple of dimension {stride}",
            nums.len()
        )));
    }
    Ok(nums
        .chunks(stride)
        .map(|c| [c[0], c[1], if stride >= 3 { c[2] } else { 0.0 }])
        .collect())
}

fn eof(ctx: &str) -> CityParquetError {
    CityParquetError::Schema(format!("unexpected end of CityGML document inside <{ctx}>"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_coords_pos_and_poslist() {
        let pts = parse_coords("0 0 0 100 0 0", 3).unwrap();
        assert_eq!(pts, vec![[0.0, 0.0, 0.0], [100.0, 0.0, 0.0]]);
        assert!(parse_coords("0 0 0 1", 3).is_err()); // not a multiple of 3
        let two_d = parse_coords("1 2 3 4", 2).unwrap();
        assert_eq!(two_d, vec![[1.0, 2.0, 0.0], [3.0, 4.0, 0.0]]);
    }
}
