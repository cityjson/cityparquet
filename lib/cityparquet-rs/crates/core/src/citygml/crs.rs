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
//! - A **geographic** (degree-valued) name resolves like any other. The
//!   quantisation step is no longer a fixed millimetre: it is derived per axis
//!   from the resolved CRS's own declared units
//!   ([`cityparquet_schema::crs::axis_scale`]), so a degree axis is quantised
//!   at a degree-sized step. A CRS whose units the encoder has no step for
//!   fails there, loudly, rather than being refused from a hand-maintained
//!   list here.

use cityparquet_schema::{CityParquetError, Result};
use cjseq::ReferenceSystem;

/// Outcome of resolving a CityGML `srsName`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CrsResolution {
    /// Resolved to an EPSG code we advertise as a CRS.
    Epsg(String),
    /// `OGC:CRS84` — WGS 84 in longitude/latitude order. It is a real,
    /// resolvable CRS with no EPSG code of its own, so it cannot travel as
    /// [`Self::Epsg`]; a resolver that only looked for a code would report it
    /// as unresolved and quantise a degree document at a metre step.
    Crs84,
    /// A syntactically understood name we choose not to advertise (kept as
    /// provenance only).
    Unresolved,
}

/// Resolve a raw `srsName` to the EPSG code it names, or [`CrsResolution::Unresolved`]
/// for a syntactically understood name we choose not to advertise.
pub fn resolve(srs_name: &str) -> Result<CrsResolution> {
    let name = srs_name.trim();
    // CRS84 is spelled as a name, never as an EPSG code, so it is matched
    // before the code parsing below can fail to find one.
    if name.to_ascii_uppercase().contains("CRS84") {
        return Ok(CrsResolution::Crs84);
    }
    let Some(code) = epsg_code(name) else {
        return Ok(CrsResolution::Unresolved);
    };
    Ok(CrsResolution::Epsg(code))
}

/// The `OGC:CRS84` reference system, for a document that names it.
pub fn reference_system_crs84() -> ReferenceSystem {
    ReferenceSystem::new(
        None,
        "OGC".to_string(),
        "1.3".to_string(),
        "CRS84".to_string(),
    )
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

/// Map the package CRS metadata (an OGC EPSG URL string, or a PROJJSON object
/// with an `id.authority`/`id.code`) to a validated writer `srsName`.
///
/// `None` CRS -> `Ok(None)` (no CRS to advertise). Otherwise the candidate
/// EPSG code is built into a `urn:ogc:def:crs:EPSG::<code>` and round-tripped
/// through [`resolve`], which must hand back the *same* code — reusing the
/// reader's exact-syntax parsing so the writer can never emit an `srsName` the
/// reader would refuse to consume. An unsupported/non-EPSG authority, or
/// anything else `resolve` cannot parse back to the same code, is an error.
pub fn srs_name_for(crs: Option<&serde_json::Value>) -> Result<Option<String>> {
    let Some(crs) = crs else {
        return Ok(None);
    };
    let code = extract_epsg_code(crs).ok_or_else(|| {
        CityParquetError::Schema(format!(
            "package CRS {crs:?} is not a recognised EPSG identifier (expected an OGC EPSG \
             URL or a PROJJSON object with an EPSG authority)"
        ))
    })?;
    let urn = format!("urn:ogc:def:crs:EPSG::{code}");
    match resolve(&urn)? {
        CrsResolution::Epsg(resolved) if resolved == code => Ok(Some(urn)),
        _ => Err(CityParquetError::Schema(format!(
            "package CRS EPSG:{code} did not round-trip through the reader's srsName resolver"
        ))),
    }
}

/// Extract a bare EPSG code from the package CRS metadata: either an OGC EPSG
/// URL string (reusing [`epsg_code`]'s parsing) or a PROJJSON object with
/// `id.authority == "EPSG"` and a numeric (or all-digit string) `id.code`.
fn extract_epsg_code(crs: &serde_json::Value) -> Option<String> {
    if let Some(s) = crs.as_str() {
        return epsg_code(s);
    }
    let id = crs.as_object()?.get("id")?;
    let authority = id.get("authority")?.as_str()?;
    if !authority.eq_ignore_ascii_case("EPSG") {
        return None;
    }
    match id.get("code")? {
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::String(s) if !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()) => {
            Some(s.clone())
        }
        _ => None,
    }
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
    fn crs84_resolves_to_its_own_variant() {
        // CRS84 is WGS 84 in longitude/latitude order, spelled as a name
        // rather than an EPSG code — so it carries no code to parse, and a
        // resolver that only looks for one silently reports "no CRS" and
        // quantises a degree document at a metre step.
        assert_eq!(
            resolve("urn:ogc:def:crs:OGC:1.3:CRS84").unwrap(),
            CrsResolution::Crs84
        );
        assert_eq!(
            resolve("http://www.opengis.net/def/crs/OGC/1.3/CRS84").unwrap(),
            CrsResolution::Crs84
        );
        assert_eq!(
            reference_system_crs84().to_url(),
            "https://www.opengis.net/def/crs/OGC/1.3/CRS84"
        );
    }

    #[test]
    fn geographic_crs_resolves_like_any_other() {
        // A degree-valued CRS is no longer refused here: the quantisation step
        // is derived from its declared axis units downstream, so it encodes
        // correctly rather than not at all. EPSG:6697 is what every PLATEAU
        // (Japan) export declares.
        assert_eq!(
            resolve("https://www.opengis.net/def/crs/EPSG/0/6697").unwrap(),
            CrsResolution::Epsg("6697".into())
        );
        assert_eq!(
            resolve("EPSG:4326").unwrap(),
            CrsResolution::Epsg("4326".into())
        );
        assert_eq!(
            resolve("urn:ogc:def:crs:EPSG::4979").unwrap(),
            CrsResolution::Epsg("4979".into())
        );
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

    #[test]
    fn srs_name_from_epsg_url_round_trips_through_resolve() {
        let crs = serde_json::json!("https://www.opengis.net/def/crs/EPSG/0/28992");
        let srs = srs_name_for(Some(&crs)).unwrap().unwrap();
        assert_eq!(srs, "urn:ogc:def:crs:EPSG::28992");
        // The emitted srsName must resolve back to the SAME code.
        assert!(matches!(resolve(&srs).unwrap(), CrsResolution::Epsg(c) if c == "28992"));
    }

    #[test]
    fn srs_name_from_projjson_epsg_object() {
        let crs = serde_json::json!({ "id": { "authority": "EPSG", "code": 28992 } });
        assert_eq!(
            srs_name_for(Some(&crs)).unwrap().unwrap(),
            "urn:ogc:def:crs:EPSG::28992"
        );
    }

    #[test]
    fn srs_name_none_when_no_crs() {
        assert_eq!(srs_name_for(None).unwrap(), None);
    }

    #[test]
    fn srs_name_round_trips_a_geographic_crs() {
        let crs = serde_json::json!("https://www.opengis.net/def/crs/EPSG/0/6697");
        assert_eq!(
            srs_name_for(Some(&crs)).unwrap().as_deref(),
            Some("urn:ogc:def:crs:EPSG::6697")
        );
    }

    #[test]
    fn srs_name_non_epsg_authority_errors() {
        let crs = serde_json::json!({ "id": { "authority": "ESRI", "code": 102100 } });
        assert!(srs_name_for(Some(&crs)).is_err());
    }

    #[test]
    fn srs_name_unparseable_crs_errors() {
        let crs = serde_json::json!("some-local-engineering-crs");
        assert!(srs_name_for(Some(&crs)).is_err());
    }
}
