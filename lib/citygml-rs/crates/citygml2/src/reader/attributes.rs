//! Attributes: typed `bldg:` properties, `gen:` generic attributes, and the
//! text-only properties of other objects.

use std::io::BufRead;

use citygml_core::model::Value;
use citygml_core::xml::{read_text, xml_err};
use citygml_core::{Error, Result};
use quick_xml::events::Event;
use quick_xml::reader::NsReader;

/// How to type an attribute's text.
#[derive(Clone, Copy)]
pub enum AttrType {
    Str,
    Float,
    Int,
}

/// A typed `bldg:` attribute property (by local name), or `None`.
pub fn typed_building_attr(local: &[u8]) -> Option<AttrType> {
    Some(match local {
        b"function"
        | b"usage"
        | b"class"
        | b"roofType"
        | b"yearOfConstruction"
        | b"yearOfDemolition" => AttrType::Str,
        b"measuredHeight" => AttrType::Float,
        b"storeysAboveGround" | b"storeysBelowGround" => AttrType::Int,
        _ => return None,
    })
}

/// A `gen:` generic attribute element (by local name), or `None`.
pub fn generic_attr(local: &[u8]) -> Option<AttrType> {
    Some(match local {
        b"stringAttribute" | b"dateAttribute" | b"uriAttribute" => AttrType::Str,
        b"intAttribute" => AttrType::Int,
        b"doubleAttribute" | b"measureAttribute" => AttrType::Float,
        _ => return None,
    })
}

/// A typed property's value, or `None` when it is empty. Positioned after
/// the element's `Start`.
pub fn read_typed<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    ty: AttrType,
) -> Result<Option<Value>> {
    let text = read_text(reader, buf)?;
    // A truly empty value is skipped; string content is kept verbatim (only
    // numeric values are trimmed, inside `to_value`).
    if text.trim().is_empty() {
        return Ok(None);
    }
    Ok(Some(to_value(&text, ty)))
}

/// A `gen:` generic attribute's value, from its nested `gen:value`, or `None`
/// when it is empty. Positioned after the element's `Start`; consumes through
/// its `End`.
pub fn read_generic<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    ty: AttrType,
) -> Result<Option<Value>> {
    let mut value_text: Option<String> = None;
    let mut depth = 1usize;
    loop {
        buf.clear();
        match reader.read_event_into(buf).map_err(xml_err)? {
            Event::Start(child) => {
                if child.local_name().as_ref() == b"value" {
                    // `read_text` consumes the value element's own `End`, so
                    // the depth is unchanged by this branch.
                    value_text = Some(read_text(reader, buf)?);
                } else {
                    depth += 1;
                }
            }
            Event::End(_) => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            Event::Eof => {
                return Err(Error::invalid(
                    "unexpected end of document inside a gen: attribute",
                ));
            }
            _ => {}
        }
    }
    Ok(value_text
        .filter(|text| !text.trim().is_empty())
        .map(|text| to_value(&text, ty)))
}

fn to_value(text: &str, ty: AttrType) -> Value {
    match ty {
        // String content is kept verbatim (a numeric-looking string attribute
        // stays a string); only numeric attributes are trimmed and parsed.
        AttrType::Str => Value::String(text.to_string()),
        AttrType::Float => text
            .trim()
            .parse::<f64>()
            .map_or_else(|_| Value::String(text.trim().to_string()), Value::Double),
        AttrType::Int => text
            .trim()
            .parse::<i64>()
            .map_or_else(|_| Value::String(text.trim().to_string()), Value::Integer),
    }
}
