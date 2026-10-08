//! WKB cells (spec 03 "WKB encoding", "Geometry-type mapping").

use super::*;

// ---------------------------------------------------------------------------
// WKB
// ---------------------------------------------------------------------------

/// The WKB type names `city.columns[].geometry_types` uses (spec 05).
pub(super) fn wkb_type_name(code: u32) -> Option<&'static str> {
    Some(match code {
        1001 => "Point Z",
        1002 => "LineString Z",
        1003 => "Polygon Z",
        1004 => "MultiPoint Z",
        1005 => "MultiLineString Z",
        1006 => "MultiPolygon Z",
        1007 => "GeometryCollection Z",
        1015 => "PolyhedralSurface Z",
        _ => return None,
    })
}

/// What a walk over one WKB cell saw: every geometry type code (the
/// top-level one first, then each collection member's), each polygon's
/// ring point counts, and the face count of each polyhedral surface.
#[derive(Default)]
pub(super) struct WkbShape {
    pub(super) codes: Vec<u32>,
    pub(super) faces: Vec<Vec<usize>>,
    pub(super) solids: Vec<usize>,
    pub(super) pending_rings: Vec<usize>,
    pub(super) in_solid: bool,
    /// `[xmin, ymin, zmin, xmax, ymax, zmax]` over every stored vertex.
    pub(super) extent: Option<[f64; 6]>,
}

impl WkbVisitor for WkbShape {
    fn coord(&mut self, xyz: [f64; 3]) {
        let point = [xyz[0], xyz[1], xyz[2], xyz[0], xyz[1], xyz[2]];
        self.extent = Some(match self.extent {
            None => point,
            Some(e) => union_box(e, point),
        });
    }

    fn geometry(&mut self, type_code: u32) {
        self.codes.push(type_code);
        self.in_solid = type_code == POLYHEDRALSURFACE_Z;
        if self.in_solid {
            self.solids.push(0);
        }
    }

    fn ring_end(&mut self, n_points: usize) {
        self.pending_rings.push(n_points);
    }

    fn polygon_end(&mut self, _n_rings: usize) {
        self.faces.push(std::mem::take(&mut self.pending_rings));
        if self.in_solid
            && let Some(n) = self.solids.last_mut()
        {
            *n += 1;
        }
    }
}

impl WkbShape {
    pub(super) fn top(&self) -> u32 {
        self.codes[0]
    }

    /// Surfaces are per WKB face only for the polygonal encodings.
    pub(super) fn is_polygonal(&self) -> bool {
        matches!(
            self.top(),
            MULTIPOLYGON_Z | POLYHEDRALSURFACE_Z | GEOMETRYCOLLECTION_Z
        )
    }

    pub(super) fn is_solid_family(&self) -> bool {
        self.top() == POLYHEDRALSURFACE_Z
            || (self.top() == GEOMETRYCOLLECTION_Z
                && self.codes[1..].iter().all(|&c| c == POLYHEDRALSURFACE_Z))
    }
}

/// Parse one WKB cell (spec 03 "WKB encoding": little-endian ISO WKB with
/// 1000-series Z codes, every polygon ring explicitly closed with at least
/// four points, encoded per the geometry-type mapping).
pub(super) fn parse_wkb(bytes: &[u8]) -> std::result::Result<WkbShape, String> {
    let mut shape = WkbShape::default();
    match bytes.first() {
        Some(0x01) => {}
        Some(other) => {
            return Err(format!(
                "byte-order marker {other:#04x}; CityParquet WKB is little-endian (0x01)"
            ));
        }
        None => return Err("empty WKB".to_string()),
    }
    visit_wkb(bytes, &mut shape).map_err(|e| e.to_string())?;
    let top = shape.top();
    if !matches!(top, 1004..=1007 | 1015) {
        return Err(format!(
            "top-level type code {top} is not one of the geometry-type mapping's encodings"
        ));
    }
    Ok(shape)
}

/// The WKB encoding each CityGML CM geometry type maps to (spec 03
/// "Geometry-type mapping").
pub(super) fn wkb_code_for_cm_type(cm: &str) -> Option<u32> {
    Some(match cm {
        "MultiPoint" => MULTIPOINT_Z,
        "MultiCurve" => 1005,
        "MultiSurface" | "CompositeSurface" => MULTIPOLYGON_Z,
        "Solid" => POLYHEDRALSURFACE_Z,
        "MultiSolid" | "CompositeSolid" => GEOMETRYCOLLECTION_Z,
        _ => return None,
    })
}

/// The smallest box holding both boxes (`[xmin, ymin, zmin, xmax, ymax, zmax]`).
pub(super) fn union_box(a: [f64; 6], b: [f64; 6]) -> [f64; 6] {
    [
        a[0].min(b[0]),
        a[1].min(b[1]),
        a[2].min(b[2]),
        a[3].max(b[3]),
        a[4].max(b[4]),
        a[5].max(b[5]),
    ]
}

/// Whether `outer` holds `inner`, to a relative tolerance of one part in
/// 10^9 (both come from the same coordinates, rounded independently).
pub(super) fn box_contains(outer: [f64; 6], inner: [f64; 6]) -> bool {
    let slack = |v: f64| 1e-9 * v.abs().max(1.0);
    (0..3).all(|k| outer[k] <= inner[k] + slack(inner[k]))
        && (3..6).all(|k| outer[k] >= inner[k] - slack(inner[k]))
}
