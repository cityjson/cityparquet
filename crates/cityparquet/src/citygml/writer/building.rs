//! `bldg:Building` serialisation with LoD-major mapping and `gml:id` validation.
//!
//! CityGML 2.0 only has `bldg:lod1Solid`..`bldg:lod4Solid`, each 0..1 per
//! building. A CityParquet package can carry several geometry LoD columns that
//! share a major (e.g. `lod2` and `lod2_2` both map to major 2), so this picks
//! ONE per major — the most detailed (highest minor) — and counts the rest as
//! skipped. Only WKB `PolyhedralSurface` (a CityJSON `Solid`) is emitted in
//! W-M1; `GeometryCollection` (MultiSolid/CompositeSolid) and other shapes are
//! the driver's concern to count, but a stray non-Solid is guarded here too.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Write;

use cityparquet_schema::{AttributeType, CityParquetError, Lod};
use quick_xml::Writer;
use quick_xml::events::{BytesEnd, BytesStart, Event};
use serde_json::Value;

use super::WriteReport;
use super::appearance::{
    AppearanceAcc, TextureAcc, TextureFaceMaps, count_faces, face_ring_counts, material_face_maps,
    material_union, texture_face_maps, texture_ring_needs, write_appearance,
};
use super::attributes::write_attributes;
use super::document::Bounds;
use super::geometry::{FaceIds, write_composite_solid_with_ids, write_solid_with_ids};
use super::semantics::{
    IdAlloc, Semantics, droppable_surface_count, has_nonnull_value, parse_semantics,
    surfaces_emittable, write_multisurface_with_semantics, write_solid_with_semantics,
};
use crate::Result;
use crate::wkb_read::{DecodedGeometry, DecodedKind};

/// The semantics a geometry can actually emit as `bldg:boundedBy`, or `None`.
/// Requires: a major in `2..=4` (CityGML 2.0 `_BoundarySurface` has no
/// `lod1MultiSurface`), an emittable geometry kind, every surface type a legal
/// NCName, and at least one non-null value (a real semantic face — not a
/// building-wide surfaces array stamped with all-null values on every geometry).
fn emittable_semantics(major: u8, kind: &DecodedKind, props: Option<&Value>) -> Option<Semantics> {
    if !(2..=4).contains(&major) {
        return None;
    }
    if !matches!(
        kind,
        DecodedKind::PolyhedralSurface(_)
            | DecodedKind::GeometryCollection(_)
            | DecodedKind::MultiPolygon(_)
    ) {
        return None;
    }
    let sem = parse_semantics(props)?;
    (surfaces_emittable(&sem) && has_nonnull_value(&sem.values)).then_some(sem)
}

/// Emit a `bldg:lod<major>Solid` with plain (geometry-only) inline geometry —
/// the W-M2 form, used when a geometry has no emittable semantics (or is not the
/// building's chosen semantic LoD). `face_ids` stamps a `gml:id` on each
/// material-bearing face's inline polygon (so appearance can target it). Any real
/// (non-null) semantics not emitted here are counted as dropped.
fn write_plain_lodn_solid<W: Write>(
    w: &mut Writer<W>,
    geom: &DecodedGeometry,
    props: Option<&Value>,
    major: u8,
    report: &mut WriteReport,
    face_ids: Option<&[FaceIds]>,
) -> Result<()> {
    let elem = format!("bldg:lod{major}Solid");
    w.write_event(Event::Start(BytesStart::new(elem.as_str())))?;
    match &geom.kind {
        DecodedKind::PolyhedralSurface(faces) => {
            write_solid_with_ids(w, &geom.coords, faces, props, face_ids)?
        }
        DecodedKind::GeometryCollection(members) => {
            write_composite_solid_with_ids(w, &geom.coords, members, props, face_ids)?;
            report.composite_solids_written += 1;
        }
        _ => unreachable!("only PolyhedralSurface/GeometryCollection reach the plain solid path"),
    }
    w.write_event(Event::End(BytesEnd::new(elem.as_str())))?;
    report.semantic_surfaces_dropped += droppable_surface_count(props);
    Ok(())
}

/// Pre-allocate polygon + ring `gml:id`s for each appearance-bearing face (in
/// face-walk order) — a polygon id when the face has a material OR any textured
/// ring, and a `<polyid>_r<K>` id per textured ring. Used by the plain path and
/// the semantic fallback so `app:target`/`app:textureCoordinates ring` can
/// reference the inline polygon.
fn plain_face_ids(mat_union: &[bool], ring_needs: &[Vec<bool>], ids: &mut IdAlloc) -> Vec<FaceIds> {
    let n = mat_union.len().max(ring_needs.len());
    (0..n)
        .map(|i| {
            let mat = mat_union.get(i).copied().unwrap_or(false);
            let rings = ring_needs.get(i).map(Vec::as_slice).unwrap_or(&[]);
            let needs_face = mat || rings.iter().any(|&b| b);
            let poly = needs_face.then(|| ids.alloc());
            let ring_ids = rings
                .iter()
                .enumerate()
                .map(|(r, &need)| need.then(|| format!("{}_r{r}", poly.as_deref().unwrap())))
                .collect();
            FaceIds {
                poly,
                rings: ring_ids,
            }
        })
        .collect()
}

/// A valid XML `NCName` (ASCII-pragmatic, matching real CityGML ids): first
/// char a letter or `_`, the rest letters/digits/`.`/`-`/`_`, never a `:`.
pub fn is_ncname(id: &str) -> bool {
    let mut chars = id.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

/// One Building's `id` plus its per-LoD candidate solids, as gathered by the
/// driver. Each solid is `(lod, decoded geometry, its geometry_properties)`.
pub struct BuildingSolids {
    pub id: String,
    /// Building-level attributes (already decoded from the package's typed
    /// columns), emitted before geometry.
    pub attributes: serde_json::Map<String, Value>,
    /// `(lod, decoded geometry, geometry_properties, material map, texture map)`.
    /// The material/texture maps are this geometry's `{theme: {values|value}}`
    /// (already keyed out of the row's per-LoD `material`/`texture` columns), with
    /// global ids into the package's materials/textures tables; `None` when the
    /// geometry carries that kind of appearance not at all.
    pub solids: Vec<SolidEntry>,
}

/// One candidate geometry of a Building: its LoD, decoded WKB, properties, and
/// (Compatibility profile) its material/texture maps.
pub type SolidEntry = (
    Lod,
    DecodedGeometry,
    Option<Value>,
    Option<Value>,
    Option<Value>,
);

/// Emit `<cityObjectMember><bldg:Building gml:id="…">` with one
/// `bldg:lod<major>Solid` per major LoD (highest minor wins), accumulating
/// every emitted coordinate into `bounds`. Returns `Ok(false)` (nothing
/// written) when no solid survives — the caller counts that building in
/// `buildings_without_solid_skipped`. Errors when `id` is not a valid NCName.
#[allow(clippy::too_many_arguments)]
pub fn write_building<W: Write>(
    w: &mut Writer<W>,
    b: &BuildingSolids,
    types: &HashMap<String, AttributeType>,
    feature_index: usize,
    bounds: &mut Bounds,
    report: &mut WriteReport,
    materials: Option<&[Value]>,
    textures: Option<&[Value]>,
) -> Result<bool> {
    if !is_ncname(&b.id) {
        return Err(CityParquetError::Schema(format!(
            "CityObject id {:?} is not a valid XML NCName; cannot serialise as gml:id",
            b.id
        )));
    }
    // Render the content into a buffer so the emptiness decision (no geometry AND
    // no writable attribute) can suppress the whole element.
    let mut content = Writer::new(Vec::new());
    if !write_object_content(
        &mut content,
        b,
        types,
        feature_index,
        bounds,
        report,
        materials,
        textures,
    )? {
        return Ok(false);
    }
    w.write_event(Event::Start(BytesStart::new("cityObjectMember")))?;
    let mut bldg = BytesStart::new("bldg:Building");
    bldg.push_attribute(("gml:id", b.id.as_str()));
    w.write_event(Event::Start(bldg))?;
    w.get_mut().write_all(&content.into_inner())?;
    w.write_event(Event::End(BytesEnd::new("bldg:Building")))?;
    w.write_event(Event::End(BytesEnd::new("cityObjectMember")))?;
    Ok(true)
}

/// Emit the `_AbstractBuilding` content — attributes then geometry, the inside of
/// a `<bldg:Building>`/`<bldg:BuildingPart>` WITHOUT the wrapper. Returns whether
/// anything (a writable attribute or a geometry) was emitted; the caller
/// suppresses the wrapper when nothing was.
#[allow(clippy::too_many_arguments)]
pub fn write_object_content<W: Write>(
    w: &mut Writer<W>,
    b: &BuildingSolids,
    types: &HashMap<String, AttributeType>,
    feature_index: usize,
    bounds: &mut Bounds,
    report: &mut WriteReport,
    materials: Option<&[Value]>,
    textures: Option<&[Value]>,
) -> Result<bool> {
    // Keep at most one solid per major LoD (1..=4), the highest minor.
    // BTreeMap keeps the majors in ascending order for emission.
    type MajorGeom<'a> = (
        Lod,
        &'a DecodedGeometry,
        Option<&'a Value>,
        Option<&'a Value>,
        Option<&'a Value>,
    );
    let mut by_major: BTreeMap<u8, MajorGeom> = BTreeMap::new();
    for (lod, geom, props, material, texture) in &b.solids {
        // A Solid (PolyhedralSurface) or a CompositeSolid (GeometryCollection)
        // becomes a lodNSolid; a MultiSurface (MultiPolygon) is representable
        // only when it carries emittable semantics (W-M3 boundedBy). Any other
        // shape is skipped.
        let representable = match &geom.kind {
            DecodedKind::PolyhedralSurface(_) | DecodedKind::GeometryCollection(_) => true,
            // A MultiSurface has no geometry-only form, so it is representable
            // only as an emittable semantic surface set.
            DecodedKind::MultiPolygon(_) => {
                emittable_semantics(lod.major(), &geom.kind, props.as_ref()).is_some()
            }
            _ => false,
        };
        if !representable {
            report.lod_columns_skipped += 1;
            report.semantic_surfaces_dropped += droppable_surface_count(props.as_ref());
            continue;
        }
        let major = lod.major();
        if !(1..=4).contains(&major) {
            report.lod_columns_skipped += 1;
            continue;
        }
        let entry = (
            *lod,
            geom,
            props.as_ref(),
            material.as_ref(),
            texture.as_ref(),
        );
        match by_major.get(&major) {
            None => {
                by_major.insert(major, entry);
            }
            Some((existing, _, _, _, _)) => {
                // One of the two collides away; keep the higher minor.
                if *lod > *existing {
                    by_major.insert(major, entry);
                }
                report.lod_columns_skipped += 1;
            }
        }
    }

    // Reject non-finite coordinates before emitting anything: `inf`/`-inf`/`NaN`
    // are not valid XML Schema `double` lexical forms and NaN would poison the
    // envelope. (WKB's 2^53 magnitude guard does not catch NaN, whose
    // comparisons are always false.) No-op for an attributes-only building.
    for (_, geom, _, _, _) in by_major.values() {
        if geom.coords.iter().any(|c| c.iter().any(|v| !v.is_finite())) {
            return Err(CityParquetError::Geometry(format!(
                "building {:?} has a non-finite coordinate; cannot serialise as gml:posList",
                b.id
            )));
        }
    }

    // Attributes precede geometry in the CityGML _CityObject / Building sequence.
    let attrs_written = write_attributes(w, &b.attributes, types, report)?;

    // A building's semantics can round-trip for at most ONE LoD: the reader
    // builds a single building-wide `surfaces` array applied to every geometry,
    // so per-LoD boundedBy blocks would duplicate/offset it. Emit semantics for
    // the highest emittable-semantic major; every other LoD is geometry-only.
    let chosen_major = by_major
        .iter()
        .filter(|(major, (_, geom, props, _, _))| {
            emittable_semantics(**major, &geom.kind, *props).is_some()
        })
        .map(|(major, _)| *major)
        .max();

    let mut any_geometry = false;
    let mut mat_acc = AppearanceAcc::new();
    let mut tex_acc = TextureAcc::new();
    for (major, (_, geom, props, material, texture)) in &by_major {
        let major = *major;
        let props = *props;
        // Resolve this geometry's material/texture maps to flat per-face/-ring
        // appearance; a resolution failure (or a Core-profile package with no
        // table) drops that appearance for this geometry but keeps the geometry.
        let n_faces = count_faces(&geom.kind);
        let (maps, mat_union) = resolve_materials(*material, materials, &geom.kind, report);
        let tex_maps = resolve_textures(*texture, textures, &geom.kind, report);
        let ring_needs = texture_ring_needs(&tex_maps, n_faces);
        let mut ids = IdAlloc::new(feature_index, major);
        let mut emitted_geometry = true;
        let face_ids: Vec<FaceIds> = if Some(major) == chosen_major {
            // The one LoD that emits bldg:boundedBy semantic surfaces.
            let sem = emittable_semantics(major, &geom.kind, props)
                .expect("chosen major has emittable semantics");
            match &geom.kind {
                DecodedKind::PolyhedralSurface(_) | DecodedKind::GeometryCollection(_) => {
                    match write_solid_with_semantics(
                        w,
                        &geom.coords,
                        &geom.kind,
                        props,
                        &sem,
                        &mut ids,
                        major,
                        &mat_union,
                        &ring_needs,
                    ) {
                        Ok(face_ids) => {
                            report.semantic_surfaces_written += sem.surfaces.len();
                            if matches!(geom.kind, DecodedKind::GeometryCollection(_)) {
                                report.composite_solids_written += 1;
                            }
                            face_ids
                        }
                        // Resolution failed (corrupt/external values): fall back
                        // to plain geometry, counting the surfaces as dropped.
                        Err(_) => {
                            let fids = plain_face_ids(&mat_union, &ring_needs, &mut ids);
                            write_plain_lodn_solid(w, geom, props, major, report, Some(&fids))?;
                            fids
                        }
                    }
                }
                DecodedKind::MultiPolygon(faces) => {
                    match write_multisurface_with_semantics(
                        w,
                        &geom.coords,
                        faces,
                        &sem,
                        major,
                        report,
                        &mut ids,
                        &mat_union,
                        &ring_needs,
                    ) {
                        Ok(face_ids) => {
                            report.semantic_surfaces_written += sem.surfaces.len();
                            face_ids
                        }
                        // A MultiSurface has no geometry-only form; drop it.
                        Err(_) => {
                            report.semantic_surfaces_dropped += sem.surfaces.len();
                            emitted_geometry = false;
                            Vec::new()
                        }
                    }
                }
                _ => unreachable!("the representable gate excludes other kinds"),
            }
        } else {
            // Not the chosen semantic LoD: emit geometry only (solids), or drop a
            // MultiSurface (which has no geometry-only form). Real semantics are
            // counted as dropped (by write_plain_lodn_solid or here).
            match &geom.kind {
                DecodedKind::PolyhedralSurface(_) | DecodedKind::GeometryCollection(_) => {
                    let fids = plain_face_ids(&mat_union, &ring_needs, &mut ids);
                    write_plain_lodn_solid(w, geom, props, major, report, Some(&fids))?;
                    fids
                }
                DecodedKind::MultiPolygon(_) => {
                    report.semantic_surfaces_dropped += droppable_surface_count(props);
                    emitted_geometry = false;
                    Vec::new()
                }
                _ => unreachable!("the representable gate excludes other kinds"),
            }
        };
        // Accumulate the coord pool + appearance of geometry we actually emitted
        // (a dropped MultiSurface contributes nothing).
        if emitted_geometry {
            any_geometry = true;
            for c in &geom.coords {
                bounds.add(*c);
            }
            let poly_ids: Vec<Option<String>> = face_ids.iter().map(|f| f.poly.clone()).collect();
            mat_acc.add(&maps, &poly_ids);
            tex_acc.add(&tex_maps, &face_ids);
        }
    }

    // Feature-local appearance: one app:appearance/app:Appearance per theme, after
    // the geometry (order-independent for the round-trip reader).
    if !mat_acc.is_empty() || !tex_acc.is_empty() {
        write_appearance(
            w,
            &mat_acc,
            &tex_acc,
            materials.unwrap_or(&[]),
            textures.unwrap_or(&[]),
            report,
        )?;
    }

    // Emitted iff a writable attribute or an ACTUALLY-written geometry — a
    // by_major entry that dropped (e.g. a MultiSurface resolution failure) must
    // not make an otherwise-empty object non-empty (no husk elements).
    Ok(attrs_written > 0 || any_geometry)
}

/// Resolve one geometry's stored `material` map to per-theme flat global ids and
/// the per-face union. A missing map → empty. A present map with no materials
/// table (Core profile) or an unresolvable map → empty + a report counter, so
/// the geometry still emits without appearance.
fn resolve_materials(
    material: Option<&Value>,
    materials: Option<&[Value]>,
    kind: &DecodedKind,
    report: &mut WriteReport,
) -> (
    std::collections::BTreeMap<String, Vec<Option<usize>>>,
    Vec<bool>,
) {
    let empty = std::collections::BTreeMap::new();
    let Some(material) = material else {
        return (empty, Vec::new());
    };
    let Some(table) = materials else {
        report.appearance_skipped_core_profile += 1;
        return (empty, Vec::new());
    };
    let n_faces = count_faces(kind);
    match material_face_maps(material, n_faces, table.len()) {
        Ok(maps) => {
            let union = material_union(&maps, n_faces);
            (maps, union)
        }
        Err(_) => {
            report.material_geometries_dropped += 1;
            (empty, Vec::new())
        }
    }
}

/// Resolve one geometry's stored `texture` map to per-theme `[face][ring]`
/// textures. A missing map → empty. A present map with no textures table (Core
/// profile) or an unresolvable map → empty + a counter, so the geometry still
/// emits without texture appearance.
fn resolve_textures(
    texture: Option<&Value>,
    textures: Option<&[Value]>,
    kind: &DecodedKind,
    report: &mut WriteReport,
) -> TextureFaceMaps {
    let Some(texture) = texture else {
        return TextureFaceMaps::new();
    };
    let Some(table) = textures else {
        report.appearance_skipped_core_profile += 1;
        return TextureFaceMaps::new();
    };
    match texture_face_maps(texture, table.len()) {
        // The `[face][ring]` tree must match the geometry's shape exactly — else
        // ring ids would be allocated for phantom rings and `app:target`s would
        // dangle. A mismatch drops this geometry's textures with a counter.
        Ok(maps) if texture_shape_matches(&maps, kind) => maps,
        Ok(_) | Err(_) => {
            report.texture_geometries_dropped += 1;
            TextureFaceMaps::new()
        }
    }
}

/// Whether every theme's flattened `[face][ring]` texture shape matches the
/// geometry's face count and per-face ring counts.
fn texture_shape_matches(maps: &TextureFaceMaps, kind: &DecodedKind) -> bool {
    let want = face_ring_counts(kind);
    maps.values().all(|faces| {
        faces.len() == want.len() && faces.iter().zip(&want).all(|(rings, &n)| rings.len() == n)
    })
}

/// Maximum `consistsOfBuildingPart` nesting the writer emits — symmetric with the
/// reader's bound, so a written document is always re-readable (and a deep
/// acyclic chain cannot overflow the recursion).
const MAX_PART_DEPTH: usize = 32;

/// The read-only object graph a Building assembly is rendered from: every
/// object's content, its `children` id list, its CityObject `type`, and the
/// attribute type map.
pub struct BuildingTree<'a> {
    pub content_by_id: &'a HashMap<String, BuildingSolids>,
    pub children_by_id: &'a HashMap<String, Vec<String>>,
    pub type_by_id: &'a HashMap<String, String>,
    pub types: &'a HashMap<String, AttributeType>,
    /// The package's global materials table (`materials.parquet`), or `None` on a
    /// Core-profile package with no appearance definitions.
    pub materials: Option<&'a [Value]>,
    /// The package's global textures table (`textures.parquet`), or `None`.
    pub textures: Option<&'a [Value]>,
}

/// Render an object's INNER bytes (attributes + geometry, then each non-empty
/// child part wrapped in `consistsOfBuildingPart/bldg:BuildingPart`, in the
/// stored `children` order — `consistsOfBuildingPart` comes last per the CityGML
/// 2.0 sequence). Returns `(inner, non_empty)`; `non_empty` is false when the
/// object has no geometry, no writable attribute, and no rendered part. The
/// caller wraps a non-empty root in `cityObjectMember/bldg:Building`. `visited`
/// guards cycles; `seen_ids` enforces document-unique `gml:id`s on emit.
#[allow(clippy::too_many_arguments)]
pub fn render_abstract_object(
    obj_id: &str,
    tree: &BuildingTree,
    depth: usize,
    next_feature_index: &mut usize,
    bounds: &mut Bounds,
    report: &mut WriteReport,
    visited: &mut HashSet<String>,
    seen_ids: &mut HashSet<String>,
    reached_parts: &mut HashSet<String>,
) -> Result<(Vec<u8>, bool)> {
    if depth > MAX_PART_DEPTH {
        return Err(CityParquetError::Schema(format!(
            "consistsOfBuildingPart nested deeper than {MAX_PART_DEPTH}"
        )));
    }
    if !is_ncname(obj_id) {
        return Err(CityParquetError::Schema(format!(
            "CityObject id {obj_id:?} is not a valid XML NCName; cannot serialise as gml:id"
        )));
    }
    // A cycle in the stored parents/children: this object is its own ancestor.
    if !visited.insert(obj_id.to_string()) {
        return Ok((Vec::new(), false));
    }

    let Some(obj) = tree.content_by_id.get(obj_id) else {
        visited.remove(obj_id);
        return Ok((Vec::new(), false));
    };
    let feature_index = *next_feature_index;
    *next_feature_index += 1;

    let mut inner = Writer::new(Vec::new());
    let self_emitted = write_object_content(
        &mut inner,
        obj,
        tree.types,
        feature_index,
        bounds,
        report,
        tree.materials,
        tree.textures,
    )?;

    let children = tree
        .children_by_id
        .get(obj_id)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let mut any_part = false;
    for child_id in children {
        // Only a BuildingPart row is nested; a `children` entry naming an absent
        // row or a non-BuildingPart (e.g. a Building) is an unresolved child.
        if tree.type_by_id.get(child_id).map(String::as_str) != Some("BuildingPart") {
            report.children_unresolved += 1;
            continue;
        }
        reached_parts.insert(child_id.clone());
        let (child_inner, child_nonempty) = render_abstract_object(
            child_id,
            tree,
            depth + 1,
            next_feature_index,
            bounds,
            report,
            visited,
            seen_ids,
            reached_parts,
        )?;
        if child_nonempty {
            inner.write_event(Event::Start(BytesStart::new("bldg:consistsOfBuildingPart")))?;
            let mut bp = BytesStart::new("bldg:BuildingPart");
            bp.push_attribute(("gml:id", child_id.as_str()));
            inner.write_event(Event::Start(bp))?;
            inner.get_mut().write_all(&child_inner)?;
            inner.write_event(Event::End(BytesEnd::new("bldg:BuildingPart")))?;
            inner.write_event(Event::End(BytesEnd::new("bldg:consistsOfBuildingPart")))?;
            report.building_parts_written += 1;
            any_part = true;
        } else {
            report.building_parts_skipped += 1;
        }
    }

    visited.remove(obj_id);
    let non_empty = self_emitted || any_part;
    if non_empty && !seen_ids.insert(obj_id.to_string()) {
        return Err(CityParquetError::Schema(format!(
            "duplicate CityObject id {obj_id:?}; CityGML gml:id must be document-unique"
        )));
    }
    Ok((inner.into_inner(), non_empty))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tri_solid() -> DecodedGeometry {
        DecodedGeometry {
            coords: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0]],
            kind: DecodedKind::PolyhedralSurface(vec![vec![vec![0, 1, 2]]]),
        }
    }

    /// The `geometry_properties` a stored `Solid` always carries — `write_solid`
    /// requires `type: "Solid"`.
    fn solid_props() -> Value {
        serde_json::json!({ "type": "Solid" })
    }

    fn composite_props() -> Value {
        serde_json::json!({ "type": "CompositeSolid", "shells": [[1]] })
    }

    fn composite_geom() -> DecodedGeometry {
        DecodedGeometry {
            coords: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0]],
            kind: DecodedKind::GeometryCollection(vec![DecodedKind::PolyhedralSurface(vec![
                vec![vec![0usize, 1, 2]],
            ])]),
        }
    }

    /// A type map covering the attribute names these tests use (`roofType`); an
    /// unused entry is harmless for the geometry-only tests that share it.
    fn types() -> HashMap<String, AttributeType> {
        [("roofType".to_string(), AttributeType::String)]
            .into_iter()
            .collect()
    }

    fn run_building(b: &BuildingSolids) -> (String, WriteReport) {
        let mut report = WriteReport::default();
        let mut w = Writer::new(Vec::new());
        write_building(
            &mut w,
            b,
            &types(),
            0,
            &mut Bounds::new(),
            &mut report,
            None,
            None,
        )
        .unwrap();
        (String::from_utf8(w.into_inner()).unwrap(), report)
    }

    #[test]
    fn solid_with_semantics_emits_boundedby() {
        let props = serde_json::json!({
            "type": "Solid", "shells": [[1]],
            "surfaces": [{"type": "WallSurface"}], "face_semantics": [0]
        });
        let b = BuildingSolids {
            id: "S1".into(),
            attributes: serde_json::Map::new(),
            solids: vec![(
                Lod::parse("2").unwrap(),
                tri_solid(),
                Some(props),
                None,
                None,
            )],
        };
        let (xml, r) = run_building(&b);
        assert!(xml.contains("<bldg:boundedBy><bldg:WallSurface>"), "{xml}");
        assert!(xml.contains("<bldg:lod2Solid><gml:Solid>"), "{xml}");
        assert_eq!(r.semantic_surfaces_written, 1);
        assert_eq!(r.semantic_surfaces_dropped, 0);
    }

    #[test]
    fn multisurface_with_semantics_emits_boundedby_no_solid() {
        let geom = DecodedGeometry {
            coords: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0]],
            kind: DecodedKind::MultiPolygon(vec![vec![vec![0, 1, 2]]]),
        };
        let props = serde_json::json!({
            "type": "MultiSurface",
            "surfaces": [{"type": "RoofSurface"}], "face_semantics": [0]
        });
        let b = BuildingSolids {
            id: "M1".into(),
            attributes: serde_json::Map::new(),
            solids: vec![(Lod::parse("2").unwrap(), geom, Some(props), None, None)],
        };
        let (xml, r) = run_building(&b);
        assert!(xml.contains("<bldg:boundedBy><bldg:RoofSurface>"), "{xml}");
        assert!(
            !xml.contains("<bldg:lod2Solid>"),
            "no solid for a MultiSurface: {xml}"
        );
        assert_eq!(r.semantic_surfaces_written, 1);
    }

    #[test]
    fn multisurface_without_semantics_is_skipped() {
        let geom = DecodedGeometry {
            coords: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0]],
            kind: DecodedKind::MultiPolygon(vec![vec![vec![0, 1, 2]]]),
        };
        let b = BuildingSolids {
            id: "M2".into(),
            attributes: serde_json::Map::new(),
            solids: vec![(
                Lod::parse("2").unwrap(),
                geom,
                Some(serde_json::json!({"type": "MultiSurface"})),
                None,
                None,
            )],
        };
        let (xml, r) = run_building(&b);
        assert!(
            xml.is_empty(),
            "an unsemantic MultiSurface is not representable: {xml}"
        );
        assert_eq!(r.lod_columns_skipped, 1);
    }

    fn sem_solid_props(value: serde_json::Value) -> Value {
        serde_json::json!({
            "type": "Solid", "shells": [[1]],
            "surfaces": [{"type": "WallSurface"}], "face_semantics": [value]
        })
    }

    #[test]
    fn only_the_highest_semantic_lod_emits_boundedby() {
        // Two semantic LoDs -> only the highest (lod3) emits boundedBy; lod2 is
        // geometry-only and its surface is counted dropped (CityGML shares one
        // building-wide surface set, so per-LoD semantics can't both round-trip).
        let b = BuildingSolids {
            id: "ML".into(),
            attributes: serde_json::Map::new(),
            solids: vec![
                (
                    Lod::parse("2").unwrap(),
                    tri_solid(),
                    Some(sem_solid_props(serde_json::json!(0))),
                    None,
                    None,
                ),
                (
                    Lod::parse("3").unwrap(),
                    tri_solid(),
                    Some(sem_solid_props(serde_json::json!(0))),
                    None,
                    None,
                ),
            ],
        };
        let (xml, r) = run_building(&b);
        assert_eq!(
            xml.matches("<bldg:boundedBy>").count(),
            1,
            "only one LoD emits semantics: {xml}"
        );
        assert!(
            xml.contains("<bldg:lod2Solid>") && xml.contains("<bldg:lod3Solid>"),
            "{xml}"
        );
        assert_eq!(r.semantic_surfaces_written, 1, "lod3's surface");
        assert_eq!(r.semantic_surfaces_dropped, 1, "lod2's surface dropped");
    }

    #[test]
    fn plain_lod1_with_stamped_null_semantics_does_not_over_count() {
        // The common 3DBAG shape after one CityGML cycle: a plain lod1 solid
        // carries the building-wide surfaces stamped all-null, and lod2 has real
        // semantics. lod2 emits; lod1 is geometry-only and counts NOTHING dropped.
        let b = BuildingSolids {
            id: "PL".into(),
            attributes: serde_json::Map::new(),
            solids: vec![
                (
                    Lod::parse("1").unwrap(),
                    tri_solid(),
                    Some(sem_solid_props(serde_json::json!(null))),
                    None,
                    None,
                ),
                (
                    Lod::parse("2").unwrap(),
                    tri_solid(),
                    Some(sem_solid_props(serde_json::json!(0))),
                    None,
                    None,
                ),
            ],
        };
        let (xml, r) = run_building(&b);
        assert_eq!(
            xml.matches("<bldg:boundedBy>").count(),
            1,
            "only lod2 emits: {xml}"
        );
        assert!(
            xml.contains("<bldg:lod1Solid>"),
            "lod1 geometry-only: {xml}"
        );
        assert_eq!(r.semantic_surfaces_written, 1);
        assert_eq!(
            r.semantic_surfaces_dropped, 0,
            "all-null lod1 must not count as dropped"
        );
    }

    #[test]
    fn lod1_only_semantics_are_dropped_no_lod1_multisurface() {
        // CityGML 2.0 has no lod1MultiSurface, so lod1 semantics cannot be
        // emitted: geometry-only + counted dropped.
        let b = BuildingSolids {
            id: "L1".into(),
            attributes: serde_json::Map::new(),
            solids: vec![(
                Lod::parse("1").unwrap(),
                tri_solid(),
                Some(sem_solid_props(serde_json::json!(0))),
                None,
                None,
            )],
        };
        let (xml, r) = run_building(&b);
        assert!(!xml.contains("boundedBy"), "no boundedBy for lod1: {xml}");
        assert!(
            xml.contains("<bldg:lod1Solid>"),
            "lod1 geometry emitted: {xml}"
        );
        assert_eq!(r.semantic_surfaces_written, 0);
        assert_eq!(r.semantic_surfaces_dropped, 1);
    }

    #[test]
    fn semantics_resolution_error_falls_back_to_plain_solid() {
        // 1 face but values claims 2 -> resolution error -> geometry-only fallback.
        let props = serde_json::json!({
            "type": "Solid", "shells": [[1]],
            "surfaces": [{"type": "WallSurface"}], "face_semantics": [0, 1]
        });
        let b = BuildingSolids {
            id: "S2".into(),
            attributes: serde_json::Map::new(),
            solids: vec![(
                Lod::parse("2").unwrap(),
                tri_solid(),
                Some(props),
                None,
                None,
            )],
        };
        let (xml, r) = run_building(&b);
        assert!(xml.contains("<bldg:lod2Solid><gml:Solid>"), "{xml}");
        assert!(
            !xml.contains("<bldg:boundedBy>"),
            "fallback emits no boundedBy: {xml}"
        );
        assert_eq!(r.semantic_surfaces_written, 0);
        assert_eq!(r.semantic_surfaces_dropped, 1);
    }

    #[test]
    fn composite_solid_is_emitted_and_counted() {
        let b = BuildingSolids {
            id: "C1".into(),
            attributes: serde_json::Map::new(),
            solids: vec![(
                Lod::parse("2").unwrap(),
                composite_geom(),
                Some(composite_props()),
                None,
                None,
            )],
        };
        let mut report = WriteReport::default();
        let mut w = Writer::new(Vec::new());
        assert!(
            write_building(
                &mut w,
                &b,
                &types(),
                0,
                &mut Bounds::new(),
                &mut report,
                None,
                None
            )
            .unwrap()
        );
        let xml = String::from_utf8(w.into_inner()).unwrap();
        assert!(
            xml.contains("<bldg:lod2Solid><gml:CompositeSolid>"),
            "{xml}"
        );
        assert_eq!(report.composite_solids_written, 1);
    }

    #[test]
    fn is_ncname_accepts_3dbag_ids_rejects_bad() {
        assert!(is_ncname("NL.IMBAG.Pand.0503100000013175-0"));
        assert!(is_ncname("_x"));
        assert!(!is_ncname("3leadingdigit"));
        assert!(!is_ncname("has:colon"));
        assert!(!is_ncname(""));
    }

    #[test]
    fn major_lod_collision_keeps_highest_minor_and_counts_the_rest() {
        let b = BuildingSolids {
            id: "B1".into(),
            attributes: serde_json::Map::new(),
            solids: vec![
                (
                    Lod::parse("2").unwrap(),
                    tri_solid(),
                    Some(solid_props()),
                    None,
                    None,
                ),
                (
                    Lod::parse("2.2").unwrap(),
                    tri_solid(),
                    Some(solid_props()),
                    None,
                    None,
                ),
            ],
        };
        let mut bounds = Bounds::new();
        let mut report = WriteReport::default();
        let mut w = Writer::new(Vec::new());
        assert!(
            write_building(
                &mut w,
                &b,
                &types(),
                0,
                &mut bounds,
                &mut report,
                None,
                None
            )
            .unwrap()
        );
        let xml = String::from_utf8(w.into_inner()).unwrap();
        assert_eq!(xml.matches("<bldg:lod2Solid>").count(), 1);
        assert_eq!(report.lod_columns_skipped, 1);
        assert!(bounds.any);
    }

    #[test]
    fn multiple_majors_emitted_in_ascending_order() {
        let b = BuildingSolids {
            id: "B2".into(),
            attributes: serde_json::Map::new(),
            solids: vec![
                (
                    Lod::parse("2").unwrap(),
                    tri_solid(),
                    Some(solid_props()),
                    None,
                    None,
                ),
                (
                    Lod::parse("1").unwrap(),
                    tri_solid(),
                    Some(solid_props()),
                    None,
                    None,
                ),
            ],
        };
        let mut w = Writer::new(Vec::new());
        assert!(
            write_building(
                &mut w,
                &b,
                &types(),
                0,
                &mut Bounds::new(),
                &mut WriteReport::default(),
                None,
                None
            )
            .unwrap()
        );
        let xml = String::from_utf8(w.into_inner()).unwrap();
        let lod1 = xml.find("<bldg:lod1Solid>").unwrap();
        let lod2 = xml.find("<bldg:lod2Solid>").unwrap();
        assert!(lod1 < lod2, "lod1Solid must precede lod2Solid");
    }

    #[test]
    fn no_representable_solid_returns_false_and_emits_nothing() {
        // A lod0 solid is not a valid lodNSolid (only 1..4).
        let b = BuildingSolids {
            id: "B3".into(),
            attributes: serde_json::Map::new(),
            solids: vec![(Lod::parse("0").unwrap(), tri_solid(), None, None, None)],
        };
        let mut report = WriteReport::default();
        let mut w = Writer::new(Vec::new());
        assert!(
            !write_building(
                &mut w,
                &b,
                &types(),
                0,
                &mut Bounds::new(),
                &mut report,
                None,
                None
            )
            .unwrap()
        );
        assert!(w.into_inner().is_empty());
        assert_eq!(report.lod_columns_skipped, 1);
    }

    #[test]
    fn non_finite_coordinate_errors() {
        let geom = DecodedGeometry {
            coords: vec![[0.0, 0.0, 0.0], [1.0, f64::NAN, 0.0], [1.0, 1.0, 0.0]],
            kind: DecodedKind::PolyhedralSurface(vec![vec![vec![0, 1, 2]]]),
        };
        let b = BuildingSolids {
            id: "B4".into(),
            attributes: serde_json::Map::new(),
            solids: vec![(Lod::parse("2").unwrap(), geom, None, None, None)],
        };
        let mut w = Writer::new(Vec::new());
        assert!(
            write_building(
                &mut w,
                &b,
                &types(),
                0,
                &mut Bounds::new(),
                &mut WriteReport::default(),
                None,
                None
            )
            .is_err()
        );
        // Nothing should have been emitted before the error.
        assert!(w.into_inner().is_empty());
    }

    fn content(id: &str, solids: Vec<SolidEntry>) -> BuildingSolids {
        BuildingSolids {
            id: id.into(),
            attributes: serde_json::Map::new(),
            solids,
        }
    }

    fn solid_lod2() -> SolidEntry {
        (
            Lod::parse("2").unwrap(),
            tri_solid(),
            Some(solid_props()),
            None,
            None,
        )
    }

    fn render(
        root: &str,
        content_by_id: HashMap<String, BuildingSolids>,
        children_by_id: HashMap<String, Vec<String>>,
    ) -> (String, bool, WriteReport) {
        let no_types = HashMap::new();
        // The root is a Building; every other present object is a BuildingPart.
        let type_by_id: HashMap<String, String> = content_by_id
            .keys()
            .map(|id| {
                let ty = if id == root {
                    "Building"
                } else {
                    "BuildingPart"
                };
                (id.clone(), ty.to_string())
            })
            .collect();
        let tree = BuildingTree {
            content_by_id: &content_by_id,
            children_by_id: &children_by_id,
            type_by_id: &type_by_id,
            types: &no_types,
            materials: None,
            textures: None,
        };
        let mut nfi = 0usize;
        let mut bounds = Bounds::new();
        let mut report = WriteReport::default();
        let mut visited = HashSet::new();
        let mut seen = HashSet::new();
        let mut reached = HashSet::new();
        let (inner, ne) = render_abstract_object(
            root,
            &tree,
            0,
            &mut nfi,
            &mut bounds,
            &mut report,
            &mut visited,
            &mut seen,
            &mut reached,
        )
        .unwrap();
        (String::from_utf8(inner).unwrap(), ne, report)
    }

    #[test]
    fn geometryless_parent_with_part_emits_consists_of_building_part() {
        let content_by_id = HashMap::from([
            ("P".to_string(), content("P", vec![])), // geometry-less parent
            ("P_c".to_string(), content("P_c", vec![solid_lod2()])),
        ]);
        let children_by_id = HashMap::from([("P".to_string(), vec!["P_c".to_string()])]);
        let (xml, ne, r) = render("P", content_by_id, children_by_id);
        assert!(ne, "a geometry-less parent WITH a rendered part emits");
        assert!(
            xml.contains("<bldg:consistsOfBuildingPart><bldg:BuildingPart gml:id=\"P_c\">"),
            "{xml}"
        );
        assert!(xml.contains("<bldg:lod2Solid>"), "{xml}");
        assert_eq!(r.building_parts_written, 1);
    }

    #[test]
    fn parts_emitted_in_stored_children_order() {
        let content_by_id = HashMap::from([
            ("P".to_string(), content("P", vec![])),
            ("c1".to_string(), content("c1", vec![solid_lod2()])),
            ("c2".to_string(), content("c2", vec![solid_lod2()])),
        ]);
        // children order [c2, c1] must emit c2 before c1 (not map/row order).
        let children_by_id =
            HashMap::from([("P".to_string(), vec!["c2".to_string(), "c1".to_string()])]);
        let (xml, _, _) = render("P", content_by_id, children_by_id);
        assert!(
            xml.find("gml:id=\"c2\"").unwrap() < xml.find("gml:id=\"c1\"").unwrap(),
            "parts follow children order: {xml}"
        );
    }

    #[test]
    fn parent_with_only_empty_parts_skips() {
        let content_by_id = HashMap::from([
            ("P".to_string(), content("P", vec![])),
            ("P_c".to_string(), content("P_c", vec![])), // empty part
        ]);
        let children_by_id = HashMap::from([("P".to_string(), vec!["P_c".to_string()])]);
        let (_, ne, r) = render("P", content_by_id, children_by_id);
        assert!(!ne, "a parent with only empty parts collapses");
        assert_eq!(r.building_parts_skipped, 1);
    }

    #[test]
    fn cycle_guard_terminates() {
        // P -> c -> P (a cycle in the stored children); must terminate.
        let content_by_id = HashMap::from([
            ("P".to_string(), content("P", vec![solid_lod2()])),
            ("c".to_string(), content("c", vec![])),
        ]);
        let children_by_id = HashMap::from([
            ("P".to_string(), vec!["c".to_string()]),
            ("c".to_string(), vec!["P".to_string()]),
        ]);
        let (xml, ne, _) = render("P", content_by_id, children_by_id);
        assert!(ne, "P has a solid");
        assert!(xml.contains("<bldg:lod2Solid>"), "{xml}");
    }

    #[test]
    fn multisurface_resolution_failure_yields_no_husk() {
        // A MultiSurface with emittable-looking semantics but a values/faces
        // length mismatch: it fails at emit, so the object has no geometry and
        // (no attributes) must render EMPTY — not an empty husk counted as one.
        let geom = DecodedGeometry {
            coords: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0]],
            kind: DecodedKind::MultiPolygon(vec![vec![vec![0, 1, 2]]]), // 1 face
        };
        let props = serde_json::json!({
            "type": "MultiSurface",
            "surfaces": [{"type": "WallSurface"}], "face_semantics": [0, 1]
        });
        let content_by_id = HashMap::from([(
            "P".to_string(),
            BuildingSolids {
                id: "P".into(),
                attributes: serde_json::Map::new(),
                solids: vec![(Lod::parse("2").unwrap(), geom, Some(props), None, None)],
            },
        )]);
        let (xml, ne, _) = render("P", content_by_id, HashMap::new());
        assert!(
            !ne,
            "a geometry-less-after-failure, attribute-less object renders empty"
        );
        assert!(xml.is_empty(), "{xml}");
    }

    #[test]
    fn unresolved_child_is_counted() {
        let content_by_id = HashMap::from([("P".to_string(), content("P", vec![solid_lod2()]))]);
        let children_by_id = HashMap::from([("P".to_string(), vec!["missing".to_string()])]);
        let (_, _, r) = render("P", content_by_id, children_by_id);
        assert_eq!(r.children_unresolved, 1);
    }

    #[test]
    fn invalid_ncname_id_errors() {
        let b = BuildingSolids {
            id: "3bad".into(),
            attributes: serde_json::Map::new(),
            solids: vec![],
        };
        let mut w = Writer::new(Vec::new());
        assert!(
            write_building(
                &mut w,
                &b,
                &types(),
                0,
                &mut Bounds::new(),
                &mut WriteReport::default(),
                None,
                None
            )
            .is_err()
        );
    }

    #[test]
    fn attributes_only_building_emits_with_no_solid() {
        let mut attributes = serde_json::Map::new();
        attributes.insert("roofType".into(), serde_json::json!("1000"));
        let b = BuildingSolids {
            id: "B5".into(),
            attributes,
            solids: vec![],
        };
        let mut report = WriteReport::default();
        let mut w = Writer::new(Vec::new());
        assert!(
            write_building(
                &mut w,
                &b,
                &types(),
                0,
                &mut Bounds::new(),
                &mut report,
                None,
                None
            )
            .unwrap()
        );
        let xml = String::from_utf8(w.into_inner()).unwrap();
        assert!(xml.contains("<bldg:Building gml:id=\"B5\">"));
        assert!(xml.contains("<bldg:roofType>1000</bldg:roofType>"));
        assert_eq!(report.attributes_written, 1);
    }

    #[test]
    fn empty_building_still_returns_false() {
        let b = BuildingSolids {
            id: "B6".into(),
            attributes: serde_json::Map::new(),
            solids: vec![],
        };
        let mut w = Writer::new(Vec::new());
        assert!(
            !write_building(
                &mut w,
                &b,
                &types(),
                0,
                &mut Bounds::new(),
                &mut WriteReport::default(),
                None,
                None
            )
            .unwrap()
        );
        assert!(w.into_inner().is_empty());
    }

    #[test]
    fn attributes_precede_geometry() {
        let mut attributes = serde_json::Map::new();
        attributes.insert("roofType".into(), serde_json::json!("1000"));
        let b = BuildingSolids {
            id: "B7".into(),
            attributes,
            solids: vec![(
                Lod::parse("2").unwrap(),
                tri_solid(),
                Some(solid_props()),
                None,
                None,
            )],
        };
        let mut w = Writer::new(Vec::new());
        write_building(
            &mut w,
            &b,
            &types(),
            0,
            &mut Bounds::new(),
            &mut WriteReport::default(),
            None,
            None,
        )
        .unwrap();
        let xml = String::from_utf8(w.into_inner()).unwrap();
        assert!(xml.find("<bldg:roofType>").unwrap() < xml.find("<bldg:lod2Solid>").unwrap());
    }

    #[test]
    fn plain_solid_emits_x3d_material_with_targets() {
        // A plain lod2 solid of 3 faces; material.visual.values = [[0,0,1]] -> red
        // (global 0) on faces 0,1 and green (global 1) on face 2.
        let coords: Vec<[f64; 3]> = (0..9).map(|i| [i as f64, 0.0, 0.0]).collect();
        let geom = DecodedGeometry {
            coords,
            kind: DecodedKind::PolyhedralSurface(vec![
                vec![vec![0, 1, 2]],
                vec![vec![3, 4, 5]],
                vec![vec![6, 7, 8]],
            ]),
        };
        let props = serde_json::json!({ "type": "Solid", "shells": [[3]] });
        let material = serde_json::json!({ "visual": { "values": [[0, 0, 1]] } });
        let table = vec![
            serde_json::json!({ "name": "red", "diffuseColor": [1.0, 0.0, 0.0] }),
            serde_json::json!({ "name": "green", "diffuseColor": [0.0, 1.0, 0.0] }),
        ];
        let b = BuildingSolids {
            id: "MAT".into(),
            attributes: serde_json::Map::new(),
            solids: vec![(
                Lod::parse("2").unwrap(),
                geom,
                Some(props),
                Some(material),
                None,
            )],
        };
        let mut report = WriteReport::default();
        let mut w = Writer::new(Vec::new());
        write_building(
            &mut w,
            &b,
            &types(),
            0,
            &mut Bounds::new(),
            &mut report,
            Some(&table),
            None,
        )
        .unwrap();
        let xml = String::from_utf8(w.into_inner()).unwrap();
        // Two X3DMaterials (red used by two faces, green by one), theme "visual".
        assert_eq!(report.materials_written, 2, "{xml}");
        assert_eq!(xml.matches("<app:X3DMaterial>").count(), 2, "{xml}");
        assert!(xml.contains("<app:theme>visual</app:theme>"), "{xml}");
        assert!(
            xml.contains("<gml:name>red</gml:name>")
                && xml.contains("<app:diffuseColor>1 0 0</app:diffuseColor>"),
            "{xml}"
        );
        // Every material-bearing face got a gml:id, referenced by an app:target.
        assert_eq!(xml.matches("<app:target>").count(), 3, "{xml}");
        assert!(
            xml.contains("<gml:Polygon gml:id=\"_cpq_b0_l2_p0\">"),
            "{xml}"
        );
        assert!(
            xml.contains("<app:target>#_cpq_b0_l2_p0</app:target>"),
            "{xml}"
        );
        // Appearance follows the geometry.
        assert!(
            xml.find("<bldg:lod2Solid>").unwrap() < xml.find("<app:appearance>").unwrap(),
            "{xml}"
        );
    }

    #[test]
    fn texture_shape_mismatch_is_dropped_and_counted() {
        // A 1-ring face whose texture values carry TWO ring leaves: the shape does
        // not match the geometry, so the texture is dropped (no dangling ring
        // ids/targets) and counted — not silently emitted.
        let coords: Vec<[f64; 3]> = (0..3).map(|i| [i as f64, 0.0, 0.0]).collect();
        let geom = DecodedGeometry {
            coords,
            kind: DecodedKind::PolyhedralSurface(vec![vec![vec![0, 1, 2]]]),
        };
        let props = serde_json::json!({ "type": "Solid", "shells": [[1]] });
        // Face 0 has 1 ring, but the texture gives it 2 ring leaves.
        let texture = serde_json::json!({
            "visual": { "values": [[[0, [0.0, 0.0], [1.0, 0.0], [0.0, 1.0]], [null]]] }
        });
        let tex_table = vec![serde_json::json!({ "type": "JPG", "image": "t.jpg" })];
        let b = BuildingSolids {
            id: "TM".into(),
            attributes: serde_json::Map::new(),
            solids: vec![(
                Lod::parse("2").unwrap(),
                geom,
                Some(props),
                None,
                Some(texture),
            )],
        };
        let mut report = WriteReport::default();
        let mut w = Writer::new(Vec::new());
        write_building(
            &mut w,
            &b,
            &types(),
            0,
            &mut Bounds::new(),
            &mut report,
            None,
            Some(&tex_table),
        )
        .unwrap();
        let xml = String::from_utf8(w.into_inner()).unwrap();
        assert_eq!(report.textures_written, 0, "{xml}");
        assert_eq!(report.texture_geometries_dropped, 1);
        assert!(!xml.contains("app:ParameterizedTexture"), "{xml}");
        assert!(!xml.contains("app:appearance"), "{xml}");
    }
}
