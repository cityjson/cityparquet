//! `cjseq::Geometry` -> compacted `DecodedGeometry` for the arrow-native
//! encoding (design doc "Approaches considered", Option B). Reuses
//! `wkb_write`'s ring/shell normalisation so degenerate-geometry handling
//! matches the WKB path exactly; differs only in the target shape (indexed
//! `DecodedGeometry` instead of WKB bytes) and in how the vertex pool is
//! built: **distinct-source-index compaction**, never coordinate-value
//! dedup (design doc round-2 correction — two different source indices
//! with identical coordinates stay two separate pool entries).
//!
//! Nothing in this crate calls [`geometry_to_compacted`] yet — Task 6 wires
//! it into `encode.rs` wherever it currently calls
//! `wkb_write::geometry_to_wkb`. Until then the module is otherwise inert,
//! hence the blanket `dead_code` allow below.

#![allow(dead_code)]

use std::collections::HashMap;

use cityparquet_schema::{CityParquetError, Result};
use cjseq::{Geometry, GeometryType};

use crate::wkb_read::{DecodedGeometry, DecodedKind};
use crate::wkb_write::{Drops, VertexPool, boundaries, normalise_shells, normalise_surface};

/// Per-geometry, distinct-source-index vertex-pool compactor. Maps each
/// FIRST-SEEN raw source index to a dense local index and remembers its
/// dereferenced coordinate; a repeat occurrence of the SAME raw index reuses
/// its local index. Two different raw indices are never merged even if
/// bitwise-identical coordinates (design doc round-2 correction).
struct Compactor<'a, 'p> {
    pool: &'a VertexPool<'p>,
    seen: HashMap<usize, usize>,
    coords: Vec<[f64; 3]>,
}

impl<'a, 'p> Compactor<'a, 'p> {
    fn new(pool: &'a VertexPool<'p>) -> Self {
        Self {
            pool,
            seen: HashMap::new(),
            coords: Vec::new(),
        }
    }

    fn local_index(&mut self, raw: usize) -> Result<usize> {
        if let Some(&local) = self.seen.get(&raw) {
            return Ok(local);
        }
        let local = self.coords.len();
        self.coords.push(self.pool.coord(raw)?);
        self.seen.insert(raw, local);
        Ok(local)
    }

    fn ring(&mut self, ring: &[usize]) -> Result<Vec<usize>> {
        ring.iter().map(|&raw| self.local_index(raw)).collect()
    }

    fn surface(&mut self, rings: &[&[usize]]) -> Result<Vec<Vec<usize>>> {
        rings.iter().map(|r| self.ring(r)).collect()
    }
}

/// `cjseq::Geometry` -> `Option<DecodedGeometry>`, phase-1 types only
/// (`MultiSurface`/`CompositeSurface`/`Solid`/`MultiSolid`/`CompositeSolid`
/// — design doc "Type coverage (v1)"). Mirrors `wkb_write::geometry_to_wkb`'s
/// dispatch and degenerate-ring/-surface handling exactly (same `Drops`
/// tracking, same `normalise_surface`/`normalise_shells` calls) — differs
/// only in the output shape. Returns `Ok(None)` for `GeometryInstance`
/// (no geometry cell, same as WKB) and for an empty/fully-degenerate result
/// (same "no coordinates written" rule as `wkb_write`).
pub(crate) fn geometry_to_compacted(
    geom: &Geometry,
    pool: &VertexPool,
) -> Result<Option<DecodedGeometry>> {
    let mut drops = Drops::default();
    let mut c = Compactor::new(pool);
    let kind = match geom.thetype {
        GeometryType::GeometryInstance => return Ok(None),
        GeometryType::MultiPoint | GeometryType::MultiLineString => {
            return Err(CityParquetError::Geometry(format!(
                "{:?} is not supported by the arrow-native encoding in phase 1 \
                 (design doc \"Type coverage (v1)\") — use --geometry-encoding wkb for this source",
                geom.thetype
            )));
        }
        GeometryType::MultiSurface | GeometryType::CompositeSurface => {
            let surfaces: Vec<Vec<Vec<usize>>> = boundaries(geom)?;
            let kept: Vec<Vec<&[usize]>> = surfaces
                .iter()
                .enumerate()
                .filter_map(|(pos, s)| normalise_surface(s, pos, &mut drops))
                .collect();
            let mut out = Vec::with_capacity(kept.len());
            for surface in &kept {
                out.push(c.surface(surface)?);
            }
            DecodedKind::MultiPolygon(out)
        }
        GeometryType::Solid => {
            let shells: Vec<Vec<Vec<Vec<usize>>>> = boundaries(geom)?;
            let mut pos = 0;
            let kept = normalise_shells(&shells, &mut pos, &mut drops);
            let mut out = Vec::with_capacity(kept.len());
            for face in &kept {
                out.push(c.surface(face)?);
            }
            DecodedKind::PolyhedralSurface(out)
        }
        GeometryType::MultiSolid | GeometryType::CompositeSolid => {
            let solids: Vec<Vec<Vec<Vec<Vec<usize>>>>> = boundaries(geom)?;
            let mut pos = 0;
            let mut members = Vec::with_capacity(solids.len());
            for solid in &solids {
                let kept = normalise_shells(solid, &mut pos, &mut drops);
                let mut out = Vec::with_capacity(kept.len());
                for face in &kept {
                    out.push(c.surface(face)?);
                }
                members.push(DecodedKind::PolyhedralSurface(out));
            }
            DecodedKind::GeometryCollection(members)
        }
    };
    if c.coords.is_empty() {
        return Ok(None);
    }
    Ok(Some(DecodedGeometry {
        coords: c.coords,
        kind,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transform_identity() -> cjseq::Transform {
        cjseq::Transform {
            scale: vec![1.0, 1.0, 1.0],
            translate: vec![0.0, 0.0, 0.0],
        }
    }

    fn multisurface_geom(boundaries: serde_json::Value) -> Geometry {
        Geometry {
            thetype: GeometryType::MultiSurface,
            lod: Some("2".to_string()),
            boundaries,
            semantics: None,
            material: None,
            texture: None,
            template: None,
            transformation_matrix: None,
        }
    }

    #[test]
    fn multisurface_two_triangles_sharing_an_edge_compacts_the_shared_pair() {
        // Two triangles sharing edge (1,2): 4 distinct vertices total, not 6.
        let vertices: Vec<Vec<i64>> =
            vec![vec![0, 0, 0], vec![1, 0, 0], vec![1, 1, 0], vec![0, 1, 0]];
        let pool = VertexPool::new(&vertices, &transform_identity());
        let geom = multisurface_geom(serde_json::json!([[[0, 1, 2]], [[0, 2, 3]]]));
        let decoded = geometry_to_compacted(&geom, &pool).unwrap().unwrap();
        assert_eq!(
            decoded.coords.len(),
            4,
            "shared indices 0 and 2 must be compacted, not duplicated"
        );
        match &decoded.kind {
            DecodedKind::MultiPolygon(surfaces) => {
                assert_eq!(surfaces.len(), 2);
                assert_eq!(surfaces[0], vec![vec![0, 1, 2]]);
                assert_eq!(surfaces[1], vec![vec![0, 2, 3]]);
            }
            other => panic!("expected MultiPolygon, got {other:?}"),
        }
    }

    #[test]
    fn distinct_indices_with_equal_coordinates_are_never_merged() {
        // Two source indices, SAME coordinate value — must stay two pool entries.
        let vertices: Vec<Vec<i64>> = vec![vec![0, 0, 0], vec![0, 0, 0], vec![1, 0, 0]];
        let pool = VertexPool::new(&vertices, &transform_identity());
        let geom = multisurface_geom(serde_json::json!([[[0, 1, 2]]]));
        let decoded = geometry_to_compacted(&geom, &pool).unwrap().unwrap();
        assert_eq!(
            decoded.coords.len(),
            3,
            "indices 0 and 1 have identical coordinates but are DISTINCT source vertices \
             (design doc: index-identity compaction, not coordinate-value dedup)"
        );
    }

    #[test]
    fn solid_two_shells_flattens_faces_like_wkb_and_reports_no_shell_distinction() {
        // A minimal 2-shell Solid: exterior (1 face, a triangle) + one interior
        // cavity face (also a triangle) sharing no vertices with the exterior.
        let vertices: Vec<Vec<i64>> = vec![
            vec![0, 0, 0],
            vec![1, 0, 0],
            vec![0, 1, 0],
            vec![0, 0, 1],
            vec![1, 0, 1],
            vec![0, 1, 1],
        ];
        let pool = VertexPool::new(&vertices, &transform_identity());
        let geom = Geometry {
            thetype: GeometryType::Solid,
            lod: Some("2".to_string()),
            boundaries: serde_json::json!([[[[0, 1, 2]]], [[[3, 4, 5]]]]),
            semantics: None,
            material: None,
            texture: None,
            template: None,
            transformation_matrix: None,
        };
        let decoded = geometry_to_compacted(&geom, &pool).unwrap().unwrap();
        assert_eq!(decoded.coords.len(), 6);
        match &decoded.kind {
            // Flattened to 2 faces, shell boundary NOT represented here —
            // exactly mirroring wkb_write::geometry_to_wkb's PolyhedralSurfaceZ
            // output (shell structure lives only in geometry_properties.shells,
            // unchanged, design doc "Face traversal order").
            DecodedKind::PolyhedralSurface(faces) => assert_eq!(faces.len(), 2),
            other => panic!("expected PolyhedralSurface, got {other:?}"),
        }
    }
}
