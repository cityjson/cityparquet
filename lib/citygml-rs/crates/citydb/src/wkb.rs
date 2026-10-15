//! Well-known binary, as `ST_AsBinary` returns a 3DCityDB geometry, read
//! into the primitives a `geometry_properties` description indexes.

use citygml_core::model::{Line, Ring};
use citygml_core::{Error, Result};

/// One primitive of a geometry: what a `geometryIndex` names.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Primitive {
    Point([f64; 3]),
    LineString(Line),
    /// Exterior ring first, each without its closing point.
    Polygon(Vec<Ring>),
}

/// The primitives of a WKB geometry, in order: a collection (a
/// `MULTIPOLYGON`, a `POLYHEDRALSURFACE`, a `TIN`, …) is flattened into its
/// members, which is how 3DCityDB numbers them.
pub(crate) fn primitives(bytes: &[u8]) -> Result<Vec<Primitive>> {
    let mut reader = Reader { bytes, at: 0 };
    let mut out = Vec::new();
    reader.geometry(&mut out, 0)?;
    Ok(out)
}

/// How deep collections may nest: a guard against a malformed geometry.
const MAX_DEPTH: usize = 8;

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

/// The coordinates a geometry stores per point.
#[derive(Clone, Copy)]
struct Layout {
    little: bool,
    z: bool,
    m: bool,
}

impl Reader<'_> {
    fn geometry(&mut self, out: &mut Vec<Primitive>, depth: usize) -> Result<()> {
        if depth > MAX_DEPTH {
            return Err(invalid("geometry collections nest too deep"));
        }
        let little = match self.byte()? {
            0 => false,
            1 => true,
            other => return Err(invalid(&format!("byte order {other}"))),
        };
        let raw = self.u32(little)?;
        // ISO WKB adds 1000 for Z, 2000 for M and 3000 for both; extended
        // WKB sets flag bits instead, and may carry an SRID.
        let (z_flag, m_flag, srid_flag) = (
            raw & 0x8000_0000 != 0,
            raw & 0x4000_0000 != 0,
            raw & 0x2000_0000 != 0,
        );
        let code = raw & 0x0FFF_FFFF;
        let (base, dims) = (code % 1000, code / 1000);
        let layout = Layout {
            little,
            z: z_flag || dims == 1 || dims == 3,
            m: m_flag || dims == 2 || dims == 3,
        };
        if srid_flag {
            self.u32(little)?;
        }
        match base {
            1 => out.push(Primitive::Point(self.point(layout)?)),
            2 => out.push(Primitive::LineString(self.points(layout)?)),
            3 | 17 => out.push(Primitive::Polygon(self.polygon(layout)?)),
            // MultiPoint, MultiLineString, MultiPolygon, GeometryCollection,
            // PolyhedralSurface, TIN: each member a geometry of its own.
            4..=7 | 15 | 16 => {
                let n = self.u32(little)?;
                for _ in 0..n {
                    self.geometry(out, depth + 1)?;
                }
            }
            other => return Err(invalid(&format!("geometry type {other}"))),
        }
        Ok(())
    }

    fn polygon(&mut self, layout: Layout) -> Result<Vec<Ring>> {
        let n = self.u32(layout.little)?;
        let mut rings = Vec::with_capacity(n.min(1024) as usize);
        for _ in 0..n {
            let mut ring = self.points(layout)?;
            if ring.len() >= 2 && ring.first() == ring.last() {
                ring.pop();
            }
            rings.push(ring);
        }
        Ok(rings)
    }

    fn points(&mut self, layout: Layout) -> Result<Vec<[f64; 3]>> {
        let n = self.u32(layout.little)? as usize;
        // Each point takes at least 16 bytes, so a count the bytes cannot
        // hold is refused before anything is allocated for it.
        if n > (self.bytes.len() - self.at) / 16 {
            return Err(invalid("a point count past the end of the geometry"));
        }
        (0..n).map(|_| self.point(layout)).collect()
    }

    fn point(&mut self, layout: Layout) -> Result<[f64; 3]> {
        let x = self.f64(layout.little)?;
        let y = self.f64(layout.little)?;
        let z = if layout.z {
            self.f64(layout.little)?
        } else {
            0.0
        };
        if layout.m {
            self.f64(layout.little)?;
        }
        Ok([x, y, z])
    }

    fn take<const N: usize>(&mut self) -> Result<[u8; N]> {
        let bytes = self
            .bytes
            .get(self.at..self.at + N)
            .ok_or_else(|| invalid("truncated"))?;
        self.at += N;
        Ok(bytes.try_into().expect("N bytes"))
    }

    fn byte(&mut self) -> Result<u8> {
        Ok(self.take::<1>()?[0])
    }

    fn u32(&mut self, little: bool) -> Result<u32> {
        let b = self.take::<4>()?;
        Ok(if little {
            u32::from_le_bytes(b)
        } else {
            u32::from_be_bytes(b)
        })
    }

    fn f64(&mut self, little: bool) -> Result<f64> {
        let b = self.take::<8>()?;
        Ok(if little {
            f64::from_le_bytes(b)
        } else {
            f64::from_be_bytes(b)
        })
    }
}

fn invalid(what: &str) -> Error {
    Error::invalid(format!("unreadable WKB geometry: {what}"))
}
