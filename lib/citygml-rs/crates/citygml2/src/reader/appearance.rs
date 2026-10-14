//! `app:appearance` (a feature's) and `app:appearanceMember` (the
//! CityModel's), read into the model's [`Appearance`].
//!
//! An `app:Appearance` has an optional `app:theme` (`""` when absent) and
//! `app:surfaceDataMember`s. An `app:X3DMaterial` becomes a [`Material`] with
//! the polygon ids its `app:target`s name; an `app:ParameterizedTexture` a
//! [`Texture`] with the UVs of each ring its `app:textureCoordinates` cover,
//! the closing pair dropped as a ring's closing point is. Other surface data
//! (`GeoreferencedTexture`, `TexCoordGen`) is skipped.

use std::io::BufRead;

use citygml_core::model::{Appearance, Material, RingUvs, Texture};
use citygml_core::xml::{
    NS_GML, get_attr_local, gml_id, ns_is, read_text, skip_element, unexpected_eof, xml_err,
};
use citygml_core::{Error, Result};
use quick_xml::events::Event;
use quick_xml::reader::NsReader;

use crate::ns;

/// Read an appearance property (positioned after its `Start`, ending at
/// `End(end_tag)`): its `app:Appearance`, if it has one.
pub fn read_appearance<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    end_tag: &[u8],
) -> Result<Appearance> {
    let mut out = Appearance::default();
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        match ev {
            Event::Start(e) => {
                if ns_is(&rr, ns::APPEARANCE) && e.local_name().as_ref() == b"Appearance" {
                    read_appearance_body(reader, buf, &mut out)?;
                } else {
                    skip_element(reader, buf)?;
                }
            }
            Event::End(e) if e.local_name().as_ref() == end_tag => break,
            Event::Eof => return Err(unexpected_eof("appearance")),
            _ => {}
        }
    }
    Ok(out)
}

fn read_appearance_body<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    out: &mut Appearance,
) -> Result<()> {
    let mut theme = String::new();
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        let app = ns_is(&rr, ns::APPEARANCE);
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
            Event::Eof => return Err(unexpected_eof("Appearance")),
            _ => {}
        }
    }
    Ok(())
}

fn read_surface_data_member<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    theme: &str,
    out: &mut Appearance,
) -> Result<()> {
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        let app = ns_is(&rr, ns::APPEARANCE);
        match ev {
            Event::Start(e) => {
                let local = e.local_name();
                if app && local.as_ref() == b"X3DMaterial" {
                    let id = gml_id(&e);
                    let mut material = read_x3d_material(reader, buf)?;
                    material.id = id;
                    material.theme = theme.to_string();
                    out.materials.push(material);
                } else if app && local.as_ref() == b"ParameterizedTexture" {
                    let mut texture = read_parameterized_texture(reader, buf)?;
                    texture.theme = theme.to_string();
                    out.textures.push(texture);
                } else {
                    skip_element(reader, buf)?;
                }
            }
            Event::End(e) if e.local_name().as_ref() == b"surfaceDataMember" => break,
            Event::Eof => return Err(unexpected_eof("surfaceDataMember")),
            _ => {}
        }
    }
    Ok(())
}

/// An `app:X3DMaterial` (positioned after its `Start`). `gml:name` is its
/// name; only the X3D properties present are set, the last of a repeated one
/// winning.
fn read_x3d_material<R: BufRead>(reader: &mut NsReader<R>, buf: &mut Vec<u8>) -> Result<Material> {
    let mut m = Material::default();
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        let app = ns_is(&rr, ns::APPEARANCE);
        let gml = ns_is(&rr, NS_GML);
        match ev {
            Event::Start(e) => {
                let local = e.local_name();
                let name = local.as_ref();
                if gml && name == b"name" {
                    m.name = Some(read_text(reader, buf)?.trim().to_string());
                } else if app {
                    match name {
                        b"diffuseColor" => {
                            m.diffuse_color = Some(parse_floats(&read_text(reader, buf)?)?)
                        }
                        b"emissiveColor" => {
                            m.emissive_color = Some(parse_floats(&read_text(reader, buf)?)?)
                        }
                        b"specularColor" => {
                            m.specular_color = Some(parse_floats(&read_text(reader, buf)?)?)
                        }
                        b"ambientIntensity" => {
                            m.ambient_intensity = Some(parse_scalar(&read_text(reader, buf)?)?)
                        }
                        b"shininess" => m.shininess = Some(parse_scalar(&read_text(reader, buf)?)?),
                        b"transparency" => {
                            m.transparency = Some(parse_scalar(&read_text(reader, buf)?)?)
                        }
                        b"isSmooth" => {
                            // An XML Schema boolean: "true" and "1" are true.
                            let v = read_text(reader, buf)?;
                            m.is_smooth = Some(matches!(v.trim(), "true" | "1"));
                        }
                        b"target" => {
                            let t = read_text(reader, buf)?;
                            if let Some(id) = t.trim().strip_prefix('#')
                                && !id.is_empty()
                            {
                                m.targets.push(id.to_string());
                            }
                        }
                        _ => skip_element(reader, buf)?,
                    }
                } else {
                    skip_element(reader, buf)?;
                }
            }
            Event::End(e) if e.local_name().as_ref() == b"X3DMaterial" => break,
            Event::Eof => return Err(unexpected_eof("X3DMaterial")),
            _ => {}
        }
    }
    Ok(m)
}

/// An `app:ParameterizedTexture` (positioned after its `Start`).
fn read_parameterized_texture<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
) -> Result<Texture> {
    let mut t = Texture::default();
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        let app = ns_is(&rr, ns::APPEARANCE);
        match ev {
            Event::Start(e) => {
                let local = e.local_name();
                let name = local.as_ref();
                if app && name == b"imageURI" {
                    t.image_uri = Some(read_text(reader, buf)?.trim().to_string());
                } else if app && name == b"mimeType" {
                    t.mime_type = Some(read_text(reader, buf)?.trim().to_string());
                } else if app && name == b"textureType" {
                    t.texture_type = Some(read_text(reader, buf)?.trim().to_string());
                } else if app && name == b"wrapMode" {
                    t.wrap_mode = Some(read_text(reader, buf)?.trim().to_string());
                } else if app && name == b"borderColor" {
                    t.border_color = Some(parse_floats(&read_text(reader, buf)?)?);
                } else if app && name == b"target" {
                    read_texture_target(reader, buf, &mut t.rings)?;
                } else {
                    skip_element(reader, buf)?;
                }
            }
            Event::End(e) if e.local_name().as_ref() == b"ParameterizedTexture" => break,
            Event::Eof => return Err(unexpected_eof("ParameterizedTexture")),
            _ => {}
        }
    }
    Ok(t)
}

/// An `app:target` (a `TexCoordList` of per-ring `app:textureCoordinates`):
/// each ring's id and UVs. A `textureCoordinates` with no `ring` id, and an
/// `app:TexCoordGen`, are skipped.
fn read_texture_target<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    rings: &mut Vec<RingUvs>,
) -> Result<()> {
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        let app = ns_is(&rr, ns::APPEARANCE);
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
                } else if app && local.as_ref() == b"TexCoordList" {
                    read_texture_target(reader, buf, rings)?;
                } else {
                    skip_element(reader, buf)?;
                }
            }
            Event::End(e) if e.local_name().as_ref() == b"target" => break,
            Event::End(e) if e.local_name().as_ref() == b"TexCoordList" => break,
            Event::Eof => return Err(unexpected_eof("texture target")),
            _ => {}
        }
    }
    Ok(())
}

/// A whitespace-separated UV list as `[u, v]` pairs, without the closing
/// pair (a GML texture ring is closed, like the ring itself).
fn parse_uvs(s: &str) -> Result<Vec<[f64; 2]>> {
    let nums = parse_floats(s)?;
    if nums.len() % 2 != 0 {
        return Err(Error::invalid(format!(
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

fn parse_floats(s: &str) -> Result<Vec<f64>> {
    s.split_whitespace()
        .map(|t| {
            t.parse::<f64>()
                .map_err(|e| Error::invalid(format!("invalid number in CityGML appearance: {e}")))
        })
        .collect()
}

fn parse_scalar(s: &str) -> Result<f64> {
    s.trim()
        .parse::<f64>()
        .map_err(|e| Error::invalid(format!("invalid number in CityGML appearance: {e}")))
}
