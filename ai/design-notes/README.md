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
| `specs/2026-08-24-sql-playground-design.md` | The DuckDB-WASM SQL playground at `/playground`: runtime, extension sourcing, and the CORS the data bucket must serve |

Note that these notes were written while the code lived in several separate
repositories, so they refer to paths (and, in one case, a local working
directory and a process id) that no longer exist. The migration note's §4.1 is
the map from the old paths to the current ones.
