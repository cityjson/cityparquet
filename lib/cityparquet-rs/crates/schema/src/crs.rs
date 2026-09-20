//! Resolve a source CRS identifier (an EPSG code or an OGC CRS URL) to
//! PROJJSON, using a vendored offline lookup table (spec §13.3, gap G1).
//!
//! GeoParquet 1.1 requires each geometry column's CRS to be **PROJJSON**, and
//! no maintained pure-Rust crate emits PROJJSON from an EPSG code (even
//! geoarrow-rs defers to pyproj). So the table is generated offline by
//! `tools/gen_projjson.py` from PROJ's `proj.db` — byte-for-byte the same
//! definitions GDAL/GeoPandas write — and committed gzipped, kept out of the
//! build so `cargo build` needs no C toolchain or network.

use std::collections::HashMap;
use std::io::Read;
use std::sync::OnceLock;

use serde_json::Value;

use crate::error::{CityParquetError, Result};

/// The quantisation step for a **linear** (metre-valued) axis: one millimetre.
pub const MM: f64 = 1e-3;

/// The quantisation step for an **angular** (degree-valued) axis: 1e-9 degree,
/// ~0.11 mm of latitude and never coarser than that in longitude. The angular
/// counterpart of [`MM`], and a format constant per unit rather than a value
/// tuned to any one corpus: deriving it from how many decimals a source
/// happens to write would make a package's precision depend on its input.
pub const NANO_DEGREE: f64 = 1e-9;

/// Every axis of a PROJJSON CRS, in order, flattening a `CompoundCRS` into its
/// components and following a `BoundCRS` to the CRS it wraps.
fn axes(crs: &Value) -> Vec<&Value> {
    if let Some(components) = crs.get("components").and_then(Value::as_array) {
        return components.iter().flat_map(|c| axes(c)).collect();
    }
    if let Some(source) = crs.get("source_crs") {
        return axes(source);
    }
    crs.get("coordinate_system")
        .and_then(|cs| cs.get("axis"))
        .and_then(Value::as_array)
        .map(|a| a.iter().collect())
        .unwrap_or_default()
}

/// Radians per degree — the scale PROJJSON's `AngularUnit` conversion factors
/// are expressed in, so a non-degree angular unit converts exactly.
const RADIANS_PER_DEGREE: f64 = std::f64::consts::PI / 180.0;

/// EPSG's packed sexagesimal pseudo-unit. It carries a `conversion_factor`
/// like any other `AngularUnit`, but a value in it reads `DDDMMSS.sss` —
/// digits at different magnitudes meaning different things — not a number on a
/// linear scale. Quantising one would be arithmetic on a string, so it is
/// refused by name rather than run through the angular rule.
const SEXAGESIMAL: &str = "degree minute second hemisphere";

/// The quantisation step for one PROJJSON axis, from its **declared unit**.
///
/// `metre` and `degree` — the two units PROJ writes as bare strings, and the
/// only two any real city model uses — map straight to [`MM`] and
/// [`NANO_DEGREE`]. Every other unit arrives as an object carrying an exact
/// `conversion_factor`, so its step is *converted* rather than guessed: a
/// linear unit gets whatever length equals one millimetre (a US survey foot's
/// step is `0.001 / 0.3048006… = 0.00328…` ft), an angular one whatever angle
/// equals [`NANO_DEGREE`]. That keeps the ~2 300 foot-, link- and chain-valued
/// national CRS in the vendored table convertible, at exactly the precision a
/// metre-valued one gets, rather than refusing them over the unit they happen
/// to be written in.
fn axis_step(axis: &Value, i: usize) -> Result<f64> {
    let refuse = |what: String| {
        CityParquetError::Schema(format!(
            "PROJJSON axis {i} is in {what}; this writer quantises metre- and \
             degree-valued axes, and any unit carrying an exact conversion factor to \
             one of them — reproject the source into a CRS it can encode"
        ))
    };
    let unit = axis
        .get("unit")
        .ok_or_else(|| CityParquetError::Schema(format!("PROJJSON axis {i} declares no unit")))?;

    if let Value::String(name) = unit {
        return match name.as_str() {
            "metre" => Ok(MM),
            "degree" => Ok(NANO_DEGREE),
            other => Err(refuse(format!("{other:?}"))),
        };
    }

    let name = unit.get("name").and_then(Value::as_str).unwrap_or("?");
    let factor = unit
        .get("conversion_factor")
        .and_then(Value::as_f64)
        .filter(|f| f.is_finite() && *f > 0.0)
        .ok_or_else(|| refuse(format!("{name:?} (no usable conversion factor)")))?;

    match unit.get("type").and_then(Value::as_str) {
        Some("LinearUnit") => Ok(MM / factor),
        Some("AngularUnit") if name != SEXAGESIMAL => Ok(NANO_DEGREE * RADIANS_PER_DEGREE / factor),
        _ => Err(refuse(format!("{name:?}"))),
    }
}

/// The per-axis quantisation scale for a resolved PROJJSON CRS.
///
/// The CityJSON `transform` carries no CRS of its own, so both ends of the
/// pipeline — the CityGML reader building a header, and the exporter
/// synthesising one — must derive the same scale from the CRS they are
/// encoding against, or a degree-valued coordinate is quantised at a
/// metre-sized step and destroyed. Each axis's step comes from its own
/// declared unit ([`axis_step`]), never from the magnitude of the coordinates
/// — sniffing is what the spec's CRS rules forbid.
///
/// A CRS with fewer than three axes — a 2D geographic or projected CRS —
/// extends with [`MM`] for z rather than erroring: a CityGML document may
/// declare a 2D `srsName` and still carry `srsDimension="3"` coordinates, and
/// a height is metres in every CRS that pairs with one. A CRS with **no**
/// readable axes at all is an error, not a silent millimetre default: that
/// default is right only where there is no CRS to derive from, and applying it
/// to a resolved-but-unreadable one is how a degree axis would get a
/// metre-sized step again.
pub fn axis_scale(crs: &Value) -> Result<[f64; 3]> {
    let axes = axes(crs);
    if axes.is_empty() {
        return Err(CityParquetError::Schema(format!(
            "PROJJSON CRS declares no coordinate system axes, so no quantisation step \
             can be derived from it: {crs}"
        )));
    }
    let mut scale = [MM; 3];
    for (i, axis) in axes.iter().take(3).enumerate() {
        scale[i] = axis_step(axis, i)?;
    }
    Ok(scale)
}

/// Whether the CRS declares **latitude (northing) before longitude
/// (easting)**, as EPSG does for `4326`, `6697` and every other geographic
/// code a national export is likely to carry.
///
/// GeoParquet stores WKB coordinates as `(x, y) = (longitude, latitude)`
/// whatever the authority's axis order says, so a writer must swap for these.
/// Read from the declared axis `direction`s — not guessed from coordinate
/// magnitudes, which is unreliable wherever |longitude| <= 90.
pub fn is_latitude_first(crs: &Value) -> bool {
    let axes = axes(crs);
    let direction = |i: usize| {
        axes.get(i)
            .and_then(|a| a.get("direction"))
            .and_then(Value::as_str)
    };
    direction(0) == Some("north") && direction(1) == Some("east")
}

/// Whether a dataset's coordinates are stored **latitude first**, and the swap
/// that reconciles that with WKB.
///
/// GeoParquet stores WKB as `(x, y) = (longitude, latitude)` whatever axis
/// order the CRS authority declares, while CityJSON coordinates keep the
/// source's own order. [`AxisOrder`] is the one thing that has to be known at
/// every crossing between the two, in either direction — the swap is its own
/// inverse, so [`AxisOrder::apply`] serves the write and the read alike.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AxisOrder {
    /// Longitude/easting first — WKB's order already, so nothing to do. Every
    /// projected CRS a city model is normally in lands here.
    #[default]
    LonLat,
    /// Latitude/northing first, as EPSG declares for `4326`, `6697` and the
    /// other geographic codes a national export carries.
    LatLon,
}

impl AxisOrder {
    /// The axis order a resolved PROJJSON CRS declares.
    pub fn of(crs: &Value) -> Self {
        if is_latitude_first(crs) {
            Self::LatLon
        } else {
            Self::LonLat
        }
    }

    /// Reorder one coordinate between the dataset's order and WKB's. The
    /// vertical axis never moves.
    pub fn apply(self, c: [f64; 3]) -> [f64; 3] {
        match self {
            Self::LonLat => c,
            Self::LatLon => [c[1], c[0], c[2]],
        }
    }
}

/// The committed EPSG -> PROJJSON table, gzipped. Regenerate with
/// `tools/gen_projjson.py` when the pinned PROJ/EPSG dataset is bumped (the
/// version pins live in the asset's `_meta`).
static ASSET: &[u8] = include_bytes!("../assets/epsg_projjson.json.gz");

/// The parsed table: `"7415"` / `"OGC:CRS84"` -> PROJJSON object. Decompressed
/// and parsed once, lazily (only the first CRS resolution pays for it).
fn table() -> &'static HashMap<String, Value> {
    static TABLE: OnceLock<HashMap<String, Value>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut gz = flate2::read::GzDecoder::new(ASSET);
        let mut json = String::new();
        gz.read_to_string(&mut json)
            .expect("vendored PROJJSON asset must gunzip");
        let mut map: HashMap<String, Value> =
            serde_json::from_str(&json).expect("vendored PROJJSON asset must parse");
        map.remove("_meta");
        map
    })
}

/// Extract the lookup key (an EPSG code, or `OGC:CRS84`/`OGC:CRS84h`) from a
/// source CRS identifier: a bare code `7415`, `EPSG:7415`,
/// `urn:ogc:def:crs:EPSG::7415`, an OGC CRS URL
/// `https://www.opengis.net/def/crs/EPSG/0/7415`, or a CRS84 URL.
fn lookup_key(source: &str) -> Option<String> {
    let s = source.trim().trim_end_matches('/');
    if s.is_empty() {
        return None;
    }
    // A bare numeric code is taken as EPSG (the common convenience form).
    if s.chars().all(|c| c.is_ascii_digit()) {
        return Some(s.to_string());
    }
    // `EPSG:7415` shorthand.
    if let Some(code) = s.strip_prefix("EPSG:") {
        let code = code.trim();
        return (!code.is_empty() && code.chars().all(|c| c.is_ascii_digit()))
            .then(|| code.to_string());
    }
    // `OGC:CRS84` / `OGC:CRS84h` shorthand.
    if let Some(rest) = s.strip_prefix("OGC:") {
        return matches!(rest, "CRS84" | "CRS84h").then(|| format!("OGC:{rest}"));
    }
    // urn / OGC-URL forms. Tokenise on `:`/`/` and require the *explicit*
    // authority token, so a numeric code under a non-EPSG authority
    // (e.g. `.../def/crs/IGNF/0/7415`) is NOT mis-read as EPSG (sol-review G1).
    // Authority tokens are matched case-sensitively: the real CRS84 URN carries
    // an uppercase `OGC` authority token, distinct from the lowercase `ogc` URN
    // scheme token that every `urn:ogc:def:crs:*` shares.
    let tokens: Vec<&str> = s.split([':', '/']).filter(|t| !t.is_empty()).collect();
    if tokens.contains(&"EPSG")
        && let Some(code) = tokens
            .iter()
            .rev()
            .find(|t| !t.is_empty() && t.chars().all(|c| c.is_ascii_digit()))
    {
        return Some((*code).to_string());
    }
    if tokens.contains(&"OGC")
        && let Some(crs) = tokens
            .iter()
            .rev()
            .find(|t| matches!(**t, "CRS84" | "CRS84h"))
    {
        return Some(format!("OGC:{crs}"));
    }
    None
}

/// A PROJJSON CRS object always carries a `type` naming a CRS variant
/// (`GeographicCRS`, `ProjectedCRS`, `CompoundCRS`, `BoundCRS`, …) — every one
/// ends in `CRS`. Used to tell an already-PROJJSON input apart from an
/// identifier string or an unrelated JSON object.
fn is_projjson_crs(value: &Value) -> bool {
    value
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(|t| t.ends_with("CRS"))
}

/// Resolve a source CRS identifier to its PROJJSON from the vendored EPSG
/// table. Errors if the identifier is unparseable or its code is not in the
/// table — it MUST NOT silently omit the CRS (§13.3: an absent GeoParquet
/// `crs` is taken to mean OGC:CRS84, silently mis-georeferencing a projected
/// national CRS).
pub fn resolve_to_projjson(source: &str) -> Result<Value> {
    // Already PROJJSON? (a CityGML source may hand one straight through.) Only
    // a real PROJJSON CRS object — one whose `type` names a CRS variant — may
    // short-circuit; an arbitrary object must not be emitted verbatim as an
    // (invalid) GeoParquet `crs` (sol-review G1).
    if let Ok(value) = serde_json::from_str::<Value>(source)
        && is_projjson_crs(&value)
    {
        return Ok(value);
    }
    let key = lookup_key(source).ok_or_else(|| {
        CityParquetError::Schema(format!(
            "cannot extract an EPSG/OGC code from CRS {source:?}"
        ))
    })?;
    table().get(&key).cloned().ok_or_else(|| {
        CityParquetError::Schema(format!(
            "CRS {source:?} (code {key}) is not in the vendored EPSG->PROJJSON table; \
             regenerate it with tools/gen_projjson.py if the code is valid"
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_key_extracts_the_epsg_code() {
        assert_eq!(lookup_key("7415").as_deref(), Some("7415"));
        assert_eq!(lookup_key("EPSG:7415").as_deref(), Some("7415"));
        assert_eq!(
            lookup_key("urn:ogc:def:crs:EPSG::7415").as_deref(),
            Some("7415")
        );
        assert_eq!(
            lookup_key("https://www.opengis.net/def/crs/EPSG/0/7415").as_deref(),
            Some("7415")
        );
        assert_eq!(lookup_key("OGC:CRS84").as_deref(), Some("OGC:CRS84"));
        assert_eq!(
            lookup_key("http://www.opengis.net/def/crs/OGC/1.3/CRS84").as_deref(),
            Some("OGC:CRS84")
        );
        assert_eq!(lookup_key("not a crs").as_deref(), None);
    }

    #[test]
    fn axis_scale_is_per_axis_and_follows_the_declared_units() {
        // EPSG:7415 = Amersfoort/RD New + NAP height: metre, metre, metre.
        let projected = resolve_to_projjson("EPSG:7415").unwrap();
        assert_eq!(axis_scale(&projected).unwrap(), [MM, MM, MM]);

        // EPSG:6697 = JGD2011 + JGD2011 (vertical) height, the CRS every
        // PLATEAU export declares: degree, degree, metre. A uniform millimetre
        // scale would quantise 0.001 DEGREE (~90-111 m) and destroy the data.
        let plateau = resolve_to_projjson("https://www.opengis.net/def/crs/EPSG/0/6697").unwrap();
        assert_eq!(
            axis_scale(&plateau).unwrap(),
            [NANO_DEGREE, NANO_DEGREE, MM]
        );

        // A 2D geographic CRS carries no vertical axis; z falls back to the
        // linear step rather than erroring, because a CityGML document may
        // declare a 2D `srsName` and still carry `srsDimension="3"` posLists.
        let two_d = resolve_to_projjson("EPSG:4326").unwrap();
        assert_eq!(axis_scale(&two_d).unwrap(), [NANO_DEGREE, NANO_DEGREE, MM]);

        // CRS84 is lon/lat degrees under a name rather than an EPSG code.
        let crs84 = resolve_to_projjson("OGC:CRS84").unwrap();
        assert_eq!(axis_scale(&crs84).unwrap(), [NANO_DEGREE, NANO_DEGREE, MM]);
    }

    #[test]
    fn a_linear_unit_is_converted_to_the_length_that_equals_one_millimetre() {
        // EPSG:2225 (NAD83 / California zone 1, US survey foot) — one of ~2 300
        // foot-, link- and chain-valued codes in the vendored table. Refusing
        // them over their unit would be refusing real national CRS for no
        // reason; the conversion factor is exact, so the step is too.
        let feet = resolve_to_projjson("EPSG:2225").unwrap();
        let scale = axis_scale(&feet).unwrap();
        let us_foot = 0.304_800_609_601_219;
        assert!((scale[0] - MM / us_foot).abs() < f64::EPSILON);
        assert!(
            (scale[0] * us_foot - MM).abs() < 1e-18,
            "one step must be 1 mm"
        );
        // No vertical axis: z keeps the linear default.
        assert_eq!(scale[2], MM);
    }

    #[test]
    fn axis_scale_refuses_a_unit_it_has_no_step_for() {
        // EPSG:4035 is in "degree minute second hemisphere" — a packed
        // sexagesimal spelling, not a linear scale, so it carries a
        // conversion factor that must NOT be applied.
        let sexagesimal = resolve_to_projjson("EPSG:4035").unwrap();
        let err = axis_scale(&sexagesimal).unwrap_err().to_string();
        assert!(err.contains("minute second"), "unexpected message: {err}");

        // A CRS object with no coordinate system at all is an error, never a
        // silent millimetre default.
        let axis_less = serde_json::json!({"type": "GeographicCRS", "name": "nonsense"});
        assert!(axis_scale(&axis_less).is_err());
    }

    #[test]
    fn latitude_first_is_read_from_the_declared_axis_directions() {
        // EPSG:6697 is (north, east) — latitude first. GeoParquet WKB is
        // always (longitude, latitude), so the writer must swap for it.
        let plateau = resolve_to_projjson("EPSG:6697").unwrap();
        assert!(is_latitude_first(&plateau));
        assert!(is_latitude_first(
            &resolve_to_projjson("EPSG:4326").unwrap()
        ));

        // CRS84 exists precisely to spell WGS 84 in lon/lat order.
        assert!(!is_latitude_first(
            &resolve_to_projjson("OGC:CRS84").unwrap()
        ));
        // A projected national CRS in (east, north) order.
        assert!(!is_latitude_first(
            &resolve_to_projjson("EPSG:7415").unwrap()
        ));
        assert!(!is_latitude_first(
            &resolve_to_projjson("EPSG:28992").unwrap()
        ));
    }

    #[test]
    fn axis_order_swap_is_its_own_inverse() {
        let plateau = resolve_to_projjson("EPSG:6697").unwrap();
        let order = AxisOrder::of(&plateau);
        assert_eq!(order, AxisOrder::LatLon);
        let source = [35.4, 139.6, 12.0];
        assert_eq!(order.apply(source), [139.6, 35.4, 12.0]);
        assert_eq!(order.apply(order.apply(source)), source);

        let dutch = AxisOrder::of(&resolve_to_projjson("EPSG:7415").unwrap());
        assert_eq!(dutch, AxisOrder::LonLat);
        assert_eq!(dutch.apply(source), source);
    }

    #[test]
    fn resolves_a_compound_national_crs() {
        // EPSG:7415 = Amersfoort/RD New + NAP height — a CompoundCRS.
        let crs = resolve_to_projjson("https://www.opengis.net/def/crs/EPSG/0/7415").unwrap();
        assert_eq!(crs["type"], "CompoundCRS");
        assert_eq!(crs["id"]["authority"], "EPSG");
        assert_eq!(crs["id"]["code"], 7415);
    }

    #[test]
    fn resolves_a_projected_crs() {
        let crs = resolve_to_projjson("EPSG:28992").unwrap();
        assert_eq!(crs["type"], "ProjectedCRS");
        assert_eq!(crs["id"]["code"], 28992);
    }

    #[test]
    fn unknown_code_is_an_error_not_a_silent_omission() {
        assert!(resolve_to_projjson("EPSG:999999999").is_err());
        assert!(resolve_to_projjson("garbage").is_err());
    }

    #[test]
    fn rejects_non_epsg_authority_even_with_a_numeric_code() {
        // sol-review G1: a numeric code under a NON-EPSG authority must not be
        // mis-read as EPSG (which would georeference under the wrong system).
        assert_eq!(
            lookup_key("https://www.opengis.net/def/crs/IGNF/0/7415"),
            None
        );
        assert!(resolve_to_projjson("https://www.opengis.net/def/crs/IGNF/0/7415").is_err());
        assert!(resolve_to_projjson("urn:ogc:def:crs:ESRI::102100").is_err());
    }

    #[test]
    fn passthrough_requires_a_projjson_crs_shape() {
        // sol-review G1: only a real PROJJSON CRS object (a `type` naming a CRS
        // variant) may short-circuit; an arbitrary object must not be emitted
        // verbatim as an (invalid) GeoParquet `crs`.
        assert!(resolve_to_projjson("{}").is_err());
        assert!(resolve_to_projjson(r#"{"type":"Feature"}"#).is_err());
        let projjson =
            r#"{"type":"GeographicCRS","name":"WGS 84","id":{"authority":"EPSG","code":4326}}"#;
        assert_eq!(
            resolve_to_projjson(projjson).unwrap()["type"],
            "GeographicCRS"
        );
    }
}
