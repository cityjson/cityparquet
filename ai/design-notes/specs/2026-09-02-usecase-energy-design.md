# Use-case tool: building-energy feature factory — design

**Date:** 2026-09-02 · **Status:** approved design, pre-implementation ·
**Home:** `usecase/energy/` (first module of the new `usecase/` directory) ·
**Origin:** candidate 4 of the paper repo's
`references/2026-08-28-cityparquet-duckdb-usecase-candidates.md`.

## Purpose

Demonstrate that the CityParquet + DuckDB stack solves a real, literature-anchored
problem end to end: producing the geometric inputs that urban building energy models
(UBEM) and retrofit programmes need, straight from a CityParquet package on disk or
object storage, with no database load step. Each `usecase/` module is an individual
CLI tool, runnable headless (CI-friendly), taking a CityParquet path plus per-use-case
parameters.

The physics stays external. This tool is the *data-preparation and screening* stage —
which the literature identifies as the actual bottleneck — not an energy simulator.

## Non-goals

- No energy-balance simulation (SimStadt, CitySim, EnergyPlus remain the consumers).
- No party-wall adjacency detection in v1 (see Limitations).
- No new `duckdb-3d` kernel work; the face module deliberately *prototypes*
  `ST_3DFaces` in Python instead (see The `ST_3DFaces` contract).

## Methodology from the literature

The tool README documents this first, before any implementation detail — the
methodology is the argument, the tool is its execution.

1. **UBEM envelope inputs.** Bottom-up urban energy platforms driven by CityGML-class
   models — SimStadt (Nouvel et al. 2015), the Energy ADE (Agugiaro et al. 2018,
   doi:10.1186/s40965-018-0042-y), and the 3DBAG→SimStadt/CitySim pipeline
   (León-Sánchez et al. 2021) — consume, per building: the **heated volume** derived
   from the LoD2 solid, and the **envelope areas split by semantic surface class**
   (roof, external wall, ground plate), because the energy balance assigns each class
   its own thermal transmittance (U-value). Producing this split from GML is the
   bespoke ETL step every platform re-implements; it is exactly the step this tool
   turns into a query.
2. **Degree-day screening.** A first-order heating-demand proxy needing no weather
   time series: transmission heat-loss coefficient `H_T = Σᵢ Uᵢ·Aᵢ` [W/K] over the
   envelope classes, annual demand ≈ `H_T × HDD × 24 / 1000` [kWh/a], with the
   **surface-to-volume ratio** as the governing form metric — Rode et al. 2014
   (doi:10.1068/b39065) show morphology alone drives up to ~6× differences in
   heat-energy demand across European urban typologies.
3. **Retrofit prioritisation.** Stock models built for policy — 3DStock (Evans et al.
   2017, doi:10.1177/0265813516652898), the London Building Stock Model (Steadman et
   al. 2020, doi:10.5334/bc.52), CityBES retrofit analysis (Chen et al. 2017) — rank
   buildings by construction-year band (U-value proxy) × envelope-per-volume ×
   **measure-specific area**: roof area targets loft insulation, wall area targets
   cavity/external-wall insulation. The ranking is a filter-and-sort over exactly the
   feature table produced in steps 1–2.

## Why CityParquet + DuckDB (the implementation mapping)

- **One store, no load step.** Input is a path, glob, or S3 URL to `building.parquet`
  files; DuckDB reads them in place (httpfs for remote). Columnar pruning touches only
  the id/attribute/geometry columns the query needs.
- **Semantics are columns, not reconstruction.** CityParquet carries
  `geometry_properties_lod2_2.face_semantics` (per-face index) and `.surfaces`
  (semantic objects: RoofSurface, WallSurface, GroundSurface) natively — "roof faces"
  is a filter, not a geometric heuristic.
- **The kernel serves what it can.** Whole-solid metrics run in SQL via `duckdb-3d`;
  the per-face split runs in the Python prototype until `ST_3DFaces` exists.
- **Structure is relational.** In 3DBAG-as-CityParquet, `Building` rows carry
  attributes (`oorspronkelijkbouwjaar`, the `b3_*` reference values) and
  `BuildingPart` children carry geometry; the core query is a parent–child join via
  `parents`, with parts aggregated (summed) per parent.

## Architecture

```
usecase/
  README.md            ← what this directory is; pointer to the candidates note
  energy/
    README.md          ← §1 methodology (above), §2 mapping, §3 usage, §4 validation & limits
    pyproject.toml     ← uv project; console script `energy`
    src/energy/
      cli.py           ← argparse/click entry; subcommands `features`, `screen`
      db.py            ← DuckDB session: extension loading, input resolution
      features.py      ← SQL core: join, ST_3D* metrics, S/V
      faces.py         ← ST_3DFaces prototype: WKB + face_semantics → per-face table
      screen.py        ← degree-day H_T, kWh/a, retrofit ranking
      validate.py      ← comparison against b3_* reference columns
      params/u_values.toml  ← default U-values by construction-year band (TABULA NL;
                              values confirmed at implementation, sources cited inline)
    tests/
      test_faces.py    ← synthetic-solid unit tests (known areas/volume per class)
      test_integration.py  ← fixture tile vs b3_* within tolerance; skips if
                              extensions unavailable
      fixtures/        ← small slice of a real 3DBAG tile (a few hundred rows max)
```

Justfile recipes at monorepo root: `just usecase-energy-features`, `just
usecase-energy-test`.

### CLI

```
energy features --input <path|glob|s3://…> [--lod 2.2] [--output features.parquet]
                [--faces faces.parquet] [--validate report.json]
energy screen   --features features.parquet [--hdd 2900] [--params u_values.toml]
                [--year-before N] [--sv-above X] [--top N] [--output screen.parquet]
```

`features` output, one row per Building: id, year, n_parts, volume_m3, envelope_m2,
sv_ratio, footprint_m2, height_m, a_roof_flat_m2, a_roof_pitched_m2, a_wall_m2,
a_ground_m2, is_closed, plus the `b3_*` reference columns when `--validate` is given.

`screen` output: the features joined with u_roof/u_wall/u_ground (by year band),
h_t_w_per_k, annual_kwh, rank — filtered and sorted per the flags.

### The `ST_3DFaces` contract (prototype output)

`--faces` emits the per-face table the future kernel primitive should return:

```
(object_id, part_id, face_idx, semantic, nx, ny, nz, tilt_deg, azimuth_deg,
 area_m2, cx, cy, cz)
```

Face areas and normals via Newell's method over the WKB polygon rings; semantic label
resolved through `face_semantics[i] → surfaces[j].type`; roofs classed flat vs pitched
by a `--flat-tilt-deg` threshold (default 5°). This schema is the working spec to file
against `duckdb-3d` when `ST_3DFaces` is scheduled.

## Validation

Self-contained on any 3DBAG tile — no external data:

| Computed | Reference (3DBAG attribute) |
|---|---|
| volume_m3 (`ST_3DVolume`) | `b3_volume_lod22` |
| a_roof_flat_m2 / a_roof_pitched_m2 | `b3_opp_dak_plat` / `b3_opp_dak_schuin` |
| a_ground_m2 | `b3_opp_grond` |
| a_wall_m2 | `b3_opp_buitenmuur + b3_opp_scheidingsmuur` |

`--validate` writes per-metric MAE, median relative error and worst-N offenders to a
JSON report. Reporting only — no hard failure thresholds in v1.

## Error handling

- Requested LoD column absent → error listing the LoD columns actually present.
- NULL geometry rows skipped, counted in the run summary.
- Open/invalid solids (`ST_3DIsClosed = false`) flagged in output, never fatal.
- Extensions located via `--ext-dir`, else auto-detected from the repo build tree
  (`lib/duckdb-cityjson/build/…`, `lib/duckdb-3d/build/…`); clear error naming both
  paths if neither loads.
- S3 inputs require httpfs; credentials pass through DuckDB's standard chain.

## Testing

- **Unit (always runs):** `faces.py` against synthetic solids with known per-class
  areas, normals, tilt classes; degenerate faces; multi-shell solids.
- **Integration (skips without built extensions):** full `features` run on the
  committed fixture slice; assert agreement with `b3_*` within tolerance (exact
  bounds set empirically during implementation and recorded in the test).
- CI entry: `uv run --project usecase/energy pytest`.

## Limitations & future work

- **Party walls:** in 3DBAG LoD2.2 each building is a closed solid, so shared walls
  count as WallSurface in the geometric total. v1 documents this and exposes 3DBAG's
  own `buitenmuur`/`scheidingsmuur` split as reference columns; geometric shared-face
  detection is future work (and another `ST_3DFaces` consumer).
- **U-value defaults are screening-grade** (TABULA NL age bands), not calibrated;
  the params file is the override point.
- **Migration path:** when `ST_3DFaces` lands in `duckdb-3d`, `faces.py` reduces to a
  SQL query; the CLI and outputs are designed so that swap is invisible to consumers.
- **Scale demo:** a national-scale run (all tiles, then S3-remote) is a follow-up
  once the tool is correct on single tiles — it feeds the roadmap's Spine ④ benchmark
  argument.
