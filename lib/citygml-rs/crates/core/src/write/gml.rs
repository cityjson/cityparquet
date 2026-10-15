//! GML geometry, in GML 3.1.1 or 3.2 as the [`XmlWriter`] says.
//!
//! The two spell geometry alike but for a solid's shells (a
//! `gml:CompositeSurface` in 3.1.1, a `gml:Shell` in 3.2) and a
//! `gml:TriangulatedSurface`'s triangles (`gml:trianglePatches`,
//! `gml:patches`). Every ring is written closed (its first point repeated) and every
//! coordinate as `gml:posList srsDimension="3"`. A face reached through a
//! reversed `gml:OrientableSurface` is written through one again. A face
//! another element defines is written as a reference to its polygon's
//! `gml:id`.

use std::io::Write;

use super::{XmlWriter, numbers};
use crate::model::{Face, Line, Polygon, Ring, Shape};
use crate::{Error, Result};

/// GML 3.2's namespace URI.
pub const NS_GML32: &str = "http://www.opengis.net/gml/3.2";

/// The GML a document's geometry is written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Version {
    /// GML 3.1.1, CityGML 2.0's.
    V3_1,
    /// GML 3.2, CityGML 3.0's.
    V3_2,
}

/// How a face goes where a surface member goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Member {
    /// The polygon itself.
    Inline,
    /// A reference to its polygon's `gml:id`, the polygon being defined
    /// elsewhere.
    Reference,
}

/// How each face of a geometry is written, asked in the order
/// [`Shape::faces`] gives them.
pub type How<'h> = &'h mut dyn FnMut(&Face) -> Member;

/// A `gml:Polygon`, with its `gml:id` and its rings' when it has them, and
/// `attributes` besides.
pub fn polygon<W: Write>(
    x: &mut XmlWriter<W>,
    p: &Polygon,
    attributes: &[(&str, &str)],
) -> Result<()> {
    let mut attrs: Vec<(&str, &str)> = Vec::new();
    if let Some(id) = &p.id {
        attrs.push(("gml:id", id));
    }
    attrs.extend_from_slice(attributes);
    x.open("gml:Polygon", &attrs)?;
    ring(x, "gml:exterior", &p.exterior, p.ring_ids.first())?;
    for (i, hole) in p.interiors.iter().enumerate() {
        ring(x, "gml:interior", hole, p.ring_ids.get(i + 1))?;
    }
    x.close()
}

fn ring<W: Write>(
    x: &mut XmlWriter<W>,
    property: &str,
    points: &Ring,
    id: Option<&Option<String>>,
) -> Result<()> {
    x.open(property, &[])?;
    match id.and_then(Option::as_deref) {
        Some(id) => x.open("gml:LinearRing", &[("gml:id", id)])?,
        None => x.open("gml:LinearRing", &[])?,
    }
    let closed = points.iter().chain(points.first());
    x.leaf(
        "gml:posList",
        &[("srsDimension", "3")],
        &numbers(closed.flat_map(|p| p.iter().copied())),
    )?;
    x.close()?;
    x.close()
}

/// A face as the content of a surface member property (`gml:surfaceMember`,
/// `gml:baseSurface`'s holder): the polygon, or a reference to it, through
/// a reversed `gml:OrientableSurface` when the face is reversed. `element`
/// names the property.
pub fn member<W: Write>(
    x: &mut XmlWriter<W>,
    element: &str,
    face: &Face,
    how: Member,
) -> Result<()> {
    let href = match how {
        Member::Inline => None,
        Member::Reference => match &face.polygon.id {
            Some(id) => Some(format!("#{id}")),
            None => return Err(Error::invalid("a reference to a polygon with no gml:id")),
        },
    };
    match (face.reversed, &href) {
        (false, None) => {
            x.open(element, &[])?;
            polygon(x, &face.polygon, &[])?;
            x.close()
        }
        (false, Some(href)) => x.empty(element, &[("xlink:href", href)]),
        (true, href) => {
            x.open(element, &[])?;
            x.open("gml:OrientableSurface", &[("orientation", "-")])?;
            match href {
                None => {
                    x.open("gml:baseSurface", &[])?;
                    polygon(x, &face.polygon, &[])?;
                    x.close()?;
                }
                Some(href) => x.empty("gml:baseSurface", &[("xlink:href", href)])?,
            }
            x.close()?;
            x.close()
        }
    }
}

/// A face as a surface element itself: its polygon, or a reversed
/// `gml:OrientableSurface` of it, with `attributes`.
pub fn surface<W: Write>(
    x: &mut XmlWriter<W>,
    face: &Face,
    attributes: &[(&str, &str)],
) -> Result<()> {
    if !face.reversed {
        return polygon(x, &face.polygon, attributes);
    }
    let mut attrs = vec![("orientation", "-")];
    attrs.extend_from_slice(attributes);
    x.open("gml:OrientableSurface", &attrs)?;
    x.open("gml:baseSurface", &[])?;
    polygon(x, &face.polygon, &[])?;
    x.close()?;
    x.close()
}

/// Faces as a surface aggregate element (`gml:MultiSurface`,
/// `gml:CompositeSurface`) with `attributes`, each face as `how` says.
pub fn surfaces<'f, W: Write>(
    x: &mut XmlWriter<W>,
    element: &str,
    attributes: &[(&str, &str)],
    faces: impl IntoIterator<Item = &'f Face>,
    how: How<'_>,
) -> Result<()> {
    x.open(element, attributes)?;
    for face in faces {
        member(x, "gml:surfaceMember", face, how(face))?;
    }
    x.close()
}

/// A `gml:Solid` of `shells`, the first its exterior.
pub fn solid<W: Write>(
    x: &mut XmlWriter<W>,
    attributes: &[(&str, &str)],
    shells: &[Vec<Face>],
    how: How<'_>,
) -> Result<()> {
    x.open("gml:Solid", attributes)?;
    for (i, shell) in shells.iter().enumerate() {
        x.open(
            if i == 0 {
                "gml:exterior"
            } else {
                "gml:interior"
            },
            &[],
        )?;
        let shell_element = match x.gml {
            Version::V3_1 => "gml:CompositeSurface",
            Version::V3_2 => "gml:Shell",
        };
        surfaces(x, shell_element, &[], shell, &mut *how)?;
        x.close()?;
    }
    x.close()
}

/// A shape as its GML geometry element, with `attributes` on that element.
/// `how` says how each face is written. An [`Shape::Instance`] is not GML
/// geometry, and is the caller's to write.
pub fn shape<W: Write>(
    x: &mut XmlWriter<W>,
    shape: &Shape,
    attributes: &[(&str, &str)],
    how: How<'_>,
) -> Result<()> {
    match shape {
        Shape::MultiPoint(points) => {
            x.open("gml:MultiPoint", attributes)?;
            for p in points {
                x.open("gml:pointMember", &[])?;
                point(x, p, &[])?;
                x.close()?;
            }
            x.close()
        }
        Shape::MultiLineString(lines) => {
            x.open("gml:MultiCurve", attributes)?;
            for l in lines {
                x.open("gml:curveMember", &[])?;
                line(x, l)?;
                x.close()?;
            }
            x.close()
        }
        Shape::MultiSurface(faces) => surfaces(x, "gml:MultiSurface", attributes, faces, how),
        Shape::CompositeSurface(faces) => {
            surfaces(x, "gml:CompositeSurface", attributes, faces, how)
        }
        Shape::Solid(shells) => solid(x, attributes, shells, how),
        Shape::CompositeSolid(solids) | Shape::MultiSolid(solids) => {
            let element = match shape {
                Shape::CompositeSolid(_) => "gml:CompositeSolid",
                _ => "gml:MultiSolid",
            };
            x.open(element, attributes)?;
            for shells in solids {
                x.open("gml:solidMember", &[])?;
                solid(x, &[], shells, &mut *how)?;
                x.close()?;
            }
            x.close()
        }
        Shape::Instance(_) => Err(Error::invalid("an implicit geometry is not GML geometry")),
    }
}

/// A `gml:Point`.
pub fn point<W: Write>(
    x: &mut XmlWriter<W>,
    p: &[f64; 3],
    attributes: &[(&str, &str)],
) -> Result<()> {
    x.open("gml:Point", attributes)?;
    x.leaf(
        "gml:pos",
        &[("srsDimension", "3")],
        &numbers(p.iter().copied()),
    )?;
    x.close()
}

/// A `gml:LineString`.
pub fn line<W: Write>(x: &mut XmlWriter<W>, l: &Line) -> Result<()> {
    x.open("gml:LineString", &[])?;
    x.leaf(
        "gml:posList",
        &[("srsDimension", "3")],
        &numbers(l.iter().flat_map(|p| p.iter().copied())),
    )?;
    x.close()
}

/// Whether `face` is a triangle without holes, as a
/// `gml:TriangulatedSurface` holds them.
pub fn is_triangle(face: &Face) -> bool {
    face.polygon.exterior.len() == 3 && face.polygon.interiors.is_empty() && !face.reversed
}

/// Faces that are all triangles ([`is_triangle`]) as a
/// `gml:TriangulatedSurface` with `attributes`, or `false`, having written
/// nothing, when one is not.
pub fn triangulated<W: Write>(
    x: &mut XmlWriter<W>,
    faces: &[&Face],
    attributes: &[(&str, &str)],
) -> Result<bool> {
    if !faces.iter().all(|f| is_triangle(f)) {
        return Ok(false);
    }
    x.open("gml:TriangulatedSurface", attributes)?;
    x.open(
        match x.gml {
            Version::V3_1 => "gml:trianglePatches",
            Version::V3_2 => "gml:patches",
        },
        &[],
    )?;
    for f in faces {
        x.open("gml:Triangle", &[])?;
        ring(x, "gml:exterior", &f.polygon.exterior, None)?;
        x.close()?;
    }
    x.close()?;
    x.close()?;
    Ok(true)
}
