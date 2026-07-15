//! Parse `geometry_properties.semantics` and resolve each WKB face to its
//! surface index, in the face-walk order the solid/multisurface emitter uses.
//!
//! `surfaces` is a flat list of surface type strings; `values` maps faces to
//! surface indices, nested by geometry: Solid `[shell][face]`, CompositeSolid
//! `[solid][shell][face]`, MultiSurface flat `[position]`. A `null` value means
//! the face has no semantic surface.

use std::collections::HashSet;
use std::io::Write;

use cityparquet_schema::CityParquetError;
use quick_xml::Writer;
use quick_xml::events::{BytesEnd, BytesStart, Event};
use serde_json::Value;

use super::WriteReport;
use super::building::is_ncname;
use super::geometry::{write_inline_member, write_xlink_member};
use crate::Result;
use crate::wkb_read::DecodedKind;

fn io_err(e: std::io::Error) -> CityParquetError {
    CityParquetError::Io(e.to_string())
}

/// A face is rings of coord indices (ring 0 exterior, 1.. holes).
pub type Face = Vec<Vec<usize>>;

/// One solid's shells, each a list of faces (as `partition_shells` returns).
type Shells = Vec<Vec<Face>>;

/// A flat per-face surface index in shell-concatenation order (`None` = null).
type FaceSurfaces = Vec<Option<usize>>;

/// Allocates document-unique `gml:id`s for emitted polygons: `_cpq_p<N>` with a
/// monotonic counter, checked against (and inserted into) the shared `seen`
/// set that also holds CityObject `gml:id`s, so a generated id can never clash
/// with an object id or another polygon.
pub struct IdAlloc<'a> {
    next: usize,
    seen: &'a mut HashSet<String>,
}

impl<'a> IdAlloc<'a> {
    pub fn new(seen: &'a mut HashSet<String>) -> Self {
        Self { next: 0, seen }
    }

    pub fn alloc(&mut self) -> String {
        loop {
            let id = format!("_cpq_p{}", self.next);
            self.next += 1;
            if self.seen.insert(id.clone()) {
                return id;
            }
        }
    }
}

/// Parsed `geometry_properties.semantics`: a flat list of surface type strings
/// and the raw (nested) `values` array.
pub struct Semantics {
    pub surfaces: Vec<String>,
    pub values: Value,
}

fn err(m: impl Into<String>) -> CityParquetError {
    CityParquetError::Geometry(m.into())
}

/// Extract `{surfaces, values}` from a geometry's `geometry_properties`, or
/// `None` when there is no (well-formed) semantics object.
pub fn parse_semantics(props: Option<&Value>) -> Option<Semantics> {
    let sem = props?.get("semantics")?;
    let surfaces = sem.get("surfaces")?.as_array()?;
    let surfaces: Vec<String> = surfaces
        .iter()
        .map(|s| {
            s.get("type")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string()
        })
        .collect();
    let values = sem.get("values")?.clone();
    Some(Semantics { surfaces, values })
}

/// Whether every surface type is a legal XML element name (NCName). A CityJSON
/// extension type (`+Foo`) is not, so such a geometry falls back to geometry-only.
pub fn surfaces_emittable(s: &Semantics) -> bool {
    !s.surfaces.is_empty() && s.surfaces.iter().all(|t| is_ncname(t))
}

/// One face's value: `null` -> `None`, a non-negative integer in range ->
/// `Some(idx)`, anything else -> `Err`.
fn face_index(v: &Value, nsurfaces: usize) -> Result<Option<usize>> {
    match v {
        Value::Null => Ok(None),
        Value::Number(n) => {
            let i = n
                .as_u64()
                .ok_or_else(|| err("semantics value is not a non-negative integer"))?
                as usize;
            if i >= nsurfaces {
                return Err(err(format!(
                    "semantics value {i} >= surfaces len {nsurfaces}"
                )));
            }
            Ok(Some(i))
        }
        _ => Err(err("semantics value is neither null nor an integer")),
    }
}

/// Resolve a Solid's `[shell][face]` values to a flat per-face surface index, in
/// shell-concatenation order (the order the emitter walks faces).
pub fn solid_face_surfaces(
    values: &Value,
    shells: &[Vec<Face>],
    nsurfaces: usize,
) -> Result<FaceSurfaces> {
    let vshells = values
        .as_array()
        .ok_or_else(|| err("solid semantics values must be an array of shells"))?;
    if vshells.len() != shells.len() {
        return Err(err(format!(
            "semantics has {} shells but geometry has {}",
            vshells.len(),
            shells.len()
        )));
    }
    let mut out = Vec::new();
    for (vs, shell) in vshells.iter().zip(shells) {
        let vfaces = vs
            .as_array()
            .ok_or_else(|| err("shell values must be an array"))?;
        if vfaces.len() != shell.len() {
            return Err(err(format!(
                "semantics shell has {} faces but geometry has {}",
                vfaces.len(),
                shell.len()
            )));
        }
        for v in vfaces {
            out.push(face_index(v, nsurfaces)?);
        }
    }
    Ok(out)
}

/// Resolve a CompositeSolid's `[solid][shell][face]` values, one flat per-face
/// vec per member.
pub fn composite_face_surfaces(
    values: &Value,
    members: &[Shells],
    nsurfaces: usize,
) -> Result<Vec<FaceSurfaces>> {
    let vmembers = values
        .as_array()
        .ok_or_else(|| err("composite semantics values must be an array of solids"))?;
    if vmembers.len() != members.len() {
        return Err(err(format!(
            "semantics has {} solids but geometry has {}",
            vmembers.len(),
            members.len()
        )));
    }
    let mut out = Vec::with_capacity(members.len());
    for (vm, shells) in vmembers.iter().zip(members) {
        out.push(solid_face_surfaces(vm, shells, nsurfaces)?);
    }
    Ok(out)
}

/// Resolve a MultiSurface's flat `[position]` values.
pub fn multisurface_face_surfaces(
    values: &Value,
    nfaces: usize,
    nsurfaces: usize,
) -> Result<Vec<Option<usize>>> {
    let vs = values
        .as_array()
        .ok_or_else(|| err("multisurface semantics values must be an array"))?;
    if vs.len() != nfaces {
        return Err(err(format!(
            "semantics has {} values but geometry has {} faces",
            vs.len(),
            nfaces
        )));
    }
    vs.iter().map(|v| face_index(v, nsurfaces)).collect()
}

/// Emit one `<gml:Solid>` whose shells are `gml:CompositeSurface`s of
/// `surfaceMember`s: an `xlink:href` for a semantic face (its polygon lives in
/// `boundedBy`) or an inline `gml:Polygon` for a null face. `ids` is the flat
/// per-face id list in shell-concatenation order.
fn write_one_solid<W: Write>(
    w: &mut Writer<W>,
    coords: &[[f64; 3]],
    shells: &[Vec<Face>],
    ids: &[Option<String>],
) -> Result<()> {
    w.write_event(Event::Start(BytesStart::new("gml:Solid")))
        .map_err(io_err)?;
    let mut fi = 0usize;
    for (si, shell) in shells.iter().enumerate() {
        let boundary = if si == 0 {
            "gml:exterior"
        } else {
            "gml:interior"
        };
        w.write_event(Event::Start(BytesStart::new(boundary)))
            .map_err(io_err)?;
        w.write_event(Event::Start(BytesStart::new("gml:CompositeSurface")))
            .map_err(io_err)?;
        for face in shell {
            match &ids[fi] {
                Some(id) => write_xlink_member(w, id)?,
                None => write_inline_member(w, coords, face, None)?,
            }
            fi += 1;
        }
        w.write_event(Event::End(BytesEnd::new("gml:CompositeSurface")))
            .map_err(io_err)?;
        w.write_event(Event::End(BytesEnd::new(boundary)))
            .map_err(io_err)?;
    }
    w.write_event(Event::End(BytesEnd::new("gml:Solid")))
        .map_err(io_err)?;
    Ok(())
}

/// Emit one `<bldg:boundedBy><bldg:{ty}>…` surface holding `polys` as inline
/// `gml:Polygon`s (each with its optional `gml:id`); a zero-face surface is an
/// empty `<bldg:{ty}/>`. Shared by the solid (xlinked ids) and MultiSurface
/// (no ids) paths.
fn write_boundedby_surface<W: Write>(
    w: &mut Writer<W>,
    coords: &[[f64; 3]],
    ty: &str,
    major: u8,
    polys: &[(&Face, Option<&str>)],
) -> Result<()> {
    w.write_event(Event::Start(BytesStart::new("bldg:boundedBy")))
        .map_err(io_err)?;
    let tag = format!("bldg:{ty}");
    if polys.is_empty() {
        w.write_event(Event::Empty(BytesStart::new(tag.as_str())))
            .map_err(io_err)?;
    } else {
        w.write_event(Event::Start(BytesStart::new(tag.as_str())))
            .map_err(io_err)?;
        let ms = format!("bldg:lod{major}MultiSurface");
        w.write_event(Event::Start(BytesStart::new(ms.as_str())))
            .map_err(io_err)?;
        for (face, id) in polys {
            write_inline_member(w, coords, face, *id)?;
        }
        w.write_event(Event::End(BytesEnd::new(ms.as_str())))
            .map_err(io_err)?;
        w.write_event(Event::End(BytesEnd::new(tag.as_str())))
            .map_err(io_err)?;
    }
    w.write_event(Event::End(BytesEnd::new("bldg:boundedBy")))
        .map_err(io_err)?;
    Ok(())
}

/// Emit the `bldg:boundedBy` block for a solid: one element per `surface_types`
/// entry, in array order, each holding the inline (id-stamped) polygons of the
/// faces assigned to it.
fn write_boundedby<W: Write>(
    w: &mut Writer<W>,
    coords: &[[f64; 3]],
    surface_types: &[String],
    members: &[Shells],
    surfaces: &[FaceSurfaces],
    face_ids: &[Vec<Option<String>>],
    major: u8,
) -> Result<()> {
    for (i, ty) in surface_types.iter().enumerate() {
        let mut polys: Vec<(&Face, Option<&str>)> = Vec::new();
        for (m, shells) in members.iter().enumerate() {
            let mut fi = 0usize;
            for shell in shells {
                for face in shell {
                    if surfaces[m][fi] == Some(i) {
                        let id = face_ids[m][fi].as_deref().expect("non-null face has an id");
                        polys.push((face, Some(id)));
                    }
                    fi += 1;
                }
            }
        }
        write_boundedby_surface(w, coords, ty, major, &polys)?;
    }
    Ok(())
}

/// Emit a Solid/CompositeSolid with semantics: `bldg:lod<major>Solid` whose
/// faces are xlink references (semantic faces) or inline (null faces), followed
/// by the `bldg:boundedBy` surfaces holding the inline geometry. Errors bubble
/// up so the caller can fall back to geometry-only.
pub fn write_solid_with_semantics<W: Write>(
    w: &mut Writer<W>,
    coords: &[[f64; 3]],
    kind: &DecodedKind,
    props: Option<&Value>,
    sem: &Semantics,
    ids: &mut IdAlloc,
    major: u8,
) -> Result<()> {
    let nsurf = sem.surfaces.len();

    // 1. Partition into members -> shells -> faces + per-face surface index.
    let (members, surfaces, composite): (Vec<Shells>, Vec<FaceSurfaces>, bool) = match kind {
        DecodedKind::PolyhedralSurface(faces) => {
            let counts = crate::export::shell_faces_flat(props)?;
            let shells = crate::export::partition_shells(faces.clone(), counts.as_deref())?;
            let flat = solid_face_surfaces(&sem.values, &shells, nsurf)?;
            (vec![shells], vec![flat], false)
        }
        DecodedKind::GeometryCollection(gc) => {
            let nested = crate::export::shell_faces_nested(props)?;
            if let Some(c) = &nested
                && c.len() != gc.len()
            {
                return Err(err(format!(
                    "solid_shell_faces lists {} solids but the CompositeSolid has {}",
                    c.len(),
                    gc.len()
                )));
            }
            let mut mshells = Vec::with_capacity(gc.len());
            for (m, member) in gc.iter().enumerate() {
                let DecodedKind::PolyhedralSurface(faces) = member else {
                    return Err(err("CompositeSolid member is not a PolyhedralSurface"));
                };
                let counts = nested.as_ref().map(|c| c[m].as_slice());
                mshells.push(crate::export::partition_shells(faces.clone(), counts)?);
            }
            let surfaces = composite_face_surfaces(&sem.values, &mshells, nsurf)?;
            (mshells, surfaces, true)
        }
        _ => {
            return Err(err(
                "semantics solid path needs a PolyhedralSurface/GeometryCollection",
            ));
        }
    };

    // 2. Allocate a gml:id for every non-null face, aligned to `surfaces`.
    let face_ids: Vec<Vec<Option<String>>> = surfaces
        .iter()
        .map(|msurf| msurf.iter().map(|s| s.map(|_| ids.alloc())).collect())
        .collect();

    // 3. Emit bldg:lod<major>Solid (xlink/inline members).
    let elem = format!("bldg:lod{major}Solid");
    w.write_event(Event::Start(BytesStart::new(elem.as_str())))
        .map_err(io_err)?;
    if composite {
        w.write_event(Event::Start(BytesStart::new("gml:CompositeSolid")))
            .map_err(io_err)?;
        for (m, shells) in members.iter().enumerate() {
            w.write_event(Event::Start(BytesStart::new("gml:solidMember")))
                .map_err(io_err)?;
            write_one_solid(w, coords, shells, &face_ids[m])?;
            w.write_event(Event::End(BytesEnd::new("gml:solidMember")))
                .map_err(io_err)?;
        }
        w.write_event(Event::End(BytesEnd::new("gml:CompositeSolid")))
            .map_err(io_err)?;
    } else {
        write_one_solid(w, coords, &members[0], &face_ids[0])?;
    }
    w.write_event(Event::End(BytesEnd::new(elem.as_str())))
        .map_err(io_err)?;

    // 4. Emit bldg:boundedBy per surface.
    write_boundedby(
        w,
        coords,
        &sem.surfaces,
        &members,
        &surfaces,
        &face_ids,
        major,
    )?;
    Ok(())
}

/// Emit a semantics-bearing MultiSurface (no solid): one `bldg:boundedBy` per
/// surface, with the surface's faces as inline `gml:Polygon`s. A `null`-value
/// face has no CityGML home here (the reader's MultiSurface path never produces
/// a null value) and is dropped with a counter.
pub fn write_multisurface_with_semantics<W: Write>(
    w: &mut Writer<W>,
    coords: &[[f64; 3]],
    faces: &[Face],
    sem: &Semantics,
    major: u8,
    report: &mut WriteReport,
) -> Result<()> {
    let nsurf = sem.surfaces.len();
    let surfaces = multisurface_face_surfaces(&sem.values, faces.len(), nsurf)?;
    report.multisurface_null_faces_dropped += surfaces.iter().filter(|s| s.is_none()).count();
    for (i, ty) in sem.surfaces.iter().enumerate() {
        let polys: Vec<(&Face, Option<&str>)> = faces
            .iter()
            .zip(&surfaces)
            .filter(|(_, s)| **s == Some(i))
            .map(|(f, _)| (f, None))
            .collect();
        write_boundedby_surface(w, coords, ty, major, &polys)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn f() -> Face {
        vec![vec![0, 1, 2]]
    }

    #[test]
    fn parse_extracts_surfaces_and_values() {
        let props = json!({"type":"Solid","semantics":{
            "surfaces":[{"type":"WallSurface"},{"type":"RoofSurface"}],
            "values":[[0,1]]}});
        let s = parse_semantics(Some(&props)).unwrap();
        assert_eq!(s.surfaces, vec!["WallSurface", "RoofSurface"]);
        assert!(surfaces_emittable(&s));
    }

    #[test]
    fn parse_none_when_absent() {
        assert!(parse_semantics(Some(&json!({"type":"Solid"}))).is_none());
        assert!(parse_semantics(None).is_none());
    }

    #[test]
    fn extension_type_is_not_emittable() {
        let s = Semantics {
            surfaces: vec!["+Custom".into()],
            values: json!([[0]]),
        };
        assert!(!surfaces_emittable(&s));
    }

    #[test]
    fn solid_values_map_faces_to_surfaces_with_null() {
        let shells = vec![vec![f(), f()], vec![f()]];
        let out = solid_face_surfaces(&json!([[0, null], [1]]), &shells, 2).unwrap();
        assert_eq!(out, vec![Some(0), None, Some(1)]);
    }

    #[test]
    fn solid_nesting_mismatch_errors() {
        let shells = vec![vec![f(), f()]];
        assert!(solid_face_surfaces(&json!([[0]]), &shells, 2).is_err()); // 1 value, 2 faces
    }

    #[test]
    fn solid_index_out_of_range_errors() {
        let shells = vec![vec![f()]];
        assert!(solid_face_surfaces(&json!([[5]]), &shells, 2).is_err());
    }

    #[test]
    fn composite_values_per_member() {
        let m = vec![vec![f()]];
        let members = vec![m.clone(), m];
        let out = composite_face_surfaces(&json!([[[0]], [[null]]]), &members, 1).unwrap();
        assert_eq!(out, vec![vec![Some(0)], vec![None]]);
    }

    #[test]
    fn multisurface_flat_values() {
        let out = multisurface_face_surfaces(&json!([0, 1, 0]), 3, 2).unwrap();
        assert_eq!(out, vec![Some(0), Some(1), Some(0)]);
    }

    #[test]
    fn multisurface_length_mismatch_errors() {
        assert!(multisurface_face_surfaces(&json!([0, 1]), 3, 2).is_err());
    }

    #[test]
    fn id_alloc_is_unique_and_avoids_seen() {
        let mut seen = HashSet::from(["_cpq_p0".to_string()]);
        let mut a = IdAlloc::new(&mut seen);
        assert_eq!(a.alloc(), "_cpq_p1"); // p0 already taken
        assert_eq!(a.alloc(), "_cpq_p2");
        assert!(seen.contains("_cpq_p1") && seen.contains("_cpq_p2"));
    }

    use crate::wkb_read::DecodedKind;

    fn tri(a: usize, b: usize, c: usize) -> DecodedKind {
        // one triangular face; coords are irrelevant to the structural asserts.
        DecodedKind::PolyhedralSurface(vec![vec![vec![a, b, c]]])
    }

    fn semantic_coords() -> Vec<[f64; 3]> {
        (0..9).map(|i| [i as f64, 0.0, 0.0]).collect()
    }

    fn emit_solid_sem(
        coords: &[[f64; 3]],
        kind: &DecodedKind,
        props: &Value,
        sem: &Semantics,
    ) -> String {
        let mut seen = HashSet::new();
        let mut ids = IdAlloc::new(&mut seen);
        let mut w = Writer::new(Vec::new());
        write_solid_with_semantics(&mut w, coords, kind, Some(props), sem, &mut ids, 2).unwrap();
        String::from_utf8(w.into_inner()).unwrap()
    }

    #[test]
    fn solid_semantics_emits_xlinked_solid_and_boundedby() {
        // 1 shell, 3 faces; values [[0, null, 1]]; surfaces [Wall, Roof].
        let coords = semantic_coords();
        let kind = DecodedKind::PolyhedralSurface(vec![
            vec![vec![0, 1, 2]],
            vec![vec![3, 4, 5]],
            vec![vec![6, 7, 8]],
        ]);
        let props = json!({ "type": "Solid", "solid_shell_faces": [3] });
        let sem = Semantics {
            surfaces: vec!["WallSurface".into(), "RoofSurface".into()],
            values: json!([[0, null, 1]]),
        };
        let xml = emit_solid_sem(&coords, &kind, &props, &sem);
        // Solid comes first, with xlink members for faces 0 & 2 and inline for face 1.
        assert!(xml.starts_with("<bldg:lod2Solid><gml:Solid>"), "{xml}");
        assert_eq!(
            xml.matches("<gml:surfaceMember xlink:href=").count(),
            2,
            "{xml}"
        );
        // The null face (index 1) is inline in the solid.
        let solid_end = xml.find("</bldg:lod2Solid>").unwrap();
        assert!(
            xml[..solid_end].contains("<gml:surfaceMember><gml:Polygon>"),
            "inline null face: {xml}"
        );
        // boundedBy: Wall then Roof, each xlink id matches a gml:Polygon gml:id.
        assert!(xml.contains("<bldg:boundedBy><bldg:WallSurface>"), "{xml}");
        assert!(xml.contains("<bldg:boundedBy><bldg:RoofSurface>"), "{xml}");
        assert!(xml.contains("xlink:href=\"#_cpq_p0\""), "{xml}");
        assert!(xml.contains("<gml:Polygon gml:id=\"_cpq_p0\">"), "{xml}");
        assert!(xml.contains("xlink:href=\"#_cpq_p1\""), "{xml}");
        assert!(xml.contains("<gml:Polygon gml:id=\"_cpq_p1\">"), "{xml}");
        // Wall precedes Roof (surfaces array order).
        assert!(xml.find("WallSurface").unwrap() < xml.find("RoofSurface").unwrap());
    }

    #[test]
    fn zero_face_surface_is_emitted_empty() {
        // 1 face -> surface 0; surface 1 (Roof) has no faces.
        let coords = semantic_coords();
        let kind = tri(0, 1, 2);
        let props = json!({ "type": "Solid", "solid_shell_faces": [1] });
        let sem = Semantics {
            surfaces: vec!["WallSurface".into(), "RoofSurface".into()],
            values: json!([[0]]),
        };
        let xml = emit_solid_sem(&coords, &kind, &props, &sem);
        assert!(
            xml.contains("<bldg:boundedBy><bldg:RoofSurface/></bldg:boundedBy>"),
            "{xml}"
        );
    }

    fn emit_ms_sem(coords: &[[f64; 3]], faces: &[Face], sem: &Semantics) -> (String, WriteReport) {
        let mut report = WriteReport::default();
        let mut w = Writer::new(Vec::new());
        write_multisurface_with_semantics(&mut w, coords, faces, sem, 3, &mut report).unwrap();
        (String::from_utf8(w.into_inner()).unwrap(), report)
    }

    #[test]
    fn multisurface_semantics_groups_faces_by_surface_no_solid() {
        let coords = semantic_coords();
        let faces = vec![
            vec![vec![0, 1, 2]],
            vec![vec![3, 4, 5]],
            vec![vec![6, 7, 8]],
        ];
        let sem = Semantics {
            surfaces: vec!["WallSurface".into(), "RoofSurface".into()],
            values: json!([0, 0, 1]),
        };
        let (xml, r) = emit_ms_sem(&coords, &faces, &sem);
        assert!(
            !xml.contains("<gml:Solid>"),
            "no solid in the MultiSurface case: {xml}"
        );
        assert!(xml.contains("<bldg:boundedBy><bldg:WallSurface>"), "{xml}");
        // Wall has 2 polygons, Roof 1.
        let wall = xml.find("WallSurface").unwrap();
        let roof = xml.find("RoofSurface").unwrap();
        assert_eq!(xml[wall..roof].matches("<gml:Polygon>").count(), 2, "{xml}");
        assert_eq!(r.multisurface_null_faces_dropped, 0);
    }

    #[test]
    fn multisurface_null_face_is_dropped_and_counted() {
        let coords = semantic_coords();
        let faces = vec![vec![vec![0, 1, 2]], vec![vec![3, 4, 5]]];
        let sem = Semantics {
            surfaces: vec!["WallSurface".into()],
            values: json!([0, null]),
        };
        let (_xml, r) = emit_ms_sem(&coords, &faces, &sem);
        assert_eq!(r.multisurface_null_faces_dropped, 1);
    }

    #[test]
    fn multisurface_zero_face_surface_is_empty() {
        let coords = semantic_coords();
        let faces = vec![vec![vec![0, 1, 2]]];
        let sem = Semantics {
            surfaces: vec!["WallSurface".into(), "RoofSurface".into()],
            values: json!([0]),
        };
        let (xml, _) = emit_ms_sem(&coords, &faces, &sem);
        assert!(
            xml.contains("<bldg:boundedBy><bldg:RoofSurface/></bldg:boundedBy>"),
            "{xml}"
        );
    }

    #[test]
    fn composite_solid_semantics_spans_members() {
        // 2 members, each 1 shell of 1 face; values [[[0]], [[1]]].
        let coords = semantic_coords();
        let kind = DecodedKind::GeometryCollection(vec![
            DecodedKind::PolyhedralSurface(vec![vec![vec![0, 1, 2]]]),
            DecodedKind::PolyhedralSurface(vec![vec![vec![3, 4, 5]]]),
        ]);
        let props = json!({ "type": "CompositeSolid", "solid_shell_faces": [[1], [1]] });
        let sem = Semantics {
            surfaces: vec!["WallSurface".into(), "RoofSurface".into()],
            values: json!([[[0]], [[1]]]),
        };
        let xml = emit_solid_sem(&coords, &kind, &props, &sem);
        assert!(xml.contains("<gml:CompositeSolid>"), "{xml}");
        assert_eq!(xml.matches("<gml:solidMember>").count(), 2, "{xml}");
        assert!(
            xml.contains("<bldg:WallSurface>") && xml.contains("<bldg:RoofSurface>"),
            "{xml}"
        );
    }
}
