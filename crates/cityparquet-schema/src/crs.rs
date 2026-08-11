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

/// Known geographic (degree-valued) EPSG codes. Nothing in this stack
/// reprojects, and coordinates are quantised at millimetre scale (the CityGML
/// reader at a fixed 1 mm), so a degree coordinate would be destroyed —
/// declaring one of these is refused rather than silently mis-encoded. Common
/// 2D/3D geographic CRS (WGS 84, ETRS89, NAD27/83, DHDN, ...); not exhaustive,
/// so an unlisted geographic code is a documented residual limitation (no
/// coordinate-magnitude sniffing: guessing is exactly what the spec's CRS
/// rules forbid).
const GEOGRAPHIC_EPSG: &[&str] = &[
    "4326", "4258", "4269", "4267", "4283", "4171", "4173", "4207", "4230", "4312", "4314", "4619",
    "4674", "4759", "4979", "4937", "4936", "4896", "4327", "4329",
];

/// Whether `code` (a bare EPSG code, e.g. `"4326"`) names a known geographic
/// CRS — see [`GEOGRAPHIC_EPSG`]. Used by the CityGML `srsName` resolver and by
/// the operator-supplied `--crs` override alike, which is why it lives here
/// beside the EPSG table rather than in either consumer.
pub fn is_geographic_epsg(code: &str) -> bool {
    GEOGRAPHIC_EPSG.contains(&code)
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
    fn known_geographic_codes_are_recognised_as_such() {
        // Degree-valued: WGS 84, ETRS89, NAD27 — refused by every consumer,
        // since nothing here reprojects and the quantiser is sized for metres.
        assert!(is_geographic_epsg("4326"));
        assert!(is_geographic_epsg("4258"));
        assert!(is_geographic_epsg("4267"));
        // Projected/compound national CRS must NOT be caught by it.
        assert!(!is_geographic_epsg("28992"));
        assert!(!is_geographic_epsg("7415"));
        assert!(!is_geographic_epsg("31256"));
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
