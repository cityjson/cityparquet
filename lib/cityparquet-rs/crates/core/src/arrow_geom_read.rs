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
//! passing it an array from the wrong level would either downcast-fail or
//! silently read the wrong indices, which is exactly the bug this module's
//! tests are written to catch.
//!
//! Every helper below is fallible (`Result`, never `expect`/`unwrap`/a
//! silent-wraparound cast), matching [`crate::wkb_read`]'s standard for
//! malformed input: `decode_row` runs on ANY Parquet file
//! [`crate::decode::decode_batch`] opens for reading/export, not only files
//! this crate itself wrote, so a structurally-valid-but-corrupt or
//! foreign-writer arrow-native column (a wrong-shape padding dimension, an
//! out-of-range or negative vertex-pool index, a downcast that doesn't match
//! the declared level) must surface as a `CityParquetError`, never crash the
//! reader process. WKB has no equivalent risk for the index case — its
//! indices are assigned during decode itself by
//! [`crate::wkb_read::CoordInterner`], so they cannot be out of range by
//! construction — but arrow-native stores its indices on the wire, so a
//! corrupt/hand-rolled file can declare one out of range; left unchecked, it
//! would only surface later as a panic in `crate::export`'s vertex-pool
//! remap (`vmap[index]`).

use arrow_array::{Array, Float64Array, Int32Array, ListArray, StructArray};
use cityparquet_schema::{CityParquetError, Result};

use crate::wkb_read::{DecodedGeometry, DecodedKind};

fn geometry_err(msg: impl Into<String>) -> CityParquetError {
    CityParquetError::Geometry(msg.into())
}

/// `Any`-based downcast with a `CityParquetError` instead of a panic on
/// mismatch — mirrors `crate::decode`'s identical helper. `what` names the
/// expected level/type for the error message (e.g. `"ring level item type
/// Int32 (vertex-pool index)"`).
fn downcast<'a, T: 'static>(array: &'a dyn Array, what: &str) -> Result<&'a T> {
    array.as_any().downcast_ref::<T>().ok_or_else(|| {
        geometry_err(format!(
            "arrow-native geometry decode: expected {what}, found a different array type \
             (corrupt or foreign file)"
        ))
    })
}

/// A raw `Int32` vertex-pool index, checked against `vertex_count` (the
/// row's own `geometry_vertices_lod*` pool size). Uses `usize::try_from`,
/// never a silent `as usize` cast — a negative `i32` would otherwise wrap to
/// a huge `usize` and surface as a confusing out-of-bounds panic far from
/// here (`crate::export`'s vertex-pool remap) instead of a clear error at
/// the point the bad value was actually read.
fn checked_vertex_index(raw: i32, vertex_count: usize) -> Result<usize> {
    let idx = usize::try_from(raw).map_err(|_| {
        geometry_err(format!(
            "arrow-native geometry decode: negative vertex-pool index {raw} (row's vertex \
             pool has {vertex_count} entries)"
        ))
    })?;
    if idx >= vertex_count {
        return Err(geometry_err(format!(
            "arrow-native geometry decode: vertex-pool index {idx} out of range (row's vertex \
             pool has {vertex_count} entries)"
        )));
    }
    Ok(idx)
}

/// This row's vertex pool: the `geometry_vertices_lod*` column's `row`-th
/// entry, a `Struct<x,y,z>` list (`arrow_native_vertices_data_type`).
fn read_vertices(vertices: &ListArray, row: usize) -> Result<Vec<[f64; 3]>> {
    let row_values = vertices.value(row);
    let structs = downcast::<StructArray>(
        row_values.as_ref(),
        "geometry_vertices_lod* item type Struct<x,y,z>",
    )?;
    // `crate::geometry_encoding` verifies the declared column type before any
    // row reaches here, so a three-field struct is guaranteed on every path
    // through `decode_batch`. This check keeps the guarantee local anyway:
    // `column(0..=2)` panics on a shorter struct, and this module's contract
    // (see the module docs) is that malformed input NEVER crashes the reader
    // process, whichever entry point reached it.
    if structs.num_columns() < 3 {
        return Err(geometry_err(format!(
            "arrow-native geometry decode: vertex-pool items must be a Struct<x,y,z> with 3 \
             fields, found {} (corrupt or foreign file)",
            structs.num_columns()
        )));
    }
    let x = downcast::<Float64Array>(
        structs.column(0).as_ref(),
        "vertex struct field 0 (x: Float64)",
    )?;
    let y = downcast::<Float64Array>(
        structs.column(1).as_ref(),
        "vertex struct field 1 (y: Float64)",
    )?;
    let z = downcast::<Float64Array>(
        structs.column(2).as_ref(),
        "vertex struct field 2 (z: Float64)",
    )?;
    Ok((0..structs.len())
        .map(|i| [x.value(i), y.value(i), z.value(i)])
        .collect())
}

/// One ring: `ring_array` is the RING-level array (its `value(i)` is the
/// leaf `Int32Array` of vertex-pool indices directly — the innermost list,
/// one level up from `Int32`). Every index is bounds-checked against
/// `vertex_count` (see [`checked_vertex_index`]).
fn read_ring(ring_array: &ListArray, i: usize, vertex_count: usize) -> Result<Vec<usize>> {
    let row = ring_array.value(i);
    let ints = downcast::<Int32Array>(
        row.as_ref(),
        "ring level item type Int32 (vertex-pool index)",
    )?;
    (0..ints.len())
        .map(|j| checked_vertex_index(ints.value(j), vertex_count))
        .collect()
}

/// One face's rings: `face_array` is the FACE-level array (its `value(i)`
/// is the RING-level array for that one face).
fn read_face(face_array: &ListArray, i: usize, vertex_count: usize) -> Result<Vec<Vec<usize>>> {
    let row = face_array.value(i);
    let ring_array =
        downcast::<ListArray>(row.as_ref(), "face level item type List<Int32> (a ring)")?;
    (0..ring_array.len())
        .map(|j| read_ring(ring_array, j, vertex_count))
        .collect()
}

/// One shell's faces: `shell_array` is the SHELL-level array (its
/// `value(i)` is the FACE-level array for that one shell).
fn read_shell(
    shell_array: &ListArray,
    i: usize,
    vertex_count: usize,
) -> Result<Vec<Vec<Vec<usize>>>> {
    let row = shell_array.value(i);
    let face_array = downcast::<ListArray>(
        row.as_ref(),
        "shell level item type List<List<Int32>> (a face)",
    )?;
    (0..face_array.len())
        .map(|j| read_face(face_array, j, vertex_count))
        .collect()
}

/// This solid's shells: `solid_array` is the SOLID-level array (its
/// `value(i)` is the SHELL-level array for that one solid) — one level up
/// from [`read_shell`]'s input.
fn shells_of(solid_array: &ListArray, i: usize) -> Result<ListArray> {
    let row = solid_array.value(i);
    Ok(downcast::<ListArray>(
        row.as_ref(),
        "solid level item type List<List<List<Int32>>> (a shell)",
    )?
    .clone())
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
fn flatten_shells(shell_array: &ListArray, vertex_count: usize) -> Result<Vec<Vec<Vec<usize>>>> {
    let mut faces = Vec::new();
    for shell_idx in 0..shell_array.len() {
        faces.extend(read_shell(shell_array, shell_idx, vertex_count)?);
    }
    Ok(faces)
}

/// Reads row `row` of the `geometry_lod*`/`geometry_vertices_lod*` column
/// pair back into a [`DecodedGeometry`], dispatching on
/// `geometry_properties.type_name` to decide how to interpret/strip the
/// physical shape's padding dimensions — never inferred from nesting depth
/// (design doc "Critical invariant"). The `MultiSurface`/`CompositeSurface`/
/// `Solid` padding-cardinality checks (solid-count, and for surfaces also
/// shell-count, must be exactly 1) are real, always-checked errors, not
/// `debug_assert!` — a corrupt or foreign-writer file that violates the
/// padding invariant must be rejected in every build profile, not only debug
/// builds, and never via an unconditional index-zero access that panics
/// instead.
pub(crate) fn decode_row(
    geometry: &ListArray,
    vertices: &ListArray,
    row: usize,
    type_name: &str,
) -> Result<DecodedGeometry> {
    let coords = read_vertices(vertices, row)?;
    let vertex_count = coords.len();

    let solids_row = geometry.value(row);
    // SOLID-level array: solids of this row.
    let solids = downcast::<ListArray>(
        solids_row.as_ref(),
        "geometry_lod* row item type List<List<List<List<Int32>>>> (a solid)",
    )?;

    let kind = match type_name {
        "MultiSurface" | "CompositeSurface" => {
            // Padded to solid-count=1/shell-count=1 — strip both, recovering
            // the original MultiPolygon's faces directly from that one shell.
            if solids.len() != 1 {
                return Err(geometry_err(format!(
                    "arrow-native decode: MultiSurface/CompositeSurface must be padded to \
                     solid-count 1, found {} (corrupt or foreign file)",
                    solids.len()
                )));
            }
            let shells = shells_of(solids, 0)?;
            if shells.len() != 1 {
                return Err(geometry_err(format!(
                    "arrow-native decode: MultiSurface/CompositeSurface must be padded to \
                     shell-count 1, found {} (corrupt or foreign file)",
                    shells.len()
                )));
            }
            let faces = read_shell(&shells, 0, vertex_count)?;
            DecodedKind::MultiPolygon(faces)
        }
        "Solid" => {
            if solids.len() != 1 {
                return Err(geometry_err(format!(
                    "arrow-native decode: Solid must be padded to solid-count 1, found {} \
                     (corrupt or foreign file)",
                    solids.len()
                )));
            }
            let shells = shells_of(solids, 0)?;
            DecodedKind::PolyhedralSurface(flatten_shells(&shells, vertex_count)?)
        }
        "MultiSolid" | "CompositeSolid" => {
            let mut members = Vec::with_capacity(solids.len());
            for solid_idx in 0..solids.len() {
                let shells = shells_of(solids, solid_idx)?;
                members.push(DecodedKind::PolyhedralSurface(flatten_shells(
                    &shells,
                    vertex_count,
                )?));
            }
            DecodedKind::GeometryCollection(members)
        }
        other => {
            return Err(geometry_err(format!(
                "arrow-native decode: unsupported geometry_properties.type {other:?} \
                 (phase-1 scope: MultiSurface/CompositeSurface/Solid/MultiSolid/CompositeSolid)"
            )));
        }
    };
    Ok(DecodedGeometry { coords, kind })
}

#[cfg(test)]
mod tests {
    use arrow_array::{ArrayRef, Float64Array, Int32Array, ListArray, StructArray};
    use arrow_buffer::{NullBuffer, OffsetBuffer};
    use arrow_schema::{DataType, FieldRef};
    use std::sync::Arc;

    use crate::arrow_geom_write::ArrowGeomBuilders; // test-only cross-module use is fine within one crate
    use crate::wkb_read::{DecodedGeometry, DecodedKind};

    /// Peels the five `List` item fields off the declared schema type itself
    /// (mirrors `arrow_geom_write::geometry_item_fields`, which is private
    /// to that module) so a hand-built array cannot drift from the real
    /// column type. Shared by every test below that bypasses
    /// `ArrowGeomBuilders` to hand-build a specific (often malformed) shape
    /// directly.
    fn geometry_item_fields() -> [FieldRef; 5] {
        let mut data_type = cityparquet_schema::model::arrow_native_geometry_data_type();
        let mut items = Vec::with_capacity(5);
        for _ in 0..5 {
            match data_type {
                DataType::List(item) => {
                    data_type = item.data_type().clone();
                    items.push(item);
                }
                other => {
                    panic!("arrow_native_geometry_data_type must be 5 nested Lists, got {other:?}")
                }
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

    /// A one-row `geometry_vertices_lod*` column holding exactly `coords`.
    fn vertex_pool_column(coords: &[[f64; 3]]) -> ListArray {
        let xs = Float64Array::from(coords.iter().map(|c| c[0]).collect::<Vec<_>>());
        let ys = Float64Array::from(coords.iter().map(|c| c[1]).collect::<Vec<_>>());
        let zs = Float64Array::from(coords.iter().map(|c| c[2]).collect::<Vec<_>>());
        let coord_fields = match vertices_item_field().data_type() {
            DataType::Struct(fields) => fields.clone(),
            other => panic!("expected Struct, got {other:?}"),
        };
        let structs = StructArray::new(
            coord_fields,
            vec![Arc::new(xs), Arc::new(ys), Arc::new(zs)],
            None,
        );
        ListArray::new(
            vertices_item_field(),
            OffsetBuffer::from_lengths([coords.len()]),
            Arc::new(structs),
            Some(NullBuffer::from(vec![true])),
        )
    }

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
        let vertices = vertex_pool_column(&[
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [1.0, 0.0, 1.0],
            [0.0, 1.0, 1.0],
        ]);

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

    /// Codex round-2 review finding (Important): `decode_row` runs on ANY
    /// Parquet file `decode_batch` opens, not only files this crate itself
    /// wrote — a structurally-valid-but-corrupt or foreign-writer
    /// arrow-native geometry column can declare an out-of-range vertex-pool
    /// index (unlike WKB, where an index is assigned during decode itself
    /// and so cannot be out of range by construction). This must surface as
    /// a clean `CityParquetError`, never a downstream panic in
    /// `crate::export`'s vertex-pool remap.
    #[test]
    fn decode_row_errors_on_an_out_of_range_vertex_index() {
        let [
            solid_field,
            shell_field,
            face_field,
            ring_field,
            index_field,
        ] = geometry_item_fields();

        // One row, one solid, one shell, one face, one ring: 3 indices
        // [0, 1, 5] — index 5 is out of range, the vertex pool below only
        // has 3 entries (valid indices 0..=2).
        let indices: ArrayRef = Arc::new(Int32Array::from(vec![0, 1, 5]));
        let rings = ListArray::new(
            index_field,
            OffsetBuffer::from_lengths([3usize]),
            indices,
            None,
        );
        let faces = ListArray::new(
            ring_field,
            OffsetBuffer::from_lengths([1usize]),
            Arc::new(rings),
            None,
        );
        let shells = ListArray::new(
            face_field,
            OffsetBuffer::from_lengths([1usize]),
            Arc::new(faces),
            None,
        );
        let solids = ListArray::new(
            shell_field,
            OffsetBuffer::from_lengths([1usize]),
            Arc::new(shells),
            None,
        );
        let geometry = ListArray::new(
            solid_field,
            OffsetBuffer::from_lengths([1usize]),
            Arc::new(solids),
            Some(NullBuffer::from(vec![true])),
        );
        let vertices = vertex_pool_column(&[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]);

        let err = super::decode_row(&geometry, &vertices, 0, "MultiSurface")
            .expect_err("an out-of-range vertex-pool index must be a clean error, not a panic");
        assert!(
            matches!(err, cityparquet_schema::CityParquetError::Geometry(_)),
            "expected a Geometry error, got {err:?}"
        );
        assert!(
            err.to_string().contains("out of range"),
            "error should name the out-of-range index, got: {err}"
        );
    }

    /// The `i32` -> `usize` cast is checked (`usize::try_from`), not a
    /// silent `as usize` wraparound — a negative raw index must error
    /// cleanly rather than wrap to a huge, effectively-random `usize`.
    #[test]
    fn decode_row_errors_on_a_negative_vertex_index() {
        let [
            solid_field,
            shell_field,
            face_field,
            ring_field,
            index_field,
        ] = geometry_item_fields();

        let indices: ArrayRef = Arc::new(Int32Array::from(vec![0, -1, 2]));
        let rings = ListArray::new(
            index_field,
            OffsetBuffer::from_lengths([3usize]),
            indices,
            None,
        );
        let faces = ListArray::new(
            ring_field,
            OffsetBuffer::from_lengths([1usize]),
            Arc::new(rings),
            None,
        );
        let shells = ListArray::new(
            face_field,
            OffsetBuffer::from_lengths([1usize]),
            Arc::new(faces),
            None,
        );
        let solids = ListArray::new(
            shell_field,
            OffsetBuffer::from_lengths([1usize]),
            Arc::new(shells),
            None,
        );
        let geometry = ListArray::new(
            solid_field,
            OffsetBuffer::from_lengths([1usize]),
            Arc::new(solids),
            Some(NullBuffer::from(vec![true])),
        );
        let vertices = vertex_pool_column(&[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]);

        let err = super::decode_row(&geometry, &vertices, 0, "MultiSurface")
            .expect_err("a negative vertex-pool index must be a clean error, not a panic");
        assert!(
            matches!(err, cityparquet_schema::CityParquetError::Geometry(_)),
            "expected a Geometry error, got {err:?}"
        );
        assert!(
            err.to_string().contains("negative"),
            "error should name the negative index, got: {err}"
        );
    }

    /// Codex round-2 review finding (Important), the other named example: an
    /// empty MultiSurface outer (solid-count) list — the padding-cardinality
    /// invariant (`solid-count == 1` for `MultiSurface`/`CompositeSurface`)
    /// is a real, always-checked error now, not a `debug_assert!` that
    /// compiles out in release builds before an unconditional
    /// index-zero access.
    #[test]
    fn decode_row_errors_on_a_zero_length_solid_padding_dimension_for_multisurface() {
        let [
            solid_field,
            shell_field,
            face_field,
            ring_field,
            index_field,
        ] = geometry_item_fields();

        // Every level built zero-length: the ROW-level list (`geometry`)
        // itself has offset-length 0 for row 0 (zero solids), so
        // `decode_row`'s solid-count check must reject it before ever
        // calling `.value(0)` on the (also empty, but validly typed) levels
        // beneath — an unconditional index-zero access on any of them would
        // panic ("index out of bounds") rather than surfacing the intended
        // padding-cardinality error.
        let empty_indices: ArrayRef = Arc::new(Int32Array::from(Vec::<i32>::new()));
        let rings = ListArray::new(
            index_field,
            OffsetBuffer::from_lengths([]),
            empty_indices,
            None,
        );
        let faces = ListArray::new(
            ring_field,
            OffsetBuffer::from_lengths([]),
            Arc::new(rings),
            None,
        );
        let shells = ListArray::new(
            face_field,
            OffsetBuffer::from_lengths([]),
            Arc::new(faces),
            None,
        );
        let solids = ListArray::new(
            shell_field,
            OffsetBuffer::from_lengths([]),
            Arc::new(shells),
            None,
        );
        let geometry = ListArray::new(
            solid_field,
            OffsetBuffer::from_lengths([0usize]), // 1 ROW, 0 solids
            Arc::new(solids),
            Some(NullBuffer::from(vec![true])),
        );
        let vertices = vertex_pool_column(&[]);

        let err = super::decode_row(&geometry, &vertices, 0, "MultiSurface").expect_err(
            "a zero-length solid-count padding dimension must be a clean error, not a panic",
        );
        assert!(
            matches!(err, cityparquet_schema::CityParquetError::Geometry(_)),
            "expected a Geometry error, got {err:?}"
        );
        assert!(
            err.to_string().contains("solid-count"),
            "error should name the solid-count padding invariant, got: {err}"
        );
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
