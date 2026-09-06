# Typed Appearance Columns — Specification (phase 1) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the normative CityParquet specification say that `material_lod*` / `texture_lod*` are typed `MAP` columns, flat per WKB face, and update every page that argues from or restates that shape.

**Architecture:** Documentation-only change in `documents/docs/`. The specification pages are the authority; the design-decision pages record why; the open-questions page records that the question is settled; the software matrix records that no implementation has caught up yet. The tutorial page for `duckdb-cityjson` describes that extension's *present* behaviour and is left for phase 3.

**Tech Stack:** Blume docs site (`documents/`, MDX), built with `pnpm` via the root `just docs-build`. Prettier runs on staged files through the repository hook (`just hooks`).

**Spec:** `ai/design-notes/specs/2026-09-04-typed-appearance-columns-design.md` — read it first; every type and invariant below is copied from it.

## Global Constraints

- **British English** in prose (`colour`, `normalise`, `serialisation`).
- **Document the present, never the past.** No "previously", "used to be", "was JSON" outside a clearly-marked design-decision or note block that explains a choice. The specification pages state what the format *is*.
- **Type notation is the specification's own**: `MAP<K, V>`, `LIST<T>`, `STRUCT<name TYPE, …>`, `INT`, `DOUBLE`, `VARCHAR`, `JSON` — exactly as the existing tables on `03-specification/index.mdx` and `02-object-table-schema.mdx` use them. Never DuckDB SQL spellings (`INTEGER[]`, `MAP(VARCHAR, …)`) on the specification pages.
- The two column types, verbatim, everywhere they appear:
  - `material_lod*`: `MAP<VARCHAR, LIST<BIGINT>>`
  - `texture_lod*`: `MAP<VARCHAR, LIST<LIST<STRUCT<id BIGINT, uv LIST<LIST<DOUBLE>>>>>>`
- A map key is the **theme** name; CityJSON's unnamed theme is the empty string `""`.
- An untextured ring is `{id: null, uv: null}`. There is no broadcast (`value`) form: a whole-geometry material is expanded to one entry per WKB face.
- Every task ends with `just docs-build` from the repository root passing (it needs `pnpm`; `just docs-install` runs inside it). A build warning about a broken internal link is a failure.
- Commit per task in the **parent** repository (`/data2/hideba/cityparquet`), on `develop`. Commit messages: conventional prefix `docs(spec): …`, body in British English, and the trailer line `Claude-Session: https://claude.ai/code/session_018spx3RYGD6ZRYC8xaznCLf` last. Never `--no-verify`.
- Do not touch `lib/duckdb-3d` (a submodule pointer with an unrelated pending change), `lib/duckdb-cityjson`, `lib/cityparquet-rs`, or `documents/docs/07-tutorials/03-duckdb-cityjson.mdx`.
- Markdown tables in these files are hand-aligned only where the file already aligns them; otherwise match the file's existing style. Prettier will reformat staged files in the hook — re-read the file after committing if you edit it again.

---

## File map

| File | Responsibility in this plan |
| --- | --- |
| `documents/docs/03-specification/index.mdx` | The logical-type → Parquet table gains a `MAP<K, V>` row (Task 1) |
| `documents/docs/03-specification/02-object-table-schema.mdx` | The two reserved-column rows (Task 1) |
| `documents/docs/03-specification/04-appearance-templates.mdx` | The normative "material / texture columns" section, the template sidecar rows, the "why MAP" note (Task 2) |
| `documents/docs/03-specification/07-mapping-cityjson.mdx` | Three CityJSON → CityParquet mapping rows (Task 3) |
| `documents/docs/03-specification/08-worked-example.mdx` | The `material_lod2_2` cell and its two explanatory notes (Task 3) |
| `documents/docs/04-design-decisions/02-geometry-encoding.mdx` | One sentence: appearance columns share the WKB face order and are typed (Task 4) |
| `documents/docs/04-design-decisions/04-appearance-shared-resources.mdx` | The decision text, alternatives and status for the column shape (Task 4) |
| `documents/docs/05-open-questions/index.mdx` | The "Appearance column shape" resolved row (Task 4) |
| `documents/docs/06-resources/02-software.mdx` | The `cityparquet-rs` conformance row for "Appearance & templates" (Task 4) |

---

### Task 1: Type notation and the object-table rows

**Files:**
- Modify: `documents/docs/03-specification/index.mdx` (the "Logical type | Physical Parquet encoding" table, around lines 48–61)
- Modify: `documents/docs/03-specification/02-object-table-schema.mdx:30-31`

**Interfaces:**
- Produces: the `MAP<K, V>` row that every later page's type strings rely on being defined.

- [ ] **Step 1: Add the `MAP<K, V>` row to the type table in `index.mdx`**

Insert this row directly after the `LIST<T>` row (keep the table's existing style):

```markdown
| `MAP<K, V>` | Parquet `MAP` logical type: a required key of type `K`, a nullable value of type `V`, entries in no defined order |
```

- [ ] **Step 2: Replace the two appearance rows in `02-object-table-schema.mdx`**

Replace lines 30–31 (the `material_lod*` and `texture_lod*` rows) with:

```markdown
| `material_lod*` | `MAP<VARCHAR, LIST<BIGINT>>` |  | nullable | Per-theme material references for the geometry in the matching `geometry_lod*` cell: theme → one `materials.parquet` `id` (or `null`) per WKB face (see [appearance & templates](/specification/appearance-templates)); one per LoD |
| `texture_lod*` | `MAP<VARCHAR, LIST<LIST<STRUCT<id BIGINT, uv LIST<LIST<DOUBLE>>>>>>` |  | nullable | Per-theme texture references for the geometry in the matching `geometry_lod*` cell: theme → per WKB face → per ring → the `textures.parquet` `id` and the ring's `[u, v]` pairs; one per LoD |
```

- [ ] **Step 3: Build**

Run from the repository root: `just docs-build`
Expected: exits 0, no broken-link warnings.

- [ ] **Step 4: Commit**

```bash
git add documents/docs/03-specification/index.mdx documents/docs/03-specification/02-object-table-schema.mdx
git commit -m "docs(spec): type the appearance columns as MAP in the schema tables

Adds the MAP<K, V> logical type and declares material_lod* /
texture_lod* with their typed, per-WKB-face shape.

Claude-Session: https://claude.ai/code/session_018spx3RYGD6ZRYC8xaznCLf"
```

---

### Task 2: The normative appearance section

**Files:**
- Modify: `documents/docs/03-specification/04-appearance-templates.mdx` — the intro paragraph (lines 8–11), the whole `## material / texture columns` section (lines 13–68), the two template rows (lines 139–140), and a new note after the existing `:::note[Why typed columns rather than JSON]` block (lines 111–117).

**Interfaces:**
- Consumes: the `MAP<K, V>` type from Task 1.
- Produces: the section every other page links to as `/specification/appearance-templates`; the invariant wording Task 3 and Task 4 quote.

- [ ] **Step 1: Rewrite the intro paragraph (lines 8–11)**

Replace:

```markdown
Appearance — material and texture — is separated from geometry and carried in per-LoD
JSON columns, with the shared definitions deduplicated into sidecar files. Reusable
geometry templates use the same geometry strategy as the object table. All three
sidecars are optional: a source with no appearance produces none of them.
```

with:

```markdown
Appearance — material and texture — is separated from geometry and carried in per-LoD
typed `MAP` columns aligned to the geometry's WKB faces, with the shared definitions
deduplicated into sidecar files. Reusable geometry templates use the same geometry
strategy as the object table. All three sidecars are optional: a source with no
appearance produces none of them.
```

- [ ] **Step 2: Replace the `## material / texture columns` section (lines 13–68) in full**

Everything from the `## material / texture columns` heading up to (not including) `## materials.parquet` becomes:

````markdown
## material / texture columns

Appearance is [separated from geometry](/design-decisions/appearance-shared-resources),
following the OBJ / COLLADA / glTF principle. The object table has stable nullable
`material_lod*` and `texture_lod*` columns; the shared definitions live in sidecar
files.

Appearance is per-surface data attached to a geometry, and every geometry has exactly
one LoD — so appearance is **inherently per-LoD**, and CityParquet gives it **one
column per LoD**, named by the [suffix grammar](/specification/geometry-semantics) and
paired to the geometry it decorates: `material_lod2_2` / `texture_lod2_2` describe the
surfaces of the same row's `geometry_lod2_2` cell, exactly as
`geometry_properties_lod2_2` carries that geometry's semantics. So a query for one
LoD's appearance reads only that column, and the LoD-to-appearance pairing is carried
by the column name rather than by a key a consumer must match.

| Column | Type |
|---|---|
| `material_lod*` | `MAP<VARCHAR, LIST<BIGINT>>` |
| `texture_lod*` | `MAP<VARCHAR, LIST<LIST<STRUCT<id BIGINT, uv LIST<LIST<DOUBLE>>>>>>` |

These columns carry no `lod` value — the column name already fixes the LoD. The outer
dimension is a **theme**: a dynamic string key whose set is not known in advance (a
geometry may carry any number of named appearance themes), which is why the column is
a `MAP` rather than a `STRUCT` — a fixed struct cannot express an open key set. What a
theme holds has a fixed shape, so the map's value is typed. The key is the theme name;
CityJSON's unnamed theme is the empty string `""`.

```text
-- material_lod2_2 — theme → one material id (or null) per WKB face
{ "<theme>": [ <matId|null>, … ] }

-- texture_lod2_2 — theme → per WKB face → per ring of that face → { id, uv }
{ "<theme>": [ [ { id: <texId>, uv: [ [u,v], [u,v], … ] }, … ], … ] }
```

A `material_lod<suffix>` / `texture_lod<suffix>` cell is non-null only where the same
row's `geometry_lod<suffix>` is non-null. A cell is `null` when the geometry carries no
material (or no texture) in any theme; a theme absent from the map carries nothing for
that geometry.

- **Material** is per **face**: a theme's list aligns to the geometry's WKB faces
  exactly as `face_semantics` does — one `matId` (or `null`) per face — so its length
  equals the WKB face count (see [geometry & semantics](/specification/geometry-semantics)).
  A source that assigns one material to the whole geometry is expanded to one entry per
  face; there is no whole-geometry shorthand in the column.
- **Texture** is per **ring**, because texture coordinates attach to ring vertices: a
  theme's list is one entry per WKB face, each of which is a list over that face's
  rings (exterior ring first, then interior rings, matching the `PolygonZ` ring order),
  and each ring is a struct of the texture `id` and its `uv` list — **one `[u, v]` pair
  per vertex of that ring**, in ring vertex order, each inner list holding exactly two
  values. A ring with no texture is `{ id: null, uv: null }`. Flattening a face to one
  `{ id, uv }` would lose the appearance of any interior ring, so the ring level is
  retained even when a face has only its exterior ring.

**Invariants.** For every theme in a non-null cell:

- `len(material[theme])` **MUST** equal the WKB face count.
- `len(texture[theme])` **MUST** equal the WKB face count; `len(texture[theme][i])`
  **MUST** equal face `i`'s ring count; when `uv` is non-null,
  `len(texture[theme][i][r].uv)` **MUST** equal ring `r`'s vertex count and every
  entry **MUST** hold exactly two values.
- Every non-null `id` **MUST** match an `id` in the corresponding sidecar.

The key property is the same one `face_semantics` has: **`material[theme][i]` is the
material of WKB face `i`**, with no geometry-type-specific nesting to walk. A `Solid`'s
per-shell partition is already recorded once, in `geometry_properties.shells`.

Two normalisations apply relative to a CityJSON source:

- **Material and texture references are sidecar `id` values** — dataset-global and
  stable — not the feature-local indices CityJSONSeq uses. A reference resolves by
  matching the sidecar's `id` column; it **MUST NOT** be interpreted as a row
  position, which is not a stable key and may change when a package is rewritten.
- **Texture UV indices are replaced inline** by the actual `[u, v]` coordinate pair
  in the ring's `uv` list, so the object table is self-contained and no stored UV
  pool is needed. On export, UV pairs are re-interned into a feature-local pool and
  indices re-derived.

:::note[Why a typed MAP rather than JSON]
A theme's content has a fixed, well-known shape — ids per face, and `{id, uv}` per
ring — so a typed column costs nothing and buys predicate pushdown, engine-side
validation of the invariants above, and better compression, for the same reasons the
sidecar tables give below. The one open dimension, the theme, is exactly what a `MAP`
key expresses. `uv` is `LIST<LIST<DOUBLE>>` rather than a fixed-size pair for the
reason `diffuseColor` gives: fixed-size lists are unevenly supported across Parquet
readers, so the cardinality is stated as a constraint instead.
:::

````

- [ ] **Step 3: Replace the two template rows (lines 139–140 before your edits; find them by content)**

Replace:

```markdown
| `material_lod*` | `JSON` |  | Template material mapping, per LoD |
| `texture_lod*` | `JSON` |  | Template texture mapping, per LoD |
```

with:

```markdown
| `material_lod*` | `MAP<VARCHAR, LIST<BIGINT>>` |  | Template material mapping, per LoD — the [same shape and invariants](#material--texture-columns) as an object row's |
| `texture_lod*` | `MAP<VARCHAR, LIST<LIST<STRUCT<id BIGINT, uv LIST<LIST<DOUBLE>>>>>>` |  | Template texture mapping, per LoD — the same shape and invariants as an object row's |
```

Check the anchor: Blume slugifies `## material / texture columns` — after building, open `documents/dist/specification/appearance-templates/index.html` (or the equivalent output path) and confirm the heading's `id`. If it differs from `material--texture-columns`, use the generated one.

- [ ] **Step 4: Check the existing sidecar note still reads correctly**

The block `:::note[Why typed columns rather than JSON]` (about `diffuseColor` etc.) stays as it is. Read it once to confirm nothing in it now contradicts the new section.

- [ ] **Step 5: Build**

Run from the repository root: `just docs-build`
Expected: exits 0, no broken-link warnings (the `#material--texture-columns` anchor in particular).

- [ ] **Step 6: Commit**

```bash
git add documents/docs/03-specification/04-appearance-templates.mdx
git commit -m "docs(spec): define the typed, WKB-face-aligned appearance columns

material_lod* and texture_lod* are MAP columns keyed by theme, flat per
WKB face like face_semantics, with the texture ring struct carrying the
sidecar id and inlined UV pairs. States the invariants and the reason for
MAP over JSON.

Claude-Session: https://claude.ai/code/session_018spx3RYGD6ZRYC8xaznCLf"
```

---

### Task 3: The CityJSON mapping and the worked example

**Files:**
- Modify: `documents/docs/03-specification/07-mapping-cityjson.mdx:54-55` and `:62`
- Modify: `documents/docs/03-specification/08-worked-example.mdx:33`, `:48-50`, `:90-91`

**Interfaces:**
- Consumes: the wording of Task 2.

- [ ] **Step 1: Update the three mapping rows in `07-mapping-cityjson.mdx`**

Replace the `geometry[].material` and `geometry[].texture` rows with:

```markdown
| `geometry[].material`                     | [`material_lod*`](/specification/appearance-templates) for the geometry's LoD — one map entry per theme, `values` flattened to one sidecar `id` per WKB face, a whole-geometry `value` expanded per face |
| `geometry[].texture`                      | `texture_lod*` for the geometry's LoD — one map entry per theme, `values` flattened per WKB face, each ring's texture index and UV indices resolved to `{id, uv}` |
```

Replace the `appearance.vertices-texture` row with:

```markdown
| `appearance.vertices-texture`             | resolved into the `uv` pairs of `texture_lod*`; no UV pool is stored |
```

Keep the column widths of the surrounding rows if the file pads them; otherwise leave them unpadded.

- [ ] **Step 2: Update the worked example's cell (line 33)**

In the `b-42-0` row, replace the `material_lod2_2` cell `` `{"":{"values":[3,3]}}` `` with `` `{"": [3, 3]}` ``.

- [ ] **Step 3: Update the "Material is per face" note (lines 48–50)**

Replace:

```markdown
- Material is **per face**: the two faces both reference material `3`, so
  `material_lod2_2` is `{"":{"values":[3,3]}}`. The `3` is the **`id`** of a row in
  [`materials.parquet`](/specification/appearance-templates), not a row position.
```

with:

```markdown
- Material is **per face**: the two faces both reference material `3` in the unnamed
  theme, so `material_lod2_2` is the map `{"": [3, 3]}` — one entry per WKB face,
  aligned like `face_semantics`. The `3` is the **`id`** of a row in
  [`materials.parquet`](/specification/appearance-templates), not a row position.
```

- [ ] **Step 4: Update the last "what the encoding changes" bullet (lines 90–91)**

Replace:

```markdown
- The per-shell `material.values` (`[[3, 3]]`) is flattened to the per-face
  `[3, 3]`, and the feature-local material index becomes the sidecar `id`.
```

with:

```markdown
- The per-shell `material.values` (`[[3, 3]]`) is flattened to the per-face
  `[3, 3]` under its theme key (`""`), and the feature-local material index becomes
  the sidecar `id`.
```

- [ ] **Step 5: Build**

Run from the repository root: `just docs-build`
Expected: exits 0.

- [ ] **Step 6: Commit**

```bash
git add documents/docs/03-specification/07-mapping-cityjson.mdx documents/docs/03-specification/08-worked-example.mdx
git commit -m "docs(spec): map CityJSON appearance onto the typed columns

The mapping table and the worked example show the per-theme map with
one sidecar id per WKB face.

Claude-Session: https://claude.ai/code/session_018spx3RYGD6ZRYC8xaznCLf"
```

---

### Task 4: Design decisions, the resolved question, and the conformance matrix

**Files:**
- Modify: `documents/docs/04-design-decisions/02-geometry-encoding.mdx:39`
- Modify: `documents/docs/04-design-decisions/04-appearance-shared-resources.mdx:14-25`
- Modify: `documents/docs/05-open-questions/index.mdx:80`
- Modify: `documents/docs/06-resources/02-software.mdx:120`

**Interfaces:**
- Consumes: the type strings and the invariant wording of Task 2.

- [ ] **Step 1: One sentence in the geometry-encoding decision (line 39)**

In the **Decision.** paragraph, replace the sentence

```markdown
Appearance goes in separate `material_lod*` / `texture_lod*` columns (see [appearance and shared resources](/design-decisions/appearance-shared-resources)).
```

with

```markdown
Appearance goes in separate `material_lod*` / `texture_lod*` columns — typed `MAP`s keyed by theme, aligned to the same WKB face order (see [appearance and shared resources](/design-decisions/appearance-shared-resources)).
```

- [ ] **Step 2: The appearance decision (lines 14–25 of `04-appearance-shared-resources.mdx`)**

Replace the **Decision.** paragraph with:

```markdown
**Decision.** `materials.parquet` and `textures.parquet` hold reusable definitions; the object table's nullable `material_lod*`/`texture_lod*` columns reference them by the sidecar's **dataset-global `id`** — matched against the `id` column, never a row position. The columns are typed `MAP`s keyed by theme whose values are **flat per WKB face**, in WKB face order — `MAP<VARCHAR, LIST<BIGINT>>` for materials, and per face → per ring `STRUCT<id BIGINT, uv LIST<LIST<DOUBLE>>>` for textures, with the UV coordinates **inlined**. See the [specification](/specification/appearance-templates) → *material / texture columns*.
```

Under **Alternatives considered.**, after the two existing bullets, add:

```markdown
- **JSON cells nested as CityJSON nests them** (per shell for a `Solid`, per solid and shell for a `MultiSolid`): a consumer must know the geometry type to find a face's material, which is the traversal the flattened `face_semantics` exists to avoid, and the shell partition is stored twice.
- **JSON cells, flat per face**: the right shape, but a JSON parse on every analytical query over values whose shape is fixed and well known.
```

In **Why this.**, append one sentence at the end of the paragraph:

```markdown
The column shape follows `face_semantics`: one positional model for everything attached to a WKB face, and a typed column because a theme's content has a fixed shape — only the theme key is open, which is what `MAP` expresses.
```

Replace the **Status.** line with:

```markdown
**Status.** Decided. The global-id + inline-UV normalisation and the typed per-face column shape are normative; the [conformance matrix](/resources/software) records which implementation has caught up.
```

- [ ] **Step 3: The resolved row in `05-open-questions/index.mdx` (line 80)**

Replace:

```markdown
| Appearance column shape | Per-LoD `material_lod*` / `texture_lod*` columns, parallel to `geometry_lod*`. |
```

with:

```markdown
| Appearance column shape | Per-LoD `material_lod*` / `texture_lod*` columns, parallel to `geometry_lod*`, typed as `MAP`s from theme to per-WKB-face values (flat, like `face_semantics`; settled in the 2026-09-04 review). |
```

- [ ] **Step 4: The `cityparquet-rs` conformance row in `06-resources/02-software.mdx` (line 120)**

Replace the Notes cell of the "Appearance & templates" row with:

```markdown
Per-LoD `material_lod*` / `texture_lod*` columns and the `textures.parquet` schema are done. Not yet aligned to the spec: the writer emits those two columns as JSON cells in CityJSON's nesting rather than the typed, per-WKB-face `MAP`s; the stricter sidecar column types, the per-LoD template sidecar, geometry-template instances, and unreferenced appearance definitions are also not yet done.
```

Then search the same file for the `duckdb-cityjson` section (`grep -n "material\|appearance" documents/docs/06-resources/02-software.mdx`). If a row or sentence there claims the extension's appearance columns conform, amend it in the same way (JSON cells today; typed `MAP`s not yet done). If it makes no such claim, leave it.

- [ ] **Step 5: Build**

Run from the repository root: `just docs-build`
Expected: exits 0.

- [ ] **Step 6: Commit**

```bash
git add documents/docs/04-design-decisions/02-geometry-encoding.mdx documents/docs/04-design-decisions/04-appearance-shared-resources.mdx documents/docs/05-open-questions/index.mdx documents/docs/06-resources/02-software.mdx
git commit -m "docs(spec): record the typed appearance column decision

The design-decision pages carry the alternatives and the reasoning, the
open-questions page marks the shape settled, and the conformance matrix
says the Rust writer has not caught up.

Claude-Session: https://claude.ai/code/session_018spx3RYGD6ZRYC8xaznCLf"
```

---

### Task 5: Whole-site consistency sweep

**Files:**
- Read: every `.mdx` under `documents/docs/`.

- [ ] **Step 1: Grep for stale wording**

From the repository root:

```bash
grep -rn "\"values\"\|{\"value\"\|\[ null \]\|JSON columns\|JSON column\|as JSON text" documents/docs --include=*.mdx
```

Expected hits that are **correct and stay**: `07-tutorials/03-duckdb-cityjson.mdx` (describes the extension's present behaviour; phase 3 updates it). Anything else naming the appearance columns as JSON is a miss — fix it in the same voice as the pages above.

- [ ] **Step 2: Grep for the column names**

```bash
grep -rn "material_lod\|texture_lod" documents/docs --include=*.mdx
```

Read each hit in context. `03-geometry-semantics.mdx:245` and `07-tutorials/04-cloud-scale-3dbag.mdx:52` only name the columns and need no change.

- [ ] **Step 3: Build, and commit any fix**

Run: `just docs-build`. If Step 1 or 2 changed anything:

```bash
git add documents/docs
git commit -m "docs(spec): align remaining appearance-column mentions

Claude-Session: https://claude.ai/code/session_018spx3RYGD6ZRYC8xaznCLf"
```

---

## Not in this plan

- `documents/docs/07-tutorials/03-duckdb-cityjson.mdx` — describes `duckdb-cityjson` as it behaves today; updated in phase 3 with the extension.
- Any change under `lib/` — phases 2 and 3 have their own plans.
- Pushing — the three phases land on `develop` and are pushed together at the end.
