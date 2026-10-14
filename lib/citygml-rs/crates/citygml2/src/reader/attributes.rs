//! Attributes: the text-only properties a CityGML 2.0 class declares, typed
//! as its schema types them, and `gen:` generic attributes.

use std::io::BufRead;

use citygml_core::model::{Module, Value};
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
    Bool,
    /// A whitespace-separated list of numbers.
    FloatList,
}

/// The CityGML 2.0 properties whose schema type is a number or a boolean, by
/// module and local name. Every other text-only property is text: codes,
/// dates and years among them, kept as the document spells them.
const TYPED: &[(Module, &str, AttrType)] = &[
    (Module::Building, "measuredHeight", AttrType::Float),
    (Module::Building, "storeysAboveGround", AttrType::Int),
    (Module::Building, "storeysBelowGround", AttrType::Int),
    (
        Module::Building,
        "storeyHeightsAboveGround",
        AttrType::FloatList,
    ),
    (
        Module::Building,
        "storeyHeightsBelowGround",
        AttrType::FloatList,
    ),
    (Module::Bridge, "isMovable", AttrType::Bool),
    (Module::Vegetation, "height", AttrType::Float),
    (Module::Vegetation, "trunkDiameter", AttrType::Float),
    (Module::Vegetation, "crownDiameter", AttrType::Float),
    (Module::Vegetation, "averageHeight", AttrType::Float),
];

/// How a property of `module` named `local` is typed.
pub fn property_type(module: Module, local: &[u8]) -> AttrType {
    TYPED
        .iter()
        .find(|(m, name, _)| *m == module && name.as_bytes() == local)
        .map_or(AttrType::Str, |(_, _, ty)| *ty)
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
