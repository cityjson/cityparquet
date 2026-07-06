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

/// 2^53: no `f64` can represent an integer beyond this magnitude exactly, so
/// it is the shared ceiling for both quantised-component and final
/// world-coordinate guards.
const MAX_SAFE: f64 = 9_007_199_254_740_992.0;

/// Vertex storage backing a [`VertexPool`]: either the dataset's quantised
/// integer vertices (dequantised through the CityJSON `transform` on lookup)
/// or a template's raw floats (CityJSON spec §3.4: `vertices-templates` are
/// NOT subject to the dataset transform, so they are looked up verbatim).
enum VertexStorage<'a> {
    Quantised {
        vertices: &'a [Vec<i64>],
        scale: [f64; 3],
        translate: [f64; 3],
    },
    Raw {
        vertices: &'a [Vec<f64>],
    },
}

pub struct VertexPool<'a>(VertexStorage<'a>);

impl<'a> VertexPool<'a> {
    pub fn new(vertices: &'a [Vec<i64>], transform: &Transform) -> Self {
        let take3 = |v: &[f64], d: f64| {
            [
                *v.first().unwrap_or(&d),
                *v.get(1).unwrap_or(&d),
                *v.get(2).unwrap_or(&d),
            ]
        };
        Self(VertexStorage::Quantised {
            vertices,
            scale: take3(&transform.scale, 1.0),
            translate: take3(&transform.translate, 0.0),
        })
    }

    /// Template-local vertex pool: coordinates are looked up verbatim, no
    /// scale/translate applied (CityJSON spec §3.4 — a geometry template's
    /// `vertices-templates` are raw floats, unlike the dataset's quantised
    /// `vertices`). The 2^53 world-coordinate guard still applies (a
    /// too-large `f64` loses precision regardless of where it came from);
    /// the quantised-*component* guard from [`Self::new`] does not apply
    /// here, since there is no quantised integer component to check.
    pub fn raw(vertices: &'a [Vec<f64>]) -> VertexPool<'a> {
        VertexPool(VertexStorage::Raw { vertices })
    }

    pub fn coord(&self, idx: usize) -> Result<[f64; 3]> {
        match &self.0 {
            VertexStorage::Quantised {
                vertices,
                scale,
                translate,
            } => {
                const MAX_QUANTISED: u64 = 1u64 << 53;

                let v = vertices.get(idx).ok_or_else(|| {
                    CityParquetError::Geometry(format!(
                        "vertex index {idx} out of range ({} vertices)",
                        vertices.len()
                    ))
                })?;
                if v.len() < 3 {
                    return Err(CityParquetError::Geometry(format!(
                        "vertex {idx} has {} components",
                        v.len()
                    )));
                }

                // Guard quantised components: no component can exceed 2^53 in
                // magnitude or it loses precision when converted to f64.
                for (i, &val) in [v[0], v[1], v[2]].iter().enumerate() {
                    if val.unsigned_abs() > MAX_QUANTISED {
                        return Err(CityParquetError::Geometry(format!(
                            "vertex {idx} component {i}: quantised value {val} exceeds 2^53 magnitude (loses f64 precision)"
                        )));
                    }
                }

                // Compute world coordinates and check their magnitude
                let coords = [
                    v[0] as f64 * scale[0] + translate[0],
                    v[1] as f64 * scale[1] + translate[1],
                    v[2] as f64 * scale[2] + translate[2],
                ];

                // Guard final world coordinates: no coordinate can exceed
                // 2^53 in magnitude.
                for (i, &c) in coords.iter().enumerate() {
                    if c.abs() >= MAX_SAFE {
                        return Err(CityParquetError::Geometry(format!(
                            "vertex {idx} coordinate {i}: computed value {c} exceeds 2^53 magnitude (loses f64 precision)"
                        )));
                    }
                }

                Ok(coords)
            }
            VertexStorage::Raw { vertices } => {
                let v = vertices.get(idx).ok_or_else(|| {
                    CityParquetError::Geometry(format!(
                        "vertex index {idx} out of range ({} vertices)",
                        vertices.len()
                    ))
                })?;
                if v.len() < 3 {
                    return Err(CityParquetError::Geometry(format!(
                        "vertex {idx} has {} components",
                        v.len()
                    )));
                }
                let coords = [v[0], v[1], v[2]];

                // Guard final coordinates: same 2^53 ceiling as the
                // quantised path, applied directly since raw floats have no
                // scale/translate step to compute through.
                for (i, &c) in coords.iter().enumerate() {
                    if c.abs() >= MAX_SAFE {
                        return Err(CityParquetError::Geometry(format!(
                            "vertex {idx} coordinate {i}: raw value {c} exceeds 2^53 magnitude (loses f64 precision)"
                        )));
                    }
                }

                Ok(coords)
            }
        }
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

/// Structural drops performed while normalising one geometry's rings.
#[derive(Debug, Default)]
struct Drops {
    /// Rings dropped because they cannot form a valid WKB ring (< 3
    /// effective vertices after stripping a pre-baked closure).
    rings: usize,
    /// Original flat surface/face positions (within the geometry) of
    /// surfaces dropped because their EXTERIOR ring was degenerate.
    surfaces: Vec<usize>,
}

/// Structural ring normalisation (narrow by design — this is NOT zero-area
/// cleanup): strip trailing duplicates of the first vertex index to a
/// fixpoint (sources in the wild bake the WKB closure more than once, e.g.
/// `[0, 1, 0, 0]`: a single strip would leave a still-closed `[0, 1, 0]`
/// that the hardened reader rejects); if fewer than 3 vertices remain the
/// ring cannot form a valid WKB ring and is dropped (`None`). Zero-area
/// rings with >= 3 effective vertices pass through unchanged — data quality
/// is not the format's business.
///
/// Known limitation (deliberate): closure detection is INDEX-based. A ring
/// whose last vertex is a DIFFERENT index carrying a bitwise-identical
/// coordinate to the first is treated as unclosed — it gets a closing
/// vertex appended and survives both this policy and the reader's checks
/// as a structurally valid zero-area ring (a→b→a). The narrow policy only
/// drops rings that cannot form a structural WKB ring at all;
/// coordinate-level degeneracy is data quality, out of scope.
fn normalise_ring(ring: &[usize]) -> Option<&[usize]> {
    let mut stripped = ring;
    // Fixpoint, not single-strip: sources in the wild bake the closure more
    // than once ([0,1,0,0]); a single strip left a still-closed [0,1,0] that
    // the hardened reader rejects.
    while stripped.len() >= 2 && stripped.first() == stripped.last() {
        stripped = &stripped[..stripped.len() - 1];
    }
    (stripped.len() >= 3).then_some(stripped)
}

/// Normalise one surface's rings. Returns the kept rings, or `None` when
/// the surface must be dropped entirely: either it has no rings at all (no
/// WKB polygon can be formed), or its EXTERIOR ring (index 0) is degenerate
/// (interior rings cannot stand without it). Every degenerate ring is
/// counted in `drops.rings` — including interior rings of a surface that is
/// dropped anyway — so the reported ring total is exact. `pos` is the
/// surface's original flat position within the geometry, recorded so the
/// encoder can realign per-surface semantics/material/texture arrays.
fn normalise_surface<'r>(
    rings: &'r [Vec<usize>],
    pos: usize,
    drops: &mut Drops,
) -> Option<Vec<&'r [usize]>> {
    if rings.is_empty() {
        // A surface with no rings at all cannot form a WKB polygon.
        drops.surfaces.push(pos);
        return None;
    }
    let mut kept = Vec::with_capacity(rings.len());
    let mut exterior_dropped = false;
    for (i, ring) in rings.iter().enumerate() {
        match normalise_ring(ring) {
            Some(r) => kept.push(r),
            None => {
                drops.rings += 1;
                if i == 0 {
                    exterior_dropped = true;
                }
            }
        }
    }
    if exterior_dropped {
        drops.surfaces.push(pos);
        return None;
    }
    Some(kept)
}

/// Full nested PolygonZ WKB (header + normalised rings, each explicitly
/// closed by repeating its first coordinate).
fn write_polygon(
    buf: &mut Vec<u8>,
    rings: &[&[usize]],
    pool: &VertexPool,
    bbox: &mut Bbox,
) -> Result<()> {
    header(buf, POLYGON_Z);
    u32le(buf, rings.len());
    for ring in rings {
        u32le(buf, ring.len() + 1);
        for &idx in *ring {
            coord(buf, pool.coord(idx)?, bbox);
        }
        coord(buf, pool.coord(ring[0])?, bbox);
    }
    Ok(())
}

/// PolyhedralSurfaceZ from normalised faces (flattened; shell structure is
/// preserved in geometry_properties, not in WKB).
fn write_polyhedral(
    buf: &mut Vec<u8>,
    faces: &[Vec<&[usize]>],
    pool: &VertexPool,
    bbox: &mut Bbox,
) -> Result<()> {
    header(buf, POLYHEDRALSURFACE_Z);
    u32le(buf, faces.len());
    for face in faces {
        write_polygon(buf, face, pool, bbox)?;
    }
    Ok(())
}

/// Normalise a Solid's shells into a flat kept-face list, advancing `pos`
/// (the flat face position within the whole geometry) across every source
/// face — dropped or kept — so recorded drop positions stay original.
fn normalise_shells<'r>(
    shells: &'r [Vec<Vec<Vec<usize>>>],
    pos: &mut usize,
    drops: &mut Drops,
) -> Vec<Vec<&'r [usize]>> {
    let mut kept = Vec::new();
    for shell in shells {
        for surface in shell {
            if let Some(k) = normalise_surface(surface, *pos, drops) {
                kept.push(k);
            }
            *pos += 1;
        }
    }
    kept
}

pub fn point_to_wkb(c: [f64; 3]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(29);
    header(&mut buf, POINT_Z);
    let mut b = Bbox::new();
    coord(&mut buf, c, &mut b);
    buf
}

/// One geometry's WKB encoding plus what structural normalisation dropped
/// to produce it. The writer's output always satisfies the hardened
/// `wkb_read` checks by construction.
#[derive(Debug, Clone, PartialEq)]
pub struct WkbOutcome {
    pub bytes: Vec<u8>,
    pub bbox: [f64; 6],
    /// Structurally degenerate rings dropped (the [a,b,a] closure shape:
    /// fewer than 3 effective vertices).
    pub dropped_rings: usize,
    /// Original flat surface positions (within this geometry) of surfaces
    /// dropped because their exterior ring was degenerate. Flat means: the
    /// boundaries index for MultiSurface/CompositeSurface; the face position
    /// counted across shells (and across solids for MultiSolid/
    /// CompositeSolid) for the solid types.
    pub dropped_surfaces: Vec<usize>,
}

pub fn geometry_to_wkb(geom: &Geometry, pool: &VertexPool) -> Result<Option<WkbOutcome>> {
    let mut buf = Vec::new();
    let mut bbox = Bbox::new();
    let mut drops = Drops::default();
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
            let kept: Vec<Vec<&[usize]>> = surfaces
                .iter()
                .enumerate()
                .filter_map(|(pos, surface)| normalise_surface(surface, pos, &mut drops))
                .collect();
            header(&mut buf, MULTIPOLYGON_Z);
            u32le(&mut buf, kept.len());
            for surface in &kept {
                write_polygon(&mut buf, surface, pool, &mut bbox)?;
            }
        }
        GeometryType::Solid => {
            let shells: Vec<Vec<Vec<Vec<usize>>>> = boundaries(geom)?;
            let mut pos = 0;
            let kept = normalise_shells(&shells, &mut pos, &mut drops);
            write_polyhedral(&mut buf, &kept, pool, &mut bbox)?;
        }
        GeometryType::MultiSolid | GeometryType::CompositeSolid => {
            let solids: Vec<Vec<Vec<Vec<Vec<usize>>>>> = boundaries(geom)?;
            header(&mut buf, GEOMETRYCOLLECTION_Z);
            u32le(&mut buf, solids.len());
            let mut pos = 0;
            for solid in &solids {
                let kept = normalise_shells(solid, &mut pos, &mut drops);
                write_polyhedral(&mut buf, &kept, pool, &mut bbox)?;
            }
        }
    }
    if bbox.count == 0 {
        // No coordinates were actually written (empty boundaries, or every
        // surface dropped as degenerate): an infinite placeholder bbox is
        // worse than no geometry at all, so report this the same way as
        // GeometryInstance — no WKB emitted (drop info is not carried in
        // this corner; there is no geometry value to attach it to).
        return Ok(None);
    }
    Ok(Some(WkbOutcome {
        bytes: buf,
        bbox: bbox.bounds,
        dropped_rings: drops.rings,
        dropped_surfaces: drops.surfaces,
    }))
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
        let outcome = geometry_to_wkb(&geom, &pool).unwrap().unwrap();
        let (bytes, bbox) = (outcome.bytes, outcome.bbox);
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
    fn ring_with_fewer_than_3_vertices_is_dropped_not_an_error() {
        // Policy change (task 3b): a ring that cannot form a valid WKB ring
        // is dropped rather than failing the whole geometry. Here it is the
        // only surface's exterior ring, so nothing is left to write.
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
        assert!(
            geometry_to_wkb(&geom, &pool).unwrap().is_none(),
            "a geometry reduced to nothing must emit no WKB, not an error"
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
        let outcome = geometry_to_wkb(&geom, &pool).unwrap().unwrap();
        let bytes = outcome.bytes;
        let num_points = u32::from_le_bytes(bytes[18..22].try_into().unwrap());
        assert_eq!(
            num_points, 4,
            "pre-closed 4-index ring must yield 4 WKB points, not 5"
        );
        assert_eq!(
            outcome.dropped_rings, 0,
            "a pre-closed ring with 3 effective vertices is normalised, not dropped"
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
        let bytes = geometry_to_wkb(&geom, &pool).unwrap().unwrap().bytes;
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

    #[test]
    fn degenerate_ring_drops_with_its_surface() {
        // Surface 0's exterior ring is the structural [a,b,a] closure shape
        // (2 effective vertices): the ring is dropped, and with it the whole
        // surface. Surface 1 is fine and must survive as the ONLY polygon.
        let (v, t) = pool_and(1.0);
        let pool = VertexPool::new(&v, &t);
        let geom = cjseq::Geometry {
            thetype: cjseq::GeometryType::MultiSurface,
            lod: Some("2".into()),
            boundaries: serde_json::json!([[[0, 1, 0]], [[0, 1, 2, 3]]]),
            semantics: None,
            material: None,
            texture: None,
            template: None,
            transformation_matrix: None,
        };
        let outcome = geometry_to_wkb(&geom, &pool).unwrap().unwrap();
        assert_eq!(outcome.dropped_rings, 1);
        assert_eq!(outcome.dropped_surfaces, vec![0]);
        assert_eq!(
            u32::from_le_bytes(outcome.bytes[5..9].try_into().unwrap()),
            1,
            "WKB must contain exactly ONE polygon (the surviving surface)"
        );
        // The writer's output must satisfy the hardened reader by construction.
        let decoded = crate::wkb_read::wkb_to_geometry(&outcome.bytes)
            .expect("hardened reader must accept the writer's output");
        let crate::wkb_read::DecodedKind::MultiPolygon(surfaces) = &decoded.kind else {
            panic!("expected MultiPolygon, got {:?}", decoded.kind);
        };
        assert_eq!(surfaces.len(), 1);
        assert_eq!(
            surfaces[0][0].len(),
            4,
            "surviving ring keeps its 4 vertices"
        );

        // Counter precision: a dropped surface's interior degenerate rings
        // are still counted in dropped_rings (surface drop unchanged).
        let geom = cjseq::Geometry {
            thetype: cjseq::GeometryType::MultiSurface,
            lod: Some("2".into()),
            boundaries: serde_json::json!([[[0, 1, 0], [2, 3, 2]], [[0, 1, 2, 3]]]),
            semantics: None,
            material: None,
            texture: None,
            template: None,
            transformation_matrix: None,
        };
        let outcome = geometry_to_wkb(&geom, &pool).unwrap().unwrap();
        assert_eq!(
            outcome.dropped_rings, 2,
            "degenerate exterior AND degenerate interior must both be counted"
        );
        assert_eq!(outcome.dropped_surfaces, vec![0]);
        assert_eq!(
            u32::from_le_bytes(outcome.bytes[5..9].try_into().unwrap()),
            1,
            "the surviving surface is still the only polygon"
        );
    }

    #[test]
    fn zero_area_ring_with_3_effective_vertices_passes_through() {
        // Policy is narrow and structural: only the [a,b,a] closure shape
        // drops. A ring of 3 distinct indices passes through even if it is
        // geometrically degenerate — data quality is not the format's
        // business.
        let (v, t) = pool_and(1.0);
        let pool = VertexPool::new(&v, &t);
        let geom = cjseq::Geometry {
            thetype: cjseq::GeometryType::MultiSurface,
            lod: Some("2".into()),
            boundaries: serde_json::json!([[[0, 1, 2]]]),
            semantics: None,
            material: None,
            texture: None,
            template: None,
            transformation_matrix: None,
        };
        let outcome = geometry_to_wkb(&geom, &pool).unwrap().unwrap();
        assert_eq!(outcome.dropped_rings, 0);
        assert!(outcome.dropped_surfaces.is_empty());
        assert_eq!(
            u32::from_le_bytes(outcome.bytes[5..9].try_into().unwrap()),
            1
        );
    }

    #[test]
    fn all_surfaces_dropped_yields_no_wkb() {
        // Every surface degenerate -> nothing to write; reported the same
        // way as empty boundaries (None). Drop info is not carried in this
        // corner (there is no WKB value to attach it to).
        let (v, t) = pool_and(1.0);
        let pool = VertexPool::new(&v, &t);
        let geom = cjseq::Geometry {
            thetype: cjseq::GeometryType::MultiSurface,
            lod: Some("2".into()),
            boundaries: serde_json::json!([[[0, 1, 0]]]),
            semantics: None,
            material: None,
            texture: None,
            template: None,
            transformation_matrix: None,
        };
        assert!(geometry_to_wkb(&geom, &pool).unwrap().is_none());
    }

    #[test]
    fn normalise_ring_strips_trailing_duplicates_to_fixpoint() {
        // The exact Codex ring: [0, 1, 0, 0]. One strip leaves [0, 1, 0], which
        // still closes (first == last); the fixpoint strip must continue until
        // [0, 1] and then drop the ring (< 3 effective vertices).
        assert_eq!(normalise_ring(&[0, 1, 0, 0]), None);
        // Still only trailing-closure strips: a healthy pre-closed ring keeps
        // its body, and an unclosed ring is untouched.
        assert_eq!(normalise_ring(&[0, 1, 2, 0]), Some(&[0usize, 1, 2][..]));
        assert_eq!(normalise_ring(&[0, 1, 2]), Some(&[0usize, 1, 2][..]));
    }

    #[test]
    fn zero_ring_surface_is_dropped_and_counted() {
        let mut drops = Drops::default();
        let empty: [Vec<usize>; 0] = [];
        assert_eq!(normalise_surface(&empty, 4, &mut drops), None);
        assert_eq!(drops.surfaces, vec![4]);
        assert_eq!(drops.rings, 0); // no ring existed to count
    }

    #[test]
    fn vertex_beyond_2_53_is_rejected() {
        // Quantised component 2^53 + 1 is not representable in f64: silent
        // precision loss, so the writer must refuse it.
        let vertices = vec![vec![9_007_199_254_740_993_i64, 0, 0]];
        let transform: Transform = serde_json::from_value(serde_json::json!({
            "scale": [1.0, 1.0, 1.0], "translate": [0.0, 0.0, 0.0]
        }))
        .unwrap();
        let pool = VertexPool::new(&vertices, &transform);
        assert!(matches!(pool.coord(0), Err(CityParquetError::Geometry(_))));
    }

    #[test]
    fn large_translate_cancellation_is_rejected_not_silently_lossy() {
        // v = -(2^53 + 8): converting v to f64 already loses bits BEFORE the
        // translate cancels it back into small-magnitude territory — the final
        // coordinate looks harmless but is wrong. The guard must fire on the
        // quantised component, not only on the final value.
        let vertices = vec![vec![-9_007_199_254_741_000_i64, 0, 0]];
        let transform: Transform = serde_json::from_value(serde_json::json!({
            "scale": [1.0, 1.0, 1.0], "translate": [9_007_199_254_741_000.0, 0.0, 0.0]
        }))
        .unwrap();
        let pool = VertexPool::new(&vertices, &transform);
        assert!(matches!(pool.coord(0), Err(CityParquetError::Geometry(_))));
    }

    /// `VertexPool::raw` over a geometry template's real `vertices-templates`
    /// (railway fixture): coordinates must come back bitwise-exact (no
    /// scale/translate applied), and a template geometry looked up through it
    /// must encode/decode exactly like a regular geometry.
    #[test]
    fn raw_pool_over_template_vertices_round_trips() {
        let fixture_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures")
            .join("lod3_railway.city.json");
        assert!(
            fixture_path.exists(),
            "missing fixture lod3_railway.city.json; run `just fixtures`"
        );
        let raw_text = std::fs::read_to_string(&fixture_path).unwrap();
        let doc = cjseq::CityJSON::from_str(&raw_text).unwrap();
        let templates = doc
            .geometry_templates
            .as_ref()
            .expect("railway has geometry-templates");
        let verts: Vec<Vec<f64>> =
            serde_json::from_value(templates.vertices_templates.clone()).unwrap();
        assert_eq!(verts.len(), 338, "railway has 338 template vertices");
        assert_eq!(
            verts[0],
            vec![0.112, 0.121, 0.502],
            "first template vertex, straight from the fixture"
        );

        let pool = VertexPool::raw(&verts);
        assert_eq!(
            pool.coord(0).unwrap(),
            [0.112, 0.121, 0.502],
            "raw pool must return the float verbatim, bitwise-exact"
        );

        let tpl0 = &templates.templates[0];
        let outcome = geometry_to_wkb(tpl0, &pool)
            .unwrap()
            .expect("template 0's geometry must encode to WKB");
        let decoded = crate::wkb_read::wkb_to_geometry(&outcome.bytes)
            .expect("hardened reader must accept the raw-pool writer's output");
        let crate::wkb_read::DecodedKind::MultiPolygon(surfaces) = &decoded.kind else {
            panic!("expected MultiPolygon, got {:?}", decoded.kind);
        };
        // Template 0's first surface has a 16-index exterior ring: the
        // writer closes it (17 WKB points on the wire), and the hardened
        // reader strips the closing vertex back off on decode, so 16 indices
        // round-trip.
        assert_eq!(
            surfaces[0][0].len(),
            16,
            "first face's ring must round-trip its source coordinate count"
        );
    }
}
