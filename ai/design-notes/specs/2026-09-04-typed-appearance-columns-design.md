# Typed, WKB-face-aligned appearance columns — design

Date: 2026-09-04. Status: approved design; implemented in three phases
(specification, `cityparquet-rs`, `duckdb-cityjson`).

## Problem

The specification describes `material_lod*` / `texture_lod*` as JSON cells whose
`values` are **flat per WKB face**, aligned like `face_semantics`. Neither
implementation writes that: `cityparquet-rs` (`crates/core/src/appearance.rs`)
and `duckdb-cityjson` (`scan_function.cpp`, `appearance_normalise.cpp`) both
keep CityJSON's per-geometry-type nesting — per shell for a `Solid`, per solid
and shell for a `MultiSolid`. A consumer therefore needs the geometry type to
index a face's material, which is exactly what the flattened `face_semantics`
design exists to avoid, and the mesh writers in `duckdb-cityjson` carry a
depth classifier to accept both shapes.

The columns are also JSON text where the values have a fixed, well-known shape;
the specification's own note on the sidecar tables says why typed columns beat
JSON for such shapes (pushdown, engine-side validation, compression).

## Decision

One positional model for everything attached to a WKB face: `face_semantics`,
materials and textures are all **flat per WKB face, in WKB face order**, and
the two appearance columns become typed Parquet columns.

### Column types (normative)

Per LoD, beside `geometry_lodX_Y` and `geometry_properties_lodX_Y`:

| Column | Type |
| --- | --- |
| `material_lodX_Y` | `MAP<VARCHAR, LIST<BIGINT>>` |
| `texture_lodX_Y` | `MAP<VARCHAR, LIST<LIST<STRUCT<id BIGINT, uv LIST<LIST<DOUBLE>>>>>>` |

- The map key is the **theme** (CityJSON's theme name; `""` for the unnamed
  theme). Themes are an open set, which is what a MAP expresses and a STRUCT
  cannot.
- `material`: one entry per WKB face, the sidecar `materials.parquet` `id` or
  NULL. CityJSON's `{"value": n}` broadcast is expanded by the writer to one
  entry per face.
- `texture`: one entry per WKB face; each entry is a list over that face's
  rings in `PolygonZ` ring order (exterior first); each ring is a struct with
  the sidecar `textures.parquet` `id` and one `[u, v]` pair per ring vertex, in
  ring vertex order. A ring with no texture is `{id: NULL, uv: NULL}`.
- A cell is NULL when the geometry carries no material (or texture) in any
  theme. A theme absent from the map has no entries for that geometry.
- Ids are sidecar `id` values, matched against the sidecar's `id` column,
  never row positions (unchanged). They are `BIGINT` because the sidecar `id`
  columns are `BIGINT` ("BIGINT throughout", the resolved sidecar-id question):
  a narrower reference type would leave ids above 2³¹ unreferencable, and the
  object table's `template.id` is already `BIGINT`. UV coordinates are inlined
  (unchanged).

### Invariants (MUST)

- `len(material[theme])` equals the WKB face count.
- `len(texture[theme])` equals the WKB face count; `len(texture[theme][i])`
  equals face `i`'s ring count; `len(texture[theme][i][r].uv)` equals ring
  `r`'s vertex count when `uv` is non-null.
- Every non-null id references an existing sidecar row.
- The same rules apply to `geometry_templates.parquet`, which uses the same
  per-LoD column grammar.

### What does not change

The sidecar tables (`materials.parquet`, `textures.parquet`), their `id`
semantics, the meaning of the `appearance := 'local' | 'sidecar'` reader option
in `duckdb-cityjson` (see *Reader modes* below for what each mode's columns
look like), `surfaces` (JSON, heterogeneous per surface) and `other` (JSON).

### Reader modes in `duckdb-cityjson`

Both modes emit the typed columns above; the column types never depend on the
`appearance` option. What the option decides is the **id space**:
`'sidecar'` (the CityParquet encoding) carries dataset-global sidecar ids,
`'local'` (the default, a CityJSON view) carries the feature-local indices the
source used. UV pairs are inlined in both modes — the reader holds the
per-feature (or header) `vertices-texture` pool, so a local-form UV index is
resolved to its pair on the way out, exactly as sidecar normalisation does
today. The COPY sink re-interns UV pools for CityJSON output either way, so
nothing is lost. The mesh writers consume one shape through `AppearanceSource`,
and the depth classifier in `mesh_model.cpp` is deleted.

### The `uv` element

`uv` is `LIST<LIST<DOUBLE>>` with the inner list constrained to exactly two
values, for the reason the sidecar tables give for `diffuseColor`: fixed-size
lists are unevenly supported across Parquet readers, so the cardinality is a
stated constraint rather than a type.

### Parquet / Arrow

Parquet `MAP` logical type, key `BYTE_ARRAY` (UTF8) required, value as above
with nullable list items; the `arrow.json` extension tag disappears from these
two columns. Arrow: `DataType::Map` with `LargeList`/`List` per the crate's
existing convention. GeoParquet legality and the `city` footer are unaffected.

## Consequences by repository

### `documents/` (phase 1)

- `03-specification/02-object-table-schema.mdx`: the two rows of the reserved
  column table.
- `03-specification/04-appearance-templates.mdx`: the "material / texture
  columns" section rewritten around the typed shape; the "why JSON" reasoning
  replaced by a "why MAP" note; the two normalisations (global ids, inline UVs)
  kept; invariants stated; template sidecar sentence.
- `03-specification/07-mapping-cityjson.mdx`: rows for `geometry[].material`,
  `geometry[].texture`, `appearance.vertices-texture`.
- `03-specification/08-worked-example.mdx`: the `material_lod2_2` cell. No
  writer produces the typed cell yet, so phase 1 writes it by hand from the
  invariants and phase 3 re-verifies it against the regenerated fixture.
- The gate is `just docs-build` (needs pnpm).
- `04-design-decisions/02-geometry-encoding.mdx` and
  `04-appearance-shared-resources.mdx`: the decision text (flat per WKB face,
  typed MAP; alternatives: JSON nested as CityJSON, JSON flat).
- `05-open-questions/index.mdx`: the "Appearance column shape" row.
- `07-tutorials/03-duckdb-cityjson.mdx`, `06-resources/02-software.mdx`: the
  column table and the conformance matrix.

### `lib/cityparquet-rs` (phase 2)

- `crates/schema/src/model.rs`: the Arrow `DataType` of the two columns.
- `crates/core/src/appearance.rs`: keep the interner; the map rewrite emits the
  flat typed shape by reusing the walk that produces `face_semantics` in
  `encode.rs` (`flatten_values` / `count_boundary_faces` /
  `values_nesting_depth`, then the writer-dropped positions removed). One
  traversal defines WKB face order for semantics and appearance alike, which
  is what makes the "entry `i` is WKB face `i`" invariant meaningful. A
  texture theme's per-face entry is its ring list, kept as one unit by the
  flattening and rewritten ring by ring afterwards.
- `encode.rs` / `decode.rs`: Arrow builders and readers for the MAP columns.
- `export.rs`: re-nest from `shells` (as semantics does) on the way to CityJSON;
  re-intern UV pools.
- `compare.rs`, `sidecar.rs` (templates), the CLI: follow.
- Fixtures: regenerate `lib/duckdb-cityjson/test/data/cityparquet_rs_minimal`
  with this writer (the DuckDB tests must read a package the extension did not
  produce).

### `lib/duckdb-cityjson` (phase 3)

- `column_types` (`AppearanceJson` → typed kinds in sidecar mode), the scan
  (`NormaliseMaterialMap` / `NormaliseTextureMap` produce flat typed values),
  the templates sidecar, `cityjson_appearance_ids` (MAP input), `insert_*`,
  `reconcile`, `type_remap`, `cityparquet_write` (column typing), the COPY sink
  (re-nest from `shells`), `AppearanceSource` / `BuildMeshModel` (typed cells,
  classifier deleted), tests, `FUNCTIONS.md` / `DESIGN_DOC.md` / `TRAPS.md`.

## Alternatives rejected

- **Amend the spec to the nested CityJSON shape** (what both implementations
  do today): one doc change, no code; rejected because a consumer must then
  know the geometry's nesting to index a face, breaking the positional model
  `face_semantics` established.
- **JSON, flat**: enforces the shape without new types; rejected because the
  values have a fixed shape and stay a JSON parse on the analytical path.
- **Type materials only**: rejected because the two columns would follow
  different rules for no benefit; the texture shape is fully expressible.

## Sequencing

Specification first, then `cityparquet-rs`, then fixture regeneration, then
`duckdb-cityjson`. Each phase is one implementation plan executed with review
gates; the DuckDB phase cannot start before the regenerated fixture exists.
