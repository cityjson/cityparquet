# CityParquet `merge` / `split` CLI subcommands — design

Date: 2026-07-10
Status: approved (brainstorming), pending implementation plan

## Goal

Add two CLI subcommands to the `cityparquet` binary:

- `merge` — combine N CityParquet packages into one.
- `split` — divide one CityParquet package into N spatial tiles.

Primary motivation: stitch the per-tile packages produced by
`scripts/encode_3dbag_tiles.sh` into a single national package, and re-tile a
large package into fixed-size tiles. `split` then `merge` is a round-trip that
must reproduce the original package (modulo row order).

## Scope decisions (locked during brainstorming)

- **Both** operations ship together.
- **Core profile only** in v1. A Compatibility-profile input (non-empty
  `sidecar_files` / materials / textures / geometry_templates) makes **both**
  commands fail with a clear "not yet supported" error. Rationale: appearance /
  template row-IDs are per-package and would need re-interning (merge) or
  pruning/copying (split); the 3DBAG use case is pure geometry + attributes.
- **Split tiling** is specified by `--tile-size` in CRS units; origin defaults
  to the package bbox-min corner (overridable with `--origin X,Y`).
- **Mismatch policy for merge is strict** — error on any CRS / transform /
  profile / per-table-schema mismatch. No reconciliation.
- **Split is 2-pass and family-safe** — every CityObject family (a shared
  `feature_id`) is routed to a single tile so parent/child references never
  dangle across tiles.
- Implementation lives in **one module**, `crates/cityparquet/src/partition.rs`.

## Background facts (verified in the codebase)

- A CityParquet **package** is a directory: one or more object `*.parquet`
  tables (by-type: `building.parquet`, ...; or single: `cityobjects.parquet`),
  a `metadata.json` manifest (`PackageManifest`), and — Compatibility only —
  sidecar tables `materials.parquet` / `textures.parquet` /
  `geometry_templates.parquet`.
- Geometry is stored as **WKB with absolute f64 coordinates** (the dataset
  `transform` is applied on write, `crate::wkb_write::VertexPool`). With a
  strict same-CRS/transform gate, object rows across packages are directly
  concatenable — **no coordinate rewriting**.
- Every object row carries a nullable `feature_id` reserved column
  (`cityparquet_schema::model`), plus `id`, `parents`, `children`,
  `children_roles`, and a 6-leaf `bbox` struct column
  (`bbox.{x,y,z}{min,max}`, all `Float64`).
- File-level Parquet KV metadata round-trips through
  `CityParquetMetadata::{to_key_values, from_key_values}` and carries CRS,
  transform, columns, `default_geometry`, `bbox_column`, `sidecar_files`.
- Reusable building blocks:
  - `reader::CityParquetReaderBuilder::cityparquet_metadata() ->
    CityParquetMetadata` — reads a table's KV metadata.
  - `reader::row_group_intersects` / bbox-leaf stats helpers — cheap
    per-row-group bbox bounds without a full scan.
  - `recipe::WriterRecipe::writer_properties(schema, metadata) ->
    WriterProperties` — builds writer props with the KV metadata embedded.
  - `package::{commit_package, purge_stale_package_files, TMP_DIR_NAME,
    RESERVED_PACKAGE_FILES}` — the crash-safe write-to-`.cityparquet-tmp`,
    then atomic-swap commit. These are currently private and will be lifted to
    `pub(crate)`.
  - `recipe::{RecipePreset, Codec, WriterRecipe}` and the `convert` CLI's arg
    parsing for `--recipe` / `--compression` / `--row-group-size` /
    `--zstd-level`.

## CLI surface

```
cityparquet merge <PKG>... --output <DIR> [--overwrite]
    [--recipe NAME] [--compression CODEC] [--row-group-size N] [--zstd-level L]

cityparquet split <PKG> --output <DIR> --tile-size <F> [--origin X,Y]
    [--overwrite]
    [--recipe NAME] [--compression CODEC] [--row-group-size N] [--zstd-level L]
```

- Positional inputs are package **directories**.
- No `--layout` flag: layout is inherited from inputs. Merge groups by table
  filename; split preserves each row's original table filename.
- Recipe-flag parsing is factored out of `main.rs` so `convert`, `merge`, and
  `split` share it (Core-profile `WriterRecipe` construction:
  `statistics_for_json: false`).
- Output prints a terse machine-readable summary line (matching the existing
  convert/export/compare style), exact fields listed under each command below.

## Module: `crates/cityparquet/src/partition.rs`

Public API:

```rust
pub struct MergeOptions {
    pub inputs: Vec<PathBuf>,
    pub output_dir: PathBuf,
    pub overwrite: bool,
    pub recipe: WriterRecipe,
}
pub struct MergeReport {
    pub input_packages: usize,
    pub object_count: usize,
    pub tables: usize,
    pub files: usize,
}
pub fn merge(opts: &MergeOptions) -> Result<MergeReport>;

pub struct SplitOptions {
    pub input: PathBuf,
    pub output_dir: PathBuf,
    pub overwrite: bool,
    pub tile_size: f64,
    pub origin: Option<[f64; 2]>,
    pub recipe: WriterRecipe,
}
pub struct SplitReport {
    pub tiles_written: usize,
    pub object_count: usize,
    pub feature_count: usize,
}
pub fn split(opts: &SplitOptions) -> Result<SplitReport>;
```

Shared internal helper (also used by `convert`, factored out of
`package::write_package`'s writer loop):

```rust
pub(crate) fn write_table_batches(
    path: &Path,
    schema: SchemaRef,
    props: &WriterProperties,
    batches: impl IntoIterator<Item = Result<RecordBatch>>,
) -> Result<usize>; // returns rows written
```

### Package reading

A small internal reader wrapper opens a package directory:

- parse `metadata.json` -> `PackageManifest`;
- for each table filename in `manifest.tables`, open a
  `CityParquetReaderBuilder`, exposing its `CityParquetMetadata` and a batch
  iterator.

Compatibility gate: if `manifest.profile != Profile::Core` or
`!manifest.sidecar_files.is_empty()`, return an error:
`"merge/split supports Core-profile packages only; <dir> is Compatibility
(has sidecar tables). Re-run once appearance re-interning lands."`

## merge algorithm

1. Reject empty `inputs`.
2. Open every input package; run the Compatibility gate on each.
3. **Strict validation** across inputs, error message names the offending
   package:
   - identical `cityparquet_version`;
   - identical CRS (`CityParquetMetadata::crs`);
   - identical transform (`CityParquetMetadata::transform`);
   - all Core profile;
   - for every table filename present in more than one input, identical Arrow
     schema (attribute columns + per-LoD geometry columns) — compare the
     `CityParquetSchema` / Arrow `Schema` of that table.
4. Compute the union set of table filenames across inputs (first-appearance
   order).
5. For each output table filename: build `WriterProperties` from that table's
   `CityParquetMetadata` (equal across inputs under the gate) + `opts.recipe`;
   stream every input-that-has-this-table's batches into one output writer via
   `write_table_batches`, writing into `output_dir/<TMP_DIR_NAME>/<filename>`.
6. Write merged `metadata.json`: `profile = Core`, `tables = union`,
   `lods = union of input manifests' lods` (spec order),
   `sidecar_files = []`, `cityparquet_version = CITYPARQUET_VERSION`.
7. `commit_package` (honours `overwrite`; errors if `output_dir` exists and not
   `overwrite`, matching `convert`).
8. Return `MergeReport { input_packages, object_count, tables, files }`.
   Stdout: `"<input_packages> <object_count> <tables> <files>"`.

**No dedup.** Merge concatenates rows; it assumes inputs are spatially disjoint
(true for 3DBAG tiles). Overlapping inputs yield duplicate `id`s — documented,
not enforced.

## split algorithm

Coordinates are absolute, so tiling is pure row routing.

1. Open the input package; run the Compatibility gate.
2. Validate `tile_size > 0.0` (else error).
3. **Origin**: if `--origin` given, use it; else default to
   `[min(bbox.xmin), min(bbox.ymin)]` read from Parquet row-group statistics
   across all tables (reuses the bbox-leaf-stats helpers; no full scan). If no
   table has any bbox stats (empty / geometry-less package), origin = `[0,0]`.
4. **Pass A (assign)** — stream `feature_id` + `bbox` from every table:
   - accumulate `HashMap<feature_id, [f64;4]>` = per-feature union of
     (xmin,ymin,xmax,ymax);
   - a row with **null `feature_id`** is its own singleton group keyed by a
     synthetic unique id (routed by its own bbox);
   - after the pass, map each group to a tile:
     `col = floor((xmin - ox)/size)`, `row = floor((ymin - oy)/size)`
     from the group's bbox min corner. A group whose bbox is entirely null
     (no geometry) routes to the origin tile `(0, 0)`.
   - Result: `HashMap<feature_id, (col,row)>` and, for null-feature rows, a
     parallel per-row decision recomputed in Pass B from the row's own bbox
     (so no per-row id bookkeeping is needed — null-feature rows are
     deterministic from their own bbox alone).
5. **Pass B (write)** — re-stream every table; for each batch, partition rows
   by destination tile (boolean mask per tile via
   `arrow_select::filter::filter_record_batch`), append each masked sub-batch
   to that tile's writer for the **same table filename**, under
   `output_dir/tile_<col>_<row>/<TMP_DIR_NAME>/<filename>`. Non-null
   `feature_id` rows use the Pass-A map; null-`feature_id` rows recompute their
   tile from their own bbox (origin tile if bbox null).
6. For each non-empty tile: write its `metadata.json` (same CRS / transform /
   profile / columns; `tables` = filenames that received >=1 row; `lods` = input
   manifest's lods), then `commit_package` the tile directory.
7. Return `SplitReport { tiles_written, object_count, feature_count }`.
   Stdout: `"<tiles_written> <object_count> <feature_count>"`.

Tile directory name: `tile_<col>_<row>` (col/row are signed integers).

### Split edge cases

- Feature spanning multiple table files (by-type layout, a family with objects
  of different 1st-level types): handled because `feature_id` grouping is
  global across all tables in Pass A.
- Feature with objects in multiple candidate tiles: the whole family goes to
  the tile of its **union bbox min corner** — one tile, deterministic.
- Null `feature_id`: routed individually by own bbox.
- Null bbox (no geometry) with null feature_id: origin tile.
- Empty package: `tiles_written = 0`, no output tiles, success.

## `main.rs` wiring

- Add `Merge` and `Split` variants to `Commands`.
- Extract the `convert` recipe/compression parsing into a shared helper
  returning `Result<WriterRecipe, String>` (or an exit code), reused by all
  three commands.
- Parse `--origin "X,Y"` into `[f64;2]` (error on malformed).
- Map `partition::{merge,split}` reports to the terse stdout lines and exit
  codes (0 success, 1 error), matching existing commands.

## Testing (strict red-green TDD, real fixtures only)

Fixtures: `delft.city.jsonl`, `lod3_railway.city.json` (no inline artificial
CityJSON). Library tests in `crates/cityparquet`, CLI tests in
`crates/cityparquet-cli/tests/cli.rs`.

Keystone property tests:

1. **split∘merge identity** — `convert(fixture)` → `split(tile-size)` →
   `merge(all tiles)` → `compare_datasets(merged, original-convert)` reports
   **equal** (modulo row order). Try >=2 tile sizes: one that yields a single
   tile, one that yields several.
2. **merge of disjoint halves == whole** — split a package into 2 tiles, merge
   them back, compare equal to the un-split package.
3. **tile containment** — every feature in `tile_<col>_<row>` has its bbox min
   corner inside that tile's `[ox+col*size, ox+(col+1)*size) x [...]` bounds.
4. **coverage** — union of all tiles' `feature_id`s == input's `feature_id`s;
   summed object counts match.
5. **family integrity** — no `feature_id` appears in more than one tile.
6. **Compatibility rejected** — a Compatibility-profile package makes both
   `merge` and `split` return the gate error.
7. **strict-merge errors** — merging packages with differing CRS / transform /
   schema errors and names the offending package.
8. **CLI** — `merge`/`split` happy-path invocations produce the documented
   stdout summary and a valid, `export`-able output package.

## Reuse / refactor summary

- Lift `commit_package`, `purge_stale_package_files`, `TMP_DIR_NAME`,
  `RESERVED_PACKAGE_FILES` in `package.rs` to `pub(crate)`.
- Factor `write_table_batches` out of `package::write_package` and reuse it
  there (behaviour-preserving).
- Factor `convert`'s recipe/compression/origin arg parsing in `main.rs` into
  shared helpers.
- `pub mod partition;` in `crates/cityparquet/src/lib.rs`.

## Explicitly deferred (documented future work)

- Compatibility-profile support: merge re-interns materials/textures/templates
  into a global `AppearanceInterner`, rewriting each geometry's parallel-JSON
  index maps; split copies the full sidecar tables to every tile (correct, some
  duplication) — pruning to referenced IDs is a later optimisation.
- Schema **union** on merge (fill-missing-with-null for differing attribute /
  LoD sets) — v1 requires identical schemas.
- Merge **dedup** of overlapping inputs.
- `--grid NxM` split alternative.
