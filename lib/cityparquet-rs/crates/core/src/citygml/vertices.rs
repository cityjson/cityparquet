//! Per-feature vertex quantisation and de-duplication.
//!
//! CityGML carries inline float coordinates; CityJSON/cjseq wants a shared
//! integer vertex pool plus a `transform` (scale + translate). Each emitted
//! feature owns its own local pool (CityJSONSeq style), all dequantised by the
//! single header transform. This builder quantises `[f64; 3]` world coordinates
//! against that transform and returns a stable index per distinct vertex.

use std::collections::HashMap;

use cityparquet_schema::{CityParquetError, Result};

/// Accumulates a feature's distinct integer vertices and hands back indices.
pub struct VertexBuilder<'a> {
    scale: &'a [f64; 3],
    translate: &'a [f64; 3],
    map: HashMap<[i64; 3], usize>,
    verts: Vec<[i64; 3]>,
}

impl<'a> VertexBuilder<'a> {
    pub fn new(scale: &'a [f64; 3], translate: &'a [f64; 3]) -> Self {
        Self {
            scale,
            translate,
            map: HashMap::new(),
            verts: Vec::new(),
        }
    }

    /// Quantise a world coordinate and return its de-duplicated vertex index.
    pub fn push(&mut self, coord: [f64; 3]) -> Result<usize> {
        let mut q = [0i64; 3];
        for i in 0..3 {
            let c = coord[i];
            if !c.is_finite() {
                return Err(CityParquetError::Schema(format!(
                    "non-finite CityGML coordinate component: {c}"
                )));
            }
            let scaled = (c - self.translate[i]) / self.scale[i];
            let rounded = scaled.round();
            // f64 -> i64 saturates rather than wrapping, so guard the range
            // explicitly and report instead of silently clamping.
            if !rounded.is_finite() || rounded.abs() >= 9.223_372_036_854_775e18 {
                return Err(CityParquetError::Schema(format!(
                    "CityGML coordinate {c} out of range after quantisation"
                )));
            }
            q[i] = rounded as i64;
        }
        Ok(*self.map.entry(q).or_insert_with(|| {
            let idx = self.verts.len();
            self.verts.push(q);
            idx
        }))
    }

    /// The distinct vertices, in index order, as `Vec<Vec<i64>>` for a
    /// `CityJSONFeature.vertices`.
    pub fn into_vertices(self) -> Vec<Vec<i64>> {
        self.verts.into_iter().map(|v| v.to_vec()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantises_and_dedups() {
        let scale = [0.001, 0.001, 0.001];
        let translate = [0.0, 0.0, 0.0];
        let mut b = VertexBuilder::new(&scale, &translate);
        let a = b.push([50.0, 0.0, 150.0]).unwrap();
        let c = b.push([50.0, 0.0, 150.0]).unwrap(); // same -> same index
        let d = b.push([0.0, 0.0, 0.0]).unwrap();
        assert_eq!(a, c);
        assert_ne!(a, d);
        let verts = b.into_vertices();
        assert_eq!(verts.len(), 2);
        assert_eq!(verts[a], vec![50_000, 0, 150_000]);
    }

    #[test]
    fn rejects_non_finite() {
        let scale = [0.001, 0.001, 0.001];
        let translate = [0.0, 0.0, 0.0];
        let mut b = VertexBuilder::new(&scale, &translate);
        assert!(b.push([f64::NAN, 0.0, 0.0]).is_err());
    }
}
