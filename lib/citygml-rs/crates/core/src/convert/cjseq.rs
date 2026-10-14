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
//!
//! What an ADE adds becomes what a CityJSON Extension adds: a class, a
//! surface or a property of a CityGML object is named with a `+`
//! (`+UseDistrict`, `+buildingDetailAttribute`), as is another ADE's
//! property of an ADE object, and an ADE object's own properties are
//! attributes without one. A property's XML becomes JSON:
//! - an element is a key, its local name, and a repeated element an array;
//! - the GML property and object elements both stay, so the XML can be
//!   rebuilt without the schema;
//! - an element in a namespace other than its parent's keeps its prefix
//!   (`uro:DataQualityAttribute` inside a `urf:` property), or is named
//!   `{uri}local` when the document was read without its ADE index;
//! - an empty element is an empty object;
//! - text is a string, or a number for a measure (an element with a `uom`);
//! - an XML attribute is `@name` beside the text in `$`, unless it has no
//!   prefix and every occurrence of its element in the document carries it
//!   with one value,
//!   when [`extensions`] states it once as `xmlAttributes`;
//! - GML geometry inside an ADE property is left out.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use cjseq::{
    Appearance as CjAppearance, BorderColor, CityJSONFeature, CityObject, CityObjectAddress,
    CityObjectType, Extension, Geometry as CjGeometry, GeometryTemplates, Lod, MaterialIndex,
    MaterialObject, MaterialReference, SemanticSurfaceType, Semantics, SemanticsSurface,
    SurfaceIndex, TemplateIndex, TextureFormat, TextureIndex, TextureObject, TextureReference,
    TexturedRing, Transform, UvIndex, VertexIndex,
};
use serde_json::{Map, Value as Json};

use super::FromGml;
use crate::model::{
    Address, AdeIndex, Attribute, Face, Geometry, GmlObject, Material, MemberTarget, Module,
    Namespace, QName, Shape, Template, Texture, Value, XmlContent, XmlNode,
};
use crate::vocabulary::{cityjson_type, known_ade_schema, module_prefix, relation_to_construction};
use crate::{Error, Result};

/// What the CityJSON conversion needs besides the object.
#[derive(Debug, Clone)]
pub struct FeatureContext {
    /// The transform every vertex is quantised against.
    pub transform: Transform,
    /// The object's 1-based position in the document, which names a
    /// top-level object that has no `gml:id`.
    pub index: usize,
    /// The document's ADEs: which XML attributes are stated once for the
    /// document, and the prefixes of their namespaces.
    pub ade: Arc<AdeIndex>,
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
        let mut out = Out {
            vertices: &mut vertices,
            palettes: &mut appearance,
            feature: &mut feature,
            ade: &context.ade,
        };
        emit(object, &root_id, None, &mut out)?;
        feature.vertices = vertices.into_vertices();
        feature.appearance = appearance.into_appearance();
        Ok(feature)
    }
}

/// A document's relative geometries as CityJSON `geometry-templates`, with
/// the appearance their `material` and `texture` index: what a CityJSON
/// header carries ahead of the features whose GeometryInstances name them.
/// Template vertices keep their local coordinates unquantised. `None` when
/// there are no templates.
pub fn geometry_templates(
    templates: &[Template],
) -> Result<Option<(GeometryTemplates, Option<CjAppearance>)>> {
    if templates.is_empty() {
        return Ok(None);
    }
    let mut vertices = TemplatePool::default();
    let mut palettes = Palettes::default();
    let mut geometries = Vec::with_capacity(templates.len());
    for template in templates {
        if matches!(template.geometry.shape, Shape::Instance(_)) {
            return Err(Error::invalid(
                "a relative geometry cannot itself be an ImplicitGeometry",
            ));
        }
        // A template is converted as an object holding only its geometry
        // and appearance, so it is coloured exactly as an object is.
        let holder = GmlObject {
            id: template.id.clone(),
            class: QName::module(Module::Core, "ImplicitGeometry"),
            attributes: Vec::new(),
            ade: Vec::new(),
            addresses: Vec::new(),
            geometries: vec![template.geometry.clone()],
            boundary_surfaces: Vec::new(),
            appearance: template.appearance.clone(),
            children: Vec::new(),
            members: Vec::new(),
            groups: Vec::new(),
        };
        let mut converted = vec![geometry(&template.geometry, &holder, &mut vertices)?];
        apply_materials(&holder, &mut converted, &mut palettes)?;
        apply_textures(&holder, &mut converted, &mut palettes)?;
        geometries.extend(converted);
    }
    Ok(Some((
        GeometryTemplates {
            templates: geometries,
            vertices_templates: vertices.vertices,
        },
        palettes.into_appearance(),
    )))
}

/// Where [`emit`] puts what it converts.
struct Out<'a> {
    vertices: &'a mut VertexPool,
    palettes: &'a mut Palettes,
    feature: &'a mut CityJSONFeature,
    ade: &'a AdeIndex,
}

/// Add `object` (and, depth first, its children) to the feature as City
/// Objects, linked by `parents` and `children`.
fn emit(object: &GmlObject, my_id: &str, parent_id: Option<&str>, out: &mut Out<'_>) -> Result<()> {
    let mut geometries = object
        .geometries
        .iter()
        .map(|g| geometry(g, object, out.vertices))
        .collect::<Result<Vec<_>>>()?;
    apply_materials(object, &mut geometries, out.palettes)?;
    apply_textures(object, &mut geometries, out.palettes)?;

    let child_ids: Vec<String> = object
        .children
        .iter()
        .enumerate()
        .map(|(i, c)| c.id.clone().unwrap_or_else(|| format!("{my_id}_part_{i}")))
        .collect();

    let type_name = match &object.class.ns {
        Namespace::Other(_) => format!("+{}", object.class.local),
        _ => cityjson_type(&object.class)
            .ok_or_else(|| {
                Error::invalid(format!(
                    "CityGML class {} has no CityJSON City Object type",
                    object.class.local
                ))
            })?
            .to_string(),
    };
    let thetype = CityObjectType::try_from(type_name).map_err(build_error)?;
    let mut co = CityObject::new(thetype);
    co.geometry = Some(geometries);
    let mut attrs = attributes(&object.attributes);
    if let Some(relation) = relation_to_construction(&object.class) {
        attrs
            .entry("relationToConstruction")
            .or_insert_with(|| Json::String(relation.to_string()));
    }
    ade_attributes(object, out.ade, &mut attrs)?;
    if !attrs.is_empty() {
        co.attributes = Some(attrs);
    }
    if !object.addresses.is_empty() {
        co.address = Some(
            object
                .addresses
                .iter()
                .map(|a| address(a, out.vertices))
                .collect::<Result<_>>()?,
        );
    }
    // An object's structural parent comes first, so a group referencing it
    // never decides its feature.
    let parents: Vec<String> = parent_id
        .map(str::to_string)
        .into_iter()
        .chain(object.groups.iter().cloned())
        .collect();
    if !parents.is_empty() {
        co.parents = Some(parents);
    }
    let (children, roles) = children_and_roles(object, &child_ids);
    if !children.is_empty() {
        co.children = Some(children);
        co.children_roles = roles;
    }
    if out.feature.city_objects.contains_key(my_id) {
        return Err(Error::invalid(format!(
            "duplicate CityObject id {my_id:?} within one feature"
        )));
    }
    out.feature.add_co(my_id.to_string(), co);

    for (child, id) in object.children.iter().zip(&child_ids) {
        emit(child, id, Some(my_id), out)?;
    }
    Ok(())
}

/// Nesting deeper than this inside one ADE property is refused rather than
/// converted, since the conversion recurses.
const MAX_ADE_DEPTH: usize = 128;

/// Add `object`'s ADE properties to `attrs`: a CityGML object's as `+`
/// attributes, an ADE object's own (in its class's namespace) without the
/// `+`, and another ADE's on an ADE object under its prefix. A name two
/// ADEs, or an ADE and CityGML, both give the object is an error, since
/// CityJSON could not tell them apart.
fn ade_attributes(object: &GmlObject, ade: &AdeIndex, attrs: &mut Map<String, Json>) -> Result<()> {
    let own = match &object.class.ns {
        Namespace::Other(uri) => Some(uri),
        _ => None,
    };
    // The namespace each key added here came from.
    let mut added: HashMap<String, &Namespace> = HashMap::new();
    for node in &object.ade {
        let key = match own {
            None => format!("+{}", node.name.local),
            Some(own) if node.name.ns == Namespace::Other(own.clone()) => node.name.local.clone(),
            Some(_) => format!("+{}", node.name.local),
        };
        let Some(value) = xml_json(node, ade, 0)? else {
            continue;
        };
        match added.get(&key) {
            Some(ns) if **ns == node.name.ns => {}
            Some(_) => {
                return Err(Error::invalid(format!(
                    "two ADEs give {} the property {key:?}, which CityJSON cannot tell apart",
                    object.id.as_deref().unwrap_or(&object.class.local)
                )));
            }
            None if attrs.contains_key(&key) => {
                return Err(Error::invalid(format!(
                    "the ADE property {key:?} of {} has the name of one of its CityGML \
                     attributes",
                    object.id.as_deref().unwrap_or(&object.class.local)
                )));
            }
            None => {
                added.insert(key.clone(), &node.name.ns);
            }
        }
        accumulate(attrs, key, value);
    }
    Ok(())
}

/// A name with the prefix its namespace is bound to, or in Clark notation
/// (`{uri}local`) when the index knows no prefix for it (a document read
/// without one).
fn qualified(name: &QName, ade: &AdeIndex) -> String {
    let prefix = match &name.ns {
        Namespace::Other(uri) => ade.prefix(uri),
        Namespace::Module(module) => Some(module_prefix(*module)),
        Namespace::Gml => Some("gml"),
    };
    match (prefix, &name.ns) {
        (Some(prefix), _) => format!("{prefix}:{}", name.local),
        (None, Namespace::Other(uri)) => format!("{{{uri}}}{}", name.local),
        (None, _) => name.local.clone(),
    }
}

/// Insert `value` under `key`, a repeated key accumulating into an array.
fn accumulate(map: &mut Map<String, Json>, key: String, value: Json) {
    match map.get_mut(&key) {
        None => {
            map.insert(key, value);
        }
        Some(Json::Array(existing)) => existing.push(value),
        Some(slot) => {
            let prev = slot.take();
            *slot = Json::Array(vec![prev, value]);
        }
    }
}

/// An ADE element as JSON (see the module documentation), or `None` when
/// nothing is left of it: GML, or an element holding only GML.
fn xml_json(node: &XmlNode, ade: &AdeIndex, depth: usize) -> Result<Option<Json>> {
    if depth > MAX_ADE_DEPTH {
        return Err(Error::invalid(format!(
            "ADE property nested deeper than {MAX_ADE_DEPTH}"
        )));
    }
    if node.name.ns == Namespace::Gml {
        return Ok(None);
    }
    let constant = match &node.name.ns {
        Namespace::Other(uri) => ade.constant_attributes(uri, &node.name.local),
        _ => None,
    };
    let mut map = Map::new();
    for attribute in &node.attributes {
        if constant.and_then(|c| c.get(&attribute.name)) != Some(&attribute.value) {
            map.insert(
                format!("@{}", attribute.name),
                Json::String(attribute.value.clone()),
            );
        }
    }
    let value = match &node.content {
        // Present but empty: an object, so the element is not lost.
        XmlContent::Empty => return Ok(Some(Json::Object(map))),
        XmlContent::Text(text) => match node.attribute("uom") {
            Some(_) => text
                .trim()
                .parse::<f64>()
                .ok()
                .filter(|v| v.is_finite())
                .map_or_else(|| Json::String(text.clone()), Json::from),
            None => Json::String(text.clone()),
        },
        XmlContent::Children(children) => {
            let mut kept = false;
            for child in children {
                let Some(value) = xml_json(child, ade, depth + 1)? else {
                    continue;
                };
                kept = true;
                let key = if child.name.ns == node.name.ns {
                    child.name.local.clone()
                } else {
                    qualified(&child.name, ade)
                };
                accumulate(&mut map, key, value);
            }
            if !kept && map.is_empty() {
                return Ok(None);
            }
            return Ok(Some(Json::Object(map)));
        }
    };
    if map.is_empty() {
        return Ok(Some(value));
    }
    map.insert("$".to_string(), value);
    Ok(Some(Json::Object(map)))
}

/// The CityJSON `extensions` member for a document's ADEs, one per ADE in
/// the order the document first uses it, keyed by the prefix the document
/// binds to it. Besides `url` and `version`, each carries:
/// - `xmlns`, the ADE's namespace URI;
/// - `xmlAttributes`, the XML attributes every occurrence of an element
///   carries with one value, by element and attribute name;
/// - `extraCityObjects`, `extraSemanticSurfaces` and `extraAttributes`, as a
///   CityJSON Extension document has them, naming the `+` City Object types,
///   semantic surface types and attributes (by City Object type) the ADE
///   adds, each with an empty schema.
///
/// `url` is the schema `xsi:schemaLocation` names when it is absolute, a
/// known ADE's published schema ([`known_ade_schema`]) when it is not, and
/// else the location as written, or the namespace URI. `version` is the
/// namespace URI's last path segment when that is a version number, else
/// empty.
pub fn extensions(ade: &AdeIndex) -> Vec<(String, Extension)> {
    ade.ades()
        .iter()
        .map(|a| {
            let absolute = a
                .schema_location
                .clone()
                .filter(|l| l.starts_with("http://") || l.starts_with("https://"));
            let url = absolute
                .or_else(|| known_ade_schema(&a.namespace))
                .or_else(|| a.schema_location.clone())
                .unwrap_or_else(|| a.namespace.clone());
            let version = a
                .namespace
                .rsplit('/')
                .next()
                .filter(|v| !v.is_empty() && v.chars().all(|c| c.is_ascii_digit() || c == '.'))
                .unwrap_or_default()
                .to_string();
            let mut other = Map::new();
            other.insert("xmlns".into(), Json::String(a.namespace.clone()));
            if !a.constant_attributes.is_empty() {
                other.insert(
                    "xmlAttributes".into(),
                    serde_json::to_value(&a.constant_attributes).unwrap_or_default(),
                );
            }
            let names = |set: &std::collections::BTreeSet<String>| {
                Json::Object(
                    set.iter()
                        .map(|n| (format!("+{n}"), Json::Object(Map::new())))
                        .collect(),
                )
            };
            if !a.classes.is_empty() {
                other.insert("extraCityObjects".into(), names(&a.classes));
            }
            if !a.surfaces.is_empty() {
                other.insert("extraSemanticSurfaces".into(), names(&a.surfaces));
            }
            if !a.properties.is_empty() {
                let by_type: BTreeMap<&String, Json> = a
                    .properties
                    .iter()
                    .map(|(ty, set)| (ty, names(set)))
                    .collect();
                other.insert(
                    "extraAttributes".into(),
                    serde_json::to_value(by_type).unwrap_or_default(),
                );
            }
            (
                a.prefix.clone(),
                Extension {
                    url,
                    version,
                    other,
                },
            )
        })
        .collect()
}

/// An object's `children`: its nested children, then the members a group
/// references by id, in document order; with `children_roles` when any
/// member has a role.
fn children_and_roles(
    object: &GmlObject,
    child_ids: &[String],
) -> (Vec<String>, Option<Vec<Option<String>>>) {
    let mut children = child_ids.to_vec();
    let mut roles = vec![None; child_ids.len()];
    for member in &object.members {
        match &member.target {
            MemberTarget::Child(i) => roles[*i] = member.role.clone(),
            // A member names its group by `gml:id`, so an id-less group's
            // referenced members cannot name it back: they are left out.
            MemberTarget::Id(id) if object.id.is_some() => {
                children.push(id.clone());
                roles.push(member.role.clone());
            }
            MemberTarget::Id(_) => {}
        }
    }
    let roles = roles.iter().any(Option::is_some).then_some(roles);
    (children, roles)
}

fn build_error(e: impl std::fmt::Display) -> Error {
    Error::invalid(format!("failed to build CityObject from CityGML: {e}"))
}

/// Attributes as a JSON object, a repeated name accumulating into an array
/// (CityGML allows repeated `bldg:function` and repeated generic names).
fn attributes(attributes: &[Attribute]) -> Map<String, Json> {
    let mut map = Map::new();
    for a in attributes {
        accumulate(&mut map, a.name.local().to_string(), json(&a.value));
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
fn geometry(g: &Geometry, object: &GmlObject, vertices: &mut impl Vertices) -> Result<CjGeometry> {
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
        Shape::Instance(instance) => CjGeometry::GeometryInstance {
            boundaries: [vertices.push(instance.reference_point)?],
            template: TemplateIndex(instance.template),
            transformation_matrix: instance.matrix,
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
                thetype: SemanticSurfaceType::try_from(match &surface.class.ns {
                    Namespace::Other(_) => format!("+{}", surface.class.local),
                    _ => surface.class.local.clone(),
                })
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

    /// The palettes as a CityJSON `appearance`, or `None` when they hold no
    /// material and no texture.
    fn into_appearance(self) -> Option<CjAppearance> {
        if self.materials.is_empty() && self.textures.is_empty() {
            return None;
        }
        Some(CjAppearance {
            materials: (!self.materials.is_empty()).then_some(self.materials),
            textures: (!self.textures.is_empty()).then_some(self.textures),
            vertices_texture: (!self.uvs.is_empty()).then_some(self.uvs),
            default_theme_texture: None,
            default_theme_material: None,
        })
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

/// Where a geometry's coordinates go: a feature's quantised vertex pool, or
/// the unquantised template vertices.
trait Vertices {
    fn push(&mut self, coord: [f64; 3]) -> Result<VertexIndex>;

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
}

/// The templates' vertex pool: local coordinates as they are, each distinct
/// vertex (by its exact bits) stored once.
#[derive(Default)]
struct TemplatePool {
    index: HashMap<[u64; 3], usize>,
    vertices: Vec<[f64; 3]>,
}

impl Vertices for TemplatePool {
    fn push(&mut self, coord: [f64; 3]) -> Result<VertexIndex> {
        if let Some(c) = coord.iter().find(|c| !c.is_finite()) {
            return Err(Error::invalid(format!(
                "non-finite CityGML coordinate component: {c}"
            )));
        }
        let next = self.vertices.len();
        Ok(VertexIndex(
            *self
                .index
                .entry(coord.map(f64::to_bits))
                .or_insert_with(|| {
                    self.vertices.push(coord);
                    next
                }),
        ))
    }
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

    fn into_vertices(self) -> Vec<[i64; 3]> {
        self.vertices
    }
}

impl Vertices for VertexPool {
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
