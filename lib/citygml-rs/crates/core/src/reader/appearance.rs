//! A city object's appearance property, and the CityModel's
//! `appearanceMember`s, read into the model's [`Appearance`].
//!
//! An `app:Appearance` has an optional `app:theme` (`""` when absent) and
//! surface data (`app:surfaceDataMember` in CityGML 2.0, `app:surfaceData`
//! in 3.0). An `app:X3DMaterial` becomes a [`Material`] with the polygon ids
//! its `app:target`s name; an `app:ParameterizedTexture` a [`Texture`] with
//! the UVs of each ring its texture coordinates cover, the closing pair
//! dropped as a ring's closing point is. CityGML 2.0 puts those in a
//! `target`'s `TexCoordList`, each `textureCoordinates` naming its `ring`;
//! 3.0 in a `textureParameterization`'s `TextureAssociation`, whose
//! `TexCoordList` lists the coordinates and then the rings, paired in order.
//! Other surface data (`GeoreferencedTexture`, `TexCoordGen`) is skipped.

use std::io::BufRead;

use quick_xml::events::Event;
use quick_xml::reader::NsReader;

use crate::model::{Appearance, Material, RingUvs, Texture};
use crate::xml::{
    NS_GML, get_attr_local, gml_id, ns_is, read_text, skip_element, unexpected_eof, xml_err,
};
use crate::{Error, Result};

/// Read an appearance property (positioned after its `Start`, ending at
/// `End(end_tag)`): its `app:Appearance`, if it has one, the appearance
/// module being the namespace `app`.
pub fn read_appearance<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    end_tag: &[u8],
    app: &str,
) -> Result<Appearance> {
    let mut out = Appearance::default();
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        match ev {
            Event::Start(e) => {
                if ns_is(&rr, app) && e.local_name().as_ref() == b"Appearance" {
                    read_appearance_body(reader, buf, app, &mut out)?;
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
    app: &str,
    out: &mut Appearance,
) -> Result<()> {
    let mut theme = String::new();
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        let in_app = ns_is(&rr, app);
        match ev {
            Event::Start(e) => {
                let local = e.local_name();
                if in_app && local.as_ref() == b"theme" {
                    theme = read_text(reader, buf)?.trim().to_string();
                } else if in_app && matches!(local.as_ref(), b"surfaceDataMember" | b"surfaceData")
                {
                    let end = local.as_ref().to_vec();
                    read_surface_data(reader, buf, &end, app, &theme, out)?;
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

fn read_surface_data<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    end: &[u8],
    app: &str,
    theme: &str,
    out: &mut Appearance,
) -> Result<()> {
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        let in_app = ns_is(&rr, app);
        match ev {
            Event::Start(e) => {
                let local = e.local_name();
                if in_app && local.as_ref() == b"X3DMaterial" {
                    let id = gml_id(&e);
                    let mut material = read_x3d_material(reader, buf, app)?;
                    material.id = id;
                    material.theme = theme.to_string();
                    out.materials.push(material);
                } else if in_app && local.as_ref() == b"ParameterizedTexture" {
                    let mut texture = read_parameterized_texture(reader, buf, app)?;
                    texture.theme = theme.to_string();
                    out.textures.push(texture);
                } else {
                    skip_element(reader, buf)?;
                }
            }
            Event::End(e) if e.local_name().as_ref() == end => break,
            Event::Eof => return Err(unexpected_eof("surface data")),
            _ => {}
        }
    }
    Ok(())
}

/// An `app:X3DMaterial` (positioned after its `Start`). `gml:name` is its
/// name; only the X3D properties present are set, the last of a repeated one
/// winning.
fn read_x3d_material<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    app: &str,
) -> Result<Material> {
    let mut m = Material::default();
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        let in_app = ns_is(&rr, app);
        let gml = ns_is(&rr, NS_GML);
        match ev {
            Event::Start(e) => {
                let local = e.local_name();
                let name = local.as_ref();
                if gml && name == b"name" {
                    m.name = Some(read_text(reader, buf)?.trim().to_string());
                } else if in_app {
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
                            if let Some(id) = local_ref(&read_text(reader, buf)?) {
                                m.targets.push(id);
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
    app: &str,
) -> Result<Texture> {
    let mut t = Texture::default();
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        let in_app = ns_is(&rr, app);
        match ev {
            Event::Start(e) => {
                let local = e.local_name();
                let name = local.as_ref();
                if !in_app {
                    skip_element(reader, buf)?;
                } else if name == b"imageURI" {
                    t.image_uri = Some(read_text(reader, buf)?.trim().to_string());
                } else if name == b"mimeType" {
                    t.mime_type = Some(read_text(reader, buf)?.trim().to_string());
                } else if name == b"textureType" {
                    t.texture_type = Some(read_text(reader, buf)?.trim().to_string());
                } else if name == b"wrapMode" {
                    t.wrap_mode = Some(read_text(reader, buf)?.trim().to_string());
                } else if name == b"borderColor" {
                    t.border_color = Some(parse_floats(&read_text(reader, buf)?)?);
                } else if matches!(name, b"target" | b"textureParameterization") {
                    read_texture_parameters(reader, buf, app, &mut t.rings)?;
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

/// A texture's parameters (positioned after the `Start` of a CityGML 2.0
/// `target` or a 3.0 `textureParameterization`, consuming through its
/// `End`): each ring's id and UVs, from every `TexCoordList` inside, through
/// a 3.0 `TextureAssociation`. An `app:TexCoordGen` is skipped.
fn read_texture_parameters<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    app: &str,
    rings: &mut Vec<RingUvs>,
) -> Result<()> {
    let mut depth = 1usize;
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        let in_app = ns_is(&rr, app);
        match ev {
            Event::Start(e) => match e.local_name().as_ref() {
                b"TexCoordList" if in_app => read_tex_coord_list(reader, buf, app, rings)?,
                b"TextureAssociation" | b"textureParameterization" if in_app => depth += 1,
                _ => skip_element(reader, buf)?,
            },
            Event::End(_) => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            Event::Eof => return Err(unexpected_eof("texture parameterization")),
            _ => {}
        }
    }
    Ok(())
}

/// An `app:TexCoordList` (positioned after its `Start`): a CityGML 2.0
/// `textureCoordinates` names its ring in its `ring` attribute; 3.0 lists
/// the `textureCoordinates` and then the `ring`s, paired in order. Texture
/// coordinates naming no ring are skipped.
fn read_tex_coord_list<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    app: &str,
    rings: &mut Vec<RingUvs>,
) -> Result<()> {
    let ring_id = local_ref;
    // 3.0's coordinates, waiting for the `ring`s that follow them.
    let mut unpaired: Vec<Vec<[f64; 2]>> = Vec::new();
    let mut ids: Vec<String> = Vec::new();
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        let in_app = ns_is(&rr, app);
        match ev {
            Event::Start(e) if in_app && e.local_name().as_ref() == b"textureCoordinates" => {
                let attribute = get_attr_local(&e, b"ring");
                let uvs = parse_uvs(&read_text(reader, buf)?)?;
                match attribute {
                    Some(ring) => {
                        if let Some(id) = ring_id(&ring) {
                            rings.push((id, uvs));
                        }
                    }
                    None => unpaired.push(uvs),
                }
            }
            Event::Start(e) if in_app && e.local_name().as_ref() == b"ring" => {
                if let Some(id) = ring_id(&read_text(reader, buf)?) {
                    ids.push(id);
                }
            }
            Event::Start(_) => skip_element(reader, buf)?,
            Event::End(e) if e.local_name().as_ref() == b"TexCoordList" => break,
            Event::Eof => return Err(unexpected_eof("TexCoordList")),
            _ => {}
        }
    }
    rings.extend(ids.into_iter().zip(unpaired));
    Ok(())
}

/// The `gml:id` an appearance's `target` or `ring` names. Both are
/// `anyURI`s: `#id` is the usual spelling, but a bare `id` is the same
/// same-document reference (the published LoD3_Railway.gml names its rings
/// so), and a reference into another document names nothing here.
fn local_ref(text: &str) -> Option<String> {
    let text = text.trim();
    let id = text.strip_prefix('#').unwrap_or(text);
    (!id.is_empty() && !id.contains(['#', '/'])).then(|| id.to_string())
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
