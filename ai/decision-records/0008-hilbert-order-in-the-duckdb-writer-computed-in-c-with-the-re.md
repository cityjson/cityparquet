---
title: Hilbert order in the DuckDB writer, computed in C++ with the reference key
date: 2026-10-08
status: accepted
summary: >-
  The DuckDB extension writes object tables in Hilbert order computed in C++ with
  the Rust writer's exact key, so both writers order a dataset the same way without
  loading the spatial extension.
tags: [ordering, duckdb, interop]
related: []
supersedes: []
superseded_by: null
---

## What changed

The Rust writer orders rows along a 2D Hilbert curve by default; the DuckDB writer
wrote rows in table order. It now sorts by the same key: each feature's centroid,
quantised to 16 bits per axis over the dataset extent, mapped onto the curve. The
key is also available as a SQL function, and `ordering => 'source'` keeps table
order.

## Why

Ordering is optional in the specification, but spatial row-group pruning — the
reason for it — only works if rows that are close in space share row groups. Using
the reference key makes the two writers' packages comparable row for row, which the
conformance suite and benchmarks rely on.

### Alternatives considered

- **Use the spatial extension's Hilbert function.** Rejected: the package write runs
  on a fresh connection that does not inherit loaded extensions, so it would have
  to load spatial there, and the key would not match the reference writer's.

## Consequences

The keys agree exactly whenever every source vertex is one the package keeps; the
Rust writer keys on the raw vertex pool, so a feature with unused source vertices
can sort differently. Ordering is not normative, so this is a documented limit of
compatibility, not a conformance failure.
