---
title: Read FlatCityBuf through fcb_core behind a cargo feature
date: 2026-10-08
status: accepted
summary: >-
  `convert` reads FlatCityBuf through `fcb_core`, pinned exactly to the version the
  benchmark measures and behind a cargo feature, bridging its feature type to ours
  by a per-feature JSON round trip.
tags: [input, dependencies, flatcitybuf]
related: []
supersedes: []
superseded_by: null
---

## What changed

The paper listed FlatCityBuf as an input format, but the reference implementation
had no reader for it. `convert` now recognises `.fcb` files by their magic bytes and
streams their features through the same path as CityJSONSeq. The reader is behind
an `fcb` feature on the library and CLI; `fcb_core` is pinned exactly to the version
the read benchmark already uses, so the tree carries one copy of it.

## Why

`fcb_core` decodes into a different CityJSON feature type than the one the writer
consumes, and it pulls in an HTTP stack the library otherwise does without. Keeping
the reader optional limits that cost to builds that want FlatCityBuf input. The
JSON round trip between the two feature types is the simplest correct bridge.

### Alternatives considered

- **Buffer the decoded features and hand them over as a pre-built source.** Weighed
  in the audit as the cheaper bridge; not taken because streaming keeps memory flat
  for large files.
- **Drop FlatCityBuf from the paper instead.** The author chose to implement it.

## Consequences

The JSON bridge costs time per feature, acceptable for an input format rather than a
hot path. The reader must enforce the header's feature count itself — a zero count
means "read to the end", and a short file is an error — because the iterator does
not end on its own. Revisit the bridge if `fcb_core` adopts the same feature type.
