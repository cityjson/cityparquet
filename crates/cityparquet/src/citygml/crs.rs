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

// The known-geographic EPSG list this reader rejects lives with the
// EPSG->PROJJSON table in `cityparquet_schema::crs`: it governs CityJSON input
// and the CLI's `--crs` override as much as it governs a CityGML `srsName`, so
// it is not CityGML-specific policy.
use cityparquet_schema::crs::is_geographic_epsg;
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
    if is_geographic_epsg(&code) {
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

/// Map the package CRS metadata (an OGC EPSG URL string, or a PROJJSON object
/// with an `id.authority`/`id.code`) to a validated writer `srsName`.
///
/// `None` CRS -> `Ok(None)` (no CRS to advertise). Otherwise the candidate
/// EPSG code is built into a `urn:ogc:def:crs:EPSG::<code>` and round-tripped
/// through [`resolve`], which must hand back the *same* projected code — this
/// reuses the reader's geographic-CRS rejection (and its exact-syntax
/// parsing) so the writer can never emit an `srsName` the reader would refuse
/// to consume. A geographic code (e.g. 4326), an unsupported/non-EPSG
/// authority, or anything else `resolve` cannot parse back to the same code
/// is an error.
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
    fn srs_name_geographic_crs_errors() {
        let crs = serde_json::json!("https://www.opengis.net/def/crs/EPSG/0/4326");
        assert!(srs_name_for(Some(&crs)).is_err());
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
