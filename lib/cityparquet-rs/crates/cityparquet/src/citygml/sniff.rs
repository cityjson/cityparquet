//! Detect a CityGML document — of any version — by its root element.
//!
//! Reads events only until the first `Start` element and checks it is a
//! `CityModel` in a CityGML core namespace. The CityGML **2.0** namespace is
//! matched first, because that is the only version this reader supports; a
//! `CityModel` in any other `.../citygml/*` namespace is reported as
//! [`CityGmlVersion::Other`] so the caller can name the version rather than
//! letting an XML file fall through to the CityJSON sniff and fail as
//! malformed JSON. Any I/O or XML error (e.g. a genuine JSON file) simply means
//! "not CityGML", so the caller falls back to CityJSON as before.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use quick_xml::events::Event;
use quick_xml::reader::NsReader;

use super::xml::{NS_CITYGML_FAMILY, NS_CORE, ns_is};

/// Which CityGML version a document declares on its `CityModel` root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CityGmlVersion {
    /// CityGML 2.0 — the only version this reader supports.
    V2_0,
    /// Any other CityGML version, as it appears in the namespace (e.g. "1.0").
    Other(String),
}

/// Detect a CityGML document of any version by its root element.
///
/// `None` means "not CityGML at all" (so the caller falls back to CityJSON);
/// `Some(Other(v))` means "CityGML, but not a version we read" — which the
/// caller must report as a version error rather than a JSON parse failure.
pub fn sniff_citygml(path: &Path) -> Option<CityGmlVersion> {
    let file = File::open(path).ok()?;
    let mut reader = NsReader::from_reader(BufReader::new(file));
    let mut buf = Vec::new();
    // Bound the scan: the root element appears within the first handful of
    // events (declaration, comments, then the root Start).
    for _ in 0..64 {
        buf.clear();
        match reader.read_resolved_event_into(&mut buf) {
            Ok((rr, Event::Start(e))) => {
                if e.local_name().as_ref() != b"CityModel" {
                    return None;
                }
                // NS_CORE first: `ns_is` is a prefix match, so the family
                // namespace also matches 2.0 and would shadow it.
                if ns_is(&rr, NS_CORE) {
                    return Some(CityGmlVersion::V2_0);
                }
                if ns_is(&rr, NS_CITYGML_FAMILY) {
                    return Some(CityGmlVersion::Other(version_from_ns(&rr)));
                }
                return None;
            }
            Ok((_, Event::Eof)) | Err(_) => return None,
            Ok(_) => {}
        }
    }
    None
}

/// The trailing version segment of a CityGML core namespace
/// (`http://www.opengis.net/citygml/1.0` -> `"1.0"`), or `"unknown"`.
fn version_from_ns(rr: &quick_xml::name::ResolveResult) -> String {
    let quick_xml::name::ResolveResult::Bound(ns) = rr else {
        return "unknown".to_string();
    };
    std::str::from_utf8(ns.as_ref())
        .ok()
        .and_then(|s| s.rsplit('/').next())
        .filter(|s| !s.is_empty())
        .unwrap_or("unknown")
        .to_string()
}

/// True when `path` is a CityGML **2.0** document. Retained for callers that
/// only need the yes/no answer; [`sniff_citygml`] carries the version.
pub fn is_citygml(path: &Path) -> bool {
    matches!(sniff_citygml(path), Some(CityGmlVersion::V2_0))
}
