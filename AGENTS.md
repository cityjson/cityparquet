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

Reference implementation: https://github.com/HideBa/citylake
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

## Key SQL Patterns

```sql
-- Create table from CityJSON source
CREATE TABLE citylake.{table} AS SELECT * FROM read_cityjsonseq('{path}');

-- Insert from file
INSERT INTO citylake.{table} SELECT * FROM read_cityjsonseq('{path}');

-- Export
COPY (SELECT * FROM citylake.{table}) TO '{path}' (FORMAT cityjsonseq);

-- Metadata
SELECT * FROM cityjsonseq_metadata('{path}');
```

## Dev Commands

```bash
cargo build                       # Library + binary (default features include `server`)
cargo build --no-default-features # Library only (no axum/tower deps)
cargo run                         # Start the HTTP server (binds host:port from CityLakeConfig)
cargo test                        # All tests; integration tests live in src/tests/e2e
cargo test --lib                  # Unit tests only (faster; skips e2e)
```

Tests use `DuckLakeService::new_for_testing()` (see `src/core/db/service.rs:69`), which skips
extension auto-install to avoid network flakiness — production code path uses
`Connection::open_in_memory()` followed by `INSTALL cityjson FROM community; LOAD cityjson; INSTALL ducklake; LOAD ducklake;`.

## API Endpoints

```
POST   /tables/:name           — Create table from CityJSON source
POST   /tables/:name/objects   — Insert objects (file upload or server path)
GET    /tables/:name/objects   — Query objects (optional ?filter=...)
PUT    /tables/:name/objects/:id    — Update object by ID
DELETE /tables/:name/objects/:id    — Delete object by ID
POST   /tables/:name/compact   — Trigger compaction
POST   /tables/:name/export    — Export table to CityJSON format
```
