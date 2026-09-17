# duckdb-3d — three kernel primitives the energy use-case evidenced (issue drafts)

**Date:** 2026-09-03 · **Status:** drafts, not yet filed against `HideBa/duckdb-3d` ·
**Evidence base:** `usecase/energy/` (merged), its README, and the paper repo's
use-case candidates note.

These are ready-to-file issue texts. Each names the primitive, the consumers that
motivated it, and the contract the prototype already validated.

---

## Issue 1 — `ST_3DFaces`: per-face decomposition with semantics

**Title:** `ST_3DFaces` table function: per-face semantics, normal, tilt, azimuth, area, centroid

**Body:**

Every domain workflow we have built or scoped needs the same table: the faces of a
solid with their CityJSON semantic labels and per-face geometry. The energy tool's
envelope split (wall/roof/ground areas for UBEM U-value assignment), the PV
roof+façade inventory, CNOSSOS-EU façade receiver generation, and exact directional
frontal-area indices all reduce to filters and aggregations over this one table.

A Python prototype exists in the monorepo at
`usecase/energy/src/energy/faces.py` and is validated against 3DBAG's own
per-building reference attributes at **< 0.02% median relative error over 800
buildings** (12,793 faces). Its output schema is the proposed contract:

```
ST_3DFaces(solid SOLID_3D, geometry_properties STRUCT)
  → TABLE(
      face_idx    INTEGER,   -- storage order; what face_semantics indexes
      semantic    VARCHAR,   -- resolved via face_semantics[i] → surfaces[j].type;
                             -- 'Unknown' on missing/out-of-range/wrong-shaped input
      nx, ny, nz  DOUBLE,    -- unit normal (Newell's method over the exterior ring)
      tilt_deg    DOUBLE,    -- angle normal↔+Z: 0 flat-up, 90 vertical, 180 flat-down
      azimuth_deg DOUBLE,    -- compass from north, clockwise; NULL for horizontal faces
      area_m2     DOUBLE,    -- exterior ring area minus interior rings, clamped ≥ 0
      cx, cy, cz  DOUBLE     -- area-weighted centroid (fan triangulation, signed
                             -- areas against the unit normal; holes subtracted;
                             -- vertex-mean fallback for degenerate faces)
    )
```

Semantics resolution must degrade to `'Unknown'`, never raise, on malformed
`surfaces` JSON (non-list, non-dict entries) — the prototype's tests encode this.
When this lands, the energy tool's `faces.py` reduces to a SQL query with no
consumer-visible change; its pytest suite doubles as an acceptance suite.

---

## Issue 2 — solid/half-space clipping (volume below a plane)

**Title:** `ST_3DVolumeBelow(solid, z)` (or `ST_3DClipZ → SOLID_3D`): clip a solid at a horizontal plane

**Body:**

Flood-damage indicators need the volume of a building solid below a scenario water
level — irreducibly 3D (roof shape, overhangs and sloped ground change the answer
versus footprint × depth), and established in the literature (Elfouly & Labetski
2020, doi:10.1080/19475705.2020.1777213; Schröter et al. 2018,
doi:10.1016/j.envsoft.2018.03.032). `ST_3DVolume` today measures whole solids only.

Minimal useful form: `ST_3DVolumeBelow(solid SOLID_3D, z DOUBLE) → DOUBLE`.
More general (and reusable for storey slicing): `ST_3DClipZ(solid, z) → SOLID_3D`,
composing with the existing `ST_3DVolume`. The half-space case avoids general 3D
booleans; open solids should degrade the same way `ST_3DVolume` does.

---

## Issue 3 — segment–solid intersection predicate (line of sight)

**Title:** `ST_3DIntersectsSegment(solid, p1, p2)`: does a segment hit the solid?

**Body:**

Exact line-of-sight tests in SQL would upgrade three scoped workflows from
"candidate-set screening" to full answers: mmWave base-station link feasibility,
statutory sunlight-duration checks (solar rights), and occluder confirmation for
shadow/SVF engines. Today `ST_3DDWithin`/`ST_3DIntersects` bound candidate sets, but
the per-link verdict must leave SQL.

Proposed: `ST_3DIntersectsSegment(solid SOLID_3D, x1 DOUBLE, y1 DOUBLE, z1 DOUBLE,
x2 DOUBLE, y2 DOUBLE, z2 DOUBLE) → BOOLEAN` (a `GEOM_3D` linestring overload is the
friendlier long-term shape). This is the smallest step toward ray casting that the
candidates note flags as the genuinely hard national-scale research step — a boolean
predicate is enough for every screening use above.
