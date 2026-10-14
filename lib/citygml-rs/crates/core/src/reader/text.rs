//! Text-only properties: their text, and that text as a typed [`Value`].

use std::io::BufRead;

use quick_xml::events::Event;
use quick_xml::reader::NsReader;

use crate::model::Value;
use crate::xml::{skip_element, unescaped_text, xml_err};
use crate::{Error, Result};

/// How to type an attribute's text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttrType {
    Str,
    Float,
    Int,
    Bool,
    /// A whitespace-separated list of numbers.
    FloatList,
}

/// A property's text as `ty`. A value that does not parse as its type stays
/// text, trimmed.
pub fn to_value(text: &str, ty: AttrType) -> Value {
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
        AttrType::Bool => match text.trim() {
            "true" | "1" => Value::Boolean(true),
            "false" | "0" => Value::Boolean(false),
            other => Value::String(other.to_string()),
        },
        AttrType::FloatList => text
            .split_whitespace()
            .map(|t| t.parse::<f64>().map(Value::Double))
            .collect::<std::result::Result<_, _>>()
            .map_or_else(|_| Value::String(text.trim().to_string()), Value::List),
    }
}

/// A leaf element's text (positioned after its `Start`, ending at
/// `End(end)`), verbatim: `None` for an empty or blank one, or one with
/// child elements.
pub fn read_leaf_text<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    end: &[u8],
) -> Result<Option<String>> {
    let mut text = String::new();
    let mut has_child = false;
    loop {
        buf.clear();
        match reader.read_event_into(buf).map_err(xml_err)? {
            Event::Text(t) => text.push_str(&unescaped_text(&t)?),
            Event::CData(t) => text.push_str(&String::from_utf8_lossy(&t)),
            Event::Start(_) => {
                has_child = true;
                skip_element(reader, buf)?;
            }
            Event::End(e) if e.local_name().as_ref() == end => break,
            Event::Eof => {
                return Err(Error::invalid(
                    "unexpected end of document inside a typed property",
                ));
            }
            _ => {}
        }
    }
    Ok((!has_child && !text.trim().is_empty()).then_some(text))
}
