# `energy` — building-energy feature factory

A CLI tool that produces the geometric inputs urban building energy models
(UBEM) and retrofit programmes need — heated volume and envelope areas split
by semantic surface class — straight from a CityParquet package on disk or
object storage, with no database load step. It also runs a first-order,
weather-free heating-demand screen and ranks buildings for retrofit targeting.

The physics stays external: this tool is the *data-preparation and
screening* stage, which the literature identifies as the actual bottleneck,
not an energy simulator. `energy features` and `energy screen` are separate
subcommands so the feature table can feed any downstream consumer, not only
the screen below.

## Methodology

The argument for this tool comes from the literature, not the implementation
— the sections below document that argument first.

### UBEM envelope inputs

Bottom-up urban energy platforms driven by CityGML-class models — SimStadt
(Nouvel et al. 2015), the Energy ADE (Agugiaro et al. 2018,
doi:10.1186/s40965-018-0042-y), and the 3DBAG→SimStadt/CitySim pipeline
(León-Sánchez et al. 2021) — consume, per building: the **heated volume**
derived from the LoD2 solid, and the **envelope areas split by semantic
surface class** (roof, external wall, ground plate), because the energy
balance assigns each class its own thermal transmittance (U-value).
Producing this split from GML is the bespoke ETL step every platform
re-implements; it is exactly the step this tool turns into a query.

### Degree-day screening

A first-order heating-demand proxy needing no weather time series: the
transmission heat-loss coefficient

```
H_T = Σᵢ Uᵢ·Aᵢ    [W/K]
```

over the envelope classes, with annual demand approximated as

```
annual kWh ≈ H_T × HDD × 24 / 1000
```

The **surface-to-volume ratio** is the governing form metric — Rode et al.
2014 (doi:10.1068/b39065) show morphology alone drives up to roughly 6×
differences in heat-energy demand across European urban typologies.

### Retrofit prioritisation

Stock models built for policy — 3DStock (Evans et al. 2017,
doi:10.1177/0265813516652898), the London Building Stock Model (Steadman et
al. 2020, doi:10.5334/bc.52), CityBES retrofit analysis (Chen et al. 2017) —
rank buildings by construction-year band (a U-value proxy) × envelope-per-
volume × **measure-specific area**: roof area targets loft insulation, wall
area targets cavity/external-wall insulation. The ranking is a filter-and-
sort over exactly the feature table `energy features` produces.

## How it maps onto the stack

- **One store, no load step.** Input is a path, glob, or `s3://` URL to
  `building.parquet` files; DuckDB reads them in place (`httpfs` for
  remote). Columnar pruning touches only the id/attribute/geometry columns
  the query needs.
- **Semantics are columns, not reconstruction.** CityParquet carries
  `geometry_properties_lod2_2.face_semantics` (per-face index) and
  `.surfaces` (semantic objects: `RoofSurface`, `WallSurface`,
  `GroundSurface`) natively — "roof faces" is a filter, not a geometric
  heuristic.
- **Structure is relational.** `Building` rows carry attributes
  (`oorspronkelijkbouwjaar`, the `b3_*` reference values) and `BuildingPart`
  children carry geometry. `features.py` joins them via
  `BuildingPart.parents[1] = Building.id` and aggregates (sums) metrics
  across a building's parts before joining back to the building's
  attributes.
- **The kernel serves what it can.** Whole-solid metrics run in SQL, via the
  `duckdb-3d` and `duckdb-cityjson` extensions, on every `BuildingPart`
  solid:

  | Column         | Expression                          |
  | --------------- | ------------------------------------ |
  | `volume_m3`     | `ST_3DVolume(solid)`, summed over parts |
  | `envelope_m2`   | `ST_3DSurfaceArea(solid)`, summed over parts |
  | `footprint_m2`  | `ST_3DFootprintArea(solid)`, summed over parts |
  | `height_m`      | `max(ST_3DZMax) - min(ST_3DZMin)` over parts |
  | `is_closed`     | `bool_and(ST_3DIsClosed(solid))` over parts |
  | `sv_ratio`      | `envelope_m2 / volume_m3`             |

  Each `BuildingPart.geometry_lodX_Y` column is read via
  `ST_3DTryFromWKB`; rows with `NULL` geometry at the requested LoD are
  skipped and counted (see Usage below).

### The per-face module: an `ST_3DFaces` prototype

The per-class split — wall vs roof vs ground area — needs face-level
decomposition with semantic labels, which the kernel does not yet expose as
a SQL primitive. `faces.py` reimplements, in Python, what a future
`ST_3DFaces` table function in `duckdb-3d` should do: it parses each
`BuildingPart`'s WKB solid (`PolyhedralSurface`/`MultiPolygon`) directly,
computes each face's normal and area via Newell's method, an
area-weighted centroid via fan triangulation from the ring's first vertex
(holes subtracted the same way), and resolves the face's semantic label by
looking up `face_semantics[i]` into the part's `surfaces[j].type`. Roof
faces are further split into flat vs pitched by the `--flat-tilt-deg`
threshold (default 5°).

This is the working schema to file against `duckdb-3d` when `ST_3DFaces` is
scheduled; when it lands, `faces.py` reduces to a SQL query and the CLI and
outputs are unaffected:

| Column        | Meaning                                                      |
| ------------- | ------------------------------------------------------------- |
| `building_id` | parent `Building.id` (via `BuildingPart.parents[1]`)          |
| `part_id`     | the `BuildingPart.id` the face belongs to                     |
| `face_idx`    | index of the face within the part's solid                     |
| `semantic`    | `RoofSurface` / `WallSurface` / `GroundSurface` / `Unknown`    |
| `nx, ny, nz`  | unit normal (outward for a correctly oriented solid)           |
| `tilt_deg`    | angle from vertical-up (0° = flat roof facing up)              |
| `azimuth_deg` | compass bearing of the normal; `NULL` for horizontal faces      |
| `area_m2`     | face area, exterior ring minus holes                          |
| `cx, cy, cz`  | area-weighted centroid                                        |

## Usage

Input to `energy features` must be 3DBAG-as-CityParquet: the `b3_*` reference
columns and `oorspronkelijkbouwjaar` present. Other CityParquet packages are
rejected with a clear error rather than failing deep inside a SQL query.

```
energy features --input <path|glob|s3://…> [--lod 2.2] [--output features.parquet]
                [--faces faces.parquet] [--validate report.json]
                [--flat-tilt-deg 5.0] [--ext-dir DIR]
energy screen   --features features.parquet [--hdd 2900] [--params u_values.toml]
                [--year-before N] [--sv-above X] [--top N] [--output screen.parquet]
```

### `features`

| Flag              | Default            | Meaning                                                          |
| ----------------- | ------------------- | ----------------------------------------------------------------- |
| `--input`         | *(required)*         | path, glob or `s3://` URL of `building.parquet` file(s)          |
| `--lod`           | `2.2`                | LoD to read                                                       |
| `--output`        | `features.parquet`   | per-building feature table                                       |
| `--faces`         | *(none)*             | also write the per-face table (the `ST_3DFaces` prototype output) |
| `--validate`      | *(none)*             | write a JSON comparison against 3DBAG's `b3_*` reference columns |
| `--flat-tilt-deg` | `5.0`                 | roof tilt at or below this counts as flat                        |
| `--ext-dir`       | *(auto-detected)*    | directory holding the `.duckdb_extension` binaries                |

Output is one row per `Building`: `building_id`, `year`, `n_parts`,
`volume_m3`, `envelope_m2`, `sv_ratio`, `footprint_m2`, `height_m`,
`is_closed`, `a_roof_flat_m2`, `a_roof_pitched_m2`, `a_wall_m2`,
`a_ground_m2`, `a_other_m2` (faces with no resolvable semantic label), plus
the `b3_*` reference columns when `--validate` is given.

The command finishes by printing a run summary:

```
<N> buildings, <M> parts (<X> null-geometry parts skipped, <Y> buildings
without usable geometry, <Z> open solids flagged)
wrote <output>
```

`<Y>` counts buildings whose parts *all* lack usable geometry at the
requested LoD — they are silently dropped by the Building/BuildingPart
inner join, so the summary is the only place they are visible. `<Z>` counts
buildings flagged `is_closed = false`; this is never fatal, only reported.

### `screen`

| Flag            | Default             | Meaning                                              |
| --------------- | -------------------- | ------------------------------------------------------ |
| `--features`    | *(required)*          | `features.parquet` from `features`                    |
| `--hdd`         | `2900.0`               | heating degree days, K·d (default: NL base 18 °C)     |
| `--params`      | *(built-in)*           | U-value bands TOML (default: `params/u_values.toml`)  |
| `--year-before` | *(none)*               | keep only buildings built before this year            |
| `--sv-above`    | *(none)*               | keep only buildings with S/V above this                |
| `--top`         | *(none)*               | keep only the top-N ranked                             |
| `--output`      | `screen.parquet`       | ranked output table                                    |

Output is the features table joined with `u_roof`/`u_wall`/`u_ground` (by
construction-year band), `h_t_w_per_k`, `annual_kwh` and `rank`, filtered and
sorted per the flags above. `h_t_w_per_k` sums the roof, wall and ground
classes only — `a_other_m2` (faces with no resolvable semantic label) does
not contribute a U-value term and is excluded from the heat-loss sum.

Filter semantics, since "unknown" is not the same as "excluded":

- `--year-before`: a building with an unknown construction year is **kept**
  (treated as the oldest band, since an unknown-age building cannot be
  assumed newer than the cutoff); a building with a known year is dropped
  once `year >= year-before`.
- `--sv-above`: a building with no computable S/V ratio (zero or missing
  volume) is **dropped** — the tool cannot assert it is above the threshold.
- `rank` is assigned by descending `annual_kwh` over the *filtered* set,
  before `--top` truncates it, so rank 1 is always the highest-demand
  building that survived the filters.
- An empty result (no rows survive filtering) still carries the full output
  schema, so downstream consumers do not have to special-case a
  zero-row file.

### Example: a local tile

```sh
cd usecase/energy
uv run energy features \
    --input /data2/hideba/cityparquet_data/10-756-44/building.parquet \
    --output /tmp/features.parquet
uv run energy screen \
    --features /tmp/features.parquet --year-before 1975 --top 20 \
    --output /tmp/screen.parquet
```

or, from the monorepo root, via the justfile recipes (see below):

```sh
just usecase-energy-features /data2/hideba/cityparquet_data/10-756-44/building.parquet /tmp/features.parquet
```

### Remote input (`s3://`)

`--input` accepts an `s3://` URL or glob directly; the tool installs and
loads DuckDB's `httpfs` extension automatically whenever the input starts
with `s3://`. Credentials pass through DuckDB's standard chain (environment
variables, `~/.aws/credentials`, an assumed role) — no extra flags are
needed.

## Validation & limitations

### `--validate`

`energy features --validate report.json` compares the computed columns
against 3DBAG's own reference attributes — self-contained, no external data
needed:

| Computed                              | Reference (3DBAG attribute)                  |
| -------------------------------------- | ---------------------------------------------- |
| `volume_m3` (`ST_3DVolume`)            | `b3_volume_lod22`                             |
| `a_roof_flat_m2` / `a_roof_pitched_m2` | `b3_opp_dak_plat` / `b3_opp_dak_schuin`       |
| `a_ground_m2`                          | `b3_opp_grond`                                |
| `a_wall_m2`                            | `b3_opp_buitenmuur + b3_opp_scheidingsmuur`   |

For each metric the report gives `n`, `mae`, `median_rel_err_pct`,
`n_zero_reference_mismatches` and up to five `worst` offenders
(`building_id`, `computed`, `reference`, `rel_err_pct`). A building where the
reference is exactly zero but the computed value is not is a **zero-reference
mismatch**: it is excluded from `median_rel_err_pct` (a relative error against
zero is undefined) and surfaced separately, with `rel_err_pct: null` in
`worst`, rather than masked as either a perfect match or silently dropped. On
the committed fixture this actually occurs for `roof_flat` (7 buildings) —
e.g. building `NL.IMBAG.Pand.0928100000041788`, computed `a_roof_flat_m2` =
33.94 m² against `b3_opp_dak_plat` = 0.0, most plausibly a flat/pitched
classification-threshold difference between this tool's `--flat-tilt-deg`
default and 3DBAG's own criterion. Reporting is descriptive only — v1 sets
no hard failure thresholds.

Measured on the committed fixture (150 buildings, 3DBAG tile 10-756-44):
volume median relative error 0.0212%, ground median relative error 0.0177%;
`volume`, `roof_pitched`, `ground` and `wall` have no zero-reference
mismatches on this fixture (`roof_flat`'s 7 are the example above).

### Party walls

In 3DBAG LoD2.2 each building is a closed solid, so a wall shared with a
neighbour is counted as `WallSurface` in `a_wall_m2` like any other external
wall — there is no geometric party-wall detection in v1. The output exposes
3DBAG's own `buitenmuur`/`scheidingsmuur` split as reference columns (when
`--validate` is given) so a consumer can separate them if the distinction
matters; detecting shared faces geometrically is future work, and another
candidate consumer of `ST_3DFaces`.

### U-values are screening-grade

The default U-values in `params/u_values.toml` are order-of-magnitude,
Dutch construction-year bands intended for *ranking*, not calibrated thermal
design. They are pending confirmation against TABULA NL (episcope.eu)
age-band values before any publication-grade run; `--params` is the
override point for a calibrated set.

### Migration path

When `ST_3DFaces` lands in `duckdb-3d`, `faces.py`'s per-face computation
reduces to a SQL query against that primitive. The CLI, its flags and the
output schemas are designed so that swap is invisible to anything consuming
`energy features`/`energy screen` output.
