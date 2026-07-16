//! Parse one `bldg:Building` subtree and turn it into a `cjseq::CityJSONFeature`.
//!
//! Two passes over the buffered building:
//! 1. Collect the solid geometry as an unresolved [`SolidGeom`] (its surface
//!    members may `xlink` forward), every inline `gml:Polygon` by `gml:id`
//!    (from `boundedBy` semantic surfaces and any standalone
//!    `lodNMultiSurface`), the semantic-surface list, and each boundary
//!    polygon's semantic index.
//! 2. Resolve the solid's xlinks against the polygon registry and emit exactly
//!    one CityJSON geometry (`Solid`/`CompositeSolid`) with its `semantics`.
//!
//! Scope so far: LoD1/LoD2 solids with semantic surfaces referenced by xlink;
//! the boundedBy-only case (a Building with no `lodNSolid`, geometry living in
//! its `boundedBy` semantic surfaces' `lodN` geometry, e.g. the Railway
//! chapel) emitted as a `MultiSurface` with semantics, including nested
//! `opening` Door/Window surfaces; plus building-level typed `bldg:` and
//! `gen:` generic attributes (repeats accumulate into arrays). BuildingParts
//! and `BuildingInstallation` child geometry remain future work.
//!
//! boundedBy geometry is read from inline `gml:Polygon`s AND from
//! `gml:surfaceMember xlink:href` references (CG-1): an xlinked boundary member
//! tags any solid face carrying that `gml:id` (via `semantic_of_polygon`) and,
//! for a LoD with no solid, is resolved against the polygon registry into the
//! MultiSurface geometry. An xlink to an id defined in no accessible geometry
//! contributes no face (its tag is simply never consulted).

use std::collections::HashMap;
use std::io::BufRead;

use cityparquet_schema::{CityParquetError, Result};
use cjseq::{Appearance, CityJSONFeature, CityObject};
use quick_xml::events::Event;
use quick_xml::reader::NsReader;
use serde_json::{Value, json};

use super::appearance::ReadAppearance;
use super::attributes;
use super::geometry::{self, Polygon, RawSolid, RefTarget, SolidGeom, SurfaceRef};
use super::vertices::VertexBuilder;
use super::xml::{
    NS_APP, NS_BLDG, NS_GEN, NS_GML, get_attr_local, gml_id, ns_is, skip_element, xml_err,
};
use crate::appearance::AppearanceInterner;

/// A feature-local UV vertex pool for textures, deduped by exact bit pattern
/// (`f64::to_bits`) — the same identity `export`'s pool uses.
#[derive(Default)]
struct UvPool {
    uvs: Vec<[f64; 2]>,
    index: HashMap<(u64, u64), usize>,
}

impl UvPool {
    fn intern(&mut self, uv: [f64; 2]) -> usize {
        let key = (uv[0].to_bits(), uv[1].to_bits());
        let next = self.uvs.len();
        *self.index.entry(key).or_insert_with(|| {
            self.uvs.push(uv);
            next
        })
    }
}

/// Feature-local appearance state threaded through an assembly's emission: one
/// interner (materials + textures, deduped by canonical JSON) plus one UV pool.
struct AppearanceState {
    interner: AppearanceInterner,
    uvs: UvPool,
}

/// One solid's `(boundaries, semantic values, face-ids, ring-ids)`, each
/// `[shell]`-nested (see [`RawBuilding::build_solid`]).
type SolidTrees = (Vec<Value>, Vec<Value>, Vec<Value>, Vec<Value>);

/// A buffered building: everything pass 1 collects before resolution.
pub struct RawBuilding {
    pub id: Option<String>,
    /// One solid geometry per distinct LoD (`bldg:lodNSolid`); each LoD maps to
    /// its own CityParquet geometry column, so all are emitted.
    solids: Vec<(String, SolidGeom)>,
    /// Every inline polygon by `gml:id`, resolvable by the solid's xlinks.
    polygons: HashMap<String, Polygon>,
    /// Semantic surface kinds (`"WallSurface"`, ...), in document order.
    surfaces: Vec<String>,
    /// `gml:id` of a boundary polygon -> its index into `surfaces`.
    semantic_of_polygon: HashMap<String, usize>,
    /// Every `boundedBy`/opening polygon in document order, tagged with its
    /// semantic-surface index and lod. Used to emit a MultiSurface geometry per
    /// lod that has no solid (the geometry-in-boundedBy case, e.g. Railway).
    boundary_polys: Vec<(usize, String, Polygon)>,
    /// `boundedBy` members that are `xlink:href` references (not inline
    /// polygons), tagged `(sem_idx, lod, fragment id)`. Resolved against
    /// `polygons` at emit time for the no-solid MultiSurface path; their
    /// `semantic_of_polygon` entry (set at read time) already tags any solid
    /// face carrying that id (CG-1).
    boundary_refs: Vec<(usize, String, String)>,
    /// Building-level attributes (typed `bldg:` + `gen:` generic), by name.
    attributes: serde_json::Map<String, Value>,
    /// Nested `bldg:BuildingPart`s (`consistsOfBuildingPart`), in document order.
    /// A tree: a part may itself have parts.
    parts: Vec<RawBuilding>,
    /// This object's `app:appearance` (X3DMaterials + ParameterizedTextures), in
    /// document order. Applied to faces/rings (by target `gml:id`) during
    /// geometry emission.
    appearance: ReadAppearance,
}

/// Maximum `bldg:consistsOfBuildingPart` nesting depth — a guard against
/// attacker-controlled XML recursion.
const MAX_PART_DEPTH: usize = 32;

/// Read a `bldg:Building` subtree (positioned after its `Start`).
pub fn read_building<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    id: Option<String>,
) -> Result<RawBuilding> {
    read_abstract_building(reader, buf, id, b"Building", 0)
}

/// Read a `_AbstractBuilding` subtree (a `bldg:Building` or `bldg:BuildingPart`,
/// identical content model), breaking on `End(end_name)`.
fn read_abstract_building<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    id: Option<String>,
    end_name: &[u8],
    depth: usize,
) -> Result<RawBuilding> {
    if depth > MAX_PART_DEPTH {
        return Err(CityParquetError::Schema(format!(
            "bldg:consistsOfBuildingPart nested deeper than {MAX_PART_DEPTH}"
        )));
    }
    let mut b = RawBuilding {
        id,
        solids: Vec::new(),
        polygons: HashMap::new(),
        surfaces: Vec::new(),
        semantic_of_polygon: HashMap::new(),
        boundary_polys: Vec::new(),
        boundary_refs: Vec::new(),
        attributes: serde_json::Map::new(),
        parts: Vec::new(),
        appearance: ReadAppearance::default(),
    };

    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        let bldg = ns_is(&rr, NS_BLDG);
        match ev {
            Event::Start(e) => {
                let local = e.local_name();
                let name = local.as_ref().to_vec();
                if bldg && let Some(lod) = lod_suffix(&name, b"Solid") {
                    let geom = read_lod_solid(reader, buf, name)?;
                    // Keep one solid per distinct LoD (the encoder keeps only the
                    // first geometry per object+LoD anyway).
                    if !b.solids.iter().any(|(l, _)| *l == lod) {
                        b.solids.push((lod, geom));
                    }
                } else if bldg && lod_suffix(&name, b"MultiSurface").is_some() {
                    // A standalone lodNMultiSurface under the Building: harvest
                    // its polygons (they may be xlink targets, e.g. an internal
                    // ceiling). Emitting it as primary geometry is M4.
                    for (id, poly) in geometry::collect_polygons(reader, buf)? {
                        if let Some(id) = id {
                            b.polygons.insert(id, poly);
                        }
                    }
                } else if bldg && name == b"boundedBy" {
                    read_bounded_by(reader, buf, &mut b)?;
                } else if bldg && name == b"consistsOfBuildingPart" {
                    read_consists_of_part(reader, buf, &mut b, depth)?;
                } else if ns_is(&rr, NS_APP) && name == b"appearance" {
                    let app = super::appearance::read_appearance(reader, buf)?;
                    b.appearance.materials.extend(app.materials);
                    b.appearance.textures.extend(app.textures);
                } else if bldg && let Some(ty) = attributes::typed_building_attr(&name) {
                    if let Some((k, v)) = attributes::read_typed_attribute(reader, buf, &name, ty)?
                    {
                        attributes::accumulate(&mut b.attributes, k, v);
                    }
                } else if let (true, Some(ty)) =
                    (ns_is(&rr, NS_GEN), attributes::generic_attr(&name))
                {
                    let attr_name = get_attr_local(&e, b"name");
                    if let Some((k, v)) =
                        attributes::read_generic_attribute(reader, buf, attr_name, ty)?
                    {
                        attributes::accumulate(&mut b.attributes, k, v);
                    }
                } else {
                    skip_element(reader, buf)?;
                }
            }
            Event::End(e) if e.local_name().as_ref() == end_name => break,
            Event::Eof => {
                return Err(CityParquetError::Schema(
                    "unexpected end of document inside a bldg:Building/BuildingPart".to_string(),
                ));
            }
            _ => {}
        }
    }
    Ok(b)
}

/// Read a `bldg:consistsOfBuildingPart` PROPERTY: descend to the inner
/// `bldg:BuildingPart`, recurse into it as a child `RawBuilding`, and consume
/// the property's `End`. An empty or `xlink:href`-only property (which
/// `expand_empty_elements` delivers as `Start`+`End` with no child) yields no
/// part.
fn read_consists_of_part<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    b: &mut RawBuilding,
    depth: usize,
) -> Result<()> {
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        match ev {
            Event::Start(e) => {
                if ns_is(&rr, NS_BLDG) && e.local_name().as_ref() == b"BuildingPart" {
                    let id = gml_id(&e);
                    let part = read_abstract_building(reader, buf, id, b"BuildingPart", depth + 1)?;
                    b.parts.push(part);
                } else {
                    skip_element(reader, buf)?;
                }
            }
            Event::End(e) if e.local_name().as_ref() == b"consistsOfBuildingPart" => break,
            Event::Eof => {
                return Err(CityParquetError::Schema(
                    "unexpected end of document inside <bldg:consistsOfBuildingPart>".to_string(),
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

/// `bldg:lodN<suffix>` -> the CityJSON lod string `"N"`, else `None`.
fn lod_suffix(local: &[u8], suffix: &[u8]) -> Option<String> {
    let s = std::str::from_utf8(local).ok()?;
    let suffix = std::str::from_utf8(suffix).ok()?;
    let digits = s.strip_prefix("lod")?.strip_suffix(suffix)?;
    if !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) {
        Some(digits.to_string())
    } else {
        None
    }
}

/// Inside a `bldg:lodNSolid`: read the wrapped `gml:Solid` or
/// `gml:CompositeSolid`.
fn read_lod_solid<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    end: Vec<u8>,
) -> Result<SolidGeom> {
    let mut geom: Option<SolidGeom> = None;
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        let gml = ns_is(&rr, NS_GML);
        match ev {
            Event::Start(e) => {
                let local = e.local_name();
                match (gml, local.as_ref()) {
                    (true, b"Solid") => {
                        geom = Some(SolidGeom::Solid(geometry::read_solid(reader, buf)?))
                    }
                    (true, b"CompositeSolid") => {
                        geom = Some(SolidGeom::Composite(geometry::read_composite_solid(
                            reader, buf,
                        )?))
                    }
                    _ => skip_element(reader, buf)?,
                }
            }
            Event::End(e) if e.local_name().as_ref() == end.as_slice() => break,
            Event::Eof => {
                return Err(CityParquetError::Schema(
                    "unexpected end of document inside <bldg:lodNSolid>".to_string(),
                ));
            }
            _ => {}
        }
    }
    geom.ok_or_else(|| {
        CityParquetError::Schema(
            "bldg:lodNSolid without gml:Solid or gml:CompositeSolid".to_string(),
        )
    })
}

/// CityGML semantic surface element names, in the CityJSON surface-type
/// spelling. A nested one (e.g. a `Door`/`Window` inside a `WallSurface`'s
/// `bldg:opening`) becomes its own semantic surface.
const SEMANTIC_SURFACES: &[&[u8]] = &[
    b"WallSurface",
    b"RoofSurface",
    b"GroundSurface",
    b"ClosureSurface",
    b"OuterCeilingSurface",
    b"OuterFloorSurface",
    b"CeilingSurface",
    b"FloorSurface",
    b"InteriorWallSurface",
    b"Door",
    b"Window",
];

fn is_semantic_surface(local: &[u8]) -> bool {
    SEMANTIC_SURFACES.contains(&local)
}

/// A semantic surface's geometry container is `bldg:lodN{MultiSurface,Geometry,
/// Surface}`; return its LoD digits. `MultiSurface` is tried first so its
/// trailing `Surface` is not mis-stripped.
fn boundary_lod(local: &[u8]) -> Option<String> {
    lod_suffix(local, b"MultiSurface")
        .or_else(|| lod_suffix(local, b"Geometry"))
        .or_else(|| lod_suffix(local, b"Surface"))
}

/// A `bldg:boundedBy`: each semantic surface child (`WallSurface`, ...)
/// contributes one entry to `surfaces` and its polygons are read from its
/// `lodN` geometry (registered for xlink resolution AND recorded in
/// `boundary_polys` for the no-solid MultiSurface path).
fn read_bounded_by<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    b: &mut RawBuilding,
) -> Result<()> {
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        let bldg = ns_is(&rr, NS_BLDG);
        match ev {
            Event::Start(e) => {
                let local = e.local_name();
                if bldg {
                    // The semantic surface: its element name is the CityJSON
                    // surface type.
                    let name = local.as_ref().to_vec();
                    let idx = b.surfaces.len();
                    b.surfaces.push(String::from_utf8_lossy(&name).into_owned());
                    read_semantic_surface(reader, buf, b, idx, &name)?;
                } else {
                    skip_element(reader, buf)?;
                }
            }
            Event::End(e) if e.local_name().as_ref() == b"boundedBy" => break,
            Event::Eof => {
                return Err(CityParquetError::Schema(
                    "unexpected end of document inside <bldg:boundedBy>".to_string(),
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

/// Read one semantic surface's subtree (already pushed at `sem_idx`; ends at
/// `end_name`). Polygons under its `lodN` geometry are registered (for xlink)
/// and recorded in `boundary_polys` tagged with `(sem_idx, lod)`. A nested
/// `bldg:opening` is descended transparently, so a `Door`/`Window` inside it
/// becomes its own semantic surface.
fn read_semantic_surface<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    b: &mut RawBuilding,
    sem_idx: usize,
    end_name: &[u8],
) -> Result<()> {
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        let bldg = ns_is(&rr, NS_BLDG);
        match ev {
            Event::Start(e) => {
                let local = e.local_name();
                let name = local.as_ref().to_vec();
                if bldg && let Some(lod) = boundary_lod(&name) {
                    let (polys, xlinks) = geometry::collect_polygons_with_xlinks(reader, buf)?;
                    for (id, poly) in polys {
                        if let Some(id) = &id {
                            b.semantic_of_polygon.insert(id.clone(), sem_idx);
                            b.polygons.insert(id.clone(), poly.clone());
                        }
                        b.boundary_polys.push((sem_idx, lod.clone(), poly));
                    }
                    for href in xlinks {
                        // Tag any solid face carrying this id, and keep the ref
                        // for the no-solid MultiSurface path (resolved at emit).
                        b.semantic_of_polygon.insert(href.clone(), sem_idx);
                        b.boundary_refs.push((sem_idx, lod.clone(), href));
                    }
                } else if bldg && is_semantic_surface(&name) {
                    // A nested semantic surface (e.g. a Door/Window directly
                    // present): its own entry, resolved recursively.
                    let idx = b.surfaces.len();
                    b.surfaces.push(String::from_utf8_lossy(&name).into_owned());
                    read_semantic_surface(reader, buf, b, idx, &name)?;
                } else if bldg && name == b"opening" {
                    // Transparent wrapper: its Door/Window child is a nested
                    // semantic surface, handled by the branch above on recursion.
                    read_semantic_surface(reader, buf, b, sem_idx, &name)?;
                } else {
                    skip_element(reader, buf)?;
                }
            }
            Event::End(e) if e.local_name().as_ref() == end_name => break,
            Event::Eof => {
                return Err(CityParquetError::Schema(
                    "unexpected end of document inside a boundedBy semantic surface".to_string(),
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

impl RawBuilding {
    /// Resolve xlinks and emit a `CityJSONFeature` with its own local vertex
    /// pool, quantised against `scale`/`translate`. `index` names the building
    /// when it has no `gml:id`.
    pub fn into_feature(
        self,
        scale: &[f64; 3],
        translate: &[f64; 3],
        index: usize,
    ) -> Result<CityJSONFeature> {
        let mut vb = VertexBuilder::new(scale, translate);
        // Feature-local appearance: an assembly (a Building and its parts) shares
        // ONE appearance block, so the parent and every part intern into the same
        // material/texture tables + UV pool, feature-wide.
        let mut app = AppearanceState {
            interner: AppearanceInterner::new(),
            uvs: UvPool::default(),
        };
        let mut feature = CityJSONFeature::new();
        let root_id = self
            .id
            .clone()
            .unwrap_or_else(|| format!("Building_{index}"));
        feature.id = root_id.clone();
        self.emit_into(&root_id, None, &mut vb, &mut app, &mut feature)?;
        feature.vertices = vb.into_vertices();
        let materials = app.interner.materials();
        let textures = app.interner.textures();
        if !materials.is_empty() || !textures.is_empty() {
            feature.appearance = Some(Appearance {
                materials: (!materials.is_empty()).then(|| materials.to_vec()),
                textures: (!textures.is_empty()).then(|| textures.to_vec()),
                vertices_texture: (!app.uvs.uvs.is_empty())
                    .then(|| app.uvs.uvs.iter().map(|uv| uv.to_vec()).collect()),
                default_theme_texture: None,
                default_theme_material: None,
            });
        }
        Ok(feature)
    }

    /// The id a part at position `i` uses: its own `gml:id`, else a deterministic
    /// synthesis from the parent id (`<parent>_part_<i>`, an NCName).
    fn part_id(&self, parent_id: &str, i: usize) -> String {
        self.id
            .clone()
            .unwrap_or_else(|| format!("{parent_id}_part_{i}"))
    }

    /// Build this object's CityJSON geometry array (solids + boundedBy-only
    /// MultiSurfaces), resolving against this `RawBuilding`'s own registries and
    /// stamping each geometry's `material` map from this object's appearance.
    fn build_geometries(
        &self,
        vb: &mut VertexBuilder,
        app: &mut AppearanceState,
    ) -> Result<Vec<Value>> {
        // Each entry: (geometry JSON, face-id tree, ring-id tree). The face-id
        // tree addresses `app:target`s (materials, at face level); the ring-id
        // tree addresses `app:textureCoordinates ring`s (textures, at ring level).
        let mut built: Vec<(Value, Value, Value)> = Vec::new();
        for (lod, geom) in &self.solids {
            built.push(self.build_solid_geometry(geom, lod, vb)?);
        }
        // For each LoD whose geometry lives only in `boundedBy` surfaces (no
        // `lodNSolid` at that LoD), emit one MultiSurface. When a solid exists
        // at that LoD the boundary polygons are already its xlink targets, so
        // they must NOT leak as a second geometry.
        let mut ms_lods: Vec<String> = Vec::new();
        let boundary_lods = self
            .boundary_polys
            .iter()
            .map(|(_, lod, _)| lod)
            .chain(self.boundary_refs.iter().map(|(_, lod, _)| lod));
        for lod in boundary_lods {
            let has_solid = self.solids.iter().any(|(l, _)| l == lod);
            if !has_solid && !ms_lods.contains(lod) {
                ms_lods.push(lod.clone());
            }
        }
        for lod in &ms_lods {
            let ms = self.build_multisurface_geometry(lod, vb)?;
            // A LoD whose boundedBy members were all unresolved xlinks would
            // otherwise emit a faceless MultiSurface — skip it.
            let empty = ms.0["boundaries"].as_array().is_some_and(|b| b.is_empty());
            if !empty {
                built.push(ms);
            }
        }
        self.apply_materials(&mut built, &mut app.interner);
        self.apply_textures(&mut built, app);
        Ok(built.into_iter().map(|(g, _, _)| g).collect())
    }

    /// Intern this object's `app:X3DMaterial`s and stamp each geometry's
    /// `material` map. A material is interned even when target-less (so unused
    /// definitions survive at the feature level); a theme that colours no face
    /// of a geometry is omitted for that geometry. Two materials targeting the
    /// same face+theme are last-wins (document order).
    fn apply_materials(
        &self,
        built: &mut [(Value, Value, Value)],
        interner: &mut AppearanceInterner,
    ) {
        if self.appearance.materials.is_empty() {
            return;
        }
        let mut themes: Vec<String> = Vec::new();
        let mut theme_maps: HashMap<String, HashMap<String, usize>> = HashMap::new();
        for rm in &self.appearance.materials {
            let idx = interner.intern_material(&rm.material);
            if !theme_maps.contains_key(&rm.theme) {
                themes.push(rm.theme.clone());
            }
            let map = theme_maps.entry(rm.theme.clone()).or_default();
            for target in &rm.targets {
                map.insert(target.clone(), idx); // last-wins
            }
        }
        for (geom, face_ids, _) in built.iter_mut() {
            let mut material_obj = serde_json::Map::new();
            for theme in &themes {
                let map = &theme_maps[theme];
                let (values, any) = material_values_from_face_ids(face_ids, map);
                if any {
                    material_obj.insert(theme.clone(), json!({ "values": values }));
                }
            }
            if !material_obj.is_empty()
                && let Some(obj) = geom.as_object_mut()
            {
                obj.insert("material".to_string(), Value::Object(material_obj));
            }
        }
    }

    /// Intern this object's `app:ParameterizedTexture`s (defs + UVs) and stamp
    /// each geometry's `texture` map. A ring not textured in a theme is `[null]`;
    /// a theme that textures no ring of a geometry is omitted. Two textures on the
    /// same ring+theme are last-wins (document order).
    ///
    /// Known limitation: a face reached through a REVERSED `gml:OrientableSurface`
    /// has its ring vertices wound backwards ([`surface_rings`] with `reverse`),
    /// but the ring's UVs are stored in document order — so on such a face UVs
    /// misalign with the reversed vertices. Our own writer never emits
    /// `OrientableSurface`, so the paired round-trip is unaffected; this is a
    /// foreign-file loss (like TexCoordGen / seams).
    fn apply_textures(&self, built: &mut [(Value, Value, Value)], app: &mut AppearanceState) {
        if self.appearance.textures.is_empty() {
            return;
        }
        // theme -> (ring gml:id -> (texture index, [uv indices]))
        let mut themes: Vec<String> = Vec::new();
        let mut theme_maps: HashMap<String, HashMap<String, (usize, Vec<usize>)>> = HashMap::new();
        for rt in &self.appearance.textures {
            let tex_idx = app.interner.intern_texture(&rt.texture);
            if !theme_maps.contains_key(&rt.theme) {
                themes.push(rt.theme.clone());
            }
            let map = theme_maps.entry(rt.theme.clone()).or_default();
            for (ring_id, uvs) in &rt.rings {
                let uv_idxs: Vec<usize> = uvs.iter().map(|&uv| app.uvs.intern(uv)).collect();
                map.insert(ring_id.clone(), (tex_idx, uv_idxs)); // last-wins
            }
        }
        for (geom, _, ring_ids) in built.iter_mut() {
            let mut texture_obj = serde_json::Map::new();
            for theme in &themes {
                let map = &theme_maps[theme];
                let (values, any) = texture_values_from_ring_ids(ring_ids, map);
                if any {
                    texture_obj.insert(theme.clone(), json!({ "values": values }));
                }
            }
            if !texture_obj.is_empty()
                && let Some(obj) = geom.as_object_mut()
            {
                obj.insert("texture".to_string(), Value::Object(texture_obj));
            }
        }
    }

    /// Emit this object (a `Building` when `parent_id` is `None`, else a
    /// `BuildingPart`) and, depth-first, each of its parts as sibling
    /// CityObjects of the same feature (one shared vertex pool). `parents`/
    /// `children` link the tree.
    fn emit_into(
        &self,
        my_id: &str,
        parent_id: Option<&str>,
        vb: &mut VertexBuilder,
        app: &mut AppearanceState,
        feature: &mut CityJSONFeature,
    ) -> Result<()> {
        let geoms_json = self.build_geometries(vb, app)?;
        let child_ids: Vec<String> = self
            .parts
            .iter()
            .enumerate()
            .map(|(i, p)| p.part_id(my_id, i))
            .collect();

        let mut co_obj = serde_json::Map::new();
        co_obj.insert(
            "type".to_string(),
            json!(if parent_id.is_none() {
                "Building"
            } else {
                "BuildingPart"
            }),
        );
        co_obj.insert("geometry".to_string(), Value::Array(geoms_json));
        if !self.attributes.is_empty() {
            co_obj.insert(
                "attributes".to_string(),
                Value::Object(self.attributes.clone()),
            );
        }
        if let Some(p) = parent_id {
            co_obj.insert("parents".to_string(), json!([p]));
        }
        if !child_ids.is_empty() {
            co_obj.insert("children".to_string(), json!(child_ids));
        }
        let co: CityObject = serde_json::from_value(Value::Object(co_obj)).map_err(|e| {
            CityParquetError::Schema(format!("failed to build CityObject from CityGML: {e}"))
        })?;

        if feature.city_objects.contains_key(my_id) {
            return Err(CityParquetError::Schema(format!(
                "duplicate CityObject id {my_id:?} within a Building assembly"
            )));
        }
        feature.add_co(my_id.to_string(), co);

        for (i, part) in self.parts.iter().enumerate() {
            part.emit_into(&child_ids[i], Some(my_id), vb, app, feature)?;
        }
        Ok(())
    }

    /// Build one CityJSON `Solid`/`CompositeSolid` geometry object, emitting
    /// boundaries and matching `semantics.values` in a single walk (so their
    /// per-face order is aligned by construction).
    fn build_solid_geometry(
        &self,
        geom: &SolidGeom,
        lod: &str,
        vb: &mut VertexBuilder,
    ) -> Result<(Value, Value, Value)> {
        let (gtype, boundaries, values, face_ids, ring_ids) = match geom {
            SolidGeom::Solid(solid) => {
                let (b, v, id, r) = self.build_solid(solid, vb)?;
                (
                    "Solid",
                    Value::Array(b),
                    Value::Array(v),
                    Value::Array(id),
                    Value::Array(r),
                )
            }
            SolidGeom::Composite(solids) => {
                let mut sb = Vec::with_capacity(solids.len());
                let mut sv = Vec::with_capacity(solids.len());
                let mut sid = Vec::with_capacity(solids.len());
                let mut sring = Vec::with_capacity(solids.len());
                for solid in solids {
                    let (b, v, id, r) = self.build_solid(solid, vb)?;
                    sb.push(Value::Array(b));
                    sv.push(Value::Array(v));
                    sid.push(Value::Array(id));
                    sring.push(Value::Array(r));
                }
                (
                    "CompositeSolid",
                    Value::Array(sb),
                    Value::Array(sv),
                    Value::Array(sid),
                    Value::Array(sring),
                )
            }
        };

        let mut g = json!({ "type": gtype, "lod": lod, "boundaries": boundaries });
        if !self.surfaces.is_empty() {
            let surfaces: Vec<Value> = self.surfaces.iter().map(|k| json!({ "type": k })).collect();
            g["semantics"] = json!({ "surfaces": surfaces, "values": values });
        }
        Ok((g, face_ids, ring_ids))
    }

    /// Build one CityJSON `MultiSurface` geometry for `lod` from the
    /// `boundedBy` polygons at that LoD: `boundaries` is `[surface][ring][idx]`
    /// and `semantics.values` is one entry per surface (the semantic index of
    /// the boundary polygon's surface). Used only when there is no `lodNSolid`
    /// at `lod`.
    fn build_multisurface_geometry(
        &self,
        lod: &str,
        vb: &mut VertexBuilder,
    ) -> Result<(Value, Value, Value)> {
        let mut boundaries = Vec::new();
        let mut values = Vec::new();
        let mut face_ids = Vec::new();
        let mut ring_ids = Vec::new();
        // Face ids already emitted at this LoD, so a polygon referenced more
        // than once (e.g. two boundedBy surfaces xlinking the same id) is not
        // emitted as a duplicate face.
        let mut emitted: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for (sem_idx, poly_lod, poly) in &self.boundary_polys {
            if poly_lod != lod {
                continue;
            }
            if let Some(id) = poly.id.as_deref()
                && !emitted.insert(id)
            {
                continue;
            }
            boundaries.push(surface_rings(poly, false, vb)?);
            values.push(json!(sem_idx));
            face_ids.push(poly.id.clone().map(Value::from).unwrap_or(Value::Null));
            ring_ids.push(ring_ids_value(poly));
        }
        // Resolve xlinked boundary members against the polygon registry (CG-1).
        for (sem_idx, ref_lod, href) in &self.boundary_refs {
            if ref_lod != lod {
                continue;
            }
            if !emitted.insert(href) {
                continue; // already emitted (duplicate reference)
            }
            let Some(poly) = self.polygons.get(href) else {
                // A boundedBy xlink to an id defined in no accessible geometry:
                // contribute no face, but do not stay silent.
                eprintln!(
                    "warning: bldg:boundedBy references #{href}, which resolves to no polygon in this building; skipping"
                );
                continue;
            };
            boundaries.push(surface_rings(poly, false, vb)?);
            values.push(json!(sem_idx));
            face_ids.push(poly.id.clone().map(Value::from).unwrap_or(Value::Null));
            ring_ids.push(ring_ids_value(poly));
        }
        let mut g = json!({
            "type": "MultiSurface",
            "lod": lod,
            "boundaries": Value::Array(boundaries),
        });
        if !self.surfaces.is_empty() {
            let surfaces: Vec<Value> = self.surfaces.iter().map(|k| json!({ "type": k })).collect();
            g["semantics"] = json!({ "surfaces": surfaces, "values": Value::Array(values) });
        }
        Ok((g, Value::Array(face_ids), Value::Array(ring_ids)))
    }

    /// A `RawSolid` -> (`[shell][surface][ring][idx]` boundaries, `[shell][face]`
    /// semantic values, `[shell][face]` face-ids, `[shell][face][ring]` ring-ids).
    /// The face-id leaf is the polygon's `gml:id` (or `null`); the ring-id leaf
    /// each ring's `gml:id` (or `null`) — the addresses `app:target` /
    /// `app:textureCoordinates ring` resolve against.
    fn build_solid(&self, solid: &RawSolid, vb: &mut VertexBuilder) -> Result<SolidTrees> {
        let mut shells_b = Vec::with_capacity(solid.shells.len());
        let mut shells_v = Vec::with_capacity(solid.shells.len());
        let mut shells_id = Vec::with_capacity(solid.shells.len());
        let mut shells_ring = Vec::with_capacity(solid.shells.len());
        for shell in &solid.shells {
            let mut faces_b = Vec::with_capacity(shell.len());
            let mut faces_v = Vec::with_capacity(shell.len());
            let mut faces_id = Vec::with_capacity(shell.len());
            let mut faces_ring = Vec::with_capacity(shell.len());
            for sref in shell {
                let poly = self.resolve(sref)?;
                faces_b.push(surface_rings(poly, sref.reverse, vb)?);
                faces_v.push(self.semantic_value(sref));
                faces_id.push(face_id(sref, poly));
                faces_ring.push(ring_ids_value(poly));
            }
            shells_b.push(Value::Array(faces_b));
            shells_v.push(Value::Array(faces_v));
            shells_id.push(Value::Array(faces_id));
            shells_ring.push(Value::Array(faces_ring));
        }
        Ok((shells_b, shells_v, shells_id, shells_ring))
    }

    /// The polygon a surface reference points at (xlink resolved against the
    /// registry, or inline).
    fn resolve<'a>(&'a self, sref: &'a SurfaceRef) -> Result<&'a Polygon> {
        match &sref.target {
            RefTarget::Inline(poly) => Ok(poly),
            RefTarget::Xlink(id) => self.polygons.get(id).ok_or_else(|| {
                CityParquetError::Schema(format!(
                    "CityGML solid references #{id}, which is not defined in this building \
                     (cross-building/shared geometry is out of scope)"
                ))
            }),
        }
    }

    /// The `semantics.values` leaf for a surface reference: the semantic index
    /// of the referenced boundary polygon, or `null` (inline / no boundary
    /// surface, e.g. an internal ceiling).
    fn semantic_value(&self, sref: &SurfaceRef) -> Value {
        // The face's own `gml:id`: the xlink target, or an inline polygon's own
        // id. Either may be tagged in `semantic_of_polygon` — an inline solid
        // face is tagged when a boundedBy surface xlinks to it (CG-1).
        let id = match &sref.target {
            RefTarget::Xlink(id) => Some(id.as_str()),
            RefTarget::Inline(poly) => poly.id.as_deref(),
        };
        match id.and_then(|id| self.semantic_of_polygon.get(id)) {
            Some(&i) => json!(i),
            None => Value::Null,
        }
    }
}

/// A face's polygon `gml:id` as a face-id-tree leaf: the xlink target id, or the
/// inline polygon's own id, or `null` when an inline polygon is anonymous (and
/// hence untargetable by appearance).
fn face_id(sref: &SurfaceRef, poly: &Polygon) -> Value {
    match &sref.target {
        RefTarget::Xlink(id) => Value::from(id.clone()),
        RefTarget::Inline(_) => poly.id.clone().map(Value::from).unwrap_or(Value::Null),
    }
}

/// A face's ring `gml:id`s as a ring-id-tree leaf: `[ring0_id, ring1_id, ...]`,
/// each a `Value::String` (or `null` for an anonymous ring), in
/// exterior-then-interior order (matching `surface_rings`).
fn ring_ids_value(poly: &Polygon) -> Value {
    Value::Array(
        poly.ring_ids
            .iter()
            .map(|id| id.clone().map(Value::from).unwrap_or(Value::Null))
            .collect(),
    )
}

/// Turn a ring-id tree into a `texture.{theme}.values` tree: each ring leaf id is
/// mapped to `[texture index, uv indices…]` (a hit) or `[null]` (a miss/anonymous
/// ring). Returns the tree and whether any ring resolved (to omit an untextured
/// theme). The ring-id tree is one level deeper than the face-id tree, so a
/// `String`/`Null` node is a RING (producing the `[tex, uv…]` array), and an
/// `Array` is a container to recurse into.
fn texture_values_from_ring_ids(
    node: &Value,
    map: &HashMap<String, (usize, Vec<usize>)>,
) -> (Value, bool) {
    match node {
        Value::String(id) => match map.get(id) {
            Some((tex, uvs)) => {
                let mut leaf = Vec::with_capacity(1 + uvs.len());
                leaf.push(json!(tex));
                leaf.extend(uvs.iter().map(|u| json!(u)));
                (Value::Array(leaf), true)
            }
            None => (json!([Value::Null]), false),
        },
        Value::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            let mut any = false;
            for it in items {
                let (v, hit) = texture_values_from_ring_ids(it, map);
                any |= hit;
                out.push(v);
            }
            (Value::Array(out), any)
        }
        _ => (json!([Value::Null]), false),
    }
}

/// Turn a face-id tree into a `material.{theme}.values` tree by mapping each
/// leaf id through `map` (a hit → its material index, a miss/`null` → `null`).
/// Returns the tree and whether any leaf resolved (used to omit a theme that
/// colours no face of the geometry).
fn material_values_from_face_ids(node: &Value, map: &HashMap<String, usize>) -> (Value, bool) {
    match node {
        Value::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            let mut any = false;
            for it in items {
                let (v, hit) = material_values_from_face_ids(it, map);
                any |= hit;
                out.push(v);
            }
            (Value::Array(out), any)
        }
        Value::String(id) => match map.get(id) {
            Some(&i) => (json!(i), true),
            None => (Value::Null, false),
        },
        _ => (Value::Null, false),
    }
}

/// A surface's rings `[exterior, hole...]`, each a list of vertex indices; when
/// `reverse`, every ring is wound backwards so the outward normal flips.
fn surface_rings(poly: &Polygon, reverse: bool, vb: &mut VertexBuilder) -> Result<Value> {
    let mut rings = Vec::with_capacity(1 + poly.interiors.len());
    rings.push(ring_indices(&poly.exterior, reverse, vb)?);
    for hole in &poly.interiors {
        rings.push(ring_indices(hole, reverse, vb)?);
    }
    Ok(Value::Array(rings))
}

fn ring_indices(ring: &[[f64; 3]], reverse: bool, vb: &mut VertexBuilder) -> Result<Value> {
    let mut idxs = Vec::with_capacity(ring.len());
    if reverse {
        for &coord in ring.iter().rev() {
            idxs.push(json!(vb.push(coord)?));
        }
    } else {
        for &coord in ring {
            idxs.push(json!(vb.push(coord)?));
        }
    }
    Ok(Value::Array(idxs))
}
