//! Parse the flattened `geometry_properties` (§8) and resolve each WKB face to
//! its surface index, in the face-walk order the solid/multisurface emitter
//! uses.
//!
//! `surfaces` is the CityJSON surface array (its `type` strings are taken here);
//! `face_semantics` is already a FLAT per-face list (one entry per WKB face, in
//! WKB order) — a surface index, or `null` for a face with no semantic surface,
//! never CityJSON's nested `values` tree. This module therefore only validates
//! and, for a CompositeSolid, splits that flat list per member.

use std::io::Write;

use cityparquet_schema::CityParquetError;
use quick_xml::Writer;
use quick_xml::events::{BytesEnd, BytesStart, Event};
use serde_json::Value;

use super::WriteReport;
use super::building::is_ncname;
use super::geometry::{FaceIds, write_inline_member, write_inline_member_ids, write_xlink_member};
use crate::Result;
use crate::wkb_read::DecodedKind;

/// A face is rings of coord indices (ring 0 exterior, 1.. holes).
pub type Face = Vec<Vec<usize>>;

/// One solid's shells, each a list of faces (as `partition_shells` returns).
type Shells = Vec<Vec<Face>>;

/// A flat per-face surface index in shell-concatenation order (`None` = null).
type FaceSurfaces = Vec<Option<usize>>;

/// Allocates `gml:id`s for a single building-LoD's emitted polygons:
/// `_cpq_b<feature_index>_l<major>_p<N>`. The per-building `feature_index`
/// (unique per package row) plus the LoD major make the ids document-unique by
/// construction — no shared counter or seen-set is needed. (A CityObject id
/// literally equal to such a string would collide; the distinctive prefix makes
/// this negligible in practice — see the design's known limitations.)
pub struct IdAlloc {
    prefix: String,
    next: usize,
}

impl IdAlloc {
    pub fn new(feature_index: usize, major: u8) -> Self {
        Self {
            prefix: format!("_cpq_b{feature_index}_l{major}_p"),
            next: 0,
        }
    }

    pub fn alloc(&mut self) -> String {
        let id = format!("{}{}", self.prefix, self.next);
        self.next += 1;
        id
    }
}

/// Allocate a face's ids: a polygon id when it needs one (semantic OR material OR
/// any textured ring), and a `<polyid>_r<K>` id per textured ring (in
/// exterior-then-hole order).
fn plan_face_ids(
    semantic: bool,
    material: bool,
    ring_needs: &[bool],
    ids: &mut IdAlloc,
) -> FaceIds {
    let needs_face = semantic || material || ring_needs.iter().any(|&b| b);
    let poly = needs_face.then(|| ids.alloc());
    let rings = ring_needs
        .iter()
        .enumerate()
        .map(|(r, &need)| need.then(|| format!("{}_r{r}", poly.as_deref().unwrap())))
        .collect();
    FaceIds { poly, rings }
}

/// Whether a (possibly nested) `values` array contains at least one non-null
/// (integer) leaf — i.e. at least one face is assigned a semantic surface.
/// Distinguishes a geometry with real semantics from one that merely carries a
/// building-wide surfaces array stamped with all-null values (which the reader
/// attaches to every geometry of a semantic building).
pub fn has_nonnull_value(values: &Value) -> bool {
    match values {
        Value::Number(_) => true,
        Value::Array(a) => a.iter().any(has_nonnull_value),
        _ => false,
    }
}

/// Parsed geometry_properties semantics (§8): the surface type strings and the
/// FLAT `face_semantics` array (`values` holds it verbatim, one entry per WKB
/// face — a surface index or `null`).
pub struct Semantics {
    pub surfaces: Vec<String>,
    pub values: Value,
}

fn err(m: impl Into<String>) -> CityParquetError {
    CityParquetError::Geometry(m.into())
}

/// Extract the surface types and flat `face_semantics` from a geometry's
/// `geometry_properties` (§8), or `None` when there is no semantics
/// (`surfaces`/`face_semantics` absent).
pub fn parse_semantics(props: Option<&Value>) -> Option<Semantics> {
    let props = props?;
    let surfaces = props.get("surfaces")?.as_array()?;
    let surfaces: Vec<String> = surfaces
        .iter()
        .map(|s| {
            s.get("type")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string()
        })
        .collect();
    let values = props.get("face_semantics")?.clone();
    Some(Semantics { surfaces, values })
}

/// Whether every surface type is a legal XML element name (NCName). A CityJSON
/// extension type (`+Foo`) is not, so such a geometry falls back to geometry-only.
pub fn surfaces_emittable(s: &Semantics) -> bool {
    !s.surfaces.is_empty() && s.surfaces.iter().all(|t| is_ncname(t))
}

/// The number of *real* semantic surfaces on a geometry (surfaces present AND
/// at least one non-null value), used to count drops. Returns 0 for a geometry
/// that only carries an all-null (stamped) surfaces array, so dropping such a
/// geometry does not spuriously inflate `semantic_surfaces_dropped`.
pub fn droppable_surface_count(props: Option<&Value>) -> usize {
    match parse_semantics(props) {
        Some(sem) if has_nonnull_value(&sem.values) => sem.surfaces.len(),
        _ => 0,
    }
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

/// Validate the flat `face_semantics` list against `nfaces` and `nsurfaces`,
/// returning it as `FaceSurfaces`. Under G7 the stored form is already flat in
/// WKB face order (§8), so this is a length + range check — no shell walking.
fn flat_face_surfaces(values: &Value, nfaces: usize, nsurfaces: usize) -> Result<FaceSurfaces> {
    let vs = values
        .as_array()
        .ok_or_else(|| err("face_semantics must be an array"))?;
    if vs.len() != nfaces {
        return Err(err(format!(
            "face_semantics has {} entries but the geometry has {nfaces} faces",
            vs.len()
        )));
    }
    vs.iter().map(|v| face_index(v, nsurfaces)).collect()
}

/// Resolve a Solid's flat `face_semantics` to a per-face surface index. The
/// total face count comes from the geometry's `shells` partition.
pub fn solid_face_surfaces(
    values: &Value,
    shells: &[Vec<Face>],
    nsurfaces: usize,
) -> Result<FaceSurfaces> {
    let nfaces: usize = shells.iter().map(Vec::len).sum();
    flat_face_surfaces(values, nfaces, nsurfaces)
}

/// Resolve a CompositeSolid's flat `face_semantics`, split into one per-face
/// vec per member using each member's shell partition — the flat list runs in
/// member-then-shell-then-face order (the same order the WKB flattens).
pub fn composite_face_surfaces(
    values: &Value,
    members: &[Shells],
    nsurfaces: usize,
) -> Result<Vec<FaceSurfaces>> {
    let total: usize = members
        .iter()
        .flat_map(|shells| shells.iter())
        .map(Vec::len)
        .sum();
    let flat = flat_face_surfaces(values, total, nsurfaces)?;
    let mut out = Vec::with_capacity(members.len());
    let mut offset = 0;
    for shells in members {
        let n: usize = shells.iter().map(Vec::len).sum();
        out.push(flat[offset..offset + n].to_vec());
        offset += n;
    }
    Ok(out)
}

/// Resolve a MultiSurface's flat `face_semantics`.
pub fn multisurface_face_surfaces(
    values: &Value,
    nfaces: usize,
    nsurfaces: usize,
) -> Result<Vec<Option<usize>>> {
    flat_face_surfaces(values, nfaces, nsurfaces)
}

/// How one face of a semantic solid is emitted inside the `gml:Solid`:
/// - `Xlink` — a semantic face; the solid emits `<gml:surfaceMember
///   xlink:href="#polyid">` and the polygon (with its `gml:id`s) lives in
///   `bldg:boundedBy`.
/// - `InlineId` — a material/texture-only face (no semantics but appearance
///   targets it); the solid emits the polygon inline WITH its `gml:id`s.
/// - `Plain` — neither; the polygon is emitted inline with no ids.
pub enum FaceEmit {
    Xlink(FaceIds),
    InlineId(FaceIds),
    Plain,
}

/// Emit one `<gml:Solid>` whose shells are `gml:CompositeSurface`s of
/// `surfaceMember`s, one per face per its [`FaceEmit`]. `plan` is the flat
/// per-face list in shell-concatenation order.
fn write_one_solid<W: Write>(
    w: &mut Writer<W>,
    coords: &[[f64; 3]],
    shells: &[Vec<Face>],
    plan: &[FaceEmit],
) -> Result<()> {
    w.write_event(Event::Start(BytesStart::new("gml:Solid")))?;
    let mut fi = 0usize;
    for (si, shell) in shells.iter().enumerate() {
        let boundary = if si == 0 {
            "gml:exterior"
        } else {
            "gml:interior"
        };
        w.write_event(Event::Start(BytesStart::new(boundary)))?;
        w.write_event(Event::Start(BytesStart::new("gml:CompositeSurface")))?;
        for face in shell {
            match &plan[fi] {
                // The polygon (with its ring ids) lives in boundedBy; the solid
                // only references it. Texture coords target the boundedBy rings.
                FaceEmit::Xlink(ids) => {
                    write_xlink_member(w, ids.poly.as_deref().expect("xlink face has an id"))?
                }
                FaceEmit::InlineId(ids) => {
                    write_inline_member_ids(w, coords, face, ids.poly.as_deref(), ids.ring_slice())?
                }
                FaceEmit::Plain => write_inline_member(w, coords, face, None)?,
            }
            fi += 1;
        }
        w.write_event(Event::End(BytesEnd::new("gml:CompositeSurface")))?;
        w.write_event(Event::End(BytesEnd::new(boundary)))?;
    }
    w.write_event(Event::End(BytesEnd::new("gml:Solid")))?;
    Ok(())
}

/// Emit one `<bldg:boundedBy><bldg:{ty}>…` surface holding `polys` as inline
/// `gml:Polygon`s (each with its optional polygon + ring `gml:id`s); a zero-face
/// surface is an empty `<bldg:{ty}/>`. Shared by the solid (xlinked ids) and
/// MultiSurface paths.
fn write_boundedby_surface<W: Write>(
    w: &mut Writer<W>,
    coords: &[[f64; 3]],
    ty: &str,
    major: u8,
    polys: &[(&Face, Option<&FaceIds>)],
) -> Result<()> {
    w.write_event(Event::Start(BytesStart::new("bldg:boundedBy")))?;
    let tag = format!("bldg:{ty}");
    if polys.is_empty() {
        w.write_event(Event::Empty(BytesStart::new(tag.as_str())))?;
    } else {
        w.write_event(Event::Start(BytesStart::new(tag.as_str())))?;
        let ms = format!("bldg:lod{major}MultiSurface");
        w.write_event(Event::Start(BytesStart::new(ms.as_str())))?;
        for (face, ids) in polys {
            let poly_id = ids.and_then(|f| f.poly.as_deref());
            let ring_ids = ids.and_then(FaceIds::ring_slice);
            write_inline_member_ids(w, coords, face, poly_id, ring_ids)?;
        }
        w.write_event(Event::End(BytesEnd::new(ms.as_str())))?;
        w.write_event(Event::End(BytesEnd::new(tag.as_str())))?;
    }
    w.write_event(Event::End(BytesEnd::new("bldg:boundedBy")))?;
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
    plans: &[Vec<FaceEmit>],
    major: u8,
) -> Result<()> {
    for (i, ty) in surface_types.iter().enumerate() {
        let mut polys: Vec<(&Face, Option<&FaceIds>)> = Vec::new();
        for (m, shells) in members.iter().enumerate() {
            let mut fi = 0usize;
            for shell in shells {
                for face in shell {
                    if surfaces[m][fi] == Some(i) {
                        let ids = match &plans[m][fi] {
                            FaceEmit::Xlink(ids) => ids,
                            _ => panic!("semantic face must be an Xlink FaceEmit"),
                        };
                        polys.push((face, Some(ids)));
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
#[allow(clippy::too_many_arguments)]
pub fn write_solid_with_semantics<W: Write>(
    w: &mut Writer<W>,
    coords: &[[f64; 3]],
    kind: &DecodedKind,
    props: Option<&Value>,
    sem: &Semantics,
    ids: &mut IdAlloc,
    major: u8,
    mat_union: &[bool],
    ring_needs: &[Vec<bool>],
) -> Result<Vec<FaceIds>> {
    let nsurf = sem.surfaces.len();

    // 1. Partition into members -> shells -> faces + per-face surface index.
    let (members, surfaces, composite): (Vec<Shells>, Vec<FaceSurfaces>, bool) = match kind {
        DecodedKind::PolyhedralSurface(faces) => {
            // `shells` (when present) has exactly one inner list — the
            // Solid's own (spec: nested one inner list per solid).
            let counts = match crate::export::shell_faces(props)? {
                Some(solids) => Some(crate::export::single_solid_shell(solids)?),
                None => None,
            };
            let shells = crate::export::partition_shells(faces.clone(), counts.as_deref())?;
            let flat = solid_face_surfaces(&sem.values, &shells, nsurf)?;
            (vec![shells], vec![flat], false)
        }
        DecodedKind::GeometryCollection(gc) => {
            let nested = crate::export::shell_faces(props)?;
            if let Some(c) = &nested
                && c.len() != gc.len()
            {
                return Err(err(format!(
                    "shells lists {} solids but the CompositeSolid has {}",
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

    // 2. Plan each face's emission, allocating a polygon gml:id for every SEMANTIC
    // face (Xlink), material-only face, or textured face (InlineId), plus a ring
    // gml:id per textured ring, in the global face-walk order the appearance
    // inputs use. `flat_ids` mirrors that order for the appearance accumulator.
    let mut plans: Vec<Vec<FaceEmit>> = Vec::with_capacity(members.len());
    let mut flat_ids: Vec<FaceIds> = Vec::new();
    let mut flat = 0usize;
    for msurf in &surfaces {
        let mut mp = Vec::with_capacity(msurf.len());
        for s in msurf {
            let fids = plan_face_ids(
                s.is_some(),
                mat_union.get(flat).copied().unwrap_or(false),
                ring_needs.get(flat).map(Vec::as_slice).unwrap_or(&[]),
                ids,
            );
            let entry = if s.is_some() {
                FaceEmit::Xlink(fids.clone())
            } else if fids.poly.is_some() {
                FaceEmit::InlineId(fids.clone())
            } else {
                FaceEmit::Plain
            };
            flat_ids.push(fids);
            mp.push(entry);
            flat += 1;
        }
        plans.push(mp);
    }

    // 3. Emit bldg:lod<major>Solid (xlink/inline members).
    let elem = format!("bldg:lod{major}Solid");
    w.write_event(Event::Start(BytesStart::new(elem.as_str())))?;
    if composite {
        w.write_event(Event::Start(BytesStart::new("gml:CompositeSolid")))?;
        for (m, shells) in members.iter().enumerate() {
            w.write_event(Event::Start(BytesStart::new("gml:solidMember")))?;
            write_one_solid(w, coords, shells, &plans[m])?;
            w.write_event(Event::End(BytesEnd::new("gml:solidMember")))?;
        }
        w.write_event(Event::End(BytesEnd::new("gml:CompositeSolid")))?;
    } else {
        write_one_solid(w, coords, &members[0], &plans[0])?;
    }
    w.write_event(Event::End(BytesEnd::new(elem.as_str())))?;

    // 4. Emit bldg:boundedBy per surface.
    write_boundedby(w, coords, &sem.surfaces, &members, &surfaces, &plans, major)?;
    Ok(flat_ids)
}

/// Emit a semantics-bearing MultiSurface (no solid): one `bldg:boundedBy` per
/// surface, with the surface's faces as inline `gml:Polygon`s. A `null`-value
/// face has no CityGML home here (the reader's MultiSurface path never produces
/// a null value) and is dropped with a counter.
#[allow(clippy::too_many_arguments)]
pub fn write_multisurface_with_semantics<W: Write>(
    w: &mut Writer<W>,
    coords: &[[f64; 3]],
    faces: &[Face],
    sem: &Semantics,
    major: u8,
    report: &mut WriteReport,
    ids: &mut IdAlloc,
    mat_union: &[bool],
    ring_needs: &[Vec<bool>],
) -> Result<Vec<FaceIds>> {
    let nsurf = sem.surfaces.len();
    let surfaces = multisurface_face_surfaces(&sem.values, faces.len(), nsurf)?;
    report.multisurface_null_faces_dropped += surfaces.iter().filter(|s| s.is_none()).count();
    // A face's boundedBy polygon gets a polygon gml:id iff it is emitted (has a
    // semantic surface) AND appearance (material/texture) targets it; textured
    // rings get ring ids. Allocate in face order so `flat_ids` aligns with the
    // appearance `values` leaf order.
    let flat_ids: Vec<FaceIds> = (0..faces.len())
        .map(|k| {
            if surfaces[k].is_none() {
                // A dropped (null-value) face: no home, no ids.
                return FaceIds::default();
            }
            plan_face_ids(
                false,
                mat_union.get(k).copied().unwrap_or(false),
                ring_needs.get(k).map(Vec::as_slice).unwrap_or(&[]),
                ids,
            )
        })
        .collect();
    for (i, ty) in sem.surfaces.iter().enumerate() {
        let polys: Vec<(&Face, Option<&FaceIds>)> = faces
            .iter()
            .zip(&surfaces)
            .enumerate()
            .filter(|(_, (_, s))| **s == Some(i))
            .map(|(k, (f, _))| (f, Some(&flat_ids[k])))
            .collect();
        write_boundedby_surface(w, coords, ty, major, &polys)?;
    }
    Ok(flat_ids)
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
        let props = json!({"type":"Solid",
            "surfaces":[{"type":"WallSurface"},{"type":"RoofSurface"}],
            "face_semantics":[0,1],"shells":[2]});
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
            values: json!([0]),
        };
        assert!(!surfaces_emittable(&s));
    }

    #[test]
    fn solid_values_map_faces_to_surfaces_with_null() {
        let shells = vec![vec![f(), f()], vec![f()]];
        let out = solid_face_surfaces(&json!([0, null, 1]), &shells, 2).unwrap();
        assert_eq!(out, vec![Some(0), None, Some(1)]);
    }

    #[test]
    fn solid_nesting_mismatch_errors() {
        let shells = vec![vec![f(), f()]];
        assert!(solid_face_surfaces(&json!([0]), &shells, 2).is_err()); // 1 value, 2 faces
    }

    #[test]
    fn solid_index_out_of_range_errors() {
        let shells = vec![vec![f()]];
        assert!(solid_face_surfaces(&json!([5]), &shells, 2).is_err());
    }

    #[test]
    fn composite_values_per_member() {
        let m = vec![vec![f()]];
        let members = vec![m.clone(), m];
        let out = composite_face_surfaces(&json!([0, null]), &members, 1).unwrap();
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
    fn id_alloc_is_per_building_lod_unique() {
        let mut a = IdAlloc::new(0, 2);
        assert_eq!(a.alloc(), "_cpq_b0_l2_p0");
        assert_eq!(a.alloc(), "_cpq_b0_l2_p1");
        // A different feature index or LoD gives a disjoint id space.
        assert_eq!(IdAlloc::new(7, 2).alloc(), "_cpq_b7_l2_p0");
        assert_eq!(IdAlloc::new(0, 3).alloc(), "_cpq_b0_l3_p0");
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
        let mut ids = IdAlloc::new(0, 2);
        let mut w = Writer::new(Vec::new());
        write_solid_with_semantics(
            &mut w,
            coords,
            kind,
            Some(props),
            sem,
            &mut ids,
            2,
            &[],
            &[],
        )
        .unwrap();
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
        let props = json!({ "type": "Solid", "shells": [[3]] });
        let sem = Semantics {
            surfaces: vec!["WallSurface".into(), "RoofSurface".into()],
            values: json!([0, null, 1]),
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
        assert!(xml.contains("xlink:href=\"#_cpq_b0_l2_p0\""), "{xml}");
        assert!(
            xml.contains("<gml:Polygon gml:id=\"_cpq_b0_l2_p0\">"),
            "{xml}"
        );
        assert!(xml.contains("xlink:href=\"#_cpq_b0_l2_p1\""), "{xml}");
        assert!(
            xml.contains("<gml:Polygon gml:id=\"_cpq_b0_l2_p1\">"),
            "{xml}"
        );
        // Wall precedes Roof (surfaces array order).
        assert!(xml.find("WallSurface").unwrap() < xml.find("RoofSurface").unwrap());
    }

    #[test]
    fn zero_face_surface_is_emitted_empty() {
        // 1 face -> surface 0; surface 1 (Roof) has no faces.
        let coords = semantic_coords();
        let kind = tri(0, 1, 2);
        let props = json!({ "type": "Solid", "shells": [[1]] });
        let sem = Semantics {
            surfaces: vec!["WallSurface".into(), "RoofSurface".into()],
            values: json!([0]),
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
        let mut ids = IdAlloc::new(0, 3);
        write_multisurface_with_semantics(
            &mut w,
            coords,
            faces,
            sem,
            3,
            &mut report,
            &mut ids,
            &[],
            &[],
        )
        .unwrap();
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
        let props = json!({ "type": "CompositeSolid", "shells": [[1], [1]] });
        let sem = Semantics {
            surfaces: vec!["WallSurface".into(), "RoofSurface".into()],
            values: json!([0, 1]),
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
