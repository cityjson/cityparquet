//! The materials and textures 3DCityDB maps onto an object's geometries.
//!
//! A database keeps an appearance's surface data apart from the geometry
//! it covers: `surface_data_mapping` links one surface data to one
//! geometry row, naming the polygons a material colours and giving each
//! textured polygon's texture coordinates, ring by ring. An object's
//! appearance is every surface data mapped onto its own geometries and its
//! boundary surfaces', one material or texture per surface data and theme,
//! as the CityGML readers read a document's appearance scoped to one
//! object.

use std::collections::{BTreeMap, HashMap};

use citygml_core::model::{Appearance, Material, Texture};
use serde_json::Value as Json;

use crate::geometry::ring_id;
use crate::rows::{MappingRow, SurfaceData};

/// The appearance mapped onto `geometries` (each a `geometry_data` id with
/// its mappings), counting in `dropped` what has no CityJSON form.
pub(crate) fn appearance<'a>(
    geometries: impl IntoIterator<Item = &'a [MappingRow]>,
    surface_data: &HashMap<i64, SurfaceData>,
    dropped: &mut BTreeMap<String, usize>,
) -> Appearance {
    // One entry per surface data and theme, in the order first met.
    let mut order: Vec<(i64, String)> = Vec::new();
    let mut materials: HashMap<(i64, String), Material> = HashMap::new();
    let mut textures: HashMap<(i64, String), Texture> = HashMap::new();
    for mappings in geometries {
        for m in mappings {
            let Some(sd) = surface_data.get(&m.surface_data) else {
                continue;
            };
            let key = (m.surface_data, m.theme.clone());
            match sd.class.as_str() {
                "X3DMaterial" => {
                    let material = materials.entry(key.clone()).or_insert_with(|| {
                        order.push(key.clone());
                        material(sd, &m.theme)
                    });
                    if let Some(targets) = m.material.as_ref().and_then(Json::as_object) {
                        material.targets.extend(
                            targets
                                .iter()
                                .filter(|(_, on)| on.as_bool() != Some(false))
                                .map(|(id, _)| id.clone()),
                        );
                    }
                }
                "ParameterizedTexture" => {
                    let texture = textures.entry(key.clone()).or_insert_with(|| {
                        order.push(key.clone());
                        texture(sd, &m.theme)
                    });
                    if let Some(polygons) = m.texture.as_ref().and_then(Json::as_object) {
                        for (polygon, rings) in polygons {
                            for (k, ring) in rings.as_array().into_iter().flatten().enumerate() {
                                texture.rings.push((ring_id(polygon, k), uvs(ring)));
                            }
                        }
                    }
                }
                other => *dropped.entry(format!("app:{other}")).or_insert(0) += 1,
            }
        }
    }
    let mut out = Appearance::default();
    for key in order {
        if let Some(material) = materials.remove(&key) {
            out.materials.push(material);
        } else if let Some(texture) = textures.remove(&key) {
            if texture.image_uri.is_none() {
                // CityJSON requires a texture's image.
                *dropped
                    .entry("app:ParameterizedTexture without an image".into())
                    .or_insert(0) += 1;
            } else {
                out.textures.push(texture);
            }
        }
    }
    out
}

fn material(sd: &SurfaceData, theme: &str) -> Material {
    Material {
        theme: theme.to_string(),
        targets: Vec::new(),
        id: sd.objectid.clone().filter(|id| !crate::is_generated_id(id)),
        name: None,
        ambient_intensity: sd.ambient_intensity,
        diffuse_color: sd.diffuse_color.as_deref().and_then(colour),
        emissive_color: sd.emissive_color.as_deref().and_then(colour),
        specular_color: sd.specular_color.as_deref().and_then(colour),
        shininess: sd.shininess,
        transparency: sd.transparency,
        is_smooth: sd.is_smooth.map(|s| s != 0),
    }
}

fn texture(sd: &SurfaceData, theme: &str) -> Texture {
    Texture {
        theme: theme.to_string(),
        rings: Vec::new(),
        image_uri: sd.image_uri.clone(),
        mime_type: sd.mime_type.clone(),
        texture_type: sd.texture_type.clone(),
        wrap_mode: sd.wrap_mode.clone(),
        border_color: sd.border_color.as_deref().and_then(colour),
    }
}

/// A ring's texture coordinates, without the closing pair.
fn uvs(ring: &Json) -> Vec<[f64; 2]> {
    let mut uvs: Vec<[f64; 2]> = ring
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|uv| {
            let uv = uv.as_array()?;
            Some([uv.first()?.as_f64()?, uv.get(1)?.as_f64()?])
        })
        .collect();
    if uvs.len() >= 2 && uvs.first() == uvs.last() {
        uvs.pop();
    }
    uvs
}

/// A colour 3DCityDB stores as `#RRGGBB` or `#RRGGBBAA`, as components
/// between 0 and 1.
fn colour(hex: &str) -> Option<Vec<f64>> {
    let digits = hex.strip_prefix('#')?;
    if !matches!(digits.len(), 6 | 8) || !digits.is_ascii() {
        return None;
    }
    (0..digits.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&digits[i..i + 2], 16)
                .ok()
                .map(|c| f64::from(c) / 255.0)
        })
        .collect()
}
