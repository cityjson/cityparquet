# CityGML 2.0 Appearance — W-M5 Design (materials + textures, paired reader + writer)

**Date:** 2026-07-16
**Milestone:** W-M5 (follows W-M4 BuildingParts) — the last listed milestone
**Status:** approved (full materials+textures; Fable-reviewed feasibility + design)

## Goal

Round-trip CityGML 2.0 appearance — `app:X3DMaterial` (colours) and
`app:ParameterizedTexture` (per-ring UV) — so a package's CityJSON appearance
(materials/textures tables + per-geometry `material`/`texture` maps) survives
CityGML → package → `.gml` → package. Paired reader+writer (the CityGML reader
reads no appearance today). Sequenced **materials first (W-M5a), then textures
(W-M5b)**.

## Global Constraints

- Round-trip invariant is PACKAGE-LEVEL, keyed by id, comparing **dereferenced**
  appearance (material/texture DEFINITIONS per face/ring, not table indices —
  index/pool order legitimately permutes on re-intern). Colours/UVs compare
  bit-exact (shortest `f64` `Display` + `f64::to_bits` dedup, as export.rs
  already does).
- **Feature-local appearance:** the writer emits `app:appearance` INSIDE each
  `bldg:Building`/`BuildingPart` (legal — appearance is a CityObject property),
  keeping the reader single-pass. Reading CityModel-level appearance (a reader
  pre-scan for foreign files) is **out of scope** (future W-M5c).
- Compatibility profile only (materials.parquet/textures.parquet present). On the
  Core profile (definitions dropped), appearance is skipped with a counter.
- Never error on unrepresentable data — skip-with-counter. `just check` gate.
- Real fixtures: a hand-authored CityGML appearance fixture + real
  `lod3_railway.city.json` (CityJSON → package → `.gml` → package).
- Codex review at milestone end.

## Background (verified)

- The package already round-trips CityJSON→package→CityJSON appearance via
  `appearance::AppearanceInterner` (dedup by `canonical_json_string`) and
  export's `LocalAppearance` (UV pool dedup by `f64::to_bits`). Materials/textures
  live in `materials.parquet`/`textures.parquet`; per-geometry
  `geometry_properties.material.{theme}.values` (per-face index, nested to the
  geometry) and `.texture.{theme}.values` (per-face-per-ring `[texIdx, uvIdx…]`).
- The reader's `collect_polygons` already captures each `gml:Polygon`'s `gml:id`;
  `read_linear_ring` (geometry.rs:445) discards ring ids (must capture for
  textures). `semantic_of_polygon` (building.rs) is the precedent for a
  face-gml:id → data map.
- The writer's `write_polygon_with_id` already stamps a `gml:id` on an inline
  polygon; the W-M3 `IdAlloc` assigns ids only to semantic faces.
- CityGML: `app:Appearance`(per `app:theme`) / `app:surfaceDataMember` /
  `app:X3DMaterial`(diffuseColor… + `app:target="#polyid"` list) or
  `app:ParameterizedTexture`(`app:imageURI`, `app:mimeType`, `app:wrapMode`,
  `app:borderColor`; per `app:target uri="#polyid"` an `app:TexCoordList` of
  `app:textureCoordinates ring="#ringid"` = the ring's UV doubles).

## Design — W-M5a: Materials

### Reader (materials)
1. While assembling each geometry, build a per-building map `polygon gml:id →
   (geometry index, face path)` (extends the existing polygon registry).
2. Parse `app:appearance/app:Appearance` per `app:theme` (default theme name
   `""` if absent). Per `app:surfaceDataMember/app:X3DMaterial`: build the
   CityJSON material object (name, diffuseColor, emissiveColor, specularColor,
   ambientIntensity, shininess, transparency, isSmooth — only present fields),
   `intern_material`, and for each `app:target="#id"` resolve the face path and
   set `material.{theme}.values[face] = index`. Faces with no material → `null`.
3. Intern EVERY X3DMaterial encountered, even target-less ones (so unused
   definitions survive) — record the index without setting any face value.
4. Emit the interned materials table + per-geometry `material` maps into the
   feature's CityJSON `appearance` (matching the CityJSON import shape the
   encoder already stores).
5. Unresolvable `app:target` (dangling id, or an id naming a non-polygon
   surface) → `appearance_targets_unresolved` counter, skipped.

### Writer (materials)
1. Extend the face-id predicate: a face gets a `gml:id` if it has semantics OR a
   material in any theme. Semantic faces reuse their existing
   `_cpq_b<f>_l<major>_p<N>` id; a material-only face gets one too (emitted inline
   with `write_polygon_with_id`). One id per face, shared by boundedBy xlink and
   material targets.
2. After geometry + boundedBy, emit one `app:appearance/app:Appearance` per
   theme inside the Building: for each theme, group faces by their material index
   (from `material.{theme}.values`), emit one `app:X3DMaterial` per USED material
   definition (its fields from the materials table) with `app:target` = the
   `#faceid`s assigned to it; then emit remaining UNUSED material definitions as
   target-less `app:X3DMaterial` (in a deterministic theme) so the table
   round-trips.
3. CityJSON `{"value": m}` scalar shorthand (whole-geometry material) → expand to
   all faces on emit (re-import produces the `values` form; the comparator
   canonicalises scalar ≡ expanded).

### Counters (`WriteReport` / reader stats)
- `materials_written` (X3DMaterial elements emitted), `appearance_targets_unresolved`
  (reader), `appearance_skipped_core_profile`.

## Design — W-M5b: Textures

### Reader (textures)
1. Extend `read_linear_ring` to capture the ring's `gml:id`; build `ring gml:id →
   (geometry index, face, ring path)`.
2. Per `app:ParameterizedTexture`: build the CityJSON texture def (type from
   mimeType — `image/jpeg`↔JPG, `image/png`↔PNG; image=imageURI; wrapMode;
   borderColor; textureType), `intern_texture`. For each `app:target
   uri="#polyid"`/`app:textureCoordinates ring="#ringid"`: parse the UV doubles,
   **drop the closing pair** (GML rings are closed; symmetric with
   `read_linear_ring` dropping the closing point), intern each UV into the
   feature-local pool (`f64::to_bits` dedup), and write `[texIdx, uvIdx…]` at the
   ring path of `texture.{theme}.values`.
3. Intern every ParameterizedTexture (even target-less) for unused-def round-trip.

### Writer (textures)
1. Extend the id predicate to also cover textured faces; assign ring ids
   `_cpq_b<f>_l<major>_p<N>_r<K>` to every textured ring (interior rings
   included — CityJSON texture values cover holes).
2. Per used texture def emit one `app:ParameterizedTexture` (`app:imageURI`,
   `app:mimeType`, `app:wrapMode`, `app:borderColor`); per textured polygon an
   `app:target uri="#polyid"` wrapping per-ring `app:textureCoordinates
   ring="#ringid"` = the ring's UV pairs dereferenced from the pool, **re-closed**
   (N+1 pairs, last=first). Unused texture defs → target-less.
3. UVs formatted with shortest `f64` `Display` (bit-exact re-parse); never fixed
   precision.

### Counters
- `textures_written`; `texture_seams_dropped` (a foreign ring whose closing UV ≠
  its first — unrepresentable in CityJSON; own output never produces this).

## Round-trip oracles

1. **Hand-authored `building_with_appearance.gml`** (new): a Building whose
   lod2Solid faces carry (a) two materials across two themes, (b) a material on a
   NON-semantic face, (c) a textured face with an interior ring, (d) an UNUSED
   material definition, (e) a definition shared across two themes (dedup), (f) the
   closed-ring UV repetition. Round-trip → assert per-face **dereferenced**
   material and per-ring **dereferenced** (texture def + ordered UV pairs,
   bit-exact) equal per theme; materials/textures tables equal as canonical-JSON
   sets.
2. **`lod3_railway.city.json`** (CityJSON→pkg→GML→pkg): scale/dedup stress (85
   materials, 34 textures, 35 645 UVs). Assert the dereferenced material/texture
   per surface survives and the tables match as sets; geometry/semantics
   unchanged (regression vs W-M1–M4).

## Out of scope (documented)

- Reading **CityModel-level** appearance (foreign files place it after all
  members) — a reader pre-scan pass, future W-M5c. The paired round-trip is
  unaffected (writer emits feature-local appearance).
- `metadata.appearance_defaults` has no CityGML equivalent: when exactly one
  theme exists, set it as the default deterministically; else drop (documented).
- `app:GeoreferencedTexture`, `app:TexCoordGen` (worldToTexture), `app:isFront`,
  UV **seams** (closing UV ≠ first), and mimeTypes outside PNG/JPG — foreign-file
  losses, skip-with-counter. The writer never emits these, so the paired round
  trip is lossless.
- Core-profile packages (definitions dropped) — appearance skipped with a counter.
- Two materials on one face in one theme (CityJSON holds one index/face/theme):
  last-wins, documented.

## Known limitations (code + paper)

Feature-local appearance emission (not CityModel-level) is CityGML-valid but
differs from the common export convention; the pre-scan reader (W-M5c) is needed
to read appearance from files that place it at CityModel level. Foreign-file
loss classes above are skip-with-counter.
