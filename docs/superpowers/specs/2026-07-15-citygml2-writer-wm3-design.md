# CityGML 2.0 Writer — W-M3 (Semantic Surfaces) Design

**Date:** 2026-07-15
**Milestone:** W-M3 (follows W-M2: attributes + CompositeSolid)
**Status:** approved (scope re-split confirmed by user; Fable-reviewed)

## Goal

Emit `bldg:boundedBy` semantic surfaces so a CityParquet package's
`geometry_properties.semantics` round-trips back to CityGML 2.0, closing the
`semantic_surfaces_dropped` gap W-M2 opened. Two geometry cases:

1. **Solid / CompositeSolid + semantics** (e.g. `b1_lod2_cs_w_sem.gml`): the
   solid's polygons live **inline inside the `boundedBy` surfaces** (each with a
   `gml:id`), and the `lodNSolid` references them by `xlink:href`; a face with no
   semantic surface (`null` value) is emitted **inline in the solid's
   `surfaceMember`** (no separate `lodNMultiSurface`).
2. **MultiSurface + semantics, no solid** (e.g. `railway_lod3_fragment.gml`): the
   `boundedBy` surfaces carry their polygons inline; there is no solid.

BuildingParts (W-M4, a paired reader+writer milestone) and appearance (W-M5) are
out of scope.

## Global Constraints

- Round-trip invariant is PACKAGE-LEVEL: a real CityGML fixture → package → `.gml`
  (writer) → package (re-convert) must produce equal stored geometry **and
  semantics** (`surfaces` + `values`), keyed by `(building_id, major_lod)`. XML
  need not be byte-identical.
- Real fixtures only for round-trip oracles. Oracles use **CityGML-sourced**
  fixtures (b1, railway), whose `surfaces` entries carry only `{"type"}` — so the
  writer (which can emit only the surface type) round-trips them exactly. Per-
  surface attributes and openings' parent/child links (which only CityJSON-
  sourced packages carry) are out of scope; see limitations.
- Strict red-green TDD; one behaviour per test; frequent commits.
- Never error on unrepresentable data — skip-with-counter, geometry-only fallback.
- `just check` (clippy `-D warnings` + fmt + tests + schema isolation) is the gate.
- Codex external review at milestone end.

## Background: verified facts

- `geometry_properties.semantics = { "surfaces": [{"type": T}, …], "values": V }`.
  `surfaces` is a FLAT list of typed surfaces (openings flattened in as `Door`/
  `Window` entries; only `type`). `V` nesting by geometry type: Solid
  `[shell][face]`; CompositeSolid `[solid][shell][face]`; MultiSurface flat
  `[position]`. A face with no surface has a `null` value.
- The decoded WKB gives a flat face list; `export::{shell_faces_flat,
  shell_faces_nested, partition_shells}` reconstruct the per-shell (and per-solid)
  face partition — the SAME order `values` indexes.
- The reader matches faces↔surfaces by shared `gml:id`
  (`semantic_of_polygon`); an **inline** polygon in a solid `surfaceMember`
  resolves to `RefTarget::Inline` → **null** semantics (verified
  `geometry.rs:237-240`, `building.rs:462-470`). So duplicate-inline geometry
  would lose all semantics — xlink is required for semantic faces, inline is
  correct for null faces.
- Forward `xlink:href` (solid references an id defined later in `boundedBy`) is
  resolved by the reader's two-pass registry (b1 is structured this way).
- `xmlns:xlink` is already declared on `<CityModel>` (W-M1).

## Design

### A. Dispatch by `geometry_properties.type`, not WKB kind

MultiSurface and CompositeSurface both decode to `MultiPolygon`, so dispatch on
the stored `type`:

- `Solid` / `CompositeSolid` **with** `semantics` → **§B** (xlinked solid + inline
  boundedBy).
- `MultiSurface` / `CompositeSurface` **with** `semantics` → **§C** (inline
  boundedBy, no solid).
- No `semantics` → W-M2 behaviour, unchanged (inline solid; MultiSurface still
  skipped as it was — a MultiSurface without semantics is out of scope, counted
  in `lod_columns_skipped` as today).
- `MultiSolid` with semantics → still skipped (`multi_solids_skipped`); its
  surfaces feed `semantic_surfaces_dropped` (currently they vanish uncounted).

### B. Solid / CompositeSolid + semantics

**gml:id scheme:** one document-global monotonic counter → `p<N>` (a valid
NCName). Every generated id is inserted into the existing `seen_ids` set (which
already holds CityObject `gml:id`s); on the rare collision with a verbatim object
id, bump `N` and retry. Ids need only be internally consistent per pass (the
oracle compares geometry+semantics, not ids).

**Per geometry:**
1. Partition faces into shells (`partition_shells` for a Solid; `shell_faces_nested`
   + per-member for a CompositeSolid), yielding faces in `values`-index order.
2. Walk faces in that order; for each face read its surface index from `values`
   (nested lookup). Validate: index `< surfaces.len()`, `values` nesting/lengths
   match the shell/member partition. On mismatch (corrupt/external input):
   geometry-only fallback for this geometry, `semantic_surfaces_dropped +=
   surfaces.len()`, and emit it as a plain W-M2 solid.
3. A face with a **non-null** index → assign a `gml:id`; record it under that
   surface. A **null** face → no id (emitted inline in the solid).
4. Emit `bldg:lod<major>Solid` first: the `gml:Solid` / `gml:CompositeSolid` whose
   every shell `gml:CompositeSurface` has, per face in order, either
   `<gml:surfaceMember xlink:href="#p<N>"/>` (semantic face) or an inline
   `<gml:surfaceMember><gml:Polygon>…` (null face).
5. Emit one `bldg:boundedBy` per `surfaces` entry, **in array order** (never
   dedupe/reorder/merge — two identical `{"type":"WallSurface"}` entries are two
   elements): `<bldg:{type}><bldg:lod<major>MultiSurface><gml:surfaceMember>
   <gml:Polygon gml:id="p<N>">…` for each face assigned to that surface; a
   zero-face surface is emitted as an empty `<bldg:{type}/>` (preserves the
   surface-index alignment the reader assigns by document order).
6. `semantic_surfaces_written += surfaces.len()`.

An **extension surface type** (CityJSON `+Foo`, not a legal XML element name):
that geometry falls back to geometry-only, `semantic_surfaces_dropped +=
surfaces.len()`.

### C. MultiSurface / CompositeSurface + semantics (no solid)

Emit one `bldg:boundedBy` per `surfaces` entry (array order, zero-face → empty),
each `<bldg:{type}><bldg:lod<major>MultiSurface>` with the inline `gml:Polygon`s
of the faces whose flat `values[position]` equals that surface index. No solid, no
xlink. A face with a **null** value has no CityGML home in this case (the reader's
MultiSurface path can only produce non-null values): drop it with
`multisurface_null_faces_dropped` (CityGML-sourced fixtures never hit this).
`semantic_surfaces_written += surfaces.len()`.

Bounds: accumulate every emitted polygon's coordinates (as W-M2 does), from
whichever path emits them.

### D. Report counters (`WriteReport` additions)

- `semantic_surfaces_written: usize` — `bldg:boundedBy` surfaces emitted.
- Keep `semantic_surfaces_dropped: usize` — now: surfaces on a geometry that fell
  back to geometry-only (extension type, shape mismatch) plus MultiSolid-with-
  semantics surfaces.
- `multisurface_null_faces_dropped: usize` — null-semantic faces in a no-solid
  MultiSurface (unreachable for CityGML-sourced input).

### E. Round-trip oracles (real fixtures)

1. **Solid + semantics** — rewrite `citygml_writer_composite.rs`'s b1 oracle: assert
   the full decoded **semantics** (`surfaces` + `values`) AND geometry structure
   are equal across `gml → package → gml → package`, keyed by `(id, major)`;
   `semantic_surfaces_written == 9`, `semantic_surfaces_dropped == 0`, and the
   re-read package **still carries** `geometry_properties.semantics` (inverting
   W-M2's "absent after" assertion). Compare `values` structurally (nested).
2. **MultiSurface + semantics** — new oracle on `railway_lod3_fragment.gml`:
   assert semantics (`surfaces` + `values`) and boundaries equal across the round
   trip; the surface-type histogram (from the reader M4 test: Wall/Roof/Ground/
   OuterCeiling/OuterFloor/Window/Door) is preserved.
3. Both assert the skip counters are `0` for these all-representable fixtures.

### F. Unit tests (in-code values)

- `values` nested lookup (Solid `[shell][face]`, CompositeSolid
  `[solid][shell][face]`); null face → inline in solid; non-null → xlink + inline
  in boundedBy.
- One `bldg:boundedBy` per surfaces entry, in order; a duplicate type → two
  elements; a zero-face surface → empty element.
- gml:id uniqueness + collision-with-object-id bump.
- Extension type (`+Foo`) → geometry-only fallback + counter.
- `values`/shape mismatch → geometry-only fallback + counter.
- MultiSurface null face → dropped + counter.
- No-semantics geometry unchanged (W-M2 regression guard).

## Out of scope (W-M4+)

BuildingParts (paired reader+writer, W-M4), appearance/material (W-M5), per-surface
attributes and opening parent/child nesting (the reader flattens openings and
stores only surface `type`), `uom` fidelity, XSD validation of emitted output.

## Known limitations (document in code + paper)

- **Openings** (`Door`/`Window`) round-trip only as flat top-level `bldg:boundedBy`
  surfaces (the reader flattens `bldg:opening` and drops the parent-surface link);
  this is XSD-invalid (`_Opening` is not a `_BoundarySurface`) but package-
  lossless through the lenient reader.
- **Per-surface attributes** are not stored by the reader (surfaces carry only
  `type`), so they cannot be emitted.
- CityJSON-sourced packages whose MultiSurface `values` are not contiguous by
  surface, or carry extra surface keys, are outside the round-trip guarantee
  (the W-M3 oracles use CityGML-sourced fixtures only).
