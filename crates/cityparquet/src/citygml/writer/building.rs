//! `bldg:Building` serialisation with LoD-major mapping and `gml:id` validation.
//!
//! CityGML 2.0 only has `bldg:lod1Solid`..`bldg:lod4Solid`, each 0..1 per
//! building. A CityParquet package can carry several geometry LoD columns that
//! share a major (e.g. `lod2` and `lod2_2` both map to major 2), so this picks
//! ONE per major — the most detailed (highest minor) — and counts the rest as
//! skipped. Only WKB `PolyhedralSurface` (a CityJSON `Solid`) is emitted in
//! W-M1; `GeometryCollection` (MultiSolid/CompositeSolid) and other shapes are
//! the driver's concern to count, but a stray non-Solid is guarded here too.

use std::collections::{BTreeMap, HashMap};
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
    IdAlloc, parse_semantics, semantic_surface_count, surfaces_emittable,
    write_multisurface_with_semantics, write_solid_with_semantics,
};
use crate::Result;
use crate::wkb_read::{DecodedGeometry, DecodedKind};

fn io_err(e: std::io::Error) -> CityParquetError {
    CityParquetError::Io(e.to_string())
}

/// Emit a `bldg:lod<major>Solid` with plain (geometry-only) inline geometry —
/// the W-M2 form, used when a geometry has no emittable semantics or when the
/// semantics path fails and falls back. Any semantics present but not emitted
/// are counted as dropped.
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
    report.semantic_surfaces_dropped += semantic_surface_count(props);
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
            DecodedKind::MultiPolygon(_) => {
                parse_semantics(props.as_ref()).is_some_and(|s| surfaces_emittable(&s))
            }
            _ => false,
        };
        if !representable {
            report.lod_columns_skipped += 1;
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

    // Buffer attributes first so the emptiness decision can see whether any
    // attribute is actually writable: an attributes-only Building is valid, but
    // a Building with neither geometry nor a writable attribute is not emitted.
    let mut attr_buf = Writer::new(Vec::new());
    let attrs_written = write_attributes(&mut attr_buf, &b.attributes, types, report)?;

    if by_major.is_empty() && attrs_written == 0 {
        return Ok(false);
    }

    w.write_event(Event::Start(BytesStart::new("cityObjectMember")))
        .map_err(io_err)?;
    let mut bldg = BytesStart::new("bldg:Building");
    bldg.push_attribute(("gml:id", b.id.as_str()));
    w.write_event(Event::Start(bldg)).map_err(io_err)?;
    // Attributes precede geometry in the CityGML _CityObject / Building sequence.
    w.get_mut()
        .write_all(&attr_buf.into_inner())
        .map_err(io_err)?;

    for (major, (_, geom, props)) in &by_major {
        let major = *major;
        let props = *props;
        // A geometry with emittable semantics takes the W-M3 path; otherwise the
        // plain W-M2 path (which counts any un-emittable semantics as dropped).
        let sem = parse_semantics(props).filter(surfaces_emittable);
        match (&geom.kind, sem) {
            (DecodedKind::PolyhedralSurface(_) | DecodedKind::GeometryCollection(_), Some(sem)) => {
                let mut ids = IdAlloc::new(feature_index);
                match write_solid_with_semantics(
                    w,
                    &geom.coords,
                    &geom.kind,
                    props,
                    &sem,
                    &mut ids,
                    major,
                ) {
                    Ok(()) => report.semantic_surfaces_written += sem.surfaces.len(),
                    // Resolution failed (corrupt/external values): emit plain
                    // geometry and count the surfaces as dropped.
                    Err(_) => write_plain_lodn_solid(w, geom, props, major, report)?,
                }
            }
            (DecodedKind::MultiPolygon(faces), Some(sem)) => {
                write_multisurface_with_semantics(w, &geom.coords, faces, &sem, major, report)?;
                report.semantic_surfaces_written += sem.surfaces.len();
            }
            (DecodedKind::PolyhedralSurface(_) | DecodedKind::GeometryCollection(_), None) => {
                write_plain_lodn_solid(w, geom, props, major, report)?;
            }
            _ => unreachable!("the representable gate excludes other (kind, semantics) cases"),
        }
        // Accumulate the geometry's coord pool (equals the referenced set for
        // WKB-decoded geometry; covers solids, composites, and multisurfaces).
        for c in &geom.coords {
            bounds.add(*c);
        }
    }

    w.write_event(Event::End(BytesEnd::new("bldg:Building")))
        .map_err(io_err)?;
    w.write_event(Event::End(BytesEnd::new("cityObjectMember")))
        .map_err(io_err)?;
    Ok(true)
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
