//! A geometry's primitives in WKB face order, and its per-primitive values
//! (semantics, material, texture) flattened onto them.
//!
//! CityJSON nests `semantics`, `material` and `texture` values as deeply as
//! the geometry's boundaries: flat per surface for a MultiSurface, per shell
//! per surface for a Solid, per solid per shell per surface for a MultiSolid.
//! CityParquet stores them flat, one entry per WKB face, so every consumer
//! walks the same depth-first order defined here. A *face* is one surface of
//! a surface-bearing geometry, or one point or line of a MultiPoint or
//! MultiLineString (whose semantics are one entry per primitive).
//!
//! The flattening keeps each part aligned to its own boundary: a shell's
//! values are sized to that shell's surface count (padded with `None`, or
//! truncated), and a `null` standing for a whole shell or solid becomes one
//! `None` per face beneath it. A malformed part therefore never shifts a later
//! part's entries onto the wrong faces.

use cjseq::{
    Geometry, MaterialIndex, MaterialReference, MultiSolidValues, PrimitiveValues,
    SemanticsSurface, Shell, SolidValues, Surface, SurfaceIndex, TextureReference, TexturedRing,
    TexturedShell, TexturedSurface, Themes,
};

/// The boundary a geometry's values are aligned to.
#[derive(Clone, Copy)]
pub(crate) enum Faces<'g> {
    /// Points or lines: one entry each, no rings.
    Primitives(usize),
    Surfaces(&'g [Surface]),
    Solid(&'g [Shell]),
    Solids(&'g [Vec<Shell>]),
    /// A GeometryInstance, which has no faces of its own.
    Instance,
}

impl<'g> Faces<'g> {
    pub(crate) fn of(geom: &'g Geometry) -> Self {
        match geom {
            Geometry::MultiPoint { boundaries, .. } => Faces::Primitives(boundaries.len()),
            Geometry::MultiLineString { boundaries, .. } => Faces::Primitives(boundaries.len()),
            Geometry::MultiSurface { boundaries, .. }
            | Geometry::CompositeSurface { boundaries, .. } => Faces::Surfaces(boundaries),
            Geometry::Solid { boundaries, .. } => Faces::Solid(boundaries),
            Geometry::MultiSolid { boundaries, .. }
            | Geometry::CompositeSolid { boundaries, .. } => Faces::Solids(boundaries),
            Geometry::GeometryInstance { .. } => Faces::Instance,
        }
    }

    /// The number of faces, before any writer drop.
    pub(crate) fn count(&self) -> usize {
        match self {
            Faces::Primitives(n) => *n,
            Faces::Surfaces(s) => s.len(),
            Faces::Solid(shells) => shell_faces(shells),
            Faces::Solids(solids) => solids.iter().map(|s| shell_faces(s)).sum(),
            Faces::Instance => 0,
        }
    }

    /// Every surface, in WKB face order; empty for points, lines and
    /// instances.
    pub(crate) fn surfaces(&self) -> Vec<&'g Surface> {
        match *self {
            Faces::Surfaces(s) => s.iter().collect(),
            Faces::Solid(shells) => shells.iter().flatten().collect(),
            Faces::Solids(solids) => solids.iter().flatten().flatten().collect(),
            Faces::Primitives(_) | Faces::Instance => Vec::new(),
        }
    }

    /// Per face, the STORED vertex count of each of its rings: `Some(n)` for
    /// a ring the writer keeps, `None` for one it drops (the
    /// [`crate::wkb_write::normalise_ring`] rule). `n` is also the number of
    /// `[u, v]` pairs a textured ring carries; the WKB ring's closing repeat
    /// takes none.
    pub(crate) fn ring_vertex_counts(&self) -> Vec<Vec<Option<usize>>> {
        self.surfaces()
            .into_iter()
            .map(|surface| {
                surface
                    .iter()
                    .map(|ring| crate::wkb_write::distinct_ring_len(ring))
                    .collect()
            })
            .collect()
    }

    /// `values` flattened to one entry per face (see the module docs).
    pub(crate) fn flatten<I: Copy>(&self, values: Values<'_, I>) -> Vec<Option<I>> {
        let mut out = Vec::with_capacity(self.count());
        match (*self, values) {
            (Faces::Primitives(n), Values::Primitive(v)) => flat_into(v, n, &mut out),
            (Faces::Surfaces(s), Values::Primitive(v)) => flat_into(v, s.len(), &mut out),
            (Faces::Solid(shells), Values::Solid(v)) => solid_into(v, shells, &mut out),
            (Faces::Solids(solids), Values::Solids(v)) => {
                for (i, solid) in solids.iter().enumerate() {
                    match v.get(i) {
                        Some(Some(v)) => solid_into(v, solid, &mut out),
                        _ => out.extend(std::iter::repeat_n(None, shell_faces(solid))),
                    }
                }
            }
            // `null` for the whole geometry, or values whose shape is not the
            // geometry's (cjseq's types rule the latter out).
            _ => {}
        }
        out.resize(self.count(), None);
        out
    }

    /// Texture `values` flattened to one entry per face: the face's rings,
    /// or `None` where the source has no entry for that face.
    pub(crate) fn flatten_texture<'v>(
        &self,
        values: TextureValues<'v>,
    ) -> Vec<Option<&'v [TexturedRing]>> {
        let mut out = Vec::with_capacity(self.count());
        match (*self, values) {
            (Faces::Surfaces(s), TextureValues::Surfaces(v)) => {
                textured_surfaces_into(v, s.len(), &mut out);
            }
            (Faces::Solid(shells), TextureValues::Solid(v)) => {
                textured_shells_into(v, shells, &mut out);
            }
            (Faces::Solids(solids), TextureValues::Solids(v)) => {
                for (i, solid) in solids.iter().enumerate() {
                    match v.get(i) {
                        Some(v) => textured_shells_into(v, solid, &mut out),
                        None => out.extend(std::iter::repeat_n(None, shell_faces(solid))),
                    }
                }
            }
            _ => {}
        }
        out.resize(self.count(), None);
        out
    }
}

fn shell_faces(shells: &[Shell]) -> usize {
    shells.iter().map(Vec::len).sum()
}

/// Exactly `n` entries: a short list padded with `None`, a long one's
/// overflow ignored.
fn flat_into<I: Copy>(values: &[Option<I>], n: usize, out: &mut Vec<Option<I>>) {
    out.extend((0..n).map(|i| values.get(i).copied().flatten()));
}

fn solid_into<I: Copy>(values: &SolidValues<I>, shells: &[Shell], out: &mut Vec<Option<I>>) {
    for (i, shell) in shells.iter().enumerate() {
        match values.get(i) {
            Some(Some(v)) => flat_into(v, shell.len(), out),
            _ => out.extend(std::iter::repeat_n(None, shell.len())),
        }
    }
}

fn textured_surfaces_into<'v>(
    values: &'v [TexturedSurface],
    n: usize,
    out: &mut Vec<Option<&'v [TexturedRing]>>,
) {
    out.extend((0..n).map(|i| values.get(i).map(Vec::as_slice)));
}

fn textured_shells_into<'v>(
    values: &'v [TexturedShell],
    shells: &[Shell],
    out: &mut Vec<Option<&'v [TexturedRing]>>,
) {
    for (i, shell) in shells.iter().enumerate() {
        match values.get(i) {
            Some(v) => textured_surfaces_into(v, shell.len(), out),
            None => out.extend(std::iter::repeat_n(None, shell.len())),
        }
    }
}

/// Semantics or material values, at the depth their geometry nests them.
#[derive(Clone, Copy)]
pub(crate) enum Values<'v, I> {
    Primitive(&'v PrimitiveValues<I>),
    Solid(&'v SolidValues<I>),
    Solids(&'v MultiSolidValues<I>),
}

/// Texture values, at the depth their geometry nests them.
#[derive(Clone, Copy)]
pub(crate) enum TextureValues<'v> {
    Surfaces(&'v [TexturedSurface]),
    Solid(&'v [TexturedShell]),
    Solids(&'v [Vec<TexturedShell>]),
}

/// A geometry's semantic surfaces and, per face, the surface each refers to.
/// `None` when the geometry has no `semantics`.
pub(crate) fn semantics(
    geom: &Geometry,
) -> Option<(&[SemanticsSurface], Vec<Option<SurfaceIndex>>)> {
    let faces = Faces::of(geom);
    let (surfaces, values) = match geom {
        Geometry::MultiPoint { semantics, .. }
        | Geometry::MultiLineString { semantics, .. }
        | Geometry::MultiSurface { semantics, .. }
        | Geometry::CompositeSurface { semantics, .. } => {
            let s = semantics.as_ref()?;
            (&s.surfaces, s.values.as_ref().map(Values::Primitive))
        }
        Geometry::Solid { semantics, .. } => {
            let s = semantics.as_ref()?;
            (&s.surfaces, s.values.as_ref().map(Values::Solid))
        }
        Geometry::MultiSolid { semantics, .. } | Geometry::CompositeSolid { semantics, .. } => {
            let s = semantics.as_ref()?;
            (&s.surfaces, s.values.as_ref().map(Values::Solids))
        }
        Geometry::GeometryInstance { .. } => return None,
    };
    let flat = match values {
        Some(v) => faces.flatten(v),
        None => vec![None; faces.count()],
    };
    Some((surfaces, flat))
}

/// One theme of a geometry's `material`, flattened.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum MaterialTheme {
    /// `values`: one entry per face (`null` expanded).
    PerFace(Vec<Option<MaterialIndex>>),
    /// `value`: one material for the whole geometry.
    Whole(MaterialIndex),
    /// Neither member, which the schema does not allow.
    Missing,
}

/// A geometry's `material` themes, in source order, flattened onto its faces.
/// `None` when the geometry has no `material` member.
pub(crate) fn material_themes(geom: &Geometry) -> Option<Vec<(&str, MaterialTheme)>> {
    let faces = Faces::of(geom);
    fn themes<'g, V>(
        faces: Faces<'_>,
        map: &'g Option<Themes<MaterialReference<V>>>,
        wrap: for<'v> fn(&'v V) -> Values<'v, MaterialIndex>,
    ) -> Option<Vec<(&'g str, MaterialTheme)>> {
        let map = map.as_ref()?;
        Some(
            map.iter()
                .map(|(name, m)| {
                    let theme = match (&m.values, m.value) {
                        (Some(Some(v)), _) => MaterialTheme::PerFace(faces.flatten(wrap(v))),
                        (Some(None), _) => MaterialTheme::PerFace(vec![None; faces.count()]),
                        (None, Some(value)) => MaterialTheme::Whole(value),
                        (None, None) => MaterialTheme::Missing,
                    };
                    (name.as_str(), theme)
                })
                .collect(),
        )
    }
    match geom {
        Geometry::MultiSurface { material, .. } | Geometry::CompositeSurface { material, .. } => {
            themes(faces, material, |v| Values::Primitive(v))
        }
        Geometry::Solid { material, .. } => themes(faces, material, |v| Values::Solid(v)),
        Geometry::MultiSolid { material, .. } | Geometry::CompositeSolid { material, .. } => {
            themes(faces, material, |v| Values::Solids(v))
        }
        _ => None,
    }
}

/// One `texture` theme: its name and, per face, the face's rings (see
/// [`texture_themes`]).
pub(crate) type TextureTheme<'g> = (&'g str, Option<Vec<Option<&'g [TexturedRing]>>>);

/// A geometry's `texture` themes, in source order, each flattened onto its
/// faces (`None` for a theme with no `values`, which the format cannot
/// place). `None` when the geometry has no `texture` member.
pub(crate) fn texture_themes(geom: &Geometry) -> Option<Vec<TextureTheme<'_>>> {
    let faces = Faces::of(geom);
    fn themes<'g, V>(
        faces: Faces<'_>,
        map: &'g Option<Themes<TextureReference<V>>>,
        wrap: fn(&'g V) -> TextureValues<'g>,
    ) -> Option<Vec<TextureTheme<'g>>> {
        let map = map.as_ref()?;
        Some(
            map.iter()
                .map(|(name, t)| {
                    let per_face = t.values.as_ref().map(|v| faces.flatten_texture(wrap(v)));
                    (name.as_str(), per_face)
                })
                .collect(),
        )
    }
    match geom {
        Geometry::MultiSurface { texture, .. } | Geometry::CompositeSurface { texture, .. } => {
            themes(faces, texture, |v| TextureValues::Surfaces(v))
        }
        Geometry::Solid { texture, .. } => themes(faces, texture, |v| TextureValues::Solid(v)),
        Geometry::MultiSolid { texture, .. } | Geometry::CompositeSolid { texture, .. } => {
            themes(faces, texture, |v| TextureValues::Solids(v))
        }
        _ => None,
    }
}

/// A geometry parsed from JSON, for tests that need a shape no fixture has.
#[cfg(test)]
pub(crate) fn geometry(v: serde_json::Value) -> Geometry {
    serde_json::from_value(v).expect("a valid CityJSON geometry")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn idx(v: &[Option<SurfaceIndex>]) -> Vec<Option<usize>> {
        v.iter().map(|i| i.map(|i| i.0)).collect()
    }

    /// A malformed `semantics.values` with a shell SHORTER than its boundary
    /// must not shift the next shell's entries onto the wrong faces. A Solid
    /// with two 2-face shells and values `[[0], [1, 1]]` flattens to
    /// `[0, null, 1, 1]` (shell 0 padded), not `[0, 1, 1, null]`.
    #[test]
    fn flatten_sizes_each_shell_to_its_boundary() {
        let geom = geometry(json!({
            "type": "Solid", "lod": "2",
            "boundaries": [[[[0, 1, 2]], [[0, 1, 3]]], [[[0, 2, 3]], [[1, 2, 3]]]],
            "semantics": {"surfaces": [{"type": "RoofSurface"}, {"type": "WallSurface"}],
                          "values": [[0], [1, 1]]}
        }));
        let (_, flat) = semantics(&geom).unwrap();
        assert_eq!(idx(&flat), vec![Some(0), None, Some(1), Some(1)]);
    }

    /// A `null` standing for a whole solid expands to one `None` per face of
    /// that solid, and the next solid's entries stay on its own faces.
    #[test]
    fn null_solid_expands_to_its_faces() {
        let geom = geometry(json!({
            "type": "MultiSolid", "lod": "2",
            "boundaries": [[[[[0, 1, 2]], [[0, 1, 3]]]], [[[[0, 2, 3]]]]],
            "semantics": {"surfaces": [{"type": "RoofSurface"}], "values": [null, [[0]]]}
        }));
        let (_, flat) = semantics(&geom).unwrap();
        assert_eq!(idx(&flat), vec![None, None, Some(0)]);
    }

    #[test]
    fn multipoint_semantics_are_one_entry_per_point() {
        let geom = geometry(json!({
            "type": "MultiPoint", "lod": "1",
            "boundaries": [0, 1, 2],
            "semantics": {"surfaces": [{"type": "+Pole"}], "values": [0, null]}
        }));
        let (_, flat) = semantics(&geom).unwrap();
        assert_eq!(idx(&flat), vec![Some(0), None, None]);
    }

    #[test]
    fn material_value_and_null_values_are_kept_apart() {
        let geom = geometry(json!({
            "type": "MultiSurface", "lod": "2",
            "boundaries": [[[0, 1, 2]], [[0, 2, 3]]],
            "material": {"a": {"value": 3}, "b": {"values": null}, "c": {"values": [1]}}
        }));
        assert_eq!(
            material_themes(&geom).unwrap(),
            vec![
                ("a", MaterialTheme::Whole(MaterialIndex(3))),
                ("b", MaterialTheme::PerFace(vec![None, None])),
                (
                    "c",
                    MaterialTheme::PerFace(vec![Some(MaterialIndex(1)), None])
                ),
            ]
        );
    }

    #[test]
    fn texture_faces_follow_the_shells() {
        let geom = geometry(json!({
            "type": "Solid", "lod": "2",
            "boundaries": [[[[0, 1, 2]], [[0, 1, 3]]], [[[0, 2, 3]]]],
            "texture": {"t": {"values": [[[[0, 0, 1, 2]]]]}}
        }));
        let themes = texture_themes(&geom).unwrap();
        let faces = themes[0].1.as_ref().unwrap();
        assert_eq!(faces.len(), 3);
        assert!(faces[0].is_some());
        assert!(faces[1].is_none() && faces[2].is_none());
    }
}
