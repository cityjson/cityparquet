use std::collections::{BTreeSet, HashMap, HashSet};

/// The materials and textures that apply to one object, in document order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Appearance {
    pub materials: Vec<Material>,
    pub textures: Vec<Texture>,
}

impl Appearance {
    pub fn is_empty(&self) -> bool {
        self.materials.is_empty() && self.textures.is_empty()
    }
}

/// An `X3DMaterial`: its theme, what it colours, and the X3D properties the
/// document gives.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Material {
    /// `""` when the appearance names no theme.
    pub theme: String,
    /// The `gml:id`s of the polygons it colours.
    pub targets: Vec<String>,
    /// The `X3DMaterial`'s own `gml:id`.
    pub id: Option<String>,
    pub name: Option<String>,
    pub ambient_intensity: Option<f64>,
    pub diffuse_color: Option<Vec<f64>>,
    pub emissive_color: Option<Vec<f64>>,
    pub specular_color: Option<Vec<f64>>,
    pub shininess: Option<f64>,
    pub transparency: Option<f64>,
    pub is_smooth: Option<bool>,
}

/// One ring's texture coordinates: the ring's `gml:id` and one UV per vertex,
/// without the closing pair.
pub type RingUvs = (String, Vec<[f64; 2]>);

/// A `ParameterizedTexture`: its theme, the image, and the UVs of each ring
/// it covers.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Texture {
    /// `""` when the appearance names no theme.
    pub theme: String,
    pub rings: Vec<RingUvs>,
    pub image_uri: Option<String>,
    pub mime_type: Option<String>,
    pub texture_type: Option<String>,
    pub wrap_mode: Option<String>,
    pub border_color: Option<Vec<f64>>,
}

/// The CityModel-level appearance of a document, indexed by target and ring
/// `gml:id`, so scoping it to one object costs that object's ids rather than
/// every definition in the document.
#[derive(Debug, Default)]
pub struct ModelAppearance {
    materials: Vec<Material>,
    textures: Vec<Texture>,
    material_by_target: HashMap<String, Vec<usize>>,
    texture_by_ring: HashMap<String, Vec<usize>>,
}

impl ModelAppearance {
    pub fn new(appearance: Appearance) -> Self {
        let mut material_by_target: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, m) in appearance.materials.iter().enumerate() {
            for t in &m.targets {
                material_by_target.entry(t.clone()).or_default().push(i);
            }
        }
        let mut texture_by_ring: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, t) in appearance.textures.iter().enumerate() {
            for (rid, _) in &t.rings {
                texture_by_ring.entry(rid.clone()).or_default().push(i);
            }
        }
        Self {
            materials: appearance.materials,
            textures: appearance.textures,
            material_by_target,
            texture_by_ring,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.materials.is_empty() && self.textures.is_empty()
    }

    /// The materials targeting any of `face_ids`, and the textures covering
    /// any of `ring_ids` with only those rings kept, each in document order.
    pub fn scoped_for(
        &self,
        face_ids: &HashSet<String>,
        ring_ids: &HashSet<String>,
    ) -> (Vec<Material>, Vec<Texture>) {
        let mut material_idx: BTreeSet<usize> = BTreeSet::new();
        for id in face_ids {
            if let Some(v) = self.material_by_target.get(id) {
                material_idx.extend(v);
            }
        }
        let materials = material_idx
            .iter()
            .map(|&i| self.materials[i].clone())
            .collect();

        let mut texture_idx: BTreeSet<usize> = BTreeSet::new();
        for id in ring_ids {
            if let Some(v) = self.texture_by_ring.get(id) {
                texture_idx.extend(v);
            }
        }
        let textures = texture_idx
            .iter()
            .map(|&i| {
                let t = &self.textures[i];
                Texture {
                    theme: t.theme.clone(),
                    rings: t
                        .rings
                        .iter()
                        .filter(|(rid, _)| ring_ids.contains(rid))
                        .cloned()
                        .collect(),
                    image_uri: t.image_uri.clone(),
                    mime_type: t.mime_type.clone(),
                    texture_type: t.texture_type.clone(),
                    wrap_mode: t.wrap_mode.clone(),
                    border_color: t.border_color.clone(),
                }
            })
            .collect();
        (materials, textures)
    }
}
