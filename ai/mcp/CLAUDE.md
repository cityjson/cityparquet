# CLAUDE.md

Orientation for the **CityParquet MCP server** — the package under `ai/mcp/`
that gives an agent the specification, the two DuckDB extensions' function
references, and a sandboxed DuckDB engine to describe and query CityParquet
datasets. User-facing setup and the tool table are in `README.md`; this file
is for whoever next touches the code, and states the things a plausible change
would break.

## The exact pins, and why

`package.json` pins `@duckdb/node-api` at `1.5.4-r.1` (DuckDB v1.5.4) and
`@modelcontextprotocol/server` at `2.0.0`, both **exact**, no caret. v1.5.4 is
the newest DuckDB version for which *both* `cityjson` and `three_d` exist in
the community extension repository — `three_d` is absent at v1.5.5. A caret
range on `@duckdb/node-api` would let `pnpm install` silently bring up a newer
DuckDB that has no `three_d` build, and the server would fail at `LOAD` time
with no code change to explain why. Do not loosen this pin without first
checking the community repository for the target DuckDB version.

## `spatial` is never loaded, and cannot be

`spatial` and `three_d` cannot both be loaded into one DuckDB connection, in
either order: `spatial` first breaks `three_d` with "Cannot AlterEntry without
client context"; `three_d` first breaks `spatial` with "Scalar Function with
name …". This is a defect in `three_d` / DuckDB's extension loading, not
something fixable here — see `lib/duckdb-3d`'s own repository for whether it
has been reported. `DEFAULT_EXTENSIONS` in `src/duckdb.ts` loads `three_d`
(CityParquet's 3D solid geometry needs it) and so never loads `spatial`. The
consequence for tool authors and for `cityparquet_query` callers: `ST_Area`
and `ST_GeomFromWKB` are unavailable, and there is no 2D PostGIS-style
vocabulary at all. `ST_3DFootprintArea` and `ST_3DTryFromWKB` are the
substitutes — see the table in `README.md`.

## The startup sequence in `src/duckdb.ts` is load-bearing

`createEngine`'s steps are numbered in the source for a reason — reordering
any of them reopens a hole or breaks initialisation outright:

1. `allow_persistent_secrets = false` must be set **before any extension
   loads**. `cityjson` touches the secret manager on `LOAD` (it can read URLs
   itself), and DuckDB permanently locks secret-manager settings once the
   secret manager has been used — set this after loading `cityjson` and it no
   longer takes effect, silently.
2. `INSTALL`/`LOAD` of the wanted extensions must run **before**
   `disabled_filesystems` is set, because installing and loading touch the
   local filesystem.
3. The `duckdb_extensions()` query that verifies what actually loaded must
   also run **before** the filesystem is disabled — it too reads the
   extension directory on disk, not just DuckDB's in-memory state.
4. `lock_configuration = true` comes **last**, after every other sandbox
   setting, since it is what stops all of them being reverted by a later
   query.

## A caution for auditors: `duckdb_settings()` lies about `disabled_filesystems`

`SELECT * FROM duckdb_settings() WHERE name = 'disabled_filesystems'` reports
an **empty** value even while the sandbox is fully enforced — this is a
DuckDB reporting quirk, not evidence the setting did not take. Do not conclude
the sandbox is off from the settings table. The only reliable evidence is
behavioural: run a query that would touch the local filesystem
(`read_csv('/etc/hostname')`, `ATTACH '/tmp/x.db'`, …) and confirm it fails.
`test/duckdb.test.ts` does exactly this.

## The negative tests in `test/duckdb.test.ts` are a security contract

The `blocked` table in that file — local CSV/Parquet reads, `ATTACH`, `COPY
… TO`, extension install, and attempts to unlock the filesystem, the memory
limit or the configuration itself — is not a set of ordinary unit tests. A
change that makes any one of them **pass** (i.e. the blocked operation now
succeeds) is a regression in the sandbox, not a test to relax. Treat a red
test here as a stop-and-investigate signal before treating it as a test bug.

The one test in that block that verifies something *succeeds* —
`LOAD json` — is correct as written, not an oversight: `json` is statically
linked into the DuckDB binary, so loading it reads no file. The sandbox's
actual property is "only extensions already compiled into the binary can be
loaded"; every extension that would need a disk read is blocked.

## `corpus/corpus.json` is generated and committed, never hand-edited

It is built by `src/build-corpus.ts` from `documents/docs/03-specification`,
`documents/docs/04-design-decisions`, and both extensions'
`docs/FUNCTIONS.md`. Regenerate it with `just mcp-corpus` from the repository
root (needs `lib/duckdb-cityjson` and `lib/duckdb-3d` checked out — `just
setup`) and commit the result; do not edit `corpus/corpus.json` by hand. It is
committed, not built at request time or at server startup, because the two
extensions live in submodules a plain clone does not have, and because a
container image built from this package alone must still serve all three
corpora.

The freshness gate (`just mcp-check`, `pnpm corpus:check`,
`src/check-corpus-fresh.ts`) compares the committed file's `corpora` against a
freshly built one **excluding** the `generatedFrom` field. `generatedFrom` is
a `git describe --always --dirty` stamp that changes on every commit
regardless of whether the corpus content changed, so comparing the file
byte-for-byte (or with `git diff` after regenerating) would fail on every
commit forever and could never distinguish a genuinely stale corpus from the
ordinary case. The check builds its comparison in memory and never writes to
`corpus/corpus.json`, so it leaves the working tree exactly as found whether
it passes or fails — nobody has to remember not to commit a churned stamp.

## The published community extension builds lag their own documentation

`lib/duckdb-cityjson/docs/FUNCTIONS.md` documents `cityjson_geoparquet_geo`
and `cityparquet_city_field`. Neither function exists in the extension build
currently published to the DuckDB community repository — the submodule's docs
describe work that has not shipped yet. When writing or changing a tool that
calls into `cityjson` or `three_d`, verify what the loaded build actually
provides with `SELECT * FROM duckdb_functions() WHERE function_name = '…'`
rather than trusting the submodule's `FUNCTIONS.md`. The corpus still indexes
that file's prose for `cityparquet_docs_search` and `cityparquet_docs_read` —
an agent can legitimately read about a function it cannot yet call — but tool
*implementations* must not assume it is callable.

## The `cityparquet_` tool prefix is provisional

All five tool names start with `cityparquet_` (see `src/server.ts`). This is a
phase-1 choice, not a commitment: a later merge with a sibling CityJSON MCP
server may adopt one neutral prefix shared by both servers. Do not let other
code, tests, or documentation depend on this specific prefix persisting.
