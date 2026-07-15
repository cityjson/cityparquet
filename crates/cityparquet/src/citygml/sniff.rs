//! Detect a CityGML 2.0 document by its root element.
//!
//! Reads events only until the first `Start` element and checks it is a
//! `CityModel` in the CityGML **2.0** core namespace. A 1.0/3.0 document is not
//! matched (so it is not misread as 2.0). Any I/O or XML error (e.g. a JSON
//! file) simply means "not CityGML", so the caller falls back to CityJSON.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use quick_xml::events::Event;
use quick_xml::reader::NsReader;

use super::xml::{NS_CORE, ns_is};

/// True when `path` is a CityGML 2.0 document (root `CityModel` in the CityGML
/// 2.0 core namespace). Never errors — a negative result routes to the CityJSON
/// sniff.
pub fn is_citygml(path: &Path) -> bool {
    let Ok(file) = File::open(path) else {
        return false;
    };
    let mut reader = NsReader::from_reader(BufReader::new(file));
    let mut buf = Vec::new();
    // Bound the scan: the root element appears within the first handful of
    // events (declaration, comments, then the root Start).
    for _ in 0..64 {
        buf.clear();
        match reader.read_resolved_event_into(&mut buf) {
            Ok((rr, Event::Start(e))) => {
                return e.local_name().as_ref() == b"CityModel" && ns_is(&rr, NS_CORE);
            }
            Ok((_, Event::Eof)) | Err(_) => return false,
            Ok(_) => {}
        }
    }
    false
}
