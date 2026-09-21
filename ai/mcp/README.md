# CityParquet MCP server

An [MCP](https://modelcontextprotocol.io) server that gives an agent the
CityParquet specification, the two DuckDB extensions' function references, and
a sandboxed DuckDB engine to describe and query CityParquet datasets — over
stdio for a local MCP client, or over streamable HTTP as a hosted service.

## Tools

| Tool | What it does |
| --- | --- |
| `cityparquet_docs_outline` | Lists the chapters of one or all three documentation corpora: `spec` (the normative specification and its design decisions), `duckdb-cityjson` and `duckdb-3d` (the DuckDB extension function references). Call this first to see what can be read. |
| `cityparquet_docs_search` | Searches the documentation for a term and returns matching sections with snippets — faster than reading whole chapters when looking for a specific column, function or rule. |
| `cityparquet_docs_read` | Reads one chapter, or one section of a chapter. Takes chapter ids from `cityparquet_docs_outline` or `cityparquet_docs_search`. |
| `cityparquet_describe` | Describes a CityParquet dataset from a package directory or a single `.parquet` file — an `http(s)` URL, or a local path or `file://` URL when the engine is not sandboxed: its module tables, row counts, LoDs, geometry columns and CRS, per table and for the package. Call this before querying an unfamiliar dataset. |
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

## Running as an HTTP server

`dist/http.js` serves the same five tools over streamable HTTP at `/mcp`,
statelessly, with a health check at `/health`. It is the hosted entry point,
so it is **always sandboxed**, and every request that runs SQL gets its own
freshly built engine, discarded afterwards. Its network is an allowlist: every
engine's `http_proxy` is locked to a proxy in the same process that admits
HTTPS to `CITYPARQUET_MCP_EGRESS_HOSTS` and nothing else, and statements that
mention `SECRET` are refused, because a DuckDB secret can override the proxy.

```sh
pnpm build
PORT=8080 node dist/http.js
claude mcp add --transport http cityparquet http://localhost:8080/mcp
```

The container image is built from this directory alone, with the extensions
baked in:

```sh
docker build --platform linux/amd64 -t cityparquet-mcp .
docker run -p 8080:8080 cityparquet-mcp
```

`scripts/smoke.mjs <url>` checks a running server from outside: the tools
answer, an allowlisted host is readable, and another host, the metadata
address, a secret and the local filesystem are all refused.

## Deploying to Cloud Run

`.github/workflows/mcp-deploy.yml` builds the image, pushes it to Artifact
Registry, deploys it to Cloud Run in `europe-west4`, and smoke-tests the
result, sending traffic back to the previous revision if that fails. It
authenticates like `cityjson/flatcitybuf`'s deploy, through Workload Identity
Federation. One-off setup, in the same GCP project:

```sh
PROJECT=<project id>
# The image repository.
gcloud artifacts repositories create cityparquet \
  --project "$PROJECT" --location europe-west4 --repository-format docker
# The account the service runs as. It gets no roles: the metadata server
# hands its token to anything in the container, so it must open nothing.
gcloud iam service-accounts create cityparquet-mcp-runtime \
  --project "$PROJECT" --display-name "cityparquet MCP runtime (no roles)"
```

Then let the deploy reach them:

- Admit `cityjson/cityparquet` in the WIF provider's attribute condition
  (it probably admits only `cityjson/flatcitybuf` today).
- Grant the repository's WIF principal `roles/artifactregistry.writer` on the
  `cityparquet` repository, `roles/run.admin` on the project, and
  `roles/iam.serviceAccountUser` on `cityparquet-mcp-runtime`.
- Add the secrets `WIF_PROVIDER` and `PROJECT_ID` to `cityjson/cityparquet`,
  with the same values as on `cityjson/flatcitybuf`.

## Environment variables

| Variable | Default | Meaning |
| --- | --- | --- |
| `CITYPARQUET_MCP_SANDBOX` | off (`sandbox: false`) | Set to `1` to lock the DuckDB engine down: no local filesystem, no installing further extensions, resource limits that cannot be raised again. Off by default for the stdio entry point, because a local client's own machine is already the trust boundary; a hosted deployment should set it. |
| `CITYPARQUET_MCP_EXTENSION_DIR` | `~/.cityparquet-mcp/extensions` | Where DuckDB installs and loads its extensions from. Always explicit, never DuckDB's own default — a shared default directory can hold artefacts built for a different DuckDB version, and the failure is an opaque error at `LOAD` time. |
| `CITYPARQUET_MCP_EXTENSIONS` | `httpfs,cityjson,three_d,spatial` | Comma-separated list overriding the default extension set. Blank entries are ignored, and an empty value means the default. |
| `CITYPARQUET_MCP_MEMORY_LIMIT` | DuckDB's own default; `1GB` over HTTP | DuckDB's `memory_limit` setting, e.g. `2GB`, per engine. Worth raising under the sandbox, since a sandboxed engine cannot spill a large query to disk. |

The HTTP entry point also reads these; `CITYPARQUET_MCP_SANDBOX` does not
apply to it, since it is always sandboxed:

| Variable | Default | Meaning |
| --- | --- | --- |
| `PORT` | `8080` | The port to listen on. |
| `CITYPARQUET_MCP_POOL_SIZE` | `2` | Engines at once, and so requests running SQL at once. |
| `CITYPARQUET_MCP_MAX_WAITING` | `8` | Requests allowed to queue for an engine; beyond it the answer is 503. |
| `CITYPARQUET_MCP_MAX_WAIT_MS` | `30000` | How long a queued request waits before a 503. |
| `CITYPARQUET_MCP_THREADS` | `1` | DuckDB `threads` per engine. |
| `CITYPARQUET_MCP_MAX_ROWS` | `1000` | The most rows one `cityparquet_query` statement may return, whatever it asks for. |
| `CITYPARQUET_MCP_MAX_TIMEOUT_MS` | `60000` | The longest deadline a statement may ask for. |
| `CITYPARQUET_MCP_MAX_BODY_BYTES` | `262144` | Larger request bodies are refused with 413. |
| `CITYPARQUET_MCP_EGRESS_HOSTS` | the three `open3d.city` data hosts | Comma-separated hostnames a query may read, over HTTPS only. |

## `spatial` and `three_d` together

The default set loads both. `spatial` gives DuckDB's 2D vocabulary —
`ST_Area`, `ST_Transform`, `ST_AsText` — for a package's LoD0 column, which
arrives as DuckDB's `GEOMETRY` type because it is GeoParquet. It cannot read
the solids: its WKB reader rejects `PolyhedralSurface Z`, so LoD1 and above
go through `three_d`. (At DuckDB v1.5.4 the two could not be loaded into one
connection, which is why older notes say `spatial` is unavailable.)

| Task | LoD0 footprint (`GEOMETRY`) | Solid (`BLOB`) |
| --- | --- | --- |
| Area | `ST_Area(geometry_lod0_0)` | `ST_3DFootprintArea(ST_3DTryFromWKB(geometry_lod2_2, geometry_properties_lod2_2))` |
| Reproject | `ST_Transform(geometry_lod0_0, 'EPSG:7415', 'EPSG:4326', always_xy := true)` | `ST_3DTransform(solid, 'EPSG:7415', 'EPSG:4326')` |
| Volume | — | `ST_3DVolume(solid)`, gated on validity |

Without `always_xy := true`, `spatial`'s `ST_Transform` returns EPSG:4326 in
the authority's (lat, lon) order; `ST_3DTransform` always returns (lon, lat).
