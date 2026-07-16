//! Streaming parse of CityGML 2.0 `app:appearance` into CityJSON appearance.
//!
//! Scope (W-M5a): `app:X3DMaterial` — the CityJSON *material* of a surface. Each
//! `app:Appearance` carries an optional `app:theme` and a list of
//! `app:surfaceDataMember`s; an `app:X3DMaterial` holds the X3D colour/shading
//! properties plus zero or more `app:target="#polyid"` references to the polygons
//! it colours. We return one [`ReadMaterial`] per X3DMaterial (its theme, the
//! CityJSON material object, and the target polygon ids); the caller
//! ([`super::building`]) interns the materials and resolves the targets to face
//! positions.
//!
//! `app:theme` is absent → the empty-string theme `""` (round-trips to an absent
//! `app:theme` on write). `app:ParameterizedTexture` (textures) is W-M5b.

use std::io::BufRead;

use cityparquet_schema::{CityParquetError, Result};
use quick_xml::events::Event;
use quick_xml::reader::NsReader;
use serde_json::{Map, Value, json};

use super::xml::{NS_APP, NS_GML, ns_is, read_text, skip_element, xml_err};

/// One `app:X3DMaterial`: its theme, the CityJSON material object, and the
/// `gml:id`s of the polygons it targets (`app:target="#id"`, `#` stripped).
pub struct ReadMaterial {
    pub theme: String,
    pub material: Value,
    pub targets: Vec<String>,
}

/// Read an `app:appearance` property (positioned after its `Start`): descend to
/// the inner `app:Appearance` and parse its surface data. Consumes through the
/// property's `End`. A property with no `app:Appearance` child (empty or
/// `xlink`-only) yields nothing.
pub fn read_appearance<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
) -> Result<Vec<ReadMaterial>> {
    let mut out = Vec::new();
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        match ev {
            Event::Start(e) => {
                if ns_is(&rr, NS_APP) && e.local_name().as_ref() == b"Appearance" {
                    read_appearance_body(reader, buf, &mut out)?;
                } else {
                    skip_element(reader, buf)?;
                }
            }
            Event::End(e) if e.local_name().as_ref() == b"appearance" => break,
            Event::Eof => return Err(eof("appearance")),
            _ => {}
        }
    }
    Ok(out)
}

/// The body of an `app:Appearance`: an optional `app:theme` (XSD-first) then
/// `app:surfaceDataMember`s.
fn read_appearance_body<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    out: &mut Vec<ReadMaterial>,
) -> Result<()> {
    let mut theme = String::new();
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        let app = ns_is(&rr, NS_APP);
        match ev {
            Event::Start(e) => {
                let local = e.local_name();
                if app && local.as_ref() == b"theme" {
                    theme = read_text(reader, buf)?.trim().to_string();
                } else if app && local.as_ref() == b"surfaceDataMember" {
                    read_surface_data_member(reader, buf, &theme, out)?;
                } else {
                    skip_element(reader, buf)?;
                }
            }
            Event::End(e) if e.local_name().as_ref() == b"Appearance" => break,
            Event::Eof => return Err(eof("Appearance")),
            _ => {}
        }
    }
    Ok(())
}

/// An `app:surfaceDataMember` wraps one surface-data object. We handle
/// `app:X3DMaterial`; `app:ParameterizedTexture` (W-M5b) and other kinds are
/// skipped.
fn read_surface_data_member<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    theme: &str,
    out: &mut Vec<ReadMaterial>,
) -> Result<()> {
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        let app = ns_is(&rr, NS_APP);
        match ev {
            Event::Start(e) => {
                if app && e.local_name().as_ref() == b"X3DMaterial" {
                    let (material, targets) = read_x3d_material(reader, buf)?;
                    out.push(ReadMaterial {
                        theme: theme.to_string(),
                        material,
                        targets,
                    });
                } else {
                    skip_element(reader, buf)?;
                }
            }
            Event::End(e) if e.local_name().as_ref() == b"surfaceDataMember" => break,
            Event::Eof => return Err(eof("surfaceDataMember")),
            _ => {}
        }
    }
    Ok(())
}

/// Parse an `app:X3DMaterial` (positioned after its `Start`) into a CityJSON
/// material object and its `app:target` polygon ids. `gml:name` becomes the
/// material `name`; only the X3D properties actually present are set.
fn read_x3d_material<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
) -> Result<(Value, Vec<String>)> {
    let mut m = Map::new();
    let mut targets = Vec::new();
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        let app = ns_is(&rr, NS_APP);
        let gml = ns_is(&rr, NS_GML);
        match ev {
            Event::Start(e) => {
                let local = e.local_name();
                let name = local.as_ref();
                if gml && name == b"name" {
                    m.insert("name".to_string(), json!(read_text(reader, buf)?.trim()));
                } else if app {
                    match name {
                        b"diffuseColor" | b"emissiveColor" | b"specularColor" => {
                            let key = String::from_utf8_lossy(name).into_owned();
                            let rgb = parse_floats(&read_text(reader, buf)?)?;
                            m.insert(key, json!(rgb));
                        }
                        b"ambientIntensity" | b"shininess" | b"transparency" => {
                            let key = String::from_utf8_lossy(name).into_owned();
                            m.insert(key, json!(parse_scalar(&read_text(reader, buf)?)?));
                        }
                        b"isSmooth" => {
                            let v = read_text(reader, buf)?;
                            m.insert("isSmooth".to_string(), json!(v.trim() == "true"));
                        }
                        b"target" => {
                            let t = read_text(reader, buf)?;
                            if let Some(id) = t.trim().strip_prefix('#')
                                && !id.is_empty()
                            {
                                targets.push(id.to_string());
                            }
                        }
                        _ => skip_element(reader, buf)?,
                    }
                } else {
                    skip_element(reader, buf)?;
                }
            }
            Event::End(e) if e.local_name().as_ref() == b"X3DMaterial" => break,
            Event::Eof => return Err(eof("X3DMaterial")),
            _ => {}
        }
    }
    Ok((Value::Object(m), targets))
}

/// Parse a whitespace-separated list of floats (an X3D colour).
fn parse_floats(s: &str) -> Result<Vec<f64>> {
    s.split_whitespace()
        .map(|t| {
            t.parse::<f64>().map_err(|e| {
                CityParquetError::Schema(format!("invalid number in CityGML appearance: {e}"))
            })
        })
        .collect()
}

/// Parse a single float (an X3D scalar property).
fn parse_scalar(s: &str) -> Result<f64> {
    s.trim()
        .parse::<f64>()
        .map_err(|e| CityParquetError::Schema(format!("invalid number in CityGML appearance: {e}")))
}

fn eof(ctx: &str) -> CityParquetError {
    CityParquetError::Schema(format!("unexpected end of CityGML document inside <{ctx}>"))
}
