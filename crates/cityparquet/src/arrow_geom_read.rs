//! Arrow `geometry_lod*`/`geometry_vertices_lod*` columns -> `DecodedGeometry`,
//! the inverse of `arrow_geom_write::ArrowGeomBuilders`. The row's
//! `geometry_properties.type_name` decides how to interpret/strip the
//! physical shape's padding dimensions (design doc "Critical invariant" —
//! never infer the CM type from nesting depth).
//!
//! Nesting reminder (matches `arrow_geom_write`'s five `List` levels,
//! outermost first): a `geometry_lod*` row is a list of **solids**, each
//! solid a list of **shells**, each shell a list of **faces**, each face a
//! list of **rings**, each ring a flat `Int32Array` of vertex-pool indices.
//! `ListArray::value(i)` peels exactly one of those levels at a time, so
//! every helper below is written for one specific level's array type —
//! passing it an array from the wrong level either panics on a bad downcast
//! or silently reads the wrong indices, which is exactly the bug this
//! module's tests are written to catch.

// `decode_row` and its helpers are only exercised by this module's own tests
// until Task 8 wires them into `decode.rs` — mirrors `arrow_geom_write.rs`'s
// own `#![allow(dead_code)]` between Task 4 and Task 6.
#![allow(dead_code)]

use arrow_array::{Array, Float64Array, Int32Array, ListArray, StructArray};
use cityparquet_schema::{CityParquetError, Result};

use crate::wkb_read::{DecodedGeometry, DecodedKind};

/// This row's vertex pool: the `geometry_vertices_lod*` column's `row`-th
/// entry, a `Struct<x,y,z>` list (`arrow_native_vertices_data_type`).
fn read_vertices(vertices: &ListArray, row: usize) -> Vec<[f64; 3]> {
    let row_values = vertices.value(row);
    let structs = row_values
        .as_any()
        .downcast_ref::<StructArray>()
        .expect("geometry_vertices_lod* item type is Struct<x,y,z>");
    let x = structs
        .column(0)
        .as_any()
        .downcast_ref::<Float64Array>()
        .expect("vertex struct field 0 is x: Float64");
    let y = structs
        .column(1)
        .as_any()
        .downcast_ref::<Float64Array>()
        .expect("vertex struct field 1 is y: Float64");
    let z = structs
        .column(2)
        .as_any()
        .downcast_ref::<Float64Array>()
        .expect("vertex struct field 2 is z: Float64");
    (0..structs.len())
        .map(|i| [x.value(i), y.value(i), z.value(i)])
        .collect()
}

/// One ring: `ring_array` is the RING-level array (its `value(i)` is the
/// leaf `Int32Array` of vertex-pool indices directly — the innermost list,
/// one level up from `Int32`).
fn read_ring(ring_array: &ListArray, i: usize) -> Vec<usize> {
    let row = ring_array.value(i);
    let ints = row
        .as_any()
        .downcast_ref::<Int32Array>()
        .expect("ring level's item type is Int32 (vertex-pool index)");
    (0..ints.len()).map(|j| ints.value(j) as usize).collect()
}

/// One face's rings: `face_array` is the FACE-level array (its `value(i)`
/// is the RING-level array for that one face).
fn read_face(face_array: &ListArray, i: usize) -> Vec<Vec<usize>> {
    let row = face_array.value(i);
    let ring_array = row
        .as_any()
        .downcast_ref::<ListArray>()
        .expect("face level's item type is List<Int32> (a ring)");
    (0..ring_array.len())
        .map(|j| read_ring(ring_array, j))
        .collect()
}

/// One shell's faces: `shell_array` is the SHELL-level array (its
/// `value(i)` is the FACE-level array for that one shell).
fn read_shell(shell_array: &ListArray, i: usize) -> Vec<Vec<Vec<usize>>> {
    let row = shell_array.value(i);
    let face_array = row
        .as_any()
        .downcast_ref::<ListArray>()
        .expect("shell level's item type is List<List<Int32>> (a face)");
    (0..face_array.len())
        .map(|j| read_face(face_array, j))
        .collect()
}

/// This solid's shells: `solid_array` is the SOLID-level array (its
/// `value(i)` is the SHELL-level array for that one solid) — one level up
/// from [`read_shell`]'s input.
fn shells_of(solid_array: &ListArray, i: usize) -> ListArray {
    let row = solid_array.value(i);
    row.as_any()
        .downcast_ref::<ListArray>()
        .expect("solid level's item type is List<List<List<Int32>>> (a shell)")
        .clone()
}

/// Flattens every face across every shell of one solid — mirrors
/// `wkb_write`/`geometry_to_compacted`'s own Solid handling, where shell
/// grouping carries no meaning in the geometry column itself (it survives
/// only in `geometry_properties.shells`). In practice `ArrowGeomBuilders`
/// always pads a `Solid`'s shell-count to 1 (its own already-flattened
/// `PolyhedralSurface` face list becomes that single shell's faces), but
/// this loop does not assume that — it walks every shell the physical
/// column actually reports, so a differently-encoded input with a genuine
/// multi-shell physical layout still decodes correctly.
fn flatten_shells(shell_array: &ListArray) -> Vec<Vec<Vec<usize>>> {
    let mut faces = Vec::new();
    for shell_idx in 0..shell_array.len() {
        faces.extend(read_shell(shell_array, shell_idx));
    }
    faces
}

/// Reads row `row` of the `geometry_lod*`/`geometry_vertices_lod*` column
/// pair back into a [`DecodedGeometry`], dispatching on
/// `geometry_properties.type_name` to decide how to interpret/strip the
/// physical shape's padding dimensions — never inferred from nesting depth
/// (design doc "Critical invariant").
pub(crate) fn decode_row(
    geometry: &ListArray,
    vertices: &ListArray,
    row: usize,
    type_name: &str,
) -> Result<DecodedGeometry> {
    let coords = read_vertices(vertices, row);

    let solids_row = geometry.value(row);
    let solids = solids_row
        .as_any()
        .downcast_ref::<ListArray>()
        .expect("geometry_lod* row item type is List<List<List<List<Int32>>>> (a solid)"); // SOLID-level array: solids of this row

    let kind = match type_name {
        "MultiSurface" | "CompositeSurface" => {
            // Padded to solid-count=1/shell-count=1 — strip both, recovering
            // the original MultiPolygon's faces directly from that one shell.
            debug_assert_eq!(
                solids.len(),
                1,
                "MultiSurface/CompositeSurface must be padded to solid-count 1"
            );
            let shells = shells_of(solids, 0);
            debug_assert_eq!(
                shells.len(),
                1,
                "MultiSurface/CompositeSurface must be padded to shell-count 1"
            );
            let faces = read_shell(&shells, 0);
            DecodedKind::MultiPolygon(faces)
        }
        "Solid" => {
            debug_assert_eq!(solids.len(), 1, "Solid must be padded to solid-count 1");
            let shells = shells_of(solids, 0);
            DecodedKind::PolyhedralSurface(flatten_shells(&shells))
        }
        "MultiSolid" | "CompositeSolid" => {
            let mut members = Vec::with_capacity(solids.len());
            for solid_idx in 0..solids.len() {
                let shells = shells_of(solids, solid_idx);
                members.push(DecodedKind::PolyhedralSurface(flatten_shells(&shells)));
            }
            DecodedKind::GeometryCollection(members)
        }
        other => {
            return Err(CityParquetError::Geometry(format!(
                "arrow-native decode: unsupported geometry_properties.type {other:?} \
                 (phase-1 scope: MultiSurface/CompositeSurface/Solid/MultiSolid/CompositeSolid)"
            )));
        }
    };
    Ok(DecodedGeometry { coords, kind })
}

#[cfg(test)]
mod tests {
    use crate::arrow_geom_write::ArrowGeomBuilders; // test-only cross-module use is fine within one crate
    use crate::wkb_read::{DecodedGeometry, DecodedKind};

    #[test]
    fn decode_row_inverts_arrow_geom_builders_for_a_solid() {
        let decoded_in = DecodedGeometry {
            coords: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            kind: DecodedKind::PolyhedralSurface(vec![vec![vec![0, 1, 2]]]),
        };
        let mut b = ArrowGeomBuilders::new();
        b.append_value(&decoded_in);
        let (geometry, vertices) = b.finish();
        let geometry = geometry
            .as_any()
            .downcast_ref::<arrow_array::ListArray>()
            .unwrap();
        let vertices = vertices
            .as_any()
            .downcast_ref::<arrow_array::ListArray>()
            .unwrap();

        let decoded_out = super::decode_row(geometry, vertices, 0, "Solid").unwrap();
        assert_eq!(decoded_out, decoded_in);
    }

    #[test]
    fn decode_row_strips_padding_dimensions_for_multisurface() {
        let decoded_in = DecodedGeometry {
            coords: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            kind: DecodedKind::MultiPolygon(vec![vec![vec![0, 1, 2]]]),
        };
        let mut b = ArrowGeomBuilders::new();
        b.append_value(&decoded_in);
        let (geometry, vertices) = b.finish();
        let geometry = geometry
            .as_any()
            .downcast_ref::<arrow_array::ListArray>()
            .unwrap();
        let vertices = vertices
            .as_any()
            .downcast_ref::<arrow_array::ListArray>()
            .unwrap();

        let decoded_out = super::decode_row(geometry, vertices, 0, "MultiSurface").unwrap();
        assert_eq!(
            decoded_out, decoded_in,
            "type_name=\"MultiSurface\" must strip the 2 padding dimensions ArrowGeomBuilders added, recovering the original MultiPolygon shape"
        );
    }

    /// The real risk the brief flagged: a `Solid` written by Task 4/5 can
    /// have faces from MULTIPLE shells (exterior + interior cavities)
    /// flattened into one `PolyhedralSurface`'s face list — mirrors Task 4's
    /// own `solid_two_shells_flattens_faces_like_wkb_and_reports_no_shell_distinction`
    /// fixture. `decode_row`'s `"Solid"` branch must recover every face, not
    /// just the first shell's.
    #[test]
    fn decode_row_flattens_a_two_shell_solid_correctly() {
        let decoded_in = DecodedGeometry {
            coords: vec![
                [0., 0., 0.],
                [1., 0., 0.],
                [0., 1., 0.],
                [0., 0., 1.],
                [1., 0., 1.],
                [0., 1., 1.],
            ],
            kind: DecodedKind::PolyhedralSurface(vec![vec![vec![0, 1, 2]], vec![vec![3, 4, 5]]]), // 2 faces, no shell tag (WKB-flattened shape, matching Task 4's own Solid test)
        };
        let mut b = ArrowGeomBuilders::new();
        b.append_value(&decoded_in);
        let (geometry, vertices) = b.finish();
        let geometry = geometry
            .as_any()
            .downcast_ref::<arrow_array::ListArray>()
            .unwrap();
        let vertices = vertices
            .as_any()
            .downcast_ref::<arrow_array::ListArray>()
            .unwrap();
        let decoded_out = super::decode_row(geometry, vertices, 0, "Solid").unwrap();
        assert_eq!(decoded_out, decoded_in);
    }

    /// Codex review finding (Important): the test above round-trips through
    /// `ArrowGeomBuilders`, but `push_padded_solid` (arrow_geom_write.rs)
    /// always writes exactly ONE physical shell — Task 4's
    /// `geometry_to_compacted` has already flattened a real `Solid`'s
    /// multiple shells into one face list before `ArrowGeomBuilders` ever
    /// sees it. So that test can never exercise `flatten_shells`'s loop over
    /// more than one shell; an implementation that silently dropped every
    /// shell after the first would still pass it.
    ///
    /// This test instead hand-builds the raw 5-level Arrow arrays directly
    /// (the same `OffsetBuffer`/`ListArray::new` primitives
    /// `ArrowGeomBuilders::finish` uses), bypassing the builder entirely, so
    /// the SHELL-level array genuinely has `len() == 2` — a shape the schema
    /// permits even though today's writer never produces it. `decode_row`
    /// must still recover faces from both shells, because a reader has to
    /// handle whatever the declared column type allows, not only whatever
    /// today's writer happens to emit.
    #[test]
    fn decode_row_flattens_multiple_physical_shells_not_just_the_first() {
        use arrow_array::{ArrayRef, Float64Array, Int32Array, ListArray, StructArray};
        use arrow_buffer::{NullBuffer, OffsetBuffer};
        use arrow_schema::{DataType, FieldRef};
        use std::sync::Arc;

        /// Peels the five `List` item fields off the declared schema type
        /// itself (mirrors `arrow_geom_write::geometry_item_fields`, which
        /// is private to that module) so this hand-built array cannot drift
        /// from the real column type.
        fn geometry_item_fields() -> [FieldRef; 5] {
            let mut data_type = cityparquet_schema::model::arrow_native_geometry_data_type();
            let mut items = Vec::with_capacity(5);
            for _ in 0..5 {
                match data_type {
                    DataType::List(item) => {
                        data_type = item.data_type().clone();
                        items.push(item);
                    }
                    other => panic!(
                        "arrow_native_geometry_data_type must be 5 nested Lists, got {other:?}"
                    ),
                }
            }
            items
                .try_into()
                .expect("the loop above pushes exactly 5 items")
        }

        fn vertices_item_field() -> FieldRef {
            match cityparquet_schema::model::arrow_native_vertices_data_type() {
                DataType::List(item) => item,
                other => panic!("arrow_native_vertices_data_type must be a List, got {other:?}"),
            }
        }

        let [
            solid_field,
            shell_field,
            face_field,
            ring_field,
            index_field,
        ] = geometry_item_fields();

        // One row, one solid, TWO physical shells: shell 0 -> face [0,1,2],
        // shell 1 -> face [3,4,5]. Built bottom-up, exactly like
        // `ArrowGeomBuilders::finish`, but with `shell_lengths = [1, 1]`
        // (two shell-level list entries) instead of the builder's always-one.
        let indices: ArrayRef = Arc::new(Int32Array::from(vec![0, 1, 2, 3, 4, 5]));
        let rings = ListArray::new(
            index_field,
            OffsetBuffer::from_lengths([3usize, 3]), // 2 rings, 3 indices each
            indices,
            None,
        );
        let faces = ListArray::new(
            ring_field,
            OffsetBuffer::from_lengths([1usize, 1]), // 2 faces, 1 ring each
            Arc::new(rings),
            None,
        );
        let shells = ListArray::new(
            face_field,
            OffsetBuffer::from_lengths([1usize, 1]), // 2 SHELLS, 1 face each — the case under test
            Arc::new(faces),
            None,
        );
        let solids = ListArray::new(
            shell_field,
            OffsetBuffer::from_lengths([2usize]), // 1 solid, spanning both shells
            Arc::new(shells),
            None,
        );
        let geometry = ListArray::new(
            solid_field,
            OffsetBuffer::from_lengths([1usize]), // 1 row, 1 solid
            Arc::new(solids),
            Some(NullBuffer::from(vec![true])),
        );

        // Matching 6-entry vertex pool for the 6 distinct indices used above.
        let xs = Float64Array::from(vec![0.0, 1.0, 0.0, 0.0, 1.0, 0.0]);
        let ys = Float64Array::from(vec![0.0, 0.0, 1.0, 0.0, 0.0, 1.0]);
        let zs = Float64Array::from(vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0]);
        let coord_fields = match vertices_item_field().data_type() {
            DataType::Struct(fields) => fields.clone(),
            other => panic!("expected Struct, got {other:?}"),
        };
        let coords = StructArray::new(
            coord_fields,
            vec![Arc::new(xs), Arc::new(ys), Arc::new(zs)],
            None,
        );
        let vertices = ListArray::new(
            vertices_item_field(),
            OffsetBuffer::from_lengths([6usize]),
            Arc::new(coords),
            Some(NullBuffer::from(vec![true])),
        );

        let decoded = super::decode_row(&geometry, &vertices, 0, "Solid").unwrap();
        match decoded.kind {
            DecodedKind::PolyhedralSurface(faces) => {
                assert_eq!(
                    faces,
                    vec![vec![vec![0, 1, 2]], vec![vec![3, 4, 5]]],
                    "must include faces from BOTH physical shells, not just shell 0"
                );
            }
            other => panic!("expected PolyhedralSurface, got {other:?}"),
        }
    }

    /// Codex review finding (Minor): `MultiSolid`/`CompositeSolid` dispatch
    /// had no test in this module. Two members with distinct indices and
    /// different face counts (1 face vs. 2 faces) — proves each member keeps
    /// its own indices and its own face list, with no cross-contamination.
    #[test]
    fn decode_row_multisolid_round_trips_with_distinct_members() {
        let decoded_in = DecodedGeometry {
            coords: vec![
                // member 0: a single triangle.
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                // member 1: two triangles sharing an edge.
                [5.0, 5.0, 5.0],
                [6.0, 5.0, 5.0],
                [5.0, 6.0, 5.0],
                [5.0, 5.0, 6.0],
            ],
            kind: DecodedKind::GeometryCollection(vec![
                DecodedKind::PolyhedralSurface(vec![vec![vec![0, 1, 2]]]),
                DecodedKind::PolyhedralSurface(vec![vec![vec![3, 4, 5]], vec![vec![3, 4, 6]]]),
            ]),
        };
        let mut b = ArrowGeomBuilders::new();
        b.append_value(&decoded_in);
        let (geometry, vertices) = b.finish();
        let geometry = geometry
            .as_any()
            .downcast_ref::<arrow_array::ListArray>()
            .unwrap();
        let vertices = vertices
            .as_any()
            .downcast_ref::<arrow_array::ListArray>()
            .unwrap();

        let decoded_out = super::decode_row(geometry, vertices, 0, "MultiSolid").unwrap();
        assert_eq!(
            decoded_out, decoded_in,
            "each member must keep its own indices/face count, no cross-contamination"
        );
    }
}
