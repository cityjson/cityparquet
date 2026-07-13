//! Shared quick-xml helpers for the CityGML reader: namespace matching,
//! attribute access, and subtree skipping/text collection.

use std::io::BufRead;

use cityparquet_schema::{CityParquetError, Result};
use quick_xml::events::{BytesStart, Event};
use quick_xml::name::ResolveResult;
use quick_xml::reader::NsReader;

/// GML namespace prefix — matches both GML 3.1 (`.../gml`) and 3.2
/// (`.../gml/3.2`), which real CityGML 2.0 files mix.
pub const NS_GML: &str = "http://www.opengis.net/gml";
pub const NS_BLDG: &str = "http://www.opengis.net/citygml/building/2.0";
/// CityGML 2.0 core namespace — used to sniff the `CityModel` root. Matching
/// this exact version (not the `.../citygml` family) means a 1.0/3.0 document is
/// not misclassified as CityGML 2.0 and then silently yielding no features.
pub const NS_CORE: &str = "http://www.opengis.net/citygml/2.0";

/// True when a resolved namespace starts with `prefix`.
pub fn ns_is(rr: &ResolveResult, prefix: &str) -> bool {
    matches!(rr, ResolveResult::Bound(ns) if ns.as_ref().starts_with(prefix.as_bytes()))
}

/// Fetch an attribute value (by its raw, possibly-prefixed name) as a String.
pub fn get_attr(e: &BytesStart, name: &[u8]) -> Option<String> {
    e.attributes().flatten().find_map(|a| {
        if a.key.as_ref() == name {
            std::str::from_utf8(a.value.as_ref())
                .ok()
                .map(str::to_owned)
        } else {
            None
        }
    })
}

/// A CityGML/GML `gml:id` (or bare `id`).
pub fn gml_id(e: &BytesStart) -> Option<String> {
    get_attr(e, b"gml:id").or_else(|| get_attr(e, b"id"))
}

/// The value of an attribute matched by its *local* name (prefix stripped), so
/// `xlink:href` is found regardless of the prefix the document binds.
pub fn get_attr_local(e: &BytesStart, local: &[u8]) -> Option<String> {
    e.attributes().flatten().find_map(|a| {
        let key = a.key.as_ref();
        let key_local = key.rsplit(|&b| b == b':').next().unwrap_or(key);
        if key_local == local {
            std::str::from_utf8(a.value.as_ref())
                .ok()
                .map(str::to_owned)
        } else {
            None
        }
    })
}

/// The `#fragment` target id of an `xlink:href` on this element, if present.
/// Errors on an external reference (`other.gml#id`) or a non-fragment href —
/// only intra-document references are resolvable by the streaming reader.
pub fn xlink_fragment(e: &BytesStart) -> Result<Option<String>> {
    let Some(href) = get_attr_local(e, b"href") else {
        return Ok(None);
    };
    match href.strip_prefix('#') {
        Some(id) if !id.is_empty() && !id.contains('#') => Ok(Some(id.to_string())),
        _ => Err(CityParquetError::Schema(format!(
            "CityGML xlink:href {href:?} is not an intra-document #fragment reference \
             (external/shared geometry is out of scope)"
        ))),
    }
}

pub fn xml_err(e: impl std::fmt::Display) -> CityParquetError {
    CityParquetError::Schema(format!("CityGML XML parse error: {e}"))
}

fn eof_err(ctx: &str) -> CityParquetError {
    CityParquetError::Schema(format!("unexpected end of CityGML document inside <{ctx}>"))
}

/// Consume the current element's remaining subtree. Call right after reading its
/// `Start`; returns once its matching `End` is consumed.
pub fn skip_element<R: BufRead>(reader: &mut NsReader<R>, buf: &mut Vec<u8>) -> Result<()> {
    let mut depth = 1usize;
    loop {
        buf.clear();
        match reader.read_event_into(buf).map_err(xml_err)? {
            Event::Start(_) => depth += 1,
            Event::End(_) => {
                depth -= 1;
                if depth == 0 {
                    return Ok(());
                }
            }
            Event::Eof => return Err(eof_err("skip")),
            _ => {}
        }
    }
}

/// Collect the text content of the current element (assumes only text children),
/// consuming through its matching `End`.
pub fn read_text<R: BufRead>(reader: &mut NsReader<R>, buf: &mut Vec<u8>) -> Result<String> {
    let mut s = String::new();
    loop {
        buf.clear();
        match reader.read_event_into(buf).map_err(xml_err)? {
            Event::Text(t) => s.push_str(&t.unescape().map_err(xml_err)?),
            Event::CData(t) => s.push_str(&String::from_utf8_lossy(&t)),
            Event::End(_) => return Ok(s),
            Event::Eof => return Err(eof_err("text")),
            _ => {}
        }
    }
}
