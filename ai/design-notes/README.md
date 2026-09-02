# Design notes

An archive of the written plans and specs that preceded the larger changes in
this repository. They are kept verbatim, dated, and **not maintained** — each
one is a record of what was decided and why at the time it was written, not a
description of the code as it stands today. Where a note and the code disagree,
the code is right and the note is history.

They are here because the *reasoning* behind a format decision outlives the diff
that implemented it, and because a specification is easier to argue with when
the alternatives that lost are still legible.

For decisions that are still load-bearing, read
[`documents/docs/04-design-decisions/`](../../documents/docs/04-design-decisions/)
instead — that is the maintained, normative half.

| Note | Subject |
|---|---|
| `plans/2026-08-16-cross-stack-test-pass.md` | A full manual test pass across the Rust library and both DuckDB extensions |
| `specs/2026-08-21-other-column-bbox-simplification-design.md` | Collapsing per-member overflow columns into one `other`; `bbox` as subtree ∪ declared extent |
| `plans/2026-08-21-other-column-bbox-simplification.md` | The implementation plan for the above |
| `plans/2026-08-23-cityparquet-monorepo-migration.md` | The plan that produced this repository's layout |
| `specs/2026-08-24-geoparquet-2-conformance-design.md` | Removing the arrow-native encoding and adopting GeoParquet 2.0: the `GEOMETRY` logical type, `GeospatialStatistics`, and why a solid column is left unannotated |
| `specs/2026-08-24-sql-playground-design.md` | The DuckDB-WASM SQL playground at `/playground`: runtime, extension sourcing, and the CORS the data bucket must serve |
| `specs/2026-08-26-cityparquet-mcp-and-skills-design.md` | The MCP server at `ai/mcp/` and the skills at `ai/plugin/`: the generated documentation corpus, the five tools, the sandbox a public `query` needs, and why the stack pins DuckDB v1.5.4 |
| `plans/2026-08-26-cityparquet-mcp-phase-1.md` | Phase 1 of the above: the corpus build, the stdio server and its five tools, task by task |
| `specs/2026-08-27-citylake-cityparquet-rebuild-design.md` | Rebuilding CityLake on the CityParquet package model: a dataset as a DuckLake schema of module tables, the `cityparquet_*` pragmas in place of hand-rolled operations, and the catalog-qualification fix `cityparquet_write` needs |
| `plans/2026-08-28-citylake-cityparquet-rebuild.md` | The implementation plan for the above, task by task: the pinned toolchain, the upstream write fix, the pure SQL module, the package operations, and the CRS footer the extension has to mint |
| `specs/2026-09-02-citylake-real-data-and-ui-e2e-design.md` | Three follow-ups to the CityLake rebuild: network-gated tests against the published Delft datasets, re-pointing the web client at the dataset/module API, and Playwright coverage with a DEV-gated authentication bypass |
| `plans/2026-09-02-citylake-real-data-tests.md` | Piece A of the above: the network-gated suite against the published Delft feed, with the object-type split, CRS, package round trip and a real cascade pinned to measured figures |

Note that these notes were written while the code lived in several separate
repositories, so they refer to paths (and, in one case, a local working
directory and a process id) that no longer exist. The migration note's §4.1 is
the map from the old paths to the current ones.
