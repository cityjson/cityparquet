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
//! `app:theme` on write). `app:ParameterizedTexture` becomes a CityJSON texture
//! with per-ring UV coordinates (the GML closing UV pair dropped, symmetric with
//! the ring's closing point).

use std::io::BufRead;

use cityparquet_schema::{CityParquetError, Result};
use quick_xml::events::Event;
use quick_xml::reader::NsReader;
use serde_json::{Map, Value, json};

use super::xml::{NS_APP, NS_GML, get_attr_local, ns_is, read_text, skip_element, xml_err};

/// One `app:X3DMaterial`: its theme, the CityJSON material object, and the
/// `gml:id`s of the polygons it targets (`app:target="#id"`, `#` stripped).
pub struct ReadMaterial {
    pub theme: String,
    pub material: Value,
    pub targets: Vec<String>,
}

/// One ring's texture coordinates: `(ring gml:id, per-vertex UVs)`.
pub type RingUvs = (String, Vec<[f64; 2]>);

/// One `app:ParameterizedTexture`: its theme, the CityJSON texture object, and
/// the per-ring UV coordinates (keyed by the ring `gml:id` the
/// `app:textureCoordinates ring="#id"` targets; the closing pair dropped).
pub struct ReadTexture {
    pub theme: String,
    pub texture: Value,
    pub rings: Vec<RingUvs>,
}

/// The appearance parsed from one `app:appearance` property.
#[derive(Default)]
pub struct ReadAppearance {
    pub materials: Vec<ReadMaterial>,
    pub textures: Vec<ReadTexture>,
}

/// Read an `app:appearance` property (positioned after its `Start`): descend to
/// the inner `app:Appearance` and parse its surface data. Consumes through the
/// property's `End`. A property with no `app:Appearance` child (empty or
/// `xlink`-only) yields nothing.
pub fn read_appearance<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
) -> Result<ReadAppearance> {
    let mut out = ReadAppearance::default();
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
    out: &mut ReadAppearance,
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

/// An `app:surfaceDataMember` wraps one surface-data object: an
/// `app:X3DMaterial` or `app:ParameterizedTexture` (other kinds are skipped).
fn read_surface_data_member<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    theme: &str,
    out: &mut ReadAppearance,
) -> Result<()> {
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        let app = ns_is(&rr, NS_APP);
        match ev {
            Event::Start(e) => {
                let local = e.local_name();
                if app && local.as_ref() == b"X3DMaterial" {
                    let (material, targets) = read_x3d_material(reader, buf)?;
                    out.materials.push(ReadMaterial {
                        theme: theme.to_string(),
                        material,
                        targets,
                    });
                } else if app && local.as_ref() == b"ParameterizedTexture" {
                    let (texture, rings) = read_parameterized_texture(reader, buf)?;
                    out.textures.push(ReadTexture {
                        theme: theme.to_string(),
                        texture,
                        rings,
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

/// Parse an `app:ParameterizedTexture` (positioned after its `Start`) into a
/// CityJSON texture object and its per-ring UV coordinates (keyed by ring
/// `gml:id`; the GML closing UV pair dropped). Unhandled foreign forms
/// (`app:TexCoordGen`, texture coords with no `ring` id) are skipped.
fn read_parameterized_texture<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
) -> Result<(Value, Vec<RingUvs>)> {
    let mut t = Map::new();
    let mut rings = Vec::new();
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        let app = ns_is(&rr, NS_APP);
        match ev {
            Event::Start(e) => {
                let local = e.local_name();
                let name = local.as_ref();
                if app && name == b"imageURI" {
                    t.insert("image".to_string(), json!(read_text(reader, buf)?.trim()));
                } else if app && name == b"mimeType" {
                    if let Some(ty) = mime_to_type(read_text(reader, buf)?.trim()) {
                        t.insert("type".to_string(), json!(ty));
                    }
                } else if app && name == b"textureType" {
                    t.insert(
                        "textureType".to_string(),
                        json!(read_text(reader, buf)?.trim()),
                    );
                } else if app && name == b"wrapMode" {
                    t.insert(
                        "wrapMode".to_string(),
                        json!(read_text(reader, buf)?.trim()),
                    );
                } else if app && name == b"borderColor" {
                    let rgba = parse_floats(&read_text(reader, buf)?)?;
                    t.insert("borderColor".to_string(), json!(rgba));
                } else if app && name == b"target" {
                    read_texture_target(reader, buf, &mut rings)?;
                } else {
                    skip_element(reader, buf)?;
                }
            }
            Event::End(e) if e.local_name().as_ref() == b"ParameterizedTexture" => break,
            Event::Eof => return Err(eof("ParameterizedTexture")),
            _ => {}
        }
    }
    Ok((Value::Object(t), rings))
}

/// An `app:target` (a `TexCoordList` of per-ring `app:textureCoordinates`):
/// collect each ring's `(ring gml:id, UVs)` — the closing UV pair dropped
/// (symmetric with the ring's closing point). A `textureCoordinates` with no
/// `ring` id, or `app:TexCoordGen`, is skipped.
fn read_texture_target<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    rings: &mut Vec<RingUvs>,
) -> Result<()> {
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        let app = ns_is(&rr, NS_APP);
        match ev {
            Event::Start(e) => {
                let local = e.local_name();
                if app && local.as_ref() == b"textureCoordinates" {
                    let ring_id = get_attr_local(&e, b"ring")
                        .and_then(|r| r.trim().strip_prefix('#').map(str::to_string));
                    let uvs = parse_uvs(&read_text(reader, buf)?)?;
                    if let Some(id) = ring_id {
                        rings.push((id, uvs));
                    }
                } else {
                    // Descend through app:TexCoordList (and skip TexCoordGen).
                    if app && local.as_ref() == b"TexCoordList" {
                        read_texture_target(reader, buf, rings)?;
                    } else {
                        skip_element(reader, buf)?;
                    }
                }
            }
            Event::End(e) if e.local_name().as_ref() == b"target" => break,
            Event::End(e) if e.local_name().as_ref() == b"TexCoordList" => break,
            Event::Eof => return Err(eof("texture target")),
            _ => {}
        }
    }
    Ok(())
}

/// CityGML `app:mimeType` -> CityJSON texture `type` (only PNG/JPG survive).
fn mime_to_type(mime: &str) -> Option<&'static str> {
    match mime.to_ascii_lowercase().as_str() {
        "image/png" => Some("PNG"),
        "image/jpeg" | "image/jpg" => Some("JPG"),
        _ => None,
    }
}

/// Parse a whitespace-separated UV list into `[u, v]` pairs, dropping the closing
/// pair (a GML texture ring is closed; CityJSON UVs are not).
fn parse_uvs(s: &str) -> Result<Vec<[f64; 2]>> {
    let nums = parse_floats(s)?;
    if nums.len() % 2 != 0 {
        return Err(CityParquetError::Schema(format!(
            "CityGML textureCoordinates has an odd number of values ({})",
            nums.len()
        )));
    }
    let mut uvs: Vec<[f64; 2]> = nums.chunks(2).map(|c| [c[0], c[1]]).collect();
    if uvs.len() >= 2 && uvs.first() == uvs.last() {
        uvs.pop();
    }
    Ok(uvs)
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
