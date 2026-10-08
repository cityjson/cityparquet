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
}

impl WkbVisitor for WkbShape {
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
        "MultiLineString" => 1005,
        "MultiSurface" | "CompositeSurface" => MULTIPOLYGON_Z,
        "Solid" => POLYHEDRALSURFACE_Z,
        "MultiSolid" | "CompositeSolid" => GEOMETRYCOLLECTION_Z,
        _ => return None,
    })
}
