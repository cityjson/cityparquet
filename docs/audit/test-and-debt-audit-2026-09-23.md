# Test-pruning and architecture-debt audit — 2026-09-23

Scope, in priority order: `lib/cityparquet-rs`, `lib/duckdb-3d`,
`lib/duckdb-cityjson`. This is an audit only: no test or source file is changed.

## 1. Summary

_Written at wrap-up._

## 2. Architecture map

### `lib/cityparquet-rs` (Rust, three crates)

Crate direction is clean and acyclic: `cityparquet-cli` → `cityparquet`
(core) → `cityparquet-schema`. The schema crate has no `arrow-array` or
`parquet` dependency, and `just isolation` enforces that.

Inside `cityparquet` (core, ~33 modules, ~55k lines including tests) there is
**no internal layering**. The `crate::<module>` path graph (a lower bound: it
misses `use crate::{…}` groups) has cycles at every level:

| Cycle                                                            | Where                                                                                               |
| ---------------------------------------------------------------- | --------------------------------------------------------------------------------------------------- |
| `wkb_read` ↔ `wkb_write`                                         | the geometry codec's two halves import each other                                                   |
| `encode` ↔ `package` ↔ `scan` ↔ `recipe`                         | the write pipeline's stages and its orchestrator import each other                                  |
| `decode` ↔ `reader` ↔ `geometry_encoding`                        | the read path                                                                                       |
| `query` ↔ `query_async` ↔ `query_core`                           | the sync, async and shared query layers import each other                                           |
| `export` ↔ `compare`, `export` ↔ `sidecar`, `export` ↔ `citygml` | `export` is a hub: 11 modules import it, including `decode`, `order`, `stac` and the CityGML writer |
| `package` → `compare`                                            | the write orchestrator depends on the round-trip comparator                                         |

Public interfaces: `package::convert(&ConvertOptions)`,
`export::export(&ExportOptions)`, `compare`, `query`/`query_async`, and the
`CityParquetReaderBuilder` extension trait on Parquet's `ArrowReaderBuilder`.
That trait is the **only** `pub trait` in the core crate. Every other seam is a
concrete type or free function: `Source` is one struct that switches on a
`SourceFormat` tag and on which optional field is populated (`doc`, or
`buffered` for in-memory merge/partition input), rather than a trait with one
implementation per input. Package output always goes to a local directory:
12 core modules call `std::fs` directly. `object_store` is used only by the feature-gated
async query path. `lib.rs` exports 28 of the 33 modules as `pub`, so the
crate's public surface is effectively its whole internal layout.

### `lib/duckdb-3d` (C++ DuckDB extension, ~7k lines)

There are two layers, and the dependency points the right way. `src/kernel/`
(WKB parsing, the solid model, validation, measurements, triangulation, CRS)
includes no DuckDB headers. `src/functions/` holds the DuckDB scalar-function
bindings and depends on `kernel/`. Tests are split into `test/cpp` (kernel
unit tests) and `test/sql` (SQLLogic, 33 files, ~400 records).

### `lib/duckdb-cityjson` (C++ DuckDB extension, ~22k lines excluding the generated 29k-line EPSG table)

All code sits in one flat `src/cityjson/` namespace, with 49 headers. It handles
format I/O for CityJSON, CityJSONSeq, FlatCityBuf, OBJ, glTF and GeoParquet,
the CityParquet package write/insert/merge/delete/validate operations, WKB, and
appearance. `copy_function.cpp` (1.7k lines) and `cityjson_types.cpp` (1k)
are the hubs. Tests are `test/sql` (70 files, ~1.3k SQLLogic records),
`test/cpp` (9) and `test/wasm`.

_Detailed findings follow in the per-module sections._

## 3. Part 1 — tests to delete, merge or modify

### 3.0 Inventory and measured cost

`lib/cityparquet-rs` has 801 test functions. None is property-based or a
snapshot test.

| Crate                | Inline unit (`src/`) | Integration (`tests/`)                      | Measured                              |
| -------------------- | -------------------- | ------------------------------------------- | ------------------------------------- |
| `cityparquet-schema` | 100                  | 2 (1 file)                                  | 25 s CPU                              |
| `cityparquet` (core) | 381                  | 279 (45 files; 6 `#[ignore]`, bloom/DuckDB) | 355 s unit + ~2,700 s integration CPU |
| `cityparquet-cli`    | 3                    | 36 (`cli.rs`, `bench_smoke.rs`)             | 469 s CPU                             |

This comes from a single run of `cargo nextest run --workspace --all-features`
on a 128-core host: 795 passed, 6 skipped, **2 min 46 s wall-clock** and
**3,318 s summed test time**. The wall-clock figure is set by one test:
`bench_smoke::bench_run_produces_the_default_nine_variant_matrix_for_delft`
takes 129 s. Most of the summed cost is full convert → export → compare
round trips on the `delft` and `lod3_railway` fixtures, at 15–60 s each.

Slowest tests:

| s   | Test                                                                                        |
| --- | ------------------------------------------------------------------------------------------- |
| 129 | `cli::bench_smoke::bench_run_produces_the_default_nine_variant_matrix_for_delft`            |
| 63  | `core::roundtrip_real_data::every_recipe_preset_round_trips_delft_losslessly`               |
| 51  | `cli::bench_smoke::bench_run_compression_variants_differ_in_total_bytes`                    |
| 42  | `cli::bench_smoke::bench_run_accepts_a_zstd_level_suffix_and_the_levels_differ_in_bytes`    |
| 36  | `core::citygml_buildingparts::delft_building_parts_round_trip`                              |
| 34  | `cli::cli::convert_with_compression_override_changes_output_size_and_round_trips`           |
| 32  | `core::bloom_real_data::partitions_share_the_dataset_wide_attribute_decision`               |
| 30  | `core::bloom_real_data::attr_filter_prunes_filtered_string_columns_and_counts_exactly`      |
| 28  | `core::compare::tests::compare_detects_a_mutated_geometry_instance_template_material`       |
| 27  | `core::compare::tests::compare_detects_an_added_geometry_instance_template_semantics_block` |

Summed test time by binary (top): core inline units 356 s, `export_real_data`
312 s, `convert_real_data` 281 s, `roundtrip_real_data` 255 s,
`bloom_real_data` 240 s, `bench_smoke` 239 s, `cli` 230 s,
`partition_real_data` 151 s.

On flakiness: nothing has a retry marker. The only skips are six `#[ignore]`
tests in `bloom_corpus.rs` and `bloom_duckdb_interop.rs`, which need an external
corpus or DuckDB. Fixture downloads retry in the `justfile`
(`--retry-all-errors`), which points to network flakiness in CI setup rather
than in the tests.

`lib/duckdb-3d` has 18 kernel C++ test files (`test/cpp`) and 33 SQLLogic
files (~400 records). `lib/duckdb-cityjson` has 70 SQLLogic files (~1.3k
records), 9 C++ files and a WASM suite. Their timings are in their own
sections.
