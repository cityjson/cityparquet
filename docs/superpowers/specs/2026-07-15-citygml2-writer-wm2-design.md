# CityGML 2.0 Writer — W-M2 (Attributes + CompositeSolid) Design

**Date:** 2026-07-15
**Milestone:** W-M2 (follows W-M1: `CityModel` + envelope + `bldg:Building`/`bldg:lod<major>Solid`)
**Status:** approved (scope + XSD decision confirmed by user)

## Goal

Extend the native CityGML 2.0 writer (`crates/cityparquet/src/citygml/writer/`) so that a
CityParquet package round-trips **building attributes** and **CompositeSolid** geometry back
to CityGML 2.0, in addition to the plain `gml:Solid` geometry W-M1 already emits. `MultiSolid`
has no faithful CityGML 2.0 `Building` representation and is skipped-with-counter (a citable
representational-asymmetry finding for the paper).

Scope is exactly two features. Semantic surfaces (`bldg:boundedBy`), BuildingParts, and
appearance are explicitly **deferred to W-M3**.

## Global Constraints

- **Round-trip invariant is PACKAGE-LEVEL, not XML-identical.** Correctness means: a real
  CityGML fixture → package → `.gml` (writer) → package (re-convert) produces two packages
  whose stored geometry and attributes are equal, keyed by `(building_id, major_lod)`. The
  intermediate XML need not be byte-identical to the source.
- **Real fixtures only for round-trip oracles** (`no inline artificial CityJSON files`).
  In-code `DecodedGeometry` / attribute values in unit tests are permitted (they match the
  existing writer unit-test style, e.g. `tri_solid()`), because they are not CityJSON input
  files.
- **Strict red-green TDD**, frequent commits, one behaviour per test.
- **Never error on unrepresentable data; skip-with-counter.** Real CityJSON-origin packages
  legitimately carry attributes and geometries CityGML 2.0 cannot express. A conversion must
  produce a valid document and count what it dropped, never abort.
- **Float formatting must be shortest-round-trip** (Rust `Display` / `ryu`). Fixed-precision
  formatting (`format!("{:.6}", …)`) is **forbidden** — it is the only thing that breaks
  numeric package equality (the reader trims then `parse::<f64>()`, the exact inverse of
  shortest-round-trip output).
- `just check` (clippy `-D warnings` + fmt + tests + schema isolation) is the green gate.
- Codex external review at the end of the milestone (standing project instruction).

## Background: what the package stores (verified)

- Attributes are stored as **typed Arrow columns** via `AttributeInferer`
  (`crates/cityparquet-schema/src/attributes.rs`). The type alphabet is
  `Boolean | Int64 | Float64 | Date | Timestamp | String | StringList | Json`, with a
  promotion lattice (`Int64+Float64 → Float64`; any mixed/unsafe → `Json`; all-null →
  `String`). On decode (`decode.rs`), each non-null cell is reconstructed into
  `obj.object` (a `cjseq::CityObject`) as a JSON value: `Date32 → "%Y-%m-%d"` string,
  `Timestamp(ms,UTC) → RFC3339 Z` string, `List<Utf8> → JSON array`, `Utf8`+`arrow.json`
  → parsed `Value`, scalars → matching JSON scalar. **Round-trip equality is defined over
  the re-inferred `name → AttributeType` map and the values**, because re-conversion re-runs
  inference.
- The native CityGML **reader** parses attributes as: typed `bldg:` local names
  `function/usage/class/roofType/yearOfConstruction/yearOfDemolition → String`,
  `measuredHeight → Float`, `storeysAboveGround/storeysBelowGround → Int`; `gen:` generics
  `stringAttribute/dateAttribute/uriAttribute → String`, `intAttribute → Int`,
  `doubleAttribute/measureAttribute → Float`, keyed by the `name=` attribute; repeats
  accumulate into arrays; empty/whitespace values are dropped; the reader has **no boolean
  attribute type** and **drops `uom`** entirely.
- The reader emits only CityJSON `Solid` and `CompositeSolid` — never `MultiSolid`. A `Solid`
  is stored as WKB `PolyhedralSurface` + `geometry_properties {type:"Solid",
  solid_shell_faces:[flat face counts]}`. A `CompositeSolid`/`MultiSolid` is stored as WKB
  `GeometryCollection` of `PolyhedralSurface` members + `geometry_properties
  {type:"CompositeSolid"|"MultiSolid", solid_shell_faces:[[shells of solid0], …]}` (nested).
  `export::shell_faces_nested` reads the nested form; `partition_shells` splits a member's
  flat faces into shells.

## Design

### A. Attribute serialisation — route by stored type

New module `crates/cityparquet/src/citygml/writer/attributes.rs`. Input: the building's
`obj.object.attributes` (a `serde_json::Map<String, Value>`), where each value's JSON shape
reflects its stored column type. Emit into the open `bldg:Building` **before** the geometry
(the CityGML `_CityObject` generic attributes and `bldg:` properties precede `bldg:lodNSolid`
in document order; keep all attributes ahead of geometry to respect the XSD sequence).

**Routing rule (per key, all-or-nothing):**

1. Determine the value's stored type from its JSON shape (this must mirror how `decode`
   built the value, so use serde's own discriminators, not numeric range):
   - bool → Boolean (unwritable)
   - number with `is_i64()` (or `is_u64()`) → Int64 (an Int64 column decodes to an integer
     `Number`)
   - number with `is_f64()` → Float64 (a Float64 column decodes to a float `Number`, e.g.
     `8.0`, which is i64-representable in value but must route Float64)
   - string → String (Date/Timestamp are stored as strings and re-inferred; treat as String
     for element choice, except a value matching `^\d{4}-\d{2}-\d{2}$` uses `gen:dateAttribute`
     so it re-infers as Date)
   - array of all-strings, length ≥ 2 → StringList (writable)
   - array of all-strings, length ≤ 1 → **unwritable** (re-infers as scalar String / null,
     flipping the column type)
   - object, or any other array → Json (unwritable)
2. **Unwritable** (Boolean, Json, single/empty StringList): skip, increment
   `attributes_skipped`, do not emit.
3. **Empty/whitespace-only string**, or a string containing an **XML-1.0-illegal control
   character** (U+0000–U+0008, U+000B, U+000C, U+000E–U+001F): skip, increment
   `attributes_skipped`.
4. Otherwise choose the element:
   - If the key is a **known typed `bldg:` name** AND the stored type equals the type the
     reader forces back for that name (String for the string-forced names, Float64 for
     `measuredHeight`, Int64 for `storeys*`): emit the `bldg:` element. `measuredHeight` gets
     `uom="m"`. (A `storeys*` value must be a non-negative integer to be schema-clean; a
     negative one falls through to `gen:intAttribute`.)
   - Otherwise emit the `gen:` element **of the stored type**:
     `String → gen:stringAttribute`, `Int64 → gen:intAttribute`,
     `Float64 → gen:doubleAttribute`, Date-shaped string → `gen:dateAttribute`. Each
     `gen:` attribute is `<gen:XAttribute name="KEY"><gen:value>V</gen:value></gen:XAttribute>`.
5. **Arrays** (writable StringList, or a `bldg:function`/`bldg:usage` array): emit one element
   per item, using the same route for every item, preserving item order (the reader
   accumulates repeats in document order).

Numbers are formatted with shortest-round-trip `Display`. `null` values never appear in the
map (decode omits nulls), so no explicit null handling is needed; if one is present it is
skipped without counting (it self-heals to an absent → null column).

**Why route by stored type, not name:** a CityJSON-origin integer `yearOfConstruction`
(reader forces `bldg:yearOfConstruction` to String) must be written `gen:intAttribute` to
re-infer as Int64; a CityGML-origin string `yearOfConstruction` is written `<bldg:…>`. Both
preserve package equality; name-first routing would flip the column type and fail the oracle.

### B. CompositeSolid / MultiSolid geometry

In the driver (`writer/mod.rs`), the current `GeometryCollection` arm counts
`composite_solids_skipped` and drops the column. Replace that: a `GeometryCollection` at a
`Some(lod)` is a candidate composite geometry carried into `write_building` alongside the
`PolyhedralSurface` solids. `write_building` maps it to a major LoD the same way as a Solid
(highest-minor-wins per major; `maxOccurs=1` on `bldg:lodNSolid`).

New geometry writer in `writer/geometry.rs`:

- `write_composite_solid(w, coords, members, props)`:
  - Require `geometry_properties.type == "CompositeSolid"`; a `"MultiSolid"` is never routed
    here (the driver skips it — see below); any other type errors (corrupt file).
  - Reject a zero-member collection (`gml:solidMember` is `minOccurs=1`).
  - Read nested shell counts via `export::shell_faces_nested`; for member `m` use
    `counts[m]`, mismatched length → error (mirrors `export`).
  - Emit `<gml:CompositeSolid>` then, per member, `<gml:solidMember>` wrapping a `<gml:Solid>`
    built from that member's shells (reuse the existing `write_solid` shell/exterior/interior
    logic, factored so both a top-level Solid and a CompositeSolid member share it). A
    single-member CompositeSolid stays a CompositeSolid (do not collapse to Solid).

`bldg:lod<major>Solid` wraps a `gml:CompositeSolid` exactly as it wraps a `gml:Solid`
(`bldg:lodNSolid` is `gml:SolidPropertyType` → `gml:_Solid`; `gml:CompositeSolid` substitutes
for `gml:_Solid`, so this is schema-valid).

**MultiSolid:** in the driver, a `GeometryCollection` whose `geometry_properties.type` is
`"MultiSolid"` is skipped with a dedicated `multi_solids_skipped` counter (CityGML 2.0
`Building` has no `lodNMultiSolid`; `gml:MultiSolid` is a geometric aggregate, not a
`gml:_Solid`). A building whose *only* geometry is a MultiSolid still emits as an
attributes-only Building (all geometry is optional) and is **not** counted in
`buildings_without_solid_skipped` unless it also has no attributes and no other solid — keep
the existing "emitted nothing" rule: a Building element is written iff it has at least one
attribute or one emittable solid/composite.

### C. Report counters (`WriteReport` additions)

- Replace the single `composite_solids_skipped` with:
  - `composite_solids_written: usize` — `gml:CompositeSolid` emitted.
  - `multi_solids_skipped: usize` — MultiSolid geometry columns skipped.
- `attributes_written: usize`, `attributes_skipped: usize`.

Existing counters (`buildings_written`, `non_building_skipped`, `lod_columns_skipped`) keep
their meaning; a same-major-LoD collision on a composite is counted in `lod_columns_skipped`
like a Solid one. **`buildings_without_solid_skipped` narrows in meaning**: because an
attributes-only Building now emits, this counter now means "no emittable geometry **and** no
writable attribute". W-M1's Ingolstadt oracle is unaffected (all 3 buildings have both a solid
and attributes, so `buildings_written == 3` still holds).

### D. Round-trip oracles (real fixtures)

1. **Attributes** — extend the existing W-M1 oracle
   `crates/cityparquet/tests/citygml_writer_real_data.rs` (fixture
   `savenow_ingolstadt_lod2.gml`: `measuredHeight` Float, `roofType` String,
   `storeysAboveGround` Int, ~195 `gen:stringAttribute` String across 3 buildings). Add a
   second assertion: the per-building attribute map (from `obj.object.attributes`) is equal
   across the round trip, keyed by `building_id`. This exercises the Float / Int / String /
   `gen:` routes on real data.
2. **CompositeSolid** — new test using `tests/fixtures/b1_lod2_cs_w_sem.gml` (one Building,
   one `bldg:lod2Solid`/`gml:CompositeSolid` with 2 `gml:solidMember`/`gml:Solid`, no
   attributes). Assert `report.composite_solids_written == 1`, then geometry equality on the
   1 mm grid keyed by `(building_id, major_lod)` across `gml → package → gml → package`. Copy
   the fixture into `crates/cityparquet/tests/data/` (the crate's fixture dir) or reference it
   via the workspace root, consistent with how the other tests resolve fixtures.
3. Both oracles assert the relevant skip counters **exactly** (`multi_solids_skipped == 0`,
   `attributes_skipped == 0` for these all-representable fixtures) so a silent drop fails.

### E. Unit tests (in-code values)

- Attribute routing: each branch — `bldg:` type-match, `gen:` type-mismatch fallback (Int64
  `yearOfConstruction` → `gen:intAttribute`), Float64 `measuredHeight` with `uom="m"`,
  Int64 `storeys`, Date-shaped string → `gen:dateAttribute`, StringList≥2 → repeated
  `gen:stringAttribute`, and each skip case (Boolean, Json/object, single-element list,
  empty string, control char) incrementing `attributes_skipped`. Float shortest-round-trip
  (`8.0 → "8"`, `-0.0`, a long-decimal value) re-parses exactly.
- `write_composite_solid`: 2-member happy path (two `gml:solidMember`), single-member stays
  CompositeSolid, zero-member errors, nested-count mismatch errors, `type != "CompositeSolid"`
  errors.
- Driver: a `type:"MultiSolid"` `GeometryCollection` increments `multi_solids_skipped` and
  emits no `gml:MultiSolid`/`gml:CompositeSolid`; a MultiSolid-only building with an
  attribute still emits an attributes-only Building.

## Out of scope (W-M3+)

Semantic surfaces (`bldg:boundedBy`), BuildingParts (parent/child grouping), appearance
(material/texture), `uom` fidelity, XSD validation of emitted output, and any non-Building
CityObject type.

## Known limitations (document in code + paper)

- CityGML 2.0 `Building` cannot represent a CityJSON `MultiSolid` (skipped-with-counter) —
  a representational asymmetry worth citing.
- `measuredHeight` `uom` is fabricated as `"m"` on write (the reader drops `uom`); a source in
  non-metre units is silently restamped. Pre-existing reader lossiness.
- Boolean and heterogeneous/nested (`Json`) attributes, single-element string lists, and
  empty-string attributes have no round-trip-stable CityGML 2.0 form and are
  skipped-with-counter.
