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
