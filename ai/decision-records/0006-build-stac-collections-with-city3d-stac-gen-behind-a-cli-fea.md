---
title: Build STAC Collections with city3d-stac-gen behind a CLI feature
date: 2026-10-08
status: accepted
summary: >-
  `cityparquet collection` builds a STAC Collection with city3d-stac-gen's builder
  rather than its own code, behind a CLI-only cargo feature so the library never
  inherits gen's heavy dependency tree.
tags: [stac, dependencies, cli]
related: []
supersedes: []
superseded_by: null
---

## What changed

The specification allows a dataset-level STAC Collection over several packages,
but the CLI could not write one. It now can: `cityparquet collection` reads the
packages' Item `metadata.json` files and uses city3d-stac-gen's collection builder
for the extent union and `city3d:*` summaries. The dependency sits on the CLI crate
only, behind a `collection` feature that is on for the binary; the library and
schema crates do not depend on it. The vendored city3d-stac-tool submodule moved to
the same revision as the cargo dependency.

## Why

The author asked that collection logic not be implemented twice: city3d-stac-gen
already aggregates Items into Collections, and a second implementation would drift
from it. Gen, however, brings a large tree — the `stac` crate, an older
`object_store` with cloud backends, an older `fcb_core`, `reqwest` — which a
library meant to be embedded should not carry.

### Alternatives considered

- **Lift a small public function upstream** in city3d-stac-gen. Not needed: the
  builder's public API was enough, so no upstream change was made.
- **Write the Collection in this repository.** Rejected by the author to avoid a
  duplicate implementation.

## Consequences

The CLI build includes two versions of some crates. The builder's own
`item_assets` template is overwritten with the Items' real assets. Gen is a git
dependency, which blocks publishing the CLI to crates.io until gen is published or
the feature is left out of the published crate.
