---
title: LoD0 synthesis is opt-in in the CLI
date: 2026-10-08
status: accepted
summary: >-
  Converting a source no longer adds an LoD0 footprint unless `--lod0` is passed, so
  a package encodes only what its source carries; synthesis stays available but
  produces geometry indistinguishable from source LoD0.
tags: [lod0, cli, conformance]
related: []
supersedes: []
superseded_by: null
---

## What changed

The `cityparquet` CLI used to synthesise an LoD0 footprint for every object that
had none, with a `--no-lod0` switch to turn it off; the library already defaulted
to off. The CLI now matches the library: `convert` writes only the geometry the
source carries, and `--lod0` asks for synthesis. Drivers that want footprints for
GeoParquet readers — the benchmark's `convert-all`, the 3DBAG and STAC catalogue
converters — pass `--lod0` explicitly.

## Why

The specification states that a package encodes what the source model carries and
nothing else, and defines losslessness as "what was present survives". A default
that adds geometry makes the reference tool's ordinary output a package the
specification does not describe, and makes every round trip unequal by default.
Synthesis remains useful — a GeoParquet reader sees a footprint column — so it is
kept, but as a deliberate request.

### Alternatives considered

- **Keep synthesis on and record it in the footer** (a `cityparquet:lod0_synthesis`
  flag in `city.other`, as an earlier design proposed). Rejected: it adds footer
  vocabulary the specification does not define, and a reader still has to know to
  look for it before it can tell real LoD0 from invented LoD0.
- **Amend the specification to allow synthesis.** Rejected: it would weaken the
  "encodes what the source carries" rule that the round-trip guarantee rests on.

## Consequences

Default conversions round-trip `compare`-equal without flags. A synthesised
footprint still carries no per-row marker, so a package converted with `--lod0`
cannot be told apart from one whose source had LoD0; anyone who needs that
distinction must keep the conversion command. Revisit if the specification gains a
provenance field for derived geometry.
