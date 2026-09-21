---
name: cityparquet-query
description: Use when reading, filtering or aggregating CityParquet data with DuckDB — querying building.parquet or another module table over HTTPS or from disk, loading a package into DuckDB, finding which LoDs or attributes a dataset has, or reading CityJSON, CityJSONSeq or FlatCityBuf as tables.
---

# Querying CityParquet

Everything is SQL through `cityparquet_query`, which splits a script and runs
its statements one at a time. It caps rows at 100 by default and prints
geometry as `<BLOB n bytes>`.

## Loading: `cityparquet_read`, not `read_parquet`, whenever you will write

Both give the same rows. **Only `cityparquet_read` keeps each file's Parquet
footer**, which holds the CRS. `read_parquet` discards the footer, and it
cannot be recovered afterwards. A package loaded with `read_parquet` therefore
states no CRS: inserts and merges skip the CRS check, and a later write stores
the CRS as an explicit `null`.

```sql
CREATE SCHEMA delft;
PRAGMA cityparquet_read('/data/delft', 'delft');   -- a local directory
SELECT table_name, role, city IS NOT NULL AS has_footer FROM delft.__cityparquet;
```

- A package becomes a schema, and its tables are named after the files
  (`delft.building`).
- `cityparquet_read` accepts **local directories only**. On a URL it fails with
  `HTTPFileSystem: DirectoryExists is not implemented`. To edit a remote
  package, download `metadata.json` and every file it lists **byte for byte**
  (`curl -O`). Do not re-save them with `COPY … TO`: that writes new files
  without the original footers.
- For **read-only** questions on a remote package, `read_parquet` on the file
  URL is correct and needs no load step. Nothing is written, so the lost footer
  does not matter.

## Reading one table remotely

```sql
SELECT object_type, count(*) AS n
FROM read_parquet('https://cityparquet.open3d.city/data/delft/building.parquet')
GROUP BY ALL;
```

- **Never `SELECT *`** on an object table. Name the columns you need: geometry
  cells are kilobytes each, and projecting fewer columns means fewer bytes over
  HTTP.
- **Filter on `bbox`** for spatial windows, e.g.
  `WHERE bbox.xmin < 85000 AND bbox.xmax > 84000 AND bbox.ymin < 447000 AND bbox.ymax > 446000`.
  The coordinates are in the package CRS; check it with `cityparquet_describe`.
- To find which geometry columns exist without downloading data:
  `SELECT name FROM parquet_schema('<url>') WHERE name LIKE 'geometry_lod%'`.
- To read the footer: `SELECT decode(value) FROM parquet_kv_metadata('<url>') WHERE decode(key) = 'city'`.
  Keys and values come back as `BLOB`, so `decode()` both.

## Rows are objects, not buildings

A `Building` and its `BuildingPart`s are separate rows, and they share a
`feature_id`, which is the root object's `id`. Which rows carry which LoD
depends on the dataset, so check it (last bullet) before aggregating. On
3DBAG data the `Building` row has only the LoD0 footprint. Each
`BuildingPart` has LoD0 **as well**, plus the LoD 1.2 to 2.2 solids. So, on
such data:

- Take solids from all rows, and report per building with `GROUP BY feature_id`.
  Filtering to `object_type = 'Building'` finds no solids at all.
- Take LoD0 from **one** of the two kinds of row. Summing it over every row
  counts each footprint twice: on Delft that gives 643 626 m² instead of
  323 154 m².
- Check how a dataset spreads its LoDs with
  `SELECT object_type, count(geometry_lod0_0), count(geometry_lod2_2) FROM … GROUP BY 1`.

## Other sources, same column layout

```sql
SELECT id, b3_h_dak_max
FROM read_cityjsonseq('https://cityjson.open3d.city/cityjsonseq/delft.city.jsonl')
WHERE object_type = 'Building'
ORDER BY b3_h_dak_max DESC LIMIT 3;
```

- `read_cityjson` reads a `.city.json` document, and `read_cityjsonseq` reads a
  `.city.jsonl` stream. Both push equality filters on `id`, `feature_id` and
  `object_type` into the scan. Pass `lod => '2.2'` to keep one LoD.
- `read_flatcitybuf` reads `.fcb`. It uses its R-tree when you pass all four of
  `xmin =>`, `ymin =>`, `xmax =>` and `ymax =>`.
- `cityjsonseq_metadata(url)` and its siblings return the dataset's CRS and
  object count in one row.

## Common mistakes

| Mistake | Fix |
| --- | --- |
| `object_id`, `geometry`, `geometry_lod0` | The columns are `id` and `geometry_lod0_0`; take the names from `cityparquet_describe` |
| `read_parquet` load, then insert, merge or write | Load with `cityparquet_read`. After a `read_parquet` load, inserts and merges run **without** a CRS check, so data in another CRS mixes in silently, and `crs =>` on the write only labels the mixture |
| `cityparquet_read` on a URL | Download the package; `read_parquet` for read-only questions |
| `CREATE SCHEMA d; PRAGMA …('d')` sent to the DuckDB CLI in one batch | DuckDB expands every pragma before running any statement, so send them separately. `cityparquet_query` already does this |
| Assuming a function exists | `SELECT function_name FROM duckdb_functions() WHERE function_name ILIKE '…'` |

Measurements (volume, area, height) are covered in `cityparquet-3d-analysis`.
Without the server, use the same SQL in `duckdb` v1.5.5 after
`INSTALL cityjson FROM community; LOAD cityjson;`.
