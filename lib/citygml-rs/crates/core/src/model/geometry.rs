/// A ring's coordinates, exterior first then holes, without GML's closing
/// duplicate point.
pub type Ring = Vec<[f64; 3]>;

/// A polygon as GML writes it: an exterior ring and its holes.
#[derive(Debug, Clone, PartialEq)]
pub struct Polygon {
    /// The polygon's `gml:id`: what an `xlink:href` or an appearance target
    /// names. `None` for an anonymous inline polygon.
    pub id: Option<String>,
    pub exterior: Ring,
    pub interiors: Vec<Ring>,
    /// The `gml:id` of each ring, exterior first: what a texture's
    /// `ring="#id"` names.
    pub ring_ids: Vec<Option<String>>,
    /// The `gml:id`s of the identified surface aggregates (`CompositeSurface`,
    /// `MultiSurface`) the polygon is read inside, innermost first: a
    /// material targeting one of them colours the polygon.
    pub groups: Vec<String>,
}

/// One face of a geometry: a polygon in place, with the orientation it is
/// used in and the boundary surface it belongs to.
#[derive(Debug, Clone, PartialEq)]
pub struct Face {
    pub polygon: Polygon,
    /// The id an appearance targets this face by: the `xlink:href` it was
    /// reached through, else the polygon's own `gml:id`.
    pub id: Option<String>,
    /// Reached through a reversed `gml:OrientableSurface`: its rings are
    /// wound backwards, and so are their texture coordinates.
    pub reversed: bool,
    /// The [`super::GmlObject::boundary_surfaces`] entry this face belongs
    /// to.
    pub surface: Option<usize>,
}

/// How a geometry's faces are arranged.
#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    MultiSurface(Vec<Face>),
    /// Shells, exterior first.
    Solid(Vec<Vec<Face>>),
    /// Solids, each a list of shells.
    CompositeSolid(Vec<Vec<Vec<Face>>>),
}

impl Shape {
    /// Every face, in order.
    pub fn faces(&self) -> Box<dyn Iterator<Item = &Face> + '_> {
        match self {
            Shape::MultiSurface(faces) => Box::new(faces.iter()),
            Shape::Solid(shells) => Box::new(shells.iter().flatten()),
            Shape::CompositeSolid(solids) => Box::new(solids.iter().flatten().flatten()),
        }
    }
}

/// One geometry of an object at one LoD.
#[derive(Debug, Clone, PartialEq)]
pub struct Geometry {
    /// The LoD as the element name gives it (`"2"` for `lod2Solid`).
    pub lod: String,
    pub shape: Shape,
}
