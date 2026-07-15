//! Face -> `gml:Polygon` serialisation.
//!
//! A decoded face is rings of coord indices into a shared coord pool
//! (`wkb_read::DecodedGeometry`'s shape). The WKB reader strips each ring's
//! closing duplicate vertex on decode (mirroring the CityGML reader, which
//! drops it too — see `citygml::geometry::read_linear_ring`), so rings here
//! are *open* (last != first). GML's `gml:LinearRing` requires a *closed*
//! ring, so [`pos_list`] re-appends the first coordinate.

use std::io::Write;

use cityparquet_schema::CityParquetError;
use quick_xml::Writer;
use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};

use crate::Result;

fn io_err(e: std::io::Error) -> CityParquetError {
    CityParquetError::Io(e.to_string())
}

/// One ring's `posList` text: `X Y Z` per vertex, world coords, **re-closed**
/// (the WKB reader strips the closing vertex, GML requires it back).
pub fn pos_list(coords: &[[f64; 3]], ring: &[usize]) -> String {
    let mut out = String::new();
    let mut push = |i: usize| {
        let c = coords[i];
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&format!("{} {} {}", c[0], c[1], c[2]));
    };
    for &i in ring {
        push(i);
    }
    if let Some(&first) = ring.first() {
        push(first); // re-close
    }
    out
}

/// Write a `gml:LinearRing` wrapping this ring's (re-closed) `posList`.
fn write_linear_ring<W: Write>(
    w: &mut Writer<W>,
    coords: &[[f64; 3]],
    ring: &[usize],
) -> Result<()> {
    w.write_event(Event::Start(BytesStart::new("gml:LinearRing")))
        .map_err(io_err)?;

    let mut pos_list_start = BytesStart::new("gml:posList");
    pos_list_start.push_attribute(("srsDimension", "3"));
    w.write_event(Event::Start(pos_list_start))
        .map_err(io_err)?;
    w.write_event(Event::Text(BytesText::new(&pos_list(coords, ring))))
        .map_err(io_err)?;
    w.write_event(Event::End(BytesEnd::new("gml:posList")))
        .map_err(io_err)?;

    w.write_event(Event::End(BytesEnd::new("gml:LinearRing")))
        .map_err(io_err)?;
    Ok(())
}

/// One face (rings of coord indices) -> a `<gml:Polygon>`: ring 0 exterior,
/// ring 1.. interior (holes). A face with no rings is a caller error
/// (upstream guarantees at least one exterior ring).
pub fn write_polygon<W: Write>(
    w: &mut Writer<W>,
    coords: &[[f64; 3]],
    face: &[Vec<usize>],
) -> Result<()> {
    w.write_event(Event::Start(BytesStart::new("gml:Polygon")))
        .map_err(io_err)?;

    let (exterior, interiors) = face
        .split_first()
        .ok_or_else(|| CityParquetError::Geometry("face has no rings to write".to_string()))?;

    w.write_event(Event::Start(BytesStart::new("gml:exterior")))
        .map_err(io_err)?;
    write_linear_ring(w, coords, exterior)?;
    w.write_event(Event::End(BytesEnd::new("gml:exterior")))
        .map_err(io_err)?;

    for hole in interiors {
        w.write_event(Event::Start(BytesStart::new("gml:interior")))
            .map_err(io_err)?;
        write_linear_ring(w, coords, hole)?;
        w.write_event(Event::End(BytesEnd::new("gml:interior")))
            .map_err(io_err)?;
    }

    w.write_event(Event::End(BytesEnd::new("gml:Polygon")))
        .map_err(io_err)?;
    Ok(())
}

/// A `PolyhedralSurface`'s flat face list + its `geometry_properties` ->
/// `<gml:Solid>` (shell 0 exterior, shells 1.. interior). The WKB flattens a
/// Solid's shells; the partition survives only in
/// `geometry_properties.solid_shell_faces`, so this reuses export's
/// `shell_faces_flat`/`partition_shells` rather than re-deriving it —
/// including their shell-count/face mismatch error.
pub fn write_solid<W: Write>(
    w: &mut Writer<W>,
    coords: &[[f64; 3]],
    faces: &[Vec<Vec<usize>>],
    props: Option<&serde_json::Value>,
) -> Result<()> {
    // A PolyhedralSurface WKB is emitted as a gml:Solid only when its paired
    // geometry_properties actually says `type: "Solid"`. Missing or mismatched
    // properties would otherwise let `partition_shells(None)` silently collapse
    // a multi-shell solid into one exterior shell, dropping cavities.
    let is_solid = props
        .and_then(|p| p.get("type"))
        .and_then(|t| t.as_str())
        == Some("Solid");
    if !is_solid {
        return Err(CityParquetError::Schema(
            "geometry_properties.type is not \"Solid\"; refusing to emit a gml:Solid from a \
             PolyhedralSurface without Solid shell metadata"
                .to_string(),
        ));
    }
    let counts = crate::export::shell_faces_flat(props)?;
    write_gml_solid(w, coords, faces, counts.as_deref())
}

/// Emit one `<gml:Solid>` from a flat face list + its shell partition counts,
/// with shell 0 exterior and shells 1.. interior. Shared by the top-level
/// [`write_solid`] and each [`write_composite_solid`] member.
fn write_gml_solid<W: Write>(
    w: &mut Writer<W>,
    coords: &[[f64; 3]],
    faces: &[Vec<Vec<usize>>],
    counts: Option<&[usize]>,
) -> Result<()> {
    let shells = crate::export::partition_shells(faces.to_vec(), counts)?;

    w.write_event(Event::Start(BytesStart::new("gml:Solid")))
        .map_err(io_err)?;

    let (exterior, interiors) = shells
        .split_first()
        .ok_or_else(|| CityParquetError::Geometry("solid has no shells to write".to_string()))?;

    w.write_event(Event::Start(BytesStart::new("gml:exterior")))
        .map_err(io_err)?;
    write_composite_surface(w, coords, exterior)?;
    w.write_event(Event::End(BytesEnd::new("gml:exterior")))
        .map_err(io_err)?;

    for shell in interiors {
        w.write_event(Event::Start(BytesStart::new("gml:interior")))
            .map_err(io_err)?;
        write_composite_surface(w, coords, shell)?;
        w.write_event(Event::End(BytesEnd::new("gml:interior")))
            .map_err(io_err)?;
    }

    w.write_event(Event::End(BytesEnd::new("gml:Solid")))
        .map_err(io_err)?;
    Ok(())
}

/// A `GeometryCollection` of `PolyhedralSurface` members (a CityJSON
/// `CompositeSolid`) -> `<gml:CompositeSolid>` of `<gml:solidMember>`-wrapped
/// `<gml:Solid>`s. Each member's shells come from the nested
/// `geometry_properties.solid_shell_faces[m]`. `MultiSolid` is NOT routed here
/// (CityGML 2.0 `Building` has no slot; the driver skips it).
pub fn write_composite_solid<W: Write>(
    w: &mut Writer<W>,
    coords: &[[f64; 3]],
    members: &[crate::wkb_read::DecodedKind],
    props: Option<&serde_json::Value>,
) -> Result<()> {
    let is_composite = props
        .and_then(|p| p.get("type"))
        .and_then(|t| t.as_str())
        == Some("CompositeSolid");
    if !is_composite {
        return Err(CityParquetError::Schema(
            "geometry_properties.type is not \"CompositeSolid\"; refusing to emit a \
             gml:CompositeSolid"
                .to_string(),
        ));
    }
    if members.is_empty() {
        return Err(CityParquetError::Geometry(
            "CompositeSolid has no solid members (gml:solidMember is minOccurs=1)".to_string(),
        ));
    }
    let nested = crate::export::shell_faces_nested(props)?;
    if let Some(counts) = &nested
        && counts.len() != members.len()
    {
        return Err(CityParquetError::Geometry(format!(
            "solid_shell_faces lists {} solids but the CompositeSolid has {} members",
            counts.len(),
            members.len()
        )));
    }

    w.write_event(Event::Start(BytesStart::new("gml:CompositeSolid")))
        .map_err(io_err)?;
    for (m, member) in members.iter().enumerate() {
        let crate::wkb_read::DecodedKind::PolyhedralSurface(faces) = member else {
            return Err(CityParquetError::Geometry(
                "CompositeSolid member is not a PolyhedralSurface".to_string(),
            ));
        };
        let counts = nested.as_ref().map(|c| c[m].as_slice());
        w.write_event(Event::Start(BytesStart::new("gml:solidMember")))
            .map_err(io_err)?;
        write_gml_solid(w, coords, faces, counts)?;
        w.write_event(Event::End(BytesEnd::new("gml:solidMember")))
            .map_err(io_err)?;
    }
    w.write_event(Event::End(BytesEnd::new("gml:CompositeSolid")))
        .map_err(io_err)?;
    Ok(())
}

/// A shell (list of faces) -> `<gml:CompositeSurface>` of `gml:surfaceMember`
/// wrapped `gml:Polygon`s.
fn write_composite_surface<W: Write>(
    w: &mut Writer<W>,
    coords: &[[f64; 3]],
    shell: &[Vec<Vec<usize>>],
) -> Result<()> {
    w.write_event(Event::Start(BytesStart::new("gml:CompositeSurface")))
        .map_err(io_err)?;
    for face in shell {
        w.write_event(Event::Start(BytesStart::new("gml:surfaceMember")))
            .map_err(io_err)?;
        write_polygon(w, coords, face)?;
        w.write_event(Event::End(BytesEnd::new("gml:surfaceMember")))
            .map_err(io_err)?;
    }
    w.write_event(Event::End(BytesEnd::new("gml:CompositeSurface")))
        .map_err(io_err)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use quick_xml::Writer;

    fn emit<F: Fn(&mut Writer<Vec<u8>>) -> crate::Result<()>>(f: F) -> String {
        let mut w = Writer::new(Vec::new());
        f(&mut w).unwrap();
        String::from_utf8(w.into_inner()).unwrap()
    }

    #[test]
    fn pos_list_reclose_appends_first_coord() {
        let coords = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0]];
        // open ring (reader-decoded shape: last != first)
        let ring = vec![0usize, 1, 2];
        assert_eq!(pos_list(&coords, &ring), "0 0 0 1 0 0 1 1 0 0 0 0");
    }

    #[test]
    fn write_polygon_emits_exterior_and_interior_rings() {
        let coords = vec![
            [0.0, 0.0, 0.0],
            [4.0, 0.0, 0.0],
            [4.0, 4.0, 0.0],
            [0.0, 4.0, 0.0], // outer
            [1.0, 1.0, 0.0],
            [2.0, 1.0, 0.0],
            [2.0, 2.0, 0.0], // hole
        ];
        let face = vec![vec![0usize, 1, 2, 3], vec![4usize, 5, 6]];
        let xml = emit(|w| write_polygon(w, &coords, &face));
        assert!(xml.contains("<gml:Polygon>"));
        assert!(xml.contains("<gml:exterior><gml:LinearRing><gml:posList srsDimension=\"3\">0 0 0 4 0 0 4 4 0 0 4 0 0 0 0</gml:posList>"));
        assert!(xml.contains("<gml:interior><gml:LinearRing><gml:posList srsDimension=\"3\">1 1 0 2 1 0 2 2 0 1 1 0</gml:posList>"));
    }

    #[test]
    fn write_solid_partitions_exterior_and_interior_shells() {
        // 4 coords forming two trivial triangular faces per shell is overkill;
        // use a minimal 2-shell case: shell0 = 1 face, shell1 = 1 face.
        let coords = vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0], // face 0 (outer shell)
            [0.2, 0.2, 0.2],
            [0.4, 0.2, 0.2],
            [0.4, 0.4, 0.2], // face 1 (inner shell)
        ];
        let faces = vec![vec![vec![0usize, 1, 2]], vec![vec![3usize, 4, 5]]];
        let props = serde_json::json!({ "type": "Solid", "solid_shell_faces": [1, 1] });
        let xml = emit(|w| write_solid(w, &coords, &faces, Some(&props)));
        assert!(xml.starts_with("<gml:Solid>"));
        assert!(xml.contains("<gml:exterior><gml:CompositeSurface>"));
        assert!(xml.contains("<gml:interior><gml:CompositeSurface>"));
        // Exactly one exterior shell, one interior shell. `gml:exterior` /
        // `gml:interior` are legitimately reused by GML at the ring level too
        // (write_polygon wraps each face's outer ring in `<gml:exterior>`),
        // so match on the shell-level pairing with `gml:CompositeSurface`
        // rather than the bare tag, which would also count ring boundaries.
        assert_eq!(
            xml.matches("<gml:exterior><gml:CompositeSurface>").count(),
            1
        );
        assert_eq!(
            xml.matches("<gml:interior><gml:CompositeSurface>").count(),
            1
        );
    }

    #[test]
    fn write_solid_rejects_non_solid_or_missing_type() {
        let coords = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0]];
        let faces = vec![vec![vec![0usize, 1, 2]]];
        // No props at all.
        let mut w = Writer::new(Vec::new());
        assert!(write_solid(&mut w, &coords, &faces, None).is_err());
        // Props present but the wrong type.
        let props = serde_json::json!({ "type": "MultiSurface" });
        let mut w = Writer::new(Vec::new());
        assert!(write_solid(&mut w, &coords, &faces, Some(&props)).is_err());
    }

    #[test]
    fn write_solid_single_shell_has_no_interior() {
        let coords = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0]];
        let faces = vec![vec![vec![0usize, 1, 2]]];
        // type Solid, no solid_shell_faces -> single shell fallback.
        let props = serde_json::json!({ "type": "Solid" });
        let xml = emit(|w| write_solid(w, &coords, &faces, Some(&props)));
        assert_eq!(
            xml.matches("<gml:exterior><gml:CompositeSurface>").count(),
            1
        );
        assert_eq!(
            xml.matches("<gml:interior><gml:CompositeSurface>").count(),
            0
        );
    }

    #[test]
    fn write_solid_shell_count_mismatch_errors() {
        let coords = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0]];
        let faces = vec![vec![vec![0usize, 1, 2]]]; // 1 face
        let props = serde_json::json!({ "solid_shell_faces": [1, 1] }); // claims 2
        let mut w = Writer::new(Vec::new());
        assert!(write_solid(&mut w, &coords, &faces, Some(&props)).is_err());
    }

    use crate::wkb_read::DecodedKind;

    fn two_member_props() -> serde_json::Value {
        // Each solid: one shell of one face.
        serde_json::json!({ "type": "CompositeSolid", "solid_shell_faces": [[1], [1]] })
    }

    #[test]
    fn write_composite_solid_emits_a_member_per_solid() {
        let coords = vec![
            [0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], // member 0 face
            [2.0, 0.0, 0.0], [3.0, 0.0, 0.0], [3.0, 1.0, 0.0], // member 1 face
        ];
        let members = vec![
            DecodedKind::PolyhedralSurface(vec![vec![vec![0usize, 1, 2]]]),
            DecodedKind::PolyhedralSurface(vec![vec![vec![3usize, 4, 5]]]),
        ];
        let props = two_member_props();
        let xml = emit(|w| write_composite_solid(w, &coords, &members, Some(&props)));
        assert!(xml.starts_with("<gml:CompositeSolid>"), "{xml}");
        assert_eq!(xml.matches("<gml:solidMember>").count(), 2, "{xml}");
        assert_eq!(xml.matches("<gml:Solid>").count(), 2, "{xml}");
    }

    #[test]
    fn write_composite_solid_single_member_stays_composite() {
        let coords = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0]];
        let members = vec![DecodedKind::PolyhedralSurface(vec![vec![vec![0usize, 1, 2]]])];
        let props = serde_json::json!({ "type": "CompositeSolid", "solid_shell_faces": [[1]] });
        let xml = emit(|w| write_composite_solid(w, &coords, &members, Some(&props)));
        assert!(xml.starts_with("<gml:CompositeSolid>"));
        assert_eq!(xml.matches("<gml:solidMember>").count(), 1);
    }

    #[test]
    fn write_composite_solid_rejects_zero_members() {
        let members: Vec<DecodedKind> = vec![];
        let props = serde_json::json!({ "type": "CompositeSolid", "solid_shell_faces": [] });
        let mut w = Writer::new(Vec::new());
        assert!(write_composite_solid(&mut w, &[], &members, Some(&props)).is_err());
    }

    #[test]
    fn write_composite_solid_rejects_non_composite_type() {
        let coords = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0]];
        let members = vec![DecodedKind::PolyhedralSurface(vec![vec![vec![0usize, 1, 2]]])];
        let props = serde_json::json!({ "type": "MultiSolid", "solid_shell_faces": [[1]] });
        let mut w = Writer::new(Vec::new());
        assert!(write_composite_solid(&mut w, &coords, &members, Some(&props)).is_err());
    }

    #[test]
    fn write_composite_solid_rejects_member_count_mismatch() {
        let coords = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0]];
        let members = vec![DecodedKind::PolyhedralSurface(vec![vec![vec![0usize, 1, 2]]])];
        // counts claim 2 solids, only 1 member present.
        let props = serde_json::json!({ "type": "CompositeSolid", "solid_shell_faces": [[1], [1]] });
        let mut w = Writer::new(Vec::new());
        assert!(write_composite_solid(&mut w, &coords, &members, Some(&props)).is_err());
    }
}
