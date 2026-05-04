# CityLake - AI Agent Instructions

## Project Aim

CityLake is a data lake framework and web API for storing and querying 3D city models in CityJSON format. It uses DuckDB as an in-memory analytical database with two key extensions:

- **duckdb-cityjson**: Community extension that is the **sole** path for CityJSON I/O in this project. Reads/writes CityJSON, CityJSONSeq, and FlatCityBuf natively via SQL:
  - Read: `read_cityjson(path, [lod])`, `read_cityjsonseq(path, [lod])`, `read_flatcitybuf(path, [lod])` — accept local paths and HTTP/HTTPS/S3/GCS URLs.
  - Write: `COPY (...) TO '...' (FORMAT cityjson | cityjsonseq | flatcitybuf)`.
  - Metadata: `cityjson_metadata(path)`, `cityjsonseq_metadata(path)`, `flatcitybuf_metadata(path)`.
  - Schema is auto-inferred; geometry columns are encoded as WKB.
  - Install: `INSTALL cityjson FROM community; LOAD cityjson;`
- **ducklake**: Data lake catalog extension providing ACID transactions, time travel, and Parquet-based storage. Install: `INSTALL ducklake; LOAD ducklake;`. Attach: `ATTACH 'ducklake:metadata.ducklake' AS citylake;`

Reference implementation: https://github.com/HideBa/cityparquet/tree/main/citylake
DuckDB CityJSON extension: https://github.com/cityjson/duckdb-cityjson
DuckLake docs: https://ducklake.select/docs/stable/

## Architecture

This project follows **clean architecture** with clear separation of concerns:

### Layer 1: Interface (`src/core/interface/`)
- `repository.rs` — `CityLakeRepository` trait defining all database operations
- `types.rs` — Configuration, metadata, error types, and DTOs

### Layer 2: Implementation (`src/core/db/`)
- `service.rs` — `DuckLakeService` struct implementing `CityLakeRepository`
- One file per operation: `table.rs`, `insert.rs`, `update.rs`, `delete.rs`, `query.rs`, `metadata.rs`, `export.rs`, `compaction.rs`

### Layer 3: HTTP API (`src/app/`)
- `server.rs` — Axum router and server startup
- `handlers/` — One handler file per endpoint group
- `middleware/` — CORS, tracing

## Folder Structure

```
src/
├── main.rs                  # Binary entry point
├── lib.rs                   # Library exports
├── core/
│   ├── mod.rs
│   ├── interface/
│   │   ├── mod.rs
│   │   ├── repository.rs    # CityLakeRepository trait
│   │   └── types.rs         # Config, metadata, error types
│   └── db/
│       ├── mod.rs
│       ├── service.rs       # DuckLakeService
│       ├── table.rs         # Table creation/existence
│       ├── insert.rs        # Insert via cityjson extension
│       ├── update.rs        # Update by ID
│       ├── delete.rs        # Delete by ID
│       ├── query.rs         # Query with filters
│       ├── metadata.rs      # CityJSON metadata queries
│       ├── export.rs        # Export to CityJSON formats
│       └── compaction.rs    # DuckLake compaction
└── app/
    ├── mod.rs
    ├── server.rs            # Axum router
    ├── handlers/
    │   ├── mod.rs
    │   ├── table.rs
    │   ├── insert.rs
    │   ├── update.rs
    │   ├── delete.rs
    │   ├── query.rs
    │   ├── export.rs
    │   └── compaction.rs
    └── middleware/
        └── mod.rs
```

## Coding Rules

- **Language**: Rust (edition 2021)
- **Error handling**: Use `thiserror` for library errors, `anyhow` sparingly for application-level errors. The repository trait returns `RepositoryResult<T>`.
- **Async**: Use `async-trait` for the repository interface. Handlers are async axum handlers.
- **Database access**: Always go through the `CityLakeRepository` trait. Never access DuckDB directly from handlers.
- **All CityJSON I/O goes through duckdb-cityjson — no own implementation.** This applies to inserts, updates, deletes, queries, exports, and metadata extraction. Specifically:
  - Do NOT parse, decode, or construct CityJSON in Rust (e.g., with `serde_json`, custom structs, or hand-rolled vertex/geometry handling). The extension's SQL functions are the only legitimate boundary.
  - Do NOT write temp `.jsonl` files just to round-trip data through `read_cityjsonseq` — prefer SQL-native operations (e.g., `INSERT INTO ... SELECT ... FROM read_cityjsonseq('source')` directly against the source path, or in-place SQL updates against existing columns). If a temp file is genuinely unavoidable, document why in a comment.
  - Do NOT re-implement format detection, schema mapping, vertex pooling, or LOD filtering in Rust — pass the optional `lod` argument to the read functions instead.
  - Treat the cityjson extension as a hard dependency: `DuckLakeService::new` must `LOAD cityjson` before any table operation.
- **SQL injection prevention**: Use parameterized queries where possible. For table names (which can't be parameterized), validate against `[a-zA-Z0-9_]`.
- **Thread safety**: DuckDB Connection is not Send. Wrap in `Arc<Mutex<Connection>>`.
- **Feature flags**: The `server` feature gates web framework dependencies. The library can be used without the server.

## Storage layout — LOD-aware tables

Every ingested dataset produces **one table per LOD** plus a single shared metadata
table. Naming convention: `{base}_lod_X_Y` for LOD `X.Y` (e.g. `buildings_lod_2_2`).
The base name defaults to `city_objects` when not supplied. Available LODs are
discovered from the source via `DESCRIBE SELECT * FROM read_cityjson*('path')`,
scanning for columns matching `geom_lodX_Y` — no Rust-side parsing.

The shared `cityjson_metadata` table accumulates one row per ingest, prefixed with
`dataset` (= base name) and `source_path` columns; the rest of the columns mirror
the cityjson extension's `*_metadata()` table function.

LOD strings are validated through the `LodKey` newtype (`src/core/interface/types.rs`).

## Key SQL Patterns

```sql
-- Create per-LOD table from a CityJSON source
CREATE TABLE citylake.{base}_lod_2_2 AS
  SELECT * FROM read_cityjsonseq('{path}', lod => '2.2');

-- Insert into a specific LOD table
INSERT INTO citylake.{base}_lod_2_2
  SELECT * FROM read_cityjsonseq('{path}', lod => '2.2');

-- Export a single LOD table (multi-LOD round-trip is deferred — see tasks.md)
COPY (SELECT * FROM citylake.{base}_lod_2_2) TO '{path}' (FORMAT cityjsonseq);

-- Persist metadata (first ingest creates the table)
CREATE TABLE citylake.cityjson_metadata AS
  SELECT '{base}' AS dataset, '{path}' AS source_path, m.*
  FROM cityjsonseq_metadata('{path}') m;
INSERT INTO citylake.cityjson_metadata
  SELECT '{base}' AS dataset, '{path}' AS source_path, m.*
  FROM cityjsonseq_metadata('{path}') m;
```

## Dev Commands

```bash
cargo build                       # Library + binary (default features include `server`)
cargo build --no-default-features # Library only (no axum/tower deps)
cargo run                         # Start the HTTP server (binds host:port from CityLakeConfig)
cargo test                        # All non-ignored tests (unit + e2e)
cargo test --lib                  # Same, library-scope only
cargo test --lib -- --ignored     # Network-backed integration tests in src/tests/integration
```

The `duckdb` crate is pinned to `=1.10501.0` (DuckDB v1.5.1). The cityjson
community extension is only published for v1.5.0/v1.5.1 — bumping past that
breaks `LOAD cityjson`. Bump in lockstep with the extension release matrix.

Tests use `DuckLakeService::new_for_testing()` (see `src/core/db/service.rs:69`), which skips
extension auto-install to avoid network flakiness — production code path uses
`Connection::open_in_memory()` followed by `INSTALL cityjson FROM community; LOAD cityjson; INSTALL ducklake; LOAD ducklake;`.

## API Endpoints

The `:base_name` path parameter is the *base* of LOD-derived table names; CRUD endpoints
that target a specific LOD must use the full table name (e.g. `buildings_lod_2_2`).

```
POST   /tables/:base_name              — Create LOD-suffixed table(s); body { source_path, lod?, base_name? }
POST   /tables/:base_name/upload       — Multipart upload variant; ?lod=&base_name=
POST   /tables/:base_name/objects      — Insert into existing LOD table(s); body { source_path, lod? }
POST   /tables/:base_name/objects/upload — Multipart upload variant; ?lod=
GET    /tables/:table_name/objects     — Query objects (optional ?filter=...)
PUT    /tables/:table_name/objects/:id — Update object by ID (table must have _lod_X_Y suffix)
DELETE /tables/:table_name/objects/:id — Delete object by ID
POST   /tables/:table_name/compact     — Trigger compaction
POST   /tables/:table_name/export      — Export single LOD table to CityJSON format
```
