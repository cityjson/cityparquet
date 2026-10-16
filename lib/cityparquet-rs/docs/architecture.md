# Code architecture

How `cityparquet-rs` is put together: the crates, the module responsibilities,
and the data flow through conversion, reading, export, comparison, and
benchmarking. Read [`design.md`](design.md) first for the _format_; this
document is about the _code_.

## Workspace crates

Three crates, layered so the type system has no I/O dependencies:

| Crate                    | Responsibility                                                                                                                           | Notable dependency line                                       |
| ------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------- |
| **`cityparquet-schema`** | The CityParquet spec _as code_: types, CityGML taxonomy, Arrow schema, profiles, manifest, metadata. No buffers, no Parquet.             | **zero** `arrow-array` / `parquet` deps — only `arrow-schema` |
| **`cityparquet`**        | The Parquet read/write path: scan, encode, write, read, decode, export, compare, validate, WKB, appearance, sidecars, ordering, recipes. | `arrow-*`, `parquet`, `wkb`, `cjseq`, `citygml`               |
| **`cityparquet-cli`**    | The `cityparquet` binary (convert/export/compare/collection/validate/bench) and the benchmark harness library.                           | `clap`, the two crates above                                  |

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
- `reserved` — the reserved column names (case-insensitive, per-LoD forms by
  the suffix grammar) and the leading-`_` escape that stores a colliding
  source attribute and restores it on read.
- `attributes` — `AttributeInferer` / `AttributeType`: the primitive-mapping
  table that turns sampled source values into a column type.
- `profile` — `Profile` (Core/Compatibility), the three sidecar table schemas
  (`materials_schema`/`textures_schema`/`implicit_geometries_schema`), and the
  `PackageManifest` (`metadata.json`) struct.
- `metadata` — `CityParquetMetadata`: the dataset KV-metadata block as a typed
  struct, plus `CITYPARQUET_VERSION` and `SourceFormat`; `city.extensions` is
  typed as `ExtensionDeclarations` (namespace → `name`/`url`/`version`/`xmlns`).
- `extensions` — the extension-namespace rules as pure functions: namespace
  derivation from a CityJSON Extension's name, reading the source's
  `extensions` member into `city.extensions` and rebuilding it on export, and
  `ExtensionNaming`, which attributes a `+` name to the single declared
  extension (`+heatCapacity` → `energy_heatCapacity`), rejects what it cannot
  attribute and core names carrying a declared prefix, and restores the `+` on
  export.

## Conversion pipeline

`package::convert(&ConvertOptions) -> ConvertReport` is the whole
CityJSON→package path. It is a **two-pass** design over a unified `Source`:

```
Source (CityJSON doc, CityJSONSeq stream, CityGML 2.0 or 3.0 document, FlatCityBuf file, 3DCityDB v5 database)
   │
   ├─ pass 1 ── scan ─────────► ScanResult { CityParquetSchema, dataset metadata }
   │            (LoDs, attribute columns, CRS, transform, dataset bbox;
   │             retains no geometry buffers — just "what columns/metadata?")
   │
   └─ pass 2 ── encode ───────► stream of RecordBatch conforming to that schema
                │                (one row per CityObject; geometry → WKB via
                │                 wkb_write; semantics → the geometry_properties
                │                 struct; appearance → typed per-WKB-face MAP cells)
                │
                ├─ sort ───────► Hilbert order: external merge sort, spilling
                │                sorted runs to Arrow IPC under a memory budget
                │
                ├─ recipe ─────► per-column WriterProperties (the benchmark variable)
                │
                └─ write ──────► one <snake>.parquet per CityGML module + sidecars + metadata.json
```

- **`source`** — `Source` unifies whole-document CityJSON, streaming
  CityJSONSeq, CityGML 2.0 and 3.0 (`citygml`, which gives this crate's
  errors to the `citygml` crate of `../citygml-rs`: it builds a CityJSON
  header and streams the features of the `citygml2` or `citygml3` reader, as
  the document's namespace says) and FlatCityBuf (`fcb`, behind the `fcb` feature, bridging
  `fcb_core`'s CityJSON types to `cjseq` through JSON) and 3DCityDB v5
  (`Source::open_citydb`, behind the `citydb` feature: the `citygml-citydb`
  crate builds the database's header and assembles its rows into the same
  model objects the CityGML readers yield, read in one read-only transaction
  so both passes see one snapshot) behind one feature
  iterator, so nothing downstream cares which input shape it got. Several
  inputs merge (`merge::merge_sources`) into one `Source` that reads them in
  turn, requantising each onto the merged transform as it streams.
- **`scan`** (pass 1) — one read-only pass answering "what columns and
  dataset metadata does this need?" It never retains WKB or vertex data, so
  pass 2 can size its Arrow builders up front. It refuses a repeated object
  id, keeping one 128-bit hash per object.
- **`encode`** (pass 2) — trusts the `ScanResult` and emits `RecordBatch`es;
  fails fast if asked to encode something the schema doesn't describe. One row
  per object, geometry bucketed into the per-LoD columns.
- **`recipe`** — renders a `WriterRecipe`/`RecipePreset` into concrete
  `WriterProperties`. Column decisions are driven by field metadata
  (geoarrow.wkb, arrow.json) and the six fixed `bbox` leaf paths — **never by
  column name** — so the recipe can't drift from the schema. This is the
  paper's benchmark variable (see below).
- **`order`** / **`sort`** — the Hilbert key of a feature (the curve index of
  its vertex-pool centre against the dataset bbox) and the external sort that
  orders rows by `(key, row ordinal)`. Encode always runs in source order and
  tags each row with its feature's key; the sorter buffers rows until
  `ConvertOptions::memory_budget`, spills each sorted run as an LZ4-compressed
  Arrow IPC stream file in a directory of its own under `.cityparquet-tmp/`
  (or `spill_dir`), and merges the runs k-way, holding one run batch (at most
  1,024 rows or about 8 MiB) per run. Rows of one feature share a key and have consecutive
  ordinals, so a feature family stays contiguous in every table, as the spec
  requires. Spilled and in-memory sorts emit the same batches, so they write
  the same tables. `RowOrder::Source` skips the sort.
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

`ConvertOptions` bundles the knobs: `overwrite`, `batch_size`,
`recipe` (`WriterRecipe`), `ordering` (`RowOrder::{Hilbert, Source}`,
Hilbert by default), and the sort's `memory_budget` (2 GiB) and `spill_dir`.
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
  `CityObject`. Geometry is deliberately kept _out_ of the reassembled object
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

It streams. Tables are read in manifest order, each front to back, and rows
are regrouped into feature families by `family::Families`: a family's rows
are one run per table (the spec's contiguity rule), and when a run ends the
family is written if its root row and every child it names are present.
Otherwise it is parked until its rows in other tables arrive (a
CityObjectGroup and its members, an extension child); `ExportReport::
parked_features` counts these. A run that re-opens, or rows for a family
already written, are refused, so export also keeps one 128-bit hash per
feature written. CityJSONSeq output is written as families
complete; a CityJSON document collects every feature, since it has one
vertex pool. CityGML output passes each feature to the `citygml` crate's writer
of the version asked for (`OutputWriter` in `export.rs`), which writes it as it comes, the
envelope taken from the header's extent, and reports what the version cannot
hold when it finishes. A BuildingPart that a non-conforming package files in
a family of its own is held by the writer until its Building comes, or the
Building until the part does, and written inside it. Output goes to a hidden partial file renamed into place on success.

Its correctness hinges on **manifest authority**:

- **Implicit geometries** are rebuilt into the CityJSON `geometry-templates`
  member from `implicit_geometries.parquet` only when the manifest lists it;
  relative-geometry vertices are re-interned as raw floats. If the manifest
  doesn't list it (Core, or Compatibility with no implicit geometries), an
  object's `implicit_geometry` reference can't resolve, so the object is exported
  without its instance geometry and the drop is counted. A reference to a
  missing row _in a listed sidecar_ is a corrupt-file `Schema` error, not a
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
_independently_ of the writer on purpose — a comparator that reused the
writer's normalisation couldn't catch a bug in it, because both sides would
share the blind spot. Two layers:

1. **Index-based** (mirrors the writer): strip a ring's trailing duplicates
   of its first vertex index, drop rings left with < 3 entries, drop a surface
   whose exterior ring was dropped. Counts ring _elements_, not distinct
   coordinates — a genuinely-distinct zero-area ring passes through (data
   quality is not the format's job).
2. **Coordinate-based** (the round-trip-only case): also drop a ring whose
   surviving indices dequantise to < 3 _distinct_ coordinates. This catches
   the real 3DBAG occurrence where three distinct vertex indices all map to
   one quantised coordinate: the writer emits it (index-distinct), the WKB
   reader's `f64::to_bits` interner collapses it to one repeated index on read,
   and without this rule the two sides would spuriously differ. Applied
   identically to both sides, reusing the same strip/realign machinery.

`material`/`texture` blocks are compared by default, after both sides are
canonicalised to the flat per-WKB-face form the columns carry — a source's
per-shell nesting, its whole-geometry `value` broadcast and its writer-dropped
face positions all normalise away, and every index is dereferenced to its
definition, so the comparison is of appearance content rather than of CityJSON
spelling. `Exclusions::appearance` turns that off for the Core profile's
deliberate drops.

## STAC Collection

`cityparquet collection` (the CLI crate's `collection` module, behind its
`collection` feature) reads several packages' `metadata.json` Items and hands
them to `city3d_stac::stac::StacCollectionBuilder` from the City3D STAC tool's
generator crate, which computes the `city3d:*` summaries and the union extent;
the module adds the temporal extent and one relative `item` link per package.
Only the CLI depends on the generator: it brings the `stac`, `object_store`,
`reqwest` and `fcb_core` 0.6 stacks, which the library and the schema crate
must not carry.

## Validator

`validate` checks a package against the specification without trusting
anything this crate's writer does. It reads each file's Parquet schema and
footer key-value pairs, and decodes values with `ARROW:schema` ignored, so a
file from any writer is judged by what the Parquet logical types and the
`city`/`geo` objects say. The expected types are not restated: the reserved
columns come from `CityParquetSchema::to_arrow_schema`, the per-LoD and sidecar
columns from `cityparquet_schema::model` and `sidecar_schemas`, and a Parquet
type matches an Arrow field by logical type and nullability, descending into
`LIST` and `MAP` groups by their annotation rather than their child names.

Each check cites the specification page it comes from; a MUST is an error and
a SHOULD a warning. Where the spec states a rule about values (a required
column non-null on every row), the values are checked, and a nullable
declaration alone is a warning. The submodules split the work by subject:
`layout` (file names, object-table modules), `stac` (`metadata.json`),
`footer` (`city`, `geo`, the `GEOMETRY` annotation and its CRS), `types`
(logical types), `wkb`, `values` (per-cell invariants) and `tables` (object
tables and sidecars).

## Benchmark harness

`cityparquet-cli::bench` (`cityparquet bench`) drives the paper's variant
matrix. For each variant — a `RecipePreset` × optional `+source` (Hilbert
order otherwise) × optional `+rg<N>` row-group-size override × optional
`+<codec>[<level>]` × optional `+nobloom` — it converts `input`
into a fresh tempdir, times the write, measures package size, times a full
scan (deriving the dataset bbox while it's at it), times a bbox-pruned window
query anchored at the bbox lower-left, counts row groups touched vs total,
and (unless `--skip-roundtrip`) exports and compares for exact equality. One
CSV row per variant. Every variant converts with **Compatibility**
unconditionally, so the harness stays uniform across variants and datasets
(sidecars cost nothing when there's no appearance data).

The methodology and its comparability caveats live in
[`benchmark/formats/README.md`](../../../benchmark/formats/README.md) (write side) and
[`benchmark/formats/READ_BENCHMARK.md`](../../../benchmark/formats/READ_BENCHMARK.md) (read side, which
describes the CSVs a run writes).

### Recipe presets (the benchmark variable)

`RecipePreset` is the tuned default plus five ablations, so the paper can
quantify what each tuning rule buys:

| Preset (`--recipe`) | What it is                                                                                                                                                                                |
| ------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `cityparquet`       | the tuned default: delta-encoded ids, dictionary `object_type`, BYTE_STREAM_SPLIT bbox leaves, no stats/dictionary on WKB+JSON, bloom filters on ids and high-cardinality strings, zstd 3 |
| `parquet-defaults`  | parquet-rs defaults + the recipe's global compression & row-group size only — the "untuned writer" comparator                                                                             |
| `no-dictionary`     | `cityparquet` minus dictionary encoding everywhere                                                                                                                                        |
| `no-bss`            | `cityparquet` minus BYTE_STREAM_SPLIT on the bbox leaves                                                                                                                                  |
| `no-delta`          | `cityparquet` minus DELTA_BYTE_ARRAY on `id`/`feature_id`                                                                                                                                 |
| `snappy`            | `cityparquet` with Snappy instead of zstd (DuckDB COPY's default codec)                                                                                                                   |

KV metadata is embedded under every preset — it is never a benchmark
variable. Row-group size and zstd level remain independent CLI knobs on top.

Bloom filters are orthogonal to the preset: `--no-bloom` (variant `+nobloom`)
turns them off under any preset, and `parquet-defaults` writes none. The
attribute columns that get one are chosen by `scan` (a HyperLogLog distinct
count per string attribute, dataset-wide) and passed to
`WriterRecipe::writer_properties`.

## Testing discipline

Tests read **real CityJSON fixtures** (`delft.city.jsonl`,
`lod3_railway.city.json`), never inline hand-written CityJSON; edge cases are
derived from real fixtures/tiles in tempdirs. Development is strict red-green
TDD. `just check` runs clippy (`-D warnings`) and the full test suite with every feature on (the object-store transport included), the
schema/Parquet isolation check, and `cargo fmt --check`; `just interop`
additionally has DuckDB read the written Parquet natively to confirm the files
are plain, portable Parquet.
