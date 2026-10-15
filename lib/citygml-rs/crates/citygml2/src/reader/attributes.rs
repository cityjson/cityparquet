//! Attributes: the text-only properties a CityGML 2.0 class declares, typed
//! as its schema types them, and `gen:` generic attributes and sets of
//! them.

use std::io::BufRead;

use citygml_core::model::{Attribute, AttributeName, Module, Namespace, QName, Value};
use citygml_core::reader::text::{AttrType, to_value};
use citygml_core::xml::{get_attr_local, read_text, skip_element, xml_err};
use citygml_core::{Error, Result};
use quick_xml::events::Event;
use quick_xml::reader::NsReader;

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

/// The installations whose CityGML 2.0 class says whether they are inside
/// or outside the construction, which CityGML 3.0 and CityJSON say with a
/// `relationToConstruction` attribute.
const RELATION_TO_CONSTRUCTION: &[(Module, &str, &str)] = &[
    (Module::Building, "BuildingInstallation", "outside"),
    (Module::Building, "IntBuildingInstallation", "inside"),
    (Module::Bridge, "BridgeInstallation", "outside"),
    (Module::Bridge, "IntBridgeInstallation", "inside"),
    (Module::Tunnel, "TunnelInstallation", "outside"),
    (Module::Tunnel, "IntTunnelInstallation", "inside"),
];

/// The `relationToConstruction` attribute an installation's class implies,
/// or `None` for another class.
pub fn relation_to_construction(class: &QName) -> Option<Attribute> {
    let Namespace::Module(module) = class.ns else {
        return None;
    };
    let (_, _, relation) = RELATION_TO_CONSTRUCTION
        .iter()
        .find(|(m, local, _)| *m == module && *local == class.local)?;
    Some(Attribute {
        name: AttributeName::Property(QName::module(
            Module::Construction,
            "relationToConstruction",
        )),
        value: Value::String((*relation).to_string()),
    })
}

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

/// How deep generic attribute sets may nest in one another: a guard
/// against attacker-controlled recursion.
const MAX_SET_DEPTH: usize = 32;

/// A `gen:genericAttributeSet` named `name` (positioned after its `Start`;
/// consumes through its `End`): its generic attributes and nested sets, in
/// document order. A set without a `name`, or a member without one, cannot
/// be named, and is dropped.
pub fn read_generic_set<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    name: Option<String>,
) -> Result<Option<Attribute>> {
    read_set(reader, buf, name, 0)
}

fn read_set<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    name: Option<String>,
    depth: usize,
) -> Result<Option<Attribute>> {
    if depth > MAX_SET_DEPTH {
        return Err(Error::invalid(format!(
            "generic attribute sets nested deeper than {MAX_SET_DEPTH}"
        )));
    }
    let mut members = Vec::new();
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        match ev {
            Event::Start(e) => {
                let generics = citygml_core::xml::ns_eq(&rr, crate::ns::GENERICS.as_bytes());
                let local = e.local_name();
                let member_name = get_attr_local(&e, b"name");
                let member = if generics && local.as_ref() == b"genericAttributeSet" {
                    read_set(reader, buf, member_name, depth + 1)?
                } else if let Some(ty) = generics.then(|| generic_attr(local.as_ref())).flatten() {
                    read_generic(reader, buf, ty)?
                        .zip(member_name)
                        .map(|(value, name)| Attribute {
                            name: AttributeName::Generic(name),
                            value,
                        })
                } else {
                    skip_element(reader, buf)?;
                    None
                };
                members.extend(member);
            }
            Event::End(e) if e.local_name().as_ref() == b"genericAttributeSet" => break,
            Event::Eof => {
                return Err(Error::invalid(
                    "unexpected end of document inside a gen:genericAttributeSet",
                ));
            }
            _ => {}
        }
    }
    Ok(name.map(|name| Attribute {
        name: AttributeName::Generic(name),
        value: Value::Set(members),
    }))
}
