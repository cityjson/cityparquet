---
title: Write JSON-typed columns as the Parquet JSON logical type
date: 2026-10-08
status: accepted
summary: >-
  Both writers annotate `other`, `geometry_properties.surfaces` and structured
  attributes with the Parquet JSON logical type the specification names, and both
  readers still accept plain UTF8 from older or foreign files.
tags: [parquet, interop, conformance]
related: [0005-a-conformance-checker-in-the-reference-implementation-and-a-.md]
supersedes: []
superseded_by: null
---

## What changed

The specification types the object-table `other` column, the `surfaces` field of
`geometry_properties_lod*`, the sidecars' `other` and object-valued or
heterogeneous attributes as `JSON`. Both writers used to store them as plain UTF8 —
the Rust writer tagged them `arrow.json` only in the Arrow schema, which Parquet
does not see. Both now write the Parquet JSON logical type, and the conformance
checker treats a plain-string column where the specification says `JSON` as an
error. Readers accept either, so packages written before the change still load.

## Why

CityParquet is defined at the level of Parquet logical types. A column the
specification calls `JSON` but which a file declares as a string is not that column
to any generic reader: DuckDB, pyarrow and others decide whether to parse a value
from the logical annotation, not from a writer's side channel. Matching the
annotation is what lets a third-party reader treat the value as JSON without
knowing which tool wrote it.

### Alternatives considered

- **Leave the writers on UTF8 and treat the specification's wording as loose.**
  Rejected by the author: both writers agreeing with each other but not with the
  specification would make the specification wrong for every file in existence.

## Consequences

In DuckDB, a JSON-typed column needs the `json` extension for text functions or an
explicit cast to `VARCHAR`, and the extension registers its own alias for the type
because the name `JSON` belongs to that extension. In-memory reader output
(`read_cityjson`) keeps `surfaces` as `VARCHAR`; the rule governs Parquet files.
