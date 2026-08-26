# CityParquet MCP server

An [MCP](https://modelcontextprotocol.io) server that gives an agent the
CityParquet specification, the two DuckDB extensions' function references, and
a sandboxed DuckDB engine to describe and query CityParquet datasets — over
stdio, for a local MCP client.

## Tools

| Tool | What it does |
| --- | --- |
| `cityparquet_docs_outline` | Lists the chapters of one or all three documentation corpora: `spec` (the normative specification and its design decisions), `duckdb-cityjson` and `duckdb-3d` (the DuckDB extension function references). Call this first to see what can be read. |
| `cityparquet_docs_search` | Searches the documentation for a term and returns matching sections with snippets — faster than reading whole chapters when looking for a specific column, function or rule. |
| `cityparquet_docs_read` | Reads one chapter, or one section of a chapter. Takes chapter ids from `cityparquet_docs_outline` or `cityparquet_docs_search`. |
| `cityparquet_describe` | Describes a CityParquet dataset from a package directory URL or a single `.parquet` URL: its module tables, row counts, LoDs, geometry columns and CRS. Call this before querying an unfamiliar dataset. |
| `cityparquet_query` | Runs one or more SQL statements against DuckDB with the `cityjson` and `three_d` extensions loaded. Results are capped by row count and cell size; BLOB and oversized values are elided, so `SELECT *` on an object table is a poor idea — select the columns you need. |

The `cityparquet_` prefix is provisional: it may be replaced by one neutral
prefix shared with a sibling CityJSON MCP server, should the two later merge.
Do not depend on it elsewhere.

## Setup

```sh
pnpm install
pnpm corpus   # builds the package, then regenerates corpus/corpus.json — see below
pnpm build
```

`pnpm corpus` needs `documents/docs/` and both `lib/duckdb-cityjson/` and
`lib/duckdb-3d/` checked out (`just setup` from the repository root). A plain
clone without the submodules can skip this step: `corpus/corpus.json` is
committed precisely so the server builds and runs without them.

## Running as a stdio MCP server

Point a client at the built entry point:

```json
{
  "mcpServers": {
    "cityparquet": { "command": "node", "args": ["/absolute/path/to/ai/mcp/dist/stdio.js"] }
  }
}
```

## Environment variables

| Variable | Default | Meaning |
| --- | --- | --- |
| `CITYPARQUET_MCP_SANDBOX` | off (`sandbox: false`) | Set to `1` to lock the DuckDB engine down: no local filesystem, no installing further extensions, resource limits that cannot be raised again. Off by default for the stdio entry point, because a local client's own machine is already the trust boundary; a hosted deployment should set it. |
| `CITYPARQUET_MCP_EXTENSION_DIR` | `~/.cityparquet-mcp/extensions` | Where DuckDB installs and loads its extensions from. Always explicit, never DuckDB's own default — a shared default directory can hold artefacts built for a different DuckDB version, and the failure is an opaque error at `LOAD` time. |
| `CITYPARQUET_MCP_EXTENSIONS` | `httpfs,cityjson,three_d` | Comma-separated list overriding the default extension set. `spatial` cannot be added here alongside `three_d` — see below. |
| `CITYPARQUET_MCP_MEMORY_LIMIT` | DuckDB's own default | DuckDB's `memory_limit` setting, e.g. `2GB`. Worth raising under `CITYPARQUET_MCP_SANDBOX=1`, since a sandboxed engine cannot spill a large query to disk. |

## Why `spatial` is unavailable

`spatial` and `three_d` cannot both be loaded in the same DuckDB connection, in
either order: loading `spatial` first breaks `three_d` with "Cannot AlterEntry
without client context"; loading `three_d` first breaks `spatial` with "Scalar
Function with name …". This server always loads `three_d` — CityParquet's 3D
solid geometry needs it — so `spatial` is never loaded, and the ordinary 2D
PostGIS-style vocabulary (`ST_Area`, `ST_GeomFromWKB`, …) is unavailable.
`three_d` provides the substitutes:

| Instead of | Use |
| --- | --- |
| `ST_Area(ST_GeomFromWKB(geometry_lod0_0))` | `ST_3DFootprintArea(ST_3DTryFromWKB(geometry_lod0_0))` |
| `ST_GeomFromWKB(...)` | `ST_3DTryFromWKB(wkb, geometry_properties)` |
