//! quick-xml helpers shared by the CityGML readers: namespace matching,
//! attribute access, and subtree skipping and text collection.
//!
//! Every reader runs quick-xml with `expand_empty_elements`, so a
//! self-closing `<gml:surfaceMember xlink:href="#a"/>` arrives as a `Start`
//! and an `End`, and the `Start`-matching loops see its xlink.

use std::io::BufRead;

use quick_xml::events::{BytesStart, Event};
use quick_xml::name::ResolveResult;
use quick_xml::reader::NsReader;

use crate::{Error, Result};

/// GML namespace prefix. It matches both GML 3.1 (`.../gml`) and 3.2
/// (`.../gml/3.2`), which real CityGML 2.0 files mix.
pub const NS_GML: &str = "http://www.opengis.net/gml";

/// The CityGML namespace *family*: every version and module shares this
/// prefix (`.../citygml/2.0`, `.../citygml/building/3.0`, …).
pub const NS_CITYGML_FAMILY: &str = "http://www.opengis.net/citygml";

/// True when a resolved namespace starts with `prefix`.
pub fn ns_is(rr: &ResolveResult, prefix: &str) -> bool {
    matches!(rr, ResolveResult::Bound(ns) if ns.as_ref().starts_with(prefix.as_bytes()))
}

/// The resolved namespace of an element, as bytes; `None` when unbound.
///
/// Used where a prefix match is too loose: a feature's boundary surfaces are
/// in **that feature's own module** namespace, and matching the CityGML family
/// instead would also admit a `gen:`/`app:` child or an ADE hook.
pub fn ns_of(rr: &ResolveResult) -> Option<Vec<u8>> {
    match rr {
        ResolveResult::Bound(ns) => Some(ns.as_ref().to_vec()),
        _ => None,
    }
}

/// True when a resolved namespace is exactly `ns` (see [`ns_of`]).
pub fn ns_eq(rr: &ResolveResult, ns: &[u8]) -> bool {
    matches!(rr, ResolveResult::Bound(bound) if bound.as_ref() == ns)
}

/// An attribute value, by its raw (possibly prefixed) name.
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

/// A `gml:id` (or bare `id`).
pub fn gml_id(e: &BytesStart) -> Option<String> {
    get_attr(e, b"gml:id").or_else(|| get_attr(e, b"id"))
}

/// An attribute value matched by its *local* name (prefix stripped), so
/// `xlink:href` is found whatever prefix the document binds.
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
/// Errors on an external reference (`other.gml#id`) or a non-fragment href:
/// only intra-document references are resolvable by a streaming reader.
pub fn xlink_fragment(e: &BytesStart) -> Result<Option<String>> {
    let Some(href) = get_attr_local(e, b"href") else {
        return Ok(None);
    };
    match href.strip_prefix('#') {
        Some(id) if !id.is_empty() && !id.contains('#') => Ok(Some(id.to_string())),
        _ => Err(Error::invalid(format!(
            "CityGML xlink:href {href:?} is not an intra-document #fragment reference \
             (external/shared geometry is out of scope)"
        ))),
    }
}

/// A quick-xml failure as an [`Error`].
pub fn xml_err(e: impl std::fmt::Display) -> Error {
    Error::invalid(format!("CityGML XML parse error: {e}"))
}

/// The error for a document that ends inside `<ctx>`.
pub fn unexpected_eof(ctx: &str) -> Error {
    Error::invalid(format!("unexpected end of CityGML document inside <{ctx}>"))
}

/// The decoded, entity-unescaped content of a `BytesText` event.
///
/// quick-xml 0.39 split `BytesText::unescape` into `decode` (encoding only)
/// and `escape::unescape` (entities); a CityGML text node needs both, and
/// neither normalises line endings.
pub fn unescaped_text(t: &quick_xml::events::BytesText<'_>) -> Result<String> {
    let decoded = t.decode().map_err(xml_err)?;
    Ok(quick_xml::escape::unescape(&decoded)
        .map_err(xml_err)?
        .into_owned())
}

/// Consume the current element's remaining subtree. Call right after reading
/// its `Start`; returns once its matching `End` is consumed.
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
            Event::Eof => return Err(unexpected_eof("skip")),
            _ => {}
        }
    }
}

/// The text content of the current element (assumed to hold only text),
/// consuming through its matching `End`.
pub fn read_text<R: BufRead>(reader: &mut NsReader<R>, buf: &mut Vec<u8>) -> Result<String> {
    let mut s = String::new();
    loop {
        buf.clear();
        match reader.read_event_into(buf).map_err(xml_err)? {
            Event::Text(t) => s.push_str(&unescaped_text(&t)?),
            Event::CData(t) => s.push_str(&String::from_utf8_lossy(&t)),
            Event::End(_) => return Ok(s),
            Event::Eof => return Err(unexpected_eof("text")),
            _ => {}
        }
    }
}
