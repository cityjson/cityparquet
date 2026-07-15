//! `bldg:Building` serialisation with LoD-major mapping and `gml:id` validation.
//!
//! CityGML 2.0 only has `bldg:lod1Solid`..`bldg:lod4Solid`, each 0..1 per
//! building. A CityParquet package can carry several geometry LoD columns that
//! share a major (e.g. `lod2` and `lod2_2` both map to major 2), so this picks
//! ONE per major — the most detailed (highest minor) — and counts the rest as
//! skipped. Only WKB `PolyhedralSurface` (a CityJSON `Solid`) is emitted in
//! W-M1; `GeometryCollection` (MultiSolid/CompositeSolid) and other shapes are
//! the driver's concern to count, but a stray non-Solid is guarded here too.

use std::collections::BTreeMap;
use std::io::Write;

use cityparquet_schema::{CityParquetError, Lod};
use quick_xml::Writer;
use quick_xml::events::{BytesEnd, BytesStart, Event};
use serde_json::Value;

use super::WriteReport;
use super::attributes::write_attributes;
use super::document::Bounds;
use super::geometry::write_solid;
use crate::Result;
use crate::wkb_read::{DecodedGeometry, DecodedKind};

fn io_err(e: std::io::Error) -> CityParquetError {
    CityParquetError::Io(e.to_string())
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
        if !matches!(geom.kind, DecodedKind::PolyhedralSurface(_)) {
            // Only a Solid becomes a lodNSolid; a stray non-Solid is skipped.
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
        if geom
            .coords
            .iter()
            .any(|c| c.iter().any(|v| !v.is_finite()))
        {
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
    let attrs_written = write_attributes(&mut attr_buf, &b.attributes, report)?;

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
        let elem = format!("bldg:lod{major}Solid");
        let DecodedKind::PolyhedralSurface(faces) = &geom.kind else {
            unreachable!("only PolyhedralSurface entries reach by_major");
        };
        w.write_event(Event::Start(BytesStart::new(elem.as_str())))
            .map_err(io_err)?;
        write_solid(w, &geom.coords, faces, *props)?;
        for face in faces {
            for ring in face {
                for &i in ring {
                    bounds.add(geom.coords[i]);
                }
            }
        }
        w.write_event(Event::End(BytesEnd::new(elem.as_str())))
            .map_err(io_err)?;
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
        assert!(write_building(&mut w, &b, &mut bounds, &mut report).unwrap());
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
        assert!(!write_building(&mut w, &b, &mut Bounds::new(), &mut report).unwrap());
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
            write_building(&mut w, &b, &mut Bounds::new(), &mut WriteReport::default()).is_err()
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
            write_building(&mut w, &b, &mut Bounds::new(), &mut WriteReport::default()).is_err()
        );
    }

    #[test]
    fn attributes_only_building_emits_with_no_solid() {
        let mut attributes = serde_json::Map::new();
        attributes.insert("roofType".into(), serde_json::json!("1000"));
        let b = BuildingSolids { id: "B5".into(), attributes, solids: vec![] };
        let mut report = WriteReport::default();
        let mut w = Writer::new(Vec::new());
        assert!(write_building(&mut w, &b, &mut Bounds::new(), &mut report).unwrap());
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
            !write_building(&mut w, &b, &mut Bounds::new(), &mut WriteReport::default()).unwrap()
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
        write_building(&mut w, &b, &mut Bounds::new(), &mut WriteReport::default()).unwrap();
        let xml = String::from_utf8(w.into_inner()).unwrap();
        assert!(xml.find("<bldg:roofType>").unwrap() < xml.find("<bldg:lod2Solid>").unwrap());
    }
}
