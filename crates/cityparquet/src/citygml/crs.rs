//! `srsName` resolution for the CityGML 2.0 reader.
//!
//! Policy (see the reader module docs): the pipeline never reprojects —
//! coordinates are quantised as-is and the CRS is provenance only — so we
//! accept by default and reject only *provably wrong* input:
//!
//! - Resolve `srsName` to an OGC EPSG URL only for names we understand (the
//!   three EPSG syntaxes plus the German AdV compound URNs, matched exactly). A
//!   name we cannot parse advertises no CRS — the reader does not invent one
//!   (mis-advertising a wrong CRS is worse than advertising none). Preserving
//!   the raw `srsName` as a provenance field is a later enhancement.
//! - Hard-reject a name that resolves to a known **geographic** (degree) CRS:
//!   the fixed 1 mm quantisation would destroy degree coordinates. The
//!   geographic list is common-but-not-exhaustive; an unlisted geographic code
//!   is a documented residual limitation (no coordinate-magnitude sniffing —
//!   real fixtures use small local metre coordinates near the origin).

use cityparquet_schema::{CityParquetError, Result};
use cjseq::ReferenceSystem;

/// Outcome of resolving a CityGML `srsName`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CrsResolution {
    /// Resolved to a projected/compound EPSG code we advertise as a CRS.
    Epsg(String),
    /// A syntactically understood name we choose not to advertise (kept as
    /// provenance only).
    Unresolved,
}

/// Known geographic (degree-valued) EPSG codes; the 1 mm quantiser would
/// destroy their coordinates, so a file that *declares* one is rejected. Common
/// 2D/3D geographic CRS (WGS 84, ETRS89, NAD27/83, DHDN, ...); not exhaustive.
const GEOGRAPHIC_EPSG: &[&str] = &[
    "4326", "4258", "4269", "4267", "4283", "4171", "4173", "4207", "4230", "4312", "4314", "4619",
    "4674", "4759", "4979", "4937", "4936", "4896", "4327", "4329",
];

/// Resolve a raw `srsName`. Errors only when it resolves to a known geographic
/// CRS (an unrepresentable-for-us profile violation).
pub fn resolve(srs_name: &str) -> Result<CrsResolution> {
    let name = srs_name.trim();
    // OGC:CRS84 is lon/lat degrees, expressed as a name rather than an EPSG code.
    if name.to_ascii_uppercase().contains("CRS84") {
        return Err(geographic_err(srs_name, "CRS84"));
    }
    let Some(code) = epsg_code(name) else {
        return Ok(CrsResolution::Unresolved);
    };
    if GEOGRAPHIC_EPSG.contains(&code.as_str()) {
        return Err(geographic_err(srs_name, &code));
    }
    Ok(CrsResolution::Epsg(code))
}

fn geographic_err(srs_name: &str, code: &str) -> CityParquetError {
    CityParquetError::Schema(format!(
        "CityGML srsName {srs_name:?} resolves to geographic CRS {code}; the reader only \
         supports projected (metre-based) CRS (coordinates are quantised at 1 mm, which \
         would destroy degrees)"
    ))
}

/// Build the CityJSON `ReferenceSystem` (OGC EPSG URL) for a resolved code.
pub fn reference_system(code: &str) -> ReferenceSystem {
    ReferenceSystem::new(None, "EPSG".to_string(), "0".to_string(), code.to_string())
}

/// Map an `srsName` to a bare EPSG code, or `None` if we do not understand it.
fn epsg_code(name: &str) -> Option<String> {
    // German AdV URNs used pervasively by real CityGML 2.0 data. `*..._NH`
    // denotes a compound (horizontal + DHHN92 height) CRS with its own EPSG
    // code; the bare form is the horizontal code. Matched exactly so an
    // unrecognised vertical/zone falls through to `None` rather than a guess.
    if let Some(rest) = name.strip_prefix("urn:adv:crs:") {
        return match rest {
            "ETRS89_UTM32*DE_DHHN92_NH" => Some("5555".to_string()),
            "ETRS89_UTM33*DE_DHHN92_NH" => Some("5556".to_string()),
            "ETRS89_UTM32" => Some("25832".to_string()),
            "ETRS89_UTM33" => Some("25833".to_string()),
            _ => None,
        };
    }

    // EPSG syntaxes: `EPSG:25832`, `urn:ogc:def:crs:EPSG::25832`,
    // `urn:ogc:def:crs:EPSG:8.9:25832`, and the opengis URL form. Tokenise on
    // `:`/`/` and require an *exact* `EPSG` authority token (so `not-EPSG:1` is
    // rejected); the code is the last all-digit token.
    let tokens: Vec<&str> = name.split([':', '/']).filter(|t| !t.is_empty()).collect();
    if !tokens.iter().any(|t| t.eq_ignore_ascii_case("EPSG")) {
        return None;
    }
    tokens
        .iter()
        .rev()
        .find(|t| t.len() >= 4 && t.chars().all(|c| c.is_ascii_digit()))
        .map(|t| t.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epsg_short_and_urn_and_url_forms() {
        assert_eq!(
            resolve("EPSG:25832").unwrap(),
            CrsResolution::Epsg("25832".into())
        );
        assert_eq!(
            resolve("urn:ogc:def:crs:EPSG::25833").unwrap(),
            CrsResolution::Epsg("25833".into())
        );
        assert_eq!(
            resolve("urn:ogc:def:crs:EPSG:8.9:28992").unwrap(),
            CrsResolution::Epsg("28992".into())
        );
        assert_eq!(
            resolve("https://www.opengis.net/def/crs/EPSG/0/7415").unwrap(),
            CrsResolution::Epsg("7415".into())
        );
    }

    #[test]
    fn adv_urns_map_to_compound_or_horizontal_epsg() {
        assert_eq!(
            resolve("urn:adv:crs:ETRS89_UTM32*DE_DHHN92_NH").unwrap(),
            CrsResolution::Epsg("5555".into())
        );
        assert_eq!(
            resolve("urn:adv:crs:ETRS89_UTM33*DE_DHHN92_NH").unwrap(),
            CrsResolution::Epsg("5556".into())
        );
        assert_eq!(
            resolve("urn:adv:crs:ETRS89_UTM32").unwrap(),
            CrsResolution::Epsg("25832".into())
        );
    }

    #[test]
    fn geographic_crs_is_rejected() {
        assert!(resolve("EPSG:4326").is_err());
        assert!(resolve("urn:ogc:def:crs:EPSG::4979").is_err());
        // NAD27 (4267) is geographic/degrees — must not slip through as projected.
        assert!(resolve("EPSG:4267").is_err());
        assert!(resolve("urn:ogc:def:crs:OGC:1.3:CRS84").is_err());
    }

    #[test]
    fn unknown_names_are_unresolved_not_errors() {
        assert_eq!(
            resolve("urn:adv:crs:GK_3").unwrap(),
            CrsResolution::Unresolved
        );
        // An unrecognised AdV zone/vertical must not be guessed into an EPSG code.
        assert_eq!(
            resolve("urn:adv:crs:ETRS89_UTM32*SOMETHING_ELSE").unwrap(),
            CrsResolution::Unresolved
        );
        assert_eq!(
            resolve("some-local-engineering-crs").unwrap(),
            CrsResolution::Unresolved
        );
        // A stray "EPSG" substring that is not the authority token is not a CRS.
        assert_eq!(
            resolve("not-EPSG:25832").unwrap(),
            CrsResolution::Unresolved
        );
    }

    #[test]
    fn reference_system_builds_opengis_epsg_url() {
        assert_eq!(
            reference_system("5555").to_url(),
            "https://www.opengis.net/def/crs/EPSG/0/5555"
        );
    }
}
