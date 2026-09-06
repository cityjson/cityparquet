# Use-case tools

This directory holds literature-anchored, individually executable command-line
tools that demonstrate the CityParquet + DuckDB stack on real urban-analysis
problems. Each tool argues, end to end, that the stack solves a problem the
literature has already identified as real — it is not a synthetic benchmark.

Every tool is its own `uv` project, one directory per use case:

- **Headless.** No server, no notebook; a CLI entry point that runs in CI.
- **CityParquet in, Parquet out.** The input is a path, glob or `s3://` URL to
  CityParquet files; the output is one or more Parquet tables, ready for a
  downstream tool (a GIS, a plotting script, an energy simulator) to consume.
- **No database load step.** DuckDB reads the CityParquet files in place.

## Current tools

| Directory  | Problem                                                                          |
| ---------- | --------------------------------------------------------------------------------- |
| `energy/`  | UBEM envelope-feature extraction and degree-day retrofit screening for buildings |

## Where the rationale lives

Each tool starts from a candidate written up in the paper repository's
use-case survey — `references/2026-08-28-cityparquet-duckdb-usecase-candidates.md`
— which screens candidate problems against what the kernel can already do
versus what it would need. `energy/` is candidate 4 in that note.

The design spec each tool is built from lives under
`ai/design-notes/specs/` in this repository, one Markdown file per tool,
written and approved before implementation starts.
