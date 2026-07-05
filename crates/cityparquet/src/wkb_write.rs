//! Minimal ISO-WKB writer for the CityJSON → CityParquet geometry mapping.
//! Little-endian only. Container types wrap complete nested WKB geometries.

use cityparquet_schema::{CityParquetError, Result};
use cjseq::{Geometry, GeometryType, Transform};

const POINT_Z: u32 = 1001;
const LINESTRING_Z: u32 = 1002;
const POLYGON_Z: u32 = 1003;
const MULTIPOINT_Z: u32 = 1004;
const MULTILINESTRING_Z: u32 = 1005;
const MULTIPOLYGON_Z: u32 = 1006;
const GEOMETRYCOLLECTION_Z: u32 = 1007;
const POLYHEDRALSURFACE_Z: u32 = 1015;

pub struct VertexPool<'a> {
    vertices: &'a [Vec<i64>],
    scale: [f64; 3],
    translate: [f64; 3],
}

impl<'a> VertexPool<'a> {
    pub fn new(vertices: &'a [Vec<i64>], transform: &Transform) -> Self {
        let take3 = |v: &[f64], d: f64| {
            [
                *v.first().unwrap_or(&d),
                *v.get(1).unwrap_or(&d),
                *v.get(2).unwrap_or(&d),
            ]
        };
        Self {
            vertices,
            scale: take3(&transform.scale, 1.0),
            translate: take3(&transform.translate, 0.0),
        }
    }

    pub fn coord(&self, idx: usize) -> Result<[f64; 3]> {
        let v = self.vertices.get(idx).ok_or_else(|| {
            CityParquetError::Geometry(format!(
                "vertex index {idx} out of range ({} vertices)",
                self.vertices.len()
            ))
        })?;
        if v.len() < 3 {
            return Err(CityParquetError::Geometry(format!(
                "vertex {idx} has {} components",
                v.len()
            )));
        }
        Ok([
            v[0] as f64 * self.scale[0] + self.translate[0],
            v[1] as f64 * self.scale[1] + self.translate[1],
            v[2] as f64 * self.scale[2] + self.translate[2],
        ])
    }
}

#[derive(Debug)]
struct Bbox {
    bounds: [f64; 6],
    /// Coordinates written so far; zero means the bounds are still the
    /// initial +inf/-inf placeholders, i.e. no real geometry was emitted.
    count: usize,
}

impl Bbox {
    fn new() -> Self {
        Self {
            bounds: [
                f64::INFINITY,
                f64::INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
            ],
            count: 0,
        }
    }
    fn add(&mut self, c: [f64; 3]) {
        for (i, v) in c.into_iter().enumerate() {
            self.bounds[i] = self.bounds[i].min(v);
            self.bounds[i + 3] = self.bounds[i + 3].max(v);
        }
        self.count += 1;
    }
}

fn header(buf: &mut Vec<u8>, type_code: u32) {
    buf.push(0x01);
    buf.extend_from_slice(&type_code.to_le_bytes());
}

fn u32le(buf: &mut Vec<u8>, n: usize) {
    buf.extend_from_slice(&(n as u32).to_le_bytes());
}

fn coord(buf: &mut Vec<u8>, c: [f64; 3], bbox: &mut Bbox) {
    for v in c {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    bbox.add(c);
}

fn boundaries<T: serde::de::DeserializeOwned>(geom: &Geometry) -> Result<T> {
    serde_json::from_value(geom.boundaries.clone()).map_err(|e| {
        CityParquetError::Geometry(format!(
            "boundaries do not match {:?} shape: {e}",
            geom.thetype
        ))
    })
}

/// Full nested PolygonZ WKB (header + rings), rings closed.
fn polygon(
    buf: &mut Vec<u8>,
    rings: &[Vec<usize>],
    pool: &VertexPool,
    bbox: &mut Bbox,
) -> Result<()> {
    header(buf, POLYGON_Z);
    u32le(buf, rings.len());
    for ring in rings {
        if ring.len() < 3 {
            return Err(CityParquetError::Geometry(format!(
                "ring has {} vertex indices, need at least 3 to form a polygon ring",
                ring.len()
            )));
        }
        let first = ring[0];
        // Source is already closed (non-conformant CityJSON, but seen in the
        // wild) when its last INDEX equals its first: don't append a
        // duplicate closing point on top of it.
        let already_closed = ring.last() == Some(&first);
        let written_len = if already_closed {
            ring.len()
        } else {
            ring.len() + 1
        };
        u32le(buf, written_len);
        for &idx in ring {
            coord(buf, pool.coord(idx)?, bbox);
        }
        if !already_closed {
            coord(buf, pool.coord(first)?, bbox);
        }
    }
    Ok(())
}

/// PolyhedralSurfaceZ from a Solid's shells (flattened; shell structure is
/// preserved in geometry_properties, not in WKB).
fn polyhedral(
    buf: &mut Vec<u8>,
    shells: &[Vec<Vec<Vec<usize>>>],
    pool: &VertexPool,
    bbox: &mut Bbox,
) -> Result<()> {
    header(buf, POLYHEDRALSURFACE_Z);
    u32le(buf, shells.iter().map(|s| s.len()).sum());
    for shell in shells {
        for surface in shell {
            polygon(buf, surface, pool, bbox)?;
        }
    }
    Ok(())
}

pub fn point_to_wkb(c: [f64; 3]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(29);
    header(&mut buf, POINT_Z);
    let mut b = Bbox::new();
    coord(&mut buf, c, &mut b);
    buf
}

pub fn geometry_to_wkb(geom: &Geometry, pool: &VertexPool) -> Result<Option<(Vec<u8>, [f64; 6])>> {
    let mut buf = Vec::new();
    let mut bbox = Bbox::new();
    match geom.thetype {
        GeometryType::GeometryInstance => return Ok(None),
        GeometryType::MultiPoint => {
            let idxs: Vec<usize> = boundaries(geom)?;
            header(&mut buf, MULTIPOINT_Z);
            u32le(&mut buf, idxs.len());
            for idx in idxs {
                header(&mut buf, POINT_Z);
                coord(&mut buf, pool.coord(idx)?, &mut bbox);
            }
        }
        GeometryType::MultiLineString => {
            let lines: Vec<Vec<usize>> = boundaries(geom)?;
            header(&mut buf, MULTILINESTRING_Z);
            u32le(&mut buf, lines.len());
            for line in lines {
                header(&mut buf, LINESTRING_Z);
                u32le(&mut buf, line.len());
                for idx in line {
                    coord(&mut buf, pool.coord(idx)?, &mut bbox);
                }
            }
        }
        GeometryType::MultiSurface | GeometryType::CompositeSurface => {
            let surfaces: Vec<Vec<Vec<usize>>> = boundaries(geom)?;
            header(&mut buf, MULTIPOLYGON_Z);
            u32le(&mut buf, surfaces.len());
            for surface in &surfaces {
                polygon(&mut buf, surface, pool, &mut bbox)?;
            }
        }
        GeometryType::Solid => {
            let shells: Vec<Vec<Vec<Vec<usize>>>> = boundaries(geom)?;
            polyhedral(&mut buf, &shells, pool, &mut bbox)?;
        }
        GeometryType::MultiSolid | GeometryType::CompositeSolid => {
            let solids: Vec<Vec<Vec<Vec<Vec<usize>>>>> = boundaries(geom)?;
            header(&mut buf, GEOMETRYCOLLECTION_Z);
            u32le(&mut buf, solids.len());
            for solid in &solids {
                polyhedral(&mut buf, solid, pool, &mut bbox)?;
            }
        }
    }
    if bbox.count == 0 {
        // No coordinates were actually written (e.g. empty boundaries): an
        // infinite placeholder bbox is worse than no geometry at all, so
        // report this the same way as GeometryInstance — no WKB emitted.
        return Ok(None);
    }
    Ok(Some((buf, bbox.bounds)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use geo_traits::GeometryTrait;

    fn pool_and(transform_scale: f64) -> (Vec<Vec<i64>>, cjseq::Transform) {
        (
            vec![
                vec![0, 0, 0],
                vec![1000, 0, 0],
                vec![1000, 1000, 0],
                vec![0, 1000, 0],
                vec![0, 0, 1000],
            ],
            cjseq::Transform {
                scale: vec![transform_scale; 3],
                translate: vec![10.0, 20.0, 30.0],
            },
        )
    }

    #[test]
    fn dequantises_with_transform() {
        let (v, t) = pool_and(0.001);
        let pool = VertexPool::new(&v, &t);
        assert_eq!(pool.coord(1).unwrap(), [11.0, 20.0, 30.0]);
        let err = pool.coord(99).unwrap_err();
        assert!(
            matches!(err, CityParquetError::Geometry(_)),
            "expected Geometry error for out-of-range vertex index, got {err:?}"
        );
    }

    #[test]
    fn multisurface_becomes_multipolygon_z_with_closed_rings() {
        let (v, t) = pool_and(1.0);
        let pool = VertexPool::new(&v, &t);
        let geom = cjseq::Geometry {
            thetype: cjseq::GeometryType::MultiSurface,
            lod: Some("2".into()),
            boundaries: serde_json::json!([[[0, 1, 2, 3]]]),
            semantics: None,
            material: None,
            texture: None,
            template: None,
            transformation_matrix: None,
        };
        let (bytes, bbox) = geometry_to_wkb(&geom, &pool).unwrap().unwrap();
        assert_eq!(bytes[0], 0x01);
        assert_eq!(u32::from_le_bytes(bytes[1..5].try_into().unwrap()), 1006); // MultiPolygonZ
        // one nested full PolygonZ header
        assert_eq!(u32::from_le_bytes(bytes[5..9].try_into().unwrap()), 1); // numPolygons
        assert_eq!(bytes[9], 0x01);
        assert_eq!(u32::from_le_bytes(bytes[10..14].try_into().unwrap()), 1003);
        // ring closed: 4 source points -> 5 WKB points
        let num_rings = u32::from_le_bytes(bytes[14..18].try_into().unwrap());
        let num_points = u32::from_le_bytes(bytes[18..22].try_into().unwrap());
        assert_eq!((num_rings, num_points), (1, 5));
        assert_eq!(bbox, [10.0, 20.0, 30.0, 1010.0, 1020.0, 30.0]);
    }

    #[test]
    fn ring_with_fewer_than_3_vertices_errors() {
        let (v, t) = pool_and(1.0);
        let pool = VertexPool::new(&v, &t);
        let geom = cjseq::Geometry {
            thetype: cjseq::GeometryType::MultiSurface,
            lod: Some("2".into()),
            boundaries: serde_json::json!([[[0, 1]]]),
            semantics: None,
            material: None,
            texture: None,
            template: None,
            transformation_matrix: None,
        };
        let err = geometry_to_wkb(&geom, &pool).unwrap_err();
        assert!(
            matches!(err, CityParquetError::Geometry(_)),
            "expected Geometry error for a too-short ring, got {err:?}"
        );
        let msg = err.to_string();
        assert!(
            msg.contains("3") || msg.to_lowercase().contains("ring"),
            "error message should name the problem, got: {msg}"
        );
    }

    #[test]
    fn pre_closed_ring_is_not_re_closed() {
        // Source ring [0, 1, 2, 0] is already closed (non-conformant CityJSON,
        // but seen in the wild): the writer must not append a duplicate
        // closing point on top of it.
        let (v, t) = pool_and(1.0);
        let pool = VertexPool::new(&v, &t);
        let geom = cjseq::Geometry {
            thetype: cjseq::GeometryType::MultiSurface,
            lod: Some("2".into()),
            boundaries: serde_json::json!([[[0, 1, 2, 0]]]),
            semantics: None,
            material: None,
            texture: None,
            template: None,
            transformation_matrix: None,
        };
        let (bytes, _bbox) = geometry_to_wkb(&geom, &pool).unwrap().unwrap();
        let num_points = u32::from_le_bytes(bytes[18..22].try_into().unwrap());
        assert_eq!(
            num_points, 4,
            "pre-closed 4-index ring must yield 4 WKB points, not 5"
        );
        let parsed = wkb::reader::read_wkb(&bytes)
            .expect("wkb crate oracle must parse our own MultiPolygonZ output");
        assert!(matches!(
            parsed.as_type(),
            geo_traits::GeometryType::MultiPolygon(_)
        ));
    }

    #[test]
    fn solid_becomes_polyhedral_surface_z() {
        let (v, t) = pool_and(1.0);
        let pool = VertexPool::new(&v, &t);
        let geom = cjseq::Geometry {
            thetype: cjseq::GeometryType::Solid,
            lod: Some("2".into()),
            boundaries: serde_json::json!([[[[0, 1, 2, 3]], [[0, 1, 4]]]]),
            semantics: None,
            material: None,
            texture: None,
            template: None,
            transformation_matrix: None,
        };
        let (bytes, _) = geometry_to_wkb(&geom, &pool).unwrap().unwrap();
        assert_eq!(u32::from_le_bytes(bytes[1..5].try_into().unwrap()), 1015); // PolyhedralSurfaceZ
        assert_eq!(u32::from_le_bytes(bytes[5..9].try_into().unwrap()), 2); // faces across shells
        assert_eq!(u32::from_le_bytes(bytes[10..14].try_into().unwrap()), 1003); // nested PolygonZ
    }

    #[test]
    fn empty_multipoint_boundaries_yield_no_wkb() {
        let (v, t) = pool_and(1.0);
        let pool = VertexPool::new(&v, &t);
        let geom = cjseq::Geometry {
            thetype: cjseq::GeometryType::MultiPoint,
            lod: Some("2".into()),
            boundaries: serde_json::json!([]),
            semantics: None,
            material: None,
            texture: None,
            template: None,
            transformation_matrix: None,
        };
        assert!(
            geometry_to_wkb(&geom, &pool).unwrap().is_none(),
            "an empty MultiPoint must not emit an infinite-bbox WKB payload"
        );
    }

    #[test]
    fn empty_multisurface_boundaries_yield_no_wkb() {
        let (v, t) = pool_and(1.0);
        let pool = VertexPool::new(&v, &t);
        let geom = cjseq::Geometry {
            thetype: cjseq::GeometryType::MultiSurface,
            lod: Some("2".into()),
            boundaries: serde_json::json!([]),
            semantics: None,
            material: None,
            texture: None,
            template: None,
            transformation_matrix: None,
        };
        assert!(
            geometry_to_wkb(&geom, &pool).unwrap().is_none(),
            "an empty MultiSurface must not emit an infinite-bbox WKB payload"
        );
    }

    #[test]
    fn geometry_instance_is_none() {
        let (v, t) = pool_and(1.0);
        let pool = VertexPool::new(&v, &t);
        let geom = cjseq::Geometry {
            thetype: cjseq::GeometryType::GeometryInstance,
            lod: None,
            boundaries: serde_json::json!([0]),
            semantics: None,
            material: None,
            texture: None,
            template: Some(0),
            transformation_matrix: None,
        };
        assert!(geometry_to_wkb(&geom, &pool).unwrap().is_none());
    }
}
