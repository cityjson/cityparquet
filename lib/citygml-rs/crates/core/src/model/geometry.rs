/// A ring's coordinates, exterior first then holes, without GML's closing
/// duplicate point.
pub type Ring = Vec<[f64; 3]>;

/// A line's points, in order.
pub type Line = Vec<[f64; 3]>;

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
    /// `MultiSurface`, a TIN's `TriangulatedSurface` or `Tin`) the polygon is
    /// read inside, innermost first: a
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
    MultiPoint(Vec<[f64; 3]>),
    MultiLineString(Vec<Line>),
    MultiSurface(Vec<Face>),
    /// Faces that join edge to edge into one surface: a TIN.
    CompositeSurface(Vec<Face>),
    /// Shells, exterior first.
    Solid(Vec<Vec<Face>>),
    /// Solids that share faces, each a list of shells.
    CompositeSolid(Vec<Vec<Vec<Face>>>),
    /// Solids that need not touch, each a list of shells.
    MultiSolid(Vec<Vec<Vec<Face>>>),
    /// A [`Template`] placed by an `ImplicitGeometry`.
    Instance(Instance),
}

/// An `ImplicitGeometry`: a shared relative geometry placed at a reference
/// point by a transformation matrix.
#[derive(Debug, Clone, PartialEq)]
pub struct Instance {
    /// The position of the relative geometry among the document's
    /// [`Template`]s.
    pub template: usize,
    /// The 4×4 transformation matrix, row by row.
    pub matrix: [f64; 16],
    /// Where the relative geometry's origin lands, in world coordinates.
    pub reference_point: [f64; 3],
}

/// A relative geometry (`relativeGMLGeometry`) that one or more
/// `ImplicitGeometry`s instantiate, in its own local coordinates.
#[derive(Debug, Clone, PartialEq)]
pub struct Template {
    /// The geometry's `gml:id`, what another `ImplicitGeometry`'s
    /// `relativeGMLGeometry xlink:href` names.
    pub id: Option<String>,
    /// The geometry, at the LoD of the property that defines it.
    pub geometry: Geometry,
    /// The materials and textures that target its faces.
    pub appearance: super::Appearance,
}

impl Shape {
    /// Every face, in order; none for points and lines.
    pub fn faces(&self) -> Box<dyn Iterator<Item = &Face> + '_> {
        match self {
            Shape::MultiPoint(_) | Shape::MultiLineString(_) | Shape::Instance(_) => {
                Box::new(std::iter::empty())
            }
            Shape::MultiSurface(faces) | Shape::CompositeSurface(faces) => Box::new(faces.iter()),
            Shape::Solid(shells) => Box::new(shells.iter().flatten()),
            Shape::CompositeSolid(solids) | Shape::MultiSolid(solids) => {
                Box::new(solids.iter().flatten().flatten())
            }
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
