# CityParquet vs cjdb vs 3DCityDB v5: database benchmark harness

`citybench` compares CityParquet — read by DuckDB (`duckdb-cityparquet`) and,
optionally, by the native Rust reader (`cityparquet`) —
against two PostgreSQL-based 3D city model databases, **cjdb** and
**3DCityDB v5**. It follows the discipline of
`benchmark/formats/READ_BENCHMARK.md` (real inputs, repeated warm samples,
disclosed rather than hidden overheads) and adds what a cross-**system**
comparison must also control: server tuning, resource limits, index parity,
and the client-server boundary the two PostgreSQL systems sit behind.

The container runtime is rootless **Podman**.

## Purpose and claim

The harness measures **steady-state performance** — client-side
wall-clock time (`time_*`: from the harness issuing the query until it has
every row), peak resident memory of the executing process and, for
PostgreSQL read scenarios, the server-reported execution time recorded
separately (`server_time_*`, from `EXPLAIN (ANALYZE)`) — against a dataset already
loaded into each system. Ten **read** scenarios run under two disclosed
thread configurations; four **write** scenarios then run once, under
`threads=single`, reported as the write-tier rows below the reads in the
`databases` figure, under their own caveat (Caveat 19). The scenario set is the
author's query catalogue, `notes/benchmark-queries.md`, which is the
specification this harness implements. **Ingest is not compared.** Encoding a CityParquet package and populating an indexed
relational schema are different operations, not points on one scale.
Ingest wall-clock is recorded in `<dataset>.manifest.json` (never in the
results CSV) with this caveat attached:

> Ingest timings are context only and are NOT comparable across systems.
> Encoding a CityParquet package and populating an indexed relational
> schema are different operations; this benchmark is scoped to
> steady-state query performance.

The CityParquet-based systems read a package prepared beforehand by
`cityparquet convert`, so their `ingest()` is a no-op recorded as `0.0`
seconds: the absence of a load step is the property under discussion, not a
measurement gap.

## Committed evidence

The committed database results are one run over the 1,000,001-object 3DBAG
slice (`3dbag_n1000000`), measured on 23 September 2026 on this
scenario set, in both thread configurations, with the write tier:

| File                                                                | Contents                                                                                                                                                        |
| ------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `benchmark/runs/databases/results/3dbag_n1000000.csv`               | 108 rows: the read scenarios for every system under `threads=single` and `threads=parallel`, then the write tier; `repeat` = 7                                  |
| `benchmark/runs/databases/results/3dbag_n1000000.manifest.json`     | source SHA-256, host, versions, `pg_settings` per configuration, ingest times, sizes, the cjdb patch disclosure, SRIDs, memory scope, the count tolerance       |
| `benchmark/runs/databases/results/3dbag_n1000000.params.json`       | the query parameters derived from the source and the package: windows with achieved fractions, the attribute predicates, the four id probes, the append feature |
| `benchmark/runs/databases/results/3dbag_n1000000.indexes.sql`       | the DDL this harness added, plus a live `pg_indexes` dump for both PostgreSQL schemas                                                                           |
| `benchmark/runs/databases/results/3dbag_n1000000.append.city.jsonl` | the one-feature CityJSONSeq file the `append-object` scenario imports                                                                                           |

Every row is `ok` or `ok-deviation` (the nine spatial rows per configuration
differ across systems by at most 0.02 %, with the decomposition in `notes`)
except the two CityParquet `append-object` rows, which error because the
DuckDB CityJSON extension build that run loaded by name refuses
`PRAGMA insert_cityjsonseq` into a `cityparquet_read` package (see the
write tier below). The package the DuckDB systems read carries bloom filters
(Caveat 21).

`benchmark/runs/RESULTS.md` describes the run and its limitations; read it
before citing a number. In brief:

- **Scope.** One dataset, the three default systems (`duckdb-cityparquet`,
  `cjdb`, `3dcitydb`), seven timed samples per row. The native-reader
  systems were not part of the run, so `peak_heap_bytes` is empty on every
  row.
- **Provenance.** The manifest records no Git revision and no timestamp.
- **Count mismatches — now explained.** All nine `bbox-query` rows carry
  `status=mismatch`, and the run therefore exited non-zero. The mechanism
  has since been established object by object (Caveats 11 and 12): cjdb's
  importer drops 2/16/60 BuildingPart footprints, and PostGIS's float4 `&&`
  admits 4 extra objects at the 25 % window on both PostgreSQL systems. A
  re-run under the current harness will publish these as
  `status=ok-deviation` with that decomposition in `notes`, because the
  relative spread (0.03-0.04 %) is inside the stated tolerance.

## Systems

| tag                            | what it is                                                                                                                                                                           | runs                                                                       | index support                                                                                                                                                                  |
| ------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | -------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `duckdb-cityparquet`           | DuckDB (Python client) `read_parquet()` over the CityParquet package `<prepared>/<dataset>.parquet`, rows in Hilbert-curve order; no separate ingest                                 | every scenario                                                             | Parquet statistics used by DuckDB's own scan, and the package's bloom filters on `id`, `feature_id` and high-cardinality string attributes for equality predicates (Caveat 21) |
| `duckdb-cityparquet-writeback` | the same as `duckdb-cityparquet`, with `cityparquet_write` inside the timed window                                                                                                   | the write tier only                                                        | —                                                                                                                                                                              |
| `cjdb`                         | cjdb 2.2.0, **patched (Caveat 2)**, imported into PostgreSQL/PostGIS. Full geometry is JSONB (`city_object.geometry`); only a 2D footprint is a PostGIS geometry (`ground_geometry`) | every scenario                                                             | cjdb's own defaults plus one added btree(`object_id`) — see "Index sets"                                                                                                       |
| `3dcitydb`                     | 3DCityDB v5.1.2, imported with `citydb-tool` 1.3.2 into PostgreSQL/PostGIS. Generic `feature`/`property`/`geometry_data` schema: CityGML classes are rows, attributes are EAV rows   | every scenario                                                             | the indexes `citydb-tool import cityjson` creates; none added                                                                                                                  |
| `cityparquet`                  | the native Rust reader over the same package, driven per sample as `cityparquet-readbench --child --format cityparquet`                                                              | `count`, `bbox-query`, `attr-filter`, `attr-stats`, `id-lookup` (Caveat 7) | Parquet row-group min/max statistics and column projection                                                                                                                     |

`citybench run` uses the two `duckdb-cityparquet*` tags plus `cjdb` and
`3dcitydb` by default. The native reader runs only when named in
`--systems` and needs the binary
`benchmark/readbench/target/release/cityparquet-readbench`
(`cargo build --release --manifest-path benchmark/readbench/Cargo.toml`).

### Which package is "CityParquet"

Each dataset has one CityParquet package, `<prepared>/<dataset>.parquet`,
written by `cityparquet convert --ordering hilbert`: rows in Hilbert-curve
order, so each row group covers a compact region and its `bbox` statistics
are tight. It is the artefact the format family's figures display under the
name "CityParquet" (`benchmark/plot/benchviz/figures.py`), so the two
benchmark families publish the same artefact under one name
(`notes/benchmark-fairness-review-2026-09-22.md` §4.5).

Row order can change only an answer's _cost_, never the answer, and only
where a predicate is spatial. In this scenario set that is `bbox-query`:
its rows are measured on the Hilbert order the writer chose, and are not a
measurement of CityParquet in some other row order.

## Query parameters

Every system receives the **same** parameters, derived deterministically by
`citybench.params.derive` (ties are broken by sorting). No system derives
its own idea of "a 5 % window" or "a typical building".

Derivation reads two things: the source CityJSON/CityJSONSeq file, and the
**CityParquet package**. The package supplies the extent, the query
windows, the `attr-filter` predicate and `attr-range`'s threshold — every
parameter the format harness also derives from the package — so the two
families ask the same questions of the same dataset. None of these
parameters depends on the package's row order: the windows come from the
`bbox` column and the attribute picks from value counts, and the id probes
are taken from the source's CityJSONSeq stream order.

| field                | derivation                                                                                                               | from    |
| -------------------- | ------------------------------------------------------------------------------------------------------------------------ | ------- |
| `bbox_full`          | union of every row's `bbox`                                                                                              | package |
| `windows`            | three row-fraction windows, below                                                                                        | package |
| `point_xy`           | the median row centre the windows are built around; recorded as the windows' provenance, not asked as a query of its own | package |
| `attr_filter`        | the per-dataset `attr-filter` predicate, below                                                                           | package |
| `attr_range`         | `b3_h_dak_max` where present, else `numeric_column`, thresholded at its own 0.8 quantile                                 | package |
| `numeric_column`     | most frequent numeric attribute; `null` if none                                                                          | source  |
| `id_probes`          | `id-lookup`'s four targets, below                                                                                        | source  |
| `append`             | the derived one-feature file `append-object` imports, below                                                              | source  |
| `total_city_objects` | the selectivity denominator                                                                                              | source  |
| `window_rows`        | rows with a non-NULL `bbox`; the windows' own denominator                                                                | package |

`citybench run` derives the parameters afresh on every run and writes them
beside the CSV as `<dataset>.params.json`; it never reads a name-keyed
file.

### The query windows

`bbox-query` uses three windows targeting **1 %, 5 % and 25 % of the
package's ROWS**. Each is **centred on the median row
centre** and sized by bisecting a per-axis half-extent until the target
fraction is reached — a port of
`benchmark/readbench/src/params.rs::window_for_target`, step for step,
including its ±10 %-of-target `approx` disclosure. `notes` carries the tag
(`bbox-1pct`, suffixed `-approx` when the target was not reachable on this
data) and the fraction the window actually achieved.

This replaces a construction that scaled the extent's **area** from its
**lower-left corner**. The two harnesses' labels then named different
queries: the database family achieved 0.49 %, 6.37 % and 22.1 % where the
format family achieved 1.00 %, 5.00 % and 25.0 %, while two places in this
repository asserted that the constructions matched
(`notes/benchmark-fairness-review-2026-09-22.md` §4.4). A corner window is
also one workload rather than a spatial sample, and it interacts with row
ordering.

**The window's z range is never narrowed**: every object is in range
vertically, so no system is ever tested against a z-restricted window,
whatever its mechanism could support.

The median centre is still recorded in the sidecar as `point_xy`, because
it is what the windows are built around. It is no longer asked as a query:
CJDB's Q3 is not reproduced, since a point query is a window query
(`notes/benchmark-queries.md`).

### The four `id-lookup` probes

`id-lookup` is measured at **four ids, one row each**: the ids at 10 %,
50 % and 90 % of the **canonical CityJSONSeq stream order**, plus one
**verified-absent** id. `notes` carries `id-10pct`, `id-50pct`,
`id-90pct` or `id-miss`, and each probe is cross-checked against the same
probe on the other systems, never against a different one — the miss
legitimately returns 0 where the hits return 1.

The rule is the format family's own
(`benchmark/readbench/src/params.rs::id_probes`, `ID_DECILES`/
`ID_MISS_TAG`), so a probe tag names the same construction in either
family: the id at `int(fraction × feature count)` of the feature stream,
and for the miss the 50 % id with a suffix, checked against **every**
CityObject id in the source rather than only the feature ids. A single
target would have made the published time a function of where that one id
happened to sit; the miss is the only position-free probe, and it is the
one that separates a store with an id index from one without.

Unlike the format family's probes, these carry no `substituted` flag.
There a probe had to exist in a CityGML artefact synthesised separately;
here every system is fed the same source file, so every id of that file
exists in every system by construction.

### The `append-object` file

`append-object` imports a **one-feature CityJSONSeq file derived from the
source**, written beside the params sidecar as
`<dataset>.append.city.jsonl` and described by the sidecar's `append`
block (path, suffix, object count, the source feature it was cut from, and
any reference it could not rewrite).

It is the source's **last** feature — its header line copied verbatim, so
the file declares the same CRS and `transform` the destination already
holds — with **every id it owns suffixed** `-appended`: the feature id,
every `CityObjects` key, and every `parents`/`children` entry between
them. The appended object is therefore a genuinely new object carrying the
_same_ geometry, which is what makes the row a measurement of adding an
object rather than of building a different one. A suffix that would
collide with an existing id is extended until it does not.

A **plain CityJSON** source yields no file: cutting one feature out of a
single document means re-indexing its shared `vertices`, which would make
the appended object this harness's construction rather than the dataset's
own. `append-object` is then recorded as `skipped:`.

### The `attr-filter` predicate

`attr-filter` filters a real **CityJSON attribute**, picked per dataset by
the same rule the format family uses (`citybench.params.HAND_PICKED`, a
port of `params.rs`): 3DBAG `b3_dak_type = 'slanted'`, Zurich
`class = 'BB01'`, Vienna `roofType = 'FLACHDACH'`, Ingolstadt
`klumMaterialClass = 'Wood'`, NYC `BIN = '1000000'`, Rotterdam
`TerrainHeight >= q0.75`. A dataset with no hand-picked entry falls back to
the string attribute whose most frequent value's share lands closest to
25 %, then to the alphabetically first numeric attribute at its 0.75
quantile, then to `skipped:`.

It previously filtered `object_type`, which is a reserved structural
column, not an attribute. That is both an unnatural query and the exact
predicate that made every FlatCityBuf row in the format family fall back to
a full walk, because FCB's B+-tree indexes only the `attributes` map
(review §0). The column is now recorded in the params sidecar along with
the predicate, the matched count and whether it was hand-picked.

## The ten read scenarios

Each system answers each scenario through its own natural mechanism — never a
hand-tuned shortcut, never a shape contrived to match another system's plan
(the design rule stated in `sql_citydb.py` and `sql_duckdb.py`).

Where CJDB's own queries return rows, so do these: **ids**, or whole
objects. The counts these rows once returned let a columnar reader answer
from metadata or from Parquet definition levels alone, which is a real
property worth measuring but not the same question a client asking for
objects poses. Every row-returning scenario materialises its rows inside
the timed window on every system.

| scenario                  | returns                  | common target                                                      | `duckdb-cityparquet`                                                                                                                                  | `cjdb`                                                                                                                                                                                                   | `3dcitydb`                                                                                                                                                                                                                                        |
| ------------------------- | ------------------------ | ------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `geometry-scan` | ids + native binary geometry | every object's id and every geometry it carries | `SELECT id, <every geometry_lod* column>` fetched to Arrow as WKB | `SELECT object_id, geometry` (the geometry JSONB), binary wire format | `objectid` + an array of the object's geometries, one per LoD, gathered from it and its boundary parts (Caveat 17), binary wire format |
| `count`                   | count                    | total CityObject count                                             | `SELECT count(*)` — answered from file metadata; caption it as such                                                                                   | `SELECT count(*) FROM cjdb.city_object`                                                                                                                                                                  | `count(*)` over `feature` with the CityObject predicate (Caveat 1)                                                                                                                                                                                |
| `bbox-query` (1/5/25 %) | ids + highest-LoD geometry | objects whose bbox intersects the window, each with its most detailed geometry | `bbox` STRUCT comparisons — **x/y only**; `coalesce` over the per-LoD WKB columns, most detailed first | `ground_geometry && ST_MakeEnvelope(...)` then the exact double-precision recheck (Caveat 12), GIST-indexed — 2D by storage (Caveat 3); the highest-LoD element of the geometry JSONB | envelope `&& ST_MakeEnvelope(...)` then the exact double-precision recheck (Caveat 12); the object's geometry at the highest `property.val_lod` (integer tier), gathered from it and its boundary parts (Caveat 17), via `LEFT JOIN LATERAL` |
| `attr-filter`             | ids                      | objects matching the per-dataset attribute predicate               | `WHERE "<col>" = ?` — a typed, flattened top-level column                                                                                             | `WHERE attributes ->> '<col>' = %s` — expression index on the attribute (see "Index sets")                                                                                                             | `property` join on `pr.name = %s AND pr.val_string = %s` (Caveat 13)                                                                                                                                                                              |
| `attr-range`              | ids                      | objects whose numeric attribute exceeds the threshold              | `WHERE "<col>" > ?` — DOUBLE column with row-group statistics                                                                                         | `WHERE (attributes ->> '<col>')::float > %s`                                                                                                                                                             | `property` join with `coalesce(val_double, val_int) > %s`                                                                                                                                                                                         |
| `attr-stats`              | `(min, max, sum, count)` | aggregate of `numeric_column`                                      | aggregates over the flattened top-level column                                                                                                        | aggregates over `(attributes ->> '<col>')::float8` — every row's JSONB unpacked and cast                                                                                                                  | EAV join `property`→`feature` on `name = col`, aggregating `coalesce(val_double, val_int)` (Caveat 13)                                                                                                                                            |
| `id-lookup` (×4 probes)   | the object               | the row for one probe id, materialised                             | `SELECT * WHERE id = ?` — the whole object row, geometry included; no index; bloom filters prune, the cost is the row-group decode (Caveats 21, 22)   | `SELECT * WHERE object_id = %s` (added btree) — the row includes the geometry JSONB                                                                                                                      | one row: `f.*` of the `feature` row `WHERE objectid = %s` (btree), plus one array per `property` column (`name`, every `val_*`) and an array of its geometries, one per LoD, gathered from it and its boundary parts, aggregated in scalar subqueries (Caveat 17)                         |
| `lod-query`               | ids + LoD 2.2 geometry   | objects carrying an LoD 2.2 geometry, each with that geometry (Caveat 9) | `SELECT id, geometry_lod2_2 WHERE geometry_lod2_2 IS NOT NULL`, fetched to Arrow inside the timed window; skipped on a dataset without LoD 2.2 (Caveat 15) | `SELECT object_id, jsonb_path_query_first(geometry, '$[*] ? (@.lod == "2.2")')` with `WHERE geometry @? '$[*] ? (@.lod == "2.2")'` — the `@?` operator, which uses cjdb's GIN(`geometry`) index; the `jsonb_path_exists` function form does not | `f.objectid` and the object's tier-2 geometry, gathered from it and its boundary parts through the `property` rows with `val_lod = '2'` and joined to `geometry_data` — the importer stores only the integer LoD tier (`docs/3dcitydb-v5-schema.md`, "LoD value format"; Caveat 17) |
| `parts-per-building`      | one row per Building     | how many parts each Building has, **childless Buildings included** | `SELECT id, coalesce(len(children), 0) WHERE object_type = 'Building'` — a stored array, no join                                                      | `LEFT JOIN city_object_relationships cor ON cor.parent_id = co.id`, `count(cor.child_id)`, `GROUP BY co.object_id`                                                                                       | `feature parent LEFT JOIN property LEFT JOIN feature child`, the CityObject predicate on the child, `count(child.id)`                                                                                                                             |
| `parts-per-building-join` | one row per Building     | the same question in the shape a normalised store must use         | `LEFT JOIN (SELECT unnest(parents) AS parent, id … WHERE object_type = 'BuildingPart') p ON p.parent = b.id … GROUP BY b.id`                          | —                                                                                                                                                                                                        | —                                                                                                                                                                                                                                                 |

`bbox-query` produces one row per window, tagged `bbox-1pct`, `bbox-5pct`
or `bbox-25pct` in `notes` alongside the achieved fraction; `id-lookup`
produces one row per probe, tagged `id-10pct`, `id-50pct`, `id-90pct` or
`id-miss`. Every read scenario is measured under **both** thread
configurations, and `notes` carries `threads=single` or `threads=parallel`.
A scenario the dataset cannot answer (no numeric attribute for `attr-stats`
or `attr-range`, no usable attribute for `attr-filter`, no feature to cut
an append file from) is recorded as `skipped: ...`, not as an error.

`lod-query` returns **rows carrying the geometry on every system**, which
is what makes the three times comparable: each row is the object's
identifier and its LoD 2.2 geometry — DuckDB's `geometry_lod2_2` WKB,
cjdb's LoD 2.2 element of the `geometry` JSONB array, 3DCityDB's
tier-2 geometry, gathered from the object and its boundary parts and
collected into one geometry per object (Caveat 17), so its row count is
one per CityObject even when several `property` rows match. On 3DBAG the objects carrying an LoD 2.2 geometry are the
`BuildingPart`s, not their parent `Building`s; all three systems answer at
CityObject grain, so they agree on which objects those are.

`parts-per-building-join` is a **control, not a comparison**: the same
question as `parts-per-building` asked the way a normalised store must ask
it, run on DuckDB alone so the cost of the join is visible against the
natural form on the same engine and the same data. cjdb and 3DCityDB have
only the join form, which _is_ their `parts-per-building`. The two DuckDB
forms are asserted to return identical row sets
(`tests/test_duckdb_cp.py`); publishing a ratio between them otherwise
would compare different result sets.

## The write tier

Four scenarios, run **last**, once, under `threads=single`, and reported as
the write-tier rows of the `databases` figure, below the reads. They are
**different operations, not one scale** (Caveat 19).

| scenario        | `duckdb-cityparquet` / `-writeback`                                                                                                                       | `cjdb`                                                                                              | `3dcitydb`                                                                                                                                                                        |
| --------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `attr-add`      | `ALTER TABLE pkg.building ADD COLUMN footprint_area DOUBLE`, then `UPDATE … SET footprint_area = ST_Area(geometry_lod0_0) WHERE object_type = 'Building'` | Q6 verbatim: `jsonb_set(attributes::jsonb, '{footprint_area}', to_jsonb(ST_Area(ground_geometry)))` | `INSERT INTO citydb.property (feature_id, name, datatype_id, val_double) SELECT id, 'footprint_area', <double>, ST_Area(envelope) FROM feature WHERE objectclass_id = <Building>` |
| `attr-update`   | `UPDATE … SET footprint_area = footprint_area + 10.0`                                                                                                     | Q7 verbatim: the same `jsonb_set` over `(attributes->>'footprint_area')::float + 10.0`              | `UPDATE citydb.property SET val_double = val_double + 10 WHERE name = 'footprint_area'`                                                                                           |
| `attr-delete`   | `ALTER TABLE pkg.building DROP COLUMN footprint_area`                                                                                                     | Q8 verbatim: `jsonb_set_lax(…, NULL, true, 'delete_key')`                                           | `DELETE FROM citydb.property WHERE name = 'footprint_area'`                                                                                                                       |
| `append-object` | `PRAGMA insert_cityjsonseq('pkg', '<dataset>.append.city.jsonl')` on the loaded package                                                                   | `cjdb import -f <dataset>.append.city.jsonl`, an external process                                   | `citydb-tool import cityjson <dataset>.append.city.jsonl`, an external process in a container                                                                                     |

`append-object` is catalogue B18 — "add one new building, with its parts
and geometry, to the dataset" — and it is deliberately **each system's own
importer**, not three hand-written INSERTs. That is the point of the row:
the three importers do different amounts of work for the same appended
object. `insert_cityjsonseq` routes each object to its module table and
re-derives `feature_id`, the reciprocal hierarchy and `bbox`; `cjdb
import` derives a footprint per object and writes its relationship rows
and a `cj_metadata` row; `citydb-tool import` writes a `feature` row per
semantic boundary surface as well as per CityObject, plus its `property`
and `geometry_data` rows. **`result_count` is the same number on all four
tags by definition** — the CityObjects in the appended file, the Building
plus its parts — and what each importer actually wrote is measured and
stamped into `notes` (`city-object-rows-added` on cjdb,
`feature-rows-added` on 3DCityDB) rather than left to be assumed.

> **`append-object` on `duckdb-cityparquet` depends on which build of the
> DuckDB CityJSON extension answers.** A bare `LOAD cityjson` answers with
> the installed community build, which refuses `PRAGMA insert_cityjsonseq`
> into a package loaded with `cityparquet_read`: the insert fails with a
> `BinderException` saying a column of the loaded package "cannot be
> widened" to the incoming type (the community build tested rejected
> `material_lod2_0`). The committed run's two DuckDB `append-object` rows
> are that error. The fix — an insert that keeps the package's column
> types, including the LoD 0 `GEOMETRY` column — is in the submodule
> `lib/duckdb-cityjson` (commit `062279e`), so the harness loads an
> explicitly chosen build by path rather than by name (see
> [Which build of the DuckDB CityJSON extension](#which-build-of-the-duckdb-cityjson-extension)).
> The harness does **not** work around a refused insert: a system that
> cannot answer is a result, recorded as `error: BinderException`, and
> `citybench run` exits non-zero.

CityParquet has no in-place update path — a Parquet file's smallest
rewritable unit is a column chunk — so the comparable operation is the one
the DuckDB CityJSON extension's package model offers. `PRAGMA
cityparquet_read` loads the package into DuckDB tables (untimed setup;
`read_parquet` would not do, because only the pragma recovers each file's
footer and so its CRS), the statements above mutate them, and
`cityparquet_write` writes the package back. **Both costs are published**:
`duckdb-cityparquet` times the in-engine mutation alone and
`duckdb-cityparquet-writeback` additionally times the package write.

Mechanics, all of which change what the numbers mean:

- **No `EXPLAIN (ANALYZE)` re-run**, so write rows carry no
  `server_time_*` block. `EXPLAIN ANALYZE` on an INSERT/UPDATE/DELETE _executes_
  it: reusing the read path would have applied Q6 twice, incremented Q7 by
  20 rather than 10, and rewritten half a million cjdb tuples a second time
  per sample.
- **No discarded warm-up.** A warm-up here would be a real mutation. Each
  sample after the first is preceded by an **untimed reset** that restores
  the state the scenario expects (`attr-add`'s INSERT is not idempotent;
  DuckDB's `ADD COLUMN` errors the second time), so every timed sample
  measures the same work.
- **`append-object`'s reset removes what the importer added, and runs once
  more after the last sample**, so the tier leaves the databases in the
  state every other row was measured against. On CityParquet that is
  `PRAGMA cityparquet_delete` on the suffixed ids; on both PostgreSQL
  systems it is a **watermark** — each affected table's `max(id)` read
  untimed beforehand, then `DELETE … WHERE id > watermark` in
  foreign-key order — because each importer also writes rows carrying none
  of the suffixed ids (cjdb's relationship and `cj_metadata` rows,
  3DCityDB's boundary-surface `feature` rows). Leaving cjdb's
  `cj_metadata` row behind would be worse than untidy: cjdb's importer
  prompts on stdin when a file of that name was imported before, which in
  a benchmark run is a hang rather than a question.
- **cjdb's importer runs its own post-import step on every append.**
  `post_import()` calls `index_attributes()`, whose
  `get_attributes_and_types` is a `SELECT DISTINCT ON (type) … ORDER BY
type, id DESC` over the whole `city_object` table, and then issues its
  `CREATE INDEX IF NOT EXISTS` statements (no-ops after the first import).
  That sampling query is inside the timed window, it is proportional to
  the table rather than to the one appended feature, and it is genuinely
  what `cjdb import` costs — disclosed, not excluded.
- **Two of the four rows time an external process, launcher included.**
  `cjdb import` pays `uv`'s resolution and a Python start; `citydb-tool
import` pays a container start and a JVM start, which for a one-feature
  file is a large share of the number. Neither is subtracted. A
  non-mutating `--help`/`--version` invocation runs **untimed** before the
  first sample so a cold image or resolve does not land on sample 1 —
  that warms the launcher, never the mutation. Those two rows also carry
  **no `peak_working_mem_bytes`**: the work happens in a process this harness
  starts and waits on, not in the PostgreSQL backend the other rows
  sample.
- **`VACUUM ANALYZE` runs after the last sample of each PostgreSQL write
  scenario**, never inside a timed window, so the dead tuples `attr-add`
  leaves behind are not charged to `attr-update`.
- **The order is load-bearing**: `attr-add` creates the attribute
  `attr-update` increments and `attr-delete` removes. The tier runs after
  every read row of both thread configurations, because its mutations leave
  bloat a later read pass would measure as the steady state.
- **`result_count` is rows touched**, from the cursor's rowcount. DuckDB's
  `DROP COLUMN` reports none, so `attr-delete`'s CityParquet count is
  _defined_ as the Building row count the other two systems' statements
  touch. That is a definition, not a measurement.
- **The area expressions differ, inherited from the CJDB paper.** Its own
  Q6 computes `ST_Area(ground_geometry)` — a footprint area — on cjdb
  against `ST_Area(envelope)` — an envelope area, an upper bound on the
  footprint's — on 3DCityDB. This harness keeps both rather than measuring
  a query CJDB never published. CityParquet uses `ST_Area` over its LoD0
  footprint where DuckDB's spatial extension and the column are available,
  and the `bbox` rectangle's area otherwise; which one was used is stamped
  into `notes`.
- **The LoD 0 footprint is the source's own.** The package is the format
  benchmark's, written with `--no-lod0`, so `geometry_lod0_0` holds only
  LoD 0 geometry the source carries. On the 3DBAG slice every `Building`
  has a source LoD 0 footprint and no `BuildingPart` has one; the
  statement's `object_type = 'Building'` touches exactly the Buildings,
  and their areas are the ones a package with LoD 0 synthesis gives
  (checked on 3DBAG tile 9-284-556: 1,110 Buildings, the same area sum
  either way; synthesis adds LoD 0 only to the 1,111 BuildingParts). A
  Building without LoD 0 in a dataset that has the column would get a NULL
  area, where cjdb's Q6 sets the whole `attributes` document NULL for a
  NULL `ground_geometry` (bullet below) and 3DCityDB uses the importer's
  `envelope`. A dataset with no LoD 0 at all, such as
  Rotterdam under `short` and `smoke`, has no `geometry_lod0_0` and takes
  the `bbox`-rectangle branch. All three systems still touch every
  Building row.
- **cjdb's Q6 is strict.** `jsonb_set` returns NULL for a NULL argument, so
  a Building whose `ground_geometry` is NULL has its whole `attributes`
  document set to NULL. On 3DBAG the NULL footprints are all BuildingParts
  (review §5), which Q6's `type = 'Building'` excludes; on another corpus it
  could bite. Disclosed rather than guarded, because "verbatim" was the
  point.
- **One deviation from the paper's text**: its trailing `::json` cast is
  dropped, because cjdb 2.2.0 stores `attributes` as `jsonb` and PostgreSQL
  registers no assignment cast from `json` to `jsonb`. The
  `attributes::jsonb` the paper writes is kept and is a no-op here.

### Which build of the DuckDB CityJSON extension

The write tier runs through the extension's package model
(`PRAGMA cityparquet_read`, `PRAGMA insert_cityjsonseq`,
`cityparquet_write`), so which build answered is part of the result
(`src/citybench/extension.py`). The DuckDB systems load one explicitly
chosen file, by path, never `LOAD cityjson` by name:

1. the path given to `citybench --duckdb-cityjson-extension`;
2. else the path in `CITYBENCH_DUCKDB_CITYJSON_EXTENSION`;
3. else the submodule's local release build,
   `lib/duckdb-cityjson/build/release/extension/cityjson/cityjson.duckdb_extension`
   (`just -f lib/duckdb-cityjson/justfile build`).

When none of them names an existing file, a run that includes a DuckDB
system refuses to start, before any database is started. A local build is
unsigned, so every DuckDB connection is opened with
`allow_unsigned_extensions`. A C++ DuckDB extension loads only into the
exact DuckDB version it was built for, so `pyproject.toml` pins the Python
`duckdb` package to 1.5.4, the version the submodule builds against.

The manifest's `versions` block names the build that answered:
`duckdb-cityjson` (the extension's self-reported version),
`duckdb-cityjson-path`, `duckdb-cityjson-commit` (the submodule's checked-out
commit, with `+dirty` when its working tree differs; stated only for a build
inside `lib/duckdb-cityjson`) and `duckdb-cityjson-sha256` (the file's
digest, which identifies the build exactly).

## Mapping to the CJDB paper

CJDB's own benchmark ([Appendix A, pp. 16-17](https://arxiv.org/pdf/2307.06621#page=16))
runs eight queries. Seven of this harness's scenarios correspond to one
each; the eighth, Q3, is **not reproduced**. The differences are stated
here rather than left for a reader to discover.

| CJDB                  | ours                 | how ours differs                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| --------------------- | -------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Q1 `h_dak_max > 20`   | `attr-range`         | The threshold is the column's own 0.8 quantile rather than a literal 20, so the selectivity carries across datasets; on 3DBAG the two land within a rounding error of each other (20.05 against 20). Q1 is implicitly Building-grained, and so is ours: the attribute exists only on Buildings — which means the 20 % is **20 % of the rows carrying the attribute**, and the CSV's `selectivity` column, whose denominator is every CityObject, therefore reads about 0.1 on 3DBAG.                                                                                                                |
| Q2 bbox               | `bbox-query`         | **CJDB uses `ST_Contains(window, ground_geometry)` — containment. This harness uses `&&` overlap on every system**, which is what a bbox index answers natively on all three. Ours returns the count rather than ids plus footprints, and does not restrict to `type = 'Building'`, so it asks about every CityObject in the window. The containment fetch was dropped with the catalogue review: how a window query is _composed_ is not what this benchmark is comparing.                                                                                                                         |
| Q3 point              | —                    | **Not reproduced: a point query is a window query** (`notes/benchmark-queries.md`). Q3 is a bbox overlap against a degenerate window, answered by the same index and the same code path as Q2 on all three systems, so a separate row would have measured the same mechanism twice. The median row centre it would have used is still recorded as `point_xy`, because the windows are built around it.                                                                                                                                                                                              |
| Q4 parts per building | `parts-per-building` | Adapted to cjdb 2.2.0's schema, where `city_object_relationships.parent_id` is the integer `city_object.id`, not the textual `object_id`. Childless Buildings are kept, as Q4's `LEFT JOIN` keeps them. `parts-per-building-join` is an extra DuckDB-only control with no CJDB counterpart.                                                                                                                                                                                                                                                                                                         |
| Q5 LoD 1.2            | `lod-query`          | Asks for **LoD 2.2**, which the benchmark's 3DBAG slice carries (LoD 0, 1.3 and 2.2; no 1.2), and returns **ids plus that geometry**, where Q5 returns ids: the catalogue's own definition is "retrieve all buildings having a specific LoD geometry", and a projection of ids alone is answerable from one column's definition levels on a Parquet reader. cjdb uses the `@?` jsonpath operator rather than the paper's `@>`, because only the operator form cooperates with cjdb's own GIN index. 3DCityDB must test `val_lod = '2'` — its importer truncates the fractional tier, so its "LoD 2" covers every CityJSON 2.x (Caveat 17) — and joins `geometry_data` so its row carries a geometry like the other two. |
| Q6 add attribute      | `attr-add`           | See the write tier above: `::json` dropped; envelope-versus-footprint area inherited.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| Q7 update attribute   | `attr-update`        | None.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| Q8 delete attribute   | `attr-delete`        | None on the PostgreSQL side.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |

Scenarios with **no CJDB counterpart**, and what each is for:
`geometry-scan` (a whole-geometry scan; see Caveat 18 for why it is fairer
than the row it replaces but still not neutral), `count` (a legitimate
query answered from metadata on a columnar reader — caption it that way),
`attr-stats` (a single-column aggregate, the cleanest row in the set),
`attr-filter` (equality on an indexable attribute), `id-lookup` (single-object
materialisation at four stream positions plus a miss, which CityParquet
loses heavily), `append-object` (adding an object through each system's own
importer) and `parts-per-building-join`.

Dropped after the author's review of the catalogue, and **not** to be
reinstated without a reason recorded there: the containment fetch and the
point query (they measure how a query is composed, not what a store can
do), single-attribute projection (close to a full read, and not a real
workload), per-LoD geometry projection, semantic-surface presence, and
geometry-changing updates.

## Fairness controls

### Engine parity

Both PostgreSQL systems run the same engine build line. The images are pinned
in `src/citybench/lifecycle.py` and `docker/compose.yml`:
`docker.io/postgis/postgis:16-3.4` for cjdb and
`docker.io/3dcitydb/3dcitydb-pg:16-3.4-5.1.2-alpine` for 3DCityDB. The
3DCityDB tag is pinned because the untagged `5-alpine` variant resolves to a
different PostgreSQL and PostGIS major version. `docs/cjdb-schema.md` and
`docs/3dcitydb-v5-schema.md` record `server_version` 16.4 and
`PostGIS_Version()` `3.4 USE_GEOS=1 USE_PROJ=1 USE_STATS=1` on both servers
at capture time. Each run reads them again from the live servers: the
manifest's `servers` block records, per PostgreSQL system, `postgresql`
(`SHOW server_version`) and `postgis` (`postgis_lib_version()`); for
3DCityDB also `3dcitydb` (`citydb_pkg.citydb_version()`) and `citydb-tool`
(the tool image's own `--version`); and under `images` the container
engine's digest of the two PostgreSQL images and the `citybench/citydb-tool`
image. `versions` records DuckDB, the DuckDB CityJSON extension build, and
cjdb's upstream version with a patch marker; `patches.cjdb` records the
patch file `vendor/cjdb/ground-surfaces-tie.patch` and its SHA-256. The
container engine and its version are under `isolation.containers.engine`.

### Tuning and parallelism

`docker/postgresql.conf` is mounted read-only, as the same file, into both
containers. Stock PostgreSQL defaults (128 MB `shared_buffers`) would make
either database a strawman. The manifest's `pg_settings` block records the
values the committed run read back with `current_setting()`, in PostgreSQL's
own memory units, which are binary (1 GB = 1024 MB):

| setting                | cjdb  | 3dcitydb |
| ---------------------- | ----- | -------- |
| `shared_buffers`       | 8GB   | 8GB      |
| `effective_cache_size` | 24GB  | 24GB     |
| `work_mem`             | 256MB | 256MB    |
| `random_page_cost`     | 1.1   | 1.1      |
| `max_parallel_workers` | 16    | 16       |

`max_parallel_workers_per_gather` is deliberately absent from that table
and from the manifest's `pg_settings` block: it is set per run
configuration on the benchmark session, so reading it back from a fresh
connection would report the file's value and contradict the `execution`
block. That block records it while each configuration is live, along with
`max_worker_processes` — the cluster-wide pool that bounds how many workers
a query can _actually_ get, whatever the per-gather cap asks for.

### The two thread configurations

Every read scenario is measured **twice**, and both are published. `notes`
carries `threads=single` or `threads=parallel`; the manifest's `execution`
block records both settings and which is primary.

|                                       | DuckDB              | PostgreSQL                            |
| ------------------------------------- | ------------------- | ------------------------------------- |
| **`single`** — the **primary** figure | `SET threads TO 1`  | `max_parallel_workers_per_gather = 0` |
| `parallel` — a disclosed second pass  | `SET threads TO 16` | `max_parallel_workers_per_gather = 8` |

`single` is primary because it is the condition under which the two engines
are asked for the same amount of CPU, and because it matches the format
harness, which pins every reader to one thread. The committed run gave
DuckDB 16 threads against a PostgreSQL with parallel query disabled, and
the resulting advantage was not spread evenly: it concentrated on the
headline rows (7.6x on the whole-table scan, 5.5x on the retired
`lod-extract`, 5.4x on the retired `semantic-surface`) and left the
two-to-three-order-of-magnitude wins untouched (`notes/benchmark-fairness-review-2026-09-22.md` §4.3).

Under `parallel`, `parallel_setup_cost` and `min_parallel_table_scan_size`
stay at their defaults: raising the worker cap is a resource decision,
lowering the planner's thresholds would be tuning the query. DuckDB also
runs with `SET memory_limit = '32GB'` and
`SET enable_geoparquet_conversion = false` (Caveat 18) under both.

The write tier runs **once**, under `single`, after every read row.

`track_io_timing = on` makes buffer timing available to
`EXPLAIN (ANALYZE, BUFFERS)`. `max_connections = 20`.

### Container engine

Every container call goes through `src/citybench/engine.py`. The engine is
chosen at run time by a cascade: Apple `container` first, then `docker`, then
`podman`, taking the first that is installed and responding (`container
system status`, `docker info`, `podman info`). `CITYBENCH_CONTAINER_ENGINE`
or `citybench --container-engine <name>` overrides the cascade; an override
that does not respond fails the run. The priority list is one tuple
(`ENGINE_PRIORITY`). `uv run python -m citybench.engine manifest` prints what
the cascade picks on the current host.

Capabilities are probed per engine, from its `run --help` and the host
platform, not assumed: CPU and memory limits, `/dev/shm` size, the cpuset
flags, a host network for the `citydb-tool` container, and whether a
container process's `/proc/<pid>/status` is visible on the host. The run
manifest records the engine's name, version and every capability under
`isolation.containers.engine`, each as `applied` or `not applied: <reason>`.
On Apple `container`, the cpuset flags, the host network and host-visible
`/proc` are not applied: `citydb-tool` then reaches PostgreSQL at the
database container's own address. Each database is published on a free
`127.0.0.1` port chosen by the harness and passed explicitly
(`-p 127.0.0.1:<port>:5432`), because Apple `container` has no random-port
form. Apple `container`'s bind mounts also refuse the `chown`/`chmod` the
PostgreSQL entrypoint applies to its data directory, so on that engine
`PGDATA` is set to `/var/lib/postgresql/pgdata` inside the container's own
filesystem and is removed with the container, rather than bound to a host
directory (`bind_mount_ownership` among the recorded capabilities).

Both PostgreSQL images (`postgis/postgis:16-3.4` and
`3dcitydb/3dcitydb-pg:16-3.4-5.1.2-alpine`) publish `linux/amd64` only, so
the database containers are started with `--platform linux/amd64`
(`DB_PLATFORM` in `lifecycle.py`); on an arm64 host they run under
emulation (Rosetta on Apple `container`).

cjdb 2.2.0 declares SQLAlchemy without an upper bound and fails on
SQLAlchemy 2.1, so every `cjdb` invocation is launched as
`uv run --with <patched cjdb> --with 'sqlalchemy<2.1'` (`CJDB_WITH_PINS` in
`systems/cjdb.py`).

On macOS both Apple `container` and `docker` run Linux in a virtual machine,
so numbers measured there test the harness and are not citable. The
benchmark host runs rootless `podman` on Linux.

### Resource limits

Each PostgreSQL container is limited to 16 CPUs and 32 GiB of memory
(34.4 GB), with a 2 GiB (2.1 GB) `/dev/shm`: podman's `g` suffix is binary,
so these limits are spelt in its unit and converted to decimal GB in brackets. The isolated lifecycle (`lifecycle.isolated_databases`)
passes these as `run --cpus 16 --memory 32g --shm-size 2g` to the chosen engine;
`docker/compose.yml` declares the same limits in `deploy.resources.limits`.
`docs/3dcitydb-v5-schema.md` ("Resource limits — measured, not assumed")
records that on containers started from `compose.yml` podman-compose applied
`--cpus 16.0 -m 32g`, that `cpu.max` and `memory.max` read 16 cores and
32 GiB inside both containers, and that a CPU-bound load was throttled
symmetrically. No equivalent measurement is recorded for the isolated
containers.

`nproc` inside the containers reports the host's full core count, or the
pinned node's (see "Host isolation"), because it reflects CPU affinity rather
than the bandwidth quota. `citydb-tool import
cityjson` sizes its thread pool from `nproc`, and each thread opens a
connection, so the adapter passes `--threads=4` to stay within
`max_connections`.

### Host isolation

The citable runs come from a shared two-socket host without root, so the
harness isolates what an unprivileged user can, records what it applied in
the manifest's `isolation` object, and never aborts a run because a step
could not be applied (`src/citybench/isolation.py`):

- **NUMA node.** `--numa-node` (env `BENCH_NUMA_NODE`; recipe parameter
  `NUMA_NODE`) takes a node id, `auto` or `off`. `auto` picks the node with
  the most `MemFree` in `/sys/devices/system/node/node*/meminfo` at run
  start, ties going to the lowest id; without NUMA information on Linux it
  is node 0 with every CPU the process may use.
- **Client and DuckDB.** The citybench process pins itself to the node's
  cores with `os.sched_setaffinity` before any container starts. DuckDB runs
  inside that process and the `cityparquet` reader child inherits the mask,
  so both are pinned. Memory is not bound: `--membind` needs a `numactl`
  re-exec, which the harness does not do, so Linux's first-touch allocation
  favours the node's memory without enforcing it.
- **PostgreSQL containers.** When the run starts the containers
  (`--data-root`), each container `run` receives `--cpuset-cpus=<node cores>
  --cpuset-mems=<node>`, but only if the user's cgroup v2 delegation
  (`/sys/fs/cgroup/user.slice/user-<uid>.slice/user@<uid>.service/cgroup.controllers`)
  includes `cpuset`; if podman still rejects them, the containers are
  started without them and the record says so. Otherwise the flags are omitted and the record says
  `not applied: cpuset controller not delegated to the user`; podman is
  still launched from the pinned client, so the container processes are
  expected to inherit its affinity, which no cgroup enforces. The
  containers share the node's cores with the client because the matrix runs
  one system at a time. Containers started outside the run (the fixed-port
  recipes) are not pinned, and the `citydb-tool` import container only
  inherits the client's affinity.
- **Load gate.** Before each system's cell the harness reads `/proc/loadavg`
  and compares the node's share of the one-minute load,
  `load1 × node cores / total cores`, against `--max-load` (`auto`, the
  default, is half the node's cores; `off` disables it). Above it, the
  harness waits in 10 s steps up to `--max-load-wait-s` (default 600) and
  logs each wait; if the node is still loaded, the cell proceeds and its
  rows carry the `busy` token in `notes` (space-separated, like the other
  citybench note tokens). The load (`load1`, runnable tasks) and
  `MemAvailable` are recorded before and after every cell, and each cell's
  `load1_max`, `runnable_max` and `mem_available_min_bytes` are in
  `isolation.load.cells`, with the run's maxima beside them. Samples inside
  a cell are not recorded individually; the adapters' repetition loops are
  left untouched. `MemTotal` and `MemAvailable` at run start are in
  `isolation.host_memory`.
- **Memory cap.** `--memory-max` is recorded but not applied to this family:
  the client is not re-executed under `systemd-run`, and the containers keep
  their podman `--memory` limit above. `bench-run` passes the suite's ceiling
  (64,000,000,000 decimal bytes under the `full`, `quick` and `short`
  profiles, none under `smoke` or with `--memory-max off`), so the manifest's
  `isolation.requested.memory_max` holds what the read families applied and
  `isolation.memory_max` says that this family did not apply it.

Samples of one cell run back to back, and cells of different systems are
never interleaved. On a host that is not Linux (a development laptop) every
step is recorded as `not applied: not Linux` and the load as
`not applied: no /proc`. Binding memory to the node and capping the
client's memory with a cgroup need root or a `numactl` or `systemd-run`
re-exec, and are not done.

### Index sets

Every queried predicate is indexed where the system supports it, and index
sizes are reported separately. Each scenario's index requirement is checked
against what each system builds unasked, and only what is missing is added; a
duplicate index has no query benefit but inflates `size_bytes`. The
attribute indexes depend on the dataset's predicates, so `build_indexes()`
builds them after the import and before `VACUUM ANALYZE`; their build time is
the manifest's `ingest.index_build_s`, apart from the import time.

- **cjdb: one added index**, `ix_co_object_id` = btree(`object_id`)
  (`sql_cjdb.index_ddl`). cjdb's own unique index is the composite
  `(cj_metadata_id, object_id)`; every row shares one `cj_metadata_id`, so
  that index cannot serve a bare `WHERE object_id = ?` probe efficiently.
  The docstring of `sql_cjdb.index_ddl` records the `EXPLAIN` comparison and
  the default indexes that already cover the other scenarios: GIST on
  `ground_geometry` (cjdb creates two), btree(`type`), GIN(`geometry`), and
  the relationship btrees.
- **cjdb: attribute expression indexes, per dataset**
  (`sql_cjdb.attribute_index_ddl`). cjdb stores attributes as one JSONB
  document and indexes none of it, so the harness adds btree indexes on the
  exact expressions the queries evaluate: `(attributes ->> '<col>')` for an
  equality `attr-filter`, and `((attributes ->> '<col>')::float8)` for a
  numeric bound (`attr-filter`'s `>=` form and `attr-range`; one index when
  both use the same attribute).
- **3DCityDB: one attribute index added**, `property_name_numval_inx` =
  btree(`name`, `COALESCE(val_double, val_int::float8)`)
  (`sql_citydb.attribute_index_ddl`), built only when the dataset has a
  numeric predicate. citydb-tool already builds btree(`name`) and partial
  btrees on `val_string`, `val_double` and `val_int`, which serve the equality
  `attr-filter` (`name = ? AND val_string = ?`); the numeric bound compares
  `coalesce(val_double, val_int)`, which none of them covers.
- **CityParquet: nothing added.** Selective access comes from the row-group
  statistics in the footer and the Bloom filters inside the files.
- **3DCityDB structural indexes: none added.** `citydb-tool import cityjson` creates
  3DCityDB's content indexes during import, and `citydb index create` is a
  no-op afterwards (`docs/3dcitydb-v5-schema.md`, "Index coverage";
  `sql_citydb.index_ddl` docstring). The CityObject predicate is resolved
  once per ingest to a static `objectclass_id IN (...)` list
  (`sql_citydb.resolve_cityobject_class_ids`), which is sargable against
  `feature_objectclass_inx`.

`<dataset>.indexes.sql` records both the DDL this harness added and a live
`pg_indexes` dump of each PostgreSQL schema taken at run time (`pg.dump_indexes`),
so the complete index set each system queried against is auditable. The
committed 3DBAG file lists 15 cjdb indexes and 59 3DCityDB indexes, from a run
before the attribute indexes existed.

`EXPLAIN` on the Rotterdam (Delfshaven) databases confirms the plans use the
intended indexes. Rotterdam's `attr-filter` is the numeric bound
`TerrainHeight >= 2.45`, so the equality forms were not exercised there:

| Scenario | cjdb | 3DCityDB |
|---|---|---|
| `attr-filter`, `attr-range` | Bitmap Index Scan on `ix_co_attr_num_terrainheight_…` | Bitmap Index Scan on `property_name_numval_inx`, `Index Cond` on `name` and the `COALESCE` |
| `id-lookup` | Index Scan using `ix_co_object_id` | Index Scan using `feature_objectid_inx` |
| `bbox-query` | Index Scan using `city_object_ground_gix` (GiST) | Bitmap Index Scan on `feature_envelope_spx` (GiST) |

### `VACUUM ANALYZE`

Both PostgreSQL adapters end `ingest()` with `VACUUM ANALYZE` over every table
in their schema (`pg.vacuum_analyze`) before any timed scenario runs.

### The warm protocol

Every `run()` performs **one discarded warm-up** call followed by `repeat`
timed samples of the same query; `--repeat` defaults to **25**. A cell's
samples run back to back, not interleaved across systems. The format
family's optional cell time budget (`--cell-budget-s`, `--min-repeat`) is
not applied to the database family: every read cell takes exactly
`--repeat` samples.

- Each row reports the seven-column timing block shared with the format
  harness: `time_mean_s` (**arithmetic mean**), `time_std_s` (**population
  standard deviation**), `time_median_s`, `time_min_s`, `time_max_s`,
  `time_q1_s` and `time_q3_s` (`report.py`, `stats.py`), all to six decimal
  places. The median and quartiles interpolate linearly at position
  p·(n−1) on the sorted samples. The raw samples are in `raw_time_samples_s`.
  Every row carries both the mean and the median; the summaries
  (`just bench-summary`) report the median by default, with the
  interquartile range (`time_q1_s` to `time_q3_s`) as its spread, and the mean
  ± the standard deviation under `--statistic mean`.
- The PostgreSQL adapters time each sample from just before the query is sent
  to just after every row has been fetched. After each timed execution the
  same query runs again, untimed, under `EXPLAIN (ANALYZE, BUFFERS, FORMAT
JSON)` to obtain the `server_time_*` block (Caveat 4).
- `duckdb-cityparquet` times in-process. For a scenario that returns rows
  the result is materialised **inside** the timed window, to **Arrow**
  (`to_arrow_table()`), not to Python objects — a Python-object fetch of a
  row-returning scenario was measured at about 72 µs/row on `SELECT *`,
  which would make the row the client's number rather than the engine's.
  `notes` carries `fetch: arrow`. The PostgreSQL adapters already read
  every row to exhaustion inside their own timed window, so no system wins
  by handing back a lazy cursor.
- The native reader starts a fresh child process per sample, and the harness
  uses the child's own reported elapsed time, so process start-up is
  excluded.
- **Write rows follow a different protocol** — no warm-up, an untimed reset
  before each sample, and no `EXPLAIN (ANALYZE)` re-run. See "The write
  tier".

### Count cross-check

The count cross-check is the harness's main defect detector
(`citybench.runner`): every scenario's result count is compared across every
system that answered it, for the same parameters. `skipped:` and `error:`
rows carry no count and are excluded from the comparison.

A disagreement is always described in `notes` as
`count-mismatch: <system=count ...> spread=<(max-min)/max>`. What differs
is the **status**:

| relative spread                                          | `status`       | run outcome                                            |
| -------------------------------------------------------- | -------------- | ------------------------------------------------------ |
| systems agree                                            | `ok`           | —                                                      |
| ≤ the tolerance (default **0.1 %**, `--count-tolerance`) | `ok-deviation` | the run continues                                      |
| above the tolerance                                      | `mismatch`     | `citybench run` exits non-zero; the row is not citable |
| identifier sets differ (see below)                       | `id-mismatch`  | `citybench run` exits non-zero; the row is not citable |

The tolerance exists because the bbox counts on 3DBAG differ by
0.03-0.04 % for reasons that are **properties of the compared systems, not
of the query**, and whose mechanism is established down to the object
(Caveats 11 and 12, and `notes/benchmark-fairness-review-2026-09-22.md`
§5). Every system runs its own idiomatic predicate, which is the point;
a binary pass/fail over three objects in 221,005 told a reader nothing they
could act on, while the decomposition does. Above the tolerance the check
still fails the run — it remains the main defect detector, not a
formality. The tolerance is recorded in the manifest's `count_check` block
alongside what each status means.

`citybench smoke` returns the run's own non-zero exit, so any `error`,
`mismatch` or `id-mismatch` row fails it, and it then checks the CSV's
**status** column for `mismatch` and `error`. It deliberately does not
search `notes` for the text `count-mismatch`: an `ok-deviation` row keeps that text, because the
decomposition is the row's value.

Each expansion of a scenario is checked **against itself**: the 5 % window
against the 5 % window, the `id-miss` probe against the `id-miss` probe.
The miss legitimately returns 0 where the three hits return 1, and folding
the four probes into one row would have made that read as a mismatch.

The check compares answers, not work: it cannot detect a system that returns
the right count without doing the work the scenario is meant to measure
(Caveat 15).

### Identifier-set cross-check

Equal counts do not make equal answers, so the scenarios that return
object rows — `bbox-query` (each window), `attr-filter`, `attr-range`,
`lod-query` and `id-lookup` — are also compared on **which** objects came
back (`citybench.identity`). After a scenario's timed samples, each SQL
system that answered runs the scenario's SQL once more, **untimed**
(`verify_rows`); nothing inside the timed window changes. The native-reader
systems have no `verify_rows` and are left out of this check.

Each schema returns the object's CityJSON identifier under its own name:
the package's `id`, cjdb's `city_object.object_id`, and 3DCityDB's
`feature.objectid`, which `citydb import cityjson` fills with the CityJSON
identifier. The row scenarios return it as their first column; `id-lookup`
returns the whole object, so the identifier is read by that column name.
Identifiers are compared as strings, unaltered. Where the rows carry
geometry, the number of non-null geometries is compared too: the non-null
second column for `bbox-query` and `lod-query`; for `id-lookup`, DuckDB's
non-null `geometry_lod*` columns, the length of cjdb's `geometry` array
and the length of 3DCityDB's `geometries` array.

Every system is compared against the first. A disagreement sets the row's
status to `id-mismatch`, writes `id-mismatch: ...` to `notes` naming, per
pair of systems, the first five identifiers (sorted) found only on each
side together with how many there are, and makes `citybench run` exit
non-zero. The comparison is exact unless the count cross-check already
accepted the row as `ok-deviation`; only then may the symmetric difference (and
the geometry-count difference) reach the count tolerance, as a fraction of
the larger set, so an agreeing count never hides a different set.
`id-mismatch` replaces the count cross-check's status, so a row whose
counts were already a `mismatch` reads `id-mismatch` when its sets differ
too; the count decomposition stays in `notes` beside it.

### Two size figures

`size_bytes` and `size_bytes_no_index` describe the system, not the scenario,
and are repeated on every row a system contributes. Both are published
because the comparison with a file format changes depending on whether
indexes are counted. For PostgreSQL they are the sums of
`pg_total_relation_size` (heap, TOAST and every index) and `pg_table_size`
(heap and TOAST, no index) over the schema's tables, after the attribute
indexes are built. For the CityParquet systems `size_bytes` is every file in
the package directory, and `size_bytes_no_index` is the same package without
the Bloom filters and page indexes (column and offset index) inside its
Parquet files (`parquet_sizes.py`). The footer, which holds the schema and the
row-group min/max statistics, stays in both figures, since a reader cannot
read the file without it. So the CityParquet difference is the in-file index
structures a writer may omit, and the PostgreSQL difference is the separate
index relations. The manifest's `sizes` block carries the same figures plus
`index_bytes` and, for CityParquet, the Bloom-filter, page-index and footer
bytes; `size_definitions` states these definitions and the index policy.

## Metrics and the CSV contract

`<dataset>.csv` has one row per (system, scenario[, window | id probe])
and nineteen columns:

```
dataset,format,scenario,selectivity,result_count,time_mean_s,time_std_s,time_median_s,time_min_s,time_max_s,time_q1_s,time_q3_s,peak_heap_bytes,peak_working_mem_bytes,repeat,notes,bytes_read,http_requests,server_time_mean_s,server_time_std_s,server_time_median_s,server_time_min_s,server_time_max_s,server_time_q1_s,server_time_q3_s,size_bytes,size_bytes_no_index,status,raw_time_samples_s,raw_server_time_samples_s
```

Sizes and memory are recorded in bytes. The summary's figures show them in
decimal units, 1 MB = 10^6 bytes and 1 GB = 10^9 bytes (`benchviz.units`).

The first eighteen columns match, in name and order, the header of the format
harness's CSVs (`benchmark/runs/formats/results/<dataset>.csv`; `sizes.csv`
has another shape). Appending those rows to this
harness's rows needs six empty fields per row. The two harnesses use
different `dataset` identifiers (`3dbag_n1000000` here, the source file name
there) and are separate experiments.

- **`selectivity`** — `result_count / total_city_objects`; empty for
  `count`, `geometry-scan` and the write tier (a mutation's rows-touched is
  not a selection). The window's target is in `notes`, not here.
- **`time_mean_s` … `time_q3_s`** — the timing block; see "The warm protocol".
- **`peak_heap_bytes`** — populated only for the native reader (the child's
  allocator high-water mark); empty for every SQL system.
- **`peak_working_mem_bytes`** — peak working memory of the process(es)
  executing the query, in bytes, the maximum over the timed samples (Caveat 6;
  formerly `peak_rss_bytes`). The `notes` column states the scope
  (`memory-scope: duckdb-fresh-process` or
  `memory-scope: postgresql-backend-rssanon`), and the manifest's
  `memory_measurement` block describes it, with the read path and the
  provisioned memory (`shared_buffers`, `work_mem`, container limit, DuckDB
  `memory_limit`).
- **`repeat`** — the number of timed samples in that row.
- **`notes`** — `threads=single`/`threads=parallel`, memory scope, fetch
  mode, the window tag and achieved fraction or the id-probe tag
  (`id-10pct`/`id-50pct`/`id-90pct`/`id-miss`), `count-mismatch: ...`,
  `skipped: ...` or `error: <ExceptionType>`; on write rows also
  `write-tier: in-engine`, `in-engine+package-write` or
  `external-importer`, which area expression was used, and on
  `append-object` which importer ran and how many rows it wrote.
- **`bytes_read` / `http_requests`** — always empty (Caveat 8).
- **`server_time_mean_s` … `server_time_q3_s`** — the same seven statistics
  over the server-side samples; empty for the in-process systems **and for every
  write row on every system**; for `cjdb`'s and `3dcitydb`'s read rows, they
  summarise PostgreSQL's reported `Execution Time` from the
  `EXPLAIN (ANALYZE, BUFFERS)` re-runs (`pg.time_query`). Raw values are in
  `raw_server_time_samples_s`. A write row has none because `EXPLAIN
ANALYZE` would execute the mutation a second time.
- **`size_bytes` / `size_bytes_no_index`** — see "Two size figures".
- **`status`** — `ok`, `ok-deviation`, `mismatch`, `id-mismatch`, `skipped`
  or `error`. See "Count cross-check" for what `ok-deviation` means and which
  tolerance it was judged against (the manifest records the value), and
  "Identifier-set cross-check" for `id-mismatch`.

## Fairness caveats

Read these before citing a number.

1. **Counting granularity differs by storage model and is reconciled to
   CityObjects.** CityParquet and cjdb store one row per CityObject. 3DCityDB
   v5 also stores semantic boundary surfaces (`WallSurface`, `RoofSurface`,
   ...) as `feature` rows, so a bare `count(*)` over `citydb.feature`
   overcounts; `docs/3dcitydb-v5-schema.md` ("Counting granularity") records
   10045 rows against 2231 CityObjects for the Delft capture. Every 3DCityDB
   query that counts or scans objects applies the predicate derived there —
   `is_toplevel = 1 OR NOT <descends from AbstractSpaceBoundary>` — in its
   defensive form, which also survives the `ReliefFeature` anomaly documented
   in the same file.

2. **cjdb is patched.** `CjdbSystem` drives cjdb 2.2.0 built from the pinned
   PyPI sdist with `vendor/cjdb/ground-surfaces-tie.patch` applied. The patch
   makes two changes (`vendor/cjdb/README.md`):
   - **Footprint ties.** Stock `get_ground_surfaces()`
     (`cjdb/modules/geometric.py`) collects candidate footprint faces in a
     dict keyed by mean Z, so faces sharing a mean Z overwrite one another
     and `ground_geometry` loses part of the footprint — an ordinary shape
     for a flat-roofed building. The patch keeps every face and leaves the
     split threshold (mean of the distinct Z values) unchanged. The
     benchmark's claim concerns cjdb's architecture — row-oriented
     PostgreSQL with JSONB geometry — not a footprint bug in one release.
   - **Large-import batching.** The CityJSONSeq importer streams its input
     and inserts objects and relationships in 5,000-row statements, keeping
     cjdb's transaction boundaries. This changes ingestion scalability, not
     query semantics.

   The manifest's `versions.cjdb` (`2.2.0+ground-surfaces-tie-patch`) and
   `patches.cjdb` (upstream version, patch file, SHA-256, summary, build
   directory) disclose the patch on every run. Build it once with
   `just patch-cjdb` before any cjdb ingest; the step is explicit because it
   downloads from PyPI. `CjdbSystem.prepare()` fails if the build is missing
   or was made from a different version of the patch file (the build
   directory name embeds the patch's SHA-256 prefix). This caveat is not to
   be removed or weakened.

3. **Every SQL `bbox-query` is two-dimensional.** cjdb can only ever answer
   in 2D: `ground_geometry` has no z ordinate. 3DCityDB's `envelope` is a 3D
   geometry, but `ST_MakeEnvelope(xmin, ymin, xmax, ymax, srid)` is a 2D
   polygon, so the comparison is 2D. `duckdb-cityparquet` tests only the
   x/y members of the `bbox` STRUCT, although it carries `zmin`/`zmax`. Only
   the native reader tests all six bounds. Because the query windows never
   narrow z, this changes no count; the structural difference is that cjdb's
   storage cannot answer a z-restricted query, while the others' storage
   could but is not asked to.

4. **The `server_time_*` block is an instrumented upper bound, not a component of
   the `time_*` block.** The `time_*` block is the uninstrumented end-to-end figure for every
   system. The `server_time_*` block comes from a separate `EXPLAIN (ANALYZE,
BUFFERS)` execution, whose per-node timing and buffer counters (and
   `track_io_timing`) add overhead. In the committed 3DBAG CSV, 9 of the 24
   PostgreSQL rows have a mean server time greater than the mean end-to-end time, which a
   "subset of wall-clock" reading cannot explain. **Do not subtract the two
   to compute a client-server tax.** Read them side by side, qualitatively.

5. **The two thread configurations are not one number.** Under `single`,
   DuckDB runs on one thread and each PostgreSQL query in one backend:
   that is the like-for-like CPU budget and the primary figure. Under
   `parallel`, DuckDB gets 16 threads and PostgreSQL a per-gather worker
   budget of 8 — which the cluster-wide `max_worker_processes` may reduce
   further, so the manifest records both. **Never read a `threads=single`
   row against a `threads=parallel` one.** The earlier harness effectively
   published the cross of the two (DuckDB at 16, PostgreSQL at 0) as a
   single result; the advantage that produced concentrated on the headline
   rows rather than spreading evenly
   (`notes/benchmark-fairness-review-2026-09-22.md` §4.3).

6. **`peak_working_mem_bytes` is working memory, with a different process scope
   per system; it is not a like-for-like comparison.**
   - PostgreSQL rows report the peak, over sampling instants, of the summed
     `RssAnon` (`/proc/<pid>/status`) of the query's backend and, under
     `parallel`, its parallel workers (found from a second connection through
     `pg_stat_activity.leader_pid`). `RssAnon` is the backend's own heap,
     sort and hash memory; it leaves out `RssShmem`, where the 8 GB
     `shared_buffers` land (the ~8.2–8.6 GiB earlier reported for 3DCityDB
     as `peak_rss_bytes` was essentially that buffer pool), and also `RssFile`,
     the OS page cache, other backends, background processes and the client
     process. The status files are read from the host `/proc` every 5 ms
     where containers share the host kernel (rootless podman on Linux), or by
     one `exec` into the container per reading, 50 ms apart plus the exec
     round trip, on a VM-based engine (Apple `container`, docker on macOS);
     the manifest's `memory_measurement.read_path` names the path, or
     "not applied: <reason>" with a blank column. Readings come from a
     separate thread and connection, so the timed session runs no extra
     statement. One reading before and one after the query are included, so
     a query shorter than the interval records the backend at its edges: a
     lower bound, not a zero.
   - `duckdb-cityparquet` read scenarios each run in a fresh spawned process,
     so a light scenario no longer inherits a heavier one's peak. With procfs
     the value is that process's peak `RssAnon`, including the interpreter's
     and DuckDB's idle baseline; without procfs (macOS) the process's peak RSS
     from `getrusage` stands in. Write scenarios stay in the long-lived
     process.
   - Native-reader rows report the reader child's allocator peak (the same
     figure as `peak_heap_bytes`), excluding its idle baseline.

7. **The native reader answers only five of the fourteen scenarios.**
   `cityparquet-readbench --child` implements `count`, `bbox-query`,
   `attr-filter`, `attr-stats` and `id-lookup`
   (`benchmark/readbench/src/scenario.rs`). `geometry-scan`, `attr-range`,
   `lod-query`, the two `parts-per-building` forms and the whole write
   tier have no counterpart there, and the read harness is not this
   family's to extend. `registry.systems_for` names exactly which systems
   answer each scenario, so the child is never handed a name it would
   raise `ValueError` for — which `run_matrix` could not tell apart from a
   genuine failure. `id-lookup` IS implemented there, and is handed the
   same four probes as every SQL system, one child invocation each.

8. **`bytes_read` and `http_requests` are always empty.** Every system reads
   local disk or a localhost socket. The HTTP and object-storage comparison
   belongs to the format harness (`--transport http`), and neither
   PostgreSQL system has an object-storage access path.

9. **`lod-query` returns rows, and the three systems' rows are close but
   not identical.** The scenario asks for the objects carrying an LoD 2.2
   geometry, and all three materialise rows inside the timed window, each
   the object's identifier and that geometry: DuckDB the `geometry_lod2_2`
   WKB, cjdb the LoD 2.2 element of the `geometry` JSONB array, 3DCityDB the
   tier-2 geometry in `geometry_data`. Three differences remain and are
   not engineered away:
   - **Each system returns the geometry in its own representation** —
     WKB, cjdb's JSON geometry object, a PostGIS `geometry` — the same
     asymmetry Caveat 18 records for `geometry-scan` and `bbox-query`.
   - **3DCityDB pays an ordering the others do not.** `DISTINCT ON
(f.id) … ORDER BY f.id` keeps the row count CityObject-grained when
     several `property` rows of one feature match, and is scoped to the
     key rather than to the whole row so PostgreSQL never compares WKB
     geometries for equality. The sort KEY is a bigint but the sorted
     tuples carry the geometry, so at scale this may spill to disk rather
     than fit `work_mem` — check the plan for a `Sort`/`Unique` node
     before reading the row against the other two. Where each CityObject
     owns exactly one tier-2 geometry, the de-duplication removes nothing.
   - **3DCityDB's "LoD 2" is wider than CityJSON's "2.2".** `citydb-tool`
     truncates the fractional tier on import, so `val_lod = '2'` covers
     2.0, 2.1, 2.2 and 2.3 alike (Caveat 17). The benchmark's 3DBAG slice
     carries LoD 0, 1.3 and 2.2, so there tier 2 is LoD 2.2; on a dataset
     carrying another LoD 2.x besides 2.2, 3DCityDB's row set is a
     superset of the other two systems' and the cross-checks will say so.

   **Client-side object construction is kept off both sides.** DuckDB
   materialises to Arrow (`fetch: arrow`), and the PostgreSQL connections
   fetch the timed query in PostgreSQL's binary wire format (`fetch: binary`)
   and load `json`/`jsonb` as raw bytes (`pg.register_text_passthrough`), so
   geometry is never converted to text and psycopg does not parse every
   geometry document into Python dicts. Both sides still transfer every
   row and read it to exhaustion inside the timed window; what is skipped
   is the client's per-value object construction, which is not what this
   benchmark compares — and which, left in, would have been tens of
   gigabytes of Python objects for cjdb's half a million inline-vertex
   geometry documents on the 1M slice.

10. **cjdb's footprint is NULL for an object with no geometry of its own.**
    `get_ground_surfaces()` derives a footprint only from the object's own
    `geometry` array and never aggregates its children's. A parent whose
    geometry lives entirely on its children — a legitimate CityJSON shape,
    such as a `Building` whose LoD surfaces are all on `BuildingPart`s — gets
    `ground_geometry` NULL (cjdb logs `No ground surfaces were found`), and
    cjdb's `bbox-query` never returns it. CityParquet's `bbox` unions the
    object's subtree, and 3DCityDB's importer populates `envelope` for such
    parents, so on datasets with this shape cjdb undercounts. No patch is
    applied: this follows from cjdb's per-object design, and cjdb has no
    indexed column holding the subtree box (its geometry is JSONB).
    Scenarios that do not use `ground_geometry` are unaffected.

    The harness accepts this undercount as `ok-deviation` only when it is
    verified on the row: the other systems return the identical set,
    cjdb's set is a subset of it, and every missing id is decomposed into
    a NULL footprint or a footprint missing the window (Caveat 11), the
    decomposition written into `notes`. Anything else stays `id-mismatch`.
    On Tokyo (236 BuildingInstallations without a footprint) cjdb lacks
    4 of 499 (4 NULL), 12 of 2,496 (9 NULL, 3 outside) and 77 of 12,479
    (73 NULL, 4 outside) at the 1 / 5 / 25 % windows, all
    BuildingInstallations: 0.80 %, 0.48 % and 0.62 %.

11. **cjdb's footprint heuristic can also drop or shrink an object's own
    footprint — and on 3DBAG it demonstrably does.** After discarding
    (near-)vertical faces, `get_ground_surfaces()` keeps only faces whose
    mean Z is strictly below the mean of the remaining distinct Z values. An
    object whose non-vertical faces share a single Z value keeps nothing, so
    `ground_geometry` is NULL even though the object has geometry. For flat,
    elongated geometry with little Z variation the split can keep only part
    of the horizontal extent, producing an undersized footprint. Either way
    cjdb's `bbox-query` undercounts. Neither case is the tie bug the patch
    fixes, and neither is patched: this follows from cjdb's own importer,
    and patching it would benchmark a cjdb that does not exist.

    **Measured, not hypothesised.** On the committed 1M 3DBAG run the bbox
    counts decompose exactly
    (`notes/benchmark-fairness-review-2026-09-22.md` §5, which replays
    cjdb's own patched `get_ground_geometry()` over the source and
    reproduces all three counts):

    | window | `duckdb-cityparquet` | `cjdb`  |                             | `3dcitydb` |                                     |
    | ------ | -------------------- | ------- | --------------------------- | ---------- | ----------------------------------- |
    | 1 %    | 4,903                | 4,901   | = 4,903 − 2 NULL footprints | 4,903      | + 0                                 |
    | 5 %    | 63,745               | 63,729  | = 63,745 − 16               | 63,745     | + 0                                 |
    | 25 %   | 221,005              | 220,949 | = 221,005 − 60 + 4 float4   | 221,008    | + 3 (the float4 model predicts + 4) |

    Of the 60 NULL-footprint BuildingParts at the 25 % window, 39 have no
    lower horizontal face in the minimum-LoD geometry and 21 have one that
    shapely rejects as invalid, so only the roof survives and the split
    discards it. Two competing explanations were **refuted**: "footprint
    versus 3D extent" accounts for 0 objects at every window (the LoD1.2 /
    1.3 / 2.2 solids are extrusions of the footprint), and Caveat 16 does
    not operate either (0 of 1,000,001 CityParquet rows has a NULL `bbox`).
    The 3DCityDB residual of 3 rather than the modelled 4 is explained in
    class but not in count; settling it needs the four boundary rows'
    `envelope` read on a live import.

12. **PostGIS `&&` alone returns false positives, so both PostgreSQL
    systems recheck it.** Serialised PostGIS geometries cache a
    single-precision (float4) bounding box, which `&&` compares whether or
    not an index is involved. One float4 step is **0.0156 m** at
    x ≈ 1.86e5 (EPSG:7415) and about **1 m** at Tokyo's EPSG:6697
    longitudes, so an envelope lying outside the window can test as
    overlapping. Both PostgreSQL `bbox-query` forms therefore keep `&&` as
    the GiST probe and follow it with the exact, edge-inclusive test on the
    box's double-precision bounds (`ST_XMax(col) >= minx AND ST_XMin(col)
    <= maxx AND …`), the format benchmark's definition. On Tokyo the bare
    probe admitted +14 / +15 / +20 objects on 3DCityDB at the 1 / 5 / 25 %
    windows; with the recheck 3DCityDB's set equals `duckdb-cityparquet`'s
    exactly at all three, so `envelope` is the box over the object's
    subtree, as CityParquet's `bbox` is. Caveat 11's 3DBAG table predates
    the recheck and still shows the float4 term (+4 / +3 at 25 %).
    `duckdb-cityparquet` and the native reader compare double-precision
    bounds directly.

13. **3DCityDB's importer restructures CityGML-recognised attributes, so
    the attribute queries resolve each attribute's storage first.** A
    generic attribute is a top-level `property` row of its own name. An
    attribute CityGML 3.0 models as a structured datatype is not:
    `citydb-tool` writes CityJSON's `measuredHeight` as a `height` property
    (namespace `con`, type `Height`) whose scalar is the child row `value`
    and whose child `status` is `measured`; no row is named
    `measuredHeight`. Once per import, untimed, the 3DCityDB system looks
    for a top-level row of the attribute's name and otherwise for the
    importer's structured form of it (`CITYJSON_STRUCTURED_ATTRIBUTES` in
    `sql_citydb.py`, used only when present in the loaded catalogue);
    `attr-filter`, `attr-range` and `attr-stats` then read that row, as a
    3DCityDB user would. On Tokyo this turns 3DCityDB's 0 into the others'
    7,735 (`attr-range`) and 38,743 (`attr-stats`); the range form is
    served by `property_name_numval_inx` (`name = 'value' AND coalesce(…)
    > 29.1`) with the parent and `status` rows joined by key. `usage`
    (`attr-filter`) keeps its name under CityGML 3.0 (`bldg:usage`, a
    `Code` in `val_string`), so it agrees by design (12,348). `id-lookup`
    already returns every `property` row of the feature, nested ones
    included.

14. **The native reader accepts only single-table packages.**
    `cityparquet convert` writes one table per first-level CityObject family.
    `cityparquet-readbench` refuses a package with more than one object table
    (`benchmark/readbench/src/formats/cityparquet.rs`), so on a multi-family
    dataset every native-reader row that reaches the child is `error:`.
    `duckdb-cityparquet` reads every object table listed with role
    `cityparquet-objects` in `metadata.json`, combined with
    `read_parquet([...], union_by_name = true)`, and is unaffected.

15. **`lod-query` is skipped, not answered with an empty query, on a
    dataset without LoD 2.2.** It targets LoD 2.2 (`LOD_QUERY_TARGET` in
    `scenarios/registry.py`). The dataset's LoDs are read from the
    package's `geometry_lod<major>_<minor>` column names (`lods` in the
    params sidecar), and when 2.2 is not among them every system's SQL
    builder raises `ScenarioUnavailable`, so the scenario is recorded as
    `skipped: dataset carries no LoD 2.2 geometry ...` on all three. No
    system then runs a query that DuckDB could fold at plan time into an
    empty result: the count cross-check compares answers, not work, and
    would not tell such a row from one that scanned the data. The committed
    run's `lod-query` rows (all three systems count 500,296) answer an LoD
    1.2 form of the scenario and are not comparable with an LoD 2.2 run.

16. **CityParquet's `bbox` is NULL for an object whose only geometry is a
    `GeometryInstance`.** The writer computes `bbox` from the object's own
    geometry and its descendants' geometry, widened by any declared
    `geographicalExtent` (`lib/cityparquet-rs/crates/core/src/encode.rs`). A
    `GeometryInstance` contributes no extent (`geometry_bbox` in
    `wkb_write.rs`). An object with only implicit geometries in its subtree
    and no declared extent therefore has a NULL `bbox`, and every
    `bbox-query` on CityParquet excludes it, while cjdb and 3DCityDB resolve
    the placed geometry and can include it. Such a row is also outside the
    query-window derivation's denominator, which is recorded as
    `window_rows` in the params sidecar. On the committed 3DBAG slice this
    caveat does not operate: 0 of 1,000,001 rows has a NULL `bbox`.

17. **3DCityDB answers `id-lookup` and `lod-query` in its own shape.**
    `id-lookup` returns the whole object on every system, as one row:
    `duckdb-cityparquet` the package row (`SELECT *`, every geometry
    column included), `cjdb` the `city_object` row (`SELECT *`, the
    `geometry` JSONB included), and 3DCityDB the `feature` row plus one
    array per `property` column (`name` and every `val_*`) and an array of
    the object's geometries, one per LoD, aggregated in scalar subqueries
    so the lookup stays one row. 3DCityDB therefore reads three tables
    where the others read one, and its row is shaped differently: the
    attributes come back as parallel arrays rather than as named columns
    or a JSON object.

    "The object's geometry" is not a single row in 3DCityDB's model.
    citydb-tool turns each semantic surface of a CityJSON geometry into a
    boundary feature (`RoofSurface`, `WallSurface`, ...) that the object
    contains (a `property` row with `val_feature_id` and
    `val_relation_type = 1`), and the surface's polygons become a
    `geometry_data` row of that boundary feature. A Building whose LoD 2
    MultiSurface carries semantics owns no `geometry_data` row at all. So
    every 3DCityDB scenario that returns geometry (`geometry-scan`,
    `bbox-query`, `id-lookup`, `lod-query`) gathers it the way citydb-tool's
    export reassembles an object: from the object and its contained parts,
    followed recursively and stopping at CityObject classes (a BuildingPart
    stays its own object); per LoD, the rows of the shallowest level that
    has any, so an object's own root row (a Solid, say) is returned as
    stored rather than the surfaces it is built from; and, where that level
    has several rows, one `ST_Collect` collection of them, with no union or
    other geometry processing. That walk and collection is work the other
    two systems do not do, and it is inside the timed query.

    The scenario is one CityParquet loses heavily in any case, for a
    reason Caveat 22 isolates (a row-group decode, not a missing index).
    `lod-query` on 3DCityDB targets the integer LoD tier 2
    (`CITYDB_LOD_TIER`), because the importer stores only the integer tier
    on the `property` row; tier 2 equals LoD 2.2 only where 2.2 is the
    dataset's only LoD 2.x. The benchmark's 3DBAG slice carries LoD 0, 1.3
    and 2.2, so there the two coincide (Caveat 9).

18. **`geometry-scan` and `bbox-query` return geometry in each system's
    native binary form, which is not one form.** DuckDB hands back the
    stored WKB of the per-LoD columns (a LoD 0 footprint typed with Parquet's
    GEOMETRY logical type is re-encoded by `ST_AsWKB` in `bbox-query`, so
    `coalesce` binds). cjdb returns its geometry JSONB, whose binary wire
    form is still JSON text with a version byte. 3DCityDB returns PostGIS's
    binary geometry. No system converts geometry to text and no byte sizes
    are summed, but the amount transferred per object differs with each
    system's storage, and that difference is part of what is measured.
    3DCityDB's geometry is the object's geometry as Caveat 17 defines it,
    one per LoD, so a semantically split surface comes back as an
    `ST_Collect` collection of its boundary features' rows. Its
    `property.val_lod` holds only the integer LoD tier, so the rows of two
    LoDs inside one tier (1.2 and 1.3) fall into one collection, and the
    highest-LoD pick is by tier.

19. **The write tier is different operations, not one scale.** cjdb rewrites
    roughly half a million JSONB tuples under MVCC; 3DCityDB inserts,
    updates and deletes half a million EAV rows; CityParquet adds or drops a
    column of an in-memory table and, on the `-writeback` rows, re-encodes
    the package and its footers. `append-object` is the widest gap of the
    four and the most deliberate: three different importers, two of them
    external processes whose launcher cost (a Python start, a container
    plus a JVM) is inside the timed window, writing different numbers of
    rows for the same appended object and maintaining different derived
    state. Its `result_count` is a definition — the CityObjects in the
    appended file — and what each importer wrote is in `notes`. Read the
    four `append-object` rows as four accounts of "what it costs this
    system to add a building", never as one ratio. The area expressions also differ, inherited
    from the CJDB paper (footprint area on cjdb, envelope area on
    3DCityDB). The `databases` figure puts the write rows below the reads
    and colours them by ratio to 3DCityDB like the reads, so it carries
    this caveat as its footnote: a write ratio says what each system pays
    for the same request, not how fast one common operation runs, and is
    never quoted without this caveat. The CityParquet (DuckDB) write cell
    stacks the in-engine value over the `-writeback` value (marked `+wb`),
    each with its own ratio. `attr-delete`'s CityParquet `result_count` is a
    definition rather than a measurement (see "The write tier").

20. **`EXPLAIN (ANALYZE, BUFFERS)` doubles the per-sample work on both
    PostgreSQL systems for every read row.** The instrumented re-run is
    outside the timed window, so it does not inflate the `time_*` block directly, but
    it does mean each PostgreSQL read sample executes its query twice while
    neither DuckDB row does — relevant to cache state between samples, and
    to wall-clock planning of a full run. Write rows are exempt, because
    there the re-run would apply the mutation twice (see "The write tier").

21. **The CityParquet package carries bloom filters, and DuckDB consults
    them.** Since the writer's 2026-09-23 default, `cityparquet convert`
    writes Parquet bloom filters on `id`, `feature_id` and every string
    attribute whose estimated distinct count reaches a fifth of its
    non-null count (FPP 0.01, filters after the last row group). DuckDB
    reads them for equality predicates, so `id-lookup` and `attr-filter`
    on `duckdb-cityparquet` can skip row groups a min/max statistic could
    not. cjdb and 3DCityDB answer the same probes through their btree
    indexes, so this is an index-versus-index comparison, not a scan
    against an index as it was in the 12 September run, whose package
    carried no filters. A figure that puts the two runs side by side must
    say so. `parquet_metadata(...)` on the package shows
    `bloom_filter_offset` non-null on the filtered columns; the format
    family's `bloom` family measures the effect in isolation. The filters
    explain the `id-miss` row, not the hit rows: what a hit costs on
    `duckdb-cityparquet` is the decode of the surviving row group (Caveat
    22).

22. **`duckdb-cityparquet`'s `id-lookup` hit is a row-group decode, not an
    index miss.** DuckDB does use the bloom filters; what the hit rows
    measure is DuckDB materialising every column of the one surviving row
    group for a `SELECT *`. A diagnostic probe on 25 September 2026, on the
    committed 1M Hilbert package with the harness's own probe ids, the same
    DuckDB 1.5.5 the committed run used, one thread, five samples after a
    warm-up (a probe, not the committed rows' means) measured:

    | query                            | hit            | miss           |
    | -------------------------------- | -------------- | -------------- |
    | `SELECT * … WHERE id = ?`        | 385 ms         | 32 ms          |
    | `SELECT id … WHERE id = ?`       | 9.4 ms         | 6.0 ms         |
    | `SELECT count(*) … WHERE id = ?` | as `SELECT id` | as `SELECT id` |

    `parquet_bloom_probe` confirms that the miss id is excluded by the
    filters of every row group. The committed `threads=single` rows agree
    in shape: 374, 373 and 274 ms for the three hits, 44 ms for the miss.
    The id-only probe is faster than the native Rust reader's own lookup
    (the `bloom` family reports 150 ms for a hit and 16 ms for a miss), so
    the bloom filters are not what is missing. DuckDB has no late
    materialisation for Parquet: once the filters leave one 65 536-row row
    group, a `SELECT *` decodes that whole row group across all 84
    columns, four geometry columns included, before it keeps one row. The
    Rust reader decodes only the pages that hold the hit.

    So the database `id-lookup` and the `bloom` family's `id-lookup` are
    **different operations** — the whole object through a SQL engine,
    against the reader's own lookup — and must not be compared across
    figures. A smaller row group would cut the cost of the hit, at a
    price elsewhere that the suite does not measure; the committed
    package uses the writer's default.

## Running the benchmark

### Through the suite (the path that produced the committed run)

From the repository root, the database family runs through
`benchmark/scripts/bench_suite.py`:

```sh
just bench-prep --families databases   # prepare the 3DBAG slice and packages; build citydb-tool image and patched cjdb
just bench-run  --families databases   # isolated databases, one run
```

`bench-prep` fetches and prepares the 3DBAG slice named in
`benchmark/manifest.toml` (`slice_dataset`, the 1,000,000-object slice
`3dbag_n1000000`) into `benchmark/runs/data/`, and runs `citybench prep`,
which runs `just build-citydb` and `just patch-cjdb`. `bench-run` calls
`citybench run --data-root benchmark/runs --prepared-dir
benchmark/runs/data/readbench --dataset <slice> --output-dir
benchmark/runs/databases/results --repeat 25`. `--profile quick` measures
the same slice with `--repeat 7` into `benchmark/runs/databases/quick/`; the
CSV's `repeat` column and the manifest carry the 7. Under `--profile short` and
`--profile smoke` (or `--smoke`), the database family measures the
manifest's `small_database_dataset`, Rotterdam, through its prepared
`rotterdam_delfshaven.city.jsonl`, and writes to
`benchmark/runs/databases/short/` or `benchmark/runs/databases/smoke/`.
Only the `full` and `quick` profiles measure the slice. Figures come from
`just bench-summary`, which only reads results.

### A single run with the CLI

From `benchmark/databases/`:

```sh
uv run python -m citybench.cli run \
  --data-root ../runs \
  --prepared-dir ../runs/data/readbench \
  --dataset <path/to/dataset>.city.jsonl \
  [--systems duckdb-cityparquet,duckdb-cityparquet-writeback,cjdb,3dcitydb] \
  [--repeat 25] [--srid 7415] [--count-tolerance 0.001] \
  [--numa-node auto|off|<N>] [--max-load auto|off|<x>] \
  [--max-load-wait-s 600] [--memory-max <bytes>] \
  [--output-dir <dir>]
```

One invocation measures both thread configurations and then the write
tier; there is no flag to run half of it, because the order is what keeps
the write tier's bloat out of the read rows.

With `--data-root` (which must lie below `benchmark/runs/`),
`lifecycle.isolated_databases`:

- creates two fresh containers named `citybench-cjdb-<uuid>` and
  `citybench-citydb-<uuid>`, each with the resource limits above, published
  on a free `127.0.0.1` port chosen by the harness;
- binds each PostgreSQL data directory to
  `<data-root>/databases/<uuid>/{cjdb,3dcitydb}`, except on Apple
  `container`, where it stays inside the container ("Container engine");
- creates a per-run temporary directory under `$TMPDIR` (or
  `/data2/hideba/tmp` if unset) with separate subdirectories bound to each
  container's `/tmp`, used by `citydb-tool`, and set as DuckDB's
  `temp_directory`;
- waits until both servers accept connections and 3DCityDB's v5 schema
  exists;
- passes `--srid` to the 3DCityDB container as `SRID` (default 7415);
- stops (and thereby removes) the containers and deletes the temporary
  directory when the run ends. The data directories under
  `<data-root>/databases/<uuid>/` remain on disk.

3DCityDB's SRID is fixed when its database is initialised and cannot be
changed afterwards; a wrong SRID does not raise an error but silently
mislabels spatial results. The manifest's `srid` block records the SRID each
PostgreSQL system reports after import (`cj_metadata` and `database_srs`),
not the requested value.

The CityParquet package must already exist as
`<prepared-dir>/<dataset>.parquet`, for example from
`just readbench-prepare <input> <outdir> cityparquet` at the repository
root. `--dataset` names the CityJSONSeq file, `<dataset>.city.jsonl`, or
the CityJSON file `<dataset>.city.json`. cjdb imports CityJSONSeq only, so
for a CityJSON input `_dataset` in `cli.py` runs every system, the
parameter derivation and the manifest's source hash from
`<prepared-dir>/<dataset>.city.jsonl`, or else a `<dataset>.city.jsonl`
beside the input, and stops with an error when neither exists. The dataset
name, and so the package looked up, is the same for both inputs. Without
`--output-dir`, results are written to
`benchmark/runs/databases/results/`. Every run overwrites
`<dataset>.csv`, `<dataset>.manifest.json`, `<dataset>.params.json` and
`<dataset>.indexes.sql`.

`citybench smoke` runs the same pipeline with `repeat = 2` and fails on any
count mismatch, identifier mismatch or error. Its default dataset,
`benchmark/databases/data/delft.city.jsonl`, is not in the repository
(`data/` is git-ignored), so pass `--dataset` or place the file there first.

### Fixed-port databases (`benchmark/databases/justfile`)

The local recipes use `docker/compose.yml`, which starts `citybench-cjdb` on
port 55432 and `citybench-citydb` on port 55433 with bind-mounted data under
`$CITYBENCH_DB_ROOT`. Compose refuses to start without that variable:

```sh
cd benchmark/databases
export CITYBENCH_DB_ROOT=<dir below benchmark/runs>
just build-citydb                  # pinned citydb-tool image
just patch-cjdb                    # patched cjdb (Caveat 2)
CITYDB_SRID=<epsg> just up         # start both, wait for readiness and the citydb schema
just bench <dataset> [REPEAT] [NUMA_NODE] [MAX_LOAD] [MAX_LOAD_WAIT_S]  # citybench run without --data-root: ports 55432/55433
just down
```

`just bench` passes no `--prepared-dir`, so packages are read from
`benchmark/runs/data/readbench/`, where `just bench-prep` leaves the format
benchmark's packages (written with `--no-lod0`). `just smoke` has no `--dataset`
argument and therefore needs `benchmark/databases/data/delft.city.jsonl`.
`just capture-schema <dataset>` reads both schemas from these fixed ports and
overwrites `docs/cjdb-schema.md` and `docs/3dcitydb-v5-schema.md` with fresh
table, column and index listings, discarding their hand-written sections.

`just down` runs the engine's compose front end (`podman-compose`, or
`docker compose`; Apple `container` has none) with `down -v`, which removes the containers but
not the bind-mounted data under `$CITYBENCH_DB_ROOT`. `citydb-tool import
cityjson` has no replace mode, so importing again into an existing 3DCityDB
database duplicates every object. Before a new import, or before changing
`CITYDB_SRID`, delete `$CITYBENCH_DB_ROOT/cjdb` and
`$CITYBENCH_DB_ROOT/citydb`.

### Tests

```sh
cd benchmark/databases
just test       # uv run pytest -m "not integration"
just test-all   # includes integration tests, which need running databases
```

## Environment

From the committed manifest (`3dbag_n1000000.manifest.json`):

```
platform:  Linux-6.8.0-136-generic-x86_64-with-glibc2.39
processor: x86_64
python:    3.12.1
duckdb (Python client): 1.5.5
cjdb:      2.2.0+ground-surfaces-tie-patch (patch SHA-256 a54a9fd1909a…, identical to the committed patch file)
SRID:      7415 (cjdb and 3dcitydb)
```

Pinned in code rather than recorded in the manifest: the PostgreSQL images
(`src/citybench/lifecycle.py`), 3DCityDB 5.1.2 through the 3DCityDB image tag,
and `citydb-tool` 1.3.2 on `eclipse-temurin:21-jre`
(`docker/citydb.Dockerfile`). PostgreSQL 16.4 and PostGIS 3.4 are the values
captured in `docs/*-schema.md`. The manifest records no CPU model, core
count or memory size.
