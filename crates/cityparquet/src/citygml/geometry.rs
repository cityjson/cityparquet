//! Streaming GML geometry parsing.
//!
//! Two concerns live here:
//! - Parsing a `gml:Solid` / `gml:CompositeSolid` into an *unresolved* tree of
//!   [`SurfaceRef`]s: each surface member is either an `xlink:href` to a polygon
//!   defined elsewhere in the same building, or an inline `gml:Polygon`, with an
//!   optional orientation flip (`gml:OrientableSurface orientation="-"`).
//!   Resolution against the building's polygon registry happens in `building`.
//! - Harvesting inline `gml:Polygon`s (with their `gml:id`) from a subtree
//!   ([`collect_polygons`]), used for `boundedBy` semantic surfaces and the
//!   standalone `lodNMultiSurface` polygons the solid references.
//!
//! Coordinates come from `gml:posList` or per-point `gml:pos`; the GML closing
//! duplicate ring point is dropped (CityJSON rings are not closed). The reader
//! runs with `expand_empty_elements`, so a self-closing `<... xlink:href=.../>`
//! arrives as a `Start`+`End` pair.

use std::io::BufRead;

use cityparquet_schema::{CityParquetError, Result};
use quick_xml::events::Event;
use quick_xml::reader::NsReader;

use super::xml::{
    NS_GML, get_attr, gml_id, ns_is, read_text, skip_element, xlink_fragment, xml_err,
};

pub type Ring = Vec<[f64; 3]>;

/// A CityJSON "surface": an exterior ring plus optional interior (hole) rings.
#[derive(Debug, Clone)]
pub struct Polygon {
    /// The polygon's `gml:id`, if any — the id an `app:target="#id"` appearance
    /// reference (or an `xlink:href`) resolves to. `None` for an anonymous inline
    /// polygon (which is therefore untargetable by appearance).
    pub id: Option<String>,
    pub exterior: Ring,
    pub interiors: Vec<Ring>,
    /// The `gml:id` of each ring, parallel to `[exterior, interiors...]` — the id
    /// an `app:textureCoordinates ring="#id"` reference resolves to. `None` for an
    /// anonymous ring.
    pub ring_ids: Vec<Option<String>>,
}

/// One surface of a solid shell, before xlink resolution.
#[derive(Debug, Clone)]
pub struct SurfaceRef {
    /// `true` when the surface's orientation is reversed (its rings must be
    /// wound backwards so the outward normal flips — CityJSON has no
    /// orientation flag).
    pub reverse: bool,
    pub target: RefTarget,
}

#[derive(Debug, Clone)]
pub enum RefTarget {
    /// A `#fragment` reference to a polygon defined elsewhere in the building.
    Xlink(String),
    /// A polygon defined inline in the solid.
    Inline(Polygon),
}

/// A shell: an ordered list of surface references.
pub type Shell = Vec<SurfaceRef>;

/// A `gml:Solid`: exterior shell first, then interior shells.
#[derive(Debug, Clone)]
pub struct RawSolid {
    pub shells: Vec<Shell>,
}

/// The solid geometry of a `bldg:lodNSolid`.
#[derive(Debug, Clone)]
pub enum SolidGeom {
    Solid(RawSolid),
    Composite(Vec<RawSolid>),
}

/// Parse a `gml:CompositeSolid` (positioned after its `Start`) into its solids.
pub fn read_composite_solid<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
) -> Result<Vec<RawSolid>> {
    let mut solids = Vec::new();
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        let gml = ns_is(&rr, NS_GML);
        match ev {
            Event::Start(e) => {
                let local = e.local_name();
                if gml && local.as_ref() == b"solidMember" {
                    solids.push(read_solid_member(reader, buf)?);
                } else {
                    skip_element(reader, buf)?;
                }
            }
            Event::End(e) if e.local_name().as_ref() == b"CompositeSolid" => break,
            Event::Eof => return Err(eof("CompositeSolid")),
            _ => {}
        }
    }
    Ok(solids)
}

/// A `gml:solidMember`: wraps one `gml:Solid`.
fn read_solid_member<R: BufRead>(reader: &mut NsReader<R>, buf: &mut Vec<u8>) -> Result<RawSolid> {
    let mut solid: Option<RawSolid> = None;
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        let gml = ns_is(&rr, NS_GML);
        match ev {
            Event::Start(e) => {
                let local = e.local_name();
                if gml && local.as_ref() == b"Solid" {
                    solid = Some(read_solid(reader, buf)?);
                } else {
                    skip_element(reader, buf)?;
                }
            }
            Event::End(e) if e.local_name().as_ref() == b"solidMember" => break,
            Event::Eof => return Err(eof("solidMember")),
            _ => {}
        }
    }
    solid.ok_or_else(|| CityParquetError::Schema("gml:solidMember without gml:Solid".to_string()))
}

/// Parse a `gml:Solid` (positioned after its `Start`).
pub fn read_solid<R: BufRead>(reader: &mut NsReader<R>, buf: &mut Vec<u8>) -> Result<RawSolid> {
    let mut exterior: Option<Shell> = None;
    let mut interiors: Vec<Shell> = Vec::new();
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
    Ok(RawSolid { shells })
}

/// A shell wrapper (`gml:exterior`/`gml:interior`), holding a
/// `gml:CompositeSurface` (or `gml:Surface`) of surface members.
fn read_shell<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    end: &[u8],
) -> Result<Shell> {
    let mut refs = Vec::new();
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
                        refs = read_surface_refs(reader, buf, end)?
                    }
                    _ => skip_element(reader, buf)?,
                }
            }
            Event::End(e) if e.local_name().as_ref() == end => break,
            Event::Eof => return Err(eof("shell")),
            _ => {}
        }
    }
    Ok(refs)
}

/// The `gml:surfaceMember`s of a `CompositeSurface`/`MultiSurface`, each a
/// [`SurfaceRef`].
fn read_surface_refs<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    end: Vec<u8>,
) -> Result<Vec<SurfaceRef>> {
    let mut refs = Vec::new();
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        let gml = ns_is(&rr, NS_GML);
        match ev {
            Event::Start(e) => {
                let local = e.local_name();
                if gml && local.as_ref() == b"surfaceMember" {
                    // An xlink on the surfaceMember itself references a polygon
                    // defined elsewhere.
                    if let Some(id) = xlink_fragment(&e)? {
                        refs.push(SurfaceRef {
                            reverse: false,
                            target: RefTarget::Xlink(id),
                        });
                        skip_element(reader, buf)?;
                    } else {
                        refs.append(&mut read_surface_member_children(reader, buf)?);
                    }
                } else {
                    skip_element(reader, buf)?;
                }
            }
            Event::End(e) if e.local_name().as_ref() == end.as_slice() => break,
            Event::Eof => return Err(eof("surface collection")),
            _ => {}
        }
    }
    Ok(refs)
}

/// The inline content of a non-xlink `gml:surfaceMember`: an inline
/// `gml:Polygon`, a `gml:OrientableSurface`, or a nested surface collection.
fn read_surface_member_children<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
) -> Result<Vec<SurfaceRef>> {
    let mut refs = Vec::new();
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        let gml = ns_is(&rr, NS_GML);
        match ev {
            Event::Start(e) => {
                let local = e.local_name();
                match (gml, local.as_ref()) {
                    (true, b"Polygon") => {
                        let id = gml_id(&e);
                        let mut poly = read_polygon(reader, buf)?;
                        poly.id = id;
                        refs.push(SurfaceRef {
                            reverse: false,
                            target: RefTarget::Inline(poly),
                        });
                    }
                    (true, b"OrientableSurface") => {
                        let reverse = orientation_reversed(&e);
                        refs.push(read_orientable_surface(reader, buf, reverse)?);
                    }
                    (true, b"CompositeSurface") | (true, b"MultiSurface") => {
                        let end = local.as_ref().to_vec();
                        refs.append(&mut read_surface_refs(reader, buf, end)?);
                    }
                    _ => skip_element(reader, buf)?,
                }
            }
            Event::End(e) if e.local_name().as_ref() == b"surfaceMember" => break,
            Event::Eof => return Err(eof("surfaceMember")),
            _ => {}
        }
    }
    Ok(refs)
}

/// A `gml:OrientableSurface` (positioned after its `Start`): a single
/// `gml:baseSurface` whose orientation is flipped when `reverse` is set.
/// Nested orientable surfaces XOR their flips.
fn read_orientable_surface<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    reverse: bool,
) -> Result<SurfaceRef> {
    let mut result: Option<SurfaceRef> = None;
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        let gml = ns_is(&rr, NS_GML);
        match ev {
            Event::Start(e) => {
                let local = e.local_name();
                if gml && local.as_ref() == b"baseSurface" {
                    if let Some(id) = xlink_fragment(&e)? {
                        result = Some(SurfaceRef {
                            reverse,
                            target: RefTarget::Xlink(id),
                        });
                        skip_element(reader, buf)?;
                    } else {
                        let inner = read_base_surface_inline(reader, buf)?;
                        result = Some(SurfaceRef {
                            reverse: reverse ^ inner.reverse,
                            target: inner.target,
                        });
                    }
                } else {
                    skip_element(reader, buf)?;
                }
            }
            Event::End(e) if e.local_name().as_ref() == b"OrientableSurface" => break,
            Event::Eof => return Err(eof("OrientableSurface")),
            _ => {}
        }
    }
    result
        .ok_or_else(|| CityParquetError::Schema("gml:OrientableSurface without baseSurface".into()))
}

/// The inline content of a `gml:baseSurface`: a `gml:Polygon` or a nested
/// `gml:OrientableSurface`.
fn read_base_surface_inline<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
) -> Result<SurfaceRef> {
    let mut result: Option<SurfaceRef> = None;
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        let gml = ns_is(&rr, NS_GML);
        match ev {
            Event::Start(e) => {
                let local = e.local_name();
                match (gml, local.as_ref()) {
                    (true, b"Polygon") => {
                        let id = gml_id(&e);
                        let mut poly = read_polygon(reader, buf)?;
                        poly.id = id;
                        result = Some(SurfaceRef {
                            reverse: false,
                            target: RefTarget::Inline(poly),
                        })
                    }
                    (true, b"OrientableSurface") => {
                        let reverse = orientation_reversed(&e);
                        result = Some(read_orientable_surface(reader, buf, reverse)?);
                    }
                    _ => skip_element(reader, buf)?,
                }
            }
            Event::End(e) if e.local_name().as_ref() == b"baseSurface" => break,
            Event::Eof => return Err(eof("baseSurface")),
            _ => {}
        }
    }
    result.ok_or_else(|| CityParquetError::Schema("gml:baseSurface without a surface".into()))
}

fn orientation_reversed(e: &quick_xml::events::BytesStart) -> bool {
    get_attr(e, b"orientation").as_deref() == Some("-")
}

/// Harvest every `gml:Polygon` (with its `gml:id`, if any) inside the current
/// element's subtree. Call right after the subtree's `Start`; consumes through
/// its matching `End`.
pub fn collect_polygons<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
) -> Result<Vec<(Option<String>, Polygon)>> {
    let mut out = Vec::new();
    let mut depth = 1usize;
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        let gml = ns_is(&rr, NS_GML);
        match ev {
            Event::Start(e) => {
                if gml && e.local_name().as_ref() == b"Polygon" {
                    // `read_polygon` consumes the whole Polygon subtree, so the
                    // depth is unchanged by this branch.
                    let id = gml_id(&e);
                    let mut poly = read_polygon(reader, buf)?;
                    poly.id = id.clone();
                    out.push((id, poly));
                } else {
                    depth += 1;
                }
            }
            Event::End(_) => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            Event::Eof => return Err(eof("polygon collection")),
            _ => {}
        }
    }
    Ok(out)
}

/// A `gml:Polygon` (positioned after its `Start`): exterior ring + holes.
pub fn read_polygon<R: BufRead>(reader: &mut NsReader<R>, buf: &mut Vec<u8>) -> Result<Polygon> {
    let mut exterior: Option<(Ring, Option<String>)> = None;
    let mut interiors: Vec<(Ring, Option<String>)> = Vec::new();
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
    let (exterior, ext_id) = exterior
        .ok_or_else(|| CityParquetError::Schema("gml:Polygon without exterior ring".to_string()))?;
    let mut ring_ids = Vec::with_capacity(1 + interiors.len());
    ring_ids.push(ext_id);
    let interior_rings = interiors
        .into_iter()
        .map(|(ring, id)| {
            ring_ids.push(id);
            ring
        })
        .collect();
    Ok(Polygon {
        id: None,
        exterior,
        interiors: interior_rings,
        ring_ids,
    })
}

/// The `gml:exterior`/`gml:interior` of a Polygon, wrapping a `gml:LinearRing`.
/// Returns the ring coords plus the `gml:LinearRing`'s `gml:id` (for texture
/// coordinate targeting), if any.
fn read_ring_container<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    end: &[u8],
) -> Result<(Ring, Option<String>)> {
    let mut ring: Option<(Ring, Option<String>)> = None;
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        let gml = ns_is(&rr, NS_GML);
        match ev {
            Event::Start(e) => {
                let local = e.local_name();
                if gml && local.as_ref() == b"LinearRing" {
                    let id = gml_id(&e);
                    ring = Some((read_linear_ring(reader, buf)?, id));
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
                    (true, b"pos") | (true, b"posList") => {
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
