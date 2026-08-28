# CLAUDE.md

Repo-wide orientation for the **CityParquet** monorepo — the specification, its
implementations, and the benchmarks the design is argued from. Each sub-area has
its own `CLAUDE.md` with detailed, authoritative instructions — **defer to those
when working inside them.**

## What this repo is

A **cloud-native delivery stack for 3D city models.** It takes CityJSON /
CityJSONSeq / CityGML and makes them storable, discoverable and queryable at
national-to-global scale over cloud object storage, via a columnar Parquet
encoding (**CityParquet**) and a DuckDB/DuckLake lakehouse (**CityLake**).

The **normative specification** lives in `documents/docs/03-specification/`,
with the reasoning in `04-design-decisions/` and the genuinely unsettled parts in
`05-open-questions/`. It is the authoritative _what_; the implementations under
`lib/` are catching up to it.

## Layout

| Path                   | What                                                                                                           | Authoritative instructions                  |
| ---------------------- | -------------------------------------------------------------------------------------------------------------- | ------------------------------------------- |
| `documents/`           | Blume docs site — the **normative CityParquet specification**, design decisions, open questions                | `documents/blume.config.ts`; skill: `blume` |
| `lib/cityparquet-rs/`  | **Rust reference implementation** — the reader/writer that _owns_ the encoding; the `cityparquet` CLI          | its `CLAUDE.md`                             |
| `lib/citylake/`        | Rust data-lake framework + web API; the lakehouse runtime. **Work in progress.** Its own Cargo workspace       | its `CLAUDE.md`                             |
| `lib/duckdb-cityjson/` | DuckDB CityJSON extension — SQL-native CityJSON I/O and an executable prototype of the encoding. **Submodule** | its `CLAUDE.md`                             |
| `lib/duckdb-3d/`       | DuckDB 3D extension — 3D solid processing (`SOLID_3D`). Strict TDD. **Submodule**                              | its `CLAUDE.md`                             |
| `benchmark/`           | Three benchmark families: `formats/` (cross-format), `databases/` (vs cjdb / 3DCityDB v5), `plot/` (renderers) | `benchmark/README.md`                       |
| `test/`                | `TESTING.md`, the cross-module manual walkthrough, and `run-all.sh`                                            | —                                           |
| `ai/design-notes/`     | Dated, unmaintained plans and specs — the record of decisions, not a description of the code                   | `ai/design-notes/README.md`                 |
| `ai/mcp/`              | The **MCP server** — the specification, the function references, dataset description and sandboxed SQL, for agents | its `CLAUDE.md`                             |
| `example/`             | Small inputs; anything worth measuring is fetch-scripted                                                       | —                                           |

## How the pieces fit together

- **CityParquet** is a **directory of Parquet files split by CityGML module**
  (`building.parquet`, `transportation.parquet`, …), one row per city object,
  with **WKB geometry in a per-LoD `geometry_lod*` column** paired with a
  `geometry_properties_lod*` struct carrying the CityGML CM information WKB
  cannot hold (semantic surfaces, shell structure), plus optional `materials` /
  `textures` / `geometry_templates` sidecars. Footer metadata is a `city` object
  alongside a GeoParquet-conformant `geo` object; `metadata.json` is a STAC Item
  (city3d extension). At LoD0 a footprint _is_ GeoParquet; solids step beyond
  GeoParquet's WKB vocabulary and are declared only in `city`.
- **`lib/duckdb-cityjson`** is a second, SQL-native executable prototype of the
  same encoding, and CityLake's only CityJSON I/O path. (`cityparquet-rs` does
  carry its own Rust CityJSON/CityGML reader/writer — that rule is scoped to
  CityLake.)
- **`lib/duckdb-3d`** operates on the WKB 3D geometry CityParquet carries.
- The **City3D STAC extension** (separate `city3d-stac-tool` repo, vendored under
  `lib/cityparquet-rs/vendor/`) is the discovery layer in front of the stack.
- Recurring design principle: **separation of geometry from appearance**
  (material/texture), following OBJ/COLLADA/glTF precedent.

## Three Cargo workspaces, and why

`lib/cityparquet-rs` is the library. `benchmark/readbench` is the read
benchmark's harness — **its own workspace**, living with the corpora, results,
scripts and renderers it belongs to. `lib/citylake` is a third. Consequences:

- `cd lib/cityparquet-rs && just check` gates the **library alone** and is
  self-contained: no `uv`, no `jq`, no corpus, no local extension build. That
  is the point of the split. The root `just check` runs all three workspaces
  — the library's own gate, the two `benchmark/readbench` harness suites and
  `citylake-check` — plus the MCP server's gate.
- `benchmark/readbench` path-depends on `../../lib/cityparquet-rs/crates/core`
  and **must repeat the `[patch.crates-io] cjseq` line** — `[patch]` is honoured
  only in the workspace root being built, and without it the benchmark would
  silently resolve the unpatched upstream.
- Its `fcb_core`/`cjseq2` pins are **exact** (`=0.7.6`, `=0.1.0`). They are a
  measured format's reader; a caret range would let a later release change what
  the published figures mean.
- Recipes that reach both the library and the benchmark — `bench`,
  `convert-all`, `write-bench`, `compression-bench`, the fetchers, the
  renderers, `plot-test`, `scripts-test`, `catalog-*` — are in the **root
  `justfile`** and run from the repository root.
- The four per-dataset recipes live in ONE file because
  `benchmark/readbench/tests/strip_extension.rs` extracts all four and runs them
  to prove the input-extension convention has not drifted. Do not split them.

## Crate directories vs package names

Directories under `crates/` are short — `core`, `schema`, `cli` — because the
enclosing directory already says `cityparquet-rs`. The **package** names stay
namespaced (`cityparquet`, `cityparquet-schema`, `cityparquet-cli`) because
those are global on crates.io, where `core`, `schema` and `cli` are taken or
reserved. Do not "tidy" the package names to match the directories.

## Build and gates

```sh
just setup                          # every submodule, recursively (~1.2 GB)
just setup-shallow                  # the same at --depth 1
just hooks                          # rustfmt + Prettier on staged files, one-off

cd lib/cityparquet-rs && just check  # the Rust gate
just plot-test                       # benchmark plotting suite   (needs uv)
just scripts-test                    # benchmark shell suites     (needs jq)
just mcp-check                       # the MCP server's gate      (needs pnpm)
just citylake-check                  # CityLake's gate            (needs a local duckdb-cityjson build)
just check                           # all five, from the root
just docs-build                      # the specification site     (needs pnpm)
```

Benchmarks are **not** a gate and are not in CI — multi-hour, corpus-dependent,
and the database family needs rootless podman.

**`just check` needs the submodules checked out.** `mcp-check` runs
`corpus:check`, which rebuilds the MCP server's documentation corpus from
`lib/duckdb-cityjson/docs/FUNCTIONS.md` and `lib/duckdb-3d/docs/FUNCTIONS.md`
to compare it against the committed one — so `just check` (and `just
mcp-check` and `just mcp-corpus` on their own) fail with an ENOENT deep inside
the corpus build on a fresh clone that has not run `just setup` or `just
setup-shallow` first.

**`citylake-check` needs a local `duckdb-cityjson` build.** CityLake's
integration tests exercise the `cityparquet_*` package pragmas, which the
published community extension does not carry — so `just citylake-check` looks
for `lib/duckdb-cityjson/build/release/extension/cityjson/cityjson.duckdb_extension`
and points `CITYLAKE_CITYJSON_EXTENSION` at it when found. Run `just -f
lib/duckdb-cityjson/justfile build` first, or export
`CITYLAKE_CITYJSON_EXTENSION` yourself to point at a build elsewhere.

## Submodules

`lib/duckdb-cityjson` and `lib/duckdb-3d` are independent git repositories with
their own `CLAUDE.md`, tests and CI. They **can** be edited from here — consult
their own instructions as you do, and commit in their own repos, since this one
only records which commit it pins.

What their independence means is a **design** boundary, not an editing one. They
are libraries in their own right: they must not encode knowledge of _how_
`cityparquet-rs` implements its reader and writer — its internal types, its
module layout, its private invariants. What they may depend on is what the
specification states, because that is the contract all three implement. The
whole stack is meant to be used together; the rule keeps them coupled through
the spec rather than through each other's internals.

`lib/cityparquet-rs` and `lib/citylake` are _not_ submodules: they live in this
tree and are edited here.

`lib/cityparquet-rs/vendor/cjseq` is a **patched vendored copy**, not a
submodule; `lib/cityparquet-rs/vendor/city3d-stac-tool` is a submodule and is
gated by `just vendor-check`.

## Conventions

- Each level keeps an `AGENTS.md` byte-identical to its `CLAUDE.md` — edit one,
  copy it to the other.
- **British English** in prose.
- **Breaking changes are welcome**: there are no users yet. No shims, no
  deprecation paths, no legacy branches — update every call site instead.
- **Document the present, never the past.** No changelog voice in reference
  documentation; history belongs in git.
- Benchmark caveats are part of the artefact. A change that makes a number look
  better by dropping one is a defect.
