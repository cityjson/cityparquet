# CityGML 2.0 Appearance — W-M5 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development or superpowers:executing-plans. Steps use `- [ ]`.

**Goal:** Round-trip CityGML appearance (materials then textures), feature-local, paired reader+writer.

**Phasing (Fable):** W-M5a materials (self-contained), then W-M5b textures (adds ring ids + UV serialisation), reusing 5a's id/target/oracle plumbing.

## Global Constraints

- Package-level round-trip keyed by id, comparing DEREFERENCED appearance (defs per face/ring, not indices); colours/UVs bit-exact.
- Writer emits `app:appearance` INSIDE each Building/part (feature-local); reader stays single-pass. CityModel-level reading is out of scope.
- Compatibility profile only; Core → skip appearance + counter. Never error — skip-with-counter.
- Emission order in a Building: geometry (with ids) → boundedBy → app:appearance.

---

## Phase W-M5a: Materials

### Task 1: Reader — parse app:X3DMaterial into CityJSON material maps
**Files:** `crates/cityparquet/src/citygml/{building.rs, appearance.rs (new), geometry.rs}`; new fixture `crates/cityparquet/tests/data/building_with_materials.gml`; test `tests/citygml_real_data.rs`.

- Create `crates/cityparquet/src/citygml/appearance.rs` (reader-side): parse `app:Appearance` per theme; `read_x3d_material` → CityJSON material `Value`; collect `(theme, material Value, targets: Vec<gml:id>)`.
- In `RawBuilding`: add a polygon-id → (geom index, face index) registry built as geometries are read (the geometry reader must record, per geometry, the gml:id of each face's polygon in face order). Add `appearance: Vec<ReadMaterial>` (theme, material, targets).
- `into_feature`/`emit_into`: build the feature's CityJSON `appearance` — intern each material via `AppearanceInterner`, resolve each target gml:id → face path, set `geometry.material.{theme}.values[face]`. Every material interned (even target-less). Unresolved target → counter.
- Fixture `building_with_materials.gml`: a Building with a lod2Solid (tetrahedron); an `app:appearance/app:Appearance theme="visual"` with 2 X3DMaterials, one targeting 2 faces, one targeting 1 face; 1 face untargeted (null); plus an unused X3DMaterial.
- **TDD:** convert fixture → read features → assert geometry `material.visual.values` per face + the interned materials.
- Steps: failing test → implement → pass → clippy+fmt → commit.

### Task 2: Writer — extend IdAlloc predicate + emit app:X3DMaterial
**Files:** `crates/cityparquet/src/citygml/writer/{semantics.rs, appearance.rs (new), building.rs, mod.rs}`.

- New `writer/appearance.rs`: given a geometry's `material` map (per theme, per-face index) + the materials table (from the package sidecar, loaded in the driver), emit `app:appearance/app:Appearance` per theme: group face ids by material index, one `app:X3DMaterial` (fields from the table) per used index with `app:target=[#faceid]`, then target-less unused defs.
- Extend the face-id assignment so a face with a material (any theme) gets a `gml:id` (reuse the semantic id if it has one). This requires the solid/multisurface emitters to assign ids to material-bearing faces and expose the per-face id map to the appearance emitter. Design: a pre-pass computes `face_ids: per-geometry per-face Option<gml:id>` from (has-semantics OR has-material), the geometry emitter stamps them, and the appearance emitter consumes them.
- Driver (`mod.rs`): load `materials.parquet` (via the existing sidecar reader) into a materials table; thread it + per-geometry material maps to the object content emitter.
- `WriteReport`: `materials_written`, `appearance_skipped_core_profile`.
- **TDD:** in-code geometry + material map → emit → assert app:X3DMaterial with correct app:target ids; unused def target-less.
- Steps: failing tests → implement → pass → clippy+fmt → commit.

### Task 3: Materials round-trip oracle
**Files:** new `crates/cityparquet/tests/citygml_appearance.rs`.

- `building_with_materials.gml` → package → `.gml` → package: assert per-face DEREFERENCED material def per theme equal; materials table equal as canonical-JSON set; `materials_written` correct.
- Steps: write oracle → run → pass → commit `feat(citygml): W-M5a materials round-trip`.

---

## Phase W-M5b: Textures

### Task 4: Reader — parse app:ParameterizedTexture (ring ids + UV, drop closing pair)
- Extend `read_linear_ring` to capture ring `gml:id`; build ring-id → (geom, face, ring) registry.
- `citygml/appearance.rs`: `read_parameterized_texture` → CityJSON texture def (mimeType↔type, imageURI, wrapMode, borderColor); per `app:target`/`app:textureCoordinates`: parse UV doubles, drop the closing pair, intern into the feature-local UV pool (`f64::to_bits`), write `[texIdx, uvIdx…]` at the ring path.
- Fixture extension: add a textured face (with an interior ring) + texture coords to `building_with_materials.gml` → rename `building_with_appearance.gml` (materials + textures).
- **TDD:** convert → assert `texture.visual.values` per ring + interned textures + UV pool.

### Task 5: Writer — emit app:ParameterizedTexture (ring ids, re-closed UV)
- Extend id predicate to textured faces; assign ring ids `_r<K>` to textured rings (holes included).
- `writer/appearance.rs`: per used texture def one `app:ParameterizedTexture`; per textured polygon `app:target uri` wrapping per-ring `app:textureCoordinates ring="#ringid"` = UV pairs dereferenced from the pool, re-closed (N+1). Unused → target-less. UVs shortest `f64` Display.
- Driver: load `textures.parquet` + the UV pool.
- `WriteReport`: `textures_written`, `texture_seams_dropped`.
- **TDD:** in-code texture map → emit → assert textureCoordinates ring targets + re-closed UVs.

### Task 6: Full appearance oracle (materials + textures) + lod3_railway
- Extend `citygml_appearance.rs`: `building_with_appearance.gml` full round-trip (materials + textures, dereferenced, bit-exact UV); `lod3_railway.city.json` CityJSON→pkg→GML→pkg (dereferenced material/texture per surface + tables as sets; geometry/semantics unchanged).
- Steps: write oracles → run → pass → commit `feat(citygml): W-M5b textures round-trip`.

---

## Final milestone steps

- [ ] Whole-branch review; Codex external review; triage + fix Critical/Important.
- [ ] Finish branch (verify `just check`, merge to `main`, delete branch).
- [ ] Update milestone memory: W-M5 done (materials+textures, feature-local); note W-M5c (CityModel-level appearance pre-scan) as the remaining reader-robustness future work. The CityGML reader+writer round-trip stack is then complete.

## Self-Review

- Coverage: reader materials (1), writer materials (2), materials oracle (3), reader textures (4), writer textures (5), full+scale oracle (6). Matches spec W-M5a/W-M5b.
- Key risk: the face-id pre-pass (semantics OR material OR texture) shared by geometry emission, boundedBy, and appearance — one id per face, referential integrity is what the round-trip needs (not specific ids).
- Losses (documented, counted): CityModel-level appearance (deferred), appearance_defaults (single-theme convention), foreign texture seams / TexCoordGen / exotic mimeTypes, Core profile.
