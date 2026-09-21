---
name: cityparquet
description: Use when a task involves CityParquet — a package of 3D city model Parquet files (building.parquet, geometry_lod* columns, a STAC metadata.json), the CityParquet specification, or the cityjson and three_d DuckDB extensions — or asks what the specification says, before the operation (query, write, measure) is settled.
---

# CityParquet

CityParquet stores a 3D city model as a **directory of Parquet files**, one per
CityGML module, queried with DuckDB. Look at the dataset before writing SQL
against it, and quote the specification rather than recalling it.

## The format in brief

- **One file per module**, with fixed names: `building`, `bridge`, `tunnel`,
  `construction`, `transportation`, `vegetation`, `relief`, `water_body`,
  `land_use`, `city_furniture`, `generics` (`.parquet`). Optional sidecars:
  `materials`, `textures`, `geometry_templates`. A STAC Item,
  `metadata.json`, lists the files.
- **One row per city object.** Reserved columns: `id`; `feature_id` (the id of
  the object's root — group by it to combine a `Building` with its
  `BuildingPart`s); `object_type`; `parents`, `children`, `children_roles`;
  `bbox` (a STRUCT of `xmin` … `zmax`); `address`; `other` (JSON for members
  without a column of their own). Attributes are ordinary typed columns.
- **One geometry column per LoD**: LoD 2.2 is `geometry_lod2_2`. Each is paired
  with `geometry_properties_lod2_2`, which holds what WKB cannot: semantic
  surfaces and shell structure. LoD0 footprints are GeoParquet and arrive in
  DuckDB as `GEOMETRY`; solids arrive as `BLOB`.
- **Each file's Parquet footer** carries a `city` object (the authoritative CRS
  and LoDs) and a GeoParquet `geo` object. Where the footer and
  `metadata.json` disagree, the footer wins.

## Start every dataset task the same way

1. Call `cityparquet_describe` with the package URL or path. It reports the
   tables, row counts, LoDs, geometry columns and CRS. Read its `notes`.
2. Use only the geometry columns it lists. LoDs differ between datasets, and
   between rows: on 3DBAG data a `Building` row has only LoD0, and its
   `BuildingPart` rows carry LoD0 as well as the solids.
3. Check the CRS unit before measuring anything: metres give m² and m³, and
   degrees give numbers with no meaning.

## When the question is about the specification

- `cityparquet_docs_search` finds the passage; `cityparquet_docs_read` reads
  one chapter or one section of it. The corpora are `spec`, `duckdb-cityjson`
  and `duckdb-3d`.
- Quote normative text word for word. Keep MUST, SHOULD and MAY exactly as
  written. If the corpus does not answer the question, say so instead of
  filling the gap.
- A function reference can document a function the loaded build lacks. Before
  relying on one, check that it exists:
  `SELECT function_name FROM duckdb_functions() WHERE function_name ILIKE 'st_3dvolume'`.
  Use `ILIKE`, because some extensions register mixed-case names.

## Where to go next

| The task | Skill |
| --- | --- |
| Read, filter, aggregate, find what a package holds | `cityparquet-query` |
| Write or edit a package, or convert to or from CityJSON | `cityparquet-write` |
| Volume, footprint, height, validity, reprojection | `cityparquet-3d-analysis` |

## `spatial` is not loaded

The server loads `httpfs`, `cityjson` and `three_d`, not `spatial`. `ST_Area`,
`ST_Transform` and `ST_GeomFromWKB` are therefore missing unless the operator
added `spatial`. The `three_d` substitutes are covered in
`cityparquet-3d-analysis`.

## Without the MCP server

Run the same SQL in the `duckdb` CLI, **version 1.5.5**, after
`INSTALL cityjson FROM community; INSTALL three_d FROM community; LOAD cityjson; LOAD three_d;`.
Older DuckDB versions install older builds of both extensions, which lack most
of what the documentation describes. The specification is published at
<https://cityjson.github.io/cityparquet/>.
