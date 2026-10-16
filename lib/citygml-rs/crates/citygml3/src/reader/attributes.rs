//! Attributes: the text-only properties a CityGML 3.0 class declares, typed
//! as its schema types them, and `gen:` generic attributes.

use std::io::BufRead;

use citygml_core::model::{Attribute, AttributeName, Module, Value};
use citygml_core::reader::text::{AttrType, to_value};
use citygml_core::xml::{ns_eq, read_text, skip_element, xml_err};
use citygml_core::{Error, Result};
use quick_xml::events::Event;
use quick_xml::reader::NsReader;

/// The CityGML 3.0 properties whose schema type is a number or a boolean, by
/// module and local name. Every other text-only property is text: codes,
/// dates and years among them, kept as the document spells them.
const TYPED: &[(Module, &str, AttrType)] = &[
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
    (Module::Building, "sortKey", AttrType::Float),
    (Module::Bridge, "isMovable", AttrType::Bool),
    (Module::Construction, "isStructuralElement", AttrType::Bool),
    (Module::Vegetation, "height", AttrType::Float),
    (Module::Vegetation, "trunkDiameter", AttrType::Float),
    (Module::Vegetation, "crownDiameter", AttrType::Float),
    (Module::Vegetation, "rootBallDiameter", AttrType::Float),
    (Module::Vegetation, "maxRootBallDepth", AttrType::Float),
    (Module::Vegetation, "averageHeight", AttrType::Float),
    (Module::Vegetation, "minHeight", AttrType::Float),
    (Module::Vegetation, "maxHeight", AttrType::Float),
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
        b"StringAttribute" | b"DateAttribute" | b"UriAttribute" | b"CodeAttribute" => AttrType::Str,
        b"IntAttribute" => AttrType::Int,
        b"DoubleAttribute" | b"MeasureAttribute" => AttrType::Float,
        _ => return None,
    })
}

/// A `gen:` generic attribute's name and value, from its nested `gen:name`
/// and `gen:value`, or `None` when either is missing or the value is empty.
/// Positioned after the element's `Start`; consumes through its `End`.
pub fn read_generic<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    ty: AttrType,
) -> Result<Option<(String, Value)>> {
    let mut name: Option<String> = None;
    let mut value_text: Option<String> = None;
    let mut depth = 1usize;
    loop {
        buf.clear();
        match reader.read_event_into(buf).map_err(xml_err)? {
            Event::Start(child) => match child.local_name().as_ref() {
                // `read_text` consumes the element's own `End`, so the depth
                // is unchanged by these branches.
                b"name" if depth == 1 => name = Some(read_text(reader, buf)?),
                b"value" if depth == 1 => value_text = Some(read_text(reader, buf)?),
                _ => depth += 1,
            },
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
    let value = value_text.filter(|text| !text.trim().is_empty());
    Ok(name
        .zip(value)
        .map(|(name, text)| (name.trim().to_string(), to_value(&text, ty))))
}

/// How deep generic attribute sets may nest in one another: a guard
/// against attacker-controlled recursion.
const MAX_SET_DEPTH: usize = 32;

/// A `gen:GenericAttributeSet` (positioned after its `Start`; consumes
/// through its `End`), `depth` sets deep: its `gen:name`, and the generic
/// attributes and sets its `gen:genericAttribute`s hold, in document order.
/// A set without a name, or a member without one, cannot be named, and is
/// dropped.
pub fn read_generic_set<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    depth: usize,
) -> Result<Option<Attribute>> {
    if depth > MAX_SET_DEPTH {
        return Err(Error::invalid(format!(
            "generic attribute sets nested deeper than {MAX_SET_DEPTH}"
        )));
    }
    let mut name: Option<String> = None;
    let mut members = Vec::new();
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        match ev {
            Event::Start(e) => {
                let generics = ns_eq(&rr, crate::ns::GENERICS.as_bytes());
                match e.local_name().as_ref() {
                    b"name" if generics => {
                        name = Some(read_text(reader, buf)?.trim().to_string());
                    }
                    b"genericAttribute" if generics => {
                        members.extend(read_set_member(reader, buf, depth)?);
                    }
                    _ => skip_element(reader, buf)?,
                }
            }
            Event::End(e) if e.local_name().as_ref() == b"GenericAttributeSet" => break,
            Event::Eof => {
                return Err(Error::invalid(
                    "unexpected end of document inside a gen:GenericAttributeSet",
                ));
            }
            _ => {}
        }
    }
    Ok(name.filter(|n| !n.is_empty()).map(|name| Attribute {
        name: AttributeName::Generic(name),
        value: Value::Set(members),
    }))
}

/// A set's `gen:genericAttribute` (positioned after its `Start`; consumes
/// through its `End`): the generic attribute or set it holds.
fn read_set_member<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    depth: usize,
) -> Result<Option<Attribute>> {
    let mut member = None;
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        match ev {
            Event::Start(e) => {
                let generics = ns_eq(&rr, crate::ns::GENERICS.as_bytes());
                let local = e.local_name();
                if generics && local.as_ref() == b"GenericAttributeSet" {
                    member = read_generic_set(reader, buf, depth + 1)?;
                } else if let Some(ty) = generics.then(|| generic_attr(local.as_ref())).flatten() {
                    member = read_generic(reader, buf, ty)?.map(|(name, value)| Attribute {
                        name: AttributeName::Generic(name),
                        value,
                    });
                } else {
                    skip_element(reader, buf)?;
                }
            }
            Event::End(e) if e.local_name().as_ref() == b"genericAttribute" => break,
            Event::Eof => {
                return Err(Error::invalid(
                    "unexpected end of document inside a gen:genericAttribute",
                ));
            }
            _ => {}
        }
    }
    Ok(member)
}
