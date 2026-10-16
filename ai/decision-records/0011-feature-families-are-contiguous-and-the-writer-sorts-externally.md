---
title: Feature families are contiguous per table, and the writer sorts by spilling runs
date: 2026-10-13
status: accepted
summary: >-
  Rows sharing a feature_id must be contiguous within each object table, with the root
  row first, and object ids are unique across the package. The Rust writer meets this
  with an external Hilbert sort over Arrow IPC runs, and export streams family by
  family instead of holding the package in memory.
tags: [ordering, memory, export, specification]
related: [0008-hilbert-order-in-the-duckdb-writer-computed-in-c-with-the-re]
supersedes: []
superseded_by: null
---

## What changed

The specification gains three rules on object tables:

- rows sharing a `feature_id` **MUST** be contiguous within each object table;
- the root row (`id == feature_id`) **SHOULD** be the first row of its run;
- `id` **MUST** be unique across the package's object tables.

The validator checks all three (`value.feature-contiguity`, `value.feature-root-order`
as a warning, `value.duplicate-id`). Hilbert order keys a feature family as a whole, so
ordering never splits a family.

The reference writer no longer re-reads or holds features in Hilbert order. It encodes
in source order, tags each row with `(hilbert, row ordinal)`, and sorts with an external
merge sort: sorted runs spill to compressed Arrow IPC stream files under the package's
temporary directory once a memory budget is reached, and a k-way merge writes the
tables. Export reads each table front to back and rebuilds a family as soon as its rows
are complete.

## Why

Converting and exporting national datasets on a 16 GB machine. Before this, Hilbert
order held every CityGML or FlatCityBuf feature in memory, and export rebuilt the whole
package in memory before writing a line. Contiguity is the guarantee a streaming reader
needs: without it, any family could still have rows at the end of the table.

The rule is per table, not one table per family, because a family can legitimately span
module tables: a CityObjectGroup's members take the group's `feature_id` but live in
their own module's table, and so do extension children. Requiring one table per family
would contradict the by-module layout. Only those spanning families are buffered on
export.

Duplicate object ids are refused because a writer that accepts them can emit two runs
of one family and break its own contiguity rule.

### Alternatives considered

- **One table per feature family.** Rejected: it moves grouped buildings out of
  `building.parquet`, breaking the by-module layout readers prune on.
- **Spill to temporary Parquet and sort keys only.** Rejected: random access back into
  Parquet by row is slow, and the IPC stream format writes and reads sequentially with
  dictionaries per batch, which `object_type` needs.
- **Keep the in-memory sort and document a memory ceiling.** Rejected: the ceiling is
  the dataset, not a batch.
- **Uncompressed runs, or a switch between codecs.** Not taken: LZ4 frame compression
  is cheap enough to leave on, and a switch would be a flag nobody needs to set.

## Consequences

- Peak memory for convert is the budget (default 2 GiB), plus, while merging, one run
  batch per run (at most 1,024 rows or about 8 MiB), plus two batches in flight to the
  Parquet writer and one open row group per table, plus one 128-bit hash per object
  for the duplicate-id check. A `.city.json` input is held whole, since it is one
  document. Temporary disk is roughly one compressed copy of the encoded rows. The
  merge is single-pass, so a very large number of runs costs memory; a multi-pass
  merge is not implemented.
- Export holds the families parked across tables and one 128-bit hash per feature
  written, to refuse late rows.
- An object reached through several parents (a building in two `CityObjectGroup`s) is
  emitted by the feature split in every parent's feature, so its id repeats and the
  conversion is refused. Writing it once, under its first parent's root, as the
  `feature_id` rule asks, is not implemented.
- Packages differ from earlier ones only in appearance id values, which the interner
  now assigns in source order; ids are opaque.
- Export to a CityJSON **document** still collects every feature, because a document
  has one vertex pool. Export to CityJSONSeq streams.
- The DuckDB writer orders by the same key, but whether ties between features with
  equal keys can interleave their rows is unverified. Until it is, its packages may fail
  `value.feature-contiguity`.
