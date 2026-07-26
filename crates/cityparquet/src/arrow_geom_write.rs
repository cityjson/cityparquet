//! `cjseq::Geometry` -> compacted `DecodedGeometry` for the arrow-native
//! encoding (design doc "Approaches considered", Option B). Reuses
//! `wkb_write`'s ring/shell normalisation so degenerate-geometry handling
//! matches the WKB path exactly; differs only in the target shape (indexed
//! `DecodedGeometry` instead of WKB bytes) and in how the vertex pool is
//! built: **distinct-source-index compaction**, never coordinate-value
//! dedup (design doc round-2 correction — two different source indices
//! with identical coordinates stay two separate pool entries).
//!
//! `crate::encode`'s `RowWriter` calls [`geometry_to_compacted`] and drives
//! [`ArrowGeomBuilders`] wherever it would otherwise call
//! `wkb_write::geometry_to_wkb` and append to a plain `BinaryBuilder`,
//! whenever `ConvertOptions.geometry_encoding == GeometryEncoding::ArrowNative`
//! (this plan's Task 6) — the WKB path remains the default and is otherwise
//! untouched.

use std::collections::HashMap;
use std::sync::Arc;

use arrow_array::{ArrayRef, Float64Array, Int32Array, ListArray, StructArray};
use arrow_buffer::{NullBuffer, OffsetBuffer};
use arrow_schema::{DataType, FieldRef, Fields};
use cityparquet_schema::model::{arrow_native_geometry_data_type, arrow_native_vertices_data_type};
use cityparquet_schema::{CityParquetError, Result};
use cjseq::{Geometry, GeometryType};

use crate::wkb_read::{DecodedGeometry, DecodedKind};
use crate::wkb_write::{Drops, VertexPool, boundaries, normalise_shells, normalise_surface};

/// Per-geometry, distinct-source-index vertex-pool compactor. Maps each
/// FIRST-SEEN raw source index to a dense local index and remembers its
/// dereferenced coordinate; a repeat occurrence of the SAME raw index reuses
/// its local index. Two different raw indices are never merged even if
/// bitwise-identical coordinates (design doc round-2 correction).
struct Compactor<'a, 'p> {
    pool: &'a VertexPool<'p>,
    seen: HashMap<usize, usize>,
    coords: Vec<[f64; 3]>,
}

impl<'a, 'p> Compactor<'a, 'p> {
    fn new(pool: &'a VertexPool<'p>) -> Self {
        Self {
            pool,
            seen: HashMap::new(),
            coords: Vec::new(),
        }
    }

    fn local_index(&mut self, raw: usize) -> Result<usize> {
        if let Some(&local) = self.seen.get(&raw) {
            return Ok(local);
        }
        let local = self.coords.len();
        self.coords.push(self.pool.coord(raw)?);
        self.seen.insert(raw, local);
        Ok(local)
    }

    fn ring(&mut self, ring: &[usize]) -> Result<Vec<usize>> {
        ring.iter().map(|&raw| self.local_index(raw)).collect()
    }

    fn surface(&mut self, rings: &[&[usize]]) -> Result<Vec<Vec<usize>>> {
        rings.iter().map(|r| self.ring(r)).collect()
    }
}

/// Outcome of [`geometry_to_compacted`]: the compacted geometry plus how much
/// its ring/surface normalisation dropped — mirrors
/// [`crate::wkb_write::WkbOutcome`]'s `dropped_rings`/`dropped_surfaces`
/// fields exactly, so a caller (`crate::encode::accumulate_geometry`) can
/// realign stored material/texture/semantics identically regardless of which
/// [`cityparquet_schema::GeometryEncoding`] produced the payload (this
/// plan's Task 8 — the round-trip proof needs exact parity here; this used
/// to be a documented gap — see git history — where this shape's drops were
/// silently discarded).
pub(crate) struct CompactedOutcome {
    pub(crate) geometry: DecodedGeometry,
    /// Structurally degenerate rings dropped (the [a,b,a] closure shape:
    /// fewer than 3 effective vertices) — see
    /// [`crate::wkb_write::WkbOutcome::dropped_rings`].
    pub(crate) dropped_rings: usize,
    /// Original flat surface/face positions (within this geometry) of
    /// surfaces dropped because their exterior ring was degenerate — see
    /// [`crate::wkb_write::WkbOutcome::dropped_surfaces`].
    pub(crate) dropped_surfaces: Vec<usize>,
}

/// `cjseq::Geometry` -> `Option<CompactedOutcome>`, phase-1 types only
/// (`MultiSurface`/`CompositeSurface`/`Solid`/`MultiSolid`/`CompositeSolid`
/// — design doc "Type coverage (v1)"). Mirrors `wkb_write::geometry_to_wkb`'s
/// dispatch and degenerate-ring/-surface handling exactly (same `Drops`
/// tracking, same `normalise_surface`/`normalise_shells` calls, and now the
/// same drop counts surfaced to the caller) — differs only in the output
/// shape. Returns `Ok(None)` for `GeometryInstance` (no geometry cell, same
/// as WKB) and for an empty/fully-degenerate result (same "no coordinates
/// written" rule as `wkb_write`).
pub(crate) fn geometry_to_compacted(
    geom: &Geometry,
    pool: &VertexPool,
) -> Result<Option<CompactedOutcome>> {
    let mut drops = Drops::default();
    let mut c = Compactor::new(pool);
    let kind = match geom.thetype {
        GeometryType::GeometryInstance => return Ok(None),
        GeometryType::MultiPoint | GeometryType::MultiLineString => {
            return Err(CityParquetError::Geometry(format!(
                "{:?} is not supported by the arrow-native encoding in phase 1 \
                 (design doc \"Type coverage (v1)\") — use --geometry-encoding wkb for this source",
                geom.thetype
            )));
        }
        GeometryType::MultiSurface | GeometryType::CompositeSurface => {
            let surfaces: Vec<Vec<Vec<usize>>> = boundaries(geom)?;
            let kept: Vec<Vec<&[usize]>> = surfaces
                .iter()
                .enumerate()
                .filter_map(|(pos, s)| normalise_surface(s, pos, &mut drops))
                .collect();
            let mut out = Vec::with_capacity(kept.len());
            for surface in &kept {
                out.push(c.surface(surface)?);
            }
            DecodedKind::MultiPolygon(out)
        }
        GeometryType::Solid => {
            let shells: Vec<Vec<Vec<Vec<usize>>>> = boundaries(geom)?;
            let mut pos = 0;
            let kept = normalise_shells(&shells, &mut pos, &mut drops);
            let mut out = Vec::with_capacity(kept.len());
            for face in &kept {
                out.push(c.surface(face)?);
            }
            DecodedKind::PolyhedralSurface(out)
        }
        GeometryType::MultiSolid | GeometryType::CompositeSolid => {
            let solids: Vec<Vec<Vec<Vec<Vec<usize>>>>> = boundaries(geom)?;
            let mut pos = 0;
            let mut members = Vec::with_capacity(solids.len());
            for solid in &solids {
                let kept = normalise_shells(solid, &mut pos, &mut drops);
                let mut out = Vec::with_capacity(kept.len());
                for face in &kept {
                    out.push(c.surface(face)?);
                }
                members.push(DecodedKind::PolyhedralSurface(out));
            }
            DecodedKind::GeometryCollection(members)
        }
    };
    if c.coords.is_empty() {
        return Ok(None);
    }
    Ok(Some(CompactedOutcome {
        geometry: DecodedGeometry {
            coords: c.coords,
            kind,
        },
        dropped_rings: drops.rings,
        dropped_surfaces: drops.surfaces,
    }))
}

/// The number of `List` levels in [`arrow_native_geometry_data_type`]:
/// row → solid → shell → face → ring → `Int32` index. **Five**, not four —
/// the row level and the solid level are distinct levels, so a `MultiSolid`'s
/// members stay separate solids instead of collapsing into extra shells of a
/// single solid.
const GEOMETRY_LIST_LEVELS: usize = 5;

/// The five `List` item fields of [`arrow_native_geometry_data_type`],
/// outermost first: `[solid, shell, face, ring, index]`. Peeled off the
/// schema type itself rather than rebuilt by hand, so the arrays this module
/// finishes can never drift from the declared column type (and a level lost
/// or gained here fails loudly in `ListArray::try_new`'s data-type check
/// instead of silently producing a mis-nested column).
fn geometry_item_fields() -> [FieldRef; GEOMETRY_LIST_LEVELS] {
    let mut data_type = arrow_native_geometry_data_type();
    let mut items = Vec::with_capacity(GEOMETRY_LIST_LEVELS);
    for level in 0..GEOMETRY_LIST_LEVELS {
        match data_type {
            DataType::List(item) => {
                data_type = item.data_type().clone();
                items.push(item);
            }
            other => unreachable!(
                "arrow_native_geometry_data_type must be {GEOMETRY_LIST_LEVELS} nested Lists; \
                 level {level} is {other}"
            ),
        }
    }
    assert_eq!(
        data_type,
        DataType::Int32,
        "arrow_native_geometry_data_type's innermost item must be an Int32 vertex index"
    );
    items
        .try_into()
        .expect("the loop above pushes exactly GEOMETRY_LIST_LEVELS items")
}

/// The `List` item field of [`arrow_native_vertices_data_type`] — one
/// `Struct<x,y,z>` coordinate.
fn vertices_item_field() -> FieldRef {
    match arrow_native_vertices_data_type() {
        DataType::List(item) => item,
        other => unreachable!("arrow_native_vertices_data_type must be a List, got {other}"),
    }
}

/// The `x`/`y`/`z` fields of one vertex-pool entry.
fn vertex_struct_fields() -> Fields {
    match vertices_item_field().data_type() {
        DataType::Struct(fields) => fields.clone(),
        other => {
            unreachable!("arrow_native_vertices_data_type's item must be a Struct, got {other}")
        }
    }
}

/// Accumulates one `geometry_lod*` / `geometry_vertices_lod*` column pair for
/// the arrow-native encoding — one instance per `GeometrySlot` (Task 6),
/// fed one [`append_value`](Self::append_value) or
/// [`append_null`](Self::append_null) call per row and drained once by
/// [`finish`](Self::finish).
///
/// Because the nesting depth is fixed by the schema (only the per-row
/// cardinalities vary), this builds the arrays **bottom-up from flat
/// accumulators** — `Vec`s of indices plus one length vector per list level —
/// and materialises them with `OffsetBuffer::from_lengths` +
/// `ListArray::new` in `finish`, rather than nesting
/// `ListBuilder<Box<dyn ArrayBuilder>>` five deep. Every level is a `List` of
/// the same shape, so a chain of downcast-and-append builders makes it easy to
/// close the wrong boundary (or none at all) without the compiler noticing;
/// with one explicit length vector per level, a missing boundary is a missing
/// `push` on a named vector, and the finished array's `DataType` is checked
/// against the schema type at construction.
#[derive(Default)]
pub(crate) struct ArrowGeomBuilders {
    /// Every ring's vertex indices, concatenated across all rows.
    indices: Vec<i32>,
    /// Per ring: how many indices it contributes to `indices`.
    ring_lengths: Vec<usize>,
    /// Per face: how many rings (exterior first, then interior rings).
    face_lengths: Vec<usize>,
    /// Per shell: how many faces.
    shell_lengths: Vec<usize>,
    /// Per solid: how many shells.
    solid_lengths: Vec<usize>,
    /// Per row: how many solids.
    row_lengths: Vec<usize>,
    /// Per row: the validity both columns share (an appended null row is null
    /// in the geometry column *and* in its vertices sibling).
    row_valid: Vec<bool>,
    /// The vertex pools of all rows, concatenated, split by `vertex_lengths`.
    xs: Vec<f64>,
    ys: Vec<f64>,
    zs: Vec<f64>,
    /// Per row: how many vertices its pool holds.
    vertex_lengths: Vec<usize>,
}

impl ArrowGeomBuilders {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Appends a row with no geometry: a null entry in both columns.
    pub(crate) fn append_null(&mut self) {
        self.row_lengths.push(0);
        self.vertex_lengths.push(0);
        self.row_valid.push(false);
    }

    /// Appends one row's compacted geometry: its vertex pool into the
    /// vertices column, its indexed boundaries into the geometry column.
    pub(crate) fn append_value(&mut self, decoded: &DecodedGeometry) {
        for c in &decoded.coords {
            self.xs.push(c[0]);
            self.ys.push(c[1]);
            self.zs.push(c[2]);
        }
        self.vertex_lengths.push(decoded.coords.len());

        let solids_before = self.solid_lengths.len();
        self.append_kind(&decoded.kind);
        self.row_lengths
            .push(self.solid_lengths.len() - solids_before);
        self.row_valid.push(true);
    }

    /// Appends one [`DecodedKind`] as this row's solids.
    ///
    /// Surface types (`MultiSurface`/`CompositeSurface` → `MultiPolygon`) and
    /// a `Solid` (→ `PolyhedralSurface`, whose shells Task 4 has already
    /// flattened into a single face list, exactly as the WKB path does) are
    /// both **padded** to one solid holding one shell: those levels carry no
    /// semantic distinction here, and the real shell structure stays in
    /// `geometry_properties.shells` (design doc "padding dimensions").
    /// `MultiSolid`/`CompositeSolid` (→ `GeometryCollection`) instead yields
    /// **one solid per member**, so member boundaries survive.
    fn append_kind(&mut self, kind: &DecodedKind) {
        match kind {
            DecodedKind::MultiPolygon(faces) | DecodedKind::PolyhedralSurface(faces) => {
                self.push_padded_solid(faces);
            }
            DecodedKind::GeometryCollection(members) => {
                for member in members {
                    match member {
                        DecodedKind::PolyhedralSurface(faces) => self.push_padded_solid(faces),
                        other => unreachable!(
                            "GeometryCollection member must be PolyhedralSurface \
                             (MultiSolid/CompositeSolid invariant) — got {other:?}"
                        ),
                    }
                }
            }
            DecodedKind::MultiPoint(_) | DecodedKind::MultiLineString(_) => unreachable!(
                "phase-1 scope excludes MultiPoint/MultiLineString (Task 4 already \
                 rejects them before a DecodedGeometry with this kind can exist)"
            ),
        }
    }

    /// Pushes `faces` as one solid of exactly one shell, closing every level's
    /// boundary: one `ring_lengths` entry per ring, one `face_lengths` entry
    /// per face, one `shell_lengths` entry for the shell, one `solid_lengths`
    /// entry for the solid.
    fn push_padded_solid(&mut self, faces: &[Vec<Vec<usize>>]) {
        for rings in faces {
            for ring in rings {
                self.indices.extend(ring.iter().map(|&i| {
                    i32::try_from(i).expect(
                        "a row's compacted vertex pool cannot hold more than i32::MAX entries",
                    )
                }));
                self.ring_lengths.push(ring.len());
            }
            self.face_lengths.push(rings.len());
        }
        self.shell_lengths.push(faces.len());
        self.solid_lengths.push(1);
    }

    /// Materialises both columns. The geometry column is assembled
    /// innermost-out — indices → rings → faces → shells → solids → rows — so
    /// each level's `OffsetBuffer` closes exactly the boundaries recorded for
    /// it, and only the row level carries a validity buffer (every inner item
    /// field is declared non-nullable).
    pub(crate) fn finish(self) -> (ArrayRef, ArrayRef) {
        let [
            solid_field,
            shell_field,
            face_field,
            ring_field,
            index_field,
        ] = geometry_item_fields();
        let nulls = NullBuffer::from(self.row_valid);

        let indices: ArrayRef = Arc::new(Int32Array::from(self.indices));
        let rings = ListArray::new(
            index_field,
            OffsetBuffer::from_lengths(self.ring_lengths),
            indices,
            None,
        );
        let faces = ListArray::new(
            ring_field,
            OffsetBuffer::from_lengths(self.face_lengths),
            Arc::new(rings),
            None,
        );
        let shells = ListArray::new(
            face_field,
            OffsetBuffer::from_lengths(self.shell_lengths),
            Arc::new(faces),
            None,
        );
        let solids = ListArray::new(
            shell_field,
            OffsetBuffer::from_lengths(self.solid_lengths),
            Arc::new(shells),
            None,
        );
        let geometry = ListArray::new(
            solid_field,
            OffsetBuffer::from_lengths(self.row_lengths),
            Arc::new(solids),
            Some(nulls.clone()),
        );

        let coords = StructArray::new(
            vertex_struct_fields(),
            vec![
                Arc::new(Float64Array::from(self.xs)),
                Arc::new(Float64Array::from(self.ys)),
                Arc::new(Float64Array::from(self.zs)),
            ],
            None,
        );
        let vertices = ListArray::new(
            vertices_item_field(),
            OffsetBuffer::from_lengths(self.vertex_lengths),
            Arc::new(coords),
            Some(nulls),
        );

        (Arc::new(geometry), Arc::new(vertices))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transform_identity() -> cjseq::Transform {
        cjseq::Transform {
            scale: vec![1.0, 1.0, 1.0],
            translate: vec![0.0, 0.0, 0.0],
        }
    }

    fn multisurface_geom(boundaries: serde_json::Value) -> Geometry {
        Geometry {
            thetype: GeometryType::MultiSurface,
            lod: Some("2".to_string()),
            boundaries,
            semantics: None,
            material: None,
            texture: None,
            template: None,
            transformation_matrix: None,
        }
    }

    #[test]
    fn multisurface_two_triangles_sharing_an_edge_compacts_the_shared_pair() {
        // Two triangles sharing edge (1,2): 4 distinct vertices total, not 6.
        let vertices: Vec<Vec<i64>> =
            vec![vec![0, 0, 0], vec![1, 0, 0], vec![1, 1, 0], vec![0, 1, 0]];
        let pool = VertexPool::new(&vertices, &transform_identity());
        let geom = multisurface_geom(serde_json::json!([[[0, 1, 2]], [[0, 2, 3]]]));
        let decoded = geometry_to_compacted(&geom, &pool)
            .unwrap()
            .unwrap()
            .geometry;
        assert_eq!(
            decoded.coords.len(),
            4,
            "shared indices 0 and 2 must be compacted, not duplicated"
        );
        match &decoded.kind {
            DecodedKind::MultiPolygon(surfaces) => {
                assert_eq!(surfaces.len(), 2);
                assert_eq!(surfaces[0], vec![vec![0, 1, 2]]);
                assert_eq!(surfaces[1], vec![vec![0, 2, 3]]);
            }
            other => panic!("expected MultiPolygon, got {other:?}"),
        }
    }

    #[test]
    fn distinct_indices_with_equal_coordinates_are_never_merged() {
        // Two source indices, SAME coordinate value — must stay two pool entries.
        let vertices: Vec<Vec<i64>> = vec![vec![0, 0, 0], vec![0, 0, 0], vec![1, 0, 0]];
        let pool = VertexPool::new(&vertices, &transform_identity());
        let geom = multisurface_geom(serde_json::json!([[[0, 1, 2]]]));
        let decoded = geometry_to_compacted(&geom, &pool)
            .unwrap()
            .unwrap()
            .geometry;
        assert_eq!(
            decoded.coords.len(),
            3,
            "indices 0 and 1 have identical coordinates but are DISTINCT source vertices \
             (design doc: index-identity compaction, not coordinate-value dedup)"
        );
    }

    #[test]
    fn solid_two_shells_flattens_faces_like_wkb_and_reports_no_shell_distinction() {
        // A minimal 2-shell Solid: exterior (1 face, a triangle) + one interior
        // cavity face (also a triangle) sharing no vertices with the exterior.
        let vertices: Vec<Vec<i64>> = vec![
            vec![0, 0, 0],
            vec![1, 0, 0],
            vec![0, 1, 0],
            vec![0, 0, 1],
            vec![1, 0, 1],
            vec![0, 1, 1],
        ];
        let pool = VertexPool::new(&vertices, &transform_identity());
        let geom = Geometry {
            thetype: GeometryType::Solid,
            lod: Some("2".to_string()),
            boundaries: serde_json::json!([[[[0, 1, 2]]], [[[3, 4, 5]]]]),
            semantics: None,
            material: None,
            texture: None,
            template: None,
            transformation_matrix: None,
        };
        let decoded = geometry_to_compacted(&geom, &pool)
            .unwrap()
            .unwrap()
            .geometry;
        assert_eq!(decoded.coords.len(), 6);
        match &decoded.kind {
            // Flattened to 2 faces, shell boundary NOT represented here —
            // exactly mirroring wkb_write::geometry_to_wkb's PolyhedralSurfaceZ
            // output (shell structure lives only in geometry_properties.shells,
            // unchanged, design doc "Face traversal order").
            DecodedKind::PolyhedralSurface(faces) => assert_eq!(faces.len(), 2),
            other => panic!("expected PolyhedralSurface, got {other:?}"),
        }
    }

    /// Task 8: `geometry_to_compacted` must report the SAME drop counts
    /// `geometry_to_wkb` does for identical input — mirrors
    /// `wkb_write::tests::degenerate_ring_drops_with_its_surface` exactly
    /// (same boundaries shapes, both sub-cases). This used to be a
    /// documented gap (the function tracked drops internally via `Drops`
    /// but never returned them, so `crate::encode::accumulate_geometry`'s
    /// arrow-native branch always saw `(0, Vec::new())` regardless of what
    /// was actually dropped) — real-fixture evidence: railway's
    /// `--geometry-encoding arrow-native` round trip silently desynced a
    /// dropped surface's stored `material_lod*` length from its stored
    /// geometry's face count.
    ///
    /// Codex round-2 review finding (Important): an earlier version of this
    /// test asserted only hardcoded literal values against
    /// `geometry_to_compacted`'s own output — it never called
    /// `geometry_to_wkb` at all, so it would have kept passing even if the
    /// two encoders' drop-reporting silently diverged (the actual parity
    /// this test's name, and `CompactedOutcome` mirroring `WkbOutcome` in
    /// the first place, exist to prove). Fixed: both encoders now run on
    /// the SAME `geom`/`pool`, and their outcomes are asserted equal to
    /// each other directly, not each merely equal to a separately-chosen
    /// literal.
    #[test]
    fn geometry_to_compacted_reports_the_same_drops_as_geometry_to_wkb() {
        use crate::wkb_write::geometry_to_wkb;

        // Surface 0's exterior ring is the structural [a,b,a] closure shape
        // (2 effective vertices): the ring is dropped, and with it the whole
        // surface. Surface 1 is fine and must survive as the ONLY polygon.
        let vertices: Vec<Vec<i64>> =
            vec![vec![0, 0, 0], vec![1, 0, 0], vec![0, 1, 0], vec![0, 0, 1]];
        let pool = VertexPool::new(&vertices, &transform_identity());
        let geom = multisurface_geom(serde_json::json!([[[0, 1, 0]], [[0, 1, 2, 3]]]));

        let wkb_outcome = geometry_to_wkb(&geom, &pool).unwrap().unwrap();
        let compacted_outcome = geometry_to_compacted(&geom, &pool).unwrap().unwrap();

        // The actual parity requirement: identical input, directly compared
        // against each other — not against separately hardcoded literals,
        // which could not catch the two encoders drifting apart.
        assert_eq!(
            compacted_outcome.dropped_rings, wkb_outcome.dropped_rings,
            "arrow-native and WKB must report the same dropped_rings count for identical input"
        );
        assert_eq!(
            compacted_outcome.dropped_surfaces, wkb_outcome.dropped_surfaces,
            "arrow-native and WKB must report the same dropped_surfaces positions for identical \
             input"
        );

        // Additional coverage: the concrete expected values, pinning the
        // fixture's own intent (and catching both encoders drifting
        // together in the same wrong direction, which the cross-comparison
        // above alone could not).
        assert_eq!(wkb_outcome.dropped_rings, 1);
        assert_eq!(wkb_outcome.dropped_surfaces, vec![0]);
        match &compacted_outcome.geometry.kind {
            DecodedKind::MultiPolygon(surfaces) => {
                assert_eq!(surfaces.len(), 1, "only the surviving surface is kept");
            }
            other => panic!("expected MultiPolygon, got {other:?}"),
        }

        // Second sub-case (mirrors `wkb_write`'s own test exactly): a
        // dropped surface's interior degenerate ring is still counted in
        // dropped_rings (surface drop unchanged) — proves the parity holds
        // even when dropped_rings and dropped_surfaces.len() diverge, not
        // just in the trivial 1-dropped-ring-equals-1-dropped-surface case
        // above.
        let geom2 = multisurface_geom(serde_json::json!([[[0, 1, 0], [2, 3, 2]], [[0, 1, 2, 3]]]));
        let wkb_outcome2 = geometry_to_wkb(&geom2, &pool).unwrap().unwrap();
        let compacted_outcome2 = geometry_to_compacted(&geom2, &pool).unwrap().unwrap();
        assert_eq!(compacted_outcome2.dropped_rings, wkb_outcome2.dropped_rings);
        assert_eq!(
            compacted_outcome2.dropped_surfaces,
            wkb_outcome2.dropped_surfaces
        );
        assert_eq!(
            wkb_outcome2.dropped_rings, 2,
            "degenerate exterior AND degenerate interior must both be counted"
        );
        assert_eq!(wkb_outcome2.dropped_surfaces, vec![0]);
    }

    #[test]
    fn arrow_geom_builders_round_trip_a_multisurface_through_arrow_arrays() {
        use arrow_array::{Array, ListArray, StructArray};

        let decoded = DecodedGeometry {
            coords: vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
            kind: DecodedKind::MultiPolygon(vec![vec![vec![0, 1, 2]], vec![vec![0, 2, 3]]]),
        };
        let mut b = ArrowGeomBuilders::new();
        b.append_value(&decoded);
        b.append_null();
        let (geometry, vertices) = b.finish();

        let geom_list = geometry.as_any().downcast_ref::<ListArray>().unwrap();
        assert_eq!(geom_list.len(), 2);
        assert!(geom_list.is_valid(0));
        assert!(geom_list.is_null(1));

        let vert_list = vertices.as_any().downcast_ref::<ListArray>().unwrap();
        assert!(vert_list.is_valid(0));
        let row0_vertices = vert_list.value(0);
        let structs = row0_vertices
            .as_any()
            .downcast_ref::<StructArray>()
            .unwrap();
        assert_eq!(
            structs.len(),
            4,
            "4 distinct vertices, matching the Task-4 compaction test"
        );
    }

    #[test]
    fn arrow_geom_builders_pad_solid_and_flatten_multisolid_members() {
        use arrow_array::{Array, Int32Array, ListArray};

        fn as_list(array: &ArrayRef) -> &ListArray {
            array.as_any().downcast_ref::<ListArray>().unwrap()
        }

        let solid = DecodedGeometry {
            coords: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            kind: DecodedKind::PolyhedralSurface(vec![vec![vec![0, 1, 2]]]),
        };
        let mut b = ArrowGeomBuilders::new();
        b.append_value(&solid);
        let (geometry, _vertices) = b.finish();
        let outer = geometry.as_any().downcast_ref::<ListArray>().unwrap();
        let solids_row0 = outer.value(0); // List<shell>
        let shells = solids_row0.as_any().downcast_ref::<ListArray>().unwrap();
        assert_eq!(
            shells.len(),
            1,
            "a bare Solid pads to solid-count 1 (design doc: not a semantic distinction)"
        );
        // ...and that one solid pads to one shell, holding the one face.
        let solid0_shells = shells.value(0);
        let solid0_shells = as_list(&solid0_shells);
        assert_eq!(solid0_shells.len(), 1, "a bare Solid pads to shell-count 1");
        let shell0_faces = solid0_shells.value(0);
        assert_eq!(as_list(&shell0_faces).len(), 1, "one face in this fixture");

        let multisolid = DecodedGeometry {
            coords: vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [5.0, 5.0, 5.0],
                [6.0, 5.0, 5.0],
                [5.0, 6.0, 5.0],
            ],
            kind: DecodedKind::GeometryCollection(vec![
                DecodedKind::PolyhedralSurface(vec![vec![vec![0, 1, 2]]]),
                DecodedKind::PolyhedralSurface(vec![vec![vec![3, 4, 5]]]),
            ]),
        };
        let mut b2 = ArrowGeomBuilders::new();
        b2.append_value(&multisolid);
        let (geometry2, _) = b2.finish();
        let outer2 = geometry2.as_any().downcast_ref::<ListArray>().unwrap();
        let solids_row0_2 = outer2.value(0);
        let solids2 = solids_row0_2.as_any().downcast_ref::<ListArray>().unwrap();
        assert_eq!(
            solids2.len(),
            2,
            "MultiSolid with 2 members -> solid-count 2"
        );

        // The member boundary must be real, not merely counted: each member is
        // its own solid of one shell, and keeps its own indices. This is what
        // separates a genuine 5-level build from one that collapses the row and
        // solid levels (which would show up here as one solid of two shells).
        for (member, expected) in [(0usize, [0, 1, 2]), (1, [3, 4, 5])] {
            let member_shells = solids2.value(member);
            let member_shells = as_list(&member_shells);
            assert_eq!(
                member_shells.len(),
                1,
                "member {member} pads to shell-count 1"
            );
            let faces = member_shells.value(0);
            let faces = as_list(&faces);
            assert_eq!(faces.len(), 1, "member {member} has one face");
            let rings = faces.value(0);
            let rings = as_list(&rings);
            assert_eq!(rings.len(), 1, "member {member}'s face has one ring");
            let ring = rings.value(0);
            let ring = ring.as_any().downcast_ref::<Int32Array>().unwrap();
            assert_eq!(
                ring.values(),
                &expected[..],
                "member {member}'s vertex indices stay with its own solid"
            );
        }
    }

    /// The strongest available check that no nesting level collapsed: the
    /// finished columns' types must equal the declared arrow-native column
    /// types exactly — five `List` levels (row/solid/shell/face/ring) over
    /// `Int32`, and `List<Struct<x,y,z>>` for the vertex pool.
    #[test]
    fn arrow_geom_builders_columns_match_the_declared_arrow_native_schema_types() {
        use arrow_array::Array;

        let decoded = DecodedGeometry {
            coords: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            kind: DecodedKind::PolyhedralSurface(vec![vec![vec![0, 1, 2]]]),
        };
        let mut b = ArrowGeomBuilders::new();
        b.append_value(&decoded);
        b.append_null();
        let (geometry, vertices) = b.finish();

        assert_eq!(geometry.data_type(), &arrow_native_geometry_data_type());
        assert_eq!(vertices.data_type(), &arrow_native_vertices_data_type());

        // Counted independently of `geometry_item_fields`, which derives its
        // fields from the same schema function.
        let mut data_type = geometry.data_type().clone();
        let mut levels = 0;
        while let DataType::List(item) = data_type {
            levels += 1;
            data_type = item.data_type().clone();
        }
        assert_eq!(
            levels, GEOMETRY_LIST_LEVELS,
            "row/solid/shell/face/ring — five List levels, not four"
        );
        assert_eq!(data_type, DataType::Int32);
    }

    /// A slot that never saw a row must still finish into a well-formed,
    /// zero-length column pair rather than panicking on an empty offset
    /// buffer — the writer allocates a builder per `GeometrySlot`, so an
    /// empty batch is reachable.
    #[test]
    fn arrow_geom_builders_finish_with_no_rows_yields_empty_columns() {
        use arrow_array::Array;

        let (geometry, vertices) = ArrowGeomBuilders::new().finish();
        assert_eq!(geometry.len(), 0);
        assert_eq!(vertices.len(), 0);
        assert_eq!(geometry.data_type(), &arrow_native_geometry_data_type());
        assert_eq!(vertices.data_type(), &arrow_native_vertices_data_type());
    }
}
