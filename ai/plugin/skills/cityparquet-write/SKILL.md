---
name: cityparquet-write
description: Use when creating, editing or exporting CityParquet data — building a package from CityJSON or CityJSONSeq, deleting or updating objects in a package, merging packages, writing a package directory with metadata.json, or converting CityParquet to CityJSON, CityJSONSeq or FlatCityBuf.
---

# Writing and converting CityParquet

Writing needs a writable filesystem. Use a local server (the stdio MCP server,
not sandboxed) or the `duckdb` CLI v1.5.5. A sandboxed or hosted server refuses
every `COPY … TO` and every `cityparquet_write`.

## Two rules that fail silently when broken

1. **Send each pragma as its own statement.** DuckDB expands every pragma in a
   batch before running any statement, so `CREATE SCHEMA d; PRAGMA
   cityparquet_init('d');` sent to the CLI as one batch fails.
   `cityparquet_query` splits scripts for you; the CLI does not.
2. **The CRS is never reprojected.** On insert or merge it must match the
   package's CRS, and an unknown on one side is refused. A package loaded with
   `read_parquet` states no CRS at all. Pass `crs =>` when you write it,
   otherwise the footer records an explicit `null`.

## Edit an existing package

```sql
CREATE SCHEMA delft;
PRAGMA cityparquet_read('/data/delft', 'delft');        -- local directory; keeps the footers
PRAGMA cityparquet_delete('delft', 'object_type = ''BuildingPart''');
SELECT * FROM cityparquet_write('delft', '/data/delft-edited/');
```

- `cityparquet_delete` cascades down `children` by default, and it removes the
  deleted ids from their parents' `children`. Quotes inside the predicate are
  doubled, as in any SQL string.
- **Attribute edits** are an ordinary `UPDATE delft.building SET …`. **Geometry
  or hierarchy edits** need `PRAGMA cityparquet_reconcile('delft')` afterwards,
  which re-derives `feature_id`, `bbox` and the parent/child arrays. There is
  deliberately no `cityparquet_update`.
- `PRAGMA cityparquet_validate('delft'); SELECT * FROM cityparquet_validation;`
  lists dangling references and duplicate ids. No rows means none were found.
- `cityparquet_write` regenerates every footer and `metadata.json`. It sees
  only committed data.

## Build a package from CityJSON

`cityparquet_init` needs at least one object table to exist first, and
`create_tables = true` does not replace that step:

```sql
CREATE SCHEMA pkg;
CREATE TABLE pkg.building AS
  SELECT * FROM read_cityjsonseq('https://cityjson.open3d.city/cityjsonseq/delft.city.jsonl', appearance := 'sidecar');
PRAGMA cityparquet_init('pkg');
SELECT * FROM cityparquet_write('pkg', '/data/pkg/', crs => 'EPSG:7415');
```

- Name each table after the module its objects belong to (`building`,
  `transportation`, …).
- After that, `PRAGMA insert_cityjson('pkg', 'more.city.json')` adds a file.
  `insert_cityjsonseq` and `insert_flatcitybuf` do the same for the other
  formats. Each object goes to its module's table, and a type with no module is
  an error. An id already present refuses the **whole** insert.
- `PRAGMA cityparquet_merge('dst', 'src')` combines two loaded packages under
  the same id and CRS rules.

## Convert

```sql
COPY (SELECT * FROM read_parquet('https://cityparquet.open3d.city/data/delft/building.parquet'))
TO '/data/delft.city.jsonl'
(FORMAT cityjsonseq, crs 'https://www.opengis.net/def/crs/EPSG/0/7415');
```

- The formats are `cityjsonseq`, `cityjson` and `flatcitybuf`. The required
  columns are `id`, `feature_id` and `object_type`.
- **State the `crs` option whenever the source is Parquet or a table.** `COPY`
  inherits a CRS only from a single `read_cityjson…` reader. From Parquet, it
  otherwise writes a file with **no** reference system and gives no error.
- `bbox`, `other`, `address` and templates are not written to CityJSON.
- `flatcitybuf` takes `attr_index 'col1,col2'` to index attributes for later
  filtering.

## Verify by reading back

Round-trip every write before reporting success:

```sql
SELECT reference_system.code, city_objects_count FROM cityjsonseq_metadata('/data/delft.city.jsonl');
PRAGMA cityparquet_read('/data/pkg', 'readback');
SELECT json_extract_string(city, '$.crs.id.code') AS epsg FROM readback.__cityparquet;
```

A `NULL` code means the CRS was lost on the way out.
