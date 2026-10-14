use super::{Address, Appearance, Attribute, Geometry, ModelAppearance};

/// A CityGML thematic module. The same module keeps its name across
/// versions; each version's reader maps its own namespace URIs onto these.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Module {
    Core,
    Appearance,
    Bridge,
    Building,
    CityFurniture,
    CityObjectGroup,
    Generics,
    LandUse,
    Relief,
    Transportation,
    Tunnel,
    Vegetation,
    WaterBody,
}

/// Where a name comes from.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Namespace {
    /// A CityGML module.
    Module(Module),
    /// GML itself.
    Gml,
    /// Any other namespace, by its URI.
    Other(String),
}

/// A name qualified by its namespace: a class (`bldg:Building`), a property
/// (`bldg:function`) or a boundary surface (`bldg:WallSurface`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct QName {
    pub ns: Namespace,
    pub local: String,
}

impl QName {
    pub fn new(ns: Namespace, local: impl Into<String>) -> Self {
        Self {
            ns,
            local: local.into(),
        }
    }

    /// A name in a CityGML module.
    pub fn module(module: Module, local: impl Into<String>) -> Self {
        Self::new(Namespace::Module(module), local)
    }
}

/// One thematic boundary surface of an object (`WallSurface`, `TrafficArea`,
/// a `Door` inside an opening). Faces refer to it by its position in
/// [`GmlObject::boundary_surfaces`].
#[derive(Debug, Clone, PartialEq)]
pub struct BoundarySurface {
    pub class: QName,
    /// The surface this one is nested in: a wall, for the `Door` or `Window`
    /// in one of its openings.
    pub parent: Option<usize>,
}

/// One city object as CityGML describes it, with its geometry resolved: every
/// `xlink:href` replaced by the polygon it names.
///
/// A top-level object owns its sub-objects (parts, installations, construction
/// elements) in [`Self::children`], in document order, so one `GmlObject` is
/// what a CityJSON feature holds.
#[derive(Debug, Clone, PartialEq)]
pub struct GmlObject {
    /// The `gml:id`, if the document gives one.
    pub id: Option<String>,
    /// The CityGML class (`bldg:Building`, `tran:Road`).
    pub class: QName,
    /// Attributes in document order. A name may repeat.
    pub attributes: Vec<Attribute>,
    /// The postal addresses, in document order.
    pub addresses: Vec<Address>,
    /// The geometries, in the order a conversion emits them.
    pub geometries: Vec<Geometry>,
    /// The object's boundary surfaces, which [`super::Face::surface`]
    /// indexes.
    pub boundary_surfaces: Vec<BoundarySurface>,
    /// Materials and textures that apply to this object's faces: those of the
    /// CityModel that target them first, then the object's own.
    pub appearance: Appearance,
    pub children: Vec<GmlObject>,
}

impl GmlObject {
    /// Every polygon `gml:id` and ring `gml:id` this object's own geometry
    /// carries (not its children's): what an appearance can target on it.
    fn targetable_ids(
        &self,
    ) -> (
        std::collections::HashSet<String>,
        std::collections::HashSet<String>,
    ) {
        let mut faces = std::collections::HashSet::new();
        let mut rings = std::collections::HashSet::new();
        for geometry in &self.geometries {
            for face in geometry.shape.faces() {
                if let Some(id) = &face.id {
                    faces.insert(id.clone());
                }
                faces.extend(face.polygon.groups.iter().cloned());
                rings.extend(face.polygon.ring_ids.iter().flatten().cloned());
            }
        }
        (faces, rings)
    }

    /// Put ahead of each object's own appearance the CityModel-level
    /// definitions that target its faces or rings, recursively. A texture
    /// keeps only the rings of the object it is copied to, so a texture
    /// spanning many buildings never carries unrelated UVs into one of them.
    pub fn scope_model_appearance(&mut self, model: &ModelAppearance) {
        if model.is_empty() {
            return;
        }
        let (faces, rings) = self.targetable_ids();
        let (materials, textures) = model.scoped_for(&faces, &rings);
        if !materials.is_empty() {
            self.appearance.materials.splice(0..0, materials);
        }
        if !textures.is_empty() {
            self.appearance.textures.splice(0..0, textures);
        }
        for child in &mut self.children {
            child.scope_model_appearance(model);
        }
    }
}
