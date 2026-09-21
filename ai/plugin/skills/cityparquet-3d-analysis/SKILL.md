---
name: cityparquet-3d-analysis
description: Use when measuring or checking 3D city-model geometry in DuckDB — building volume, footprint or surface area, height, solid validity, or reprojecting geometry — or when ST_3DVolume raises "solid is not manifold", or when ST_Area, ST_Volume or ST_GeomFromWKB are missing or reject a CityParquet geometry column.
---

# 3D analysis with `three_d`

Solids go through the `three_d` extension. `spatial` is loaded too, but it
cannot read solids: `ST_GeomFromWKB` rejects `PolyhedralSurface Z`, and
`ST_Area` or `ST_Transform` on a solid column fails. Use `spatial` only on the
LoD0 column. No function named `ST_Volume` exists; the function is
`ST_3DVolume`.

## The gated measurement pattern

`ST_3DVolume` **raises** (`solid is not manifold`) on any solid that is not
closed, manifold and oriented, and real data always contains some. On Delft,
18 of 1116 building parts fail. So build with the `TRY` constructor, keep the
properties, and filter on validity **in a `WHERE`** before measuring:

```sql
SELECT round(sum(ST_3DVolume(solid))) AS volume_m3, count(*) AS solids
FROM (
  SELECT ST_3DTryFromWKB(geometry_lod2_2, geometry_properties_lod2_2) AS solid
  FROM read_parquet('https://cityparquet.open3d.city/data/delft/building.parquet')
  WHERE geometry_lod2_2 IS NOT NULL
)
WHERE solid IS NOT NULL AND ST_3DValidationReport(solid).is_valid;
```

- **`sum(ST_3DVolume(s)) FILTER (WHERE …is_valid)` does not protect you.**
  DuckDB evaluates the aggregate's argument on every row and still raises. Use
  an outer `WHERE`, or `CASE WHEN ST_3DValidationReport(s).is_valid THEN ST_3DVolume(s) END`.
- **Always pass `geometry_properties_lod*`** as the second argument. Without it
  every solid imports as a single shell, so interior shells (cavities) no
  longer subtract from volume.
- Gate only the functions that need it: volume and surface area. Footprint,
  height and bounds work on invalid solids too, and gating them drops
  buildings for nothing.
- `ST_3DTryFromWKB` returns `NULL` where `ST_3DFromWKB` would raise. Count
  both the `NULL`s and the invalid solids, and report them next to the result.
- To see why solids fail: `SELECT ST_3DValidationReport(solid).message, count(*) … GROUP BY 1`.

## Which function

| Want | Use | Needs a valid solid |
| --- | --- | --- |
| Volume | `ST_3DVolume(solid)` | yes, raises otherwise |
| Surface area | `ST_3DSurfaceArea(solid)` | no degenerate faces, raises otherwise |
| Footprint (XY ground area) | `ST_3DFootprintArea(solid)`; for LoD0, `ST_Area(geometry_lod0_0)` | no |
| Height | `ST_3DBounds(solid).max_z - ST_3DBounds(solid).min_z` | no |
| Extent | `ST_3DBounds(solid)`, or the `bbox` column without parsing anything | no |
| Validity | `ST_3DValidationReport(solid)`, a STRUCT with `is_valid`, `code` and `message` | no |
| Reproject | `ST_3DTransform(solid, 'EPSG:7415', 'EPSG:4326')`; for LoD0, `ST_Transform` | no |

- **`SOLID_3D` and `GEOM_3D` are different types.** `ST_3DTryFromWKB` builds
  solids, which volume, validity and surface area need. `ST_Geom3DFromWKB`
  builds general geometry, which distance and LoD0 multipolygons need. LoD0 is
  a `MultiPolygon`, so `ST_3DTryFromWKB(geometry_lod0_0)` returns `NULL`.
- **Find which rows carry which LoD before choosing rows.**
  `SELECT object_type, count(geometry_lod0_0), count(geometry_lod2_2) FROM … GROUP BY 1`.
  On 3DBAG data the solids sit on `BuildingPart` rows only, and LoD0 on both
  `Building` and `BuildingPart` rows. Other datasets differ.
- **Aggregate per building** with `GROUP BY feature_id`, which is the root
  object's `id`. Height is then
  `max(ST_3DBounds(s).max_z) - min(ST_3DBounds(s).min_z)`.
- **Count each footprint once.** Where both a building and its parts carry
  LoD0, sum LoD0 over `object_type = 'Building'` only; summing every row
  counts each footprint twice. Do not sum the parts' footprints instead:
  parts can overlap in plan, such as a tower above a podium, and their sum
  then exceeds the ground area.
- **Units follow the CRS.** A metre-based CRS such as EPSG:7415 gives m² and
  m³. Geographic coordinates in degrees give meaningless numbers, so reproject
  them to a projected CRS first. `ST_3DTransform` reprojects X and Y only:
  **Z is left unchanged**, and `EPSG:4326` comes out as (lon, lat).
  `spatial`'s `ST_Transform` follows the authority's axis order instead, so
  `EPSG:4326` comes out as **(lat, lon)** unless you pass `always_xy := true`.

## Common mistakes

| Symptom | Cause and fix |
| --- | --- |
| `ST_3DVolume: solid is not manifold` | No validity gate, or a `FILTER` gate. Filter in an outer `WHERE` |
| `ST_Volume` does not exist; `ST_Area`, `ST_Transform` or `ST_ZMax` rejects a solid | `ST_3DVolume` / `ST_3DFootprintArea` / `ST_3DTransform` / `ST_3DBounds(s).max_z` |
| Volume sum far too small or zero | Filtered to an object type that carries no solids. On 3DBAG data they are on `BuildingPart` rows |
| Footprint total twice too large | LoD0 summed over both buildings and parts; sum the `Building` rows |
| Cavity volumes counted as solid | The properties argument was left out |
| A function from the docs is missing | Check `duckdb_functions()` with `ILIKE`. The loaded build may be older |

Without the server, use `duckdb` v1.5.5 after
`INSTALL three_d FROM community; LOAD three_d; LOAD spatial;`.
`ST_Area(geometry_lod0_0)` and `ST_3DFootprintArea` give the same footprint.
