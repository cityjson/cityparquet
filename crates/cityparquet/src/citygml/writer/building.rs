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
use super::attributes::write_attributes;
use super::document::Bounds;
use super::geometry::{write_composite_solid, write_solid};
use super::semantics::{
    IdAlloc, Semantics, droppable_surface_count, has_nonnull_value, parse_semantics,
    surfaces_emittable, write_multisurface_with_semantics, write_solid_with_semantics,
};
use crate::Result;
use crate::wkb_read::{DecodedGeometry, DecodedKind};

fn io_err(e: std::io::Error) -> CityParquetError {
    CityParquetError::Io(e.to_string())
}

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
/// building's chosen semantic LoD). Any real (non-null) semantics not emitted
/// here are counted as dropped.
fn write_plain_lodn_solid<W: Write>(
    w: &mut Writer<W>,
    geom: &DecodedGeometry,
    props: Option<&Value>,
    major: u8,
    report: &mut WriteReport,
) -> Result<()> {
    let elem = format!("bldg:lod{major}Solid");
    w.write_event(Event::Start(BytesStart::new(elem.as_str())))
        .map_err(io_err)?;
    match &geom.kind {
        DecodedKind::PolyhedralSurface(faces) => write_solid(w, &geom.coords, faces, props)?,
        DecodedKind::GeometryCollection(members) => {
            write_composite_solid(w, &geom.coords, members, props)?;
            report.composite_solids_written += 1;
        }
        _ => unreachable!("only PolyhedralSurface/GeometryCollection reach the plain solid path"),
    }
    w.write_event(Event::End(BytesEnd::new(elem.as_str())))
        .map_err(io_err)?;
    report.semantic_surfaces_dropped += droppable_surface_count(props);
    Ok(())
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
    pub solids: Vec<(Lod, DecodedGeometry, Option<Value>)>,
}

/// Emit `<cityObjectMember><bldg:Building gml:id="…">` with one
/// `bldg:lod<major>Solid` per major LoD (highest minor wins), accumulating
/// every emitted coordinate into `bounds`. Returns `Ok(false)` (nothing
/// written) when no solid survives — the caller counts that building in
/// `buildings_without_solid_skipped`. Errors when `id` is not a valid NCName.
pub fn write_building<W: Write>(
    w: &mut Writer<W>,
    b: &BuildingSolids,
    types: &HashMap<String, AttributeType>,
    feature_index: usize,
    bounds: &mut Bounds,
    report: &mut WriteReport,
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
    if !write_object_content(&mut content, b, types, feature_index, bounds, report)? {
        return Ok(false);
    }
    w.write_event(Event::Start(BytesStart::new("cityObjectMember")))
        .map_err(io_err)?;
    let mut bldg = BytesStart::new("bldg:Building");
    bldg.push_attribute(("gml:id", b.id.as_str()));
    w.write_event(Event::Start(bldg)).map_err(io_err)?;
    w.get_mut()
        .write_all(&content.into_inner())
        .map_err(io_err)?;
    w.write_event(Event::End(BytesEnd::new("bldg:Building")))
        .map_err(io_err)?;
    w.write_event(Event::End(BytesEnd::new("cityObjectMember")))
        .map_err(io_err)?;
    Ok(true)
}

/// Emit the `_AbstractBuilding` content — attributes then geometry, the inside of
/// a `<bldg:Building>`/`<bldg:BuildingPart>` WITHOUT the wrapper. Returns whether
/// anything (a writable attribute or a geometry) was emitted; the caller
/// suppresses the wrapper when nothing was.
pub fn write_object_content<W: Write>(
    w: &mut Writer<W>,
    b: &BuildingSolids,
    types: &HashMap<String, AttributeType>,
    feature_index: usize,
    bounds: &mut Bounds,
    report: &mut WriteReport,
) -> Result<bool> {
    // Keep at most one solid per major LoD (1..=4), the highest minor.
    // BTreeMap keeps the majors in ascending order for emission.
    let mut by_major: BTreeMap<u8, (Lod, &DecodedGeometry, Option<&Value>)> = BTreeMap::new();
    for (lod, geom, props) in &b.solids {
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
        match by_major.get(&major) {
            None => {
                by_major.insert(major, (*lod, geom, props.as_ref()));
            }
            Some((existing, _, _)) => {
                // One of the two collides away; keep the higher minor.
                if *lod > *existing {
                    by_major.insert(major, (*lod, geom, props.as_ref()));
                }
                report.lod_columns_skipped += 1;
            }
        }
    }

    // Reject non-finite coordinates before emitting anything: `inf`/`-inf`/`NaN`
    // are not valid XML Schema `double` lexical forms and NaN would poison the
    // envelope. (WKB's 2^53 magnitude guard does not catch NaN, whose
    // comparisons are always false.) No-op for an attributes-only building.
    for (_, geom, _) in by_major.values() {
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
        .filter(|(major, (_, geom, props))| {
            emittable_semantics(**major, &geom.kind, *props).is_some()
        })
        .map(|(major, _)| *major)
        .max();

    let mut any_geometry = false;
    for (major, (_, geom, props)) in &by_major {
        let major = *major;
        let props = *props;
        let mut emitted_geometry = true;
        if Some(major) == chosen_major {
            // The one LoD that emits bldg:boundedBy semantic surfaces.
            let sem = emittable_semantics(major, &geom.kind, props)
                .expect("chosen major has emittable semantics");
            match &geom.kind {
                DecodedKind::PolyhedralSurface(_) | DecodedKind::GeometryCollection(_) => {
                    let mut ids = IdAlloc::new(feature_index, major);
                    match write_solid_with_semantics(
                        w,
                        &geom.coords,
                        &geom.kind,
                        props,
                        &sem,
                        &mut ids,
                        major,
                    ) {
                        Ok(()) => {
                            report.semantic_surfaces_written += sem.surfaces.len();
                            if matches!(geom.kind, DecodedKind::GeometryCollection(_)) {
                                report.composite_solids_written += 1;
                            }
                        }
                        // Resolution failed (corrupt/external values): fall back
                        // to plain geometry, counting the surfaces as dropped.
                        Err(_) => write_plain_lodn_solid(w, geom, props, major, report)?,
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
                    ) {
                        Ok(()) => report.semantic_surfaces_written += sem.surfaces.len(),
                        // A MultiSurface has no geometry-only form; drop it.
                        Err(_) => {
                            report.semantic_surfaces_dropped += sem.surfaces.len();
                            emitted_geometry = false;
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
                    write_plain_lodn_solid(w, geom, props, major, report)?;
                }
                DecodedKind::MultiPolygon(_) => {
                    report.semantic_surfaces_dropped += droppable_surface_count(props);
                    emitted_geometry = false;
                }
                _ => unreachable!("the representable gate excludes other kinds"),
            }
        }
        // Accumulate the coord pool of geometry we actually emitted (a dropped
        // MultiSurface contributes nothing to the envelope).
        if emitted_geometry {
            any_geometry = true;
            for c in &geom.coords {
                bounds.add(*c);
            }
        }
    }

    // Emitted iff a writable attribute or an ACTUALLY-written geometry — a
    // by_major entry that dropped (e.g. a MultiSurface resolution failure) must
    // not make an otherwise-empty object non-empty (no husk elements).
    Ok(attrs_written > 0 || any_geometry)
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
    let self_emitted =
        write_object_content(&mut inner, obj, tree.types, feature_index, bounds, report)?;

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
            inner
                .write_event(Event::Start(BytesStart::new("bldg:consistsOfBuildingPart")))
                .map_err(io_err)?;
            let mut bp = BytesStart::new("bldg:BuildingPart");
            bp.push_attribute(("gml:id", child_id.as_str()));
            inner.write_event(Event::Start(bp)).map_err(io_err)?;
            inner.get_mut().write_all(&child_inner).map_err(io_err)?;
            inner
                .write_event(Event::End(BytesEnd::new("bldg:BuildingPart")))
                .map_err(io_err)?;
            inner
                .write_event(Event::End(BytesEnd::new("bldg:consistsOfBuildingPart")))
                .map_err(io_err)?;
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
        serde_json::json!({ "type": "CompositeSolid", "solid_shell_faces": [[1]] })
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
        write_building(&mut w, b, &types(), 0, &mut Bounds::new(), &mut report).unwrap();
        (String::from_utf8(w.into_inner()).unwrap(), report)
    }

    #[test]
    fn solid_with_semantics_emits_boundedby() {
        let props = serde_json::json!({
            "type": "Solid", "solid_shell_faces": [1],
            "semantics": { "surfaces": [{"type": "WallSurface"}], "values": [[0]] }
        });
        let b = BuildingSolids {
            id: "S1".into(),
            attributes: serde_json::Map::new(),
            solids: vec![(Lod::parse("2").unwrap(), tri_solid(), Some(props))],
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
            "semantics": { "surfaces": [{"type": "RoofSurface"}], "values": [0] }
        });
        let b = BuildingSolids {
            id: "M1".into(),
            attributes: serde_json::Map::new(),
            solids: vec![(Lod::parse("2").unwrap(), geom, Some(props))],
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
            "type": "Solid", "solid_shell_faces": [1],
            "semantics": { "surfaces": [{"type": "WallSurface"}], "values": [[value]] }
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
                ),
                (
                    Lod::parse("3").unwrap(),
                    tri_solid(),
                    Some(sem_solid_props(serde_json::json!(0))),
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
                ),
                (
                    Lod::parse("2").unwrap(),
                    tri_solid(),
                    Some(sem_solid_props(serde_json::json!(0))),
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
            "type": "Solid", "solid_shell_faces": [1],
            "semantics": { "surfaces": [{"type": "WallSurface"}], "values": [[0, 1]] }
        });
        let b = BuildingSolids {
            id: "S2".into(),
            attributes: serde_json::Map::new(),
            solids: vec![(Lod::parse("2").unwrap(), tri_solid(), Some(props))],
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
            )],
        };
        let mut report = WriteReport::default();
        let mut w = Writer::new(Vec::new());
        assert!(write_building(&mut w, &b, &types(), 0, &mut Bounds::new(), &mut report).unwrap());
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
                (Lod::parse("2").unwrap(), tri_solid(), Some(solid_props())),
                (Lod::parse("2.2").unwrap(), tri_solid(), Some(solid_props())),
            ],
        };
        let mut bounds = Bounds::new();
        let mut report = WriteReport::default();
        let mut w = Writer::new(Vec::new());
        assert!(write_building(&mut w, &b, &types(), 0, &mut bounds, &mut report).unwrap());
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
                (Lod::parse("2").unwrap(), tri_solid(), Some(solid_props())),
                (Lod::parse("1").unwrap(), tri_solid(), Some(solid_props())),
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
                &mut WriteReport::default()
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
            solids: vec![(Lod::parse("0").unwrap(), tri_solid(), None)],
        };
        let mut report = WriteReport::default();
        let mut w = Writer::new(Vec::new());
        assert!(!write_building(&mut w, &b, &types(), 0, &mut Bounds::new(), &mut report).unwrap());
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
            solids: vec![(Lod::parse("2").unwrap(), geom, None)],
        };
        let mut w = Writer::new(Vec::new());
        assert!(
            write_building(
                &mut w,
                &b,
                &types(),
                0,
                &mut Bounds::new(),
                &mut WriteReport::default()
            )
            .is_err()
        );
        // Nothing should have been emitted before the error.
        assert!(w.into_inner().is_empty());
    }

    fn content(id: &str, solids: Vec<(Lod, DecodedGeometry, Option<Value>)>) -> BuildingSolids {
        BuildingSolids {
            id: id.into(),
            attributes: serde_json::Map::new(),
            solids,
        }
    }

    fn solid_lod2() -> (Lod, DecodedGeometry, Option<Value>) {
        (Lod::parse("2").unwrap(), tri_solid(), Some(solid_props()))
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
            "semantics": { "surfaces": [{"type": "WallSurface"}], "values": [0, 1] } // 2 values
        });
        let content_by_id = HashMap::from([(
            "P".to_string(),
            BuildingSolids {
                id: "P".into(),
                attributes: serde_json::Map::new(),
                solids: vec![(Lod::parse("2").unwrap(), geom, Some(props))],
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
                &mut WriteReport::default()
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
        assert!(write_building(&mut w, &b, &types(), 0, &mut Bounds::new(), &mut report).unwrap());
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
                &mut WriteReport::default()
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
            solids: vec![(Lod::parse("2").unwrap(), tri_solid(), Some(solid_props()))],
        };
        let mut w = Writer::new(Vec::new());
        write_building(
            &mut w,
            &b,
            &types(),
            0,
            &mut Bounds::new(),
            &mut WriteReport::default(),
        )
        .unwrap();
        let xml = String::from_utf8(w.into_inner()).unwrap();
        assert!(xml.find("<bldg:roofType>").unwrap() < xml.find("<bldg:lod2Solid>").unwrap());
    }
}
