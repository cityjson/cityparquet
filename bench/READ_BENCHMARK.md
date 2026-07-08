# CityParquet read-benchmark methodology

This is the **read**-side counterpart to `bench/README.md` (the M5
write-benchmark doc): same repo, same discipline (real fixtures, warm
medians + MAD at 6-decimal precision, disclosed rather than hidden fixed
overheads), but a separate methodology and a separate set of committed
artefacts (`bench/read_results/*.csv`, produced by `just readbench-all` /
`cityparquet-readbench run` + `scripts/readbench_duckdb.sh`, not
`bench/results/*.csv`).

## Purpose

Compare **read** performance — wall-clock time and memory — of five
formats plus a SQL-engine baseline, across seven access-pattern scenarios
that mirror how a consumer of 3D city model data actually reads it: a full
scan, a metadata-only count, a spatial window query at three selectivities,
an attribute-equality filter, a numeric-attribute aggregate, a single-id
lookup, and a single-column projection. The read side is the geometry- and
query-facing half of the CityParquet argument; the write side (encoding
size, write time, row-group pruning) is already covered by `bench/README.md`.

## Formats

| format tag | what it is | index available |
|---|---|---|
| `cityjsonseq` | plain CityJSONSeq (`.city.jsonl`), one JSON feature per line | none — every scenario is a full parse |
| `cityjsonseq-gz` | the same stream, `gzip -9`'d | none — full parse, plus gzip inflate |
| `cityparquet` | our CityParquet package, **source row order** (`cityparquet convert --overwrite`) | Parquet row-group min/max statistics + column projection |
| `cityparquet-hilbert` | the same package, rows written in Hilbert-curve order (`--ordering hilbert`) | the same Parquet statistics, but tighter per-row-group bboxes from spatial clustering |
| `flatcitybuf` | FlatCityBuf (`fcb ser -A`) | R-tree spatial index (2D) + B+-tree index over **every** attribute (`-A`) |
| `duckdb-parquet` | DuckDB (v1.5.x) SQL, `read_parquet()` directly over our `cityparquet` package's own table | whatever Parquet statistics DuckDB's own scan uses — same file as `cityparquet`, different engine |

`duckdb-parquet` is **not** the M5 write-benchmark's `duckdb-copy` baseline.
`duckdb-copy` there reads CityJSON through the community `cityjson`
extension's `read_cityjson`/`read_cityjsonseq` table functions and re-writes
it via `COPY ... TO (FORMAT PARQUET)` — a baseline with well-documented
partial-geometry gaps (see `bench/README.md`'s "Baseline geometry
coverage"). `duckdb-parquet` here instead runs `read_parquet()` straight
over a `cityparquet-rs`-**written** package: it carries our full geometry
and our typed `bbox` STRUCT column, so none of that write-side coverage
caveat applies to it (see Caveat 5 below for what *does* apply).

## The seven scenarios

Every format implements every scenario via its own natural mechanism —
never a hand-tuned shortcut, never an artificial common code path:

| scenario | common target | `cityparquet` mechanism | `flatcitybuf` mechanism | `cityjsonseq`(+gz) mechanism | `duckdb-parquet` mechanism |
|---|---|---|---|---|---|
| `full-read` | decode every feature's geometry; `(feature_count, boundary_count)` | scan all row groups, decode WKB | `select_all` + `cur_cj_feature`, walked to completion | parse every line, decode geometry | `SELECT sum(hash(COLUMNS(*)))` — forces every column decoded |
| `count` | total feature/object count | Parquet file metadata `num_rows` (O(1), no scan) | `features_count()` header field (O(1)) | count parsed lines (full parse) | `SELECT count(*)` |
| `bbox-query` (1%/5%/25%) | ids/count of objects whose bbox intersects a query window | row-group prune (`with_bbox_row_groups`) + row-level bbox test — **exact** | `select_query(Query::BBox)` — R-tree, **2D only** (see Caveat 4) | parse all, test each feature's own unioned bbox | `WHERE bbox.xmax>=.. AND bbox.xmin<=.. AND bbox.ymax>=.. AND bbox.ymin<=..` (full z window, so no z clause needed) |
| `attr-filter` | count of objects matching `attr == v` (or a numeric range) | `RowFilter` (`ArrowPredicateFn`) + row-group statistics prune | B+-tree attribute index (`select_attr_query`) | parse all, test each CityObject's `attributes` | `WHERE object_type = '<v>'` |
| `attr-stats` | `(min, max, sum, count)` of a numeric attribute | min/max from Parquet column-chunk statistics (near-free); sum/count from a 1-column projected scan | full walk, aggregate (no numeric-range index) | parse all, aggregate | `SELECT min(c), max(c), sum(c), count(c)` |
| `id-lookup` | the single object with a given id, materialised | `RowFilter` on `id` + decode of the one surviving row | B+-tree attribute index on the id field | parse until found | not run (id lookup is not a distinct DuckDB SQL pattern worth timing separately from `attr-filter`'s `WHERE` plan; the coordinator's own `cityparquet`/`flatcitybuf`/`cityjsonseq` rows carry it) |
| `project` | one attribute column read across every row; non-null count | single-column `ProjectionMask` | full walk, read that one attribute | parse all, read that attribute | `SELECT count(object_type)` |

`bbox-query` is measured at **three** selectivity targets — windows sized to
~1%, ~5%, and ~25% of the dataset's own x/y bbox extent, anchored at its
lower-left corner (the same window construction `bench/README.md`'s M5
harness uses for its own single window) — one CSV row per target, tagged
`bbox-1pct`/`bbox-5pct`/`bbox-25pct` in `notes`.

## Metrics and the CSV contract

`bench/read_results/*.csv`, one row per (dataset, format, scenario
[, selectivity target]):

```
dataset,format,scenario,selectivity,result_count,time_s,time_mad_s,peak_heap_bytes,peak_rss_bytes,repeat,notes
```

- `time_s` / `time_mad_s` — **warm-cache** median and median-absolute-
  deviation of `repeat` samples (default 7; one further, discarded warmup
  precedes them), 6-decimal precision. A fresh child process is spawned per
  sample (see "Warm vs cold" below) — independent OS page-cache and
  independent `peak_alloc` state per sample, never reused across repeats.
- `peak_heap_bytes` — the `peak_alloc` global-allocator high-water mark for
  that one in-process scenario call. **Empty for `duckdb-parquet`**: DuckDB
  runs out-of-process, so there is no allocator hook into it the way the
  `--child` protocol has one into `cityparquet-readbench` itself.
- `peak_rss_bytes` — the child's own `getrusage(RUSAGE_SELF).ru_maxrss`
  (`cityparquet`/`cityparquet-hilbert`/`flatcitybuf`/`cityjsonseq`/
  `cityjsonseq-gz`), or a separate untimed `/usr/bin/time -l`/`-v` capture
  around the same query (`duckdb-parquet`). **Units are platform-dependent
  and NOT normalised**: this is **bytes** on macOS (`ru_maxrss`'s native
  unit on BSD/Darwin, this benchmark's development platform) but **KiB** on
  Linux (glibc's `getrusage(2)`) — the environment block below states which
  machine produced the committed numbers; a reader combining these CSVs
  with a Linux-produced run must convert one side.
- `selectivity` = `result_count / total_object_count`, empty where N/A
  (`count`, `full-read`). See Caveat 2 for what `total_object_count` means
  per scenario.
- `notes` — free text: the `bbox-*pct` selectivity tag, the attribute
  name/predicate used for `attr-filter`/`attr-stats`/`project`, the sampled
  id for `id-lookup`, or `cold` for the one cold-cache row.

## Warm vs cold protocol

The **headline number is the warm-cache median**: `repeat` fresh child
processes (default 7), a further discarded warmup beforehand, OS page cache
and (for the in-process formats) allocator state left however the previous
sample left them — i.e. "warm" describes the OS/filesystem cache, not a
long-lived process, since every sample is already a brand-new process (see
`crates/cityparquet-readbench/src/coordinator.rs`'s own module doc on why:
independent peak-RSS and independent cache state per sample, mirroring
FlatCityBuf's own `benches/read.rs` harness).

**Cold** is a single, separate measurement per format: `cityparquet-readbench
run --cold` runs one additional `full-read` after prompting the operator to
run `sudo purge` (macOS) — or the Linux equivalent,
`echo 3 | sudo tee /proc/sys/vm/drop_caches` — to evict the OS disk/page
cache first. The coordinator cannot invoke `sudo` itself, so this is a
manual, one-format-at-a-time step, never folded into the bulk
`just readbench-all` run. The resulting row is tagged `cold` in `notes` and
is never averaged, medianed, or otherwise mixed with the warm samples —
each cold number stands alone, one per format, one `full-read` only.

## Fairness caveats (read before citing a number)

1. **Counting granularity differs by scenario, not just by format.**
   CityParquet's `count`/`full-read` count **one row per CityObject** —
   parents *and* children each get a row. `cityjsonseq`(+gz) and
   `flatcitybuf` instead count top-level **features** for `count`/
   `full-read`/`bbox-query` (a CityJSONSeq/FCB feature bundles one
   top-level CityObject with all its children inline). But `attr-filter`,
   `attr-stats`, `project`, and `id-lookup` are **CityObject-granular in
   every format** — `cityjsonseq`/`flatcitybuf` deliberately flatten to
   per-CityObject counting for exactly these four scenarios (`flatcitybuf`
   because that is what its B+-tree attribute index naturally returns per
   entry; `cityjsonseq` by explicit choice, to match) — so these four are
   directly, honestly comparable across every format; `count`/`full-read`/
   `bbox-query` are not. Empirically, on the two committed fixtures:
   `lod3_railway.city.json` is 121 CityObjects / 38 top-level features;
   `delft.city.jsonl`'s `object_type == "BuildingPart"` count is 1116
   CityObjects (out of 2231 total CityObjects / 1115 features).

2. **Selectivity's denominator differs by scenario, on purpose.** The four
   CityObject-granular scenarios (`attr-filter`/`attr-stats`/`project`/
   `id-lookup`) divide by the **dataset-global CityObject total** — the
   same number as CityParquet's own `count` — as a single shared
   denominator across every format, so their selectivity is always in
   `(0, 1]` and directly comparable format-to-format. `bbox-query` instead
   divides by **each format's own** feature/object total (its own `count`
   result), because a spatial query's numerator is native to that format's
   own counting unit (see Caveat 1) — a shared CityObject denominator would
   make `bbox-query` selectivity exceed 1 for the feature-counting formats
   whenever a feature contains more than one matching CityObject. This is
   why selectivity is a meaningful, bounded number in this benchmark rather
   than an artefact to explain away.

3. **`full-read`'s materialisation is honestly different work per format,
   not identical work in different clothes.** CityParquet decodes every
   row's WKB geometry and counts surfaces; CityJSONSeq(+gz) serde-parses
   every JSON line and traverses its geometry; FlatCityBuf decodes its own
   FlatBuffers representation; DuckDB runs
   `SELECT sum(hash(COLUMNS(*)))` (forcing every column, including every
   geometry column, to be decoded — the same "force full decode" pattern
   `bench/README.md`'s M5 baseline uses). Each is that format's own honest
   full-read cost — reported per-format, never normalised into a shared
   unit of work that doesn't actually exist across four different
   encodings.

4. **FlatCityBuf's `bbox-query` is 2D.** Its spatial R-tree indexes x/y
   only; the query window's z component is silently dropped when querying
   FCB (the window itself is still constructed with a full z range, as it
   is for every other format, but FCB's own index simply has no z
   dimension to test against). A comparison of `bbox-query` result counts
   between FlatCityBuf and any 3D-tested format (CityParquet, DuckDB) can
   therefore only ever show FlatCityBuf matching **more** rows for the same
   window, never fewer, purely from the missing z test — not a query-plan
   or index-quality difference.

5. **`duckdb-parquet` reads OUR CityParquet package directly — the M5
   write-side geometry-coverage caveats do NOT carry over.** Unlike
   `bench/README.md`'s `duckdb-copy`/`duckdb-copy-zstd` rows (which go
   through the community `cityjson` extension's `read_cityjson`/
   `read_cityjsonseq`, documented there to write 0% `geom_lod0` coverage
   everywhere and 0% of *everything* on `lod3_railway.city.json`),
   `duckdb-parquet` here runs `read_parquet()` straight over a
   `cityparquet-rs`-written package — full geometry, every LoD column.
   **However**: our packages' WKB geometry columns carry GeoParquet "geo"
   file metadata, and DuckDB's spatial extension (autoloaded) eagerly
   tries to decode them into its own native `GEOMETRY` type the instant a
   query references the column — even a bare `SELECT` with no function
   applied — and that native decode does not support the multi-surface/
   solid WKB shapes our LoD1.2/LoD1.3/LoD2 geometries use, failing with
   `Invalid Input Error: Unsupported geometry type in WKB`.
   `scripts/readbench_duckdb.sh` sets `enable_geoparquet_conversion=false`
   on every invocation to work around this, which makes every geometry
   column read back as plain `BLOB` instead. **Anyone running an ad-hoc
   DuckDB query against a `cityparquet-rs` package that touches a geometry
   column needs the same setting**, or will hit the identical error.

6. **`duckdb-parquet` has no `peak_heap_bytes`** (see the CSV contract
   above — it is an out-of-process SQL engine, not a `--child` process with
   an allocator hook) and every one of its `time_s` samples carries a fixed
   per-invocation DuckDB process-startup overhead (~0.06 s on the
   committed-run machine below; plain `read_parquet()` needs no
   `INSTALL`/`LOAD`, so this is pure process/interpreter startup, not
   extension loading). `scripts/readbench_duckdb.sh` measures this via 5
   timed `SELECT 1;` calls and prints it as a `# calibration:` stderr line
   before every run — it is **disclosed, never subtracted**, from any
   reported `time_s`/`time_mad_s`.

7. **Warm vs cold — never silently mixed.** The headline numbers everywhere
   in this document and in `bench/read_results/*.csv` are warm-cache
   medians; the single `cold`-tagged row per format (see "Warm vs cold"
   above) is a distinct, separately-reported measurement, always
   `full-read` only, never averaged into or compared unlabelled against the
   warm rows.

8. **Sub-millisecond deltas are noise; single-threaded reads are pinned.**
   As in `bench/README.md`'s M5 methodology, deltas under roughly 10 ms at
   `repeat = 7` are within scheduler/filesystem-cache noise and are not
   cited as a finding by themselves. Every format's reads here run
   single-threaded (no Parquet multi-threaded row-group decode, no
   DuckDB multi-threaded query execution) — a deliberate, disclosed choice
   so timing differences reflect the format/mechanism, not thread-count
   parallelism a production deployment might or might not enable.

## Environment

Captured 2026-07-08 (the machine this milestone's committed
`bench/read_results/*.csv` are produced on — filled in by Task 14; the
`peak_rss_bytes` unit note above applies to this machine):

```
uname -a: Darwin F19WYJD2P7 25.5.0 Darwin Kernel Version 25.5.0: Tue Jun  9 22:28:34 PDT 2026; root:xnu-12377.121.10~1/RELEASE_ARM64_T6041 arm64
CPU:      Apple M4 Max
RAM:      38654705664 bytes (36 GiB)
duckdb:   v1.5.3 (Variegata) 14eca11bd9
cargo:    cargo 1.93.1 (083ac5135 2025-12-15)
rustc:    rustc 1.93.1 (01f6ddf75 2026-02-11)
fcb:      fcb 0.7.4
```

`peak_rss_bytes` on this machine is therefore in **bytes** (BSD/Darwin
`ru_maxrss`), not KiB — see the CSV-contract note above.

## Reproduce

```sh
just fixtures          # the two committed CityJSON fixtures (network)
just readbench-prepare tests/fixtures/delft.city.jsonl   # per-format artefacts for one input
just readbench-all      # prepare + coordinator + DuckDB baseline for both fixtures; REPEAT='n' to override
```

`readbench-all` rewrites `bench/read_results/delft.csv` and
`bench/read_results/railway.csv` from scratch on every run (never appends
across runs — a fresh `rm -f` precedes each), so a committed run is always
one machine, one sitting, per dataset. Per-dataset manual invocation (what
`readbench-all` itself calls):

```sh
just readbench-prepare <input> bench/data/readbench
cargo run --release -p cityparquet-readbench -- run \
    --input <input> --prepared-dir bench/data/readbench \
    --out bench/read_results/<name>.csv --repeat 7
./scripts/readbench_duckdb.sh bench/data/readbench/<name>.parquet \
    bench/read_results/<name>.csv --numeric-column <col>
```

`--numeric-column` is only needed to enable the DuckDB baseline's
`attr-stats` row; omit it for datasets with no numeric attribute (e.g.
`lod3_railway.city.json`, where `attr-stats` is skipped for every format —
see `crates/cityparquet-readbench/src/coordinator.rs`'s own
`pick_numeric_attribute`, logged on stderr, never fabricated).
