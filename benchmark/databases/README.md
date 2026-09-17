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

The harness measures **steady-state read performance** — wall-clock time,
peak resident memory of the executing process and, for PostgreSQL,
server-reported execution time — of ten access-pattern scenarios against a
dataset already loaded into each system. **Ingest is not compared.**
Encoding a CityParquet package and populating an indexed relational schema
are different operations, not points on one scale. Ingest wall-clock is
recorded in `<dataset>.manifest.json` (never in the results CSV) with this
caveat attached:

> Ingest timings are context only and are NOT comparable across systems.
> Encoding a CityParquet package and populating an indexed relational
> schema are different operations; this benchmark is scoped to
> steady-state query performance.

The CityParquet-based systems read a package prepared beforehand by
`cityparquet convert`, so their `ingest()` is a no-op recorded as `0.0`
seconds: the absence of a load step is the property under discussion, not a
measurement gap.

## Committed evidence

The only committed database results are one run over the 1,000,001-object
3DBAG scaling slice (`3dbag_n1000000`):

| File | Contents |
|---|---|
| `benchmark/runs/databases/results/3dbag_n1000000.csv` | 36 rows: three systems × twelve scenario rows, `repeat` = 7 |
| `benchmark/runs/databases/results/3dbag_n1000000.manifest.json` | source SHA-256, host, versions, `pg_settings`, ingest times, sizes, cjdb patch disclosure, SRIDs, memory scope, temporary storage |
| `benchmark/runs/databases/results/3dbag_n1000000.params.json` | the query parameters derived from that source |
| `benchmark/runs/databases/results/3dbag_n1000000.indexes.sql` | the DDL this harness added, plus a live `pg_indexes` dump for both PostgreSQL schemas |

`benchmark/runs/RESULTS.md` describes the run and its limitations; read it
before citing a number. In brief:

- **Scope.** One dataset, the three default systems (`duckdb-cityparquet`,
  `cjdb`, `3dcitydb`), seven timed samples per row. The native-reader
  systems were not part of the run, so `peak_heap_bytes` is empty on every
  row.
- **Provenance.** The manifest records no Git revision and no timestamp.
- **Count mismatches.** All nine `bbox-query` rows carry `status=mismatch`.
  cjdb returns slightly fewer objects than the other two systems at every
  window, and `duckdb-cityparquet` and `3dcitydb` differ by three objects at
  the 25 % window (221,005 against 221,008). These rows are not citable
  until the counts are reconciled. Caveats 10–12 describe mechanisms
  that produce these kinds of disagreement; none has been confirmed as the
  cause on this dataset.
- Because of those rows the run exited non-zero (`cmd_bench` returns 1 when
  any row has `status` `error` or `mismatch`).

## Systems

| tag | what it is | index support |
|---|---|---|
| `duckdb-cityparquet` | DuckDB (Python client) `read_parquet()` over the source-order CityParquet package `<prepared>/<dataset>.parquet`; no separate ingest | Parquet statistics used by DuckDB's own scan |
| `cjdb` | cjdb 2.2.0, **patched (Caveat 2)**, imported into PostgreSQL/PostGIS. Full geometry is JSONB (`city_object.geometry`); only a 2D footprint is a PostGIS geometry (`ground_geometry`) | cjdb's own defaults plus one added btree(`object_id`) — see "Index sets" |
| `3dcitydb` | 3DCityDB v5.1.2, imported with `citydb-tool` 1.3.2 into PostgreSQL/PostGIS. Generic `feature`/`property`/`geometry_data` schema: CityGML classes are rows, attributes are EAV rows | the indexes `citydb-tool import cityjson` creates; none added |
| `cityparquet` | the native Rust reader over the same source-order package, driven per sample as `cityparquet-readbench --child` | Parquet row-group min/max statistics and column projection |
| `cityparquet-hilbert` | the same reader over `<prepared>/<dataset>-hilbert.parquet`, rows in Hilbert-curve order | the same statistics, with tighter per-row-group bounding boxes |

`citybench run` uses the first three by default. The native readers run only
when named in `--systems` and need the binary
`benchmark/readbench/target/release/cityparquet-readbench`
(`cargo build --release --manifest-path benchmark/readbench/Cargo.toml`).

## Query parameters

Every system receives the **same** parameters, derived deterministically from
the source CityJSON or CityJSONSeq file by `citybench.params.derive` (ties
are broken by sorting). No system derives its own idea of "a 5 % window" or
"a typical building".

| field | derivation |
|---|---|
| `bbox_full` | extent of every dequantised vertex in the file |
| `attr_column` | always `object_type` |
| `attr_eq` | most frequent CityObject type |
| `numeric_column` | most frequent numeric attribute; `null` if none |
| `target_id` | lexicographically first CityObject id |
| `parent_id` | lexicographically first CityObject with `children`; `null` if none |
| `total_city_objects` | the selectivity denominator |

`citybench run` derives the parameters afresh from the source on every run
and writes them beside the CSV as `<dataset>.params.json`; it never reads a
name-keyed file. `just derive-params` writes the same payload to
`params/<dataset>.json` (see "Heterogeneity corpus parameter files").

`bbox-query` uses three windows covering 1 %, 5 % and 25 % of `bbox_full`'s
x/y area, anchored at its lower-left corner (`BBox.window`), matching the
window construction of `benchmark/formats/READ_BENCHMARK.md`. **The window's
z range is never narrowed**: every object is in range vertically, so no
system is ever tested against a z-restricted window, whatever its mechanism
could support.

## The ten scenarios

Each system answers each scenario through its own natural mechanism — never a
hand-tuned shortcut, never a shape contrived to match another system's plan
(the design rule stated in `sql_citydb.py` and `sql_duckdb.py`).

| scenario | common target | `duckdb-cityparquet` | `cjdb` | `3dcitydb` | `cityparquet`(`-hilbert`) |
|---|---|---|---|---|---|
| `full-read` | decode every object; `(count, checksum)` | `SELECT count(*), sum(hash(COLUMNS(*)))::HUGEINT` — DuckDB expands `COLUMNS(*)` into one hash sum per column, forcing every column to be decoded | `count(*)` plus the summed text length of `geometry`, `attributes` and `ground_geometry`, each `coalesce`d so one NULL column cannot drop a row | pre-aggregated `geometry_data` and `property` text lengths joined back to `feature`, with the CityObject predicate (Caveat 1) | scan every row group single-threaded, decode each row's WKB (`cityparquet::query`) |
| `count` | total CityObject count | `SELECT count(*)` | `SELECT count(*) FROM cjdb.city_object` | `count(*)` over `feature` with the CityObject predicate | Parquet file metadata `num_rows` |
| `bbox-query` (1/5/25 %) | objects whose bbox intersects the window | `bbox.xmax/xmin/ymax/ymin` comparisons on the `bbox` STRUCT — **x/y only** | `ground_geometry && ST_MakeEnvelope(...)`, GIST-indexed — 2D by storage (Caveat 3) | `envelope && ST_MakeEnvelope(...)`, GIST-indexed — `envelope` is 3D, but `ST_MakeEnvelope` returns a 2D polygon, so the test is 2D | row-group pruning plus a row-level test of all six bounds |
| `attr-filter` | objects with `object_type = attr_eq` | `WHERE object_type = ?` | `WHERE "type" = %s` (btree) | `objectclass_id` = the class id for `attr_eq` (btree) | Arrow row filter plus row-group statistics |
| `attr-stats` | `(count, min, max, sum)` of `numeric_column` | aggregates over the flattened top-level column | aggregates over `(attributes->>col)::numeric` — every row's JSONB unpacked | EAV join `property`→`feature` on `name = col`, aggregating `coalesce(val_double, val_int)` because an integer value is stored in `val_int` (see also Caveat 13) | column-chunk statistics for min/max; projected scan for sum/count |
| `id-lookup` | the row for `target_id`, materialised | `SELECT * WHERE id = ?` — the whole object row, geometry included; no index | `SELECT * WHERE object_id = %s` (added btree) — the row includes the geometry JSONB | `SELECT * FROM feature WHERE objectid = %s` (btree) — the `feature` row only; `property` and `geometry_data` are not joined | row filter on `id`, decode the surviving row |
| `project` | one column read across every row; non-null count | `SELECT count(object_type)` | `SELECT count("type")` | `count(objectclass_id)` with the CityObject predicate | single-column projection |
| `lod-extract` *(SQL only)* | objects carrying an LoD 1.2 geometry | `count(geometry_lod1_2) WHERE geometry_lod1_2 IS NOT NULL` — one column projected; `WHERE FALSE` when the package has no such column (Caveat 15) | `geometry @? '$[*] ? (@.lod == "1.2")'` — the `@?` operator, which uses cjdb's GIN(`geometry`) index; the `jsonb_path_exists` function form does not | `property` join on `val_lod = '1' AND val_geometry_id IS NOT NULL` — the importer stores LoD 1.2 as `'1'` (`docs/3dcitydb-v5-schema.md`, "LoD value format") | not run (Caveat 7) |
| `semantic-surface` *(SQL only)* | objects with ≥ 1 `RoofSurface`, **any LoD** (Caveat 9) | `OR` of `list_contains(json_extract_string(<col>.surfaces, '$[*].type'), 'RoofSurface')` over every `geometry_properties_lod*` column the package has | `geometry @? '$[*].semantics.surfaces[*] ? (@.type == "RoofSurface")'` | `count(DISTINCT pr.feature_id)` over owners of a `RoofSurface` feature — a presence test, not a surface-row count | not run |
| `hierarchy` *(SQL only)* | direct children of `parent_id` | `WHERE list_contains(parents, ?)` | join `city_object_relationships` to the parent's `object_id` | `property.val_feature_id` join from parent to child, CityObject predicate on the child | not run |

`bbox-query` produces one row per window, tagged `bbox-1pct`, `bbox-5pct` or
`bbox-25pct` in `notes`, so the seven Tier-1 scenarios give nine rows per
system and each Tier-2 scenario one more row per SQL system: twelve rows for
each SQL system, nine for each native reader. A scenario the dataset cannot
answer (no numeric attribute for `attr-stats`, no parent/child pair for
`hierarchy`) is recorded as `skipped: ...`, not as an error.

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

| setting | cjdb | 3dcitydb |
|---|---|---|
| `shared_buffers` | 8GB | 8GB |
| `effective_cache_size` | 24GB | 24GB |
| `work_mem` | 256MB | 256MB |
| `random_page_cost` | 1.1 | 1.1 |
| `max_parallel_workers` | 16 | 16 |
| `max_parallel_workers_per_gather` | 0 (benchmark session) | 0 (benchmark session) |

Both adapters call `pg.disable_parallel_query`, which sets
`max_parallel_workers_per_gather = 0` on the benchmark connection, so every
timed PostgreSQL query executes in a single backend process and its resident
memory is attributable to that one PID. This overrides the configuration
file's value of 16. `cli.py` stamps `"0 (benchmark session)"` into the
manifest rather than reading it back from the benchmark session. DuckDB runs
with `SET threads TO 16` and `SET memory_limit = '32GB'`
(`systems/duckdb_cp.py`). See Caveat 5.

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
- `duckdb-cityparquet` times `execute(...).fetchall()` in-process.
- The native readers start a fresh child process per sample, and the harness
  uses the child's own reported elapsed time, so process start-up is
  excluded.

### Count cross-check

The count cross-check is the harness's main defect detector
(`citybench.runner`): every scenario's result count is compared across every
system that answered it, for the same parameters. When systems disagree, at
least one is answering a different question and its timing is meaningless
until reconciled. Every row for that scenario (and window) is tagged
`count-mismatch: <system=count ...>` in `notes` and receives
`status=mismatch`. `skipped:` and `error:` rows carry no count and are
excluded from the comparison. `citybench run` exits non-zero if any row has
`status` `error` or `mismatch`; `citybench smoke` additionally fails on any
`count-mismatch` or `error:` note.

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

`<dataset>.csv` has one row per (system, scenario[, window]) and nineteen
columns:

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

- **`selectivity`** — `result_count / total_city_objects`; empty for `count`
  and `full-read`. The window's area target is in `notes`, not here.
- **`time_s` / `time_mad_s`** — see "The warm protocol".
- **`peak_heap_bytes`** — populated only for the native readers (the child's
  allocator high-water mark); empty for every SQL system.
- **`peak_rss_bytes`** — peak resident set size of the process executing the
  query, in bytes, the maximum over the timed samples (Caveat 6). The
  `notes` column states the scope (`memory-scope: duckdb-process-rss` or
  `memory-scope: postgresql-backend-rss`), and the manifest's
  `memory_measurement` block describes it.
- **`repeat`** — the number of timed samples in that row.
- **`notes`** — memory scope, window tag, `count-mismatch: ...`,
  `skipped: ...` or `error: <ExceptionType>`.
- **`bytes_read` / `http_requests`** — always empty (Caveat 8).
- **`server_time_s`** — empty for the in-process systems; for `cjdb` and
  `3dcitydb`, the mean of PostgreSQL's reported `Execution Time` from the
  `EXPLAIN (ANALYZE, BUFFERS)` re-runs (`pg.time_query`). Raw values are in
  `raw_server_time_samples_s`.
- **`size_bytes` / `size_bytes_no_index`** — see "Two size figures".
- **`status`** — `ok`, `mismatch`, `skipped` or `error`.

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

5. **PostgreSQL runs every query without parallel workers; DuckDB uses 16
   threads.** Parallel query is disabled per session to keep resident memory
   attributable to one backend process (see "Tuning and parallelism"). Both
   PostgreSQL containers are allowed 16 CPUs, but a single benchmark query
   uses one backend. Timings therefore compare single-process PostgreSQL
   execution with multi-threaded DuckDB execution.

6. **`peak_rss_bytes` has a different process scope per system and is not a
   like-for-like comparison.**
   - PostgreSQL rows sample the query's backend process, located on the host
     through `podman top` or a verified PID-namespace mapping, every 5 ms
     from `/proc/<pid>/status`. The value excludes other backends,
     background workers and the idle server, but mapped shared pages
     (including touched `shared_buffers`) can contribute. It is blank when
     the PID cannot be mapped safely.
   - `duckdb-cityparquet` rows sample the harness's own Python process, which
     embeds DuckDB and runs the 5 ms sampler thread, so the value includes the
     interpreter and the engine's idle baseline and cached state. Every
     scenario runs on one shared connection, so the value is cumulative across
     the scenario sequence: a light scenario inherits the memory retained by a
     heavier one before it (in the committed 3DBAG CSV, `count`, which reads
     only file metadata, follows `full-read` and reports a similar peak).
   - Native-reader rows report the child process's `getrusage` high-water
     mark, converted to bytes on every platform (`rss_to_bytes` in
     `benchmark/readbench/src/main.rs`), including its idle baseline.

7. **Tier-2 scenarios have no native-reader rows.** `lod-extract`,
   `semantic-surface` and `hierarchy` run only on the SQL systems
   (`registry.SQL_SYSTEMS`); `cityparquet-readbench --child` implements only
   the seven Tier-1 scenarios. `build_child_args` raises `ValueError` for a
   Tier-2 name, and `cli._run_all_scenarios` runs the two tiers as separate
   matrices so this never appears as an `error:` row.

8. **`bytes_read` and `http_requests` are always empty.** Every system reads
   local disk or a localhost socket. The HTTP and object-storage comparison
   belongs to the format harness (`--transport http`), and neither
   PostgreSQL system has an object-storage access path.

9. **`semantic-surface` is any-LoD by choice.** A LoD-scoped query is
   expressible on 3DCityDB (the boundary surface's `lod1MultiSurface` /
   `lod2MultiSurface` property rows carry `val_lod`), but it is not part of
   the scenario set. Any-LoD asks the more natural question — "does this
   object have a classified roof surface at all" — and avoids choosing a LoD
   that one storage model represents more richly than another, in a
   benchmark where CityParquet is a participant. 3DCityDB's query counts
   distinct owning objects, because each owner can have several
   `RoofSurface` rows (one per solid); counting rows would answer a different
   question.

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
    footprint.** After discarding (near-)vertical faces,
    `get_ground_surfaces()` keeps only faces whose mean Z is strictly below
    the mean of the remaining distinct Z values. An object whose non-vertical
    faces share a single Z value — for example a small roof feature modelled
    as walls plus one `RoofSurface` — keeps nothing, so `ground_geometry` is
    NULL even though the object has geometry. For flat, elongated geometry
    with little Z variation (for example railway track surfaces) the split
    can keep only part of the horizontal extent, producing an undersized
    footprint. Either way cjdb's `bbox-query` undercounts. Neither case is
    the tie bug the patch fixes, and neither is patched.

12. **PostGIS `&&` can return false positives at large coordinate
    magnitudes.** Serialised PostGIS geometries cache a single-precision
    (float4) bounding box, which the `&&` operator uses whether or not an
    index is involved. An envelope lying a few centimetres outside the query
    window can therefore test as overlapping when coordinates are in the
    millions (for example CH1903+/LV95), where one float4 step exceeds a few
    centimetres. `3dcitydb` (and, in principle, `cjdb`) use `&&`;
    `duckdb-cityparquet` and the native readers compare double-precision
    bounds and are not exposed. The harness keeps `&&`, the idiomatic,
    index-cooperating PostGIS form, rather than switching to `ST_Intersects`.

13. **3DCityDB's importer restructures CityGML-recognised attributes, which
    can empty `attr-stats`.** `attr-stats` looks up `property.name` equal to
    the literal CityJSON attribute name every other system is given. When
    `citydb-tool` maps a CityJSON attribute onto a structured CityGML 3.0
    datatype — `measuredHeight` becomes a `height` property of type `Height`,
    with the scalar on a child `property` row named `value` — no row has the
    original name, and 3DCityDB reports 0 against the others' counts. The
    query is not special-cased for such names. `attr-stats` is comparable
    across systems only when `numeric_column` is not such an attribute; the
    committed 3DBAG run uses `b3_extrusie`, on which all three systems agree.

14. **The native reader accepts only single-table packages.**
    `cityparquet convert` writes one table per first-level CityObject family.
    `cityparquet-readbench` refuses a package with more than one object table
    (`benchmark/readbench/src/formats/cityparquet.rs`), so on a multi-family
    dataset every native-reader row that reaches the child is `error:`.
    `duckdb-cityparquet` reads every object table listed with role
    `cityparquet-objects` in `metadata.json`, combined with
    `read_parquet([...], union_by_name = true)`, and is unaffected.

15. **`lod-extract` and `semantic-surface` are degenerate on
    `duckdb-cityparquet` when the package lacks the columns they read.**
    `lod-extract` targets the column `geometry_lod1_2`. When a package has no
    such column, `sql_duckdb.sql_for` emits `SELECT count(*) ... WHERE FALSE`,
    which DuckDB folds at plan time without scanning anything, while cjdb and
    3DCityDB still execute their queries and return 0. The counts agree, so
    the cross-check raises nothing, but the `duckdb-cityparquet` time on such
    a row measures no work and must not be compared with the others or cited
    as evidence for projection pushdown. `semantic-surface` does the same
    when a package has no `geometry_properties_lod*` column. The committed
    3DBAG package has `geometry_lod1_2` (all three systems count 500,296), so
    its `lod-extract` row is not degenerate.

16. **CityParquet's `bbox` is NULL for an object whose only geometry is a
    `GeometryInstance`.** The writer computes `bbox` from the object's own
    geometry and its descendants' geometry, widened by any declared
    `geographicalExtent` (`lib/cityparquet-rs/crates/core/src/encode.rs`). A
    `GeometryInstance` contributes no extent (`geometry_bbox` in
    `wkb_write.rs`). An object with only template instances in its subtree
    and no declared extent therefore has a NULL `bbox`, and every
    `bbox-query` on CityParquet excludes it, while cjdb and 3DCityDB resolve
    the placed geometry and can include it.

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
  [--systems duckdb-cityparquet,cjdb,3dcitydb] [--repeat 7] [--srid 7415] \
  [--output-dir <dir>]
```

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

| dataset | EPSG | `bbox_full` lower-left corner in WGS 84 | location |
|---|---|---|---|
| `Montreal` | 2950 (NAD83(CSRS) / MTM zone 8) | 45.506° N, 73.561° W | Montreal |
| `Vienna` | 31256 (MGI / Austria GK East) | 48.202° N, 16.345° E | Vienna |
| `Zurich` | 2056 (CH1903+ / LV95) | 47.323° N, 8.459° E | Zurich |
| `lod3_railway` | 7415 (Amersfoort / RD New + NAP) | — | a synthetic scene of about 12 × 7 × 1.5 m near the origin; 7415 only satisfies the CRS requirement |

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
and `attr-stats` is `skipped:` on every system.

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
