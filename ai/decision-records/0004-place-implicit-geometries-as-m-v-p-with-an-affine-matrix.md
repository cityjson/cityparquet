---
title: Place implicit geometries as M·v + p with an affine matrix
date: 2026-10-09
status: accepted
summary: >-
  An implicit geometry's relative vertex v is placed at M·v + p, with the
  transformation matrix's translation applied and its last row required to be
  0 0 0 1; a projective matrix is invalid input.
tags: [geometry, implicit-geometry]
related: []
supersedes: []
superseded_by: null
---

## What changed

The specification stored a relative geometry, a reference point and a 4×4
transformation matrix for each implicit geometry, but never said how they combine.
It now states the placement: the matrix acts on the relative geometry in its own
frame, and the result is moved to the reference point. The matrix is a full affine
matrix, so its own translation column applies on top of the reference point, and
its last row must be `0, 0, 0, 1`. An absent matrix is the identity.

The DuckDB 3D extension's `ST_3DPlaceImplicit` already implemented this; the
writers now refuse a source instance with a projective matrix, and the conformance
checker reports one.

## Why

Without a stated rule, two consumers of the same package could place the same tree
in different places, and nothing would flag it. CityJSON only says the reference
point is where "the transformations are applied from"; reading the matrix in the
template's frame and translating afterwards is how CityGML's `ImplicitGeometry` and
cityjson.org's developer guide describe it.

### Alternatives considered

- **Use only the matrix's rotation and scale, ignoring its translation.** Rejected:
  it silently discards part of a source value that CityJSON allows to be non-zero.
- **Leave the rule unstated** as an open question. Rejected: the format would carry
  data whose meaning depended on the consumer.

## Consequences

Placement is reproducible across consumers, and a singular or projective matrix is
caught at the boundary rather than producing a flattened solid. An object with
several implicit geometries still cannot be represented; that remains open.
