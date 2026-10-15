//! One `geometry_data` row rebuilt as the GML geometry it was imported
//! from.
//!
//! 3DCityDB stores a geometry as PostGIS (a `POLYHEDRALSURFACE Z` for a
//! solid, a `MULTIPOLYGON Z` for a surface collection) and describes its
//! GML structure in `geometry_properties`: the root's GML type, and a flat
//! list of children, each with its type, its `gml:id` (`objectId`), its
//! parent (an index into the list, the root when absent) and, for a
//! primitive, which of the PostGIS geometry's members it is
//! (`geometryIndex`). The GML types are numbered: 1 Point, 2 MultiPoint,
//! 3 LineString, 4 MultiLineString, 5 Polygon, 6 CompositeSurface,
//! 7 TriangulatedSurface, 8 MultiSurface, 9 Solid, 10 CompositeSolid,
//! 11 MultiSolid.
//!
//! A polygon shared by several geometries in the source (a solid's face
//! that is also a boundary surface's) is stored once per geometry, each copy
//! under the same `objectId`. The database stores a surface used reversed
//! (a `gml:OrientableSurface orientation="-"`) with its rings already
//! reversed, and marks it `isReversed`; the reader gives such a surface
//! back as the XML readers read it: its base polygon, wound as defined,
//! used reversed.

use std::collections::HashSet;

use citygml_core::gml::read::{Collected, RawSolid, RefTarget, SolidGeom, SurfaceRef, Xlink};
use citygml_core::model::{Line, Polygon};
use citygml_core::{Error, Result};
use serde_json::Value as Json;

use crate::wkb::{self, Primitive};

const POINT: u64 = 1;
const MULTI_POINT: u64 = 2;
const LINE_STRING: u64 = 3;
const MULTI_LINE_STRING: u64 = 4;
const POLYGON: u64 = 5;
const COMPOSITE_SURFACE: u64 = 6;
const TRIANGULATED_SURFACE: u64 = 7;
const MULTI_SURFACE: u64 = 8;
const SOLID: u64 = 9;
const COMPOSITE_SOLID: u64 = 10;
const MULTI_SOLID: u64 = 11;

/// A geometry, as an object builder takes it.
#[derive(Debug)]
pub(crate) enum Built {
    Solid(SolidGeom),
    Surfaces(Surfaces),
    Lines(Vec<Line>),
    Points(Vec<[f64; 3]>),
    /// A GML type CityJSON cannot hold in one geometry (a `MultiSolid`),
    /// by name.
    Unsupported(&'static str),
}

/// A surface collection, each polygon wound as its base surface is defined
/// and whether it is used reversed.
#[derive(Debug, Default)]
pub(crate) struct Surfaces {
    members: Vec<(Polygon, bool)>,
}

impl Surfaces {
    /// The faces as used, in order: a polygon used reversed has its rings
    /// reversed. What a plain surface geometry, a TIN or a template holds.
    pub(crate) fn into_faces(self) -> Collected {
        Collected {
            polygons: self
                .members
                .into_iter()
                .map(|(p, reverse)| (p.id.clone(), if reverse { reversed(p) } else { p }))
                .collect(),
            ..Collected::default()
        }
    }

    /// The faces as a boundary surface holds them, as the XML readers read
    /// a `gml:OrientableSurface` whose base surface is defined elsewhere: a
    /// polygon used reversed is a reference to its base polygon, returned
    /// apart, for the object to register as an xlink target only. One
    /// without an id to refer to it by is the face as used.
    pub(crate) fn by_reference(self) -> (Vec<(Option<String>, Polygon)>, Collected) {
        let mut bases = Vec::new();
        let mut collected = Collected::default();
        for (polygon, reverse) in self.members {
            match &polygon.id {
                Some(id) if reverse => {
                    collected.xlinks.push(Xlink {
                        id: id.clone(),
                        reverse: true,
                    });
                    bases.push((Some(id.clone()), polygon));
                }
                _ => {
                    let polygon = if reverse { reversed(polygon) } else { polygon };
                    collected.polygons.push((polygon.id.clone(), polygon));
                }
            }
        }
        (bases, collected)
    }
}

/// One entry of `geometry_properties`: the root, or one of its children.
struct Node {
    kind: u64,
    id: Option<String>,
    parent: Option<usize>,
    index: Option<usize>,
    reversed: bool,
}

/// Rebuild one geometry from its WKB and its `geometry_properties`. Each
/// polygon gets ring ids when `textured` holds its id, for texture
/// coordinates to name (`{polygon id}#{ring}`).
pub(crate) fn build(wkb: &[u8], properties: &Json, textured: &HashSet<String>) -> Result<Built> {
    let primitives = wkb::primitives(wkb)?;
    let root = node(properties)?;
    let children: Vec<Node> = properties
        .get("children")
        .and_then(Json::as_array)
        .map(|c| c.iter().map(node).collect::<Result<_>>())
        .transpose()?
        .unwrap_or_default();
    let tree = Tree {
        root: &root,
        children: &children,
        primitives: &primitives,
        textured,
    };
    Ok(match root.kind {
        SOLID => Built::Solid(SolidGeom::Solid(tree.solid(None)?)),
        COMPOSITE_SOLID => Built::Solid(SolidGeom::Composite(
            tree.below(None, SOLID)
                .map(|i| tree.solid(Some(i)))
                .collect::<Result<_>>()?,
        )),
        MULTI_SOLID => Built::Unsupported("MultiSolid"),
        POLYGON => {
            let mut surfaces = Surfaces::default();
            tree.add(&mut surfaces, &root)?;
            Built::Surfaces(surfaces)
        }
        COMPOSITE_SURFACE | TRIANGULATED_SURFACE | MULTI_SURFACE => {
            let mut surfaces = Surfaces::default();
            for c in children.iter().filter(|c| c.kind == POLYGON) {
                tree.add(&mut surfaces, c)?;
            }
            Built::Surfaces(surfaces)
        }
        LINE_STRING | MULTI_LINE_STRING => Built::Lines(
            primitives
                .into_iter()
                .filter_map(|p| match p {
                    Primitive::LineString(line) if line.len() >= 2 => Some(line),
                    _ => None,
                })
                .collect(),
        ),
        POINT | MULTI_POINT => Built::Points(
            primitives
                .into_iter()
                .filter_map(|p| match p {
                    Primitive::Point(point) => Some(point),
                    _ => None,
                })
                .collect(),
        ),
        other => {
            return Err(Error::invalid(format!(
                "geometry_properties names GML type {other}, which 3DCityDB does not define"
            )));
        }
    })
}

fn node(json: &Json) -> Result<Node> {
    let kind = json
        .get("type")
        .and_then(Json::as_u64)
        .ok_or_else(|| Error::invalid("geometry_properties entry without a type"))?;
    let index = |key: &str| json.get(key).and_then(Json::as_u64).map(|i| i as usize);
    Ok(Node {
        kind,
        id: json
            .get("objectId")
            .and_then(Json::as_str)
            .map(str::to_string),
        parent: index("parent"),
        index: index("geometryIndex"),
        reversed: json.get("isReversed").and_then(Json::as_bool) == Some(true),
    })
}

struct Tree<'a> {
    root: &'a Node,
    children: &'a [Node],
    primitives: &'a [Primitive],
    textured: &'a HashSet<String>,
}

impl Tree<'_> {
    /// The children of `parent` (the root when `None`) of GML type `kind`,
    /// by index.
    fn below(&self, parent: Option<usize>, kind: u64) -> impl Iterator<Item = usize> + '_ {
        self.children
            .iter()
            .enumerate()
            .filter(move |(_, c)| c.parent == parent && c.kind == kind)
            .map(|(i, _)| i)
    }

    /// The solid at child `at` (the root when `None`): its shells, the
    /// exterior first, each a list of polygons.
    fn solid(&self, at: Option<usize>) -> Result<RawSolid> {
        let shells = self
            .below(at, COMPOSITE_SURFACE)
            .map(|shell| {
                self.below(Some(shell), POLYGON)
                    .map(|p| {
                        let node = &self.children[p];
                        Ok(SurfaceRef {
                            reverse: node.reversed,
                            target: RefTarget::Inline(self.polygon(node)?),
                        })
                    })
                    .collect::<Result<Vec<_>>>()
            })
            .collect::<Result<_>>()?;
        Ok(RawSolid { shells })
    }

    /// Add polygon `node` to `surfaces`.
    fn add(&self, surfaces: &mut Surfaces, node: &Node) -> Result<()> {
        surfaces.members.push((self.polygon(node)?, node.reversed));
        Ok(())
    }

    /// Polygon `node`, wound as its base surface is defined: a polygon used
    /// reversed is stored with its rings reversed, so they are reversed
    /// back.
    fn polygon(&self, node: &Node) -> Result<Polygon> {
        let at = node.index.unwrap_or(0);
        let Some(Primitive::Polygon(rings)) = self.primitives.get(at) else {
            return Err(Error::invalid(format!(
                "geometry_properties names polygon {at}, which the geometry does not hold"
            )));
        };
        let mut rings = rings.iter().map(|ring| {
            if node.reversed {
                reverse_ring(ring)
            } else {
                ring.clone()
            }
        });
        let exterior = rings.next().unwrap_or_default();
        let interiors: Vec<_> = rings.collect();
        let ring_ids = match &node.id {
            Some(id) if self.textured.contains(id) => (0..=interiors.len())
                .map(|k| Some(ring_id(id, k)))
                .collect(),
            _ => vec![None; interiors.len() + 1],
        };
        Ok(Polygon {
            id: node.id.clone(),
            exterior,
            interiors,
            ring_ids,
            groups: self.groups(node),
        })
    }

    /// The ids of the identified surface aggregates holding `node`,
    /// innermost first, as the XML readers record them: what a material
    /// targeting a whole `gml:MultiSurface`, or a TIN's
    /// `gml:TriangulatedSurface`, colours.
    fn groups(&self, node: &Node) -> Vec<String> {
        let aggregate = |n: &Node| {
            matches!(
                n.kind,
                COMPOSITE_SURFACE | MULTI_SURFACE | TRIANGULATED_SURFACE
            )
            .then(|| n.id.clone())
            .flatten()
        };
        let mut groups = Vec::new();
        let mut parent = node.parent;
        // A malformed parent chain could loop; no aggregate nests deeper
        // than the list is long.
        for _ in 0..=self.children.len() {
            let Some(at) = parent else {
                break;
            };
            let Some(n) = self.children.get(at) else {
                break;
            };
            groups.extend(aggregate(n));
            parent = n.parent;
        }
        if !std::ptr::eq(node, self.root) {
            groups.extend(aggregate(self.root));
        }
        groups
    }
}

/// The id the reader gives ring `k` of polygon `polygon`, which a texture's
/// coordinates name: the database keys them by polygon and ring position.
pub(crate) fn ring_id(polygon: &str, k: usize) -> String {
    format!("{polygon}#{k}")
}

/// `ring` wound the other way, from the same first point, as GML reverses
/// a ring.
fn reverse_ring(ring: &[[f64; 3]]) -> Vec<[f64; 3]> {
    let mut out = ring.to_vec();
    if out.len() > 1 {
        out[1..].reverse();
    }
    out
}

/// `polygon` with every ring reversed: a reversed face as it is used.
fn reversed(mut polygon: Polygon) -> Polygon {
    polygon.exterior = reverse_ring(&polygon.exterior);
    for ring in &mut polygon.interiors {
        *ring = reverse_ring(ring);
    }
    polygon
}
