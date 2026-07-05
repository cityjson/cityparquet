//! Minimal ISO-WKB reader for the CityJSON → CityParquet geometry mapping.
//! Inverse of [`crate::wkb_write`]. Little-endian only. Coordinates are
//! deduplicated (bitwise, via `f64::to_bits`) into a single shared pool per
//! decoded geometry, and container members hold indices into that pool.

use std::collections::HashMap;

use cityparquet_schema::{CityParquetError, Result};

const POINT_Z: u32 = 1001;
const LINESTRING_Z: u32 = 1002;
const POLYGON_Z: u32 = 1003;
const MULTIPOINT_Z: u32 = 1004;
const MULTILINESTRING_Z: u32 = 1005;
const MULTIPOLYGON_Z: u32 = 1006;
const GEOMETRYCOLLECTION_Z: u32 = 1007;
const POLYHEDRALSURFACE_Z: u32 = 1015;

/// Maximum GeometryCollection nesting depth. Our writer only ever emits one
/// level (MultiSolid → collection of PolyhedralSurface), but this reader
/// will also see files we did not write: cap recursion instead of letting a
/// hostile buffer overflow the stack.
const MAX_DEPTH: usize = 16;

/// A decoded WKB geometry: a shared, deduplicated coordinate pool plus a
/// `kind` whose indices point into it.
#[derive(Debug, Clone, PartialEq)]
pub struct DecodedGeometry {
    pub coords: Vec<[f64; 3]>,
    pub kind: DecodedKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DecodedKind {
    MultiPoint(Vec<usize>),
    MultiLineString(Vec<Vec<usize>>),
    /// surfaces -> rings -> coord indices
    MultiPolygon(Vec<Vec<Vec<usize>>>),
    /// faces -> rings -> coord indices (flattened; shell grouping lives in
    /// `geometry_properties`, not in WKB — mirrors `wkb_write::polyhedral`)
    PolyhedralSurface(Vec<Vec<Vec<usize>>>),
    /// members share the SAME coords pool as their parent
    GeometryCollection(Vec<DecodedKind>),
}

fn geometry_err(msg: impl Into<String>) -> CityParquetError {
    CityParquetError::Geometry(msg.into())
}

/// Deduplicates 3D coordinates by bitwise (`f64::to_bits`) equality into a
/// single pool, handing back the index each coordinate was interned at.
#[derive(Default)]
struct CoordInterner {
    index: HashMap<[u64; 3], usize>,
    coords: Vec<[f64; 3]>,
}

impl CoordInterner {
    fn intern(&mut self, c: [f64; 3]) -> usize {
        let key = [c[0].to_bits(), c[1].to_bits(), c[2].to_bits()];
        *self.index.entry(key).or_insert_with(|| {
            let idx = self.coords.len();
            self.coords.push(c);
            idx
        })
    }
}

/// Bounds-checked cursor over a WKB byte buffer.
struct Cursor<'a> {
    buf: &'a [u8],
    pos: usize,
    interner: CoordInterner,
}

impl<'a> Cursor<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self {
            buf,
            pos: 0,
            interner: CoordInterner::default(),
        }
    }

    fn truncated(&self, what: &str) -> CityParquetError {
        geometry_err(format!(
            "truncated WKB: expected {what} at offset {} (buffer has {} bytes)",
            self.pos,
            self.buf.len()
        ))
    }

    fn read_u8(&mut self) -> Result<u8> {
        let b = *self
            .buf
            .get(self.pos)
            .ok_or_else(|| self.truncated("1 byte"))?;
        self.pos += 1;
        Ok(b)
    }

    fn read_u32(&mut self) -> Result<u32> {
        let end = self.pos + 4;
        let bytes = self
            .buf
            .get(self.pos..end)
            .ok_or_else(|| self.truncated("4 bytes (u32)"))?;
        self.pos = end;
        Ok(u32::from_le_bytes(
            bytes.try_into().expect("slice is 4 bytes"),
        ))
    }

    fn read_f64(&mut self) -> Result<f64> {
        let end = self.pos + 8;
        let bytes = self
            .buf
            .get(self.pos..end)
            .ok_or_else(|| self.truncated("8 bytes (f64)"))?;
        self.pos = end;
        Ok(f64::from_le_bytes(
            bytes.try_into().expect("slice is 8 bytes"),
        ))
    }

    /// Byte-order marker + type code. Only the little-endian marker (`0x01`)
    /// is supported; anything else (including the big-endian `0x00`) errors.
    fn read_header(&mut self) -> Result<u32> {
        let byte_order = self.read_u8()?;
        if byte_order != 0x01 {
            return Err(geometry_err(format!(
                "unsupported WKB byte order marker {byte_order:#04x}: only little-endian (0x01) is supported"
            )));
        }
        self.read_u32()
    }

    fn read_raw_coord(&mut self) -> Result<[f64; 3]> {
        Ok([self.read_f64()?, self.read_f64()?, self.read_f64()?])
    }

    /// Reads a raw XYZ triple and interns it, returning its pool index.
    fn read_coord_index(&mut self) -> Result<usize> {
        let c = self.read_raw_coord()?;
        Ok(self.interner.intern(c))
    }

    fn expect_type(&self, type_code: u32, expected: u32, what: &str) -> Result<()> {
        if type_code != expected {
            return Err(geometry_err(format!(
                "expected {what} (type {expected}), got type code {type_code}"
            )));
        }
        Ok(())
    }

    /// Full nested PointZ member: header + XYZ, interned.
    fn parse_point_member(&mut self) -> Result<usize> {
        let tc = self.read_header()?;
        self.expect_type(tc, POINT_Z, "PointZ")?;
        self.read_coord_index()
    }

    /// Full nested LineStringZ member: header + numPoints + points.
    fn parse_linestring_member(&mut self) -> Result<Vec<usize>> {
        let tc = self.read_header()?;
        self.expect_type(tc, LINESTRING_Z, "LineStringZ")?;
        let n = self.read_u32()? as usize;
        let mut pts = Vec::new();
        for _ in 0..n {
            pts.push(self.read_coord_index()?);
        }
        Ok(pts)
    }

    /// PolygonZ body (no header — caller already consumed it): numRings +
    /// each ring's numPoints + points. The WKB ring-closing vertex (WKB
    /// rings repeat their first coordinate as the last) is validated —
    /// bitwise, via the interner: a closing vertex only shares the first
    /// point's pool index if their bits match — then stripped. A ring that
    /// is not closed, or that has fewer than 3 points once stripped, is
    /// malformed.
    fn parse_polygon_body(&mut self) -> Result<Vec<Vec<usize>>> {
        let n_rings = self.read_u32()? as usize;
        let mut rings = Vec::new();
        for _ in 0..n_rings {
            let n_points = self.read_u32()? as usize;
            if n_points == 0 {
                return Err(geometry_err("polygon ring has zero points"));
            }
            let mut pts = Vec::new();
            for _ in 0..n_points {
                pts.push(self.read_coord_index()?);
            }
            if pts.last() != pts.first() {
                return Err(geometry_err(format!(
                    "unclosed WKB ring: last of {n_points} points does not repeat the first"
                )));
            }
            pts.pop(); // strip the WKB ring-closing vertex
            if pts.len() < 3 {
                return Err(geometry_err(format!(
                    "polygon ring has {} points after stripping the closing vertex, need at least 3",
                    pts.len()
                )));
            }
            rings.push(pts);
        }
        Ok(rings)
    }

    /// Full nested PolygonZ member: header + body.
    fn parse_polygon_member(&mut self) -> Result<Vec<Vec<usize>>> {
        let tc = self.read_header()?;
        self.expect_type(tc, POLYGON_Z, "PolygonZ")?;
        self.parse_polygon_body()
    }

    /// Body shared by MultiPolygonZ and PolyhedralSurfaceZ: numPolygons/
    /// numFaces followed by that many full nested PolygonZ members.
    fn parse_polygon_list_body(&mut self) -> Result<Vec<Vec<Vec<usize>>>> {
        let n = self.read_u32()? as usize;
        let mut polys = Vec::new();
        for _ in 0..n {
            polys.push(self.parse_polygon_member()?);
        }
        Ok(polys)
    }

    /// Dispatches on a just-read type code to the matching body parser.
    /// Used both for the top-level geometry (depth 0) and for
    /// GeometryCollection members (which are themselves complete nested WKB
    /// geometries, one depth level down). Recursion is capped at
    /// [`MAX_DEPTH`] so a hostile buffer cannot overflow the stack.
    fn parse_body(&mut self, type_code: u32, depth: usize) -> Result<DecodedKind> {
        if depth > MAX_DEPTH {
            return Err(geometry_err(format!(
                "WKB geometry nesting exceeds the maximum depth of {MAX_DEPTH}"
            )));
        }
        match type_code {
            MULTIPOINT_Z => {
                let n = self.read_u32()? as usize;
                let mut idxs = Vec::new();
                for _ in 0..n {
                    idxs.push(self.parse_point_member()?);
                }
                Ok(DecodedKind::MultiPoint(idxs))
            }
            MULTILINESTRING_Z => {
                let n = self.read_u32()? as usize;
                let mut lines = Vec::new();
                for _ in 0..n {
                    lines.push(self.parse_linestring_member()?);
                }
                Ok(DecodedKind::MultiLineString(lines))
            }
            MULTIPOLYGON_Z => Ok(DecodedKind::MultiPolygon(self.parse_polygon_list_body()?)),
            POLYHEDRALSURFACE_Z => Ok(DecodedKind::PolyhedralSurface(
                self.parse_polygon_list_body()?,
            )),
            GEOMETRYCOLLECTION_Z => {
                let n = self.read_u32()? as usize;
                let mut members = Vec::new();
                for _ in 0..n {
                    let member_tc = self.read_header()?;
                    members.push(self.parse_body(member_tc, depth + 1)?);
                }
                Ok(DecodedKind::GeometryCollection(members))
            }
            other => Err(geometry_err(format!("unsupported WKB type code {other}"))),
        }
    }
}

/// Parses a complete WKB buffer (as produced by [`crate::wkb_write`]) into a
/// [`DecodedGeometry`] with a deduplicated coordinate pool. Little-endian
/// only; supports the container types the writer emits (MultiPoint,
/// MultiLineString, MultiPolygon, PolyhedralSurface, GeometryCollection of
/// those). Any other type code, a big-endian marker, or a truncated buffer
/// is a `CityParquetError::Geometry`.
pub fn wkb_to_geometry(bytes: &[u8]) -> Result<DecodedGeometry> {
    let mut cursor = Cursor::new(bytes);
    let type_code = cursor.read_header()?;
    let kind = cursor.parse_body(type_code, 0)?;
    if cursor.pos != bytes.len() {
        return Err(geometry_err(format!(
            "trailing bytes: {} bytes remain after a complete WKB geometry of {} bytes",
            bytes.len() - cursor.pos,
            cursor.pos
        )));
    }
    Ok(DecodedGeometry {
        coords: cursor.interner.coords,
        kind,
    })
}

/// Parses a standalone PointZ WKB buffer (as produced by
/// [`crate::wkb_write::point_to_wkb`]), for template points.
pub fn read_point(bytes: &[u8]) -> Result<[f64; 3]> {
    let mut cursor = Cursor::new(bytes);
    let tc = cursor.read_header()?;
    cursor.expect_type(tc, POINT_Z, "PointZ")?;
    cursor.read_raw_coord()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wkb_write::{VertexPool, geometry_to_wkb, point_to_wkb};

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

    fn geom(thetype: cjseq::GeometryType, boundaries: serde_json::Value) -> cjseq::Geometry {
        cjseq::Geometry {
            thetype,
            lod: Some("2".into()),
            boundaries,
            semantics: None,
            material: None,
            texture: None,
            template: None,
            transformation_matrix: None,
        }
    }

    #[test]
    fn multisurface_round_trips_with_closing_vertex_stripped() {
        let (v, t) = pool_and(1.0);
        let pool = VertexPool::new(&v, &t);
        let src = geom(
            cjseq::GeometryType::MultiSurface,
            serde_json::json!([[[0, 1, 2, 3]]]),
        );
        let bytes = geometry_to_wkb(&src, &pool).unwrap().unwrap().bytes;

        let decoded = wkb_to_geometry(&bytes).unwrap();
        let DecodedKind::MultiPolygon(surfaces) = &decoded.kind else {
            panic!("expected MultiPolygon, got {:?}", decoded.kind);
        };
        assert_eq!(surfaces.len(), 1);
        assert_eq!(surfaces[0].len(), 1);
        // source ring has 4 indices; the WKB closing vertex must be stripped
        // back off, not left as a 5th entry.
        let ring = &surfaces[0][0];
        assert_eq!(
            ring.len(),
            4,
            "ring vertex count must match the source, not the closed WKB encoding"
        );
        // 4 distinct source vertices (0,1,2,3) -> 4 pool entries.
        assert_eq!(decoded.coords.len(), 4);
        for (i, &src_idx) in [0usize, 1, 2, 3].iter().enumerate() {
            assert_eq!(decoded.coords[ring[i]], pool.coord(src_idx).unwrap());
        }
    }

    #[test]
    fn solid_round_trips_as_polyhedral_surface() {
        let (v, t) = pool_and(1.0);
        let pool = VertexPool::new(&v, &t);
        // one shell, two faces: a 4-vertex ring and a 3-vertex ring sharing
        // vertices 0 and 1.
        let src = geom(
            cjseq::GeometryType::Solid,
            serde_json::json!([[[[0, 1, 2, 3]], [[0, 1, 4]]]]),
        );
        let bytes = geometry_to_wkb(&src, &pool).unwrap().unwrap().bytes;

        let decoded = wkb_to_geometry(&bytes).unwrap();
        let DecodedKind::PolyhedralSurface(faces) = &decoded.kind else {
            panic!("expected PolyhedralSurface, got {:?}", decoded.kind);
        };
        assert_eq!(faces.len(), 2, "faces flattened across shells");
        assert_eq!(faces[0].len(), 1);
        assert_eq!(faces[0][0].len(), 4);
        assert_eq!(faces[1].len(), 1);
        assert_eq!(faces[1][0].len(), 3);
        // 5 distinct source vertices used (0,1,2,3,4) -> 5 pool entries,
        // shared vertices 0 and 1 must not be duplicated.
        assert_eq!(decoded.coords.len(), 5);
        assert_eq!(
            decoded.coords[faces[0][0][0]], decoded.coords[faces[1][0][0]],
            "vertex 0 shared between both faces must intern to the same coord"
        );
    }

    #[test]
    fn multipoint_and_multilinestring_round_trip() {
        let (v, t) = pool_and(1.0);
        let pool = VertexPool::new(&v, &t);

        let mp = geom(
            cjseq::GeometryType::MultiPoint,
            serde_json::json!([0, 1, 4]),
        );
        let bytes = geometry_to_wkb(&mp, &pool).unwrap().unwrap().bytes;
        let decoded = wkb_to_geometry(&bytes).unwrap();
        let DecodedKind::MultiPoint(idxs) = &decoded.kind else {
            panic!("expected MultiPoint, got {:?}", decoded.kind);
        };
        assert_eq!(idxs.len(), 3);
        assert_eq!(decoded.coords.len(), 3);

        let mls = geom(
            cjseq::GeometryType::MultiLineString,
            serde_json::json!([[0, 1, 2], [2, 3]]),
        );
        let bytes = geometry_to_wkb(&mls, &pool).unwrap().unwrap().bytes;
        let decoded = wkb_to_geometry(&bytes).unwrap();
        let DecodedKind::MultiLineString(lines) = &decoded.kind else {
            panic!("expected MultiLineString, got {:?}", decoded.kind);
        };
        // no ring-closing here: line lengths must match the source exactly.
        assert_eq!(lines.iter().map(Vec::len).collect::<Vec<_>>(), vec![3, 2]);
        // vertices 0,1,2,3 distinct, shared vertex 2 interned once -> 4 coords.
        assert_eq!(decoded.coords.len(), 4);
    }

    #[test]
    fn multisolid_becomes_geometry_collection_of_polyhedral_surfaces() {
        // Neither real fixture carries MultiSolid/CompositeSolid (per
        // wkb_real_data.rs), so this is hand-built to exercise the
        // GeometryCollection branch: 2 solids, each 1 shell / 1 face.
        let (v, t) = pool_and(1.0);
        let pool = VertexPool::new(&v, &t);
        let src = geom(
            cjseq::GeometryType::MultiSolid,
            serde_json::json!([[[[[0, 1, 2, 3]]]], [[[[0, 1, 4]]]]]),
        );
        let bytes = geometry_to_wkb(&src, &pool).unwrap().unwrap().bytes;

        let decoded = wkb_to_geometry(&bytes).unwrap();
        let DecodedKind::GeometryCollection(members) = &decoded.kind else {
            panic!("expected GeometryCollection, got {:?}", decoded.kind);
        };
        assert_eq!(members.len(), 2);
        for member in members {
            assert!(
                matches!(member, DecodedKind::PolyhedralSurface(_)),
                "each MultiSolid member must decode as PolyhedralSurface, got {member:?}"
            );
        }
        let DecodedKind::PolyhedralSurface(faces0) = &members[0] else {
            unreachable!()
        };
        assert_eq!(faces0.len(), 1);
        assert_eq!(faces0[0].len(), 1);
        assert_eq!(faces0[0][0].len(), 4);
        // shared vertices (0,1) across the two solids intern into the same
        // pool: 0,1,2,3,4 distinct -> 5 coords total.
        assert_eq!(decoded.coords.len(), 5);
    }

    #[test]
    fn truncated_buffer_errors() {
        let (v, t) = pool_and(1.0);
        let pool = VertexPool::new(&v, &t);
        let src = geom(
            cjseq::GeometryType::MultiSurface,
            serde_json::json!([[[0, 1, 2, 3]]]),
        );
        let bytes = geometry_to_wkb(&src, &pool).unwrap().unwrap().bytes;

        for cut in [0, 1, 5, bytes.len() - 1] {
            let err = wkb_to_geometry(&bytes[..cut]).unwrap_err();
            assert!(
                matches!(err, CityParquetError::Geometry(_)),
                "truncating to {cut} bytes should be a Geometry error, got {err:?}"
            );
        }
    }

    #[test]
    fn big_endian_marker_errors() {
        let (v, t) = pool_and(1.0);
        let pool = VertexPool::new(&v, &t);
        let src = geom(
            cjseq::GeometryType::MultiSurface,
            serde_json::json!([[[0, 1, 2, 3]]]),
        );
        let mut bytes = geometry_to_wkb(&src, &pool).unwrap().unwrap().bytes;
        bytes[0] = 0x00;
        let err = wkb_to_geometry(&bytes).unwrap_err();
        assert!(matches!(err, CityParquetError::Geometry(_)));
    }

    #[test]
    fn unknown_type_code_errors() {
        let (v, t) = pool_and(1.0);
        let pool = VertexPool::new(&v, &t);
        let src = geom(
            cjseq::GeometryType::MultiSurface,
            serde_json::json!([[[0, 1, 2, 3]]]),
        );
        let mut bytes = geometry_to_wkb(&src, &pool).unwrap().unwrap().bytes;
        bytes[1..5].copy_from_slice(&9999u32.to_le_bytes());
        let err = wkb_to_geometry(&bytes).unwrap_err();
        assert!(matches!(err, CityParquetError::Geometry(_)));
    }

    #[test]
    fn read_point_round_trips() {
        let bytes = point_to_wkb([1.0, 2.0, 3.0]);
        assert_eq!(read_point(&bytes).unwrap(), [1.0, 2.0, 3.0]);
    }

    /// A valid single-ring MultiSurface WKB buffer from the shared pool.
    fn valid_multisurface_bytes() -> Vec<u8> {
        let (v, t) = pool_and(1.0);
        let pool = VertexPool::new(&v, &t);
        let src = geom(
            cjseq::GeometryType::MultiSurface,
            serde_json::json!([[[0, 1, 2, 3]]]),
        );
        geometry_to_wkb(&src, &pool).unwrap().unwrap().bytes
    }

    /// A hand-built MultiPolygonZ buffer with one polygon, one ring made of
    /// the given raw coordinates (written verbatim, no auto-closing).
    fn multipolygon_with_ring(coords: &[[f64; 3]]) -> Vec<u8> {
        let mut buf = vec![0x01];
        buf.extend_from_slice(&1006u32.to_le_bytes()); // MultiPolygonZ
        buf.extend_from_slice(&1u32.to_le_bytes()); // numPolygons
        buf.push(0x01);
        buf.extend_from_slice(&1003u32.to_le_bytes()); // PolygonZ
        buf.extend_from_slice(&1u32.to_le_bytes()); // numRings
        buf.extend_from_slice(&(coords.len() as u32).to_le_bytes()); // numPoints
        for c in coords {
            for v in c {
                buf.extend_from_slice(&v.to_le_bytes());
            }
        }
        buf
    }

    #[test]
    fn unclosed_ring_errors_instead_of_silently_dropping_a_vertex() {
        // A ring whose last point does NOT bitwise-equal its first is not a
        // valid WKB ring: the reader must refuse it rather than strip a real
        // vertex. Corrupt the closing vertex of an otherwise valid buffer.
        let mut bytes = valid_multisurface_bytes();
        let n = bytes.len();
        bytes[n - 1] ^= 0x3F; // perturb the z of the closing vertex
        let err = wkb_to_geometry(&bytes).unwrap_err();
        assert!(
            matches!(err, CityParquetError::Geometry(_)),
            "unclosed ring must be a Geometry error, got {err:?}"
        );
        assert!(
            err.to_string().to_lowercase().contains("unclosed"),
            "error should name the unclosed ring, got: {err}"
        );
    }

    #[test]
    fn ring_with_fewer_than_3_points_after_stripping_errors() {
        // numPoints = 1: the lone point trivially "closes" onto itself and
        // stripping leaves 0 points.
        let a = [1.0, 2.0, 3.0];
        let err = wkb_to_geometry(&multipolygon_with_ring(&[a])).unwrap_err();
        assert!(
            matches!(err, CityParquetError::Geometry(_)),
            "1-point ring must be a Geometry error, got {err:?}"
        );

        // numPoints = 3 with last == first: closed, but stripping leaves only
        // 2 points — not a polygon ring.
        let b = [4.0, 5.0, 6.0];
        let err = wkb_to_geometry(&multipolygon_with_ring(&[a, b, a])).unwrap_err();
        assert!(
            matches!(err, CityParquetError::Geometry(_)),
            "2-point decoded ring must be a Geometry error, got {err:?}"
        );
    }

    #[test]
    fn trailing_bytes_after_a_complete_geometry_error() {
        let mut bytes = valid_multisurface_bytes();
        assert!(wkb_to_geometry(&bytes).is_ok(), "baseline must parse");
        bytes.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);
        let err = wkb_to_geometry(&bytes).unwrap_err();
        assert!(
            matches!(err, CityParquetError::Geometry(_)),
            "trailing bytes must be a Geometry error, got {err:?}"
        );
        assert!(
            err.to_string().to_lowercase().contains("trailing"),
            "error should name the trailing bytes, got: {err}"
        );
    }

    #[test]
    fn deeply_nested_geometry_collections_error_instead_of_overflowing() {
        // 20 nested GeometryCollectionZ headers, each declaring one member:
        // past the depth cap this must be a Geometry error naming the
        // nesting depth — not unbounded recursion (and never a truncation
        // error reached only after recursing all the way down).
        let mut bytes = Vec::new();
        for _ in 0..20 {
            bytes.push(0x01);
            bytes.extend_from_slice(&1007u32.to_le_bytes());
            bytes.extend_from_slice(&1u32.to_le_bytes());
        }
        let err = wkb_to_geometry(&bytes).unwrap_err();
        assert!(
            matches!(err, CityParquetError::Geometry(_)),
            "over-deep nesting must be a Geometry error, got {err:?}"
        );
        let msg = err.to_string().to_lowercase();
        assert!(
            msg.contains("depth") || msg.contains("nest"),
            "error should name the nesting depth, got: {err}"
        );
    }
}
