# CLAUDE.md

Orientation for the **CityParquet MCP server** — the package under `ai/mcp/`
that gives an agent the specification, the two DuckDB extensions' function
references, and a sandboxed DuckDB engine to describe and query CityParquet
datasets. User-facing setup and the tool table are in `README.md`; this file
is for whoever next touches the code, and states the things a plausible change
would break.

## The exact pins, and why

`package.json` pins `@duckdb/node-api` at `1.5.5-r.5` (DuckDB v1.5.5) and
`@modelcontextprotocol/server` at `2.0.0`, both **exact**, no caret. A
community extension is built against one DuckDB version, and the community
repository only carries builds for the versions it was built for: `cityjson`
and `three_d` both exist at v1.5.5, and at v1.5.4 they exist only as older
builds that lack most of what `FUNCTIONS.md` documents — every
`cityparquet_*` pragma, `ST_3DFootprintArea`, `ST_3DTransform`, the
`(BLOB, STRUCT)` overload of `ST_3DFromWKB`. A caret range would let
`pnpm install` bring up a DuckDB for which one of them has no build, and the
server would fail at `LOAD` time with no code change to explain why. Before
moving this pin, check that **both** extensions answer at the target version
(`https://community-extensions.duckdb.org/<version>/linux_amd64/<name>.duckdb_extension.gz`)
and that `test/duckdb.test.ts`'s blocked table still passes.

## `spatial` is not loaded by default

`DEFAULT_EXTENSIONS` in `src/duckdb.ts` is `httpfs`, `cityjson`, `three_d`.
`spatial` is left out because nothing CityParquet needs requires it —
`three_d` measures solids and footprints (`ST_3DFootprintArea`) and
reprojects (`ST_3DTransform`) — not because it cannot load: since the v1.5.5
builds it loads alongside `three_d` in either order. (At v1.5.4 it could not:
`spatial` first broke `three_d` with "Cannot AlterEntry without client
context", and the reverse broke `spatial`. Older notes, including the design
spec's body, describe that.) An operator may add it with
`CITYPARQUET_MCP_EXTENSIONS`. It brings GDAL, a second file reader; the
"spatial opted in" suite in `test/duckdb.test.ts` checks, with a positive
control, that the sandbox blocks GDAL's local reads too. Anything that makes
`spatial` a default must keep that suite green.

## The startup sequence in `src/duckdb.ts` is load-bearing

`createEngine`'s steps are numbered in the source for a reason — reordering
any of them reopens a hole or breaks initialisation outright. The secrets
setting and the lockdown steps below (1 and 4, and the disabling of
autoinstall/autoload/community extensions and the local filesystem that sits
between them) run only when `sandbox: true` — the local stdio entry point
defaults to `sandbox: false`, so a local client's engine neither restricts
`allow_persistent_secrets` nor locks its configuration:

1. `allow_persistent_secrets = false` must be set **before any extension
   loads**. `cityjson` touches the secret manager on `LOAD` (it can read URLs
   itself), and once the secret manager has been used DuckDB **refuses**
   further changes to secret-manager settings with `Invalid Input Error:
   Changing Secret Manager settings after the secret manager is used is not
   allowed!` — a loud error, not a silent no-op. Set this one first, or the
   sandboxed engine fails to start.
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

## Check what the loaded build provides

At v1.5.5 the published builds and the submodules' `FUNCTIONS.md` agree —
the pinned submodule commits document exactly the community refs. That was
not true at v1.5.4, and it will stop being true whenever a submodule moves
ahead of its published build. When writing or changing a tool that calls
into `cityjson` or `three_d`, confirm the function exists in the loaded
build: `SELECT function_name FROM duckdb_functions() WHERE function_name
ILIKE '…'`. Use `ILIKE`, not `=` or `IN`: `spatial` registers mixed-case
names such as `ST_Area`, and an exact lowercase match misses them. The
corpus indexes `FUNCTIONS.md` whatever the build holds, so an agent can read
about a function it cannot call.

## `describe()` reads local files through Node, outside DuckDB's sandbox

`cityparquet_describe` accepts a local package directory, a local `.parquet`
file or a `file://` URL as well as an `http(s)` URL. For a local package it
reads `metadata.json` with Node's `fs`, not with DuckDB — and DuckDB's
`disabled_filesystems` does not govern Node. So `describe()` checks
`engine.sandbox` itself and refuses every local path on a sandboxed engine
**before** any `readFile`. Without that check the hosted server would read
`<any directory>/metadata.json` for anyone who asked, which is exactly the
local-file read the sandbox exists to prevent. Any new tool that touches the
disk from Node carries the same obligation; `test/describe.test.ts` has the
case that pins it.

## `describe()`'s notes name the actual failure

"Unreachable" means a request failed. A missing `metadata.json`, an HTTP
error status, a body that is not JSON, an asset `href` that does not resolve
and a listed asset that is not readable Parquet each get their own note. The
package `crs` is reported only when every table that states one agrees;
otherwise it is null and each table's own `crs` stands. A malformed `city`
footer falls back to `geo` rather than failing the whole CRS read. Tables are
named after their file, not their STAC asset key — Items produced by this
stack list `building.parquet` under a generic `data` key as well.

One DuckDB behaviour matters to fixtures: its Parquet reader **refuses** a
file whose `geo` footer lacks `version` ("Geoparquet metadata does not have a
version") before `describe()` sees it. A test fixture with a `geo` key must be
well-formed GeoParquet metadata.

## The `cityparquet_` tool prefix is provisional

All five tool names start with `cityparquet_` (see `src/server.ts`). This is a
phase-1 choice, not a commitment: a later merge with a sibling CityJSON MCP
server may adopt one neutral prefix shared by both servers. Do not let other
code, tests, or documentation depend on this specific prefix persisting.
