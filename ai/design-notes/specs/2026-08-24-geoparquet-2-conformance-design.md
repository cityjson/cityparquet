# Drop `CityParquetArrowNative-v1`; adopt GeoParquet 2.0 conformance

**Date:** 2026-08-24
**Status:** implemented, then archived unmaintained like everything else in this
directory — where this note and the code disagree, the code is right. Two things
landed differently from what is written below, and both invert something the note
treats as settled:

- **Annotation is conditional, not universal.** The rule below — annotate every
  geometry column at the Parquet level, declare only the legal ones in `geo` — did
  not survive contact with DuckDB, which promotes any annotated column and eagerly
  decodes it. Its geometry model has no `PolyhedralSurface`, so an annotated solid
  column fails even `SELECT count(*)`, before any function sees a value. A column is
  annotated exactly when it is declared in `geo`.
- **The logical type's `crs` is inline PROJJSON**, not the short `<authority>:<code>`
  the note assumes. An authority code is resolved to PROJJSON by CRS-aware tooling
  and left verbatim otherwise, so a package's bytes depended on which extensions the
  writing session had loaded.

Because solid columns are therefore never annotated, spike 1's `geospatial_types`
caveat no longer describes any CityParquet file — the columns it concerns emit no
statistics at all. It was filed upstream on its own merits as
[apache/arrow-rs#10822](https://github.com/apache/arrow-rs/issues/10822).
**Scope:** `documents/` (normative spec), `cityparquet-rs`, `duckdb-cityjson`, `duckdb-3d`, `test/TESTING.md`
**Breaking:** yes. The `CityParquetArrowNative-v1` encoding is removed outright — no
deprecation shim, no reader fallback. Packages written with it become unreadable.

## Context

Two changes land together because they are the same decision seen from both ends.

GeoParquet **2.0.0** is released. It defers geometry typing to Parquet itself: as of
Parquet 2.11 (March 2025) the format specifies geospatial logical types and statistics,
and GeoParquet 2.0 requires that "geometry columns MUST be encoded as either `GEOMETRY`
or `GEOGRAPHY` logical types in Parquet, which both annotate a `BYTE_ARRAY` that encodes
geospatial features in the WKB format". Two consequences follow directly:

- The `encoding` field now permits **`"WKB"` only**. The GeoArrow native encodings that
  1.1.0 allowed (`point`, `linestring`, `polygon`, …) are gone.
- `covering` is gone. The column-level `bbox` in the `geo` metadata remains.

CityParquet has, on the other side, been carrying its own non-WKB encoding —
`CityParquetArrowNative-v1`, an indexed per-row vertex pool declared through
`city.columns[].encoding`, merged from the `arrow-native-type` branch and documented as
experimental. Its stated success criterion was a DuckDB query/scan benchmark that never
concluded. With the ecosystem settling on native Parquet geospatial types, a bespoke
nested-Arrow encoding is the wrong bet: it costs three implementations and buys
divergence from the direction every other reader is moving in.

So: remove ours, and adopt theirs.

### What this is *not*

`CityParquetArrowNative-v1` is **not** GeoArrow. The design decision record is explicit
that it uses an indexed vertex pool "rather than GeoArrow's coordinate-inline storage".
Two separate things in the tree match a naïve `geoarrow` grep, and only one is a target:

| Thing | Verdict |
| --- | --- |
| `CityParquetArrowNative-v1` — the encoding, its reader/writer, its token, its spec sections | **Remove** |
| `geoarrow.wkb` tagging — the `--geoarrow` flag, `geoarrow-schema` dep, `to_arrow_schema_tagged` | **Keep** (see B2) |
| GeoArrow as external prior art — `notes/format-evolution-timeline.md`, the GeoParquet 1.1 history, option (d) in the alternatives-considered list | **Keep** |

Stripping the third would gut the paper's prior-art narrative. Stripping the second
would delete a working Arrow-level interop feature that nothing about GeoParquet 2.0
obsoletes.

## Conformance model

CityParquet targets GeoParquet 2.0. Writers emit files that satisfy **both** 2.0 and
1.1.0 readers; the reader knows only the spec, never the writer.

**Writers emit, for 1.1.0 compatibility:**

- the `geo` key in the Parquet file key-value metadata;
- the column-level `bbox` field within it.

**Writers emit, for 2.0 conformance:**

- the native `GEOMETRY` logical type annotating each geometry column;
- `GeospatialStatistics` per column chunk, for pruning.

`geo.version` is a single string and cannot claim both. DuckDB's own implementation
settles this by precedent: `GeoParquetVersion::V1` and `::BOTH` write `geo` version
`"1.0.0"`, `::V2` writes `"2.0.0"`, and `::NONE` writes no `geo` at all — that much is
confirmed in `parquet_geometry.cpp`. The existence of a `BOTH` mode distinct from both
`V1` and `V2` is the precedent being cited. Dual conformance in practice therefore means
1.1-style `geo` metadata alongside native annotation, and that is what CityParquet
writes.

Separately, and usefully for B3: the logical type is emitted by `column_writer.cpp`
keyed on `LogicalTypeId::GEOMETRY` — that is, driven by the *column's DuckDB type*,
while `geoparquet_version` governs only the `geo` metadata. The two look independent,
which is what makes B3's target combination plausible. Spike 3 confirms it end to end.

`city.columns[].encoding` **stays `"WKB"`** and its vocabulary shrinks to that single
token. GeoParquet 2.0's own `encoding` field is still literally `"WKB"` — the logical
type is an orthogonal annotation, not a new encoding — so no token enters or leaves
beyond the one being removed. The key itself is retained as the forward-compatibility
hook it was always meant to be.

### GEOMETRY, never GEOGRAPHY

Every geometry column is annotated `GEOMETRY`. The input CRS is carried in the type's
`crs` parameter.

The Parquet discriminator between the two is **edge interpolation**, not CRS:
`GEOGRAPHY` declares that edges are geodesics on an ellipsoid, and requires naming the
algorithm. CityJSON geometry is polyhedral — vertices joined by straight lines in CRS
space — so `GEOGRAPHY` would misdeclare edge semantics even for lon/lat input, and its
bounding-box statistics would acquire antimeridian-wrapping semantics CityParquet does
not want. A CityParquet file in EPSG:4979 is planar-edged data that happens to be
measured in degrees.

### Solid geometry: annotate at the Parquet level, withhold at the GeoParquet level

The rule that makes this work:

> **Parquet-level annotation is universal; GeoParquet-level declaration is conditional.**
> Every geometry column gets the `GEOMETRY` logical type and `GeospatialStatistics`.
> Only GeoParquet-legal columns are declared in `geo`.

GeoParquet's permitted `geometry_types` vocabulary is the seven basic types plus a
dimension suffix — in neither 1.1.0 nor 2.0 does it name `PolyhedralSurface`, which is
precisely what CityParquet's solid geometry is. `GeoParquetLegal()` in
`cityparquet_write.cpp` already handles this by omitting such columns from `geo`
entirely, because a strict reader that eagerly decodes every declared column rejects the
whole file on one it cannot parse. That behaviour is retained unchanged.

Its legal set is deliberately **six**, not the seven GeoParquet permits: `GeometryCollection Z`
is excluded because CityParquet only ever produces one as a `MultiSolid`, whose members
are PolyhedralSurfaces. Do not "correct" this to seven.

What changes is that those columns are no longer excluded from *Parquet-level* typing.
This is sound because **CityParquet emits standard ISO WKB throughout**: `wkb_read.rs`
encodes CityJSON `Solid` as `PolyhedralSurface Z` (type code 1015) and `MultiSolid` as
`GeometryCollection Z` of PolyhedralSurfaces. There is no bespoke "Solid" WKB code
anywhere in the stack. The bytes in a solid column are genuine WKB, so annotating the
column `GEOMETRY` misdeclares nothing to a reader that trusts the annotation.

The result is a real gain: CityParquet's most distinctive columns — the solid ones —
acquire native typing and statistics-based pruning they have never had, while remaining
correctly undeclared in `geo`.

### Geometry templates are not annotated

`geometry_templates.parquet` is the one geometry column that stays a plain `BYTE_ARRAY`.
`sidecar_schemas.rs` already withholds `geoarrow.wkb`/CRS tagging from it for a reason
that applies with equal force to the Parquet logical type: **template coordinates are in
a local coordinate system, not the file CRS.** Annotating the column `GEOMETRY` would
force a `crs` parameter, and an omitted `crs` defaults to `OGC:CRS84` — so the
annotation could only ever assert something false about local coordinates. Templates are
therefore excluded from B2 and B4, and the object table's `geometry_lod*` columns
(tagged through `model.rs::geometry_field`) are the sole target.

### Reader dispatch

The reader never asks who wrote the file. It dispatches on what it finds, logical type
first:

1. `GEOMETRY`/`GEOGRAPHY` annotation present → the 2.0 path.
2. Otherwise plain `BYTE_ARRAY` + a `geo` key → the 1.1.0 fallback. Formally a spec
   violation under 2.0; supported as a fail-safe, and as the path that keeps every
   file written before this change readable.
3. Otherwise plain `BYTE_ARRAY` + `city` only → the existing solid-column path.

Reader-side *pruning* that consults `GeospatialStatistics` is a follow-up feature, not
part of this work. Writers emit the statistics; readers gain the annotation dispatch.

## Workstream A — remove `CityParquetArrowNative-v1`

Sequenced first. It is mechanical, and it shrinks the surface B touches.

**A1 `cityparquet-rs`.** `GeometryEncoding::ArrowNative`, `ARROW_NATIVE_V1_TOKEN`, and
the two-token `KNOWN_FOOTER_TOKENS` in `types.rs`. `arrow_geom_write.rs` and
`arrow_geom_read.rs` delete wholesale. `arrow_native_geometry_data_type()` and
`arrow_native_vertices_data_type()` in `model.rs`, along with the
`geometry_vertices_lod*` sibling column. The arrow-native arms in `encode`, `decode`,
`scan`, `recipe`, `reader`, `package`, `geometry_encoding.rs`, and the
`CityParquetArrowNative-v1` entry in `city.schema.json`. The CLI `--geometry-encoding`
flag drops entirely — pre-1.0, no deprecation. Delete
`tests/footer_encoding_dispatch.rs`; prune the arrow-native arms from exactly six
files: `lod0_synthesis.rs`, `convert_real_data.rs`, `encode_real_data.rs`,
`metadata_schema_real_data.rs`, `partition_real_data.rs`, `scan_real_data.rs`.

Note that `reader_real_data.rs` and `foreign_writer_schema.rs` match a `geoarrow` grep
but **not** an arrow-native one — their hits are `geoarrow.wkb` tagging assertions,
which stay. This is the goes/stays split reappearing at file level; an executor
grepping for `geoarrow` will break passing tests.
*Gate:* `cargo test` green.

**A2 `duckdb-cityjson`.** `arrow_native_encoder.{cpp,hpp}`, and the arrow-native paths
in `vector_writer`, `bind_function`, `copy_function`, `scan_function`,
`city_object_utils`, `cityparquet_write`, `geoparquet_table_function`, `types.hpp`,
`column_types.cpp`. Delete `test/sql/arrow_native_geometry.test` and
`test/cpp/test_arrow_native_encoder.cpp`; prune the arrow-native cases from
`cityparquet_footer.test`, `cityjson_geoparquet.test`, `cityjson_notebook_e2e.test`,
`cityjson_bind_data_copy.test`.
*Gate:* extension test suite green.

**A3 `duckdb-3d`.** `src/functions/arrow_native.cpp`, `src/kernel/arrow_native_import.{cpp,hpp}`,
their registration in `three_d_extension.cpp` and `three_d_functions.hpp`, and the
mentions in `solid_io.cpp` / `struct_metadata.cpp`. Delete
`test/sql/st_3d_from_arrow_native.test`, `test/sql/st_geom3d_from_arrow_native.test`,
`test/cpp/test_arrow_native_import.cpp`; prune `st_3d_metadata.test`. The vendored
`duckdb/` tree is untouched.
*Gate:* suite green.

**A4 docs and spec.** Delete the `### Arrow-native geometry encoding (experimental,
arrow-native-type branch)` section from `03-specification/03-geometry-semantics.mdx`.
Delete the whole **"Update (2026-07-26)"** paragraph from
`04-design-decisions/02-geometry-encoding.mdx` — but keep option (d) in the
alternatives-considered list, which is prior art. Reduce the `encoding` row in
`05-metadata.mdx` to `"WKB"` as the sole token. Update `06-resources/02-software.mdx`,
`test/TESTING.md`, the three per-library `docs/` sets, `lib/cityparquet-rs/docs/{design,architecture}.md`,
and the `CLAUDE.md`/`AGENTS.md` pairs.

**A5 history and rationale.** No deprecation notes, no "we removed X" tombstones, no
experimental sections left standing — consistent with the monorepo's standing rule to
*document the present, never the past*. Not a git-history rewrite: the monorepo is
public and the `arrow-native-type` merge (`cd8bf69`) is pushed, with later work built on
top — so manual removal, never `git revert -m`. The `ai/design-notes/` arrow-native spec
and plan files are deleted, along with the arrow-native passages in
`2026-08-16-cross-stack-test-pass.md` and
`2026-08-21-other-column-bbox-simplification-design.md`.

What survives is **not** a record that CityParquet once had the encoding, but a
statement of why it does not have one now. `02-geometry-encoding.mdx` keeps option (d)
in its alternatives-considered list and gains a rationale in the present tense, along
these lines:

> A packed or indexed Arrow-native geometry encoding is not used. The main goal behind
> a native Arrow geometry type is native statistics and not needing a separate `bbox`
> column — and the Parquet `GEOMETRY` logical type with `GeospatialStatistics` delivers
> exactly that, for WKB bytes every existing reader already understands. That leaves a
> native encoding offering only a modest size advantage over WKB, which does not justify
> a second encoding to specify, write, read and test. GeoParquet 2.0 reaches the same
> conclusion, permitting `"WKB"` alone.

Written so a reader who never knew the encoding existed learns only the design
position, not its history.

*Sweep:* unfiltered `grep -rniE 'CityParquetArrowNative|arrow.native|ArrowNative'`,
excluding `lib/duckdb-*/duckdb/`, returns nothing.

## Workstream B — GeoParquet 2.0 conformance

**B1 spec (`documents/`).** State the conformance model above in `05-metadata.mdx`:
geometry columns carry the `GEOMETRY` logical type; `geo` follows GeoParquet 2.0 for
legal columns; `bbox` retained; `covering` is not defined (2.0 dropped it, and the
existing note in `05-metadata.mdx` anticipating this becomes the settled rule rather
than a forward reference). Record the GEOMETRY-always decision and the solid-column
rule in `04-design-decisions/02-geometry-encoding.mdx`, replacing the deleted
arrow-native update. Revisit the `covering` entry in `05-open-questions/index.mdx`.

**B2 `cityparquet-rs` writer.** `parquet` 58.3 already ships everything needed behind a
`geospatial` cargo feature that is currently not enabled: `arrow/schema/extension.rs`
maps `parquet_geospatial::WkbType` ↔ `LogicalType::Geometry`/`Geography`, and the column
writer already calls `flush_geospatial_statistics()`. So: enable the feature, and in
`model.rs::geometry_field` tag the field with the Parquet WKB extension type carrying
the column CRS, in place of — or alongside — `geoarrow_schema::WkbType`. Which of those
two it is depends on spike 2. `geo` and `bbox` continue to be written by
`metadata.rs` unchanged.

**B3 `duckdb-cityjson` writer.** The largest piece. Today `cityparquet_write.cpp`
deliberately downgrades DuckDB `GEOMETRY` columns to WKB `BLOB` via `ST_AsWKB` and
stamps its own `geo` through `KV_METADATA`, specifically so DuckDB's GeoParquet hook
cannot write a second, conflicting `geo` key. Native typing requires keeping the column
`GEOMETRY`-typed through the `COPY` while still suppressing DuckDB's own `geo`. The
target combination is: native logical type + `GeospatialStatistics` + exactly one `geo`
key, CityParquet's. Spike 3 determines whether `geoparquet_version='none'` gives that
directly or whether a different mechanism is needed.

**B4 readers.** Implement the three-way dispatch above in `cityparquet-rs`
(`reader.rs`, `scan.rs`, `decode.rs`) and in `duckdb-cityjson`
(`geoparquet_table_function.cpp`, `bind_function.cpp`, `scan_function.cpp`). Fixtures
for all three branches, including a pre-change file to prove the 1.1.0 fallback.

**B5 `duckdb-3d` and walkthrough.** Read paths that assume an unannotated `BLOB`
geometry column; `test/TESTING.md` steps that assert footer shape.

## Where each module is worked

`lib/duckdb-cityjson` and `lib/duckdb-3d` are independent git repositories. The
monorepo's `CLAUDE.md` is explicit that they are not edited from the monorepo — work
happens in their own repos, and the monorepo only records the pinned commit. So this
design spans four execution contexts, in order:

| Modules | Worked in |
| --- | --- |
| A1, A4 (spec/docs), B1, B2, B4 (rs half) | the monorepo — `cityparquet-rs`, `documents/`, `test/`, `ai/design-notes/` |
| A2, B3, B4 (DuckDB half) | the `duckdb-cityjson` repo |
| A3, B5 | the `duckdb-3d` repo |
| paper references + submodule bumps | `cityparquet-paper` |

`cityparquet-rs` carries its own gate — `just check` (clippy `-D warnings`, tests,
schema isolation, `fmt --check`) — and it is what "green" means for A1 and B2.

## Spikes — resolved 2026-08-24

Run against `parquet` 58.3 with the `geospatial` feature (which pulls
`parquet-geospatial` 58.4), writing hand-built WKB through `ArrowWriter` and
reading the footer back.

**1. The WKB statistics accumulator does not reject type code 1015.** The write
succeeds, the `GEOMETRY` logical type is emitted with its CRS, and
`GeospatialStatistics` are produced — with a **numerically correct** bounding
box, Z included (`x[0,1] y[0,1] z[0,2]` for the test solid). So the
solid-column rule in the conformance model holds as written: annotate
universally, and pruning works on the columns that matter most.

**The one defect: `geospatial_types` misreports the type.** A
`PolyhedralSurface Z` column's statistics declare `[1007]`
(`GeometryCollection Z`), not `[1015]`. The cause is upstream and structural:
`geo-traits` has no `PolyhedralSurface` variant, so the WKB reader models one
as a collection of polygons and `geometry_type()` computes `1000 + 7`.

This is a false statement in the file, but a contained one. `geospatial_types`
is advisory; the bounding box — which is what row-group pruning actually reads
— is exact. Suppressing it is not a cheap option either: geospatial statistics
follow the logical-type annotation, so declining them means declining the
annotation, and losing the correct bbox with it. **Adopt as-is, record the
caveat in `05-metadata.mdx`, and file the type-code fidelity loss upstream.**

**2. The two WKB extension types are the same tag.** Both
`parquet_geospatial::WkbType` and `geoarrow_schema::WkbType` declare
`NAME = "geoarrow.wkb"`. The `--geoarrow` flag and the GeoParquet 2.0 path
therefore unify: one tag satisfies both, and `geoarrow-schema` becomes a
candidate for removal once `model.rs` switches to the Parquet type (which also
carries the `crs`/`edges` metadata the logical type needs).

**3. Still open — DuckDB.** Which `GeoParquetVersion` values emit the native
logical type, and whether `'none'` still emits it on `GEOMETRY`-typed columns
in vendored DuckDB v1.5.4. The code reading suggests the logical type follows
the column type rather than the version setting, but that is inferred from one
call site. **Gates B3.**

## Out of scope

- Reader-side pruning that consults `GeospatialStatistics`.
- `GEOGRAPHY` support in any form.
- Re-opening the WKB-versus-alternatives decision. WKB remains the encoding; this work
  changes how the column is *annotated*, not what the bytes are.

## Paper repository

`cityparquet-paper` needs one-line updates to
`references/2026-07-30-cityparquet-ecosystem-products.md:211` and
`references/2026-07-23-cityparquet-ecosystem-roadmap.md:160`, then a submodule bump.
`paper/` has no mentions of either encoding and needs no change. `notes/` is the
author's own material and is not touched.
