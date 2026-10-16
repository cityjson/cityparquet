//! Detect a CityGML document, of any version, by its root element.
//!
//! Reads events only until the first `Start` element and checks it is a
//! `CityModel` in a CityGML core namespace. Any I/O or XML error (a JSON file,
//! say) means "not CityGML", so a caller can fall back to another format.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use quick_xml::events::Event;
use quick_xml::reader::NsReader;

use crate::xml::{NS_CITYGML_FAMILY, ns_eq, ns_is};

/// The CityGML 2.0 core namespace, the one a 2.0 `CityModel` is in.
pub const NS_CORE_2_0: &str = "http://www.opengis.net/citygml/2.0";
/// The CityGML 3.0 core namespace.
pub const NS_CORE_3_0: &str = "http://www.opengis.net/citygml/3.0";

/// Which CityGML version a document declares on its `CityModel` root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Version {
    V2_0,
    V3_0,
    /// Any other version, as its namespace spells it (`"1.0"`).
    Other(String),
}

/// The CityGML version of the document at `path`, or `None` when it is not
/// CityGML at all.
pub fn sniff(path: &Path) -> Option<Version> {
    sniff_from(BufReader::new(File::open(path).ok()?))
}

/// [`sniff`] over any `BufRead`. It consumes what it reads (the declaration
/// and the root element's start tag), so a streaming caller that still needs
/// those bytes records them and replays them ahead of the rest.
pub fn sniff_from<R: BufRead>(input: R) -> Option<Version> {
    let mut reader = NsReader::from_reader(input);
    let mut buf = Vec::new();
    // The root element appears within the first handful of events
    // (declaration, comments, then the root `Start`).
    for _ in 0..64 {
        buf.clear();
        match reader.read_resolved_event_into(&mut buf) {
            Ok((rr, Event::Start(e))) => {
                if e.local_name().as_ref() != b"CityModel" {
                    return None;
                }
                if ns_eq(&rr, NS_CORE_2_0.as_bytes()) {
                    return Some(Version::V2_0);
                }
                if ns_eq(&rr, NS_CORE_3_0.as_bytes()) {
                    return Some(Version::V3_0);
                }
                if ns_is(&rr, NS_CITYGML_FAMILY) {
                    return Some(Version::Other(version_from_ns(&rr)));
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
