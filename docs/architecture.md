# Code architecture

How `cityparquet-rs` is put together: the crates, the module responsibilities,
and the data flow through conversion, reading, export, comparison, and
benchmarking. Read [`design.md`](design.md) first for the *format*; this
document is about the *code*.

## Workspace crates

Three crates, layered so the type system has no I/O dependencies:

| Crate | Responsibility | Notable dependency line |
|---|---|---|
| **`cityparquet-schema`** | The CityParquet spec *as code*: types, CityGML taxonomy, Arrow schema, profiles, manifest, metadata. No buffers, no Parquet. | **zero** `arrow-array` / `parquet` deps — only `arrow-schema` |
| **`cityparquet`** | The Parquet read/write path: scan, encode, write, read, decode, export, compare, WKB, appearance, sidecars, ordering, recipes. | `arrow-*`, `parquet`, `wkb`, `cjseq` |
| **`cityparquet-cli`** | The `cityparquet` binary (convert/export/compare/bench) and the benchmark harness library. | `clap`, the two crates above |

The **schema/Parquet isolation** is enforced in CI: `just isolation` fails if
`cityparquet-schema` ever pulls in `arrow-array` or `parquet`. Keeping the
type system free of the columnar runtime is what lets it stand in as an
executable specification.

### `cityparquet-schema` modules

- `types` — `Lod` (major/optional-minor, column-suffix mapping), the
  CityJSON↔CityGML 3.0 `TAXONOMY`, `CityGmlModule`, extension-type detection.
- `model` — `CityParquetSchema` → `to_arrow_schema()`: renders the reserved +
  per-LoD geometry + inferred attribute columns, each with `cityparquet:role`
  / `cityparquet:lod` / geoarrow / arrow.json field metadata, in spec order.
  Validates duplicate LoDs and attribute/reserved name collisions.
- `attributes` — `AttributeInferer` / `AttributeType`: the primitive-mapping
  table that turns sampled source values into a column type.
- `profile` — `Profile` (Core/Compatibility), the three sidecar table schemas
  (`materials_schema`/`textures_schema`/`geometry_templates_schema`), and the
  `PackageManifest` (`metadata.json`) struct.
- `metadata` — `CityParquetMetadata`: the dataset KV-metadata block as a typed
  struct, plus `CITYPARQUET_VERSION` and `SourceFormat`.

## Conversion pipeline

`package::convert(&ConvertOptions) -> ConvertReport` is the whole
CityJSON→package path. It is a **two-pass** design over a unified `Source`:

```
Source (CityJSON doc or CityJSONSeq stream)
   │
   ├─ pass 1 ── scan ─────────► ScanResult { CityParquetSchema, dataset metadata }
   │            (LoDs, attribute columns, CRS, transform, dataset bbox;
   │             retains no geometry buffers — just "what columns/metadata?")
   │
   └─ pass 2 ── encode ───────► stream of RecordBatch conforming to that schema
                │                (one row per CityObject; geometry → WKB via
                │                 wkb_write; semantics/appearance → JSON columns)
                │
                ├─ recipe ─────► per-column WriterProperties (the benchmark variable)
                │
                └─ write ──────► one <snake>.parquet per family + sidecars + metadata.json
```

- **`source`** — `Source` unifies whole-document CityJSON and streaming
  CityJSONSeq behind one feature iterator, so nothing downstream cares which
  input shape it got.
- **`scan`** (pass 1) — one read-only pass answering "what columns and
  dataset metadata does this need?" It never retains WKB or vertex data, so
  pass 2 can size its Arrow builders up front.
- **`encode`** (pass 2) — trusts the `ScanResult` and emits `RecordBatch`es;
  fails fast if asked to encode something the schema doesn't describe. One row
  per object, geometry bucketed into the per-LoD columns.
- **`recipe`** — renders a `WriterRecipe`/`RecipePreset` into concrete
  `WriterProperties`. Column decisions are driven by field metadata
  (geoarrow.wkb, arrow.json) and the six fixed `bbox` leaf paths — **never by
  column name** — so the recipe can't drift from the schema. This is the
  paper's benchmark variable (see below).
- **`order`** — optional Hilbert reordering of features before encode.
- **`sidecar`** / **`appearance`** — under Compatibility, the appearance
  interner assigns dataset-global ids and the sidecar writers emit the
  definition tables.
- **`wkb_write`** — the minimal little-endian ISO-WKB writer; container types
  wrap complete nested geometries. Ring normalisation here is deliberately
  **index-based** (see the comparator note below).

**Crash safety.** Everything is written into a `.cityparquet-tmp/`
subdirectory first; only after a fully successful write does `commit_package`
purge any prior package files and rename the temp contents into place. A
failure mid-write leaves the existing package intact and the temp dir behind
for inspection.

`ConvertOptions` bundles the knobs: `profile`, `overwrite`, `batch_size`,
`recipe` (`WriterRecipe`), and `ordering` (`RowOrder::{Source, Hilbert}`).
The table layout itself is not a knob: by-type (one `<snake>.parquet` table
per 1st-level CityObject family) is the sole, mandatory layout.

## Reader

`reader` adds three CityParquet methods to Parquet's own
`ArrowReaderBuilder<T>` via a **blanket-impl'd extension trait** — no wrapper
builder. Every existing builder method (`with_batch_size`, `with_projection`,
`with_row_selection`, …) keeps working untouched (this is the explicit
"geoparquet lesson": don't re-wrap the upstream builder). The one wrapper,
`CityParquetRecordBatchReader`, re-applies the rendered schema — field
metadata included — to every emitted batch, because a bare `parquet` read
doesn't otherwise guarantee that metadata survives a projection, or that a
file written by a non-arrow-rs writer carries an embedded Arrow schema at all.
`with_bbox_row_groups` is the bbox row-group pruning entry point the Hilbert
ordering is designed to feed.

- **`scan`** (the reader's) and **`decode`** are the read-side inverse of
  encode: `decode` turns a `RecordBatch` row back into a `cjseq`-model
  `CityObject`. Geometry is deliberately kept *out* of the reassembled object
  (that struct's `geometry` field expects CityJSON boundary arrays, not WKB) —
  callers that need CityJSON-shaped geometry own that re-encoding.
- **`wkb_read`** — the inverse of `wkb_write`. Coordinates are deduplicated
  (bitwise `f64::to_bits`) into one shared pool per decoded geometry, members
  holding pool indices. This interning is what makes coordinate-degenerate
  rings visible on round-trip (see below).

## Export

`export::export(&ExportOptions) -> ExportReport` is the package→CityJSON
inverse of `convert`, built on `decode` plus its own geometry reconstruction
(WKB → CityJSON boundary arrays, re-quantised against the dataset transform).
Its correctness hinges on **manifest authority**:

- **Templates** are rebuilt from `geometry_templates.parquet` only when the
  manifest lists it; template vertices are re-interned as raw floats. If the
  manifest doesn't list it (Core, or Compatibility with no templates), an
  object's `template` reference can't resolve, so the object is exported
  without its instance geometry and the drop is counted. A reference to a
  missing row *in a listed sidecar* is a corrupt-file `Schema` error, not a
  silent drop; a listed-but-unreadable sidecar is an `Io` error.
- **Materials/textures** are handled the same way: listed → load the global
  definitions, slice out the per-feature subset, reassign feature-local
  indices, re-intern inlined UVs into a `vertices-texture` pool, attach as the
  feature's `appearance`. Not listed → the referenced definitions aren't in
  the package at all, so the maps are dropped (counted) rather than left
  dangling.

## Comparator

`compare` proves round-trip losslessness at the **semantic** level. Both
sides open as plain `Source`s (the exported file is itself valid CityJSON), and
every `CityObject` is flattened into one `id → ObjectData` map per side, so
feature grouping/ordering differences are irrelevant.

The subtle part is **degenerate-ring normalisation**, re-implemented here
*independently* of the writer on purpose — a comparator that reused the
writer's normalisation couldn't catch a bug in it, because both sides would
share the blind spot. Two layers:

1. **Index-based** (mirrors the writer): strip a ring's trailing duplicates
   of its first vertex index, drop rings left with < 3 entries, drop a surface
   whose exterior ring was dropped. Counts ring *elements*, not distinct
   coordinates — a genuinely-distinct zero-area ring passes through (data
   quality is not the format's job).
2. **Coordinate-based** (the round-trip-only case): also drop a ring whose
   surviving indices dequantise to < 3 *distinct* coordinates. This catches
   the real 3DBAG occurrence where three distinct vertex indices all map to
   one quantised coordinate: the writer emits it (index-distinct), the WKB
   reader's `f64::to_bits` interner collapses it to one repeated index on read,
   and without this rule the two sides would spuriously differ. Applied
   identically to both sides, reusing the same strip/realign machinery.

`material`/`texture` blocks are compared by default (same JSON equality as
`semantics`, after the same surface realignment); `Exclusions::appearance`
turns that off for the Core profile's deliberate drops.

## Benchmark harness

`cityparquet-cli::bench` (`cityparquet bench`) drives the paper's variant
matrix. For each variant — a `RecipePreset` × optional `+hilbert` × optional
`+by-type` × optional `+rg<N>` row-group-size override — it converts `input`
into a fresh tempdir, times the write, measures package size, times a full
scan (deriving the dataset bbox while it's at it), times a bbox-pruned window
query anchored at the bbox lower-left, counts row groups touched vs total,
and (unless `--skip-roundtrip`) exports and compares for exact equality. One
CSV row per variant. Every variant converts with **Compatibility**
unconditionally, so the harness stays uniform across variants and datasets
(sidecars cost nothing when there's no appearance data).

The committed artefacts and their caveats live in
[`../bench/README.md`](../bench/README.md).

### Recipe presets (the benchmark variable)

`RecipePreset` is the tuned default plus five ablations, so the paper can
quantify what each tuning rule buys:

| Preset (`--recipe`) | What it is |
|---|---|
| `cityparquet` | the tuned default: delta-encoded ids, dictionary `object_type`, BYTE_STREAM_SPLIT bbox leaves, no stats/dictionary on WKB+JSON, zstd 3 |
| `parquet-defaults` | parquet-rs defaults + the recipe's global compression & row-group size only — the "untuned writer" comparator |
| `no-dictionary` | `cityparquet` minus dictionary encoding everywhere |
| `no-bss` | `cityparquet` minus BYTE_STREAM_SPLIT on the bbox leaves |
| `no-delta` | `cityparquet` minus DELTA_BYTE_ARRAY on `id`/`feature_id` |
| `snappy` | `cityparquet` with Snappy instead of zstd (DuckDB COPY's default codec) |

KV metadata is embedded under every preset — it is never a benchmark
variable. Row-group size and zstd level remain independent CLI knobs on top.

## Testing discipline

Tests read **real CityJSON fixtures** (`delft.city.jsonl`,
`lod3_railway.city.json`), never inline hand-written CityJSON; edge cases are
derived from real fixtures/tiles in tempdirs. Development is strict red-green
TDD. `just check` runs clippy (`-D warnings`), the full test suite, the
schema/Parquet isolation check, and `cargo fmt --check`; `just interop`
additionally has DuckDB read the written Parquet natively to confirm the files
are plain, portable Parquet.
