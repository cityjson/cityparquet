//! Row ordering for M5: a 2D Hilbert curve over bbox centroids, used by
//! [`crate::package::convert`] (`ordering: RowOrder::Hilbert`) to reorder
//! FEATURES before encoding, so that spatially nearby features land in the
//! same or adjacent parquet row groups — improving bbox row-group pruning
//! (see `crate::reader::CityParquetReaderBuilder::with_bbox_row_groups`).
//!
//! City models are height-thin relative to their horizontal extent, so the
//! Hilbert key is computed over `x`/`y` only; `z` never enters the curve.
//! This is a documented, deliberate choice — a 3D Hilbert curve would spend
//! curve resolution on an axis with comparatively little spread for typical
//! city-scale datasets.

use cjseq::Transform;

/// Hilbert curve order: `2^ORDER` cells per axis. 16 gives 65,536 cells per
/// axis (comfortably finer than any real dataset's row-group granularity)
/// while keeping the two-axis index within `u32` (`(2^16)^2 == 2^32`, i.e.
/// exactly `u32::MAX + 1` distinct cells — the whole `u32` range is used).
const ORDER: u32 = 16;
const SIDE: u32 = 1 << ORDER;

/// Maps a real-world `(x, y)` point into a Hilbert-curve index over `bbox`
/// (only `bbox`'s `x`/`y` extent is used — see the module doc for why `z` is
/// ignored). Points outside `bbox` clamp to the nearest edge cell rather
/// than panicking or wrapping, so a caller need not pre-clip; a degenerate
/// bbox (`xmin == xmax` and/or `ymin == ymax`) maps every point on that axis
/// to cell `0` instead of dividing by zero.
pub fn hilbert_index(x: f64, y: f64, bbox: &[f64; 6]) -> u32 {
    let cell_x = normalise_axis(x, bbox[0], bbox[3]);
    let cell_y = normalise_axis(y, bbox[1], bbox[4]);
    xy2d(cell_x, cell_y)
}

/// `v`'s position along one axis of `bbox`, quantised to a cell in
/// `0..SIDE`, clamped to that range (values outside `[min, max]` clamp to
/// the nearest edge cell). `min >= max` (a degenerate or malformed axis)
/// always yields cell `0`.
fn normalise_axis(v: f64, min: f64, max: f64) -> u32 {
    let span = max - min;
    if span <= 0.0 {
        return 0;
    }
    let t = ((v - min) / span).clamp(0.0, 1.0);
    let cell = (t * (SIDE as f64 - 1.0)).round();
    // `t` is clamped to [0, 1] above, so `cell` is already within
    // `[0, SIDE - 1]` modulo floating-point rounding at the very top edge;
    // the explicit `min` below is a defensive belt-and-braces clamp, not a
    // load-bearing one.
    (cell as u32).min(SIDE - 1)
}

/// The standard `xy2d` Hilbert-curve construction (Wikipedia's "Hilbert
/// curve" reference C implementation, transliterated): converts a cell
/// `(x, y)` with `0 <= x, y < SIDE` into its position `d` along the curve.
/// Each iteration peels off one bit of resolution (`s` halves each time,
/// from `SIDE / 2` down to `1`), so the FIRST iteration's quadrant choice
/// dominates `d` (it contributes `s * s`, at least as much as every
/// remaining iteration combined can ever add: `sum_{k=0}^{s-1} k*k*3 <
/// s*s`) — this is what gives the curve its locality property: two points
/// in different top-level quadrants are farther apart in `d` than any two
/// points sharing a quadrant, however they are positioned within it. The
/// maximum possible `d` is exactly `SIDE * SIDE - 1 == u32::MAX` (with
/// `ORDER = 16`, `SIDE = 2^16`, so `SIDE * SIDE == 2^32`), so plain `u32`
/// arithmetic never overflows here.
fn xy2d(mut x: u32, mut y: u32) -> u32 {
    let mut d: u32 = 0;
    let mut s = SIDE / 2;
    while s > 0 {
        let rx = u32::from((x & s) > 0);
        let ry = u32::from((y & s) > 0);
        d += s * s * ((3 * rx) ^ ry);
        rotate(&mut x, &mut y, rx, ry);
        s /= 2;
    }
    d
}

/// Rotates/reflects the cell so the recursion at the next (finer) `s`
/// lines back up with the curve's canonical orientation — the classic
/// Hilbert-curve "rot" step, using the constant `SIDE` (the reference
/// implementation's `n`) on every call, not a shrinking sub-square size.
fn rotate(x: &mut u32, y: &mut u32, rx: u32, ry: u32) {
    if ry == 0 {
        if rx == 1 {
            *x = SIDE - 1 - *x;
            *y = SIDE - 1 - *y;
        }
        std::mem::swap(x, y);
    }
}

/// `(scale, translate)` as fixed-size arrays, missing components defaulting
/// like [`crate::wkb_write::VertexPool`]'s and `crate::export`'s identical
/// helpers (kept local here rather than shared, per those modules' own
/// comments — it is three lines, not worth a shared dependency edge).
fn transform_axes(transform: &Transform) -> ([f64; 3], [f64; 3]) {
    let take3 = |v: &[f64], d: f64| {
        [
            *v.first().unwrap_or(&d),
            *v.get(1).unwrap_or(&d),
            *v.get(2).unwrap_or(&d),
        ]
    };
    (
        take3(&transform.scale, 1.0),
        take3(&transform.translate, 0.0),
    )
}

/// The min/max corners (world coordinates, dequantised via `transform`) of
/// every vertex in `vertices`, or `None` if `vertices` is empty (a feature
/// with no vertices at all — no geometry whatsoever).
fn vertices_minmax(vertices: &[Vec<i64>], transform: &Transform) -> Option<([f64; 3], [f64; 3])> {
    let (scale, translate) = transform_axes(transform);
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    let mut any = false;
    for v in vertices {
        if v.len() < 3 {
            continue;
        }
        any = true;
        for i in 0..3 {
            let coord = v[i] as f64 * scale[i] + translate[i];
            min[i] = min[i].min(coord);
            max[i] = max[i].max(coord);
        }
    }
    any.then_some((min, max))
}

/// The Hilbert-ordering key for one CityJSONFeature: the curve index of
/// `(x, y)` centroid `(min + max) / 2` of the feature's OWN vertex pool
/// (dequantised via `transform`), normalised against `dataset_bbox`.
///
/// `vertices` is a CityJSONFeature's own `vertices` array — a cjseq feature
/// is self-contained (its vertex pool holds exactly the vertices its own
/// geometries index into), so this is the cheapest correct source for a
/// per-feature centroid: no need to walk objects/geometries first.
///
/// Features with NO vertices at all (a feature whose objects carry no
/// geometry) get key `0` — [`crate::package::convert`]'s sort is stable, so
/// they simply retain their original relative order, all grouped at the
/// front, rather than scattering arbitrarily through the middle of the
/// dataset (documented at the call site too).
pub(crate) fn feature_hilbert_key(
    vertices: &[Vec<i64>],
    transform: &Transform,
    dataset_bbox: &[f64; 6],
) -> u32 {
    let Some((min, max)) = vertices_minmax(vertices, transform) else {
        return 0;
    };
    let cx = (min[0] + max[0]) / 2.0;
    let cy = (min[1] + max[1]) / 2.0;
    hilbert_index(cx, cy, dataset_bbox)
}

#[cfg(test)]
mod tests {
    use super::*;

    const UNIT_BBOX: [f64; 6] = [0.0, 0.0, 0.0, 1.0, 1.0, 1.0];

    #[test]
    fn origin_of_unit_bbox_is_index_zero() {
        assert_eq!(hilbert_index(0.0, 0.0, &UNIT_BBOX), 0);
    }

    /// The four quadrant centres, visited in the standard order-1 Hilbert
    /// "U" sequence (0,0) -> (0,1) -> (1,1) -> (1,0): each top-level
    /// quadrant choice dominates the index (see `xy2d`'s doc comment), so
    /// indices must be strictly increasing in this visit order regardless
    /// of the finer (order-16) resolution used underneath.
    #[test]
    fn quadrant_centres_follow_the_order1_hilbert_u() {
        let bottom_left = hilbert_index(0.25, 0.25, &UNIT_BBOX);
        let top_left = hilbert_index(0.25, 0.75, &UNIT_BBOX);
        let top_right = hilbert_index(0.75, 0.75, &UNIT_BBOX);
        let bottom_right = hilbert_index(0.75, 0.25, &UNIT_BBOX);
        assert!(
            bottom_left < top_left,
            "bottom_left={bottom_left} top_left={top_left}"
        );
        assert!(
            top_left < top_right,
            "top_left={top_left} top_right={top_right}"
        );
        assert!(
            top_right < bottom_right,
            "top_right={top_right} bottom_right={bottom_right}"
        );
    }

    #[test]
    fn points_outside_bbox_clamp_to_the_nearest_edge_cell() {
        let bbox: [f64; 6] = [0.0, 0.0, 0.0, 10.0, 10.0, 10.0];
        assert_eq!(
            hilbert_index(-5.0, 5.0, &bbox),
            hilbert_index(0.0, 5.0, &bbox),
            "a point left of xmin must clamp to xmin's cell"
        );
        assert_eq!(
            hilbert_index(15.0, 5.0, &bbox),
            hilbert_index(10.0, 5.0, &bbox),
            "a point right of xmax must clamp to xmax's cell"
        );
        assert_eq!(
            hilbert_index(5.0, -5.0, &bbox),
            hilbert_index(5.0, 0.0, &bbox),
            "a point below ymin must clamp to ymin's cell"
        );
    }

    #[test]
    fn degenerate_bbox_does_not_panic_or_divide_by_zero() {
        // xmin == xmax: every x must map to cell 0 on that axis, not NaN/Inf.
        let degenerate_x: [f64; 6] = [5.0, 0.0, 0.0, 5.0, 10.0, 10.0];
        let a = hilbert_index(5.0, 2.0, &degenerate_x);
        let b = hilbert_index(999.0, 2.0, &degenerate_x);
        assert_eq!(
            a, b,
            "every x must collapse to the same (zero) cell on a degenerate x axis"
        );

        // Fully degenerate (a point bbox): must not panic, and is trivially
        // index 0 (both axes collapse to cell 0).
        let point_bbox: [f64; 6] = [3.0, 4.0, 0.0, 3.0, 4.0, 0.0];
        assert_eq!(hilbert_index(3.0, 4.0, &point_bbox), 0);
    }

    #[test]
    fn feature_with_no_vertices_gets_key_zero() {
        let transform = Transform {
            scale: vec![1.0, 1.0, 1.0],
            translate: vec![0.0, 0.0, 0.0],
        };
        let bbox: [f64; 6] = [0.0, 0.0, 0.0, 10.0, 10.0, 10.0];
        assert_eq!(feature_hilbert_key(&[], &transform, &bbox), 0);
    }

    #[test]
    fn feature_key_uses_the_vertex_pool_centroid_after_dequantisation() {
        let transform = Transform {
            scale: vec![0.001, 0.001, 0.001],
            translate: vec![0.0, 0.0, 0.0],
        };
        // Quantised vertices spanning [0, 10000] on x and y -> world [0, 10]
        // after the 0.001 scale; centroid (5, 5).
        let vertices = vec![vec![0, 0, 0], vec![10_000, 10_000, 0]];
        let bbox: [f64; 6] = [0.0, 0.0, 0.0, 10.0, 10.0, 10.0];
        let key = feature_hilbert_key(&vertices, &transform, &bbox);
        let expected = hilbert_index(5.0, 5.0, &bbox);
        assert_eq!(key, expected);
    }
}
