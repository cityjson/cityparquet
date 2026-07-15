# CityGML 2.0 BuildingParts — W-M4 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development or superpowers:executing-plans. Steps use `- [ ]`.

**Goal:** Round-trip `bldg:BuildingPart` (reader reads `consistsOfBuildingPart`; writer emits it), fixing the geometry-less-parent skip.

**Architecture:** Reader recursion into a `parts` tree flattened to parent+part CityObjects in one feature; writer groups parts by parent and emits them bottom-up in `children` order.

## Global Constraints

- Package-level round-trip, keyed by id (never row order).
- Real fixtures: new hand-authored `building_with_parts.gml` + real `delft.city.jsonl`.
- Never error on unrepresentable data — skip-with-counter. `just check` is the gate.
- Parts emitted in the parent's STORED `children` order; `consistsOfBuildingPart` LAST inside `bldg:Building`.

---

### Task 1: Reader — `consistsOfBuildingPart` recursion + parent/part CityObjects

**Files:**
- Create: `crates/cityparquet/tests/data/building_with_parts.gml` (hand-authored fixture)
- Modify: `crates/cityparquet/src/citygml/building.rs` (parameterise `read_building`; recursion; `parts` tree; `into_feature`)
- Modify: `crates/cityparquet/src/citygml/reader.rs` (top-level call uses the parameterised form)
- Test: `crates/cityparquet/tests/citygml_real_data.rs` (parts parsing)

**Fixture** (`building_with_parts.gml`): a `CityModel` (default core ns + bldg/gml/xlink) with one `bldg:Building gml:id="B"` that has its own `bldg:lod1Solid` (a small box) AND two `bldg:consistsOfBuildingPart`:
- `bldg:BuildingPart gml:id="B_p1"` with a `bldg:lod2Solid` (a box).
- `bldg:BuildingPart gml:id="B_p2"` with `bldg:boundedBy` semantic surfaces (a WallSurface + RoofSurface) whose `lod2Solid` xlinks the boundedBy polygons (like `b1`).
Use real-ish metric coordinates.

**Interfaces (produced):** `RawBuilding.parts: Vec<RawBuilding>`; `read_building(reader, buf, id, end_name: &[u8])`; `into_feature` emits parent + parts.

- [ ] **Step 1: failing test** — a `parts_round_trip` test in `citygml_real_data.rs`: `convert(building_with_parts.gml)` then read the package; assert a `Building` CO `B` with `children` containing `B_p1`,`B_p2`, and two `BuildingPart` COs with `parents:["B"]`, each with geometry; `B` has its own lod1 solid geometry.
- [ ] **Step 2: run → FAIL** (parts skipped today).
- [ ] **Step 3: implement**
  - Add `end_name: &[u8]` param to `read_building`; break on `End` whose local name == `end_name`. Top-level caller passes `b"Building"`.
  - Add a branch: `bldg && name == b"consistsOfBuildingPart"` → `read_consists_of_part(reader, buf, depth)`: loop for the inner `Start(bldg:BuildingPart)`, capture `gml_id`, `read_building(..., b"BuildingPart")` with `depth+1`, push to `b.parts`; consume `End(consistsOfBuildingPart)`. If the wrapper is empty/xlink-only (no BuildingPart child before its End), produce no part. Bound depth (const `MAX_PART_DEPTH = 32`) → `CityParquetError::Schema` on exceed.
  - `RawBuilding` gains `parts: Vec<RawBuilding>`.
  - `into_feature`: refactor to `into_feature` (top level) that builds a shared `VertexBuilder`, then a recursive `emit_object(&self, part_id, parent_id, is_root, vb, feature)` that: builds this object's CityObject (`type` "Building" if root else "BuildingPart"; geometry via existing `build_*`; attributes; `parents` = `[parent_id]` unless root; `children` = child part ids in order), inserts it into the feature (error on duplicate id), and recurses into `self.parts` (child ids synthesised `<this_id>_part_<i>` when a part lacks a gml:id). `feature.id` = root id.
- [ ] **Step 4: run → PASS.** **Step 5: clippy+fmt.** **Step 6: commit** `feat(citygml): W-M4 reader — read consistsOfBuildingPart`.

---

### Task 2: Writer — extract shared abstract-building content emitter (no behaviour change)

**Files:** Modify `crates/cityparquet/src/citygml/writer/building.rs`.

Refactor `write_building` so the attributes + geometry emission (everything between `<bldg:Building>` open and close, i.e. the current attribute buffering + the by_major emit loop) becomes:
`fn write_object_content<W>(w, b: &BuildingContent, types, feature_index, bounds, report) -> Result<bool>` — returns whether it emitted anything (geometry or a writable attribute). `write_building` keeps the `cityObjectMember`/`bldg:Building` wrapper + NCName check + emptiness decision, calling `write_object_content`.

Rename `BuildingSolids` → `BuildingContent` (it now backs both a Building and a part) OR keep the name; either way it holds `id`, `attributes`, `solids`. Keep all existing unit tests green (pure refactor).

- [ ] Steps: refactor → run existing writer lib tests (all green, no new behaviour) → clippy+fmt → commit `refactor(citygml): W-M4 extract write_object_content`.

---

### Task 3: Writer — group parts, bottom-up rendering, emit nested in `children` order

**Files:**
- Modify: `crates/cityparquet/src/citygml/writer/building.rs` (a `write_building_with_parts` that renders parts bottom-up and nests them)
- Modify: `crates/cityparquet/src/citygml/writer/mod.rs` (`WriteReport` counters; `write_package` collects all rows, groups parts, drives emission)

**Interfaces:** `write_building_with_parts(w, parent: &BuildingContent, parts_by_id: &HashMap<String, &BuildingContent>, child_order: &[String], next_feature_index, bounds, report, visited) -> Result<bool>` — renders each child (from `child_order`, looked up in `parts_by_id`) into a buffer recursively (its own `children` from its stored children), a part emits iff geometry/attribute/≥1 rendered sub-part; the parent emits iff geometry/attribute/≥1 rendered part; nests `consistsOfBuildingPart/bldg:BuildingPart`; cycle-guard via `visited: &mut HashSet<String>`.

**`WriteReport`:** add `building_parts_written`, `building_parts_skipped`, `building_parts_orphaned`, `children_unresolved`.

**`write_package`:** decode ALL rows into `Vec<DecodedObject>`; build `by_id: HashMap<String, DecodedObject>` and, for each Building, its ordered child ids from `obj.object.children` (a child id present in `by_id` as a `BuildingPart` → part; absent → `children_unresolved`; a `BuildingPart` whose `parents[0]` is absent/non-Building → `building_parts_orphaned` when it is never emitted under any parent). Emit each Building via `write_building_with_parts`. BuildingPart rows are NOT counted in `non_building_skipped`.

- [ ] **Step 1: failing tests** (writer lib): geometry-less parent + one part → emits `<bldg:Building>` with `<bldg:consistsOfBuildingPart><bldg:BuildingPart>`; parent with all-empty parts → skipped; parts emitted in `children` order (child_order `[p2,p1]` emits p2 before p1); cycle guard terminates.
- [ ] **Step 2: run → FAIL. Step 3: implement. Step 4: run → PASS.**
- [ ] **Step 5: `just check`. Step 6: commit** `feat(citygml): W-M4 writer — emit consistsOfBuildingPart`.

---

### Task 4: Round-trip oracles + delft + edge unit tests

**Files:**
- Create: `crates/cityparquet/tests/citygml_buildingparts.rs` (hand-fixture round-trip + delft)
- Modify: existing writer/reader tests whose delft/BuildingPart counter expectations change.

**Hand-fixture oracle:** `building_with_parts.gml` → package → `.gml` → package; per-id assert `parents`/`children`/geometry/semantics equal; a raw-XML assertion on the emitted `.gml` that `consistsOfBuildingPart` index > the last `lod` / `boundedBy` index in the parent.

**delft oracle:** `convert(delft.city.jsonl)` → package → `.gml` (write_package) → package2; assert: `building_parts_written` == the delft part count; every parent Building re-reads with its `children` intact; parts carry geometry; `non_building_skipped` does not count the parts. (Use a keyed comparison of a sampled subset for geometry to keep the test fast, or full parents/children equality.)

- [ ] Steps: write oracle tests (run last, after Tasks 1–3) → run → PASS → fix any delft counter expectations in existing tests → `just check` → commit `feat(citygml): W-M4 round-trip oracles (hand fixture + delft)`.

---

## Final milestone steps

- [ ] Whole-branch review; Codex external review (`codex exec … --sandbox read-only`); triage + fix Critical/Important.
- [ ] Finish branch (verify `just check`, merge to `main`, delete branch).
- [ ] Update milestone memory: W-M4 done; W-M5 = appearance/material next.

## Self-Review

- Spec coverage: reader recursion (1), content refactor (2), parts orchestration + counters (3), oracles + delft (4). All R1/R2/W1-W4 covered.
- Type consistency: `RawBuilding.parts`, `write_object_content`, `write_building_with_parts`, the four part counters — consistent across tasks.
- Edge cases as counters: empty part skipped, orphan, unresolved child, cycle guard.
