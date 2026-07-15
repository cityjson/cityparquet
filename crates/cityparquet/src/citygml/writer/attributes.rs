//! Building attribute serialisation, routed by the attribute's **stored Arrow
//! column type** (`AttributeType`), not by the decoded value's shape.
//!
//! The round-trip invariant is package-level: the re-read `name -> type` map and
//! values must match after the reader re-infers types. Because inference is a
//! pure function of the value, routing by the stored type is what keeps a
//! `Json`/`Boolean` column from silently re-inferring as a primitive, and a
//! `String` column that happens to hold a date-shaped value from flipping to
//! `Date`. An attribute is written with a typed `bldg:` element only when its
//! stored type equals the type the reader forces back for that name; otherwise
//! it falls back to the `gen:` element of its stored type. Values CityGML 2.0
//! cannot represent (`Boolean`, `Json`, empty/whitespace or XML-illegal
//! strings, partially-unwritable string lists, un-typed columns) are
//! skipped-with-counter, never errored.

use std::collections::HashMap;
use std::io::Write;

use arrow_schema::Schema;
use arrow_schema::extension::EXTENSION_TYPE_NAME_KEY;
use cityparquet_schema::{AttributeType, CityParquetError};
use quick_xml::Writer;
use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use serde_json::{Map, Value};

use super::WriteReport;
use crate::Result;

const ARROW_JSON_EXTENSION: &str = "arrow.json";

fn io_err(e: std::io::Error) -> CityParquetError {
    CityParquetError::Io(e.to_string())
}

/// Build the `column name -> stored AttributeType` map from the package's Arrow
/// schema. A `Utf8` field tagged with the `arrow.json` extension is a `Json`
/// column (which `AttributeType::from_arrow` cannot tell from `String` on its
/// own); every other attribute column resolves through `from_arrow`. Names that
/// do not resolve are omitted (routed to skip-with-counter downstream).
pub fn attribute_types(
    schema: &Schema,
    attribute_columns: &[String],
) -> HashMap<String, AttributeType> {
    let mut map = HashMap::with_capacity(attribute_columns.len());
    for name in attribute_columns {
        let Ok(field) = schema.field_with_name(name) else {
            continue;
        };
        let is_json = field
            .metadata()
            .get(EXTENSION_TYPE_NAME_KEY)
            .map(String::as_str)
            == Some(ARROW_JSON_EXTENSION);
        let ty = if is_json {
            AttributeType::Json
        } else if let Some(ty) = AttributeType::from_arrow(field.data_type()) {
            ty
        } else {
            continue;
        };
        map.insert(name.clone(), ty);
    }
    map
}

/// A character illegal in an XML 1.0 document: a C0 control other than tab/LF/CR,
/// or U+FFFE / U+FFFF. (Unpaired surrogates cannot occur in a Rust `&str`.)
fn is_xml_illegal(c: char) -> bool {
    let u = c as u32;
    (u < 0x20 && !matches!(u, 0x9 | 0xA | 0xD)) || u == 0xFFFE || u == 0xFFFF
}

/// A string value CityGML/XML can carry losslessly: non-empty after trimming
/// (the reader drops empty/whitespace) and free of XML-illegal characters
/// (which `BytesText`'s escaping does not fix — they would corrupt the output).
fn string_writable(s: &str) -> bool {
    !s.trim().is_empty() && !s.chars().any(is_xml_illegal)
}

/// The `gen:*Attribute` `name` is an XML attribute **value** (`xs:string`), so a
/// colon is harmless there; only XML-illegal characters would corrupt output.
fn gen_name_ok(name: &str) -> bool {
    !name.is_empty() && !name.chars().any(is_xml_illegal)
}

/// The reader-forced type for a typed `bldg:` name, or `None` if not a known
/// typed attribute. Mirrors `citygml::attributes`.
fn bldg_forced_type(name: &str) -> Option<AttributeType> {
    Some(match name {
        "function" | "usage" | "class" | "roofType" | "yearOfConstruction" | "yearOfDemolition" => {
            AttributeType::String
        }
        "measuredHeight" => AttributeType::Float64,
        "storeysAboveGround" | "storeysBelowGround" => AttributeType::Int64,
        _ => return None,
    })
}

/// The only typed `bldg:` names with `maxOccurs` unbounded — a multi-valued
/// (`StringList`) attribute may repeat as these; every other name's list must
/// go to repeated `gen:stringAttribute` (the rest are `maxOccurs=1`).
fn bldg_repeatable(name: &str) -> bool {
    matches!(name, "function" | "usage")
}

/// Shortest-round-trip text for a JSON scalar (serde's `Number::to_string`),
/// verbatim for strings.
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
    w.write_event(Event::Text(BytesText::new(text)))
        .map_err(io_err)?;
    w.write_event(Event::End(BytesEnd::new(&tag)))
        .map_err(io_err)?;
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
    w.write_event(Event::Start(BytesStart::new("gen:value")))
        .map_err(io_err)?;
    w.write_event(Event::Text(BytesText::new(text)))
        .map_err(io_err)?;
    w.write_event(Event::End(BytesEnd::new("gen:value")))
        .map_err(io_err)?;
    w.write_event(Event::End(BytesEnd::new(&tag)))
        .map_err(io_err)?;
    Ok(())
}

/// Write one scalar `String`/`Int64`/`Float64`/`Date`/`Timestamp` value under
/// `name`. Returns `true` if written, `false` if skipped (caller counts).
fn write_scalar<W: Write>(
    w: &mut Writer<W>,
    name: &str,
    ty: AttributeType,
    v: &Value,
) -> Result<bool> {
    // Any string-valued type (String/Date/Timestamp) must be XML-writable.
    if let Value::String(s) = v
        && !string_writable(s)
    {
        return Ok(false);
    }
    let text = scalar_text(v);
    // Whether the typed bldg: element applies (stored type == reader-forced
    // type for this name); otherwise the gen: element of the stored type.
    let use_bldg = bldg_forced_type(name) == Some(ty)
        && match ty {
            // A negative value is not a schema-clean nonNegativeInteger storey.
            AttributeType::Int64 => v.as_i64().is_some_and(|i| i >= 0),
            _ => true,
        };
    if use_bldg {
        let uom = matches!(ty, AttributeType::Float64).then_some("m");
        write_bldg_element(w, name, &text, uom)?;
        return Ok(true);
    }
    // gen: route — the name becomes an attribute value; reject illegal chars.
    if !gen_name_ok(name) {
        return Ok(false);
    }
    let element = match ty {
        AttributeType::Int64 => "intAttribute",
        AttributeType::Float64 => "doubleAttribute",
        AttributeType::Date => "dateAttribute",
        // Timestamp re-infers from its RFC3339 string; String is verbatim.
        AttributeType::String | AttributeType::Timestamp => "stringAttribute",
        AttributeType::Boolean | AttributeType::Json | AttributeType::StringList => {
            return Ok(false);
        }
    };
    write_gen_element(w, element, name, &text)?;
    Ok(true)
}

/// Serialise a building's attributes, routed by each column's stored type.
pub fn write_attributes<W: Write>(
    w: &mut Writer<W>,
    attrs: &Map<String, Value>,
    types: &HashMap<String, AttributeType>,
    report: &mut WriteReport,
) -> Result<usize> {
    let mut written = 0usize;
    for (name, value) in attrs {
        // An attribute whose column type is unknown, Boolean, or Json has no
        // round-trip-stable CityGML 2.0 form: skip the whole attribute.
        let Some(&ty) = types.get(name) else {
            report.attributes_skipped += 1;
            continue;
        };
        match ty {
            AttributeType::Boolean | AttributeType::Json => report.attributes_skipped += 1,
            AttributeType::StringList => {
                // All-or-nothing: a partially-written list re-infers as a scalar
                // String (or, for length 1, always does), flipping the column
                // type. Emit only when every item is a writable string, and —
                // for non-`function`/`usage` names — the gen: name is valid.
                let ok = matches!(value, Value::Array(items) if !items.is_empty()
                    && items.iter().all(|it| it.as_str().is_some_and(string_writable)));
                let name_ok = bldg_repeatable(name) || gen_name_ok(name);
                if !ok || !name_ok {
                    report.attributes_skipped += 1;
                    continue;
                }
                let Value::Array(items) = value else {
                    unreachable!("checked Array above")
                };
                for it in items {
                    let s = it.as_str().expect("checked all items are strings");
                    if bldg_repeatable(name) {
                        write_bldg_element(w, name, s, None)?;
                    } else {
                        write_gen_element(w, "stringAttribute", name, s)?;
                    }
                    written += 1;
                    report.attributes_written += 1;
                }
            }
            _ => {
                if write_scalar(w, name, ty, value)? {
                    written += 1;
                    report.attributes_written += 1;
                } else {
                    report.attributes_skipped += 1;
                }
            }
        }
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn emit(
        attrs: &Map<String, Value>,
        types: Vec<(&str, AttributeType)>,
    ) -> (String, WriteReport) {
        let types: HashMap<String, AttributeType> =
            types.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
        let mut w = Writer::new(Vec::new());
        let mut report = WriteReport::default();
        let n = write_attributes(&mut w, attrs, &types, &mut report).unwrap();
        let xml = String::from_utf8(w.into_inner()).unwrap();
        assert_eq!(
            n, report.attributes_written,
            "return value counts written values"
        );
        (xml, report)
    }

    fn map(pairs: Vec<(&str, Value)>) -> Map<String, Value> {
        pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
    }

    #[test]
    fn float_bldg_name_matching_type_uses_bldg_element() {
        let (xml, r) = emit(
            &map(vec![("measuredHeight", Value::from(8.0))]),
            vec![("measuredHeight", AttributeType::Float64)],
        );
        assert!(
            xml.contains("<bldg:measuredHeight uom=\"m\">8.0</bldg:measuredHeight>"),
            "{xml}"
        );
        assert_eq!(r.attributes_written, 1);
        assert_eq!(r.attributes_skipped, 0);
    }

    #[test]
    fn string_bldg_name_uses_bldg_element() {
        let (xml, _) = emit(
            &map(vec![("roofType", Value::from("1000"))]),
            vec![("roofType", AttributeType::String)],
        );
        assert!(xml.contains("<bldg:roofType>1000</bldg:roofType>"), "{xml}");
    }

    #[test]
    fn int_storeys_uses_bldg_element() {
        let (xml, _) = emit(
            &map(vec![("storeysAboveGround", Value::from(3i64))]),
            vec![("storeysAboveGround", AttributeType::Int64)],
        );
        assert!(
            xml.contains("<bldg:storeysAboveGround>3</bldg:storeysAboveGround>"),
            "{xml}"
        );
    }

    #[test]
    fn known_name_with_mismatched_stored_type_falls_back_to_gen() {
        // yearOfConstruction is String-forced, but stored Int64 here -> gen:int.
        let (xml, _) = emit(
            &map(vec![("yearOfConstruction", Value::from(1985i64))]),
            vec![("yearOfConstruction", AttributeType::Int64)],
        );
        assert!(
            xml.contains("<gen:intAttribute name=\"yearOfConstruction\"><gen:value>1985</gen:value></gen:intAttribute>"),
            "{xml}"
        );
        assert!(!xml.contains("<bldg:yearOfConstruction>"));
    }

    #[test]
    fn negative_storeys_falls_back_to_gen() {
        let (xml, _) = emit(
            &map(vec![("storeysAboveGround", Value::from(-1i64))]),
            vec![("storeysAboveGround", AttributeType::Int64)],
        );
        assert!(
            xml.contains("<gen:intAttribute name=\"storeysAboveGround\">"),
            "{xml}"
        );
    }

    #[test]
    fn unknown_string_uses_gen_string_attribute_escaped() {
        let (xml, _) = emit(
            &map(vec![("owner", Value::from("Acme & Co <x>"))]),
            vec![("owner", AttributeType::String)],
        );
        assert!(
            xml.contains("<gen:stringAttribute name=\"owner\"><gen:value>Acme &amp; Co &lt;x&gt;</gen:value></gen:stringAttribute>"),
            "{xml}"
        );
    }

    #[test]
    fn unknown_float_uses_gen_double_attribute() {
        let (xml, _) = emit(
            &map(vec![("area", Value::from(12.5))]),
            vec![("area", AttributeType::Float64)],
        );
        assert!(xml.contains("<gen:doubleAttribute name=\"area\"><gen:value>12.5</gen:value></gen:doubleAttribute>"), "{xml}");
    }

    #[test]
    fn date_column_uses_gen_date_attribute() {
        let (xml, _) = emit(
            &map(vec![("built", Value::from("1985-06-17"))]),
            vec![("built", AttributeType::Date)],
        );
        assert!(xml.contains("<gen:dateAttribute name=\"built\"><gen:value>1985-06-17</gen:value></gen:dateAttribute>"), "{xml}");
    }

    #[test]
    fn date_shaped_value_in_string_column_stays_string() {
        // The routing-by-stored-type fix: a String column keeps its element even
        // when the value looks like a date, so it cannot flip to Date on re-read.
        let (xml, _) = emit(
            &map(vec![("label", Value::from("2025-01-01"))]),
            vec![("label", AttributeType::String)],
        );
        assert!(
            xml.contains("<gen:stringAttribute name=\"label\">"),
            "{xml}"
        );
        assert!(!xml.contains("dateAttribute"));
    }

    #[test]
    fn timestamp_column_uses_gen_string_attribute() {
        let (xml, _) = emit(
            &map(vec![("t", Value::from("2025-01-01T12:00:00.000Z"))]),
            vec![("t", AttributeType::Timestamp)],
        );
        assert!(
            xml.contains("<gen:stringAttribute name=\"t\"><gen:value>2025-01-01T12:00:00.000Z</gen:value></gen:stringAttribute>"),
            "{xml}"
        );
    }

    #[test]
    fn function_string_list_repeats_as_bldg_in_order() {
        let (xml, r) = emit(
            &map(vec![(
                "function",
                Value::Array(vec![Value::from("1000"), Value::from("1610")]),
            )]),
            vec![("function", AttributeType::StringList)],
        );
        assert!(
            xml.find("1000").unwrap() < xml.find("1610").unwrap(),
            "order: {xml}"
        );
        assert_eq!(xml.matches("<bldg:function>").count(), 2, "{xml}");
        assert_eq!(r.attributes_written, 2);
    }

    #[test]
    fn non_repeatable_bldg_name_string_list_goes_to_repeated_gen() {
        // roofType is maxOccurs=1 as bldg:, so a list must be repeated gen:.
        let (xml, _) = emit(
            &map(vec![(
                "roofType",
                Value::Array(vec![Value::from("1000"), Value::from("2000")]),
            )]),
            vec![("roofType", AttributeType::StringList)],
        );
        assert_eq!(
            xml.matches("<gen:stringAttribute name=\"roofType\">")
                .count(),
            2,
            "{xml}"
        );
        assert!(!xml.contains("<bldg:roofType>"));
    }

    #[test]
    fn boolean_column_is_skipped() {
        let (xml, r) = emit(
            &map(vec![("flag", Value::from(true))]),
            vec![("flag", AttributeType::Boolean)],
        );
        assert!(xml.is_empty(), "{xml}");
        assert_eq!(r.attributes_skipped, 1);
    }

    #[test]
    fn json_column_primitive_value_is_skipped_not_flipped() {
        // The #1 fix: a primitive value in a Json column must NOT be written as a
        // typed gen: attribute (which would re-infer Int64, flipping the column).
        let (xml, r) = emit(
            &map(vec![("meta", Value::from(5i64))]),
            vec![("meta", AttributeType::Json)],
        );
        assert!(xml.is_empty(), "{xml}");
        assert_eq!(r.attributes_skipped, 1);
        assert_eq!(r.attributes_written, 0);
    }

    #[test]
    fn partial_string_list_is_skipped_whole() {
        // ["valid", ""] must skip the WHOLE list, else it re-infers as a scalar
        // String, flipping StringList -> String.
        let (xml, r) = emit(
            &map(vec![(
                "tags",
                Value::Array(vec![Value::from("valid"), Value::from("")]),
            )]),
            vec![("tags", AttributeType::StringList)],
        );
        assert!(xml.is_empty(), "{xml}");
        assert_eq!(r.attributes_skipped, 1);
        assert_eq!(r.attributes_written, 0);
    }

    #[test]
    fn empty_and_control_and_ffff_strings_are_skipped() {
        let (_, r) = emit(
            &map(vec![
                ("a", Value::from("")),
                ("b", Value::from("x\u{0007}y")),
                ("c", Value::from("x\u{FFFF}y")),
            ]),
            vec![
                ("a", AttributeType::String),
                ("b", AttributeType::String),
                ("c", AttributeType::String),
            ],
        );
        assert_eq!(r.attributes_skipped, 3);
        assert_eq!(r.attributes_written, 0);
    }

    #[test]
    fn attribute_with_xml_illegal_name_is_skipped() {
        // A gen: route with an XML-illegal character in the column name would
        // corrupt the output; skip it.
        let (xml, r) = emit(
            &map(vec![("bad\u{0007}name", Value::from("v"))]),
            vec![("bad\u{0007}name", AttributeType::String)],
        );
        assert!(xml.is_empty(), "{xml}");
        assert_eq!(r.attributes_skipped, 1);
    }

    #[test]
    fn attribute_missing_from_type_map_is_skipped() {
        let (xml, r) = emit(&map(vec![("orphan", Value::from("v"))]), vec![]);
        assert!(xml.is_empty(), "{xml}");
        assert_eq!(r.attributes_skipped, 1);
    }

    #[test]
    fn float_formatting_is_shortest_round_trip() {
        for v in [8.0_f64, -0.0, 1.0 / 3.0, 1e21] {
            let (xml, _) = emit(
                &map(vec![("x", Value::from(v))]),
                vec![("x", AttributeType::Float64)],
            );
            let start = xml.find("<gen:value>").unwrap() + "<gen:value>".len();
            let end = xml.find("</gen:value>").unwrap();
            let parsed: f64 = xml[start..end].parse().unwrap();
            assert!(
                parsed.to_bits() == v.to_bits() || (parsed == v),
                "{v} -> {}",
                &xml[start..end]
            );
        }
    }
}
