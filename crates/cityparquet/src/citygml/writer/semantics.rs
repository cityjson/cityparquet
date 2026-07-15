//! Parse `geometry_properties.semantics` and resolve each WKB face to its
//! surface index, in the face-walk order the solid/multisurface emitter uses.
//!
//! `surfaces` is a flat list of surface type strings; `values` maps faces to
//! surface indices, nested by geometry: Solid `[shell][face]`, CompositeSolid
//! `[solid][shell][face]`, MultiSurface flat `[position]`. A `null` value means
//! the face has no semantic surface.

use cityparquet_schema::CityParquetError;
use serde_json::Value;

use super::building::is_ncname;
use crate::Result;

/// A face is rings of coord indices (ring 0 exterior, 1.. holes).
pub type Face = Vec<Vec<usize>>;

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
) -> Result<Vec<Option<usize>>> {
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
    members: &[Vec<Vec<Face>>],
    nsurfaces: usize,
) -> Result<Vec<Vec<Option<usize>>>> {
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
}
