//! The CRS a document's `srsName` names, and the step its coordinates are
//! quantised at.
//!
//! Coordinates are never reprojected: the CRS is provenance, and what it
//! decides is the quantisation step. An `srsName` resolves only when its
//! syntax is understood (the EPSG spellings, the German AdV URNs matched
//! exactly, and `CRS84`); any other names no CRS, since advertising a wrong
//! one is worse than advertising none.
//!
//! The step is per axis, from the units the CRS declares: a metre axis is
//! quantised at a millimetre ([`MM`]), a degree axis at a nanodegree
//! ([`NANO_DEGREE`]), and any other unit at what equals one of them. A
//! uniform millimetre would quantise EPSG:6697, PLATEAU's latitude and
//! longitude in degrees, at 0.001 degree, 90 to 111 m. The steps of every
//! CRS whose axes are not all metres are a table generated from the EPSG
//! dataset (`steps.rs`).

mod steps;

use crate::{Error, Result};

/// The quantisation step of a metre axis: one millimetre.
pub const MM: f64 = 1e-3;

/// The quantisation step of a degree axis: 1e-9 degree, about 0.11 mm of
/// latitude and never coarser in longitude.
pub const NANO_DEGREE: f64 = 1e-9;

/// A CRS an `srsName` names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Crs {
    /// An EPSG CRS, by its code.
    Epsg(String),
    /// `OGC:CRS84`, WGS 84 in longitude and latitude order, which has no
    /// EPSG code of its own.
    Crs84,
    /// `OGC:CRS84h`, `OGC:CRS84` with an ellipsoidal height.
    Crs84h,
}

/// The CRS `srs_name` names, or `None` when its syntax is not understood.
pub fn resolve(srs_name: &str) -> Option<Crs> {
    let name = srs_name.trim();
    // CRS84 is spelled as a name, never as an EPSG code.
    let upper = name.to_ascii_uppercase();
    if upper.contains("CRS84H") {
        return Some(Crs::Crs84h);
    }
    if upper.contains("CRS84") {
        return Some(Crs::Crs84);
    }
    epsg_code(name).map(Crs::Epsg)
}

/// The bare EPSG code an `srsName` names, or `None`.
fn epsg_code(name: &str) -> Option<String> {
    // The German AdV URNs of real CityGML 2.0 data. `*..._NH` is a compound
    // (horizontal and DHHN92 height) CRS with its own EPSG code; the bare
    // form is the horizontal code. Matched exactly, so an unknown zone or
    // vertical names nothing rather than a guess.
    if let Some(rest) = name.strip_prefix("urn:adv:crs:") {
        return match rest {
            "ETRS89_UTM32*DE_DHHN92_NH" => Some("5555".to_string()),
            "ETRS89_UTM33*DE_DHHN92_NH" => Some("5556".to_string()),
            "ETRS89_UTM32" => Some("25832".to_string()),
            "ETRS89_UTM33" => Some("25833".to_string()),
            _ => None,
        };
    }

    // `EPSG:25832`, `urn:ogc:def:crs:EPSG::25832`,
    // `urn:ogc:def:crs:EPSG:8.9:25832` and the opengis URL: an exact `EPSG`
    // authority token (so `not-EPSG:1` names nothing), and the code is the
    // last all-digit token.
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

impl Crs {
    /// The key the EPSG dataset knows the CRS by: its code, `OGC:CRS84` or
    /// `OGC:CRS84h`.
    pub fn key(&self) -> &str {
        match self {
            Self::Epsg(code) => code,
            Self::Crs84 => "OGC:CRS84",
            Self::Crs84h => "OGC:CRS84h",
        }
    }

    /// The quantisation step of each axis, from the units the CRS declares.
    /// A CRS with two axes has a metre height; one the EPSG dataset does not
    /// know is quantised at a millimetre, as a document naming no CRS is.
    pub fn scale(&self) -> Result<[f64; 3]> {
        scale_of(self.key())
    }

    /// The CRS as a CityJSON `referenceSystem`, at the version the OGC
    /// registers it under.
    #[cfg(feature = "cjseq")]
    pub fn reference_system(&self) -> cjseq::ReferenceSystem {
        match self {
            Self::Epsg(code) => {
                cjseq::ReferenceSystem::new(None, "EPSG".into(), "0".into(), code.clone())
            }
            Self::Crs84 => {
                cjseq::ReferenceSystem::new(None, "OGC".into(), "1.3".into(), "CRS84".into())
            }
            Self::Crs84h => {
                cjseq::ReferenceSystem::new(None, "OGC".into(), "0".into(), "CRS84h".into())
            }
        }
    }
}

/// The quantisation step of each axis of the CRS the EPSG dataset knows by
/// `key` (an EPSG code, `OGC:CRS84` or `OGC:CRS84h`). A key the dataset does
/// not know, or whose axes are all metres, has a millimetre on each. A CRS
/// with an axis in a unit that has no step (EPSG's packed sexagesimal
/// degrees) is refused.
pub fn scale_of(key: &str) -> Result<[f64; 3]> {
    let code = match key {
        "OGC:CRS84" => return Ok(steps::CRS84),
        "OGC:CRS84h" => return Ok(steps::CRS84H),
        code => match code.parse::<u32>() {
            Ok(code) => code,
            Err(_) => return Ok([MM; 3]),
        },
    };
    if let Ok(i) = steps::CODES.binary_search_by_key(&code, |(c, _)| *c) {
        return Ok(steps::STEPS[usize::from(steps::CODES[i].1)]);
    }
    if let Ok(i) = steps::REFUSED.binary_search_by_key(&code, |(c, _)| *c) {
        return Err(Error::invalid(format!(
            "CRS EPSG:{code}: {}",
            steps::REFUSED[i].1
        )));
    }
    Ok([MM; 3])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn epsg(code: &str) -> Option<Crs> {
        Some(Crs::Epsg(code.into()))
    }

    #[test]
    fn the_epsg_spellings_name_their_code() {
        assert_eq!(resolve("EPSG:25832"), epsg("25832"));
        assert_eq!(resolve("urn:ogc:def:crs:EPSG::25833"), epsg("25833"));
        assert_eq!(resolve("urn:ogc:def:crs:EPSG:8.9:28992"), epsg("28992"));
        assert_eq!(
            resolve("https://www.opengis.net/def/crs/EPSG/0/7415"),
            epsg("7415")
        );
        assert_eq!(
            resolve("https://www.opengis.net/def/crs/EPSG/0/6697"),
            epsg("6697")
        );
    }

    #[test]
    fn an_adv_urn_names_its_compound_or_horizontal_code() {
        assert_eq!(
            resolve("urn:adv:crs:ETRS89_UTM32*DE_DHHN92_NH"),
            epsg("5555")
        );
        assert_eq!(
            resolve("urn:adv:crs:ETRS89_UTM33*DE_DHHN92_NH"),
            epsg("5556")
        );
        assert_eq!(resolve("urn:adv:crs:ETRS89_UTM32"), epsg("25832"));
    }

    #[test]
    fn crs84_is_named_without_a_code() {
        assert_eq!(resolve("urn:ogc:def:crs:OGC:1.3:CRS84"), Some(Crs::Crs84));
        assert_eq!(resolve("urn:ogc:def:crs:OGC:0:CRS84h"), Some(Crs::Crs84h));
        assert_eq!(
            resolve("http://www.opengis.net/def/crs/OGC/1.3/CRS84"),
            Some(Crs::Crs84)
        );
    }

    #[cfg(feature = "cjseq")]
    #[test]
    fn crs84_is_advertised_at_the_version_the_ogc_registers_it_under() {
        assert_eq!(
            Crs::Crs84.reference_system().to_url(),
            "https://www.opengis.net/def/crs/OGC/1.3/CRS84"
        );
        assert_eq!(
            Crs::Crs84h.reference_system().to_url(),
            "https://www.opengis.net/def/crs/OGC/0/CRS84h"
        );
    }

    #[test]
    fn a_name_not_understood_names_no_crs() {
        assert_eq!(resolve("urn:adv:crs:GK_3"), None);
        assert_eq!(resolve("urn:adv:crs:ETRS89_UTM32*SOMETHING_ELSE"), None);
        assert_eq!(resolve("some-local-engineering-crs"), None);
        assert_eq!(resolve("not-EPSG:25832"), None);
    }

    #[test]
    fn the_step_follows_the_declared_units() {
        // Degrees over metres (JGD2011 + height), metres (RD New + NAP),
        // degrees with no height (CRS84), and a CRS no dataset knows.
        assert_eq!(scale_of("6697").unwrap(), [NANO_DEGREE, NANO_DEGREE, MM]);
        assert_eq!(scale_of("7415").unwrap(), [MM; 3]);
        assert_eq!(
            scale_of("OGC:CRS84").unwrap(),
            [NANO_DEGREE, NANO_DEGREE, MM]
        );
        assert_eq!(scale_of("999999").unwrap(), [MM; 3]);
    }

    #[test]
    fn a_foot_axis_is_quantised_at_a_millimetre_s_length() {
        // NAD83 / California zone 3 (ftUS): a US survey foot is 0.3048006 m.
        let [x, y, z] = scale_of("2227").unwrap();
        assert!((x * 1200.0 / 3937.0 - MM).abs() < 1e-15, "{x}");
        assert_eq!(x, y);
        assert_eq!(z, MM);
    }

    #[test]
    fn a_sexagesimal_axis_is_refused() {
        // EPSG:4035 is in EPSG's packed sexagesimal degrees.
        let refused = scale_of("4035").unwrap_err().to_string();
        assert!(
            refused.contains("degree minute second hemisphere"),
            "{refused}"
        );
    }

    #[test]
    fn the_steps_are_sorted_for_lookup() {
        assert!(steps::CODES.windows(2).all(|w| w[0].0 < w[1].0));
        assert!(steps::REFUSED.windows(2).all(|w| w[0].0 < w[1].0));
    }
}
