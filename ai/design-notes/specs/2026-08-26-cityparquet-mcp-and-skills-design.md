# An MCP server and skills for CityParquet

**Date:** 2026-08-26
**Status:** design, awaiting review

A Model Context Protocol server that lets an AI agent read the CityParquet
specification, look up the DuckDB extension functions, inspect a dataset and run
SQL against it — plus a small set of skills that turn those tools into
workflows. It ships in two transports: **stdio**, run locally beside a user's own
data, and **streamable HTTP**, deployed as a container behind Cloudflare and open
to the public.

Its purpose is the same as the playground's, one level up. The playground makes
the specification's argument executable for a human with a browser; this makes it
executable for an agent, and gives the agent the specification and the function
reference to reason from rather than guess at.

Every version, URL and DuckDB setting named below was checked while writing this
note, not assumed. §12 records what was checked and what was not.

## 1. Scope

**In scope, and specified here:**

- A TypeScript MCP server under `ai/mcp/`, with five tools and a mirrored set of
  resources.
- A build step that converts `documents/docs/` and the two extensions'
  `docs/FUNCTIONS.md` into the corpus the server serves.
- Four skills under `ai/plugin/skills/`, distributed both as a Claude Code
  plugin and as an APM package.
- The sandbox the public HTTP deployment needs, and the container that runs it.

**Explicit non-goals.** Each of these was considered and left out:

- **Catalog search.** There is no catalog yet — `documents/docs/06-resources/01-datasets.mdx`
  says "coming soon". A `cityparquet_search_catalog` tool and a matching skill
  are the obvious phase-4 addition; nothing here forecloses them.
- **A `convert` tool.** Conversion is `COPY … TO`, which needs a writable
  filesystem: available on stdio, disabled on the hosted server. A tool that
  works on one transport and not the other is worse than a skill that emits the
  right SQL, so conversion is skill-only (§9.3).
- **A validator tool.** Validating a package against the specification is
  worth having, but nothing in the stack implements it yet. Adding a tool over a
  validator that does not exist would be designing the wrong half first.
- **A general DuckDB documentation corpus.** Models already know DuckDB. What
  they do not know is `read_cityjsonseq` and `ST_3DVolume`, and those are what
  the corpora in §4 carry.
- **Authentication, and write access on the hosted server.** §7 makes the public
  deployment read-only by construction rather than by permission.
- **CityLake.** Out of scope entirely; it is a separate runtime.

## 2. Decomposition and phasing

The request is four projects, and they are built in this order:

| Phase | What | Gate |
| --- | --- | --- |
| 1 | Corpus build + stdio server + five tools + tests | `just mcp-check` green; corpus staleness gate in CI |
| 2 | Four skills, the plugin manifest, the APM package | Skills exercised against the phase-1 server |
| 3 | HTTP transport, sandbox, Dockerfile, Cloudflare deploy, CI/CD | The negative sandbox tests of §7.4 pass |
| 4 | Catalog tool and skill — **deferred**, not designed here | — |

Phases 1 and 2 co-design each other: the skills are what tell us whether the
tool surface is the right one, so the skills are drafted alongside phase 1 even
though they ship after it.

## 3. Where it lives

```
ai/
  mcp/                          the server — its own pnpm package
    package.json
    src/
      corpus.ts                 load corpus.json; outline, search, read
      duckdb.ts                 engine bring-up, pinning, the sandbox
      sql.ts                    statement splitting, value elision
      tools/
        docs.ts                 the three documentation tools
        describe.ts             cityparquet_describe
        query.ts                cityparquet_query
      server.ts                 tool and resource registration; transport-agnostic
      stdio.ts                  the stdio entry point
      http.ts                   the streamable-HTTP entry point
    scripts/
      build-corpus.ts           documents/docs + FUNCTIONS.md -> corpus.json
    corpus/
      corpus.json               generated, committed
    Dockerfile
    CLAUDE.md  AGENTS.md        byte-identical, per the repository convention
  plugin/                       one authored copy, two distributions
    .claude-plugin/plugin.json
    .mcp.json
    skills/
      cityparquet/SKILL.md
      cityparquet-query/SKILL.md
      cityparquet-write/SKILL.md
      cityparquet-3d-analysis/SKILL.md
  design-notes/                 unchanged
  apm.yml                       unchanged; the plugin is published from ai/plugin
```

`ai/` becomes the directory of things that serve agents rather than humans —
design notes, the server, the skills. That is a small widening of what `ai/`
already means, and it keeps the server beside the two extensions it wraps
without adding a fifth top-level directory.

**One authored copy of the skills.** `ai/plugin/` is simultaneously a valid
Claude Code plugin root and the virtual path an APM package resolves to, so both
distributions read the same `skills/` directory. Nothing is copied, and the two
cannot drift.

The root `CLAUDE.md` layout table gains rows for `ai/mcp/` and `ai/plugin/`, and
`AGENTS.md` is updated byte-identically.

## 4. The corpus

### 4.1 Why it is built, not read

The server serves documentation from a **single generated `corpus.json`**, built
from the repository and committed.

The alternative — reading `documents/docs/` and the two `FUNCTIONS.md` at request
time — fails on both deployments for the same reason. `lib/duckdb-cityjson` and
`lib/duckdb-3d` are **submodules**: in a clone without `just setup` they are
empty directories, and in the container image they are not present at all. A
server that reads them at runtime serves two of its three corpora only when the
repository happens to be fully checked out.

Fetching from the published documentation site was rejected for the opposite
reason: it puts a network round trip and an uptime dependency into the local
stdio server, which is precisely where a user expects fast and offline.

The committed artefact is the same trade the paper repository already makes with
`paper/assets/bench/` — commit the generated thing so the consumer builds
without the producer.

### 4.2 The three corpora

| id | Source | What it is |
| --- | --- | --- |
| `spec` | `documents/docs/03-specification/`, `documents/docs/04-design-decisions/` | The normative specification, and the reasoning behind it |
| `duckdb-cityjson` | `lib/duckdb-cityjson/docs/FUNCTIONS.md` | The CityJSON/CityParquet function reference |
| `duckdb-3d` | `lib/duckdb-3d/docs/FUNCTIONS.md` | The `SOLID_3D` / `GEOM_3D` function reference |

Roughly 200 KB of MDX plus 41 KB and change of function reference — small enough
that search can be naive (§5.1), far too large to hand over whole.

`05-open-questions/` is deliberately excluded: it describes what is *unsettled*,
and an agent quoting it as though it were normative is a worse failure than an
agent that does not know it exists. `07-tutorials/` is excluded because the
skills serve that role better.

### 4.3 Shape

Both kinds of source normalise to the same two levels — **chapter**, then
**section** — so one pair of tools serves them all.

```jsonc
{
  "generated_from": "<git describe --always --dirty>",
  "corpora": {
    "spec": {
      "title": "CityParquet specification",
      "chapters": [
        {
          "id": "object-table-schema",
          "title": "Object table schema",
          "description": "Reserved structural columns, typed attribute columns, and addresses.",
          "order": 2,
          "sections": [ { "heading": "Reserved columns", "level": 2 } ],
          "body": "…markdown…"
        }
      ]
    }
  }
}
```

- **Chapter ids** strip the numeric prefix and the extension:
  `02-object-table-schema.mdx` → `object-table-schema`. This matches the id style
  the CityJSON MCP already uses (`city-objects`, `geometry-objects`), which
  matters for §10.
- For a `FUNCTIONS.md` corpus, each top-level `##` heading is a **chapter**
  (`reading`, `metadata`, `writing`, `cityparquet-packages`, `scalar-helpers`,
  …) and each `###` beneath it a section. Reading the whole 41 KB file is never
  useful; reading its `cityparquet-packages` chapter frequently is.
- **`title` and `description`** come from the MDX frontmatter, which every page
  already has, and become the outline's one-line summaries.

### 4.4 The build

`ai/mcp/scripts/build-corpus.ts`, driven by a root `just mcp-corpus` recipe:

1. **Order from the filename prefix**, and *assert it agrees with `meta.ts`*.
   Each specification directory has a `meta.ts` declaring a `pages` array in
   sidebar order, and the numeric prefixes currently agree with it exactly.
   Parsing `meta.ts` as TypeScript would mean evaluating a `defineMeta` call
   from `blume`; reading the prefixes and checking them against the string list
   in `meta.ts` gets the same ordering with no import, and turns a future
   disagreement into a build failure rather than a silently wrong sidebar order.
2. **Reduce MDX to Markdown.** Strip frontmatter into the fields above; drop
   imports and JSX; unwrap Blume admonitions (`:::note[Title] … :::`) into a
   bolded line plus its body, since their content is often normative.
3. **Rewrite site-relative links** (`/specification/extensions`) to absolute URLs
   on the published site, so a link an agent surfaces to a user is one they can
   actually follow.
4. **Emit `corpus.json`** with the source commit stamped in.

**Staleness is a CI gate**, not a convention: CI runs `just mcp-corpus` and fails
if the working tree is dirty afterwards. The build needs `just setup` for the two
submodules; the server needs neither.

## 5. Tools

Five. Every tool is prefixed `cityparquet_` — see §10 for what happens to that
prefix at merge time.

### 5.1 The documentation trio, and why a corpus enum

`cityparquet_docs_outline(corpus?)`
`cityparquet_docs_search(query, corpus?, limit?)`
`cityparquet_docs_read(corpus, chapter, section?)`

Three tools over a `corpus` enum, rather than a separate outline/read pair per
corpus. Two reasons, and the second is the load-bearing one:

- A tool schema sits in the prompt on every turn whether it is called or not.
  Three tools with an enum cost materially less than the six that the per-corpus
  shape would need once the function references are included.
- **It is the better merge story.** Merging with the CityJSON MCP (§10) means
  adding the CityJSON specification as a fourth `corpus` value — a data change,
  not an API change. The per-corpus shape would make it a fourth pair of
  near-identical tools.

Discovery is self-serving: `docs_outline` with no argument lists every corpus and
every chapter in it, so the model learns the enum from a result rather than from
a description.

`docs_search` is not in the original tool sketch and earns its place. An outline
gives headings; most real questions ("how are semantic surfaces stored?", "which
function reports a solid's height?") are answered in a table cell three levels
below one. Implementation is a **case-folded scan over sections at request
time** — 250 KB total, no index, no dependency. It returns corpus, chapter id,
heading, and a surrounding snippet, so its result is directly actionable as a
`docs_read` call.

`docs_read` takes an optional `section` so a caller can read one `###` of a
long chapter rather than the whole thing.

### 5.2 `cityparquet_describe(url)`

The highest-value tool after `query`, and the one the original sketch was
missing. Every workflow opens by asking what is in a dataset — which module
tables, which LoDs, what CRS, how many objects, what extent — and answering that
by hand is four or five statements plus prior knowledge that
`cityjson_metadata`, `cityparquet_city_field` and the Parquet footer keys exist.
For a user on the hosted server with no skills installed, it is the only sane
entry point.

**It is built on footer and STAC reads, never on `PRAGMA cityparquet_read`.**
The package loader takes a directory path and the function reference documents
no remote form; a describe built on it would work on stdio and fail on the
hosted server, which is the deployment that needs it most. Footer reads and an
HTTP fetch of `metadata.json` behave identically local and remote.

Behaviour:

- **A `.parquet` URL** → the Parquet footer's `city` and `geo` objects, the
  column schema, the row count from footer metadata, and the `geometry_lod*`
  columns present.
- **A directory URL** → fetch `<url>/metadata.json`. The STAC Item's `assets`
  map is the package's file inventory, and its `city3d:*` fields carry LoDs,
  source object types, city-object count, and whether semantics, materials and
  textures are present — most of a description, for one request.
- **The fallback matters.** The specification says each `.parquet` file
  **SHOULD** be a STAC asset, not MUST, and a package may have no reachable
  `metadata.json` at all. When `assets` is absent, probe the eleven fixed module
  basenames and the three sidecars with HEAD requests. The basenames are
  normative, so this is a designed fallback rather than a guess — but the result
  says which path it took, because "no STAC Item" is itself worth reporting.

Where the STAC Item and a footer disagree, describe reports the **footer** and
flags the disagreement: the specification makes the footer authoritative for
decoding and the Item a discovery mirror.

### 5.3 `cityparquet_query(sql, max_rows?, max_cell_bytes?, timeout_ms?)`

Three requirements that are not obvious and that a later discovery would be
expensive:

**It splits statements and runs them one at a time.** DuckDB expands *every*
pragma in a submitted script before running *any* of it, so the package workflow
the function reference documents — `CREATE SCHEMA delft;` followed by
`PRAGMA cityparquet_init('delft');` — fails outright when batched, with an error
about a catalog that does not exist yet. The tool therefore accepts a script,
splits it, and executes the statements sequentially, returning one result per
statement and stopping at the first error.

The splitter must be SQL-aware, not `split(";")`. The mutation examples in the
same reference contain `PRAGMA cityparquet_delete('delft', 'object_type = ''Building''')`
— quoted strings, doubled quotes inside them, dollar quoting and both comment
forms all have to be handled.

**It elides large cell values.** `SELECT *` on a building table returns LoD2
solid WKB, and one row can be kilobytes of binary. Any `BLOB` comes back as
`<BLOB 4213 bytes>`; any other value over `max_cell_bytes` (default 256) is
truncated with its true length stated. This generalises past WKB deliberately —
the `other` overflow column and long attribute strings are context bombs by the
same mechanism. Without this, the tool is at its most destructive on its most
obvious use.

**Truncation is stated, never silent.** A capped result carries `row_count`,
`truncated: true`, and the elapsed time, so a large answer degrades into a fact
the model can act on.

Result shape, per statement: `{ statement, columns: [{name, type}], rows,
row_count, truncated, elapsed_ms }`, or `{ statement, error }`.

### 5.4 Resources

The corpus chapters are mirrored as MCP resources (`cityparquet://spec/object-table-schema`
and so on). It is perhaps twenty lines over the same `corpus.json`, and it lets
clients that support resource attachment pull a chapter directly into context.

**The tools are the contract.** Resource support across MCP clients is uneven,
so nothing in the skills or the tool descriptions may depend on a resource being
reachable.

## 6. The engine

### 6.1 Pinning, and the version that chose itself

`@duckdb/node-api` is pinned **exactly** to `1.5.4-r.1`, which carries DuckDB
**v1.5.4**.

Not a caret range, and the reason is sharper than house style. Both extensions'
`MainDistributionPipeline.yml` builds against `duckdb_version: v1.5.4`. Probing
the community repository directly:

| Extension | v1.5.4 | v1.5.5 |
| --- | --- | --- |
| `cityjson` | 200 | 200 |
| `three_d` | 200 | **404** |

**v1.5.4 is the newest version where both extensions exist** — older lines
were not probed. A caret range would
resolve to 1.5.5 and the server would come up with `three_d` missing — losing
every 3D function, and doing it at runtime rather than at install time. The pin
is what makes that a lockfile fact instead of a deployment surprise. It moves
when `three_d` is published for a later version, and not before.

This matches the playground's exact pin of `@duckdb/duckdb-wasm` and
`benchmark/readbench`'s `=0.7.6`, for the same underlying reason each time: an
extension is built against one DuckDB version.

### 6.2 Extension sourcing

Community builds by default. But the community builds **lag this repository's
sources considerably** — `lib/duckdb-3d`'s README states that the published
`cityjson` build "is older and emits a different column shape", and the
playground README says the same at more length. Anything newly added to either
extension is invisible to a server using the published builds.

The server therefore takes the same escape hatch the playground has: an
environment variable naming a DuckDB extension repository
(`<url>/<duckdb-version>/<platform>/<name>.duckdb_extension`), which implies
`allow_unsigned_extensions` because a locally built artefact carries no DuckDB
Labs signature. Default off; **never available on the public deployment**, where
§7 locks the configuration before a request is ever served.

### 6.3 Startup, in this order

The user's requirement is that DuckDB is ready when the server wakes, not when a
request arrives. The ordering below is not stylistic — steps 2 and 5 conflict,
and reversing them bricks the server.

1. Open an in-memory database.
2. `INSTALL` and `LOAD`: `httpfs`, `cityjson`, `three_d` — and **not**
   `spatial`, for the reason in §6.4. **This touches the local filesystem** —
   the extension directory is on disk — so it must complete before step 5. Set
   `extension_directory` explicitly rather than inheriting `~/.duckdb`, which
   may hold artefacts built for another DuckDB version; sharing it with a local
   CLI install produced exactly that failure while this was being tested.
3. Resource limits: `SET memory_limit`, `SET threads`.
4. Close the doors: `SET autoinstall_known_extensions = false`,
   `autoload_known_extensions = false`, `allow_community_extensions = false`,
   `allow_persistent_secrets = false`.
5. `SET disabled_filesystems = 'LocalFileSystem'` — hosted only.
6. `SET lock_configuration = true` — hosted only. After this, no statement can
   re-open anything closed above.
7. Warm the HTTP path with a trivial remote read, so the first real request does
   not pay for TLS and httpfs initialisation.

In the container, steps 2's downloads are avoided entirely by baking the
extensions into the image (§8).

Every setting named here was confirmed present in `duckdb_settings()`.

### 6.4 `spatial` and `three_d` cannot both be loaded

`lib/duckdb-3d`'s README states that `three_d` "coexists with `spatial`" and
that the two "load together in one session, in any order". **At the published
community builds on DuckDB v1.5.4, they do not.** Loading both fails whichever
order is used:

| Order | Result |
| --- | --- |
| `spatial` then `three_d` | `three_d` fails — *Cannot AlterEntry without client context* |
| `three_d` then `spatial` | `spatial` fails — *Scalar Function with name …* |
| `cityjson` + either one | fine |
| `spatial` then `cityjson` then `three_d` | `three_d` fails, as above |

The server therefore loads **`httpfs`, `cityjson`, `three_d`**. That matches
what the playground already does — its `EXTENSIONS` list is `["cityjson",
"three_d"]` and has never included `spatial` — so this is existing practice made
explicit rather than a new restriction.

The cost is real and must be stated in the skills: `ST_Area`,
`ST_GeomFromWKB` and the rest of the 2D vocabulary are unavailable. The
`cityjson` README's own footprint example (`LOAD spatial; SELECT
ST_Area(ST_GeomFromWKB(geometry_lod0_0))`) cannot run in this server as written;
`ST_3DFootprintArea` is the substitute. A `CITYPARQUET_MCP_EXTENSIONS`
environment variable lets an operator swap `three_d` for `spatial` at startup,
since the two are mutually exclusive anyway.

**This is a defect in a sibling library, not in this server.** It belongs in
`lib/duckdb-3d`'s own repository — as a fix if the collision is resolvable, or
as a correction to its README if it is not. Nothing here should work around it
beyond choosing which extension to load.

## 7. The sandbox

The hosted `query` tool is public and unauthenticated, exactly as the playground
is. The difference is that the playground runs in the user's own browser
sandbox, and this runs on our container. Unhardened, a public DuckDB is an
arbitrary-file-read and SSRF primitive: `read_csv('/etc/passwd')`,
`ATTACH '/proc/self/environ'`, `read_parquet('http://169.254.169.254/…')`.

### 7.1 What stops it

- **`disabled_filesystems = 'LocalFileSystem'`** — no local read, no local
  write, no `COPY … TO` a path, no `ATTACH` of a file.
- **`lock_configuration = true`** — the settings above cannot be undone by a
  query, which is what makes the rest of the list hold.
- **Extension loading closed** (§6.3 step 4) — no `INSTALL`, no `LOAD`, so an
  extension with a filesystem of its own cannot be introduced.
- **`memory_limit` and `threads`** set well below the container's ceiling, so
  one query cannot starve the process.
- **A statement timeout, a row cap, and a concurrency semaphore** in the server,
  not in DuckDB.
- **Egress policy at the container**, since `enable_external_access` must stay
  true for httpfs to work at all. Link-local and private ranges should be
  blocked below the tool rather than by a URL check inside it, which a redirect
  would defeat — but what Cloudflare Containers actually offers here is
  unverified (§12), and it is the one control in this section not backed by a
  probe.

### 7.2 The cost of disabling the local filesystem

DuckDB spills to temporary files under memory pressure, and those are local
filesystem writes. With `LocalFileSystem` disabled, a query that would spill
**fails instead**. That is the correct trade for a public demo — a failed query
is a message, an unbounded one is an outage — but it must be a stated limit, and
`memory_limit` should be generous enough that ordinary questions do not reach
it.

Whether `allowed_paths` can readmit a single temp directory without readmitting
the rest is untested, and is an implementation-time question (§12). The fallback
is a read-only container root with `disabled_filesystems` left off, which is
weaker and should only be reached for if the first approach proves unworkable.

### 7.3 Isolation between requests

One DuckDB instance, initialised at startup and shared — that is the user's
requirement and it is right, because extension loading is the expensive part.
But an in-memory catalog is shared across connections, so a `CREATE TABLE` from
one caller is visible to the next.

Each request therefore gets a fresh connection, and within it a fresh attached
in-memory catalog (`ATTACH ':memory:' AS …; USE …;`), detached when the request
completes. Objects a caller creates — including a package loaded as a schema —
live and die with the request. The cost of attach/detach needs measuring against
a plain shared catalog before this is settled.

### 7.4 The negative tests are the contract

These belong in CI, and a change that breaks one is a security regression, not a
test failure:

- `SELECT * FROM read_csv('/etc/passwd')` fails.
- `ATTACH '/tmp/x.db'` fails.
- `COPY (SELECT 1) TO '/tmp/x.parquet'` fails.
- `INSTALL json` fails. **`LOAD json` succeeds, and that is correct** — `json`
  is statically linked into the DuckDB binary, so loading it reads no file. The
  property the sandbox actually provides is that only extensions *already in the
  binary* can be loaded, because every other one needs a filesystem read. Assert
  the `INSTALL` failure; do not assert a `LOAD` failure that will not happen.
- `SET disabled_filesystems = ''` fails.
- `SET memory_limit = '400GB'` fails.
- A remote read of a private-range address fails.
- A query exceeding the timeout returns a timeout, and the server survives it.

## 8. Transports and deployment

**stdio** is the phase-1 entry point: `npx`-runnable, the user's own machine as
the trust boundary, the local filesystem *available* because reading the user's
own data is the point. `--sandbox` opts into §7 through the same code path, so
the hardened configuration is exercised locally rather than only in production.

**Streamable HTTP**, stateless. Each request is independent, so there is no
session state to pin a client to an instance — which is what makes horizontal
scaling and a scale-to-zero container straightforward.

**The container.** DuckDB's Node binding is native code and cannot run in a
Cloudflare Worker, so this is Cloudflare **Containers**: a Worker fronts a
container binding, the container runs the Node HTTP server. The image is
`node:24-slim`, builds the server, and **bakes the extensions into the image's
extension directory** — which removes the download from startup, removes a
network dependency from the health of a cold start, and makes the pinned
versions an image fact rather than a runtime one.

**Cold starts are real and should be stated rather than papered over.**
Initialising DuckDB at wake rather than at request is the right design, but
Cloudflare Containers scale to zero, so the first request after an idle period
pays the wake. `sleepAfter` tuning helps; a cron warm-up ping helps more;
neither makes it disappear. The tool descriptions should not promise latency the
deployment cannot keep.

CI/CD follows the repository's existing workflow style: build, run the phase-1
tests plus the §7.4 negative tests, build and push the image, deploy. It is
phase 3, and no part of phases 1–2 waits on it.

## 9. Skills

Four, not the seven originally sketched — the consolidation is deliberate and
worth arguing rather than assuming.

**"Read spec" and "use DuckDB" are not skills.** A skill whose content is "call
the MCP tool" adds a hop and no judgment; that is what a tool description is
for. What is left of them — when to consult the specification at all, and how to
quote it without inventing normative language — belongs in the router skill.

**"Convert" is write-from-read.** Splitting it from "write" duplicates the same
`COPY … TO` reference in two files that then drift apart.

### 9.1 `cityparquet` — the router

Fires on "what is CityParquet", "what does the spec say about …", and any task
that names the format without naming an operation. Carries: what the format is
in a dozen lines; the column conventions (`geometry_lod*`,
`geometry_properties_lod*`, `other`, `bbox`) that make the rest legible;
**describe before you query**; when to reach for `docs_search` versus when the
answer is already known; and the routing table to the other three.

### 9.2 `cityparquet-query` — reading

The read-side traps, of which one is a silent correctness trap and therefore the
centre of the skill: **`PRAGMA cityparquet_read` recovers each file's Parquet
footer; `read_parquet` discards it, irrecoverably.** Same rows either way — and
then no `city`/`geo` footer, no declared CRS, no CRS check on insert or merge,
and a later write that has to be told the CRS again or writes an explicit null.
A model that reaches for `read_parquet` because it is the familiar function
produces a package that states nothing about its own coordinate system.

Also: remote URLs and httpfs; per-LoD geometry columns and how to find which
exist; why never `SELECT *` on an object table; which pushdown actually works
(projection always; equality on `id` / `feature_id` / `object_type`; R-tree bbox
and attribute indexes on FlatCityBuf); and reading whole-document CityJSON,
CityJSONSeq and FlatCityBuf as tables.

### 9.3 `cityparquet-write` — writing and converting

`COPY … TO (FORMAT cityjson | cityjsonseq | flatcitybuf | parquet)`; the required
columns; building a package from scratch with `cityparquet_init` versus loading
one with `cityparquet_read`; `insert_cityjson` and its total-routing rule;
mutation as ordinary `UPDATE` plus `cityparquet_reconcile`, and why there is
deliberately no `cityparquet_update`; `cityparquet_delete` and its cascade;
footer `city`/`geo` and the STAC `metadata.json`; and round-trip verification as
the way to check the result.

Two traps it must carry: **pragmas must be submitted as separate statements**
(§5.3), and **the CRS must match on insert, with no reprojection ever
performed** — an unknown on either side is refused rather than assumed.

### 9.4 `cityparquet-3d-analysis`

Choosing among the `ST_3D*` functions, and one hard trap that makes the skill
necessary rather than merely useful: **`ST_3DVolume` raises on a solid that is
not closed, manifold and oriented**, and `ST_3DSurfaceArea` raises on degenerate
faces. Real city-model data fails these constantly. Every measurement must
therefore be gated — `ST_3DTryFromWKB` to get a null instead of an error on
construction, and `ST_3DValidationReport(solid).is_valid` in the predicate
before measuring. The reference implementation's own quick-start does exactly
this, and it is the pattern the skill teaches.

Also: passing `geometry_properties_lod*` alongside the WKB so shell and
semantic-surface structure survives; `SOLID_3D` versus `GEOM_3D` and which
functions take which; units following the CRS, so a projected metre CRS gives
m³ and a geographic one gives nonsense; and `ST_3DTransform` for reprojection.

### 9.5 Distribution, and degradation

`ai/plugin/` is both the Claude Code plugin root — `.claude-plugin/plugin.json`
plus a `.mcp.json` that registers the stdio server — and the APM package. One
authored copy, two distributions (§3).

APM already targets `claude` and `codex`, so the skills are **plain Markdown
with no Claude Code-only affordances**. Each skill names the MCP tools it
expects and states its fallback when the server is not connected: the same SQL
through a local `duckdb` CLI, and the specification on the published site.

## 10. The merge with the CityJSON MCP

The existing CityJSON MCP exposes `cityjson_read_spec_outline` and
`cityjson_read_spec_chapter` over an eleven-chapter corpus with ids like
`city-objects` and `geometry-objects`. Two decisions here keep a later merge
cheap:

- **The chapter id convention matches it** (§4.3), so the CityJSON specification
  drops in as a fourth corpus with no id rewriting.
- **The corpus enum is the merge unit** (§5.1). Merging is adding a `cityjson`
  corpus value and pointing the build at that repository's source — a data
  change.

What the merge costs is the tool names: the merged server wants one neutral
prefix, and `cityjson_read_spec_*` would either be retired or kept as aliases.
The `cityparquet_` prefix here is therefore a **phase-1 choice, not a
commitment**, and should be recorded as such wherever it appears.

## 11. Testing

- **Corpus build** — chapter ids and ordering match `meta.ts`; the MDX reduction
  is checked against golden output; CI fails on a stale `corpus.json`.
- **Statement splitter** — its own suite, with the doubled-quote case from the
  function reference as a named test.
- **Value elision** — a blob and an oversized string are elided; a small value
  is not.
- **Tools** — against a small fixture package under `example/`, offline.
- **`describe`** — both paths: a package with a STAC Item that enumerates its
  assets, and one without, where the module basenames must be probed.
- **The sandbox** — §7.4, in CI, treated as a security contract.

## 12. What was verified, and what was not

**Verified while writing this:**

- Both extensions build against `duckdb_version: v1.5.4`
  (`MainDistributionPipeline.yml`, both repositories).
- `@duckdb/node-api@1.5.4-r.1` reports `select version()` = **`v1.5.4`**, run
  directly; the current latest is `1.5.5-r.4`.
- Community repository: `cityjson` present at v1.5.4 and v1.5.5; `three_d`
  present at v1.5.4, **absent at v1.5.5**.
- Every setting §6.3 and §7 rely on exists in `duckdb_settings()`:
  `disabled_filesystems`, `allowed_directories`, `allowed_paths`,
  `enable_external_access`, `allow_unsigned_extensions`,
  `allow_community_extensions`, `allow_persistent_secrets`,
  `autoinstall_known_extensions`, `autoload_known_extensions`,
  `lock_configuration`, `memory_limit`, `threads`.
- The STAC Item's `assets` map is the package file inventory, as a **SHOULD**;
  the `city3d:*` fields carry LoDs, source object types, city-object count, and
  semantics/materials/textures presence.
- Module file basenames are normative, which is what makes the `describe`
  fallback sound.
- `ST_3DVolume` raises rather than returning null on an invalid solid.
- DuckDB expands every pragma in a submitted script before running any of it.
- `httpfs`, `cityjson` and `three_d` all install from the community repository
  and load under `@duckdb/node-api@1.5.4-r.1`; `cityjson` reports build
  `d511bdb`, `three_d` reports `a08f240`.
- **`spatial` and `three_d` cannot both be loaded**, in either order (§6.4).
- The §6.3 startup sequence and the §7.4 negative tests were run end to end:
  every filesystem escape is blocked, `lock_configuration` refuses to be
  undone, and a remote `cityjsonseq_metadata` read still succeeds afterwards.
- `LOAD json` succeeds under the locked configuration because `json` is
  statically linked; `INSTALL json` is blocked (§7.4).
- `@modelcontextprotocol/server@2.0.0` is published and exports `McpServer`,
  `createMcpHandler`, `ResourceTemplate` and
  `WebStandardStreamableHTTPServerTransport`; `@modelcontextprotocol/server/stdio`
  exports `serveStdio` and `StdioServerTransport`. The v2 packages supersede
  `@modelcontextprotocol/sdk` (latest 1.30.0) and are the SDK's blessed entry
  points.

**Not verified, and to be settled at implementation:**

- Whether `PRAGMA cityparquet_read` accepts a remote directory. The function
  reference documents no remote form, and §5.2 is designed so the answer does
  not matter — but it is worth knowing.
- Whether `allowed_paths` can readmit a temporary directory for spilling while
  `disabled_filesystems` blocks everything else (§7.2).
- The cost of per-request `ATTACH`/`DETACH` against a shared catalog (§7.3).
- Cloudflare Containers cold-start time with the extensions baked in, which
  determines whether a cron warm-up is worth its cost.
- Whether Cloudflare Containers can block link-local and private-range egress at
  the network layer (§7.1). Every other control in §7 is backed by a
  `duckdb_settings()` probe; this one is not, and the SSRF posture depends on
  it.

## Addendum, 2026-08-26: what phase-1 implementation settled

This note is kept verbatim above, per `ai/design-notes/README.md` — nothing in
§1–12 is edited. Phase-1 implementation (`ai/mcp/`) is now complete enough that
several items §12 left open are answered, and two of §12's own claims turned
out to overreach. Recorded here rather than silently left to mislead a reader
of the "verified"/"not verified" ledger above.

- **`duckdb_settings()` reports `disabled_filesystems` as empty even while
  enforcement is fully in force.** `SELECT * FROM duckdb_settings() WHERE name
  = 'disabled_filesystems'` returns an empty value on a locked-down engine that
  still refuses `read_csv('/etc/hostname')`, `ATTACH`, and every other
  filesystem escape in `test/duckdb.test.ts`. This is a DuckDB reporting
  quirk, not evidence the sandbox is off — an auditor who trusted the settings
  table alone would wrongly conclude enforcement had not taken. Behavioural
  probes (attempt the operation, confirm it fails) are the only reliable
  evidence; see `ai/mcp/CLAUDE.md`.
- **`allow_persistent_secrets` must be set before any extension loads, which
  corrects §6.3's ordering.** §6.3 groups `allow_persistent_secrets = false`
  into step 4 ("close the doors"), after step 2's `INSTALL`/`LOAD`. In
  practice `LOAD cityjson` itself uses the secret manager (it can read URLs),
  and DuckDB then refuses any further secret-manager setting with `Invalid
  Input Error: Changing Secret Manager settings after the secret manager is
  used is not allowed!` — a sandboxed engine built in the order §6.3 describes
  fails to start. `src/duckdb.ts` sets `allow_persistent_secrets` first, before
  any `INSTALL`/`LOAD`, and only then proceeds through the rest of step 4.
- **Two of §12's "not verified" items are now settled.**
  `connection.interrupt()` leaves the connection usable after a timeout — the
  very property §5.3's timeout design depends on — confirmed by
  `test/query.test.ts`'s "times out without killing the engine" case, which
  runs a second, ordinary query on the same connection immediately after an
  interrupted one and gets a correct answer.
  `@duckdb/node-api@1.5.4-r.1` reports `select version()` as `v1.5.4`, checked
  directly against a running engine in `test/duckdb.test.ts` rather than
  inferred from the package's own version string, which carries the `-r.1`
  Node-binding suffix DuckDB's own `version()` does not.
- **§12's claim that the startup sequence and the negative tests "were run end
  to end" was true only against a warm extension directory.** The cold-start
  path — `INSTALL … FROM community` fetching an extension that is not yet on
  disk — was not exercised when that line was written, and it is exactly where
  the secret-manager ordering defect above lived: it surfaces only on the very
  first `LOAD cityjson` a fresh extension directory ever sees, once secrets
  have never been touched before. A warm directory does not reveal it, because
  by then the secret manager's first use predates the run.
- **§5.2's promise that `describe` "flags the disagreement" between the STAC
  Item and the footer was dropped at plan time and is not implemented.**
  `describe()` (`ai/mcp/src/tools/describe.ts`) treats the footer as
  authoritative for CRS, exactly as designed, but it does not compare that
  value against anything the STAC Item states and so has no disagreement to
  flag. Nothing tracks this as a defect; it is simply narrower than what this
  design document said it would do.
- **§6.3 step 4 ("close the doors") is gated on `sandbox: true` in the
  implementation, not unconditional as this document's numbered sequence
  implies.** Steps 5 and 6 are marked "hosted only" above; step 4 is not, but
  `createEngine` in `src/duckdb.ts` runs all of step 4 — including
  `allow_persistent_secrets`, moved earlier per the previous point — only when
  `sandbox: true`. The local stdio entry point defaults to `sandbox: false`, so
  a local client's engine leaves autoinstall, autoload, community extensions
  and persistent secrets exactly as DuckDB defaults them; the user's own
  machine is the trust boundary there, not the engine's configuration.
- **The published community extension builds lag their own documentation, as
  §6.2 warns, and this was confirmed rather than assumed.**
  `lib/duckdb-cityjson/docs/FUNCTIONS.md` documents `cityjson_geoparquet_geo`
  and `cityparquet_city_field`; neither exists in the build currently published
  to the community repository — `SELECT * FROM duckdb_functions() WHERE
  function_name = '…'` returns nothing for either, against the extensions this
  server actually loads. `describe()` reads the footer directly with
  `parquet_kv_metadata` and `decode()` rather than calling either function, for
  exactly this reason.
