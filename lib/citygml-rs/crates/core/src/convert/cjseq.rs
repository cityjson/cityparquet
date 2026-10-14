//! [`GmlObject`] to [`cjseq::CityJSONFeature`].
//!
//! One top-level object and its children become one feature with its own
//! vertex pool (CityJSONSeq style), quantised against the transform the
//! caller chose, and its own appearance palettes. The conversion visits the
//! objects depth first, parent before children, and pushes vertices and
//! interns appearance in that order, so the output is deterministic.
//!
//! CityGML class names become CityJSON types through
//! [`crate::vocabulary::cityjson_type`]; boundary surfaces keep their CityGML
//! local name, which is the CityJSON semantic surface type for every surface
//! the readers produce.

use std::collections::HashMap;

use cjseq::{
    Appearance as CjAppearance, BorderColor, CityJSONFeature, CityObject, CityObjectAddress,
    CityObjectType, Geometry as CjGeometry, Lod, MaterialIndex, MaterialObject, MaterialReference,
    SemanticSurfaceType, Semantics, SemanticsSurface, SurfaceIndex, TextureFormat, TextureIndex,
    TextureObject, TextureReference, TexturedRing, Transform, UvIndex, VertexIndex,
};
use serde_json::{Map, Value as Json};

use super::FromGml;
use crate::model::{
    Address, Attribute, Face, Geometry, GmlObject, Material, Shape, Texture, Value,
};
use crate::vocabulary::{cityjson_type, relation_to_construction};
use crate::{Error, Result};

/// What the CityJSON conversion needs besides the object.
#[derive(Debug, Clone)]
pub struct FeatureContext {
    /// The transform every vertex is quantised against.
    pub transform: Transform,
    /// The object's 1-based position in the document, which names a
    /// top-level object that has no `gml:id`.
    pub index: usize,
}

impl FromGml for CityJSONFeature {
    type Context = FeatureContext;

    fn from_gml(object: &GmlObject, context: &FeatureContext) -> Result<Self> {
        let mut vertices = VertexPool::new(&context.transform);
        let mut appearance = Palettes::default();
        let mut feature = CityJSONFeature::new();
        let root_id = object.id.clone().unwrap_or_else(|| {
            let ty = cityjson_type(&object.class).unwrap_or(&object.class.local);
            format!("{ty}_{}", context.index)
        });
        feature.id = root_id.clone();
        emit(
            object,
            &root_id,
            None,
            &mut vertices,
            &mut appearance,
            &mut feature,
        )?;
        feature.vertices = vertices.into_vertices();
        if !appearance.materials.is_empty() || !appearance.textures.is_empty() {
            feature.appearance = Some(CjAppearance {
                materials: (!appearance.materials.is_empty()).then_some(appearance.materials),
                textures: (!appearance.textures.is_empty()).then_some(appearance.textures),
                vertices_texture: (!appearance.uvs.is_empty()).then_some(appearance.uvs),
                default_theme_texture: None,
                default_theme_material: None,
            });
        }
        Ok(feature)
    }
}

/// Add `object` (and, depth first, its children) to `feature` as City
/// Objects, linked by `parents` and `children`.
fn emit(
    object: &GmlObject,
    my_id: &str,
    parent_id: Option<&str>,
    vertices: &mut VertexPool,
    palettes: &mut Palettes,
    feature: &mut CityJSONFeature,
) -> Result<()> {
    let mut geometries = object
        .geometries
        .iter()
        .map(|g| geometry(g, object, vertices))
        .collect::<Result<Vec<_>>>()?;
    apply_materials(object, &mut geometries, palettes)?;
    apply_textures(object, &mut geometries, palettes)?;

    let child_ids: Vec<String> = object
        .children
        .iter()
        .enumerate()
        .map(|(i, c)| c.id.clone().unwrap_or_else(|| format!("{my_id}_part_{i}")))
        .collect();

    let type_name = cityjson_type(&object.class).ok_or_else(|| {
        Error::invalid(format!(
            "CityGML class {} has no CityJSON City Object type",
            object.class.local
        ))
    })?;
    let thetype = CityObjectType::try_from(type_name.to_string()).map_err(build_error)?;
    let mut co = CityObject::new(thetype);
    co.geometry = Some(geometries);
    let mut attrs = attributes(&object.attributes);
    if let Some(relation) = relation_to_construction(&object.class) {
        attrs
            .entry("relationToConstruction")
            .or_insert_with(|| Json::String(relation.to_string()));
    }
    if !attrs.is_empty() {
        co.attributes = Some(attrs);
    }
    if !object.addresses.is_empty() {
        co.address = Some(
            object
                .addresses
                .iter()
                .map(|a| address(a, vertices))
                .collect::<Result<_>>()?,
        );
    }
    if let Some(p) = parent_id {
        co.parents = Some(vec![p.to_string()]);
    }
    if !child_ids.is_empty() {
        co.children = Some(child_ids.clone());
    }
    if feature.city_objects.contains_key(my_id) {
        return Err(Error::invalid(format!(
            "duplicate CityObject id {my_id:?} within one feature"
        )));
    }
    feature.add_co(my_id.to_string(), co);

    for (child, id) in object.children.iter().zip(&child_ids) {
        emit(child, id, Some(my_id), vertices, palettes, feature)?;
    }
    Ok(())
}

fn build_error(e: impl std::fmt::Display) -> Error {
    Error::invalid(format!("failed to build CityObject from CityGML: {e}"))
}

/// Attributes as a JSON object, a repeated name accumulating into an array
/// (CityGML allows repeated `bldg:function` and repeated generic names).
fn attributes(attributes: &[Attribute]) -> Map<String, Json> {
    let mut map = Map::new();
    for a in attributes {
        let value = json(&a.value);
        match map.get_mut(a.name.local()) {
            None => {
                map.insert(a.name.local().to_string(), value);
            }
            Some(Json::Array(existing)) => existing.push(value),
            Some(slot) => {
                let prev = slot.take();
                *slot = Json::Array(vec![prev, value]);
            }
        }
    }
    map
}

fn json(value: &Value) -> Json {
    match value {
        Value::String(s) => Json::String(s.clone()),
        Value::Integer(i) => Json::from(*i),
        Value::Double(f) => Json::from(*f),
        Value::Boolean(b) => Json::Bool(*b),
        Value::List(items) => Json::Array(items.iter().map(json).collect()),
    }
}

/// The CityJSON member an xAL leaf becomes: the names CityJSON's own
/// example uses (`country`, `locality`, `thoroughfareName`,
/// `thoroughfareNumber`, `postcode`), a few more in the same style, and
/// otherwise the xAL name with its first letter lowered.
fn address_key(xal: &str) -> String {
    let known = match xal {
        "CountryName" => "country",
        "AdministrativeAreaName" => "administrativeArea",
        "SubAdministrativeAreaName" => "subAdministrativeArea",
        "LocalityName" => "locality",
        "DependentLocalityName" => "dependentLocality",
        "ThoroughfareName" => "thoroughfareName",
        "ThoroughfareNumber" => "thoroughfareNumber",
        "PostalCodeNumber" => "postcode",
        "PostBoxNumber" => "postBox",
        "BuildingName" => "buildingName",
        "PremiseName" => "premiseName",
        "PremiseNumber" => "premiseNumber",
        "AddressLine" => "freeText",
        _ => {
            let mut chars = xal.chars();
            return chars
                .next()
                .map_or_else(String::new, |c| c.to_lowercase().chain(chars).collect());
        }
    };
    known.to_string()
}

/// One address: its xAL leaves as members, a repeated one accumulating into
/// an array, and its location a LoD 1 MultiPoint.
fn address(a: &Address, vertices: &mut VertexPool) -> Result<CityObjectAddress> {
    let mut other = Map::new();
    for (name, text) in &a.fields {
        let key = address_key(name);
        let value = Json::String(text.clone());
        match other.get_mut(&key) {
            None => {
                other.insert(key, value);
            }
            Some(Json::Array(existing)) => existing.push(value),
            Some(slot) => {
                let prev = slot.take();
                *slot = Json::Array(vec![prev, value]);
            }
        }
    }
    let location = if a.location.is_empty() {
        None
    } else {
        Some(CjGeometry::MultiPoint {
            lod: "1".parse().map_err(build_error)?,
            boundaries: a
                .location
                .iter()
                .map(|&p| vertices.push(p))
                .collect::<Result<_>>()?,
            semantics: None,
        })
    };
    Ok(CityObjectAddress { location, other })
}

/// One geometry, without appearance (see [`apply_materials`] and
/// [`apply_textures`]), its vertices pushed in face order.
fn geometry(g: &Geometry, object: &GmlObject, vertices: &mut VertexPool) -> Result<CjGeometry> {
    let lod: Lod = g.lod.parse().map_err(build_error)?;
    let (surfaces, renumber) = semantic_surfaces(g, object)?;
    let semantic = |f: &Face| f.surface.map(|s| SurfaceIndex(renumber[&s]));
    Ok(match &g.shape {
        Shape::MultiPoint(points) => CjGeometry::MultiPoint {
            lod,
            boundaries: points
                .iter()
                .map(|&p| vertices.push(p))
                .collect::<Result<_>>()?,
            semantics: None,
        },
        Shape::MultiLineString(lines) => CjGeometry::MultiLineString {
            lod,
            boundaries: lines
                .iter()
                .map(|line| line.iter().map(|&p| vertices.push(p)).collect())
                .collect::<Result<_>>()?,
            semantics: None,
        },
        Shape::MultiSurface(faces) => CjGeometry::MultiSurface {
            lod,
            boundaries: faces
                .iter()
                .map(|f| vertices.surface(f))
                .collect::<Result<_>>()?,
            semantics: surfaces.map(|surfaces| Semantics {
                surfaces,
                values: Some(faces.iter().map(semantic).collect()),
                other: Map::new(),
            }),
            material: None,
            texture: None,
        },
        Shape::CompositeSurface(faces) => CjGeometry::CompositeSurface {
            lod,
            boundaries: faces
                .iter()
                .map(|f| vertices.surface(f))
                .collect::<Result<_>>()?,
            semantics: surfaces.map(|surfaces| Semantics {
                surfaces,
                values: Some(faces.iter().map(semantic).collect()),
                other: Map::new(),
            }),
            material: None,
            texture: None,
        },
        Shape::Solid(shells) => CjGeometry::Solid {
            lod,
            boundaries: shells
                .iter()
                .map(|s| s.iter().map(|f| vertices.surface(f)).collect())
                .collect::<Result<_>>()?,
            semantics: surfaces.map(|surfaces| Semantics {
                surfaces,
                values: Some(
                    shells
                        .iter()
                        .map(|s| Some(s.iter().map(semantic).collect()))
                        .collect(),
                ),
                other: Map::new(),
            }),
            material: None,
            texture: None,
        },
        Shape::CompositeSolid(solids) => CjGeometry::CompositeSolid {
            lod,
            boundaries: solids
                .iter()
                .map(|solid| {
                    solid
                        .iter()
                        .map(|s| s.iter().map(|f| vertices.surface(f)).collect())
                        .collect()
                })
                .collect::<Result<_>>()?,
            semantics: surfaces.map(|surfaces| Semantics {
                surfaces,
                values: Some(
                    solids
                        .iter()
                        .map(|solid| {
                            Some(
                                solid
                                    .iter()
                                    .map(|s| Some(s.iter().map(semantic).collect()))
                                    .collect(),
                            )
                        })
                        .collect(),
                ),
                other: Map::new(),
            }),
            material: None,
            texture: None,
        },
    })
}

/// A geometry's semantic surfaces, and where each object-level surface
/// index lands among them.
type SurfaceSubset = (Option<Vec<SemanticsSurface>>, HashMap<usize, usize>);

/// The boundary surfaces a geometry's faces are tagged with, and those they
/// are nested in, in the object's order, with each object-level index's
/// position among them. `None` when no face is tagged: the geometry then has
/// no `semantics` member.
fn semantic_surfaces(g: &Geometry, object: &GmlObject) -> Result<SurfaceSubset> {
    let all = &object.boundary_surfaces;
    let mut used = std::collections::BTreeSet::new();
    for face in g.shape.faces() {
        let mut next = face.surface;
        while let Some(s) = next {
            let surface = all.get(s).ok_or_else(|| {
                Error::invalid(format!("face tagged with missing boundary surface {s}"))
            })?;
            if !used.insert(s) {
                break;
            }
            next = surface.parent;
        }
    }
    if used.is_empty() {
        return Ok((None, HashMap::new()));
    }
    let renumber: HashMap<usize, usize> = used.iter().enumerate().map(|(i, &s)| (s, i)).collect();
    let surfaces = used
        .iter()
        .map(|&s| {
            let surface = &all[s];
            let children: Vec<SurfaceIndex> = used
                .iter()
                .filter(|&&c| all[c].parent == Some(s))
                .map(|c| SurfaceIndex(renumber[c]))
                .collect();
            Ok(SemanticsSurface {
                thetype: SemanticSurfaceType::try_from(surface.class.local.clone())
                    .map_err(build_error)?,
                parent: surface.parent.map(|p| SurfaceIndex(renumber[&p])),
                children: (!children.is_empty()).then_some(children),
                other: Map::new(),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok((Some(surfaces), renumber))
}

/// Intern the object's materials and stamp each geometry's `material`. A
/// material is interned even when it targets nothing, so a definition
/// survives; a theme that colours no face of a geometry is left off it. Two
/// materials on one face and theme: the later wins.
fn apply_materials(
    object: &GmlObject,
    geometries: &mut [CjGeometry],
    palettes: &mut Palettes,
) -> Result<()> {
    if object.appearance.materials.is_empty() {
        return Ok(());
    }
    let mut themes: Vec<&str> = Vec::new();
    let mut by_theme: HashMap<&str, HashMap<&str, MaterialIndex>> = HashMap::new();
    for m in &object.appearance.materials {
        let idx = palettes.intern_material(m)?;
        if !by_theme.contains_key(m.theme.as_str()) {
            themes.push(&m.theme);
        }
        let targets = by_theme.entry(&m.theme).or_default();
        for t in &m.targets {
            targets.insert(t, idx);
        }
    }
    // A face is coloured through its own id, else through the innermost
    // aggregate holding it that a material targets.
    let lookup = |theme: &str, f: &Face| {
        let targets = &by_theme[theme];
        f.id.as_deref()
            .into_iter()
            .chain(f.polygon.groups.iter().map(String::as_str))
            .find_map(|id| targets.get(id))
            .copied()
    };
    for (g, cj) in object.geometries.iter().zip(geometries.iter_mut()) {
        // The themes that colour some face of this geometry.
        let colouring: Vec<&str> = themes
            .iter()
            .copied()
            .filter(|theme| g.shape.faces().any(|f| lookup(theme, f).is_some()))
            .collect();
        if colouring.is_empty() {
            continue;
        }
        match (cj, &g.shape) {
            (CjGeometry::MultiSurface { material, .. }, Shape::MultiSurface(faces))
            | (CjGeometry::CompositeSurface { material, .. }, Shape::CompositeSurface(faces)) => {
                *material = Some(
                    colouring
                        .iter()
                        .map(|theme| {
                            let values = faces.iter().map(|f| lookup(theme, f)).collect();
                            (theme.to_string(), material_values(values))
                        })
                        .collect(),
                );
            }
            (CjGeometry::Solid { material, .. }, Shape::Solid(shells)) => {
                *material = Some(
                    colouring
                        .iter()
                        .map(|theme| {
                            let values = shells
                                .iter()
                                .map(|s| Some(s.iter().map(|f| lookup(theme, f)).collect()))
                                .collect();
                            (theme.to_string(), material_values(values))
                        })
                        .collect(),
                );
            }
            (CjGeometry::CompositeSolid { material, .. }, Shape::CompositeSolid(solids)) => {
                *material = Some(
                    colouring
                        .iter()
                        .map(|theme| {
                            let values = solids
                                .iter()
                                .map(|solid| {
                                    Some(
                                        solid
                                            .iter()
                                            .map(|s| {
                                                Some(s.iter().map(|f| lookup(theme, f)).collect())
                                            })
                                            .collect(),
                                    )
                                })
                                .collect();
                            (theme.to_string(), material_values(values))
                        })
                        .collect(),
                );
            }
            _ => unreachable!("each geometry is built from its own shape"),
        }
    }
    Ok(())
}

fn material_values<V>(values: V) -> MaterialReference<V> {
    MaterialReference {
        values: Some(Some(values)),
        value: None,
        other: Map::new(),
    }
}

/// Intern the object's textures and their UVs, and stamp each geometry's
/// `texture`. A ring no texture of a theme covers is `[null]`; a theme that
/// covers no ring of a geometry is left off it. A face reached through a
/// reversed `gml:OrientableSurface` has its rings wound backwards, and its UVs
/// are reversed with them.
fn apply_textures(
    object: &GmlObject,
    geometries: &mut [CjGeometry],
    palettes: &mut Palettes,
) -> Result<()> {
    if object.appearance.textures.is_empty() {
        return Ok(());
    }
    type RingMap<'a> = HashMap<&'a str, (TextureIndex, Vec<UvIndex>)>;
    let mut themes: Vec<&str> = Vec::new();
    let mut by_theme: HashMap<&str, RingMap> = HashMap::new();
    for t in &object.appearance.textures {
        let idx = palettes.intern_texture(t)?;
        if !by_theme.contains_key(t.theme.as_str()) {
            themes.push(&t.theme);
        }
        let rings = by_theme.entry(&t.theme).or_default();
        for (ring_id, uvs) in &t.rings {
            let uvs = uvs.iter().map(|&uv| palettes.intern_uv(uv)).collect();
            rings.insert(ring_id, (idx, uvs));
        }
    }
    for (g, cj) in object.geometries.iter().zip(geometries.iter_mut()) {
        let mut covering: Vec<(&str, &RingMap)> = Vec::new();
        for theme in &themes {
            let map = &by_theme[theme];
            let hit = g.shape.faces().any(|f| {
                f.polygon
                    .ring_ids
                    .iter()
                    .flatten()
                    .any(|id| map.contains_key(id.as_str()))
            });
            if hit {
                covering.push((theme, map));
            }
        }
        if covering.is_empty() {
            continue;
        }
        let face = |f: &Face, map: &RingMap| -> Result<Vec<TexturedRing>> {
            f.polygon
                .ring_ids
                .iter()
                .map(|id| match id.as_deref().and_then(|id| map.get(id)) {
                    Some((texture, uvs)) => {
                        if uvs.is_empty() {
                            return Err(build_error(format!(
                                "the texture coordinates of ring {:?} are empty",
                                id.as_deref().unwrap_or_default()
                            )));
                        }
                        let uvs = if f.reversed {
                            uvs.iter().rev().copied().collect()
                        } else {
                            uvs.clone()
                        };
                        Ok(TexturedRing::Textured {
                            texture: *texture,
                            uvs,
                        })
                    }
                    None => Ok(TexturedRing::Untextured),
                })
                .collect()
        };
        match (cj, &g.shape) {
            (CjGeometry::MultiSurface { texture, .. }, Shape::MultiSurface(faces))
            | (CjGeometry::CompositeSurface { texture, .. }, Shape::CompositeSurface(faces)) => {
                *texture = Some(
                    covering
                        .iter()
                        .map(|(theme, map)| {
                            let values =
                                faces.iter().map(|f| face(f, map)).collect::<Result<_>>()?;
                            Ok((theme.to_string(), texture_values(values)))
                        })
                        .collect::<Result<_>>()?,
                );
            }
            (CjGeometry::Solid { texture, .. }, Shape::Solid(shells)) => {
                *texture = Some(
                    covering
                        .iter()
                        .map(|(theme, map)| {
                            let values = shells
                                .iter()
                                .map(|s| s.iter().map(|f| face(f, map)).collect())
                                .collect::<Result<_>>()?;
                            Ok((theme.to_string(), texture_values(values)))
                        })
                        .collect::<Result<_>>()?,
                );
            }
            (CjGeometry::CompositeSolid { texture, .. }, Shape::CompositeSolid(solids)) => {
                *texture = Some(
                    covering
                        .iter()
                        .map(|(theme, map)| {
                            let values = solids
                                .iter()
                                .map(|solid| {
                                    solid
                                        .iter()
                                        .map(|s| s.iter().map(|f| face(f, map)).collect())
                                        .collect()
                                })
                                .collect::<Result<_>>()?;
                            Ok((theme.to_string(), texture_values(values)))
                        })
                        .collect::<Result<_>>()?,
                );
            }
            _ => unreachable!("each geometry is built from its own shape"),
        }
    }
    Ok(())
}

fn texture_values<V>(values: V) -> TextureReference<V> {
    TextureReference {
        values: Some(values),
        other: Map::new(),
    }
}

/// A feature's material, texture and texture-vertex palettes. Materials and
/// textures are deduplicated by their serialised form, texture vertices by
/// the exact bits of both coordinates.
#[derive(Default)]
struct Palettes {
    materials: Vec<MaterialObject>,
    material_ids: HashMap<String, usize>,
    textures: Vec<TextureObject>,
    texture_ids: HashMap<String, usize>,
    uvs: Vec<[f64; 2]>,
    uv_ids: HashMap<(u64, u64), usize>,
}

impl Palettes {
    /// A material without a `gml:name` is named by its `gml:id`, and one
    /// with neither by its place in the palette (`material-2`), since
    /// CityJSON requires a name. Identical unnamed materials are one entry.
    fn intern_material(&mut self, m: &Material) -> Result<MaterialIndex> {
        let mut object = material_object(m)?;
        let key = serde_json::to_string(&object).map_err(build_error)?;
        let next = self.materials.len();
        let idx = *self.material_ids.entry(key).or_insert_with(|| {
            if object.name.is_empty() {
                object.name = format!("material-{next}");
            }
            self.materials.push(object);
            next
        });
        Ok(MaterialIndex(idx))
    }

    fn intern_texture(&mut self, t: &Texture) -> Result<TextureIndex> {
        let object = texture_object(t)?;
        let key = serde_json::to_string(&object).map_err(build_error)?;
        let next = self.textures.len();
        let idx = *self.texture_ids.entry(key).or_insert_with(|| {
            self.textures.push(object);
            next
        });
        Ok(TextureIndex(idx))
    }

    fn intern_uv(&mut self, uv: [f64; 2]) -> UvIndex {
        let next = self.uvs.len();
        UvIndex(
            *self
                .uv_ids
                .entry((uv[0].to_bits(), uv[1].to_bits()))
                .or_insert_with(|| {
                    self.uvs.push(uv);
                    next
                }),
        )
    }
}

/// A CityJSON number: a non-finite value has no JSON form, so it is absent.
fn finite(v: Option<f64>) -> Option<f64> {
    v.filter(|v| v.is_finite())
}

fn material_object(m: &Material) -> Result<MaterialObject> {
    let invalid = |what: String| Error::invalid(format!("CityGML material: {what}"));
    let color = |name: &str, c: &Option<Vec<f64>>| -> Result<Option<[f64; 3]>> {
        match c.as_deref() {
            None => Ok(None),
            Some(&[r, g, b]) if r.is_finite() && g.is_finite() && b.is_finite() => {
                Ok(Some([r, g, b]))
            }
            Some(other) => Err(invalid(format!("{name} {other:?} is not three numbers"))),
        }
    };
    Ok(MaterialObject {
        name: m.name.clone().or_else(|| m.id.clone()).unwrap_or_default(),
        ambient_intensity: finite(m.ambient_intensity),
        diffuse_color: color("diffuseColor", &m.diffuse_color)?,
        emissive_color: color("emissiveColor", &m.emissive_color)?,
        specular_color: color("specularColor", &m.specular_color)?,
        shininess: finite(m.shininess),
        transparency: finite(m.transparency),
        is_smooth: m.is_smooth,
    })
}

fn texture_object(t: &Texture) -> Result<TextureObject> {
    let invalid = |what: String| Error::invalid(format!("CityGML texture: {what}"));
    // CityJSON names a texture's format by one of two words; CityGML gives
    // a MIME type, and any other image type has no CityJSON form.
    let thetype =
        t.mime_type
            .as_deref()
            .and_then(|mime| match mime.to_ascii_lowercase().as_str() {
                "image/png" => Some(TextureFormat::PNG),
                "image/jpeg" | "image/jpg" => Some(TextureFormat::JPG),
                _ => None,
            });
    fn word<T: serde::de::DeserializeOwned>(name: &str, v: &Option<String>) -> Result<Option<T>> {
        v.as_ref()
            .map(|s| serde_json::from_value(Json::String(s.clone())))
            .transpose()
            .map_err(|e| Error::invalid(format!("CityGML texture: {name}: {e}")))
    }
    let border_color = match t.border_color.as_deref() {
        None => None,
        Some(&[r, g, b]) => Some(BorderColor::Rgb([r, g, b])),
        Some(&[r, g, b, a]) => Some(BorderColor::Rgba([r, g, b, a])),
        Some(other) => {
            return Err(invalid(format!(
                "borderColor {other:?} is not three or four numbers"
            )));
        }
    };
    Ok(TextureObject {
        thetype,
        image: t.image_uri.clone(),
        wrap_mode: word("wrapMode", &t.wrap_mode)?,
        texture_type: word("textureType", &t.texture_type)?,
        border_color,
    })
}

/// A feature's vertex pool: world coordinates quantised against the
/// transform, each distinct integer vertex stored once.
struct VertexPool {
    scale: [f64; 3],
    translate: [f64; 3],
    index: HashMap<[i64; 3], usize>,
    vertices: Vec<[i64; 3]>,
}

impl VertexPool {
    fn new(transform: &Transform) -> Self {
        Self {
            scale: transform.scale,
            translate: transform.translate,
            index: HashMap::new(),
            vertices: Vec::new(),
        }
    }

    fn push(&mut self, coord: [f64; 3]) -> Result<VertexIndex> {
        let mut q = [0i64; 3];
        for i in 0..3 {
            let c = coord[i];
            if !c.is_finite() {
                return Err(Error::invalid(format!(
                    "non-finite CityGML coordinate component: {c}"
                )));
            }
            let rounded = ((c - self.translate[i]) / self.scale[i]).round();
            // f64 -> i64 saturates rather than wrapping, so the range is
            // checked rather than silently clamped.
            if !rounded.is_finite() || rounded.abs() >= 9.223_372_036_854_775e18 {
                return Err(Error::invalid(format!(
                    "CityGML coordinate {c} out of range after quantisation"
                )));
            }
            q[i] = rounded as i64;
        }
        let next = self.vertices.len();
        Ok(VertexIndex(*self.index.entry(q).or_insert_with(|| {
            self.vertices.push(q);
            next
        })))
    }

    /// A face's rings, exterior first, wound backwards when the face is
    /// reversed.
    fn surface(&mut self, face: &Face) -> Result<Vec<Vec<VertexIndex>>> {
        let poly = &face.polygon;
        std::iter::once(&poly.exterior)
            .chain(&poly.interiors)
            .map(|ring| {
                if face.reversed {
                    ring.iter().rev().map(|&c| self.push(c)).collect()
                } else {
                    ring.iter().map(|&c| self.push(c)).collect()
                }
            })
            .collect()
    }

    fn into_vertices(self) -> Vec<[i64; 3]> {
        self.vertices
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_vertex_pool_quantises_and_dedups() {
        let transform = Transform {
            scale: [0.001; 3],
            translate: [0.0; 3],
        };
        let mut pool = VertexPool::new(&transform);
        let a = pool.push([50.0, 0.0, 150.0]).unwrap();
        let b = pool.push([50.0, 0.0, 150.0]).unwrap();
        let c = pool.push([0.0, 0.0, 0.0]).unwrap();
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(pool.into_vertices(), vec![[50_000, 0, 150_000], [0, 0, 0]]);
        assert!(
            VertexPool::new(&transform)
                .push([f64::NAN, 0.0, 0.0])
                .is_err()
        );
    }
}
