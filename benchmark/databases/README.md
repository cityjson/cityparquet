# CityParquet vs cjdb vs 3DCityDB v5: database benchmark harness

`citybench` compares CityParquet — read by DuckDB (`duckdb-cityparquet`) and,
optionally, by the native Rust reader (`cityparquet`, `cityparquet-hilbert`) —
against two PostgreSQL-based 3D city model databases, **cjdb** and
**3DCityDB v5**. It follows the discipline of
`benchmark/formats/READ_BENCHMARK.md` (real inputs, repeated warm samples,
disclosed rather than hidden overheads) and adds what a cross-**system**
comparison must also control: server tuning, resource limits, index parity,
and the client-server boundary the two PostgreSQL systems sit behind.

The container runtime is rootless **Podman**.

## Purpose and claim

The harness measures **steady-state performance** — wall-clock time, peak
resident memory of the executing process and, for PostgreSQL read
scenarios, server-reported execution time — against a dataset already
loaded into each system. Ten **read** scenarios run under two disclosed
thread configurations; four **write** scenarios then run once, reported in
their own table under their own caveat (Caveat 19). The scenario set is the
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

> **The committed CSV predates this scenario set and is not citable
> against it.** It was produced by the previous harness: `full-read`,
> `project` and `hierarchy` instead of `geometry-scan`,
> `parts-per-building` and the write tier; a single `id-lookup` target
> instead of four probes; no `append-object` row; `lod-extract` returning
> ids where `lod-query` returns rows; `attr-filter` on `object_type`;
> lower-left area windows achieving 0.49 %/6.37 %/22.1 % instead of the
> row-fraction targets; DuckDB on 16 threads against a PostgreSQL with
> parallel query disabled; and the source-order package displayed as
> "CityParquet" while the format family displayed the Hilbert one. The
> database family must be re-run before any database figure is regenerated.
> `notes/benchmark-fairness-review-2026-09-22.md` records what each of
> those changed and why.

The only committed database results are one run over the 1,000,001-object
3DBAG scaling slice (`3dbag_n1000000`):

| File                                                            | Contents                                                                                                                          |
| --------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------- |
| `benchmark/runs/databases/results/3dbag_n1000000.csv`           | 36 rows: three systems × twelve scenario rows of the RETIRED set, `repeat` = 7                                                    |
| `benchmark/runs/databases/results/3dbag_n1000000.manifest.json` | source SHA-256, host, versions, `pg_settings`, ingest times, sizes, cjdb patch disclosure, SRIDs, memory scope, temporary storage |
| `benchmark/runs/databases/results/3dbag_n1000000.params.json`   | the query parameters derived from that source                                                                                     |
| `benchmark/runs/databases/results/3dbag_n1000000.indexes.sql`   | the DDL this harness added, plus a live `pg_indexes` dump for both PostgreSQL schemas                                             |

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

| tag                            | what it is                                                                                                                                                                           | runs                                                                       | index support                                                            |
| ------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | -------------------------------------------------------------------------- | ------------------------------------------------------------------------ |
| `duckdb-cityparquet`           | DuckDB (Python client) `read_parquet()` over the **Hilbert** CityParquet package `<prepared>/<dataset>-hilbert.parquet`; no separate ingest                                          | every scenario                                                             | Parquet statistics used by DuckDB's own scan                             |
| `duckdb-cityparquet-source`    | the same, over the **source-order** package `<prepared>/<dataset>.parquet`                                                                                                           | `bbox-query` only — the one scenario whose answer depends on row order     | the same statistics, with row groups in source order                     |
| `duckdb-cityparquet-writeback` | the same as `duckdb-cityparquet`, with `cityparquet_write` inside the timed window                                                                                                   | the write tier only                                                        | —                                                                        |
| `cjdb`                         | cjdb 2.2.0, **patched (Caveat 2)**, imported into PostgreSQL/PostGIS. Full geometry is JSONB (`city_object.geometry`); only a 2D footprint is a PostGIS geometry (`ground_geometry`) | every scenario                                                             | cjdb's own defaults plus one added btree(`object_id`) — see "Index sets" |
| `3dcitydb`                     | 3DCityDB v5.1.2, imported with `citydb-tool` 1.3.2 into PostgreSQL/PostGIS. Generic `feature`/`property`/`geometry_data` schema: CityGML classes are rows, attributes are EAV rows   | every scenario                                                             | the indexes `citydb-tool import cityjson` creates; none added            |
| `cityparquet`                  | the native Rust reader over the source-order package, driven per sample as `cityparquet-readbench --child`                                                                           | `count`, `bbox-query`, `attr-filter`, `attr-stats`, `id-lookup` (Caveat 7) | Parquet row-group min/max statistics and column projection               |
| `cityparquet-hilbert`          | the same reader over `<prepared>/<dataset>-hilbert.parquet`, rows in Hilbert-curve order                                                                                             | the same five                                                              | the same statistics, with tighter per-row-group bounding boxes           |

`citybench run` uses the three `duckdb-cityparquet*` tags plus `cjdb` and
`3dcitydb` by default. The native readers run only when named in
`--systems` and need the binary
`benchmark/readbench/target/release/cityparquet-readbench`
(`cargo build --release --manifest-path benchmark/readbench/Cargo.toml`).

### Which package is "CityParquet"

`duckdb-cityparquet` reads the **Hilbert** package, because that is the
one the format family's figures display under the name "CityParquet"
(`benchmark/plot/benchviz/figures.py`). Until this was changed the two
benchmark families published _different artefacts_ under one name: the
database family read the source-order package, on which Hilbert ordering
was measured to be 1.44x/2.02x/3.13x **slower** at the 1/5/25 % windows,
so the old choice flattered CityParquet on exactly the bbox rows
(`notes/benchmark-fairness-review-2026-09-22.md` §4.5).

Row order can only change the answer's _cost_, never the answer, and only
where a predicate is spatial. So the source-order package is published as a
second system tag for `bbox-query` alone — the only spatial scenario left
in the set — rather than doubling every row for a difference that would be
noise. One scenario is enough for a control, and the tag stays: without it
the two families would again publish different artefacts under one name.
Publish both orders; do not pick a winner afterwards.

## Query parameters

Every system receives the **same** parameters, derived deterministically by
`citybench.params.derive` (ties are broken by sorting). No system derives
its own idea of "a 5 % window" or "a typical building".

Derivation reads two things: the source CityJSON/CityJSONSeq file, and the
**CityParquet package**. The package supplies the extent, the query
windows, the `attr-filter` predicate and `attr-range`'s threshold — every
parameter the format harness also derives from the package — so the two
families ask the same questions of the same dataset. (The source-order and
Hilbert packages hold the same rows in a different order, so either yields
identical parameters; the source-order one is named for determinism.)

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
file. `just derive-params --dataset <src> --prepared-dir <dir>` writes the
same payload to `params/<dataset>.json` (see "Heterogeneity corpus
parameter files").

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
| `geometry-scan`           | `(count, bytes)`         | every object's geometry                                            | `count(*), sum(octet_length(...))` over every `geometry_lod*` column (Caveat 18)                                                                      | `count(*), sum(length(geometry::text))`                                                                                                                                                                  | `count(DISTINCT f.id), sum(length(gd.geometry::text))` over `geometry_data` joined to CityObject-grain features                                                                                                                                   |
| `count`                   | count                    | total CityObject count                                             | `SELECT count(*)` — answered from file metadata; caption it as such                                                                                   | `SELECT count(*) FROM cjdb.city_object`                                                                                                                                                                  | `count(*)` over `feature` with the CityObject predicate (Caveat 1)                                                                                                                                                                                |
| `bbox-query` (1/5/25 %)   | count                    | objects whose bbox intersects the window                           | `bbox.xmax/xmin/ymax/ymin` comparisons on the `bbox` STRUCT — **x/y only**                                                                            | `ground_geometry && ST_MakeEnvelope(...)`, GIST-indexed — 2D by storage (Caveat 3)                                                                                                                       | `envelope && ST_MakeEnvelope(...)`, GIST-indexed; `ST_MakeEnvelope` returns a 2D polygon, so the test is 2D                                                                                                                                       |
| `attr-filter`             | ids                      | objects matching the per-dataset attribute predicate               | `WHERE "<col>" = ?` — a typed, flattened top-level column                                                                                             | `WHERE attributes ->> '<col>' = %s` — **no index on `attributes`** (see "Index sets")                                                                                                                    | `property` join on `pr.name = %s AND pr.val_string = %s` (Caveat 13)                                                                                                                                                                              |
| `attr-range`              | ids                      | objects whose numeric attribute exceeds the threshold              | `WHERE "<col>" > ?` — DOUBLE column with row-group statistics                                                                                         | `WHERE (attributes ->> '<col>')::float > %s`                                                                                                                                                             | `property` join with `coalesce(val_double, val_int) > %s`                                                                                                                                                                                         |
| `attr-stats`              | `(count, min, max, sum)` | aggregate of `numeric_column`                                      | aggregates over the flattened top-level column                                                                                                        | aggregates over `(attributes->>col)::numeric` — every row's JSONB unpacked, and heavier arithmetic than a DOUBLE sum                                                                                     | EAV join `property`→`feature` on `name = col`, aggregating `coalesce(val_double, val_int)` (Caveat 13)                                                                                                                                            |
| `id-lookup` (×4 probes)   | the object               | the row for one probe id, materialised                             | `SELECT * WHERE id = ?` — the whole object row, geometry included; no index                                                                           | `SELECT * WHERE object_id = %s` (added btree) — the row includes the geometry JSONB                                                                                                                      | `SELECT * FROM feature WHERE objectid = %s` (btree) — the `feature` row only; `property` and `geometry_data` are not joined (Caveat 17)                                                                                                           |
| `lod-query`               | whole rows               | objects carrying an LoD 1.2 geometry (Caveat 9)                    | `SELECT * WHERE geometry_lod1_2 IS NOT NULL`, fetched to Arrow inside the timed window; `WHERE FALSE` when the package has no such column (Caveat 15) | `SELECT *` with `geometry @? '$[*] ? (@.lod == "1.2")'` — the `@?` operator, which uses cjdb's GIN(`geometry`) index; the `jsonb_path_exists` function form does not; the row carries the geometry JSONB | `SELECT DISTINCT ON (f.id) f.*, gd.geometry` through the `property` row with `val_lod = '1' AND val_geometry_id IS NOT NULL`, joined to `geometry_data` — the importer stores LoD 1.2 as `'1'` (`docs/3dcitydb-v5-schema.md`, "LoD value format") |
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
is what makes the three times comparable: DuckDB returns every column,
including all of the object's LoD geometries; cjdb's row carries the whole
`geometry` JSONB; 3DCityDB joins `geometry_data` for the LoD-1 solid.
3DCityDB still returns less than the other two — the attributes live in
`property` and are not joined (Caveat 17) — and its `DISTINCT ON (f.id)`
adds a bigint sort the others do not pay, in exchange for a row count that
is one per CityObject even when several `property` rows of that feature
match. On 3DBAG the objects carrying an LoD1.2 geometry are the
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

Four scenarios, run **last** and reported in their own table. They are
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
  `server_time_s`. `EXPLAIN ANALYZE` on an INSERT/UPDATE/DELETE _executes_
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
- **Two of the four rows time an external process, launcher included.**
  `cjdb import` pays `uv`'s resolution and a Python start; `citydb-tool
import` pays a container start and a JVM start, which for a one-feature
  file is a large share of the number. Neither is subtracted. A
  non-mutating `--help`/`--version` invocation runs **untimed** before the
  first sample so a cold image or resolve does not land on sample 1 —
  that warms the launcher, never the mutation. Those two rows also carry
  **no `peak_rss_bytes`**: the work happens in a process this harness
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
| Q5 LoD 1.2            | `lod-query`          | Returns **whole rows**, where Q5 returns ids: the catalogue's own definition is "retrieve all buildings having a specific LoD geometry", and a projection of ids alone is answerable from one column's definition levels on a Parquet reader. cjdb uses the `@?` jsonpath operator rather than the paper's `@>`, because only the operator form cooperates with cjdb's own GIN index. 3DCityDB must test `val_lod = '1'` — its importer truncates the fractional tier, so its "LoD 1" covers CityJSON's 1.2 _and_ 1.3 — and joins `geometry_data` so its row carries a geometry like the other two. |
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
at capture time. The run manifest does not record server or PostGIS
versions.

### Tuning and parallelism

`docker/postgresql.conf` is mounted read-only, as the same file, into both
containers. Stock PostgreSQL defaults (128 MB `shared_buffers`) would make
either database a strawman. The manifest's `pg_settings` block records the
values the committed run read back with `current_setting()`:

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

### Resource limits

Each PostgreSQL container is limited to 16 CPUs and 32 GB of memory, with a
2 GB `/dev/shm`. The isolated lifecycle (`lifecycle.isolated_databases`)
passes these as `podman run --cpus 16 --memory 32g --shm-size 2g`;
`docker/compose.yml` declares the same limits in `deploy.resources.limits`.
`docs/3dcitydb-v5-schema.md` ("Resource limits — measured, not assumed")
records that on containers started from `compose.yml` podman-compose applied
`--cpus 16.0 -m 32g`, that `cpu.max` and `memory.max` read 16 cores and
32 GiB inside both containers, and that a CPU-bound load was throttled
symmetrically. No equivalent measurement is recorded for the isolated
containers.

`nproc` inside the containers reports the host's full core count, because it
reflects CPU affinity rather than the bandwidth quota. `citydb-tool import
cityjson` sizes its thread pool from `nproc`, and each thread opens a
connection, so the adapter passes `--threads=4` to stay within
`max_connections`.

### Index sets

Every scenario's index requirement is checked against what each system builds
unasked, and only what is missing is added; a duplicate index has no query
benefit but inflates `size_bytes`.

- **cjdb: one added index**, `ix_co_object_id` = btree(`object_id`)
  (`sql_cjdb.index_ddl`). cjdb's own unique index is the composite
  `(cj_metadata_id, object_id)`; every row shares one `cj_metadata_id`, so
  that index cannot serve a bare `WHERE object_id = ?` probe efficiently.
  The docstring of `sql_cjdb.index_ddl` records the `EXPLAIN` comparison and
  the default indexes that already cover the other scenarios: GIST on
  `ground_geometry` (cjdb creates two), btree(`type`), GIN(`geometry`), and
  the relationship btrees.
- **3DCityDB: none added.** `citydb-tool import cityjson` creates
  3DCityDB's content indexes during import, and `citydb index create` is a
  no-op afterwards (`docs/3dcitydb-v5-schema.md`, "Index coverage";
  `sql_citydb.index_ddl` docstring). The CityObject predicate is resolved
  once per ingest to a static `objectclass_id IN (...)` list
  (`sql_citydb.resolve_cityobject_class_ids`), which is sargable against
  `feature_objectclass_inx`.

`<dataset>.indexes.sql` records both the DDL this harness added and a live
`pg_indexes` dump of each PostgreSQL schema taken at run time (`pg.dump_indexes`),
so the complete index set each system queried against is auditable. The
committed 3DBAG file lists 15 cjdb indexes and 59 3DCityDB indexes.

### `VACUUM ANALYZE`

Both PostgreSQL adapters end `ingest()` with `VACUUM ANALYZE` over every table
in their schema (`pg.vacuum_analyze`) before any timed scenario runs.

### The warm protocol

Every `run()` performs **one discarded warm-up** call followed by `repeat`
timed samples of the same query; `--repeat` defaults to **7**.

- `time_s` is the **arithmetic mean** of the samples and `time_mad_s` is the
  **median absolute deviation** about their median (`report.py`,
  `stats.py`), both to six decimal places. The raw samples are in
  `raw_time_samples_s`.
- The PostgreSQL adapters time each sample from just before the query is sent
  to just after every row has been fetched. After each timed execution the
  same query runs again, untimed, under `EXPLAIN (ANALYZE, BUFFERS, FORMAT
JSON)` to obtain `server_time_s` (Caveat 4).
- `duckdb-cityparquet` times in-process. For a scenario that returns rows
  the result is materialised **inside** the timed window, to **Arrow**
  (`to_arrow_table()`), not to Python objects — a Python-object fetch of a
  row-returning scenario was measured at about 72 µs/row on `SELECT *`,
  which would make the row the client's number rather than the engine's.
  `notes` carries `fetch: arrow`. The PostgreSQL adapters already read
  every row to exhaustion inside their own timed window, so no system wins
  by handing back a lazy cursor.
- The native readers start a fresh child process per sample, and the harness
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

`citybench smoke` fails on any row whose **status** is `mismatch` or
`error`. It deliberately does not search `notes` for the text
`count-mismatch`: an `ok-deviation` row keeps that text, because the
decomposition is the row's value.

Each expansion of a scenario is checked **against itself**: the 5 % window
against the 5 % window, the `id-miss` probe against the `id-miss` probe.
The miss legitimately returns 0 where the three hits return 1, and folding
the four probes into one row would have made that read as a mismatch.

The check compares answers, not work: it cannot detect a system that returns
the right count without doing the work the scenario is meant to measure
(Caveat 15).

### Two size figures

`size_bytes` and `size_bytes_no_index` describe the system, not the scenario,
and are repeated on every row a system contributes. Both are published
because the comparison with a file format changes depending on whether
indexes are counted. For PostgreSQL they are the sums of
`pg_total_relation_size` and `pg_table_size` over the schema's tables; for
the CityParquet systems both are the total bytes of every file in the package
directory, which has no separate index. The manifest's `sizes` block carries
the same figures.

## Metrics and the CSV contract

`<dataset>.csv` has one row per (system, scenario[, window | id probe])
and nineteen columns:

```
dataset,format,scenario,selectivity,result_count,time_s,time_mad_s,peak_heap_bytes,peak_rss_bytes,repeat,notes,bytes_read,http_requests,server_time_s,size_bytes,size_bytes_no_index,status,raw_time_samples_s,raw_server_time_samples_s
```

The first thirteen columns match, in name and order, the header of the format
harness's CSVs (`benchmark/formats/read_results/*.csv` and
`benchmark/runs/formats/results/<dataset>.csv`; `sizes.csv` and
`*.write.samples.csv` have other shapes). Appending those rows to this
harness's rows needs six empty fields per row. The two harnesses use
different `dataset` identifiers (`3dbag_n1000000` here, the source file name
there) and are separate experiments.

- **`selectivity`** — `result_count / total_city_objects`; empty for
  `count`, `geometry-scan` and the write tier (a mutation's rows-touched is
  not a selection). The window's target is in `notes`, not here.
- **`time_s` / `time_mad_s`** — see "The warm protocol".
- **`peak_heap_bytes`** — populated only for the native readers (the child's
  allocator high-water mark); empty for every SQL system.
- **`peak_rss_bytes`** — peak resident set size of the process executing the
  query, in bytes, the maximum over the timed samples (Caveat 6). The
  `notes` column states the scope (`memory-scope: duckdb-process-rss` or
  `memory-scope: postgresql-backend-rss`), and the manifest's
  `memory_measurement` block describes it.
- **`repeat`** — the number of timed samples in that row.
- **`notes`** — `threads=single`/`threads=parallel`, memory scope, fetch
  mode, the window tag and achieved fraction or the id-probe tag
  (`id-10pct`/`id-50pct`/`id-90pct`/`id-miss`), `count-mismatch: ...`,
  `skipped: ...` or `error: <ExceptionType>`; on write rows also
  `write-tier: in-engine`, `in-engine+package-write` or
  `external-importer`, which area expression was used, and on
  `append-object` which importer ran and how many rows it wrote.
- **`bytes_read` / `http_requests`** — always empty (Caveat 8).
- **`server_time_s`** — empty for the in-process systems **and for every
  write row on every system**; for `cjdb`'s and `3dcitydb`'s read rows, the
  mean of PostgreSQL's reported `Execution Time` from the
  `EXPLAIN (ANALYZE, BUFFERS)` re-runs (`pg.time_query`). Raw values are in
  `raw_server_time_samples_s`. A write row has none because `EXPLAIN
ANALYZE` would execute the mutation a second time.
- **`size_bytes` / `size_bytes_no_index`** — see "Two size figures".
- **`status`** — `ok`, `ok-deviation`, `mismatch`, `skipped` or `error`.
  See "Count cross-check" for what `ok-deviation` means and which tolerance
  it was judged against (the manifest records the value).

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
   the native readers test all six bounds. Because the query windows never
   narrow z, this changes no count; the structural difference is that cjdb's
   storage cannot answer a z-restricted query, while the others' storage
   could but is not asked to.

4. **`server_time_s` is an instrumented upper bound, not a component of
   `time_s`.** `time_s` is the uninstrumented end-to-end figure for every
   system. `server_time_s` comes from a separate `EXPLAIN (ANALYZE,
BUFFERS)` execution, whose per-node timing and buffer counters (and
   `track_io_timing`) add overhead. In the committed 3DBAG CSV, 9 of the 24
   PostgreSQL rows have `server_time_s` greater than `time_s`, which a
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

6. **`peak_rss_bytes` has a different process scope per system and is not a
   like-for-like comparison.**
   - PostgreSQL rows sample the query's backend process, located on the host
     through `podman top` or a verified PID-namespace mapping, every 5 ms
     from `/proc/<pid>/status`. The value excludes other backends,
     background workers and the idle server, but mapped shared pages
     (including touched `shared_buffers`) can contribute. It is blank when
     the PID cannot be mapped safely. **Under `parallel` only the LEADER
     backend is sampled**, so any parallel worker's resident memory is
     excluded and the figure understates the query's true footprint by an
     unmeasured amount.
   - `duckdb-cityparquet` rows sample the harness's own Python process, which
     embeds DuckDB and runs the 5 ms sampler thread, so the value includes the
     interpreter and the engine's idle baseline and cached state. Every
     scenario runs on one shared connection, so the value is cumulative across
     the scenario sequence: a light scenario inherits the memory retained by a
     heavier one before it (in the committed 3DBAG CSV, `count`, which reads
     only file metadata, follows the whole-table scan and reports a similar
     peak).
   - Native-reader rows report the child process's `getrusage` high-water
     mark, converted to bytes on every platform (`rss_to_bytes` in
     `benchmark/readbench/src/main.rs`), including its idle baseline.

7. **The native readers answer only five of the fourteen scenarios.**
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
   not identical.** The scenario asks for the objects carrying an LoD 1.2
   geometry, and all three materialise whole rows inside the timed window,
   each carrying a geometry: DuckDB every column (including the object's
   other LoD geometries, which the query never filters on but `SELECT *`
   returns), cjdb the `geometry` JSONB, 3DCityDB the `feature` row joined
   to the LoD-1 solid in `geometry_data`. Three differences remain and are
   not engineered away:
   - **3DCityDB's row carries no attributes** (they live in `property`,
     unjoined), so it returns less than the other two — the same asymmetry
     Caveat 17 records for `id-lookup`.
   - **3DCityDB pays a sort the others do not.** `DISTINCT ON (f.id)`
     keeps the row count CityObject-grained when several `property` rows
     of one feature match; it is scoped to the key rather than to the whole
     row so PostgreSQL never compares WKB geometries for equality. On
     delft it removes nothing, each CityObject owning exactly one
     `lod1Solid`.
   - **3DCityDB's "LoD 1" is wider than CityJSON's "1.2".** `citydb-tool`
     truncates the fractional tier on import, so `val_lod = '1'` covers
     1.2 and 1.3 alike. Where a dataset carries both, its row set is a
     superset of the other two systems' and the cross-check will say so.

   The row also carries a **client-side cost that is not symmetric**: the
   DuckDB rows are fetched to Arrow, while `psycopg` builds Python objects
   for every column of every row — JSONB parsed into dicts on cjdb. On a
   large result set that is real work on the PostgreSQL side of the
   comparison which the DuckDB side does not pay, in the opposite
   direction from most of this harness's asymmetries.

10. **cjdb's footprint is NULL for an object with no geometry of its own.**
    `get_ground_surfaces()` derives a footprint only from the object's own
    `geometry` array and never aggregates its children's. A parent whose
    geometry lives entirely on its children — a legitimate CityJSON shape,
    such as a `Building` whose LoD surfaces are all on `BuildingPart`s — gets
    `ground_geometry` NULL (cjdb logs `No ground surfaces were found`), and
    cjdb's `bbox-query` never returns it. CityParquet's `bbox` unions the
    object's subtree, and 3DCityDB's importer populates `envelope` for such
    parents, so on datasets with this shape cjdb undercounts. No patch is
    applied: this follows from cjdb's per-object design. Scenarios that do
    not use `ground_geometry` are unaffected.

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

12. **PostGIS `&&` returns false positives at EPSG:7415 magnitudes, and
    both PostgreSQL systems are affected in fact.** Serialised PostGIS
    geometries cache a single-precision (float4) bounding box, which the
    `&&` operator uses whether or not an index is involved. One float4 step
    at x ≈ 1.86e5 — ordinary RD New coordinates, not "in the millions" — is
    **0.0156 m**, so an envelope lying a centimetre or two outside the
    window can test as overlapping. An earlier version of this caveat said
    this needed coordinates in the millions and that `cjdb` was affected
    only "in principle"; both are wrong. On the committed 3DBAG run the
    float4 term contributes **+4 objects at the 25 % window on both
    PostgreSQL systems** (Caveat 11's table). `duckdb-cityparquet` and the
    native readers compare double-precision bounds and are not exposed.

    The harness keeps `&&`, the idiomatic, index-cooperating PostGIS form.
    The exact predicate (`ST_Intersects(envelope, env)`, or `&& env AND
ST_Intersects(ST_Envelope(ground_geometry), env)` for cjdb) would remove
    the +4/+3 at the cost of leaving each system's native form and adding
    per-candidate CPU to the timings, for a 3-in-221,005 correction.

13. **3DCityDB's importer restructures CityGML-recognised attributes, which
    can empty `attr-stats`, `attr-filter` and `attr-range`.** All three look
    up `property.name` equal to the literal CityJSON attribute name every
    other system is given. When `citydb-tool` maps a CityJSON attribute onto
    a structured CityGML 3.0 datatype — `measuredHeight` becomes a `height`
    property of type `Height`, with the scalar on a child `property` row
    named `value` — no row has the original name, and 3DCityDB reports 0
    against the others' counts. The queries are not special-cased for such
    names. These scenarios are comparable across systems only when the
    chosen attribute is not such an attribute; the committed 3DBAG run uses
    `b3_extrusie` for `attr-stats`, on which all three systems agree. If a
    re-run shows `attr-range` or `attr-filter` disagreeing for this reason,
    pick the next attribute all three agree on, as `attr-stats` did.

14. **The native reader accepts only single-table packages.**
    `cityparquet convert` writes one table per first-level CityObject family.
    `cityparquet-readbench` refuses a package with more than one object table
    (`benchmark/readbench/src/formats/cityparquet.rs`), so on a multi-family
    dataset every native-reader row that reaches the child is `error:`.
    `duckdb-cityparquet` reads every object table listed with role
    `cityparquet-objects` in `metadata.json`, combined with
    `read_parquet([...], union_by_name = true)`, and is unaffected.

15. **`lod-query` is degenerate on `duckdb-cityparquet` when the package
    lacks the column it reads.** It targets the column `geometry_lod1_2`.
    When a package has no such column, `sql_duckdb.sql_for` emits
    `SELECT * ... WHERE FALSE`, which DuckDB folds at plan time without
    scanning anything, while cjdb and 3DCityDB still execute their queries
    and return no rows. The counts agree, so the cross-check raises
    nothing, but the `duckdb-cityparquet` time on such a row measures no
    work and must not be compared with the others or cited as evidence of
    anything. The committed 3DBAG package has `geometry_lod1_2` (all three
    systems count 500,296), so the row is not degenerate on it.

16. **CityParquet's `bbox` is NULL for an object whose only geometry is a
    `GeometryInstance`.** The writer computes `bbox` from the object's own
    geometry and its descendants' geometry, widened by any declared
    `geographicalExtent` (`lib/cityparquet-rs/crates/core/src/encode.rs`). A
    `GeometryInstance` contributes no extent (`geometry_bbox` in
    `wkb_write.rs`). An object with only template instances in its subtree
    and no declared extent therefore has a NULL `bbox`, and every
    `bbox-query` on CityParquet excludes it, while cjdb and 3DCityDB resolve
    the placed geometry and can include it. Such a row is also outside the
    query-window derivation's denominator, which is recorded as
    `window_rows` in the params sidecar. On the committed 3DBAG slice this
    caveat does not operate: 0 of 1,000,001 rows has a NULL `bbox`.

17. **`id-lookup` returns a different amount of object per system.**
    `duckdb-cityparquet` and `cjdb` return the whole object row, geometry
    included. 3DCityDB returns the `feature` row only: `property` and
    `geometry_data` are not joined, so it hands back an object's identity
    and envelope without its attributes or geometry. That makes 3DCityDB's
    `id-lookup` the least work of the three, and the scenario is one
    CityParquet loses heavily in any case. Read it as "locate a row by id",
    not "materialise a comparable object". `lod-query` narrows the same gap
    without closing it: there 3DCityDB's row does carry the LoD-1 geometry,
    but still not the attributes (Caveat 9).

18. **`geometry-scan` is fairer than the row it replaces, but it is not
    neutral, and a byte length does not prove a decode.** All three systems
    now scan the same thing, which the retired `full-read` did not: that row
    hashed 84 columns on DuckDB, serialised three JSONB/PostGIS columns to
    text on cjdb, and cast whole composite records through two `GROUP BY`
    CTEs over entire tables on 3DCityDB (276 s against 1.2 s, and largely
    artefact — `notes/benchmark-fairness-review-2026-09-22.md` §4.2). Three
    asymmetries remain, and none is removed by this change:
    - **Binary against text.** DuckDB reads stored WKB lengths; both
      PostgreSQL systems serialise to text first (`geometry::text` is a
      JSONB render on cjdb, EWKT on 3DCityDB) and then throw the string
      away. A client reading JSONB genuinely pays that, but it is not the
      same operation.
    - **One DuckDB term is itself a re-serialisation.** CityParquet writes
      the LoD0 footprint with Parquet's own GEOMETRY logical type, which
      DuckDB 1.5 decodes to its native `GEOMETRY` — measured to be true
      **whatever `enable_geoparquet_conversion` is set to**, because that
      setting governs the `geo`-footer path, not the logical type.
      `octet_length` does not bind against it, so that column's term is
      `octet_length(ST_AsWKB(...))`. The other `geometry_lod*` columns are
      BLOB and are read as stored lengths.
    - **A summed byte length does not establish that a geometry was
      decoded** on any of the three.

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
    3DCityDB). Publish the write table beside the read table with this
    caveat attached, exactly as the ingest section already does — not as
    points on one axis. `attr-delete`'s CityParquet `result_count` is a
    definition rather than a measurement (see "The write tier").

20. **`EXPLAIN (ANALYZE, BUFFERS)` doubles the per-sample work on both
    PostgreSQL systems for every read row.** The instrumented re-run is
    outside the timed window, so it does not inflate `time_s` directly, but
    it does mean each PostgreSQL read sample executes its query twice while
    neither DuckDB row does — relevant to cache state between samples, and
    to wall-clock planning of a full run. Write rows are exempt, because
    there the re-run would apply the mutation twice (see "The write tier").

## Running the benchmark

### Through the suite (the path that produced the committed run)

From the repository root, the database family runs through
`benchmark/scripts/bench_suite.py`:

```sh
just bench-prep --families databases   # prepare the 3DBAG slice and packages; build citydb-tool image and patched cjdb
just bench-run  --families databases   # isolated databases, one run
```

`bench-prep` fetches and prepares the largest scaling slice named in
`benchmark/manifest.toml` (`largest_scaling_dataset`, currently
`3dbag_n1000000`) into `benchmark/runs/data/`, and runs `citybench prep`,
which runs `just build-citydb` and `just patch-cjdb`. `bench-run` calls
`citybench run --data-root benchmark/runs --prepared-dir
benchmark/runs/data/readbench --dataset <slice> --output-dir
benchmark/runs/databases/results`. With `--smoke`, the suite uses the
first selected scaling slice (by default `3dbag_n1000`) and writes to
`benchmark/runs/databases/smoke/`.
The suite always uses the largest slice for a full database run. Figures
come from `just bench-summary`, which only reads results.

### A single run with the CLI

From `benchmark/databases/`:

```sh
uv run python -m citybench.cli run \
  --data-root ../runs \
  --prepared-dir ../runs/data/readbench \
  --dataset <path/to/dataset>.city.jsonl \
  [--systems duckdb-cityparquet,duckdb-cityparquet-source,duckdb-cityparquet-writeback,cjdb,3dcitydb] \
  [--repeat 7] [--srid 7415] [--count-tolerance 0.001] \
  [--output-dir <dir>]
```

One invocation measures both thread configurations and then the write
tier; there is no flag to run half of it, because the order is what keeps
the write tier's bloat out of the read rows.

With `--data-root` (which must lie below `benchmark/runs/`),
`lifecycle.isolated_databases`:

- creates two fresh containers named `citybench-cjdb-<uuid>` and
  `citybench-citydb-<uuid>`, each with the resource limits above, published
  on a free `127.0.0.1` port and discovered with `podman port`;
- binds each PostgreSQL data directory to
  `<data-root>/databases/<uuid>/{cjdb,3dcitydb}`;
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
`<prepared-dir>/<dataset>.parquet` (and `<dataset>-hilbert.parquet` for
`cityparquet-hilbert`), for example from
`just readbench-prepare <input> <outdir> cityparquet,cityparquet-hilbert` at
the repository root. Without `--output-dir`, results are written to
`benchmark/runs/databases/results/`. Every run overwrites
`<dataset>.csv`, `<dataset>.manifest.json`, `<dataset>.params.json` and
`<dataset>.indexes.sql`.

`citybench smoke` runs the same pipeline with `repeat = 2` and fails on any
count mismatch or error. Its default dataset,
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
just bench <dataset> [REPEAT]      # citybench run without --data-root: ports 55432/55433
just down
```

`just bench` passes no `--prepared-dir`, so packages are read from
`benchmark/formats/data/readbench/`, a directory the suite does not
populate; place or link the packages there first. `just smoke` has no `--dataset`
argument and therefore needs `benchmark/databases/data/delft.city.jsonl`.
`just capture-schema <dataset>` reads both schemas from these fixed ports and
overwrites `docs/cjdb-schema.md` and `docs/3dcitydb-v5-schema.md` with fresh
table, column and index listings, discarding their hand-written sections.

`just down` runs `podman-compose down -v`, which removes the containers but
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

## Heterogeneity corpus parameter files

`params/` holds parameter files written by `just derive-params` for five
further datasets: `delft.json`, `Montreal.json`, `Vienna.json`, `Zurich.json`
and `lod3_railway.json`. No results for them are committed, and
`citybench run` does not read these files; they are reference outputs of
`citybench.params.derive` for those sources.

**These files predate both the package-derived parameters and the current
sidecar schema**, and no longer match what `derive` produces: it needs the
dataset's CityParquet package as well as its source, and it now writes
`id_probes` and `append` where these files still carry `target_id`.
`params.from_json` will not read them. Regenerating one takes
`just derive-params --dataset <src> --prepared-dir <dir>` with the package
already prepared; that also writes the `<dataset>.append.city.jsonl` the
`append-object` scenario imports, beside the sidecar.

Montreal, Vienna, Zurich and lod3_railway are fetched and checksum-pinned,
not committed:

```sh
./scripts/fetch_corpus.sh [DEST]   # default DEST: data/; verifies against scripts/corpus.sha256
```

`scripts/corpus.sha256` pins the pristine downloads. None of the four
declares `metadata.referenceSystem`, and `cityparquet convert` refuses a
source with coordinates but no CRS, so each is stamped with its EPSG code
before use:

```sh
python3 scripts/stamp_crs.py data/Montreal.city.jsonl      2950
python3 scripts/stamp_crs.py data/Vienna.city.jsonl        31256
python3 scripts/stamp_crs.py data/Zurich.city.jsonl        2056
python3 scripts/stamp_crs.py data/lod3_railway.city.json   7415
```

Stamping rewrites the file in place, so `sha256sum -c scripts/corpus.sha256`
fails against the stamped copies; re-running `fetch_corpus.sh` restores the
pristine bytes, after which stamping must be repeated. The step is idempotent
and, for CityJSONSeq, rewrites only the header line.

| dataset        | EPSG                             | `bbox_full` lower-left corner in WGS 84 | location                                                                                           |
| -------------- | -------------------------------- | --------------------------------------- | -------------------------------------------------------------------------------------------------- |
| `Montreal`     | 2950 (NAD83(CSRS) / MTM zone 8)  | 45.506° N, 73.561° W                    | Montreal                                                                                           |
| `Vienna`       | 31256 (MGI / Austria GK East)    | 48.202° N, 16.345° E                    | Vienna                                                                                             |
| `Zurich`       | 2056 (CH1903+ / LV95)            | 47.323° N, 8.459° E                     | Zurich                                                                                             |
| `lod3_railway` | 7415 (Amersfoort / RD New + NAP) | —                                       | a synthetic scene of about 12 × 7 × 1.5 m near the origin; 7415 only satisfies the CRS requirement |

The corners can be reproduced from `params/<dataset>.json` with `cs2cs
EPSG:<code> EPSG:4326`. `EPSG:31256` uses (northing, easting) axis order, so
Vienna's corner must be given as `miny minx`.

`lod3_railway.city.json` is single-document CityJSON. cjdb accepts only files
ending `.jsonl` and reads line 1 as a CityJSONSeq header, so convert it with
the `cjio` that the patched cjdb depends on:

```sh
uv run --with .cjdb-patched/cjdb-2.2.0+<patch-hash> cjio data/lod3_railway.city.json export jsonl data/lod3_railway.city.jsonl
```

and use the `.jsonl` file for every system. lod3_railway is multi-family and
has no numeric attribute, so the native readers cannot read it (Caveat 14)
and `attr-stats` is `skipped:` on every system. `lod3_railway.city.json` is
also the case `append-object` cannot serve from a single document: derive
its parameters from the exported `.jsonl`, or the row is `skipped:`.

To run one of these datasets, prepare its CityParquet package into the
prepared directory and pass the dataset's EPSG code as `--srid`, for example:

```sh
uv run python -m citybench.cli run --data-root ../runs \
  --prepared-dir ../runs/data/readbench --dataset data/Zurich.city.jsonl --srid 2056
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
