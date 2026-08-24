# `other` column, `bbox` provenance, and LoD0 provenance — simplification

**Date:** 2026-08-21
**Status:** design, approved in chat; spec review pending
**Scope:** `documents/` (normative spec), `cityparquet-rs`, `duckdb-cityjson`
**Breaking:** yes. Existing CityParquet packages must be rewritten; no back-compat shim.

## Context

A round trip through `cityparquet convert` + `cityparquet export` showed every exported
CityObject carrying a member the source never had:

```json
"cityparquet:lod0_0_source": "geometry_lod1_2"
```

Tracing that surfaced three separable problems in how unmapped data is stored.

1. **Per-row LoD0 provenance is written but never read.** `encode.rs` records the
   column a synthesised footprint was derived from; nothing in `decode`/`export`
   consults it. `decode::merge_other_members` splices every `other` key straight into
   the rebuilt CityObject, so the provenance marker leaves as a top-level CityJSON
   member. `TESTING.md` documents the consequence: round-trip equality only holds with
   `--no-lod0`.
2. **duckdb-cityjson duplicates attributes into `other`.** `city_object_utils.cpp:69`
   filters with `IsPredefinedColumn` — seven names — instead of the full emitted column
   set, so every ordinary attribute that already has its own column is copied into
   `other` as well. `COPY TO cityjson` reads only `geographicalExtent` back out of
   `other`, so the duplicates are written nowhere. Dead weight in every scan.
3. **Two columns for "unmapped stuff", with different shapes.** `other` holds
   unmapped top-level *members*, restored as members on export. `other_attributes`
   (G12) holds attributes whose name collides with a reserved column, restored into
   `attributes{}`. Two columns, two decode paths, for data that is almost always
   absent.

### What the corpus actually contains

Measured before designing, across every fixture and bench dataset:

| Question | Result |
|---|---|
| Unknown top-level CityObject members | **0** across 1,494,667 objects in 15 datasets |
| `+`-prefixed extension attributes | 0 (they become `ex_` columns; never reach `other`) |
| Objects declaring `geographicalExtent` | 3DBAG family: roots only (50% of objects). Helsinki, Zurich: every object. 3DBV, Ingolstadt, Montreal, Railway, Vienna, lod3_railway: none |

The complete set of CityObject members in the official CityJSON 2.0 schema
(`cityobjects.schema.json`, all 34 classes) is `type`, `attributes`, `geometry`,
`children`, `children_roles`, `parents`, `geographicalExtent`, `address` — every one of
which CityParquet already maps to a column. The catch-all therefore has no legal
content; it can only receive a producer's undefined member, or a member introduced by a
future CityJSON version. (The schema does not set `additionalProperties: false`, so such
a file still validates — the member is undefined, not illegal.)

## Decisions

### D1 — `other_attributes` is removed; `other` is the single escape hatch

`other` becomes one nullable JSON map, always present in the schema. Its **reader**
behaviour is the whole of its definition:

> Every entry in `other` is restored into the object's `attributes` on export, keyed by
> its map key.

Writers fill it with whatever they cannot map to a column — a reserved-name-colliding
attribute, an undefined source member — and the two are deliberately **not**
distinguished in the column. A reader must not attempt to infer which is which: a
CityParquet file may come from any writer, and how a foreign writer encoded an undefined
member is outside this specification. This follows the project's conformance line —
writers may vary, readers read general CityParquet.

Consequences:

- `other_attributes` stops being a column and stops being a reserved name. A source
  attribute literally named `other_attributes` becomes an ordinary attribute column.
- `other` remains reserved: an attribute named `other` has nowhere to divert to and is
  rejected outright (the old self-collision rule, inherited unchanged).
- The §2 paragraph making `other_attributes` the single exception to "optional data is
  `NULL`, not an omitted column" is deleted; `other` is always present, nullable.
- The decode-side reserved-key guard **inverts**. `merge_other_members` currently errors
  if `other` carries `children` or `bbox`; under the new semantics those keys are its
  legitimate content. `OTHER_RESERVED_MEMBERS` dies as a decode guard, and
  `merge_other_attributes`' logic becomes the decode path for `other`.
- Round-tripping an undefined top-level member normalises it into `attributes`. This is
  accepted: it is the price of one column with one reader rule.

### D2 — `geographicalExtent` leaves `other` and becomes `bbox`

- **Import:** `bbox` is the **union** of the object's stored geometry (its whole
  subtree, over all stored LoDs) and the source's declared `geographicalExtent` where
  the source declares one. A declared extent may therefore only ever *widen* the box,
  never narrow it.
- **Export:** `geographicalExtent` is emitted from `bbox`, on every row that has one.
- `geographicalExtent` no longer appears in `other`; `compare` treats it as derived and
  excludes it.

The union rule keeps the source's numbers verbatim wherever the source extent is
already a superset of the geometry, while guaranteeing the superset invariant §3
depends on. Measured justification for not taking the declared value verbatim is in
T1 — in the 3DBAG family the declared extent is not merely imprecise but describes the
wrong object, on 100% of sampled rows.

### D3 — per-row LoD0 provenance is removed

`encode.rs` stops writing `cityparquet:lod0_0_source`. No tolerate-and-strip shim:
existing packages get rewritten.

The **dataset-level** flag (`cityparquet:lod0_synthesis` in `city.other`) stays. It is
footer metadata, never enters an exported CityObject, and lets a consumer detect that
synthesis happened without scanning rows.

With per-row provenance gone, the `cityparquet:` key-namespace clause in §2's `other`
row loses its only referent and is deleted too.

### D4 — LoD0 synthesis stays on by default

Not changed here. It remains an open item to confirm with the supervisor, tracked
separately. Removing D3's provenance key makes the round-trip diff geometry-only, which
is what `TESTING.md`'s gotcha shrinks to.

## Changes by area

### `documents/` (normative)

| File | Change |
|---|---|
| `02-object-table-schema.mdx` | Drop the `other_attributes` row; rewrite the `other` row (reader rule, no `cityparquet:` clause); delete the "single exception" paragraph; drop the `other_attributes`-is-reserved bullet from the naming rules |
| `03-geometry-semantics.mdx` | Delete the LoD0 per-row provenance bullet; add the source-extent union to the `bbox` rules (the superset guarantee is unchanged) |
| `07-mapping-cityjson.mdx` | `geographicalExtent` → `bbox`, not `other`; "other object members" → the new `other` rule |

Spec prose stays format-level and reader-facing: no implementation status, no measured
numbers, no migration notes.

### `cityparquet-rs`

| File | Change |
|---|---|
| `src/encode.rs` | Drop `lod0_source_column` threading and the provenance insert; add `geographicalExtent` to the set `unmapped_from_json` strips (it is currently kept); merge `collect_diverted_attributes` output into the single `other` column; retire `OTHER_RESERVED_MEMBERS` as a decode guard |
| `src/decode.rs` | Delete `merge_other_attributes`; rewrite `merge_other_members` to restore every entry into `attributes`; drop the reserved-key error and the `geographicalExtent` shape check |
| `src/encode.rs` (bbox) | Union the source `geographicalExtent` into the computed subtree union |
| `src/export.rs` | Emit `geographicalExtent` from `bbox` |
| `src/compare.rs` | Exclude `geographicalExtent` as derived |
| `src/lod0.rs`, `synthesize_footprint` | Stop returning the source column name |
| `tests/lod0_synthesis.rs:335-370` | Delete the provenance test |
| `docs/design.md`, `TESTING.md` | Update the LoD0 and round-trip sections |

### `duckdb-cityjson`

| File | Change |
|---|---|
| `city_object_utils.cpp` | `other` = reserved-name-colliding attributes only (this reader parses no undefined top-level members, so it never synthesises any); filter against the **emitted column set**, not `IsPredefinedColumn`, so a case-collision loser is not silently dropped; remove the `geographicalExtent` insert |
| `scan_function.cpp` | `bbox` unions `obj.geographical_extent` into `GetObjectExtent`/`GetGeometryExtent` |
| `copy_function.cpp` | Restore `other` into `attributes{}` (fixes existing silent loss of colliding attributes); rebuild `geographicalExtent` from `bbox` only; delete the `other` branch at :1252 |
| `column_types.cpp` | Drop `other_attributes` from `IsReservedColumnName` |
| `lod_table.cpp/.hpp` | Update the `GetTrailingColumns` comments |
| `fcb_selective_convert.cpp`, `flatcitybuf_reader.cpp` | Follow the new `other` semantics in the FCB paths |

## Accepted trade-offs

**T1 — why the declared `geographicalExtent` is unioned rather than taken verbatim.**
§3 requires that `bbox` be a **superset** of everything the row stores, or pruning is
unsound. Measured against that invariant on a 400-object sample per dataset:

| Dataset | Objects checked | Declared extent contains the geometry | Violates | Worst overshoot |
|---|--:|--:|--:|--:|
| `delft.city.jsonl` | 400 | 0 | **100%** | **36.51 m** |
| `3DBAG.city.jsonl` | 400 | 0 | **100%** | 9.34 m |
| `9-284-556.city.json` | 400 | 0 | **100%** | 8.79 m |
| `Zurich.city.jsonl` | 400 | 76 | 81% | 0.001 m |
| `Helsinki.city.jsonl` | 400 | 138 | 65.5% | 9.20 m |

Zurich's violations are float rounding (1 mm). The 3DBAG family's are **semantic**.
Worked case, `NL.IMBAG.Pand.0503100000030621` — an L-shaped Pand with two
`BuildingPart`s:

```
declared geographicalExtent : x[85325.086, 85353.328]  y[446861.531, 446889.969]
parent's own LoD0 footprint : x[85293.140, 85353.332]  y[446825.024, 446889.957]
child …-0                   : x[85325.087, 85353.332]  y[446861.528, 446889.957]   <-- identical to declared
child …-1                   : x[85293.258, 85321.634]  y[446825.024, 446853.443]   <-- entirely outside declared
```

The declared extent equals the extent of **one of the two children**. It covers neither
the sibling part nor the parent's own LoD0 footprint, and is wrong by 36 m. Taken
verbatim as `bbox` it would give this building a box over roughly a quarter of its true
footprint, and a spatial query over the western half of the building would return
nothing.

Under D2's union rule this is contained: 3DBAG rows fall back to the computed box,
Helsinki's genuinely-loose declared boxes are preserved verbatim, §3's invariant holds
unchanged, and §2's worked example (a `Building` whose `bbox` spans its part's z-range)
stays true.

**T2 — undefined top-level members normalise into `attributes` on round trip.** A
consequence of D1's single reader rule. Fires on no dataset in the corpus.

**T3 — every exported object gains a `geographicalExtent`.** Including objects whose
source had none (3DBAG's `BuildingPart`s). This is an enrichment of the same class as
the LoD0 marker D3 removes; it is intended here, and `compare` excludes the field.

## Testing

Strict red-green TDD, real fixtures only (`delft.city.jsonl`, `lod3_railway.city.json`)
— no inline artificial CityJSON.

- Round trip `convert` → `export` → `compare` on both fixtures with LoD0 synthesis on:
  the only remaining difference is the synthesised geometry.
- A source attribute named `bbox` survives convert → export back inside `attributes`.
- A source object with an undefined top-level member exports it as an attribute.
- `other` is null on every row of both fixtures.
- duckdb-cityjson: one test per reader (`read_cityjson`, `read_cityjsonseq`,
  `read_flatcitybuf`) asserting `other IS NULL` where no collision exists, and that no
  attribute with its own column is duplicated into `other`.
- `just interop` and the rs↔duckdb cross-module round trip still pass.

## Out of scope

- Whether LoD0 synthesis should default on (D4).
- Any change to `materials`/`textures`/`geometry_templates` sidecars.
