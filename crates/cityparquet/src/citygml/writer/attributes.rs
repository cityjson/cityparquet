//! Building attribute serialisation: route each attribute by its stored column
//! type to a typed `bldg:` element or a type-matched `gen:` generic attribute.
//! The round-trip invariant is package-level: the re-read `name -> type` map and
//! values must match, so an attribute is written with the `bldg:` element only
//! when its stored type equals the type the reader forces back for that name;
//! otherwise it falls back to the `gen:` element of its stored type. Values
//! CityGML 2.0 cannot represent are skipped-with-counter, never errored.

use std::io::Write;

use cityparquet_schema::CityParquetError;
use quick_xml::Writer;
use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use serde_json::{Map, Value};

use super::WriteReport;
use crate::Result;

fn io_err(e: std::io::Error) -> CityParquetError {
    CityParquetError::Io(e.to_string())
}

/// The stored column type a value round-trips through, decided by the JSON shape
/// `decode` produced (serde's own `is_i64`/`is_f64`, not numeric range).
enum Kind {
    Str,
    Int,
    Float,
    /// A date-shaped string (`YYYY-MM-DD`) — re-infers as a Date column.
    Date,
    /// Boolean, nested/heterogeneous Json, single/empty string list — no
    /// round-trip-stable CityGML 2.0 form.
    Unwritable,
}

fn is_date_shaped(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b[..4].iter().all(u8::is_ascii_digit)
        && b[5..7].iter().all(u8::is_ascii_digit)
        && b[8..].iter().all(u8::is_ascii_digit)
}

/// A string CityGML/XML cannot carry losslessly: empty/whitespace-only (the
/// reader drops it) or containing an XML-1.0-illegal control char.
fn is_unwritable_string(s: &str) -> bool {
    if s.trim().is_empty() {
        return true;
    }
    s.chars().any(|c| {
        let u = c as u32;
        // Legal XML 1.0 controls are only 0x9, 0xA, 0xD; everything else below
        // 0x20 is illegal.
        (u < 0x20) && !matches!(u, 0x9 | 0xA | 0xD)
    })
}

fn value_kind(v: &Value) -> Kind {
    match v {
        Value::String(s) if is_date_shaped(s) => Kind::Date,
        Value::String(_) => Kind::Str,
        Value::Number(n) if n.is_i64() || n.is_u64() => Kind::Int,
        Value::Number(_) => Kind::Float,
        _ => Kind::Unwritable,
    }
}

/// The reader-forced type for a typed `bldg:` name, or `None` if not a known
/// typed attribute. Mirrors `citygml::attributes`.
fn bldg_forced_kind(name: &str) -> Option<Kind> {
    Some(match name {
        "function" | "usage" | "class" | "roofType" | "yearOfConstruction"
        | "yearOfDemolition" => Kind::Str,
        "measuredHeight" => Kind::Float,
        "storeysAboveGround" | "storeysBelowGround" => Kind::Int,
        _ => return None,
    })
}

/// Scalar text for a JSON scalar: shortest-round-trip for numbers (serde's
/// `Number::to_string`), verbatim for strings.
fn scalar_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        other => other.to_string(),
    }
}

fn write_bldg_element<W: Write>(
    w: &mut Writer<W>,
    name: &str,
    text: &str,
    uom: Option<&str>,
) -> Result<()> {
    let tag = format!("bldg:{name}");
    let mut start = BytesStart::new(&tag);
    if let Some(u) = uom {
        start.push_attribute(("uom", u));
    }
    w.write_event(Event::Start(start)).map_err(io_err)?;
    w.write_event(Event::Text(BytesText::new(text))).map_err(io_err)?;
    w.write_event(Event::End(BytesEnd::new(&tag))).map_err(io_err)?;
    Ok(())
}

fn write_gen_element<W: Write>(
    w: &mut Writer<W>,
    element: &str, // "stringAttribute" | "intAttribute" | "doubleAttribute" | "dateAttribute"
    name: &str,
    text: &str,
) -> Result<()> {
    let tag = format!("gen:{element}");
    let mut start = BytesStart::new(&tag);
    start.push_attribute(("name", name));
    w.write_event(Event::Start(start)).map_err(io_err)?;
    w.write_event(Event::Start(BytesStart::new("gen:value"))).map_err(io_err)?;
    w.write_event(Event::Text(BytesText::new(text))).map_err(io_err)?;
    w.write_event(Event::End(BytesEnd::new("gen:value"))).map_err(io_err)?;
    w.write_event(Event::End(BytesEnd::new(&tag))).map_err(io_err)?;
    Ok(())
}

/// Write ONE scalar value under `name`. Returns `true` if written, `false` if
/// skipped (the caller counts). Never errors on unrepresentable data.
fn write_one<W: Write>(w: &mut Writer<W>, name: &str, v: &Value) -> Result<bool> {
    // String-shaped values need an XML-writability check first.
    if let Value::String(s) = v
        && is_unwritable_string(s)
    {
        return Ok(false);
    }
    let text = scalar_text(v);
    match value_kind(v) {
        Kind::Unwritable => Ok(false),
        Kind::Str => {
            match bldg_forced_kind(name) {
                Some(Kind::Str) => write_bldg_element(w, name, &text, None)?,
                _ => write_gen_element(w, "stringAttribute", name, &text)?,
            }
            Ok(true)
        }
        Kind::Date => {
            // No typed bldg: date attribute on Building; always gen:dateAttribute.
            write_gen_element(w, "dateAttribute", name, &text)?;
            Ok(true)
        }
        Kind::Int => {
            match bldg_forced_kind(name) {
                // Only non-negative integers are schema-clean as storeys.
                Some(Kind::Int) if v.as_i64().is_some_and(|i| i >= 0) => {
                    write_bldg_element(w, name, &text, None)?
                }
                _ => write_gen_element(w, "intAttribute", name, &text)?,
            }
            Ok(true)
        }
        Kind::Float => {
            match bldg_forced_kind(name) {
                Some(Kind::Float) => write_bldg_element(w, name, &text, Some("m"))?,
                _ => write_gen_element(w, "doubleAttribute", name, &text)?,
            }
            Ok(true)
        }
    }
}

/// Serialise a building's attributes. See module docs for the routing rule.
pub fn write_attributes<W: Write>(
    w: &mut Writer<W>,
    attrs: &Map<String, Value>,
    report: &mut WriteReport,
) -> Result<usize> {
    let mut written = 0usize;
    for (name, value) in attrs {
        // A writable string list (>= 2 string items) expands to one element per
        // item, same route each, preserving order; every other value (scalars,
        // and single/empty lists which are Unwritable in `write_one`) is one.
        let items: Vec<&Value> = match value {
            Value::Array(items) if items.len() >= 2 && items.iter().all(Value::is_string) => {
                items.iter().collect()
            }
            other => vec![other],
        };
        for item in items {
            if write_one(w, name, item)? {
                written += 1;
                report.attributes_written += 1;
            } else {
                report.attributes_skipped += 1;
            }
        }
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn emit(attrs: &Map<String, Value>) -> (String, WriteReport) {
        let mut w = Writer::new(Vec::new());
        let mut report = WriteReport::default();
        let n = write_attributes(&mut w, attrs, &mut report).unwrap();
        let xml = String::from_utf8(w.into_inner()).unwrap();
        assert_eq!(n, report.attributes_written, "return value counts written values");
        (xml, report)
    }

    fn map(pairs: Vec<(&str, Value)>) -> Map<String, Value> {
        pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
    }

    #[test]
    fn known_bldg_name_with_matching_type_uses_bldg_element() {
        // measuredHeight stored as float -> matches reader-forced Float -> bldg:, uom="m".
        // serde's shortest round-trip form of 8.0 is "8.0" (re-parses to 8.0).
        let (xml, r) = emit(&map(vec![("measuredHeight", Value::from(8.0))]));
        assert!(xml.contains("<bldg:measuredHeight uom=\"m\">8.0</bldg:measuredHeight>"), "{xml}");
        assert_eq!(r.attributes_written, 1);
        assert_eq!(r.attributes_skipped, 0);
    }

    #[test]
    fn string_forced_bldg_name_uses_bldg_element() {
        let (xml, _) = emit(&map(vec![("roofType", Value::from("1000"))]));
        assert!(xml.contains("<bldg:roofType>1000</bldg:roofType>"), "{xml}");
    }

    #[test]
    fn integer_storeys_uses_bldg_element() {
        let (xml, _) = emit(&map(vec![("storeysAboveGround", Value::from(3i64))]));
        assert!(xml.contains("<bldg:storeysAboveGround>3</bldg:storeysAboveGround>"), "{xml}");
    }

    #[test]
    fn known_name_with_mismatched_type_falls_back_to_gen() {
        // yearOfConstruction is String-forced, but stored as an integer here
        // (CityJSON-origin): must go gen:intAttribute so it re-infers as Int64.
        let (xml, _) = emit(&map(vec![("yearOfConstruction", Value::from(1985i64))]));
        assert!(
            xml.contains("<gen:intAttribute name=\"yearOfConstruction\"><gen:value>1985</gen:value></gen:intAttribute>"),
            "{xml}"
        );
        assert!(!xml.contains("<bldg:yearOfConstruction>"));
    }

    #[test]
    fn unknown_string_uses_gen_string_attribute() {
        let (xml, _) = emit(&map(vec![("owner", Value::from("Acme & Co <x>"))]));
        // value is auto-escaped.
        assert!(
            xml.contains("<gen:stringAttribute name=\"owner\"><gen:value>Acme &amp; Co &lt;x&gt;</gen:value></gen:stringAttribute>"),
            "{xml}"
        );
    }

    #[test]
    fn unknown_float_uses_gen_double_attribute() {
        let (xml, _) = emit(&map(vec![("area", Value::from(12.5))]));
        assert!(xml.contains("<gen:doubleAttribute name=\"area\"><gen:value>12.5</gen:value></gen:doubleAttribute>"), "{xml}");
    }

    #[test]
    fn date_shaped_string_uses_gen_date_attribute() {
        let (xml, _) = emit(&map(vec![("built", Value::from("1985-06-17"))]));
        assert!(xml.contains("<gen:dateAttribute name=\"built\"><gen:value>1985-06-17</gen:value></gen:dateAttribute>"), "{xml}");
    }

    #[test]
    fn multi_element_string_list_emits_one_per_item_in_order() {
        let (xml, r) = emit(&map(vec![(
            "function",
            Value::Array(vec![Value::from("1000"), Value::from("1610")]),
        )]));
        // function is a bldg: name (String-forced); each item -> its own bldg: element.
        let a = xml.find("1000").unwrap();
        let b = xml.find("1610").unwrap();
        assert!(a < b, "items preserve order: {xml}");
        assert_eq!(xml.matches("<bldg:function>").count(), 2, "{xml}");
        assert_eq!(r.attributes_written, 2);
    }

    #[test]
    fn boolean_is_skipped() {
        let (xml, r) = emit(&map(vec![("flag", Value::from(true))]));
        assert!(xml.is_empty(), "{xml}");
        assert_eq!(r.attributes_written, 0);
        assert_eq!(r.attributes_skipped, 1);
    }

    #[test]
    fn nested_object_is_skipped() {
        let (_, r) = emit(&map(vec![("meta", serde_json::json!({"a": 1}))]));
        assert_eq!(r.attributes_skipped, 1);
        assert_eq!(r.attributes_written, 0);
    }

    #[test]
    fn single_element_string_list_is_skipped() {
        // ["a"] would re-infer as scalar String, flipping the column type.
        let (_, r) = emit(&map(vec![("tags", Value::Array(vec![Value::from("a")]))]));
        assert_eq!(r.attributes_skipped, 1);
        assert_eq!(r.attributes_written, 0);
    }

    #[test]
    fn empty_and_whitespace_strings_are_skipped() {
        let (_, r) = emit(&map(vec![("a", Value::from("")), ("b", Value::from("   "))]));
        assert_eq!(r.attributes_skipped, 2);
        assert_eq!(r.attributes_written, 0);
    }

    #[test]
    fn control_char_string_is_skipped() {
        let (_, r) = emit(&map(vec![("bad", Value::from("x\u{0007}y"))]));
        assert_eq!(r.attributes_skipped, 1);
    }

    #[test]
    fn float_formatting_is_shortest_round_trip() {
        // 8.0 -> "8"; -0.0 -> "-0"; long decimal preserved exactly and re-parses.
        for v in [8.0_f64, -0.0, 1.0 / 3.0, 1e21] {
            let (xml, _) = emit(&map(vec![("x", Value::from(v))]));
            let start = xml.find("<gen:value>").unwrap() + "<gen:value>".len();
            let end = xml.find("</gen:value>").unwrap();
            let parsed: f64 = xml[start..end].parse().unwrap();
            assert!(parsed.to_bits() == v.to_bits() || (parsed == v), "{v} -> {}", &xml[start..end]);
        }
    }
}
