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

/// The unit normal of a face's exterior ring (its winding decides direction);
/// `None` for a degenerate ring.
pub(crate) fn face_normal_unit(face: &Face) -> Option<[f64; 3]> {
    let n = newell_normal(face.exterior())?;
    let mag = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    Some([n[0] / mag, n[1] / mag, n[2] / mag])
}

/// Whether the face's (as-wound) normal points **downward** within a `theta_deg`
/// cone of `-Z` — the primary ground classifier (§9). Sign of the normal, not
/// height: on an outward-oriented solid this excludes flat roofs for free, so
/// `theta_deg` can be generous. A degenerate face is never downward.
pub(crate) fn is_downward(face: &Face, theta_deg: f64) -> bool {
    match face_normal_unit(face) {
        // angle(n, -Z) <= theta  <=>  -n_z >= cos(theta)  <=>  n_z <= -cos(theta)
        Some(n) => n[2] <= -theta_deg.to_radians().cos(),
        None => false,
    }
}

/// Signed volume enclosed by a set of faces via the divergence theorem
/// (fan-triangulated exterior rings, tetrahedra from the origin). For a closed
/// solid with **outward** normals this is positive; a negative result means the
/// shell is wound inward and its normals should be flipped before ground
/// classification.
pub(crate) fn signed_volume(faces: &[Face]) -> f64 {
    let mut v = 0.0;
    for face in faces {
        let ring = face.exterior();
        if ring.len() < 3 {
            continue;
        }
        let a = ring[0];
        for i in 1..ring.len() - 1 {
            let b = ring[i];
            let c = ring[i + 1];
            let cross = [
                b[1] * c[2] - b[2] * c[1],
                b[2] * c[0] - b[0] * c[2],
                b[0] * c[1] - b[1] * c[0],
            ];
            v += (a[0] * cross[0] + a[1] * cross[1] + a[2] * cross[2]) / 6.0;
        }
    }
    v
}

/// Snap a point to the `snap` grid, as integer cell coordinates — the identity
/// used for shared-edge adjacency (tessellated faces share exact vertices up to
/// quantisation).
fn snap_key(p: Point, snap: f64) -> (i64, i64, i64) {
    (
        (p[0] / snap).round() as i64,
        (p[1] / snap).round() as i64,
        (p[2] / snap).round() as i64,
    )
}

/// Select the indices of `faces` that form the object's **ground**, assuming the
/// faces are outward-oriented (the caller flips an inward-wound solid first, via
/// [`signed_volume`]). Downward faces (§9) are the candidates; the lowest
/// edge-connected component seeds the ground, and further downward components
/// are accepted while their minimum Z stays within `h_step` of the accepted
/// ground (so terraces join but balcony soffits, ~one storey up, do not).
/// Returned indices are ascending.
pub(crate) fn select_ground_faces(faces: &[Face], opts: &Lod0Options) -> Vec<usize> {
    use std::collections::{HashMap, HashSet};

    let candidates: Vec<usize> = (0..faces.len())
        .filter(|&i| is_downward(&faces[i], opts.theta_deg))
        .collect();
    if candidates.is_empty() {
        return Vec::new();
    }
    let cand_set: HashSet<usize> = candidates.iter().copied().collect();

    // Shared-edge adjacency among candidates (two consecutive shared vertices).
    let mut edge_faces: HashMap<((i64, i64, i64), (i64, i64, i64)), Vec<usize>> = HashMap::new();
    for &fi in &candidates {
        let ring = faces[fi].exterior();
        for k in 0..ring.len() {
            let a = snap_key(ring[k], opts.snap);
            let b = snap_key(ring[(k + 1) % ring.len()], opts.snap);
            let key = if a <= b { (a, b) } else { (b, a) };
            edge_faces.entry(key).or_default().push(fi);
        }
    }
    let mut adj: HashMap<usize, Vec<usize>> = HashMap::new();
    for fs in edge_faces.values() {
        for i in 0..fs.len() {
            for j in (i + 1)..fs.len() {
                if fs[i] != fs[j] {
                    adj.entry(fs[i]).or_default().push(fs[j]);
                    adj.entry(fs[j]).or_default().push(fs[i]);
                }
            }
        }
    }

    // Connected components of candidate faces.
    let mut visited: HashSet<usize> = HashSet::new();
    let mut components: Vec<Vec<usize>> = Vec::new();
    for &start in &candidates {
        if visited.contains(&start) {
            continue;
        }
        let mut comp = Vec::new();
        let mut stack = vec![start];
        while let Some(x) = stack.pop() {
            if !visited.insert(x) {
                continue;
            }
            comp.push(x);
            if let Some(ns) = adj.get(&x) {
                for &n in ns {
                    if cand_set.contains(&n) && !visited.contains(&n) {
                        stack.push(n);
                    }
                }
            }
        }
        components.push(comp);
    }

    let z_range = |comp: &[usize]| -> (f64, f64) {
        let mut zmin = f64::INFINITY;
        let mut zmax = f64::NEG_INFINITY;
        for &fi in comp {
            for r in &faces[fi].rings {
                for p in r {
                    zmin = zmin.min(p[2]);
                    zmax = zmax.max(p[2]);
                }
            }
        }
        (zmin, zmax)
    };

    // Lowest component seeds; accept further components as a rising staircase.
    let mut comps: Vec<(Vec<usize>, f64, f64)> = components
        .into_iter()
        .map(|c| {
            let (lo, hi) = z_range(&c);
            (c, lo, hi)
        })
        .collect();
    comps.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

    let mut accepted: Vec<usize> = Vec::new();
    let mut max_z = f64::NEG_INFINITY;
    for (idx, (comp, cmin, cmax)) in comps.iter().enumerate() {
        if idx == 0 || *cmin <= max_z + opts.h_step {
            accepted.extend(comp.iter().copied());
            max_z = max_z.max(*cmax);
        }
    }
    accepted.sort_unstable();
    accepted
}

/// Snap-grid integer cell of a coordinate relative to a local origin.
fn tcell(v: f64, origin: f64, snap: f64) -> i64 {
    ((v - origin) / snap).round() as i64
}

/// Assemble ground faces into a valid, GeoParquet-legal footprint: 2D-union the
/// faces (edge-sharing polygons must be dissolved — a `MultiPolygon`'s members
/// may touch only at points), then re-drape the ground Z and enforce CCW
/// exterior / CW interior winding. Returns `None` if the union is empty.
///
/// Coordinates are translated to a local origin (world coordinates at
/// 10^5..10^6 m eat f64 precision) and snapped before the union; each output
/// vertex's Z is looked up from the pre-union ground vertices (a union-created
/// intersection vertex, rare, falls back to the lowest ground Z).
pub(crate) fn assemble_footprint(ground: &[&Face], opts: &Lod0Options) -> Option<Vec<Face>> {
    use geo::BooleanOps;
    use geo::algorithm::orient::{Direction, Orient};
    use geo::{Coord, LineString, MultiPolygon, Polygon};
    use std::collections::HashMap;

    if ground.is_empty() {
        return None;
    }
    let snap = opts.snap;
    let mut ox = f64::INFINITY;
    let mut oy = f64::INFINITY;
    for f in ground {
        for r in &f.rings {
            for p in r {
                ox = ox.min(p[0]);
                oy = oy.min(p[1]);
            }
        }
    }
    if !ox.is_finite() || !oy.is_finite() {
        return None;
    }

    // Pass 1: XY snap-cell -> source Z, for re-draping after the union.
    let mut zmap: HashMap<(i64, i64), f64> = HashMap::new();
    for f in ground {
        for r in &f.rings {
            for p in r {
                zmap.insert((tcell(p[0], ox, snap), tcell(p[1], oy, snap)), p[2]);
            }
        }
    }
    let z_fallback = zmap.values().copied().fold(f64::INFINITY, f64::min);

    // Pass 2: build translated, snapped, CLOSED geo rings.
    let build_ring = |ring: &[Point]| -> LineString<f64> {
        let mut coords: Vec<Coord<f64>> = ring
            .iter()
            .map(|p| Coord {
                x: ((p[0] - ox) / snap).round() * snap,
                y: ((p[1] - oy) / snap).round() * snap,
            })
            .collect();
        if let Some(&first) = coords.first() {
            if coords.last() != Some(&first) {
                coords.push(first);
            }
        }
        LineString::new(coords)
    };
    let mut polys: Vec<Polygon<f64>> = Vec::new();
    for f in ground {
        let ext = f.exterior();
        if ext.len() < 3 {
            continue;
        }
        let interiors: Vec<LineString<f64>> =
            f.rings[1..].iter().map(|r| build_ring(r)).collect();
        polys.push(Polygon::new(build_ring(ext), interiors));
    }
    if polys.is_empty() {
        return None;
    }

    // 2D union (i_overlay-backed), then canonical winding.
    let mut acc = MultiPolygon::new(vec![polys[0].clone()]);
    for p in &polys[1..] {
        acc = acc.union(p);
    }
    let acc = acc.orient(Direction::Default);
    if acc.0.is_empty() {
        return None;
    }

    // Re-drape Z and translate back; geo rings are closed, so emit OPEN rings.
    let drape = |ls: &LineString<f64>| -> Vec<Point> {
        let coords = &ls.0;
        let n = if coords.len() >= 2 && coords.first() == coords.last() {
            coords.len() - 1
        } else {
            coords.len()
        };
        (0..n)
            .map(|i| {
                let c = coords[i];
                let z = zmap
                    .get(&((c.x / snap).round() as i64, (c.y / snap).round() as i64))
                    .copied()
                    .unwrap_or(z_fallback);
                [c.x + ox, c.y + oy, z]
            })
            .collect()
    };

    let mut out = Vec::new();
    for poly in &acc.0 {
        let mut rings = vec![drape(poly.exterior())];
        for interior in poly.interiors() {
            rings.push(drape(interior));
        }
        out.push(Face { rings });
    }
    Some(out)
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

    /// A unit cube `[0,1]^3` with each face wound so its Newell normal points
    /// OUTWARD (verified below via `signed_volume` == +1).
    fn unit_cube() -> Vec<Face> {
        vec![
            // bottom z=0, outward -Z
            Face::from_exterior(vec![[0., 0., 0.], [0., 1., 0.], [1., 1., 0.], [1., 0., 0.]]),
            // top z=1, outward +Z
            Face::from_exterior(vec![[0., 0., 1.], [1., 0., 1.], [1., 1., 1.], [0., 1., 1.]]),
            // front y=0, outward -Y
            Face::from_exterior(vec![[0., 0., 0.], [1., 0., 0.], [1., 0., 1.], [0., 0., 1.]]),
            // back y=1, outward +Y
            Face::from_exterior(vec![[0., 1., 0.], [0., 1., 1.], [1., 1., 1.], [1., 1., 0.]]),
            // left x=0, outward -X
            Face::from_exterior(vec![[0., 0., 0.], [0., 0., 1.], [0., 1., 1.], [0., 1., 0.]]),
            // right x=1, outward +X
            Face::from_exterior(vec![[1., 0., 0.], [1., 1., 0.], [1., 1., 1.], [1., 0., 1.]]),
        ]
    }

    #[test]
    fn downward_classifies_ground_not_roof_or_steep_walls() {
        let theta = 20.0;
        // Flat roof: CCW horizontal ring, normal +Z -> not downward.
        let roof = Face::from_exterior(vec![[0., 0., 3.], [4., 0., 3.], [4., 4., 3.], [0., 4., 3.]]);
        assert!(!is_downward(&roof, theta));
        // Ground: CW horizontal ring, normal -Z -> downward.
        let ground =
            Face::from_exterior(vec![[0., 0., 0.], [0., 4., 0.], [4., 4., 0.], [4., 0., 0.]]);
        assert!(is_downward(&ground, theta));
        // A vertical wall (normal horizontal) -> not downward.
        let wall = Face::from_exterior(vec![[0., 0., 0.], [4., 0., 0.], [4., 0., 3.], [0., 0., 3.]]);
        assert!(!is_downward(&wall, theta));
        // A face tilted 25 deg from horizontal (normal 25 deg from -Z) is
        // outside the 20 deg cone -> not downward.
        let a = 25.0_f64.to_radians();
        let tilt = Face::from_exterior(vec![
            [0., 0., 0.],
            [0., 4., 0.],
            [4., 4., 4.0 * a.tan()],
            [4., 0., 4.0 * a.tan()],
        ]);
        assert!(!is_downward(&tilt, theta), "25deg tilt must be excluded at theta=20");
    }

    #[test]
    fn signed_volume_of_outward_cube_is_plus_one_and_flips_when_reversed() {
        let cube = unit_cube();
        assert!((signed_volume(&cube) - 1.0).abs() < 1e-9);
        // Reverse every ring: normals flip inward, volume negates.
        let reversed: Vec<Face> = cube
            .iter()
            .map(|f| Face {
                rings: f.rings.iter().map(|r| r.iter().rev().copied().collect()).collect(),
            })
            .collect();
        assert!((signed_volume(&reversed) + 1.0).abs() < 1e-9);
    }

    /// A horizontal, downward-wound (normal -Z) square at height `z`, offset by
    /// `(dx, dy)`, side 2.
    fn ground_square(dx: f64, dy: f64, z: f64) -> Face {
        Face::from_exterior(vec![
            [dx, dy, z],
            [dx, dy + 2., z],
            [dx + 2., dy + 2., z],
            [dx + 2., dy, z],
        ])
    }

    #[test]
    fn sloped_ground_grows_across_the_shared_edge() {
        // Two downward faces sharing the edge x=2 (y in [0,2]); B ramps up to
        // z=0.3 over 2 m (~8.5deg, inside the cone). Both are ground.
        let a = ground_square(0., 0., 0.); // shares edge (2,0,0)-(2,2,0)
        let b = Face::from_exterior(vec![
            [2., 0., 0.],
            [2., 2., 0.],
            [4., 2., 0.3],
            [4., 0., 0.3],
        ]);
        let sel = select_ground_faces(&[a, b], &Lod0Options::default());
        assert_eq!(sel, vec![0, 1]);
    }

    #[test]
    fn terrace_within_h_step_is_accepted_beyond_it_is_not() {
        let opts = Lod0Options::default(); // h_step = 1.5
        // Two disconnected ground components: base at z=0, terrace at z=1.0.
        let near = select_ground_faces(
            &[ground_square(0., 0., 0.), ground_square(5., 0., 1.0)],
            &opts,
        );
        assert_eq!(near, vec![0, 1], "terrace 1.0 m up is within h_step");
        // Terrace at z=2.0 is beyond h_step -> only the base.
        let far = select_ground_faces(
            &[ground_square(0., 0., 0.), ground_square(5., 0., 2.0)],
            &opts,
        );
        assert_eq!(far, vec![0], "terrace 2.0 m up is beyond h_step");
    }

    #[test]
    fn balcony_soffit_one_storey_up_is_rejected() {
        // A downward, horizontal soffit 2.7 m up, not edge-connected to ground.
        let ground = ground_square(0., 0., 0.);
        let soffit = ground_square(0., 3., 2.7);
        let sel = select_ground_faces(&[ground, soffit], &Lod0Options::default());
        assert_eq!(sel, vec![0], "a cantilever soffit is not ground");
    }

    #[test]
    fn flat_roof_is_never_selected() {
        // Roof: CCW horizontal ring -> normal +Z -> not a candidate.
        let roof = Face::from_exterior(vec![[0., 0., 3.], [4., 0., 3.], [4., 4., 3.], [0., 4., 3.]]);
        let ground = ground_square(0., 0., 0.);
        let sel = select_ground_faces(&[roof, ground], &Lod0Options::default());
        assert_eq!(sel, vec![1], "only the ground face, never the roof");
    }

    /// Shoelace signed area of an open ring in XY (positive == CCW).
    fn signed_area_xy(ring: &[Point]) -> f64 {
        let mut a = 0.0;
        for i in 0..ring.len() {
            let p = ring[i];
            let q = ring[(i + 1) % ring.len()];
            a += p[0] * q[1] - q[0] * p[1];
        }
        a / 2.0
    }

    #[test]
    fn two_edge_sharing_squares_union_to_one_ccw_surface() {
        let a = ground_square(0., 0., 0.); // [0,2] x [0,2]
        let b = ground_square(2., 0., 0.); // [2,4] x [0,2], shares edge x=2
        let out = assemble_footprint(&[&a, &b], &Lod0Options::default()).unwrap();
        assert_eq!(out.len(), 1, "edge-sharing squares dissolve into one polygon");
        assert_eq!(out[0].rings.len(), 1, "no interior rings");
        let area = signed_area_xy(&out[0].rings[0]);
        assert!((area - 8.0).abs() < 1e-6, "unioned area is 4x2 = 8, got {area}");
        assert!(area > 0.0, "exterior must be CCW after orientation");
    }

    #[test]
    fn face_with_hole_keeps_exterior_ccw_and_interior_cw() {
        // 10x10 ground with a 4x4 courtyard hole.
        let outer = vec![[0., 0., 5.], [0., 10., 5.], [10., 10., 5.], [10., 0., 5.]];
        let hole = vec![[3., 3., 5.], [7., 3., 5.], [7., 7., 5.], [3., 7., 5.]];
        let face = Face {
            rings: vec![outer, hole],
        };
        let out = assemble_footprint(&[&face], &Lod0Options::default()).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].rings.len(), 2, "exterior + one courtyard ring");
        assert!(signed_area_xy(&out[0].rings[0]) > 0.0, "exterior CCW");
        assert!(signed_area_xy(&out[0].rings[1]) < 0.0, "interior CW");
        assert!((signed_area_xy(&out[0].rings[0]).abs() - 100.0).abs() < 1e-6);
        assert!((signed_area_xy(&out[0].rings[1]).abs() - 16.0).abs() < 1e-6);
    }

    #[test]
    fn assembled_vertices_keep_the_source_ground_z() {
        let a = ground_square(0., 0., 5.);
        let b = ground_square(2., 0., 5.);
        let out = assemble_footprint(&[&a, &b], &Lod0Options::default()).unwrap();
        for face in &out {
            for ring in &face.rings {
                for p in ring {
                    assert!((p[2] - 5.0).abs() < 1e-9, "Z draped from source (5)");
                }
            }
        }
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
