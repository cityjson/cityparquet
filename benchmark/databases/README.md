# CityParquet vs cjdb vs 3DCityDB v5: benchmark harness

Cross-system benchmark comparing CityParquet — via its native Rust reader
(`cityparquet`/`cityparquet-hilbert`) and via DuckDB (`duckdb-cityparquet`)
— against two PostgreSQL-based 3D city model databases, **cjdb** and
**3DCityDB v5**. Same discipline as `benchmark/formats/READ_BENCHMARK.md`
(real fixtures, warm medians + MAD at 6-decimal precision, disclosed rather
than hidden overheads), extended with what a cross-**system** (not just
cross-**format**) comparison additionally has to control for: server
tuning, resource limits, index parity, and the client-server boundary two
of the five systems sit behind.

Runtime is rootless **podman** throughout (`podman-compose`), never docker.

## Purpose and claim

This harness measures **steady-state read performance** — wall-clock time,
and (where meaningful) memory and server-side execution time — of ten
access-pattern scenarios run against a dataset already loaded into each
system. **Ingest is deliberately not compared.** Encoding a CityParquet
package and populating an indexed relational schema are different
operations, not points on the same scale, so putting their timings side by
side as a "which is faster" claim would be indefensible. Ingest wall-clock
is still recorded — in `results/<dataset>.manifest.json`, not the results
CSV — but tagged with an explicit caveat:

> Ingest timings are context only and are NOT comparable across systems.
> Encoding a CityParquet package and populating an indexed relational
> schema are different operations; this benchmark is scoped to
> steady-state query performance.

For context only, on `delft.city.jsonl` (2231 CityObjects): cjdb's import
took 10.6s, 3DCityDB's `citydb-tool import cityjson --threads=4` took
9.1s (both read from the currently committed `results/delft.manifest.json`'s
`ingest.wall_clock_s`), and the three CityParquet-based systems (`cityparquet`,
`cityparquet-hilbert`, `duckdb-cityparquet`) all read a package written
ahead of time by `cityparquet convert`, so their `ingest()` step is a
no-op — recorded as `0.0` wall-clock with a `"no load step"` note, which is
the honest figure for "the absence of a load step is the property under
discussion, not a measurement gap."

## Systems

| tag | what it is | index support |
|---|---|---|
| `cityparquet` | `cityparquet-rs`'s native Rust reader, package written in **source row order** (`cityparquet convert --overwrite`), driven as a subprocess via `cityparquet-readbench --child` | Parquet row-group min/max statistics + column projection; no secondary index |
| `cityparquet-hilbert` | the same package, rows written in **Hilbert-curve order** (`--ordering hilbert`) | the same Parquet statistics, but tighter per-row-group bboxes from spatial clustering |
| `duckdb-cityparquet` | DuckDB (Python client, v1.5.5) `read_parquet()` directly over the **same** `cityparquet` package `cityparquet` above reads — no separate ingest | whatever Parquet statistics DuckDB's own scan uses; same file, different engine |
| `cjdb` | cjdb 2.2.0 — **patched, see Caveat 2** — imported into PostgreSQL 16.4/PostGIS 3.4. Full geometry kept as JSONB (`city_object.geometry`); only a 2D footprint is a PostGIS geometry (`ground_geometry`) | GIST(`ground_geometry`) ×2, btree(`type`), GIN(`geometry`), btree(`cj_metadata_id, object_id`), relationship btrees — all cjdb's own defaults — plus one added btree(`object_id`) (see "Index sets" below) |
| `3dcitydb` | 3DCityDB v5.1.2, imported via `citydb-tool` 1.3.2 into PostgreSQL 16.4/PostGIS 3.4. Generic `feature`/`property`/`geometry_data` schema (CityGML classes are rows, not tables; attributes are EAV rows) | 59 indexes created automatically by `citydb-tool import cityjson`, none added — see "Index sets" |

## Query parameters

Every system is handed the **same** query parameters, derived once from
the source CityJSON file and committed to `params/<dataset>.json`
(`citybench.params.derive`, deterministic — ties broken by sorting the
candidate values). This is the mechanism that makes the comparison honest:
no system derives its own idea of "a 5% window" or "a typical building".
For `delft.city.jsonl` (`params/delft.json`):

```json
{
  "attr_column": "object_type",
  "attr_eq": "BuildingPart",
  "numeric_column": "b3_h_dak_50p",
  "target_id": "NL.IMBAG.Pand.0503100000000010",
  "parent_id": "NL.IMBAG.Pand.0503100000000010",
  "total_city_objects": 2231,
  "bbox_full": { "minx": 84501.553625, "miny": 445805.024, "minz": -3.7469973754882844,
                 "maxx": 85675.230625, "maxy": 446983.477, "maxz": 95.04200262451172 }
}
```

`bbox-query`'s three windows (1%/5%/25% of `bbox_full`'s own x/y area) are
derived from this same `bbox_full`, anchored at its lower-left corner —
the same window construction `benchmark/formats/READ_BENCHMARK.md`'s
harness uses, so selectivity tags stay comparable across the two
benchmarks. **The window's z range is never narrowed** (`BBox.window()`
always keeps `minz`/`maxz` at the dataset's full extent) — every object is
in range vertically by construction. This matters for reading the
`bbox-query` mechanisms below: no system in this harness is ever actually
tested against a z-restricted window, regardless of whether its own
mechanism is capable of one.

## The ten scenarios

Every system answers each scenario via its own natural mechanism — never a
hand-tuned shortcut, never a shape contrived to match another system's
plan (`sql_citydb.py`'s and `sql_cjdb.py`'s own module docstrings state
this as a design rule, not just an aspiration):

| scenario | common target | `cityparquet`(`-hilbert`) mechanism | `duckdb-cityparquet` mechanism | `cjdb` mechanism | `3dcitydb` mechanism |
|---|---|---|---|---|---|
| `full-read` | decode every object; `(count, checksum)` | scan all row groups, decode WKB | `SELECT count(*), sum(hash(COLUMNS(*)))::HUGEINT` — DuckDB expands `COLUMNS(*)` into one hash **per column** (77 on delft's committed `building.parquet`, verified live via `DESCRIBE`; the resulting query row is 78-wide, `count(*)` plus one hash sum per column), forcing every one decoded | `SELECT count(*), sum(length(geometry::text)+length(attributes::text)+length(ground_geometry::text))` — forces all three substantial columns, each `coalesce`d so one NULL column can't zero out a row | pre-aggregated `geometry_data`/`property` CTEs (whole-row `::text` casts, forcing `geometry_properties` and every `val_*` column) joined back to `feature`; `count(*)` stays CityObject-granular (a naive join gave 3347, not 2231 — see Caveat 1) |
| `count` | total CityObject count | Parquet file metadata `num_rows` (O(1), no scan) | `SELECT count(*) FROM read_parquet(...)` | `SELECT count(*) FROM cjdb.city_object` | `SELECT count(*) FROM feature WHERE <CityObject-granularity predicate>` (see Caveat 1) |
| `bbox-query` (1%/5%/25%) | count of objects whose bbox intersects the query window | row-group prune (`with_bbox_row_groups`) + row-level test of **all six** bounds (x/y/z) — exact | `WHERE bbox.xmax>=? AND bbox.xmin<=? AND bbox.ymax>=? AND bbox.ymin<=?` against the typed `bbox` STRUCT — **x/y only**, no z clause, even though `bbox` itself carries `zmin`/`zmax` | `WHERE ground_geometry && ST_MakeEnvelope(minx,miny,maxx,maxy,srid)`, GIST-indexed — 2D **by storage**: `ground_geometry` has no z ordinate to compare, ever (Caveat 3) | `WHERE envelope && ST_MakeEnvelope(minx,miny,maxx,maxy,srid)`, GIST-indexed — `envelope` itself is a genuine 3D `geometry(GeometryZ)` (verified: `ST_NDims`→3), but `ST_MakeEnvelope(xmin,ymin,xmax,ymax,srid)` produces a plain 2D polygon (verified: `ST_NDims`→2), so this comparison is 2D too |
| `attr-filter` | count with `object_type = 'BuildingPart'` | `RowFilter` (`ArrowPredicateFn`) + row-group statistics prune | `WHERE object_type = ?` | `WHERE "type" = ?` (btree `city_object_type_idx`) | `WHERE objectclass_id = (SELECT id FROM objectclass WHERE classname=?)` (btree `feature_objectclass_inx`) |
| `attr-stats` | `(count, min, max, sum)` of `b3_h_dak_50p` | min/max from Parquet column-chunk statistics (near-free); sum/count from a 1-column projected scan | `SELECT count(c),min(c),max(c),sum(c)` on the bare flattened column (no `attributes` struct to qualify through) | `SELECT count(x),min(x),max(x),sum(x) FROM (... (attributes->>'b3_h_dak_50p')::numeric ...)` — every row's JSONB unpacked, unconditionally | `SELECT count(coalesce(val_double,val_int)),... FROM property JOIN feature ON ... WHERE name=?` — an EAV join, no wide attributes column exists in v5; coalesced across both numeric columns since the same attribute name can land in either depending on its JSON value's own type (Task 14 — see Caveat 11) |
| `id-lookup` | the one object named by `target_id`, materialised | `RowFilter` on `id` + decode of the one surviving row | `WHERE id = ?` | `WHERE object_id = ?` (btree `ix_co_object_id` — added, see "Index sets") | `WHERE objectid = ?` (btree `feature_objectid_inx`, pre-existing) |
| `project` | one column read across every row; non-null count | single-column `ProjectionMask` | `SELECT count(object_type)` | `SELECT count("type")` | `SELECT count(objectclass_id) FROM feature WHERE <predicate>` |
| `lod-extract` *(SQL-only)* | count of objects carrying an LoD1(.2) geometry | **not run** — Tier-2, see Caveat 5 | `SELECT count(geometry_lod1_2) FROM ... WHERE geometry_lod1_2 IS NOT NULL` — a single-column projection, the LoD2 column's bytes never read. **True only on `delft`** — the other 4 of 5 datasets have no `geometry_lod1_2` column at all, so this degenerates to a compile-time-constant `WHERE FALSE` that DuckDB never scans for at all; see **Caveat 17**, which this claim has no publication-grade evidence for outside `delft` | `WHERE geometry @? '$[*]?(@.lod=="1.2")'` — the `@?` jsonpath **operator**, not the `jsonb_path_exists(...)` function: verified by `EXPLAIN` that only the operator form routes through the GIN(`geometry`) index; the function form seq-scans even with the index present | `JOIN property ON ... WHERE val_lod='1' AND val_geometry_id IS NOT NULL` — `'1'`, not CityJSON's `'1.2'`: v5's importer truncates the fractional LoD tag on import (confirmed: `val_lod='1.2'` matches zero rows, silently) |
| `semantic-surface` *(SQL-only)* | count of objects with ≥1 `RoofSurface` semantic, **any LoD** | not run | `OR`-chain of `list_contains(json_extract_string(geometry_properties_lod*.surfaces,'$[*].type'),'RoofSurface')` across every LoD column the package carries | `WHERE geometry @? '$[*].semantics.surfaces[*]?(@.type=="RoofSurface")'` — iterates the object's whole geometry JSONB array, every LoD at once, unconditionally | `count(DISTINCT pr.feature_id)` over a `property`/`feature` join — a **presence** check, not a raw RoofSurface-row count (see Caveat 8) |
| `hierarchy` *(SQL-only)* | count of `parent_id`'s direct children | not run | `WHERE list_contains(parents, ?)` — `parents` is a `VARCHAR[]` per row, not a scalar column | `JOIN city_object_relationships r ON ... WHERE parent.object_id = ?` | `JOIN property pr ON pr.val_feature_id=child.id JOIN feature parent ON ... WHERE parent.objectid=?`, CityObject-granularity predicate applied to the `child` alias |

`bbox-query` is measured at three selectivity targets (1%/5%/25% of the
dataset's own bbox area), one CSV row per target, tagged `bbox-1pct` /
`bbox-5pct` / `bbox-25pct` in `notes` — so the seven Tier-1 scenario names
above expand to **9** rows per system (`bbox-query` alone contributes 3).
All five systems run the full Tier-1 set; the three Tier-2 names each add
one further row, but only for the three SQL systems (see Caveat 5) — 9
rows for `cityparquet`/`cityparquet-hilbert`, 12 for
`duckdb-cityparquet`/`cjdb`/`3dcitydb`, 54 rows total for the whole matrix
(confirmed against the committed `results/delft.csv`: 9+9+12+12+12 = 54
data rows).

## Fairness controls

### Engine parity

Both PostgreSQL services run **the same engine version**, confirmed live,
not just configured to:

```
$ psql -h localhost -p 55432 -c "SELECT version();"   # cjdb
PostgreSQL 16.4 (Debian 16.4-1.pgdg110+2) on x86_64-pc-linux-gnu, ...
$ psql -h localhost -p 55433 -c "SELECT version();"   # 3dcitydb
PostgreSQL 16.4 on x86_64-pc-linux-musl, ...
```

Both report `server_version` `16.4` and `PostGIS_Version()`
`3.4 USE_GEOS=1 USE_PROJ=1 USE_STATS=1` (`docs/cjdb-schema.md`,
`docs/3dcitydb-v5-schema.md`). The 3DCityDB image is pinned to
`3dcitydb/3dcitydb-pg:16-3.4-5.1.2-alpine` specifically for this: the
floating `5-alpine` tag resolves to PostgreSQL 18 / PostGIS 3.6, which
would have made any measured difference partly PostgreSQL-16-vs-18 rather
than purely cjdb-vs-3DCityDB.

### Tuning parity

`docker/postgresql.conf` is mounted **identically** into both containers
(`docker/compose.yml`'s shared `x-pg-common` anchor) — not just
configured the same way, but the literal same file. Verified by reading
the values back out of both **running** servers (`pg_settings`), not by
re-reading the config:

| setting | cjdb (live) | 3dcitydb (live) | conf file |
|---|---|---|---|
| `shared_buffers` | 1,048,576 × 8kB = 8GB | 1,048,576 × 8kB = 8GB | `8GB` |
| `effective_cache_size` | 3,145,728 × 8kB = 24GB | 3,145,728 × 8kB = 24GB | `24GB` |
| `work_mem` | 262,144kB = 256MB | 262,144kB = 256MB | `256MB` |
| `random_page_cost` | 1.1 | 1.1 | `1.1` |
| `max_parallel_workers` | 16 | 16 | `16` |
| `max_parallel_workers_per_gather` | 16 | 16 | `16` |

(`shared_buffers`/`effective_cache_size` are `GUC_UNIT_BLOCKS` GUCs —
`pg_settings.setting` reports the raw block count and `.unit` reports
`8kB`; the table above is the multiplied-out, human value, cross-checked
directly against `pg_settings` on both live containers.) Stock PostgreSQL
defaults (128MB `shared_buffers`) would make either database a strawman on
this hardware, so both are tuned identically, and the manifest records
`pg_settings` on every run so a published number can always be cited
against them (`results/<dataset>.manifest.json`'s `pg_settings` object).
`DuckDBCityParquet` is configured to match on the two dimensions that
apply to it: `SET threads TO 16` and `SET memory_limit = '32GB'`
(`src/citybench/systems/duckdb_cp.py`).

**I4 (final whole-branch review): `max_parallel_workers` alone is the
wrong setting to cite for CPU parity — it is only the cluster-wide worker
pool; the setting that actually binds PER QUERY is
`max_parallel_workers_per_gather` (leader process + this many parallel
workers).** An earlier version of this table named only
`max_parallel_workers` (`16`, matching) while `docker/postgresql.conf`
left `max_parallel_workers_per_gather` at PostgreSQL's stock default of
`4` — meaning a single PostgreSQL query could use at most leader+4 = 5 of
the 16 CPUs both containers are cgroup-limited to (see "Resource limits"
below), while `duckdb_cp.py` gives `DuckDBCityParquet` all 16 threads for
every query. Both containers have the SAME measured 16-core quota, so this
was an asymmetry in how much of that already-equal, already-allotted
budget a single query could actually use — not a difference in what
either system was GIVEN. Raised to `16` (this table's current value) so
PostgreSQL can use the same per-query CPU budget it already has cgroup
access to; capping DuckDB down to leader+4 instead would have been the
unfair direction, since DuckDB was never over-allotted anything — it was
simply given the resource it was configured to be given. Both figures
(`max_parallel_workers` and `max_parallel_workers_per_gather`) are now
captured in the manifest's `pg_settings` block on every run
(`src/citybench/cli.py`'s `_pg_settings()`), not just the cluster-wide one.

### Resource limits — bind, and measured under load

`compose.yml`'s `deploy.resources.limits` (`cpus: "16"`, `memory: 32G`) is
a Compose `deploy:` block, whose support outside Swarm is inconsistent
across Compose implementations. Verified directly on the running
containers rather than assumed from the config:

```
$ podman exec citybench-cjdb   cat /sys/fs/cgroup/cpu.max /sys/fs/cgroup/memory.max
1600000 100000
34359738368
$ podman exec citybench-citydb cat /sys/fs/cgroup/cpu.max /sys/fs/cgroup/memory.max
1600000 100000
34359738368
```

`cpu.max` = `1600000 100000` — a 1,600,000µs quota per 100,000µs period, a
16-core CPU-bandwidth cap — and `memory.max` = `34359738368` bytes, exactly
32GiB, identical on both containers. `nproc` inside either container
reports 128 (the host's full core count) — CPU *affinity*, unaffected by
the bandwidth quota — which is exactly why `citydb-tool import cityjson`
needs an explicit `--threads=4`: its default thread pool reads `nproc`,
not the quota, and would otherwise blow past `postgresql.conf`'s
`max_connections = 20`. **Enforcement was confirmed under load, not just
inferred from the cgroup files** (`docs/3dcitydb-v5-schema.md`): 32
concurrent CPU-bound workers for 5 seconds drove `cpu.stat`'s
`nr_throttled` from 0 to 50 throttled periods on both containers (≈53s
accumulated throttled time on `cjdb-db`, ≈74s on `citydb-db`), over that
same 5-second burst — direct evidence the kernel's CFS bandwidth
controller actively caps both, symmetrically.

### Index sets

Every scenario's index requirement was checked against what each system
**already builds unasked**, and only what was genuinely missing was added
— the opposite failure mode (adding a same-shape index under a new name)
was found and fixed for cjdb during an earlier pass and would have
silently inflated `size_bytes`, itself a published metric:

- **cjdb needed exactly one added index**: `btree(object_id)`
  (`ix_co_object_id`, `results/delft.indexes.sql`). cjdb's own default
  unique index is composite, `(cj_metadata_id, object_id)` — under the
  leading-column rule this cannot serve a bare `WHERE object_id = ?`
  probe (`id-lookup`): verified by `EXPLAIN`, the composite index costs
  0.28..44.52 / 24 buffer hits (every row shares one `cj_metadata_id`, so
  the leading column doesn't discriminate) versus 0.28..2.50 / 3 buffer
  hits with the dedicated index. Everything else `id-lookup`/`bbox-query`/
  `attr-filter`/`lod-extract`/`semantic-surface`/`hierarchy` need — GIST
  ×2 on `ground_geometry`, btree(`type`), GIN(`geometry`), the two
  relationship btrees — cjdb already creates on import; four of an
  earlier draft's seven hand-written indexes duplicated these and were
  removed, since a duplicate index object has no query benefit but does
  inflate `size_bytes`.
- **3DCityDB needed none**: `citydb-tool import cityjson` creates
  3DCityDB's own fixed set of 16 "content indexes" automatically as part
  of import — `SELECT count(*) FROM pg_indexes WHERE schemaname='citydb'`
  read **59** immediately after import, and **59 again, unchanged**, after
  separately invoking `citydb index create` — confirming it is a genuine
  no-op here, not a step this harness's `ingest()` needs to call. Every
  column a scenario filters, joins, or aggregates on is already covered
  (`feature_objectid_inx`, `feature_objectclass_inx`,
  `feature_envelope_spx` GIST, `property_name_inx`,
  `property_val_geometry_fkx`, `property_feature_fkx` + `feature_pk`) —
  see `docs/3dcitydb-v5-schema.md`'s "Index coverage" section and
  `sql_citydb.index_ddl()`'s docstring for the full, `EXPLAIN`-verified
  column-by-column mapping. **C1 (final whole-branch review) corrected an
  earlier claim here**: this section used to say "no faster index-driven
  plan exists for the CityObject-granularity predicate's `OR`-across-two-
  subqueries shape at this data scale". That measurement genuinely showed
  the OLD correlated predicate could not be forced onto an index inside a
  JOIN (`attr-stats`/`lod-extract`/`hierarchy`'s child side) — it did NOT
  show that no better-shaped predicate existed for the same result. One
  does: the predicate is now resolved to a static, pre-computed id list
  ONCE (`sql_citydb.resolve_cityobject_class_ids()`) and rendered as a
  plain, sargable `objectclass_id IN (...)`, which the planner routes
  through `feature_objectclass_inx` on its own under default settings —
  eliminating every scenario's `feature` seq scan, still with `index_ddl()`
  correctly adding nothing (the existing index already covers it). Measured
  live on Zurich: `count` 32.2x faster, `project` 25.6x, with identical
  counts throughout. See `sql_citydb.index_ddl()`'s own docstring for the
  full, corrected, `EXPLAIN`-verified picture.

`results/<dataset>.indexes.sql` is committed alongside every run's other
artefacts. **I7 (final whole-branch review)**: an earlier version of this
artefact recorded ONLY what this harness's own `index_ddl()` functions
added on top of each system's defaults — for `3dcitydb` that is always an
empty list, so the committed file carried effectively one real line
(`cjdb`'s single added index) and could not support an index-parity audit
at all, despite this project's own spec promising "the exact DDL each
system ran". Fixed: the file now also dumps `pg_indexes` LIVE for both
`cjdb`'s and `3dcitydb`'s schemas at run time (`pg.dump_indexes`,
`cli._indexes_sql`) — the complete, real index set each system is
actually running its scenario queries against, self-built defaults
included, so the exact DDL applied really is auditable, not merely
asserted.

### `VACUUM ANALYZE`

Both PostgreSQL systems' `ingest()` ends with `VACUUM ANALYZE` over every
table in the schema (`pg.vacuum_analyze`) before any timed scenario runs —
refreshed planner statistics, never skipped, so neither system is measured
against stale statistics the other doesn't also have refreshed.

### The warm protocol

Every system's `run()` performs **one discarded warm-up** call followed by
`repeat` timed samples of the *same* query; `time_s`/`time_mad_s` are the
**median** and **median absolute deviation** of those samples, at 6-decimal
precision (`citybench.stats`, `citybench.report`) — a fresh child process
per sample for the two native-reader systems (independent OS page-cache
and independent `peak_alloc` state each time), a fresh SQL round-trip per
sample for the three SQL-backed systems. `just bench`'s CLI defaults
`--repeat` to **7**, matching the sibling `cityparquet-rs` harness's own
protocol.

**`results/delft.csv` is now `just bench --repeat 7`'s output, not `just
smoke`'s.** An earlier version of this section (and Caveat 9 below)
correctly documented that the then-committed file was `just smoke`'s
`repeat=2` correctness pass, not a publication-grade run — that was true
at the time, but the fix wave that corrected C1/I4/I7 (final whole-branch
review) required re-running `delft` (along with `Montreal`/`Vienna`/
`lod3_railway`) at `repeat=7`, since those fixes change what is measured
and recorded for the PostgreSQL-backed systems. `results/delft.csv` is
now `repeat=7` throughout, matching `Montreal`/`Vienna`/`Zurich`/
`lod3_railway` — **every dataset in this corpus is publication-grade** as
of this fix wave. Cross-system agreement was re-verified on the fresh run
(all five report `2231` on `count`/`full-read`; `bbox-25pct` reports `240`
on all five) and `--repeat 7`'s own `repeat` column confirms the sample
count directly in the committed file, rather than needing to be taken on
trust from this paragraph.

### Count cross-check

The single highest-value defect detector this harness has
(`citybench.runner`'s own module docstring): every scenario's result count
is compared **across every system that ran it**, for the same parameters.
If two systems disagree, at least one is answering a different question,
and its timing is meaningless until reconciled — such rows are tagged
`count-mismatch: <system=count ...>` in `notes`, never silently published.
`just smoke` exits non-zero if any row carries this tag (or an `error:`
note). This tag is what caught two genuine, previously-unnoticed defects
the first time all five systems ran together: 3DCityDB's original
`semantic-surface` SQL reporting 2232 against cjdb's and
`duckdb-cityparquet`'s 1116 (Caveat 8 — a different question being asked,
not a data-quality bug), and cjdb's footprint bug reporting 239 against
the other four systems' 240 on `bbox-25pct` (Caveat 2). Several other
integration defects surfaced during the same first full run through the
pipeline simply erroring or crashing outright (a missing Python
dependency, a scenario sent to a reader that doesn't implement it, SQL
referencing columns that don't exist in the real schema) — real bugs, but
caught by the pipeline failing to complete at all, not by this specific
tag. A scenario that is legitimately unanswerable for a dataset (no
parent/child pair, for `hierarchy`) is recorded as `skipped: ...`,
distinct from an `error:`, and excluded from the cross-check rather than
treated as a disagreement.

### Two size figures

`size_bytes` and `size_bytes_no_index` are reported for every row (the
figures describe the *system*, not the scenario, so they're repeated on
every row a system contributes, avoiding a join to plot size against
time). Both are published because the comparison against a file format
flips depending on whether indexes are counted, and reporting only one
would be choosing the flattering number. On `delft.city.jsonl`:

| system | `size_bytes` (total) | `size_bytes_no_index` |
|---|---:|---:|
| `cityparquet` | 2,429,972 | 2,429,972 |
| `cityparquet-hilbert` | 2,386,495 | 2,386,495 |
| `duckdb-cityparquet` | 2,429,972 (same file) | 2,429,972 |
| `cjdb` | 16,891,904 | 6,619,136 |
| `3dcitydb` | 46,997,504 | 29,802,496 |

(I1, final whole-branch review: re-derived directly from the currently
committed `results/delft.csv` — a prior version of this table cited
figures from a `delft.csv` that had since been regenerated, which this
fix wave's own re-run of `delft` at `repeat=7` also regenerated again;
always read this table as "whatever the committed CSV says now", not a
number to keep in sync by hand.)

(`pg_total_relation_size`/`pg_table_size` over the schema for the two
PostgreSQL systems; total on-disk package bytes for the three
CityParquet-based ones, which carry no separate index files.)

## Metrics and the CSV contract

`results/<dataset>.csv`, one row per (system, scenario[, selectivity
target]), sixteen columns:

```
dataset,format,scenario,selectivity,result_count,time_s,time_mad_s,peak_heap_bytes,peak_rss_bytes,repeat,notes,bytes_read,http_requests,server_time_s,size_bytes,size_bytes_no_index
```

**Relationship to the sibling harness's CSVs, verified directly against
its committed files, not assumed from its documentation**: our first
**eleven** columns (`dataset` through `notes`) are identical in name and
order to every one of `benchmark/formats/read_results/*.csv`'s
committed files — checked against all eleven scenario-shaped CSVs there
(`3DBAG`, `9-284-556`, `Ingolstadt`, `Montreal`, `NYC`, `Railway`,
`Rotterdam`, `Vienna`, `Zurich`, `delft`, `lod3_railway`; that directory
holds a twelfth file, `sizes.csv`, but it is a different, five-column
shape — `dataset,format,bytes,mb,ratio_vs_cityjsonseq` — and not part of
this comparison), every one of the eleven stopping at `notes` — **eleven
columns, not thirteen**.
`bytes_read`/`http_requests` are columns 12–13 of that harness's
*documented* contract (`READ_BENCHMARK.md`'s CSV section, for its
`--transport http` rows) but do not appear in any of its *committed*
CSVs; this harness carries them anyway — always empty here, see below —
for forward compatibility with that documented shape. `server_time_s`,
`size_bytes`, `size_bytes_no_index` (columns 14–16) are genuinely new,
added once server-bound databases entered the comparison. Concatenating
this harness's rows with the sibling's committed ones is therefore not
quite the "no transformation" `citybench.report`'s own module docstring
currently claims (a claim inherited from an earlier version of this
project's spec, itself being corrected separately) — the sibling's rows
need **five empty fields appended** (`bytes_read`, `http_requests`,
`server_time_s`, `size_bytes`, `size_bytes_no_index`) to line up.
Trivial, lossless, and column-for-column identical up to that padding —
but a real step, not nothing.

- **`time_s` / `time_mad_s`** — warm-cache median / MAD of `repeat`
  samples, 6dp (see "The warm protocol" above).
- **`peak_heap_bytes`** — populated **only** for `cityparquet` /
  `cityparquet-hilbert` (the `peak_alloc` global-allocator high-water mark
  the `--child` protocol exposes). **Empty for every SQL-backed system**
  (`duckdb-cityparquet`, `cjdb`, `3dcitydb`): each runs out-of-process,
  with no allocator hook into it the way the Rust child has one into
  itself.
- **`peak_rss_bytes`** — populated **only** for the same two native-reader
  rows (`getrusage(RUSAGE_SELF).ru_maxrss`, the child's own process).
  **Empty for `duckdb-cityparquet`/`cjdb`/`3dcitydb`** — a PostgreSQL
  backend's RSS includes a share of `shared_buffers` and is not comparable
  with an in-process figure; not captured for those three at all, rather
  than captured and silently misleading. **In the currently committed CSVs
  the populated values are in KiB, not bytes**, despite the column name:
  `ru_maxrss` is natively bytes on macOS (this crate's development target)
  but KiB on Linux (glibc's `getrusage(2)`), this harness's committed run
  is Linux (`results/<dataset>.manifest.json`'s `host.platform`), and the
  `cityparquet-readbench` build that produced those files read `ru_maxrss`
  directly with no `cfg`-gated conversion. That reader is now fixed
  upstream — `max_rss_bytes()` routes through a `cfg`-gated `rss_to_bytes`
  (`benchmark/readbench/src/main.rs`), so a run made with the
  current binary reports true bytes on Linux and macOS alike. The
  already-committed numbers are **not** rewritten in place; the correction
  is a re-run, recorded as the erratum under Caveat 6 below.
- **`repeat`** — the actual sample count for that row (`7` throughout every
  currently committed file — see "The warm protocol" above; every dataset
  in this corpus is now a publication-grade `--repeat 7` run).
- **`bytes_read` / `http_requests`** — **always empty, every row**. This
  harness measures local transport only; neither PostgreSQL system has an
  object-storage path, and the HTTP-transport comparison remains the
  sibling `cityparquet-rs` harness's own (`--transport http`).
- **`server_time_s`** — empty for the three in-process systems
  (`cityparquet`, `cityparquet-hilbert`, `duckdb-cityparquet` — no
  client-server split to report); populated for `cjdb`/`3dcitydb` from a
  second, untimed `EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON)` re-run of the
  identical query, extracting PostgreSQL's own reported `Execution Time`
  (`pg.time_query`/`pg.parse_explain_execution_time`). See Caveat 4 for
  what the gap between `time_s` and `server_time_s` means.
- **`size_bytes` / `size_bytes_no_index`** — see "Two size figures" above.

## Fairness caveats

Read before citing a number.

1. **Counting granularity differs by system for `count`/`full-read`/
   `bbox-query`, and every system's counts are reconciled to CityObject
   granularity before any number is published.** CityParquet counts one
   row per CityObject (parents *and* children). cjdb's `city_object` is
   natively per-CityObject. 3DCityDB v5 instead stores CityGML semantic
   (boundary) surfaces as ordinary rows in `citydb.feature` alongside
   `Building`/`BuildingPart` — a bare `SELECT count(*) FROM citydb.feature`
   returns **10045**, not the true 2231, because it also counts 3350
   `WallSurface` + 2232 `GroundSurface` + 2232 `RoofSurface` rows (verified
   by grouping `feature` by `objectclass_id`: `1115 Building + 1116
   BuildingPart = 2231`, exactly the target; the remaining 7814 rows are
   semantic surfaces). Every count/full-read query against 3DCityDB in this
   harness therefore uses the canonical predicate derived and documented in
   `docs/3dcitydb-v5-schema.md` (`is_toplevel = 1 OR NOT <descends from
   AbstractSpaceBoundary, objectclass id 13>` — the defensive form, chosen
   over two narrower alternatives because it alone survives a documented
   `ReliefFeature` anomaly where a genuinely top-level class descends from
   the boundary-surface superclass). **All five systems report 2231 on
   `delft.city.jsonl`'s `count`/`full-read`**, confirmed both in the
   committed `results/delft.csv` and in a fresh re-run for this task.

2. **cjdb is patched, and why.** `CjdbSystem` drives a **patched** build of
   cjdb 2.2.0, not stock cjdb from PyPI. Stock cjdb's
   `get_ground_surfaces()` (`cjdb/modules/geometric.py`) derives an
   object's 2D footprint by collecting its lowest-LoD solid's non-vertical
   faces into a **dict keyed by each face's own mean Z height** — any two
   faces sharing (or floating-point-tying) a mean Z silently overwrite one
   another, so only the last one survives. This is an ordinary shape for a
   flat-roofed building, not a contrived edge case: measured directly
   against this project's own `delft.city.jsonl` fixture, **9 of the 1116
   BuildingParts** hit this code path with two or more tied faces, losing
   up to **8** of them in one object, and one such object's resulting
   undersized footprint genuinely flipped one `bbox-25pct` count (239 vs
   the other four systems' 240) before the patch was applied. This
   benchmark's claim concerns cjdb's **architecture** — row-oriented
   PostgreSQL with JSONB geometry, against CityParquet's columnar design —
   not a footprint bug in one release's importer; benchmarking the bug
   instead of the architecture would attack a strawman and be unfair to
   cjdb. So every cjdb number in this benchmark comes from a **corrected**
   cjdb, and that fact is recorded everywhere a number derived from it
   appears:
   - **Here** (and this section is not to be removed or weakened by later
     edits to this file).
   - `results/<dataset>.manifest.json`'s `versions.cjdb`
     (`"2.2.0+ground-surfaces-tie-patch"`) and `patches.cjdb` (the full
     disclosure: upstream version, what changed, why, which local build it
     came from).
   - `vendor/cjdb/README.md` and the patch itself,
     `vendor/cjdb/ground-surfaces-tie.patch` (a minimal diff: `dict` keyed
     by Z becomes a `list` of `(z, polygon)` pairs, so a tie is appended,
     never overwritten; the split threshold's mean-Z calculation is
     deliberately left unchanged).

   Before running anything that ingests into cjdb (`just smoke`, `just
   bench`), build the patched source once with `just patch-cjdb` — a
   separate, explicit step (not run automatically) because it downloads
   from PyPI and is slow enough on a cold cache that doing it silently,
   mid-benchmark, would be a surprise rather than a courtesy.
   `CjdbSystem.prepare()` fails fast with a clear message if this step was
   skipped, or if the patch file has been edited since the last build (the
   build directory is content-addressed by the patch file's own SHA-256
   prefix, specifically to prevent silently running a stale patched build).

3. **cjdb's spatial index is 2D only, structurally.** cjdb stores full
   geometry as JSONB and only a 2D footprint as a PostGIS geometry
   (`ground_geometry`, which has no z ordinate at all, ever) — `bbox-query`
   against cjdb is therefore 2D by construction, the same limitation
   FlatCityBuf's R-tree has in the sibling harness. **This is not, in this
   harness, a differentiator from the other systems' own `bbox-query` SQL**:
   `3DCityDB`'s `envelope` column genuinely carries a full 3D extent
   (verified live: `ST_NDims(envelope)` → 3, `geometry(GeometryZ,7415)`),
   but the query compares it against `ST_MakeEnvelope(minx,miny,maxx,maxy,
   srid)`, which is itself a plain 2D polygon (verified live:
   `ST_NDims(...)` → 2) — so the comparison is 2D regardless of what
   `envelope` stores. `duckdb-cityparquet`'s own `bbox-query` SQL likewise
   tests only `bbox.xmax`/`xmin`/`ymax`/`ymin`, never `zmin`/`zmax`, even
   though the `bbox` STRUCT column carries both (verified against the real
   package: `STRUCT(xmin, ymin, zmin, xmax, ymax, zmax)`). Only the native
   `cityparquet`/`cityparquet-hilbert` reader tests all six bounds at the
   row level. In practice **none of this moves any published number**,
   because the query windows themselves never narrow z (see "Query
   parameters" above) — every object is in z-range for every system on
   every `bbox-query` row in this benchmark. The distinction that survives
   is a structural one, not an outcome one: cjdb's storage model *cannot*
   ever answer a z-restricted `bbox-query`; the other four systems' storage
   *could*, but this harness's own SQL doesn't ask them to.

4. **In-process versus client-server — and `server_time_s` is an
   instrumented UPPER BOUND, not a clean "server-only" component of
   `time_s`.** DuckDB (`cityparquet-cli`'s readbench child and
   `duckdb-cityparquet`) runs in-process; both PostgreSQL systems run
   behind a socket, even though the client and server happen to share a
   machine here. `time_s` is the end-to-end, headline figure — wall-clock
   from just before the query is issued to just after every row is fetched
   to exhaustion, for every system, with no `EXPLAIN` instrumentation
   involved. `server_time_s` is populated only for `cjdb`/`3dcitydb`, from
   a SEPARATE, untimed re-run of the identical query under `EXPLAIN
   (ANALYZE, BUFFERS)` (with `track_io_timing = on`, per "Tuning parity"
   above) — PostgreSQL's own reported `Execution Time` (`pg.time_query`).

   An earlier version of this caveat framed the gap between the two as a
   clean "client-server tax" available for subtraction (`time_s -
   server_time_s`). That framing is WRONG, and is falsified by the
   committed data itself: **17 rows across the current corpus have
   `server_time_s` strictly GREATER than `time_s`** (re-derived directly
   from every committed `results/*.csv`, not carried over by hand — this
   number moves whenever the underlying CSVs are re-run, and a prior
   version of this caveat cited `18` and a Zurich example that a later
   Zurich re-run had already made stale; both are corrected here from the
   currently committed files) (e.g. `results/Zurich.csv`'s `3dcitydb`
   `count` row, `0.024366s` / `0.043948s` — the "server-only" figure
   exceeding the full end-to-end wall-clock it is supposedly a subset of),
   which is impossible under a "tax subtracted from the total" reading.
   The real explanation:
   `EXPLAIN (ANALYZE, BUFFERS)` itself adds genuine instrumentation
   overhead on top of the query's own execution — per-plan-node timing and
   buffer-hit counters, plus `track_io_timing`'s own clock calls around
   every I/O operation — so `server_time_s` measures "this query's
   execution time PLUS the cost of measuring it in detail", not "this
   query's bare execution time". It is therefore an UPPER BOUND on the
   engine's true, uninstrumented execution time, not a lower, clean
   subcomponent of `time_s`. **Do not subtract the two to compute a
   "client-server tax"** — the true tax (connection/protocol/result-
   transfer overhead a query issued from inside the same process never
   pays) is real and still worth attributing conceptually, but
   `server_time_s` as measured here is not a clean enough instrument to
   isolate it arithmetically. Both figures remain worth publishing side by
   side: `time_s` as the uninstrumented, comparable headline figure across
   all five systems, `server_time_s` as PostgreSQL's own upper-bound
   estimate of its own execution cost — a reader should read the relationship
   between them qualitatively (both measure roughly the same query cost;
   neither should be subtracted from the other), not arithmetically.

5. **Tier-2 scenarios have no native-reader rows.** `lod-extract`,
   `semantic-surface` and `hierarchy` run on the three SQL systems only
   (`duckdb-cityparquet`, `cjdb`, `3dcitydb` — `registry.SQL_SYSTEMS`),
   because the Rust child (`cityparquet-readbench --child`) implements
   only the seven inherited Tier-1 scenarios, and this harness deliberately
   did not edit that submodule to add three more. `ReadbenchSystem`/
   `build_child_args` raises `ValueError` if handed a Tier-2 scenario name
   rather than silently answering something else; the CLI's own
   `_run_all_scenarios` splits Tier-1 and Tier-2 into two separate
   `run_matrix` calls specifically so this never surfaces as a spurious
   `error:` row against the native readers.

6. **`peak_rss_bytes` is not captured for the PostgreSQL systems or
   DuckDB.** A PostgreSQL backend's RSS includes a share of
   `shared_buffers` (8GB, shared across every connection — see "Tuning
   parity" above) and is not a like-for-like figure against an in-process
   allocator's own high-water mark; DuckDB's Python client has no
   equivalent hook wired up here either. It **is** populated for the two
   native-reader rows (`cityparquet`/`cityparquet-hilbert`), and in the
   currently committed CSVs that value is in **KiB, not bytes** — the Rust
   source's `max_rss_bytes()` targeted macOS development and applied no
   `cfg`-gated conversion for Linux's `getrusage(2)`. That conversion now
   exists (`rss_to_bytes`), so runs made with the current binary report
   true bytes on either platform; only the pre-fix files below are affected
   (see "Metrics and the CSV contract" above for the full explanation). Do
   not read the `peak_rss_bytes` column as a like-for-like comparison
   across systems, nor the committed values as literal bytes.

   > **Erratum (2026-08-15, `peak_rss_bytes` unit).** The committed results
   > CSVs below were produced on this Linux host by a `cityparquet-readbench`
   > build that read `getrusage(RUSAGE_SELF).ru_maxrss` as bytes; on Linux
   > `ru_maxrss` is KiB, so the native-reader rows' `peak_rss_bytes` values in
   > these files are actually **KiB** (a 1024× under-report if read as bytes):
   >
   > - `results/delft.csv`
   > - `results/Montreal.csv`
   > - `results/Vienna.csv`
   > - `results/Zurich.csv`
   > - `results/lod3_railway.csv`
   >
   > The reader is fixed in cityparquet-rs (`rss_to_bytes`, commit
   > "fix(readbench): convert Linux ru_maxrss KiB to bytes"); these five CSVs
   > are **flagged for re-run** with the fixed binary before any
   > `peak_rss_bytes` value is quoted in the paper. The pre-fix CSVs (and
   > their manifests and index files) were removed from `results/` on
   > 2026-08-16 pending that re-run — the correction is the re-run, not an
   > in-place rewrite. In the removed files, `time_s`, `peak_heap_bytes` and
   > all other columns were unaffected by the unit defect.

7. **`bytes_read` / `http_requests` are always empty.** This harness
   measures local transport only — every system reads from local disk or a
   localhost socket. The HTTP/cloud-object-storage comparison remains the
   sibling `cityparquet-rs` harness's own (`--transport http`), and neither
   PostgreSQL system has an object-storage access path to measure in the
   first place.

8. **`semantic-surface` is any-LoD by deliberate choice, not because
   3DCityDB forced it.** A LoD-scoped query **is** expressible against
   3DCityDB — `property.name IN ('lod1MultiSurface','lod2MultiSurface')`
   rows carry their own `val_lod` (an earlier version of this file's
   reasoning claimed any-LoD was 3DCityDB's *only* option; that was an
   overclaim, caught by review and corrected). A LoD-scoped variant was
   written and run during that investigation and reported, at the time, to
   return 1116 for LoD1 and 1116 for LoD2, both plausible — **that figure
   is investigative context, not a citable measurement of this harness's
   own**: the LoD-scoped query is not part of `sql_citydb.py`'s committed
   scenario set, and its output was not captured to any committed
   artefact, so treat it as a reported finding to be re-verified before
   citing, not a number this README stands behind the way it does for the
   committed CSV's own figures. Any-LoD was chosen anyway, for two
   reasons: it is the more natural question a scenario named
   "semantic-surface" should ask — "does this object have a roof surface
   classified at all", independent of which LoD tier happens to carry that
   classification — and picking one specific LoD to scope to would risk
   privileging whichever tier a given system's own storage model happens
   to represent most naturally or richly, a self-serving choice to make in
   a benchmark where CityParquet is itself a participant. If the reported
   LoD-scoped figures above are accurate, the choice would not have moved
   any published number on `delft.city.jsonl` specifically — every
   BuildingPart here reportedly has a RoofSurface at every LoD it stores —
   but that is exactly the kind of fixture-specific coincidence this
   README will not assert without a committed artefact behind it. This
   scenario's SQL also went through one genuine, cross-check-caught bug,
   which **is** fully traceable to the committed `results/delft.csv`:
   3DCityDB's first version counted RoofSurface *rows* directly
   (2232, since v5's importer gives each BuildingPart two solids —
   `lod1Solid`/`lod2Solid` — each with its own RoofSurface row), which
   `run_matrix`'s count cross-check flagged against cjdb's and
   `duckdb-cityparquet`'s own 1116; the fix rewrote it to the same
   *presence* question the other two systems ask (`count(DISTINCT
   pr.feature_id)`), not a raw surface count.

9. **[Superseded — kept for history, see "The warm protocol" above.]**
   `results/delft.csv` was, at the time this caveat was first written,
   `just smoke`'s `repeat=2` output. The final whole-branch review's fix
   wave (C1/I4/I7) required re-running `delft` at `repeat=7` — the
   currently committed `results/delft.csv` IS now a publication-grade
   `just bench --repeat 7` run, matching every other dataset in this
   corpus. This entry is left in place, corrected, rather than deleted and
   renumbered, so cross-references to "Caveat 9" elsewhere in this
   document and in commit history keep pointing at the right place.

10. **cjdb's footprint is undefined for a parent CityObject that carries no
    geometry of its own — a second, distinct gap from Caveat 2's
    already-patched tied-Z-face bug.** First exposed by Vienna
    (`data/Vienna.city.jsonl`): 220 of its 307 `Building` objects hold no
    `"geometry"` of their own at all — every LoD surface lives entirely on
    their `BuildingPart` children — a legitimate, spec-conformant CityJSON
    shape delft's fixture never happens to use. `cjdb import` logs
    `WARNING: No ground surfaces were found for object ID=(...)` for each
    (reproduced again on Zurich's larger corpus, same warning, many
    objects), and leaves `ground_geometry` **NULL** for that object — cjdb's
    `get_ground_surfaces()` derives a footprint only from the object's OWN
    `"geometry"` array, never by aggregating its children's. CityParquet's
    writer and 3DCityDB's importer both do the opposite: verified directly
    against Vienna's own converted package and live import, a childless
    `Building`'s `bbox` (CityParquet) / `envelope` (3DCityDB) is populated —
    both equal
    `(1036.734, 340511.47, ..., 1071.4, 340530.818, ...)` for
    `UUID_LOD2_011972-d483b116-b182-4322-ae06`, evidently synthesised as
    the union of its `BuildingPart` children's own geometry. The result:
    cjdb's `bbox-query` undercounts against the other four systems on
    Vienna whenever a childless `Building` falls inside the query window —
    `bbox-5pct` reports 34 vs the other four's 46 (12 short, all 12
    confirmed childless `Building`s by direct `ground_geometry IS NULL`
    lookup); `bbox-25pct` reports 248 vs 304. `bbox-1pct` is unaffected
    only because no childless `Building` happens to fall in that
    particular, smaller window on this fixture — a fixture-specific
    coincidence, not evidence the gap is selectivity-dependent. This is a
    genuine architectural property of cjdb (full geometry stored per-object
    as JSONB, no cross-object aggregation step), not a footprint-derivation
    bug of the kind Caveat 2 patches — no patch is applied for it, and
    `results/Vienna.csv`'s `bbox-5pct`/`bbox-25pct` rows for `cjdb` carry
    `count-mismatch` accordingly. `count`/`full-read`/`attr-filter`/
    `id-lookup`/`project`/`lod-extract`/`semantic-surface`/`hierarchy` are
    unaffected (none of them touch `ground_geometry`), and all five systems
    agree on every one of those on Vienna.

11. **3DCityDB v5's CityJSON importer re-shapes `measuredHeight`
    specifically, breaking `attr-stats` for any dataset whose derived
    numeric column happens to be it.** First exposed by Vienna, whose
    derived `numeric_column` is `measuredHeight` (delft's own,
    `b3_h_dak_50p`, is not a CityGML-recognised term and never triggers
    this). Verified live against the imported schema: no
    `citydb.property` row has `name = 'measuredHeight'` at all — every one
    of the 1102 values instead lands under `name = 'height'`, imported as
    CityGML 3.0's structured `core:Height` datatype
    (`datatype.typename = 'Height'`), with the scalar itself pushed one
    level down into a CHILD `property` row named `'value'` (siblings
    `status`/`lowReference`/`highReference` alongside it — e.g. object id 2's
    `height` row (id 6) has NULL `val_double`; its child row (id 7,
    `parent_id = 6`, `name = 'value'`) holds `val_double = 15.6`).
    `sql_citydb.py`'s `attr-stats` query (`WHERE pr.name = %s`, bound to
    `params.numeric_column` verbatim — the same literal CityJSON attribute
    name every other system is handed) therefore matches zero rows against
    3DCityDB specifically: `results/Vienna.csv`'s `attr-stats` row for
    `3dcitydb` reports `0`, tagged `count-mismatch` against the other four
    systems' agreeing `1102`. This is a genuine, verified property of
    3DCityDB v5's CityGML-3.0-aware import — CityGML 3.0 renamed and
    restructured CityGML 2.0's scalar `bldg:measuredHeight` into the
    generic structured `Height` class, and `citydb-tool` maps CityJSON's
    (CityGML-2.0-vocabulary) `measuredHeight` attribute onto that new
    representation on import — not a harness defect: the query already
    asks 3DCityDB the same natural question ("find the property named
    exactly what every other system was handed") a competent user would,
    and no generic rewrite of that question can chase an import-time rename
    that is specific to this one CityGML-recognised attribute name without
    hardcoding it (`sql_citydb.py`'s own design rule: never a shape
    contrived to match another system's plan). Left undoctored and
    disclosed here rather than special-cased. **Practical consequence**:
    `attr-stats` numbers are only citable across all five systems for a
    dataset whose derived numeric column is NOT a CityGML-3.0-recognised
    structured-datatype name (`measuredHeight` is the one instance found
    so far, on this corpus); `results/Vienna.csv`'s own `attr-stats` row
    for `3dcitydb` must be read with this caveat, not cited as "3DCityDB
    has no such attribute". **This is unlike Zurich's own `attr-stats`
    mismatch (originally the same symptom — `3dcitydb` reporting 0)**,
    which turned out to have a different, genuinely fixable cause (see
    `sql_citydb.py`'s `attr-stats` docstring): Zurich's derived numeric
    column, `Geomtype`, is a plain enum-coded integer (`1`/`2`), which
    `citydb-tool` stores whole in `property.val_int`, not `val_double` —
    still on the SAME `name='Geomtype'` row a val_double-only query
    already finds, just in a sibling column. `sql_citydb.py`'s attr-stats
    query now reads `coalesce(val_double, val_int)`, fixed and re-run
    (`results/Zurich.csv` — see Caveat 12 below for the full before/after).
    `measuredHeight`'s value, by contrast, is not reachable by coalescing
    sibling columns of the SAME row at all — it lives on a DIFFERENT row
    (a child `property` keyed by `parent_id`, named `'value'`) — so that
    one caveat, and only that one, stays genuinely undoctored.

12. **Zurich re-exercises Caveat 10's cjdb footprint gap at far higher
    incidence, plus a second, distinct mechanism inside `get_ground_surfaces()`
    itself.** `results/Zurich.csv`'s `bbox-5pct`/`bbox-25pct` rows for
    `cjdb` report 1/23440 against the other four systems' agreeing
    2/55052 — a much larger relative gap than Vienna's. Two mechanisms,
    verified live against Zurich's own import, both structural rather than
    the already-patched tied-Z-face bug (Caveat 2):
    - **Every one of Zurich's 52834 `Building` objects carries no geometry
      of its own** (`SELECT type, count(*) FILTER (WHERE ground_geometry
      IS NULL), count(*) FROM cjdb.city_object GROUP BY type` reads
      `Building | 52834 | 52834` — 100%, not Vienna's 220/307) — this is
      Caveat 10's exact mechanism, just at full incidence here rather than
      partial: every `Building` in Zurich is a pure container, all
      geometry living on its `BuildingPart` children, so cjdb's
      per-object-only footprint derivation leaves `ground_geometry` NULL
      for the entire class.
    - **A second, previously unobserved mechanism affects `BuildingPart`
      itself**: 63280 of Zurich's 145865 `BuildingPart` objects (43.4%)
      also carry `ground_geometry IS NULL`, despite HAVING their own
      geometry. Traced to `get_ground_surfaces()`'s own algorithm
      (`cjdb/modules/geometric.py`, read directly from the patched build
      this harness drives): it first discards every face whose normal is
      (near-)vertical, then keeps only the SURVIVING faces whose mean Z is
      *strictly less than* the mean of the surviving faces' own (distinct)
      Z values. A `BuildingPart` whose only non-vertical face is a single
      roof — e.g. a small elevated feature (dormer, rooftop protrusion)
      modelled as a standalone `MultiSurface` of walls + one `RoofSurface`,
      with no `GroundSurface` at all — has exactly ONE candidate Z value,
      and one value is never strictly less than its own mean, so the
      filter returns empty regardless of the Caveat 2 patch (there is no
      tie to lose here; there is nothing to keep in the first place).
      Verified directly against one such object,
      `UUID_bfc71b11-c3a6-430c-ab43-0e176615029d` (`Geomtype: 2`): a
      5-face `MultiSurface` (4 `WallSurface` + 1 `RoofSurface`, LoD `2`,
      `geographicalExtent` spanning only ~2.5m vertically), logged by cjdb
      as `WARNING: No ground surfaces were found`. Corpus-wide: 93024 of
      145865 `BuildingPart`s (63.8%) carry no `GroundSurface` semantic
      label anywhere in their own geometry (checked directly against the
      source `.city.jsonl`); 63280 of those end up with `ground_geometry
      IS NULL` — the remainder apparently still have ≥2 distinct
      non-vertical candidate faces (e.g. a stepped roof) despite carrying
      no explicit `GroundSurface` tag, so the same Z-split still finds a
      lower one. Combined, **116114 of Zurich's 198699 CityObjects
      (58.4%)** have `ground_geometry IS NULL`, matching the observed
      `bbox-25pct` shortfall (23440/55052 present ≈ 42.5%, consistent with
      the corpus-wide 41.6% non-NULL rate) and exactly accounting for
      `bbox-5pct`'s single missing object (confirmed directly:
      `UUID_d5a734d9-a19a-4c61-8699-12c0e5cc5a3c`, a `Building`).
      **Not patched further** — reported here per instruction, pending a
      decision on whether `get_ground_surfaces()`'s single-candidate case
      is in scope for a second patch, the same way Caveat 2's tied-face
      case was.

13. **3DCityDB's own `bbox-query` also overcounts on Zurich, by a
    different, PostGIS-internal mechanism: a float4-precision cached
    bounding box, not an architectural limitation.** `results/Zurich.csv`'s
    `bbox-25pct` row reports `3dcitydb=55060` against
    `cityparquet`/`cityparquet-hilbert`/`duckdb-cityparquet`'s agreeing
    `55052` — 8 extra objects (3 `Building`, 5 `BuildingPart`), all
    verified live to be FALSE POSITIVES: for every one of the 8,
    `ST_Intersects(envelope, ST_MakeEnvelope(...))` (exact, double
    precision) is `false`, while `envelope && ST_MakeEnvelope(...)` (the
    operator `sql_citydb.py`'s `bbox-query` actually uses) is `true`. Each
    extra object's `envelope` sits only 0.01–0.03m outside the query
    window on one axis (e.g. `ST_XMin(envelope) = 2683249.195` against a
    window `xmax` of `2683249.1795`) — well inside a single-precision
    (float4) rounding step at Zurich's CH1903+/LV95 coordinate magnitude
    (~2.68 million / 1.25 million; float4's ~24-bit mantissa gives an
    absolute step of roughly `2.68e6 × 2⁻²⁴ ≈ 0.16m` at that scale).
    Confirmed to live in the `&&` operator's own evaluation, not GiST
    index lossiness: `SET enable_bitmapscan = off; SET enable_indexscan =
    off` (forcing a sequential scan, bypassing `feature_envelope_spx`
    entirely) reproduces the identical, still-wrong count. This matches a
    documented PostGIS characteristic — `GSERIALIZED` geometries cache a
    float4-precision bounding box for fast overlap tests, used directly by
    `&&` regardless of indexing — not a 3DCityDB-specific defect, and not
    something any of this harness's other four `bbox-query` mechanisms are
    exposed to: `cityparquet`/`cityparquet-hilbert` compare true double
    bounds row-by-row, and `duckdb-cityparquet` compares the `bbox` STRUCT's
    own double columns directly, neither ever going through a cached
    single-precision header. `cjdb`'s own `bbox-query` SQL also uses `&&`
    against a PostGIS geometry column at this same SRID and coordinate
    magnitude, so it is plausibly exposed to the identical artefact — but
    cjdb's result is dominated by Caveat 12's much larger undercount, so a
    handful of float4-driven false positives underneath that undercount
    would not currently be visible in `cjdb`'s own reported figure. Not
    present on delft (~84500 magnitude) or confirmed absent on Vienna
    (~1000/340000 magnitude, well inside float4 precision, and `3dcitydb`
    matches the other four systems exactly on both Vienna `bbox-query`
    rows) — a coordinate-magnitude-dependent finding, consistent with the
    same "true at this scale, not guaranteed at another" framing Caveat 3
    already applies to z-restriction. **Not patched** — `sql_citydb.py`'s
    `bbox-query` still uses `&&`, the natural, index-cooperating,
    idiomatic PostGIS form; reported here per instruction rather than
    silently switched to `ST_Intersects` (which — because `envelope` is
    itself already a rectangular box, not a complex solid — would very
    likely fix this exactly, at negligible extra cost, but that is a
    judgement call left to a follow-up decision rather than made
    unilaterally here).

14. **`cityparquet-readbench` refuses `lod3_railway`'s package outright —
    every Tier-1 row for `cityparquet`/`cityparquet-hilbert` is `error:`,
    not just the Tier-2 rows Caveat 5 already explains.** `lod3_railway`
    is the corpus's only BY-TYPE (multi-table) package — 121 CityObjects
    across 14 CityGML classes with no dominant table, so `cityparquet
    convert` writes `railway.parquet`, `bridge.parquet`, `tunnel.parquet`,
    `building.parquet`, ... rather than one wide table. Confirmed live by
    invoking the child binary directly:
    `cityparquet-readbench --child --format cityparquet --scenario count
    --input data/cityparquet/lod3_railway` prints `package ... has 9
    tables (...); the read-benchmark only supports single-table
    (single-family) packages, not multi-table by-type packages` and exits
    non-zero — `ReadbenchSystem.run()` reports this as `error:
    CalledProcessError` for 8 of the 9 Tier-1 rows on BOTH `cityparquet`
    and `cityparquet-hilbert` (**16 `error:` rows total on this dataset,
    not 18** — I5, final whole-branch review, corrected: an earlier
    version of this caveat claimed all 9 Tier-1 rows error on both
    formats. `attr-stats` is the one exception — `lod3_railway` has no
    numeric attribute at all (see `params.derive()`), so it is tagged
    `skipped: dataset has no numeric attribute` on EVERY system, including
    `cityparquet`/`cityparquet-hilbert`, BEFORE the child binary is ever
    invoked (`ScenarioUnavailable` is raised before the multi-table
    refusal would fire) — a `skipped:` row, not an `error:` one, verified
    directly against the committed `results/lod3_railway.csv`: `grep -c
    "error:"` reads 16; the two `attr-stats` rows for `cityparquet`/
    `cityparquet-hilbert` read `skipped:`, not `error:`). This is a
    genuine `cityparquet-rs` (submodule) limitation —
    `duckdb_cp.py`'s own `object_table_files()`/`_table()` were fixed for
    exactly this multi-family shape during this same task (see that
    function's docstring), but the Rust child was not, and per this
    repo's own rule this harness does not edit that submodule to add
    multi-table support. Not a harness defect to patch around; the three
    SQL systems (`duckdb-cityparquet`, `cjdb`, `3dcitydb`) still cover
    every Tier-1 and Tier-2 scenario on this dataset, so the corpus entry
    is complete for them, just not for the two native-reader rows.

15. **`cityparquet convert` leaves `bbox` NULL for a CityJSON
    `GeometryInstance` (geometry-template) object — verified live on
    `lod3_railway`, a genuine gap in CityParquet's own writer, not a
    cjdb/3dcitydb defect.** `results/lod3_railway.csv`'s `bbox-5pct` row's
    own note reads `count-mismatch: 3dcitydb=8 cjdb=6
    duckdb-cityparquet=7` — **three DIFFERENT values, not
    `duckdb-cityparquet=7` against an agreeing `cjdb`/`3dcitydb` pair at
    `8`** (I5, final whole-branch review: an earlier version of this
    caveat claimed the latter, which is false — `cjdb`'s own `6` is a
    SEPARATE, unrelated gap, fully explained by Caveat 16 below, not
    evidence corroborating this caveat's own claim). This caveat is about
    the `duckdb-cityparquet=7` vs. `3dcitydb=8` half of that three-way
    mismatch specifically: the one object missing from
    `duckdb-cityparquet`'s side is
    `GMLID_SO092422_3593_9527`, a `SolitaryVegetationObject` (a tree)
    placed via `"geometry": [{"type": "GeometryInstance", "template": 2,
    "boundaries": [8683], "transformationMatrix": [...]}]` — CityJSON's
    mechanism for placing a reusable template shape at an anchor point
    rather than storing full boundary geometry per instance. Queried
    directly against the real package: `SELECT bbox FROM
    read_parquet('lod3_railway/*.parquet', union_by_name=true) WHERE id =
    'GMLID_SO092422_3593_9527'` returns `bbox = NULL`. Since `bbox-query`'s
    SQL (`WHERE bbox.xmax >= ? AND bbox.xmin <= ? AND ...`) evaluates to
    NULL, not TRUE, against a NULL `bbox` STRUCT, this object is silently
    excluded from EVERY `bbox-query` window regardless of its true spatial
    extent — not just this one 5% window; it is simply invisible to
    bbox-based selection at every selectivity, which also means row-group
    pruning (`with_bbox_row_groups`, the native readers' own bbox
    mechanism) could not find it either, though this specific dataset
    could not confirm that independently because of Caveat 14. `cjdb` and
    `3dcitydb` both resolve/store the object's actual placed geometry
    (not a template reference) and so both correctly include it — the two
    systems this benchmark exists to compare CityParquet against are, on
    this one object, MORE complete than CityParquet's own encoding. Worth
    raising as a `cityparquet-rs` issue (the writer path that computes
    `bbox` per row does not appear to resolve `GeometryInstance` boundary
    references before computing extent) — not fixed here, since
    `cityparquet-rs` is a submodule this harness does not edit.

16. **A third, distinct failure mode of cjdb's `get_ground_surfaces()`:
    a genuinely UNDERSIZED (not NULL) footprint for elongated, near-flat
    geometry, first seen on `lod3_railway`'s `Railway` objects.**
    `results/lod3_railway.csv`'s `bbox-5pct` row also shows `cjdb=6`
    against `duckdb-cityparquet`/`3dcitydb`'s `7`/`8` (a second, separate
    gap from Caveat 15's above) — `cjdb` is missing
    `GMLID_18468614_329001_318`, a `Railway` (`MultiSurface`, LoD `3`).
    Unlike every prior footprint gap in this corpus, `ground_geometry` is
    NOT null here (`SELECT object_id, ground_geometry IS NULL FROM
    cjdb.city_object WHERE object_id = 'GMLID_18468614_329001_318'`
    reads `f` — a real footprint WAS derived). The derived footprint is
    simply too small: `ST_XMin/YMin/XMax/YMax(ground_geometry)` reads
    `(2.41, 2.986, 11.436, 6.825)`, which does not reach into the
    `bbox-5pct` window's own y-range (`[0.64, 2.214]`) at all, even though
    `duckdb-cityparquet`'s and `3dcitydb`'s own stored extents for the
    same object do. Consistent with `get_ground_surfaces()`'s own
    algorithm (see Caveat 12): its "keep faces with `z` strictly less than
    the mean of the surviving faces' own distinct `z` values" split
    assumes a building-shaped object with a clear roof-versus-ground
    height separation. A railway track's `MultiSurface` (rails, ballast,
    ties) is comparatively flat and elongated along X/Y with much smaller
    Z variation, so the same height-based split can plausibly retain only
    part of the object's true horizontal footprint rather than the
    whole thing — genuinely different from Caveat 2's tied-face bug (no
    tie here) and Caveat 12's single-candidate-face case (a footprint
    WAS produced, just an incomplete one). **Not patched** — reported per
    instruction, alongside Caveats 12/13, pending a decision on whether
    `get_ground_surfaces()` is back in scope for further work.

17. **`lod-extract` is degenerate for `duckdb-cityparquet` on 4 of this
    corpus's 5 datasets — its published time on those rows is not a
    measurement of anything, and the projection-pushdown claim this
    scenario exists to demonstrate has publication-grade evidence from
    exactly ONE dataset (`delft`), not from the other four.** (I2, final
    whole-branch review.) `sql_duckdb.py`'s
    `lod-extract` branch targets one specific, hardcoded column,
    `geometry_lod1_2` — CityParquet's own LoD1.2 geometry — and only
    `delft.city.jsonl`'s converted package carries that column at all
    (verified live: `DESCRIBE` on each dataset's own package shows delft
    alone has `geometry_lod0_0`/`geometry_lod1_2`/`geometry_lod1_3`/
    `geometry_lod2_2`; **Montreal, Vienna, and Zurich carry only
    `geometry_lod0_0`/`geometry_lod2_0`, and `lod3_railway` carries only
    `geometry_lod0_0`/`geometry_lod3_0`** — no `geometry_lod1_2` column
    exists in any of the four). When the column is absent, `sql_duckdb.py`
    emits `SELECT count(*) FROM {table} WHERE FALSE` (`sql_duckdb.py:160-161`)
    — a query DuckDB **constant-folds at plan time**: `WHERE FALSE` is
    known to match zero rows before a single Parquet page is ever touched,
    so this is not "a fast scan of a small result", it is **no scan at
    all**. `cjdb` and `3DCityDB`, by contrast, run their own real,
    unconditional `lod-extract` SQL against these same four datasets (a
    fixed question — "count of objects carrying an LoD1.2 geometry",
    hardcoded `"1.2"`/`"1"` respectively) and **genuinely execute a real
    query that happens to match zero rows** — a materially different thing
    from never running a query at all. All three systems agree on `0` on
    all four datasets (verified against the committed CSVs — no
    `count-mismatch` on any of the twelve affected rows), so the
    cross-check correctly finds nothing wrong with the ANSWER; it has no
    mechanism to catch that one of the three answers involved no work.
    **`results/{Montreal,Vienna,Zurich,lod3_railway}.csv`'s `lod-extract`
    rows publish all three systems' `time_s` figures side by side with no
    note distinguishing this** — a reader comparing
    `duckdb-cityparquet`'s ~0.001–0.003s against `cjdb`'s/`3dcitydb`'s own
    (real, if fast) query times on these four rows would be comparing "the
    cost of evaluating a compile-time-constant WHERE clause" against "the
    cost of an actual index-driven query that returns nothing", not "the
    cost of extracting an LoD1.2 geometry column via projection pushdown"
    against the same. **The ONLY dataset in this corpus where
    `duckdb-cityparquet`'s `lod-extract` row reflects real projection-
    pushdown work is `delft`** (the one package that actually carries
    `geometry_lod1_2`). **Updated after this fix wave's own re-run**: an
    earlier version of this caveat additionally noted `results/delft.csv`
    was `just smoke`'s `repeat=2` output at the time, leaving NO
    non-degenerate, publication-grade `lod-extract` measurement anywhere
    in the corpus. `delft` has since been re-run at `repeat=7` as part of
    this same fix wave (see "The warm protocol" and Caveat 9 above), so
    that gap is closed: `results/delft.csv`'s `lod-extract` row (all
    three SQL systems agreeing `1116`, `repeat=7`) IS the corpus's one
    genuine, publication-grade, non-degenerate measurement of this
    scenario — `duckdb-cityparquet` 0.006087s vs `cjdb` 0.033402s vs
    `3dcitydb` 0.008433s. **The claim's evidentiary status is therefore
    narrower than "no evidence exists anywhere", but still narrow**:
    exactly ONE dataset (`delft`) supports it, out of five —
    `Montreal`/`Vienna`/`Zurich`/`lod3_railway`'s own `lod-extract` rows
    remain degenerate for `duckdb-cityparquet` for the structural reason
    above and must still not be read as corroborating or contradicting
    the projection-pushdown claim either way. Not patched or redesigned
    here per instruction (a differently-defined `lod-extract` — e.g.
    targeting whichever LoD column a dataset actually carries — is a
    scenario-design decision out of scope for this fix wave); reported so
    a reader does not mistake any currently-committed `lod-extract` timing
    comparison outside `delft` for evidence of anything.

## The heterogeneity corpus (Task 14)

`Montreal`/`Vienna`/`Zurich`/`lod3_railway` (delft's fixture is committed
directly and needs none of this) are fetched and checksum-pinned, not
committed as data:

```sh
./scripts/fetch_corpus.sh          # downloads to data/, verifies against scripts/corpus.sha256
```

On first run this WRITES `scripts/corpus.sha256` from the freshly
downloaded, pristine bytes; every later run verifies against it and fails
loudly (`sha256sum -c`) on any upstream change. **The four hashes
currently pinned in `scripts/corpus.sha256`** (`Montreal.city.jsonl`,
`Vienna.city.jsonl`, `Zurich.city.jsonl`, `lod3_railway.city.json`) are the
PRISTINE, as-downloaded bytes — see the next paragraph for why
`sha256sum -c` against the working copies in `data/` will legitimately
FAIL, and why that is not a corpus-integrity problem.

None of the four pristine downloads declares a
`metadata.referenceSystem` (verified directly against the real files, not
assumed), and `cityparquet convert` refuses outright to write a package
for a CRS-bearing coordinate source that declares no CRS — so each must
be stamped with its real-world EPSG code before anything else in this
harness can run against it:

```sh
python3 scripts/stamp_crs.py data/Montreal.city.jsonl      2950
python3 scripts/stamp_crs.py data/Vienna.city.jsonl        31256
python3 scripts/stamp_crs.py data/Zurich.city.jsonl        2056
python3 scripts/stamp_crs.py data/lod3_railway.city.json   7415
```

Idempotent (a source that already declares a `referenceSystem` is left
untouched) and lossless for CityJSONSeq sources (only the header line is
rewritten; every feature line streams through byte-for-byte, so Zurich's
259MB file is never fully loaded into memory). Each EPSG code above was
independently determined and cross-checked for this task by transforming
`params/<dataset>.json`'s own `bbox_full` corners with PROJ (`cs2cs`) and
confirming the result lands at the real city's known location (watch for
`EPSG:31256`'s own authority-defined axis order — (northing, easting), not
the more common (easting, northing) — a `cs2cs` pitfall that looks like a
wrong EPSG code but isn't):

| dataset | EPSG | `cs2cs`-transformed bbox corner (WGS84) | known location |
|---|---|---|---|
| `Montreal` | 2950 (NAD83(CSRS) / MTM zone 8) | 45.506°N, 73.561°W | Montreal, Quebec |
| `Vienna` | 31256 (MGI / Austria GK East) | 48.202°N, 16.345°E | Vienna, Austria |
| `Zurich` | 2056 (CH1903+ / LV95) | 47.323°N, 8.459°E | Zurich, Switzerland |
| `lod3_railway` | 7415 (Amersfoort / RD New + NAP) | — (not geographic) | synthetic ~12×7×1.5m LoD3 test scene near the coordinate origin; stamped with this harness's own pre-existing default purely to satisfy the writer's/importers' hard CRS requirement, no real-world location implied |

`scripts/stamp_crs.py`'s own module docstring carries the full rationale
and re-run instructions (re-running `fetch_corpus.sh` restores the
pristine, CRS-less bytes, so the stamping step must be re-run after any
re-fetch to restore the working copy every other command in this file
reads from).

**`lod3_railway` needs one further, `cjdb`-specific step.** cjdb's own
`is_valid_file()` (`cjdb/modules/utils.py` in the patched build) rejects
any path not ending `.jsonl` outright — a pure filename check, confirmed
by reading the source, not content-based — and `lod3_railway.city.json`
is a single-document CityJSON file, not CityJSONSeq. Bypassing the check
by renaming would not work either: `process_file()` treats line 1 as a
CityJSONSeq HEADER and extracts only `metadata`/`transform`/
`geometry-templates`/`extensions` from it, never re-reading an embedded
`CityObjects` dict — a renamed single-document file would import
**zero** CityObjects silently rather than error, a worse failure mode
than the loud one the extension check actually produces. cjdb's own error
message names the fix (`Use cjio to convert your city.json file to
city.jsonl`), and `cjio` ships as part of cjdb's own dependency chain:

```sh
uv run --with .cjdb-patched/cjdb-2.2.0+<hash> cjio data/lod3_railway.city.json export jsonl data/lod3_railway.city.jsonl
```

Verified lossless (121 CityObjects on both sides, same
`referenceSystem`/`transform`, a proper header line with an EMPTY
`CityObjects` followed by 38 genuine `CityJSONFeature` lines). Point
`just bench`/`just derive-params` at the resulting `.jsonl` file, not the
original `.json` — this affects every system uniformly, matching every
other heterogeneity-corpus dataset's own `.jsonl` shape (`cityparquet`/
`cityparquet-hilbert`/`duckdb-cityparquet` never read `dataset.source` at
all, only the pre-converted package directories, so this choice only
changes what `cjdb`/`3dcitydb` themselves ingest — confirmed `citydb-tool`
also accepts the `.jsonl` form directly).

## Reproduction

```sh
cd benchmark/databases
just up            # start both PostgreSQL containers (podman-compose), wait for health
just patch-cjdb    # build patched cjdb once (see Caveat 2); re-run after editing the patch
just build-citydb  # build the pinned citydb-tool image once
just smoke         # full pipeline on the small delft fixture, repeat=2, exits non-zero on any count mismatch
```

**3DCityDB's SRID is baked in at volume creation and cannot be changed
afterwards** (`docker/compose.yml`'s `citydb-db.environment.SRID`, sourced
from the `CITYDB_SRID` shell variable, default `7415` — delft's and
lod3_railway's own EPSG). Getting this wrong does NOT error — it silently
mislabels or reprojects every subsequent spatial result for that dataset.
Export the dataset's real EPSG (see the corpus table above) BEFORE `just
up`, and `just down` (which drops both containers' volumes) before
switching datasets:

```sh
just down                       # drop any stale volumes first
CITYDB_SRID=2056 just up        # e.g. Zurich; omit (defaults to 7415) for delft/lod3_railway
```

Verify it landed — read back from the LIVE database, not the requested
value — before trusting anything ingested against it:

```sh
psql -h localhost -p 55433 -U bench -d bench -c "SELECT srid FROM citydb.database_srs;"
```

`results/<dataset>.manifest.json`'s own `srid` object does exactly this
verification automatically, for both `cjdb` and `3dcitydb`, on every
`just bench` run (`cli.py`'s `_srids()`, sourced from each adapter's own
post-ingest database read-back — see `manifest.collect()`'s docstring).

For a full run against a named dataset, with the designed warm protocol:

```sh
just derive-params data/<dataset>.city.jsonl   # once per new dataset — writes params/<dataset>.json
just bench <dataset> [REPEAT]                  # REPEAT defaults to 7
```

`just bench` writes `results/<dataset>.csv`, `results/<dataset>.manifest.json`
and `results/<dataset>.indexes.sql` from scratch on every run.

```sh
just down           # tear down both containers and their volumes
```

**Operational note**: `citydb-tool import cityjson` has no `--overwrite`/
replace flag (confirmed via `citydb-tool import cityjson --help`; its
`--import-mode` choices are `import_all`/`skip`/`delete`/`terminate`, none
of which is "replace the schema"). Re-running an ingest into an already-
populated 3DCityDB schema silently **doubles** every count rather than
erroring — always `just down` (which drops both containers' volumes)
before re-ingesting, not just re-running `just bench`/`just smoke` on top
of an existing stack.

## Environment

Captured from `results/delft.manifest.json` (this run's `host` block) and
cross-checked live for this task:

```
platform: Linux-6.8.0-136-generic-x86_64-with-glibc2.39
processor: x86_64
python:    3.12.1
duckdb (Python client): 1.5.5
cjdb:      2.2.0+ground-surfaces-tie-patch
citydb-tool: 1.3.2
3DCityDB:  5.1.2
PostgreSQL: 16.4 (both cjdb-db and citydb-db, confirmed live)
PostGIS:   3.4 USE_GEOS=1 USE_PROJ=1 USE_STATS=1 (both, confirmed live)
```

`peak_rss_bytes`'s KiB-not-bytes caveat (Caveat 6, and the erratum
recorded there) applies to this platform's committed numbers.
