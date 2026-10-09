---
title: A conformance checker in the reference implementation, and a cross-writer suite
date: 2026-10-08
status: accepted
summary: >-
  `cityparquet validate` in the Rust library checks any package against the
  specification at the Parquet logical-type level, and a monorepo suite runs both
  writers' packages through it and through a round-trip compare.
tags: [conformance, testing, interop]
related: [0002-write-json-typed-columns-as-the-parquet-json-logical-type.md]
supersedes: []
superseded_by: null
---

## What changed

There was no test that the two writers — the Rust reference implementation and the
DuckDB extension — produced conforming packages; agreement rested on one manual
round trip. The Rust library now has a validator, exposed as `cityparquet
validate`, that checks layout, column logical types, footer metadata, the STAC
Item and the value-level invariants, each check citing the MUST or SHOULD it comes
from. The monorepo's `just conformance` writes real fixtures with both writers,
has DuckDB read and rewrite the Rust package, and checks every resulting package
with the validator and with an export-and-compare against the source.

## Why

Two writers that read each other's files can still agree on a wrong encoding. A
checker derived from the specification, rather than from either writer, is the only
oracle that catches that; the round-trip compare catches the complementary failure
of a conforming file with the wrong content. The validator lives in the reference
implementation because that is where the format's types are already defined, so
it reuses them instead of keeping a second copy.

### Alternatives considered

- **A cross-read check only** (each writer's package must open in the other tool),
  the minimum the paper needed. Rejected as the whole answer: it cannot detect a
  deviation both tools share.

## Consequences

The validator must stay neutral about how a writer lays a file out — column order,
encodings, row groups and legacy LIST layouts are all accepted — or it would
encode one writer's habits as the format. Every specification change now has a
second place to land. DuckDB cannot declare columns REQUIRED, so its packages carry
nullability warnings by design.
