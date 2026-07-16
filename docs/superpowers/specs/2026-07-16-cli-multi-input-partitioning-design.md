# CLI Multi-Input & Spatial Partitioning — Design

**Status:** approved (user, 2026-07-16), Fable-advised.

**Goal:** Extend the `convert` CLI command to (1) accept multiple / wildcard
inputs (CityJSON, CityJSONSeq, CityGML) merged into one dataset, and (2)
optionally **partition** the result into N self-contained CityParquet packages
by a chosen method. This is research infrastructure for a downstream benchmark
measuring how partitioning method × granularity affects read/query performance
over many Parquet files.

## Decisions (locked)

| Question | Decision |
|---|---|
| Output shape when partitioned | **N independent, self-contained CityParquet packages** under one output dir. Each has its own `metadata.json` (STAC Item) + sidecars. |
| Sizing knob | **Method-specific.** No forced "exactly N files" on spatial methods. |
| Methods this milestone | `count`, `features`, `box`. **Defer** `h3`, `s2` (no reprojection dependency taken on now). |
| Multi-input CRS mismatch | **Hard error.** No silent reprojection, no per-input fallback. |
| Multi-input transform mismatch | **Requantise** all features onto one merged transform (heterogeneous 3DBAG tiles must work). Single input → transform untouched (byte-identical to today). |
| Duplicate feature IDs across inputs | **Warn + counter**, keep both. |

## CLI surface

`convert` changes from `convert <INPUT> <OUTPUT>` to variadic inputs + an
output flag (breaking, acceptable for this private research tool):

```
cityparquet convert <INPUTS>... --output <DIR> [existing flags]
    [--partition <count|features|box>]
    [--number <N>]         # count:    split into N packages (contiguous chunks)
    [--feature-num <M>]    # features: <= M features per package
    [--cell-size <METRES>] # box:      square grid, cell edge in CRS units
```

- `--output`/`-o` takes the output directory (was the 2nd positional).
- Each `<INPUT>` may be a **file**, a **directory**, or a **glob pattern**:
  - file → used directly.
  - directory → collect immediate children whose extension is one of
    `json`, `jsonl`, `gml` (non-recursive; `*.city.json`/`*.city.jsonl` match
    via the `json`/`jsonl` extension).
  - glob (contains `*`, `?`, or `[`) → expanded with the `glob` crate; each
    match is then resolved as a file (globs that match directories are
    skipped with a warning).
  - Inputs are de-duplicated (canonicalised) and sorted for deterministic
    order before merging.
  - Zero resolved inputs → error.
- No `--partition` → **one** package written directly at `<DIR>` (current
  behaviour; single input required to keep the common path unchanged, OR
  multiple inputs merged into one package at `<DIR>`).
- `--partition` requires its matching sizing flag; a method with the wrong (or
  missing) sizing flag → clear error. `--number`/`--feature-num`/`--cell-size`
  without `--partition` → error.

### Partition method semantics

| Method | Sizing flag | Assignment | Package subdir name |
|---|---|---|---|
| `count` | `--number N` (N≥1) | contiguous chunk `floor(i·N / total)` over merged feature order | `count-00000`, `count-00001`, … |
| `features` | `--feature-num M` (M≥1) | contiguous chunk `floor(i / M)` | `features-00000`, … |
| `box` | `--cell-size S` (S>0, CRS units) | grid cell `(floor(cx/S), floor(cy/S))` of the feature's real-world centroid; absolute origin | `box_x{ix}_y{iy}` (signed ints, e.g. `box_x150_y-3`) |

- `box` centroid: `(min+max)/2` of `order::vertices_minmax(feature.vertices,
  transform)` (x,y only). A feature with **no vertices** → a dedicated
  `box_none` package (never silently dropped).
- Empty partitions are simply never created (spatial grids are sparse).
- Partition packages are written **sequentially** (one at a time), each via the
  existing per-package crash-safe temp-dir swap. Deterministic subdir order.

## Architecture

Three seams, smallest-blast-radius first:

### 1. Input resolution (`cli` / new `crates/cityparquet/src/inputs.rs`)

`resolve_inputs(patterns: &[PathBuf]) -> Result<Vec<PathBuf>>`: expand
directories + globs → a de-duplicated, sorted `Vec<PathBuf>` of concrete files.
Pure path logic, unit-testable with a temp dir.

### 2. In-memory `Source` + merge (`crates/cityparquet/src/source.rs`)

Add an in-memory `Source` variant so buffered features can flow through the
unchanged `scan`/`encode_buffered`/`convert` machinery:

- New private field shape: `Source` gains a `Buffered { features: Vec<CityJSONFeature> }`
  representation. `features()` returns `FeatureIter::BufferedRef(slice iter)`;
  `header()`, `format()`, `doc_appearance()` serve stored values.
- `Source::from_parts(header: CityJSON, features: Vec<CityJSONFeature>,
  doc_appearance: Option<Appearance>, format: SourceFormat) -> Source` — the
  constructor the merge/partition layer uses.

`merge_sources(sources: &[Source]) -> Result<MergedDataset>` (new
`crates/cityparquet/src/merge.rs`):
- CRS check: every source's `metadata.reference_system` must be equal
  (serialised form); mismatch → `Err`.
- Merged transform: if **all** transforms are equal → adopt it and keep every
  feature's vertices untouched (single-input and homogeneous multi-input fast
  path — byte-identical output). Otherwise pick a canonical merged transform
  (`translate = [0,0,0]`, `scale = componentwise-min of source scales`) and
  **requantise** every feature: for each vertex `v`, real = `v·srcScale +
  srcTranslate`, then `v' = round((real − mergedTranslate)/mergedScale)`.
  Requantise only the `vertices` pool (feature geometries index into it;
  `vertices-texture` UVs are unitless and untouched).
- Merged header: clone the first source's header, replace `transform` with the
  merged transform, union `geometry_templates` + doc `appearance` (v1: assume
  at most one source carries doc-level templates/appearance; if more than one
  does, error — documented limitation, templates are rare and the benchmark
  data is CityJSONSeq with feature-local appearance).
- Duplicate feature IDs across sources: count + warn; keep all.
- Returns `MergedDataset { header, features: Vec<CityJSONFeature>,
  doc_appearance, duplicate_ids: usize }`.

### 3. Partition + convert driver (`crates/cityparquet/src/partition.rs`)

- `PartitionSpec` enum: `Count(usize)`, `Features(usize)`, `Box { cell: f64 }`.
- `assign_partitions(features: &[CityJSONFeature], spec: &PartitionSpec,
  transform: &Transform) -> Vec<(PartitionKey, Vec<usize>)>`: returns groups of
  feature indices keyed by a `PartitionKey` that renders to the subdir name.
  Pure, unit-testable (the heart of the benchmark correctness).
- `convert_source(source: &Source, opts: &ConvertCore) -> Result<ConvertReport>`:
  extracted from today's `convert` — everything after `Source::open`.
  `convert(opts)` becomes `Source::open` + `convert_source`.
- Top-level driver `convert_partitioned(inputs, output_dir, spec, opts)`:
  resolve inputs → open each `Source` → `merge_sources` → `assign_partitions` →
  for each group build an in-memory `Source` (merged header + that group's
  features + merged doc_appearance) → `convert_source` into
  `output_dir/<subdir>` → collect a per-partition report.
- Report: `PartitionReport { partitions: Vec<(String, ConvertReport)>,
  duplicate_ids: usize }`, printed as a summary.

### CLI wiring (`crates/cityparquet-cli/src/main.rs`)

`Convert` command: `inputs: Vec<PathBuf>` (positional, variadic), `output:
PathBuf` (`-o`/`--output`), plus `partition: Option<String>`, `number`,
`feature_num`, `cell_size`. Parse/validate the method + sizing combination into
a `PartitionSpec`; `None` partition → existing single-package path (one merged
package). Print a partition summary on success.

## Composition with existing flags

All existing `convert` flags (`--profile`, `--recipe`, `--ordering hilbert`,
`--layout`, `--geoarrow`, `--row-group-size`, …) apply **per partition
package** unchanged. `--ordering hilbert` composes: Hilbert orders rows *within*
each partition; partitioning splits *across* files → a clean 2-D benchmark grid
(partition × ordering). Partitioning inherits the same buffer-all memory
profile Hilbert already has (documented).

## Benchmark payoff

Each partition package's `metadata.json` carries its own bbox + feature count
(computed by the existing `scan` over that partition's features). DuckDB
globbing `OUTPUT/*/building.parquet` then gets package-level spatial pruning
"for free" from those bboxes + Parquet footer stats — the read/query effect the
downstream benchmark measures.

## Non-goals (this milestone)

- `h3` / `s2` methods and any CRS reprojection (deferred; `box`+`count`+
  `features` carry the spatial-coherence-vs-granularity comparison).
- Combining a spatial method with a feature cap.
- Recursive directory traversal (immediate children only).
- Merging **multiple** sources that each carry doc-level geometry templates /
  appearance (errors; single-carrier is supported).
- Streaming (non-buffered) partitioning.

## Testing strategy

Strict red-green TDD, **real fixtures only** (no inline artificial CityJSON) —
`delft.city.jsonl`, `lod3_railway.city.json`, and the existing hand fixtures.

- `resolve_inputs`: temp dir with files/subdir/glob → expected resolved set.
- `merge_sources`: two copies of a fixture with differing transforms →
  requantised merged vertices round-trip to the same real coords (via
  `vertices_minmax`); CRS mismatch → error; duplicate-id count.
- `assign_partitions`: `count`/`features` chunk boundaries exact; `box` cell
  assignment for known centroids; vertexless feature → `box_none`.
- End-to-end (`tests/`): convert `delft.city.jsonl` with each method →
  assert package count, that the union of all partitions' object counts equals
  the single-package object count (no feature lost/duplicated), and each
  partition package independently `export`s + `compare`s clean against its own
  feature subset.
- Multi-input: two disjoint slices of `delft.city.jsonl` (different transforms)
  → merged single package == original single-package convert (semantic
  `compare`), proving lossless requantised merge.

## Milestone close-out

- Whole-branch review; **Codex external review with the `sol` model**; triage +
  fix Critical/Important.
- `just check` green; merge to `main`; delete branch.
- Update milestone memory: multi-input + partitioning (box/count/features)
  done; H3/S2 + reprojection noted as the next partitioning follow-up.
