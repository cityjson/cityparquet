---
title: Triangulate faces with vendored earcut, recursion-capped
date: 2026-10-08
status: accepted
summary: >-
  duckdb-3d triangulates each face whole with a vendored mapbox earcut, holes
  bridged, and rejects a face rather than return a partial triangulation; the
  vendored code is modified to cap its recursion.
tags: [geometry, duckdb-3d, dependencies]
related: []
supersedes: []
superseded_by: null
---

## What changed

The 3D extension's own ear clipping triangulated each ring separately and stopped
silently when it found no ear. On real data this left faces with real area untriangulated,
or only partly triangulated, and holes were handled only because their opposite
winding happened to cancel. Faces are now projected onto their own plane and
triangulated whole by a vendored copy of mapbox's earcut, holes included. A face
whose triangles would not tile it gets none, and validation counts it degenerate.
Non-finite projections are rejected before triangulation.

## Why

Area, volume and validity are computed from the triangulation, so an incomplete one
silently corrupts every measurement downstream. The extension's rule is to fail
clearly rather than repair silently. Earcut is small, widely used, handles holes,
and is header-only, so vendoring it adds no build dependency.

### Alternatives considered

- **Fix the existing ear clipper.** Not pursued: it would still need hole bridging
  and a robustness pass that earcut already has.

## Consequences

Measured volumes moved in the second decimal on non-planar faces, which have no
single correct triangulation. Earcut's recursion is capped in the vendored file,
so deep adversarial input is rejected instead of overflowing the stack; its CPU
time on adversarial rings is still unbounded. Updating earcut means reapplying the
cap, which the vendored copy's README records.
