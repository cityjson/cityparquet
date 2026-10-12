---
title: Escape reserved-name collisions with a leading underscore
date: 2026-10-09
status: accepted
summary: >-
  A source attribute whose name collides with a reserved column, compared
  case-insensitively, is stored as `_<name>` (one more `_` if it already starts with
  `_`) and restored by removing one `_`, replacing diversion into `other`.
tags: [schema, naming, interop]
related: [0002-write-json-typed-columns-as-the-parquet-json-logical-type.md]
supersedes: []
superseded_by: null
---

## What changed

Before, an attribute whose name was reserved (`children`, `bbox`, `other`) was
invalid input or diverted into the `other` column, and names were compared exactly.
A real Helsinki dataset carries an `ID` attribute next to the reserved `id`: the
Rust writer kept it as a separate column, DuckDB — whose column names ignore case —
renamed it on read, and a round trip through both tools failed.

Now names compare case-insensitively against every reserved name, per-LoD forms
included. A colliding attribute keeps a typed column of its own under an escaped
name: `ID` becomes `_ID`, and a source attribute already called `_ID` becomes
`__ID`, so the mapping is reversible. A reader strips one leading `_` from any
attribute column whose name, without its leading underscores, is reserved.

## Why

The collision has to be resolved the same way by every writer and reversed by every
reader, or the two implementations drift apart on real data. Case-insensitive
comparison is forced by common readers that cannot hold two columns differing only
in case. Escaping keeps the attribute typed and queryable, where `other` holds an
untyped JSON blob.

### Alternatives considered

- **Divert collisions into `other`** (the previous rule). Rejected: the value loses
  its column type, and both writers were already handling it differently.
- **Escape only names that differ in case alone**, leaving exact collisions as
  invalid input. Rejected by the author in favour of one rule for every collision.

## Consequences

`other` now carries only source members the format does not map, and a source
attribute named `other` is no longer invalid input. A column whose own name collides
with a reserved name cannot come from a conforming writer and is rejected. The
escape is visible to SQL users, who query `_ID`, not `ID`.
