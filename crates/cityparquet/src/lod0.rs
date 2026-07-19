//! LoD0 footprint synthesis (§9 "LoD0 synthesis").
//!
//! Given the higher-LoD boundary geometry of a city object, derive a 2.5D
//! **footprint** — the ground-contact polygon(s) — to populate the primary,
//! GeoParquet-legal `geometry` column when the source carries no LoD0. The
//! entry point [`synthesize_lod0`] is **semantics-first** (uses `GroundSurface`
//! faces when present) with a **purely geometric** fallback (downward-facing,
//! region-grown, 2D-unioned, Z-re-draped); it returns `None` rather than
//! fabricate a footprint when no acceptable ground is found.
//!
//! The module operates on plain owned geometry (`Face` of `[f64; 3]` rings), so
//! the core is a pure, deterministic, heavily unit-tested function decoupled
//! from CityJSON/WKB; thin adapters bridge `cjseq::Geometry` at the edges.

/// A 3D point `[x, y, z]` in world coordinates (metres, projected CRS).
pub type Point = [f64; 3];

/// One planar polygonal face: `rings[0]` is the exterior ring, `rings[1..]` are
/// interior rings (holes). Rings are **open** (the first vertex is not
/// repeated at the end), matching CityJSON boundary conventions.
#[derive(Debug, Clone, PartialEq)]
pub struct Face {
    pub rings: Vec<Vec<Point>>,
}

impl Face {
    /// A single-ring face from an exterior ring.
    pub fn from_exterior(ring: Vec<Point>) -> Self {
        Face { rings: vec![ring] }
    }

    /// The exterior ring (`rings[0]`), or an empty slice for a malformed face.
    pub fn exterior(&self) -> &[Point] {
        self.rings.first().map(Vec::as_slice).unwrap_or(&[])
    }
}

/// How a synthesised footprint was derived — for provenance/statistics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lod0Source {
    /// Taken from faces the source labelled `GroundSurface`.
    GroundSemantics,
    /// Derived by the geometric downward-face heuristic.
    Geometric,
}

/// A synthesised LoD0 footprint: ground surfaces assembled into a valid,
/// GeoParquet-legal set of polygons (exterior CCW, interiors CW).
#[derive(Debug, Clone, PartialEq)]
pub struct Footprint {
    pub surfaces: Vec<Face>,
    pub source: Lod0Source,
}

/// Optional post-processing of the draped footprint Z.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlattenMode {
    /// Flatten every vertex to the 5th-percentile ground Z (a robust minimum).
    Percentile5,
}

/// Last-resort footprint when no true ground surface is found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fallback {
    /// Union the 2D projection of *all* faces (a roofprint silhouette, not a
    /// footprint) — must be flagged by the caller; off by default.
    ProjectedSilhouette,
}

/// Tunable thresholds for [`synthesize_lod0`]. All lengths are absolute metres
/// (city models are in a projected metric CRS); defaults per §9 / the design doc.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Lod0Options {
    /// Downward-cone half-angle in degrees: a face counts as ground when the
    /// angle between its normal and `-Z` is `<= theta_deg`.
    pub theta_deg: f64,
    /// Lowest-vertex seed band (m): seed faces incident to a vertex within
    /// `eps_z` of the object's minimum Z.
    pub eps_z: f64,
    /// Disconnected-component acceptance band (m): accept a further ground
    /// component whose min Z is within `h_step` of accepted ground (rejects
    /// balcony soffits ~one storey up).
    pub h_step: f64,
    /// Planarity hard-reject (m): drop a face whose max point-to-plane
    /// deviation exceeds this.
    pub plane_reject: f64,
    /// Pre-union snap grid (m).
    pub snap: f64,
    /// Optional Z flattening; `None` keeps each vertex's draped ground Z.
    pub flatten: Option<FlattenMode>,
    /// Optional last-resort fallback; `None` means return `None` when no ground.
    pub fallback: Option<Fallback>,
}

impl Default for Lod0Options {
    fn default() -> Self {
        Lod0Options {
            theta_deg: 20.0,
            eps_z: 0.01,
            h_step: 1.5,
            plane_reject: 0.5,
            snap: 0.001,
            flatten: None,
            fallback: None,
        }
    }
}

/// Newell's method for a (possibly slightly non-planar) ring's normal, returned
/// **un-normalised**. `None` when the ring has fewer than 3 vertices or is
/// degenerate (near-zero area), so callers can drop it. Robust for real data —
/// never use a first-three-vertices cross product, which a collinear triple
/// makes zero.
pub(crate) fn newell_normal(ring: &[Point]) -> Option<[f64; 3]> {
    if ring.len() < 3 {
        return None;
    }
    let mut n = [0.0f64; 3];
    for i in 0..ring.len() {
        let a = ring[i];
        let b = ring[(i + 1) % ring.len()];
        n[0] += (a[1] - b[1]) * (a[2] + b[2]);
        n[1] += (a[2] - b[2]) * (a[0] + b[0]);
        n[2] += (a[0] - b[0]) * (a[1] + b[1]);
    }
    let mag = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    if mag < 1e-12 {
        return None;
    }
    Some(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newell_normal_of_ccw_horizontal_ring_points_up() {
        // A CCW square in the z = 5 plane, viewed from +Z.
        let ring = vec![[0., 0., 5.], [10., 0., 5.], [10., 10., 5.], [0., 10., 5.]];
        let n = newell_normal(&ring).unwrap();
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        assert!((n[0] / len).abs() < 1e-9);
        assert!((n[1] / len).abs() < 1e-9);
        assert!((n[2] / len - 1.0).abs() < 1e-9, "normal should be +Z");
    }

    #[test]
    fn newell_normal_of_cw_horizontal_ring_points_down() {
        // The same square wound CW points down (a ground face's winding).
        let ring = vec![[0., 0., 5.], [0., 10., 5.], [10., 10., 5.], [10., 0., 5.]];
        let n = newell_normal(&ring).unwrap();
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        assert!((n[2] / len + 1.0).abs() < 1e-9, "normal should be -Z");
    }

    #[test]
    fn newell_normal_of_degenerate_ring_is_none() {
        // Collinear triple: zero area, no normal.
        assert!(newell_normal(&[[0., 0., 0.], [1., 1., 1.], [2., 2., 2.]]).is_none());
        // Too few vertices.
        assert!(newell_normal(&[[0., 0., 0.], [1., 0., 0.]]).is_none());
    }

    #[test]
    fn options_default_matches_spec() {
        let o = Lod0Options::default();
        assert_eq!(o.theta_deg, 20.0);
        assert_eq!(o.eps_z, 0.01);
        assert_eq!(o.h_step, 1.5);
        assert_eq!(o.plane_reject, 0.5);
        assert_eq!(o.snap, 0.001);
        assert!(o.flatten.is_none());
        assert!(o.fallback.is_none());
    }
}
