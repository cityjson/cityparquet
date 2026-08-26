//! Building attributes: typed `bldg:` elements and `gen:` generic attributes,
//! mapped to a `serde_json` object the existing `AttributeInferer` types into
//! columns. Only attributes that are *direct children* of the `bldg:Building`
//! are collected here (per-surface generic attributes live inside `boundedBy`
//! and are not hoisted onto the building).

use std::io::BufRead;

use cityparquet_schema::{CityParquetError, Result};
use quick_xml::events::Event;
use quick_xml::reader::NsReader;
use serde_json::{Value, json};

use super::xml::{read_text, xml_err};

/// How to type an attribute's text value.
#[derive(Clone, Copy)]
pub enum AttrType {
    Str,
    Float,
    Int,
}

/// A recognised typed `bldg:` attribute element (by local name), or `None`.
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

/// A recognised `gen:` generic attribute element (by local name), or `None`.
pub fn generic_attr(local: &[u8]) -> Option<AttrType> {
    Some(match local {
        b"stringAttribute" | b"dateAttribute" | b"uriAttribute" => AttrType::Str,
        b"intAttribute" => AttrType::Int,
        b"doubleAttribute" | b"measureAttribute" => AttrType::Float,
        _ => return None,
    })
}

/// Read a typed `bldg:` attribute's text into `(name, value)`, or `None` when
/// the value is empty. Positioned after the element's `Start`.
pub fn read_typed_attribute<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    local: &[u8],
    ty: AttrType,
) -> Result<Option<(String, Value)>> {
    let name = String::from_utf8_lossy(local).into_owned();
    let text = read_text(reader, buf)?;
    // Skip a truly empty value, but keep string content verbatim (only numeric
    // values are trimmed, inside `to_value`).
    if text.trim().is_empty() {
        return Ok(None);
    }
    Ok(Some((name, to_value(&text, ty))))
}

/// Read a `gen:` generic attribute's value from its nested `gen:value`, keyed
/// by `name` (its already-extracted `name` attribute). Positioned after the
/// element's `Start`; consumes through its `End`.
pub fn read_generic_attribute<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    name: Option<String>,
    ty: AttrType,
) -> Result<Option<(String, Value)>> {
    let mut value_text: Option<String> = None;
    let mut depth = 1usize;
    loop {
        buf.clear();
        match reader.read_event_into(buf).map_err(xml_err)? {
            Event::Start(child) => {
                if child.local_name().as_ref() == b"value" {
                    // `read_text` consumes the value element's own `End`, so the
                    // depth is unchanged by this branch.
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
                return Err(CityParquetError::Schema(
                    "unexpected end of document inside a gen: attribute".to_string(),
                ));
            }
            _ => {}
        }
    }
    match (name, value_text) {
        (Some(name), Some(text)) if !text.trim().is_empty() => {
            Ok(Some((name, to_value(&text, ty))))
        }
        // No `name` (uncolumnable) or an empty value.
        _ => Ok(None),
    }
}

/// Insert `value` under `key`, accumulating repeated keys into a JSON array —
/// CityGML allows repeated `bldg:function`/`usage` and repeated generic-
/// attribute names, which would otherwise silently overwrite each other.
pub fn accumulate(map: &mut serde_json::Map<String, Value>, key: String, value: Value) {
    match map.get_mut(&key) {
        None => {
            map.insert(key, value);
        }
        Some(Value::Array(existing)) => existing.push(value),
        Some(slot) => {
            let prev = slot.take();
            *slot = Value::Array(vec![prev, value]);
        }
    }
}

fn to_value(text: &str, ty: AttrType) -> Value {
    match ty {
        // String content is kept verbatim (a numeric-looking string attribute
        // stays a string); only numeric attributes are trimmed and parsed.
        AttrType::Str => json!(text),
        AttrType::Float => text
            .trim()
            .parse::<f64>()
            .map_or_else(|_| json!(text.trim()), |f| json!(f)),
        AttrType::Int => text
            .trim()
            .parse::<i64>()
            .map_or_else(|_| json!(text.trim()), |i| json!(i)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accumulate_promotes_repeats_to_array() {
        let mut m = serde_json::Map::new();
        accumulate(&mut m, "function".into(), json!("1000"));
        assert_eq!(m["function"], json!("1000"));
        accumulate(&mut m, "function".into(), json!("1610"));
        assert_eq!(m["function"], json!(["1000", "1610"]));
        accumulate(&mut m, "function".into(), json!("2000"));
        assert_eq!(m["function"], json!(["1000", "1610", "2000"]));
        // Distinct keys are unaffected.
        accumulate(&mut m, "roofType".into(), json!("3100"));
        assert_eq!(m["roofType"], json!("3100"));
    }
}
