# CityGML 2.0 BuildingParts — W-M4 Design (paired reader + writer)

**Date:** 2026-07-15
**Milestone:** W-M4 (follows W-M3 semantic surfaces)
**Status:** approved (Fable-reviewed)

## Goal

Round-trip `bldg:BuildingPart`: the CityGML reader learns to read
`bldg:consistsOfBuildingPart/bldg:BuildingPart` into parent + child CityObjects,
and the writer learns to emit them, so a package's Building/BuildingPart
structure survives CityGML → package → `.gml` → package. This is a **paired**
milestone: a writer-only feature cannot be verified (the reader must read parts
back for the package round-trip). Fixes the pre-existing writer blocker that
skips geometry-less parent Buildings (3DBAG parents are geometry-less, holding
all geometry in their parts).

## Global Constraints

- Round-trip invariant is PACKAGE-LEVEL: after CityGML → package → `.gml` →
  package, each CityObject's `parents`/`children`, geometry, attributes, and
  semantics are equal, **keyed by id** (encode sorts object ids within a
  feature, so row order is not preserved — never compare by row order).
- Real fixtures only: a hand-authored CityGML `building_with_parts.gml` (like the
  existing hand-authored `b1_lod2_cs_w_sem.gml`) AND the real 3DBAG
  `delft.city.jsonl` (CityJSON → package → `.gml` → package).
- Never error on unrepresentable data — skip-with-counter.
- Strict red-green TDD; one behaviour per test; frequent commits.
- `just check` (clippy `-D warnings` + fmt + tests + schema isolation) is the gate.
- Codex external review at milestone end.

## Background (verified)

- A cjseq `CityJSONFeature` has ONE `vertices` pool shared by all its
  `city_objects`; each CityObject's `geometry` indices point into it
  (`encode.rs` `push_object` resolves against `feature.vertices`). delft's
  Building+BuildingPart features are already shaped this way.
- The package stores `parents`/`children` verbatim; `decode` reconstructs them
  into the cjseq CityObject; the writer reads `obj.object.{thetype, parents,
  children}`.
- The writer already buffers the whole document body in memory (`members`),
  so buffering decoded rows is a constant-factor, not asymptotic, change.
- `read_building` breaks on `</Building>` via a flat, order-independent dispatch
  loop; `expand_empty_elements = true` (empty elements arrive as Start+End).
- Polygon `gml:id`s for semantics use `IdAlloc::new(feature_index, major)`; the
  driver's `next_feature_index` increments per Building row.

## Design — Reader

### R1. Parameterised `read_building` + `consistsOfBuildingPart`

- Add an `end_name: &[u8]` parameter (or a thin wrapper) so `read_building`
  reads either a `Building` or a `BuildingPart` subtree — the `_AbstractBuilding`
  content model (solids, MultiSurface, boundedBy, attributes) is identical.
- New branch on a `bldg:consistsOfBuildingPart` **property** element:
  1. It is a wrapper, not the part itself. Descend to the inner
     `Start(bldg:BuildingPart)`, capture its `gml:id` there (mirroring the
     driver's `gml_id(&e)` before `read_building`).
  2. Recurse `read_building(..., end_name = b"BuildingPart")` → a child
     `RawBuilding`.
  3. Consume through `End(consistsOfBuildingPart)` inside the property helper —
     never leave it for the main loop's catch-all.
  - **Guard the empty/xlink-only wrapper**: `<bldg:consistsOfBuildingPart
    xlink:href=…/>` (or empty) arrives as Start+End with no `bldg:BuildingPart`
    child → produce NO part (count in a reader-side skip if useful; at minimum
    do not create a phantom empty `RawBuilding`).

### R2. `RawBuilding.parts` tree + `into_feature` flattening

- `RawBuilding` gains `parts: Vec<RawBuilding>` (a tree — part-in-part is legal
  `_AbstractBuilding` nesting).
- Bound recursion depth (e.g. 32) → hard error on exceed (attacker-controlled
  XML nesting; consistent with `read_semantic_surface`'s recursion but bounded).
- `into_feature` emits, into ONE `CityJSONFeature` sharing ONE `VertexBuilder`:
  - the parent CityObject: `type:"Building"`, its own geometry+attributes (if
    any), `children:[immediate part ids, in document order]`;
  - one CityObject per part (depth-first): `type:"BuildingPart"`,
    geometry+attributes, `parents:[immediate parent id]`, and its own
    `children` when it has sub-parts.
  - `feature.id` = the parent Building's resolved id.
- Part ids: the part's `gml:id`, else a deterministic NCName-safe synthesis
  `<parent_id>_part_<i>` (document order; depth-aware for nesting).
- **Duplicate-id detection**: if two CityObjects in the feature would share an
  id (a part id equal to the parent's or a sibling's), error (matches the
  writer's document-unique `gml:id` contract; `feature.add_co` would otherwise
  silently overwrite).

## Design — Writer

### W1. Group rows; shared content emitter

- Decode all rows into memory. Partition into Buildings (`type=="Building"`) and
  BuildingParts (`type=="BuildingPart"`); group parts by `parents[0]` (a
  multi-parent part emits under `parents[0]` only — count the lost secondary
  link). A part whose `parents[0]` names no Building/BuildingPart row → orphan.
- Extract `write_abstract_building_content(w, obj, types, feature_index, bounds,
  report)` from the current `write_building` body: emits attributes + own
  geometry (`lodNSolid`/boundedBy) but NOT the `cityObjectMember`/`bldg:Building`
  wrapper. Shared by a Building and a BuildingPart.

### W2. Bottom-up rendering + emptiness/skip

- Render bottom-up: each part is rendered into its own buffer first (recursively,
  with a **cycle-guard visited set** over ids — `parents`/`children` are stored
  data and may loop). A part is emitted iff it has representable geometry, a
  writable attribute, OR ≥1 actually-rendered sub-part; otherwise skip
  (`building_parts_skipped`).
- The parent Building is emitted iff it has representable geometry, a writable
  attribute, OR ≥1 actually-rendered part. A geometry-less parent WITH rendered
  parts emits (the delft fix); a parent whose parts all collapse skips (no
  `<bldg:Building/>` husks).
- A part/Building `gml:id` joins `seen_ids` only when its buffer is actually
  spliced in (extends the existing per-member `gml:id` reservation).
- Distinct `feature_index` per emitted candidate object (parent and each part),
  so `IdAlloc`'s `_cpq_b<idx>_l<major>_p<n>` polygon ids never collide.

### W3. Emission structure + order

```
<cityObjectMember>
  <bldg:Building gml:id="…">
    …attributes…
    …own bldg:lod<major>Solid… (if any)
    …bldg:boundedBy… (own semantics, if any)
    <bldg:consistsOfBuildingPart>
      <bldg:BuildingPart gml:id="…"> …part content… …nested parts… </bldg:BuildingPart>
    </bldg:consistsOfBuildingPart>
    …one per child part, IN THE PARENT'S STORED `children` ORDER…
  </bldg:Building>
</cityObjectMember>
```

- `consistsOfBuildingPart` comes **last** (CityGML 2.0 `building.xsd` sequence:
  scalar props → lod0.. → boundedBy → lod3/4 → interiorRoom →
  consistsOfBuildingPart → address).
- **Parts MUST be emitted in the parent's stored `children` order** (look each
  child id up in the part group). Emitting in row/map order is the top silent
  round-trip failure (the reader reconstructs `children` in document order).

### W4. Counters (`WriteReport` additions)

- `building_parts_written`, `building_parts_skipped` (empty part),
  `building_parts_orphaned` (a part whose `parents[0]` is absent / not a
  Building-or-BuildingPart row), `children_unresolved` (a Building's `children`
  entry with no matching part row — explains a shrunken re-read `children`).
- **BuildingPart rows stop feeding `non_building_skipped`** (they are now
  handled). delft: its ~200 parts move from `non_building_skipped` into the part
  counters — the existing writer test's delft expectations change; restate them.

## Round-trip oracles

1. **Hand-authored `building_with_parts.gml`** (new fixture): a Building with its
   own `lod1Solid` AND two `bldg:BuildingPart`s — one part a plain `lod2Solid`,
   one part with its own `boundedBy` semantics whose solid xlinks its boundedBy
   polygons (exercises per-part xlink registry isolation + per-part
   `chosen_major` + distinct `IdAlloc`). Round-trip → assert per-id
   `parents`/`children`/geometry/semantics equal; plus a **raw-XML assertion**
   that in the parent, `consistsOfBuildingPart` appears after the last
   `lod*Solid`/`boundedBy` (pins element order independent of both codecs).
2. **delft `delft.city.jsonl`** (mandatory, real 3DBAG independence): CityJSON →
   package → `.gml` → package. Assert the parents emit with `children` intact,
   the parts carry geometry, `building_parts_written` == parent count, and
   `non_building_skipped` no longer counts the parts.

## Unit tests (in-code / small)

- Reader: `consistsOfBuildingPart` produces a part CO with `parents`/parent
  `children`; empty/xlink-only wrapper → no part; nested part → correct
  immediate-parent linkage; duplicate id → error; depth bound → error.
- Writer: geometry-less parent with a part emits; parent with all-empty parts
  skips; attributes-only part emits; empty part skipped + counter; orphan part
  counted; parts emitted in `children` order (not row order); cycle guard
  terminates; multi-parent part emits once + counts loss.

## Out of scope (later)

BuildingInstallation / interiorRoom / other child feature types; multi-parent
faithful emission; the pre-existing `boundedBy`-after-lod3/4 XSD-order issue
(documented, not fixed here); appearance (W-M5).

## Known limitations (document in code + paper)

- Empty parts (no geometry, no attribute, no rendered sub-part) are
  skipped-with-counter — a deliberate, reported loss (mirrors empty-Building
  skipping).
- A part with multiple `parents` emits under `parents[0]` only; secondary parent
  links are dropped-with-counter.
- `boundedBy` is emitted adjacent to its chosen LoD's solid; when the chosen
  major is 3 or 4 this is XSD-order-invalid (pre-existing from W-M3), package-
  lossless via the order-independent reader.
