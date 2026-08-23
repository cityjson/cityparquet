# CityLake Development Milestones

## M1: Project Initialization [DONE]
- [x] Cargo.toml with dependencies (duckdb bundled, axum, tokio, serde, etc.)
- [x] .gitignore for Rust, DuckDB, IDE files
- [x] AGENTS.md with project documentation
- [x] claude.md symlink to AGENTS.md
- [x] Module skeleton with all directories created

## M2: Core Interfaces [DONE]
- [x] `CityLakeConfig` — storage path, catalog path, compaction settings, server config
- [x] `CityLakeRepository` trait — create_table, insert_objects, update_object, delete_object, table_exists, compact_table, get_metadata, export_table, query_objects
- [x] Supporting types: CompactionStats, CityJsonMetadata, ExportFormat, InputFormat, QueryParams
- [x] Request/response types for HTTP API

## M3: DB Service Implementation [DONE]
- [x] `DuckLakeService` — wraps DuckDB connection with cityjson + ducklake extensions
- [x] Table creation via `CREATE TABLE ... AS SELECT * FROM read_cityjsonseq()`
- [x] Insert via `INSERT INTO ... SELECT * FROM read_cityjsonseq()`
- [x] Update via delete + re-insert from temp file
- [x] Delete via `DELETE FROM ... WHERE id = ?`
- [x] Query via `SELECT to_json(t) FROM table t` with optional filters
- [x] Metadata via `cityjson_metadata()` / `cityjsonseq_metadata()`
- [x] Export via `COPY TO` with CityJSON formats
- [x] Compaction via CTAS + DROP + RENAME

## M4: HTTP API Layer [DONE]
- [x] Axum router with all routes
- [x] Handlers: table (create + upload), insert (path + upload), update, delete, query, export, compaction
- [x] Middleware: CORS (permissive), tracing
- [x] Health check endpoint
- [x] Server startup with configurable host/port

## M5: Verification [DONE]
- [x] `cargo check` passes with no errors
- [x] Clean architecture maintained (handlers → repository trait → DB implementation)

## Future Work
- [ ] Integration tests with sample CityJSON files
- [ ] Environment variable / TOML config loading
- [ ] Authentication / API keys
- [ ] DuckLake file count query for compaction stats
- [ ] Pagination metadata in query responses
- [ ] OpenAPI / Swagger documentation
- [ ] Docker deployment
