# CityParquet read-benchmark methodology

This is the **read**-side counterpart to `bench/README.md` (the M5
write-benchmark doc): same repo, same discipline (real published data, warm
medians + MAD at 6-decimal precision, disclosed rather than hidden fixed
overheads), but a separate methodology and a separate set of measurement
artefacts (`bench/read_results/*.csv`, produced by `just bench` /
`cityparquet-readbench run` + `scripts/readbench_duckdb.sh`, not
`bench/results/*.csv`).

> ## ⚠ RESULTS STATUS — every number below is STALE. Do not quote one.
>
> **This document's methodology is current; its numbers are not.** As of
> 2026-08-17 there are **no committed read-benchmark CSVs at all**: the
> `bench/read_results/*.csv` this document was written around were deleted in
> `84a2b38`, because branch-level changes to the full-read and convert paths
> meant their `time_s` values no longer described HEAD. Three further changes
> have landed since, each of which independently invalidates the old figures:
>
> 1. **The corpus was replaced** (`fdfc1c1`). `just fetch-data` now fetches 30
>    real published CityGML 2.0 / CityJSON 2.0 documents from the city3d STAC
>    catalogue; the 11 pre-converted CityJSONSeq datasets every published
>    figure was measured on moved to `just fetch-seq-data` and a *different*
>    directory. Not one dataset named in `bench/CORPUS_REPORT.md`'s tables is
>    in the default corpus any more. (The two `just fixtures` files —
>    `delft.city.jsonl`, `lod3_railway.city.json` — are unaffected: they are
>    test fixtures, not corpus, and the object counts this document quotes from
>    them are asserted by tests and remain true.)
> 2. **Three format tags were added** — `citygml`, `cityjson` and
>    `cityparquet-hilbert` — so the format axis itself is different. Every
>    published table is missing rows that now exist.
> 3. **The default format set changed**, and the ordering comparison was split
>    into a benchmark of its own (see "The two benchmark sets" below), so a CSV
>    produced by a bare run no longer holds the same series as before.
>
> Anything numeric in this file, in `bench/CORPUS_REPORT.md`, or in
> `bench/README.md` is therefore **provenance, not evidence**: it records what
> was once measured and on what, and is retained so the re-run has something
> to be compared against. **Nothing in it may be cited in the paper until the
> re-run described under "Reproduce" has been done and its CSVs committed.**

## Purpose

Compare **read** performance — wall-clock time and memory — of the formats a
3D city model can actually be published in, across seven access-pattern
scenarios that mirror how a consumer of that data actually reads it: a full
scan, a metadata-only count, a spatial window query at three selectivities,
an attribute-equality filter, a numeric-attribute aggregate, a single-id
lookup, and a single-column projection. The read side is the geometry- and
query-facing half of the CityParquet argument; the write side (encoding
size, write time, row-group pruning) is already covered by `bench/README.md`.

## Formats

Eight format tags. The canonical vocabulary, spelling and order are owned by
`Format::ALL` in `crates/cityparquet-readbench/src/format.rs` — this table is
a copy of it, and the CSV's `format` column, the `--formats` flag, the
`readbench_prepare.sh` artefact names and the plotter's ordering all use the
same eight strings. They run left-to-right from "what the data ships as
today" through "what we propose" to "a different engine over the same file":

| format tag | what it is | index available |
|---|---|---|
| `citygml` | CityGML 2.0 XML (`.gml`) — the format most national datasets are published in, read through **this repository's own reader** (`cityparquet::citygml`) | **none** — no offsets, no object directory, no spatial or attribute tree; every scenario is a full XML parse and an in-memory filter (see Caveat 12) |
| `cityjson` | plain, whole-document CityJSON (`.city.json`): one JSON document, one `CityObjects` map, one shared document-level `vertices` array | **none** — the document must be parsed in one piece before any object is readable, so every scenario is a full parse (see Caveat 13) |
| `cityjsonseq` | CityJSONSeq, one self-contained JSON feature per line, feature-local vertices. Read from the PREPARED `<base>.city.jsonl` — `readbench_prepare.sh` always materialises one (copied from a `.city.jsonl` input, `cjseq cat` from anything else), and the runner refuses a CityGML document outright | **none** — every scenario is a full parse |
| `cityjsonseq-gz` | the same stream, `gzip -9`'d | **none** — full parse, plus gzip inflate |
| `flatcitybuf` | FlatCityBuf (`fcb ser -A`) | R-tree spatial index (**2D only**, see Caveat 4) + B+-tree index over **every** attribute (`-A`) |
| `cityparquet` | our CityParquet package, **source row order** (`cityparquet convert --overwrite`) | Parquet row-group min/max statistics + column projection |
| `cityparquet-hilbert` | the same package, rows written in Hilbert-curve order (`--ordering hilbert`) | the same Parquet statistics, but tighter per-row-group bboxes from spatial clustering |
| `duckdb-parquet` | DuckDB (v1.5.x) SQL, `read_parquet()` directly over our `cityparquet` package's own table | whatever Parquet statistics DuckDB's own scan uses — same file as `cityparquet`, different engine |

The first four are **unindexed by construction**: a published `.gml`,
`.city.json` or `.city.jsonl` carries no way to answer any question without
reading all of it. That is not an omission in the harness — it is the finding
the benchmark exists to quantify, and the reason a `count` gap grows linearly
with dataset size while CityParquet's stays flat.

## The two benchmark sets

The benchmark answers **two different questions**, and measuring them in one
run would confound both — a CSV holding `citygml`, `cityjson`, `cityjsonseq`,
`flatcitybuf`, `cityparquet` *and* `cityparquet-hilbert` cannot tell you
whether a CityParquet-vs-FlatCityBuf gap is about the encoding or about the
row order, because two variables moved at once. So there are two sets, each
single-axis, each in its own results directory (`just plot` charts a whole
directory, so mixing them would put two axes on one chart and answer
neither). Both are defined once, in `Format` (`format.rs`), and threaded from
there to the justfile and the coordinator:

| set | tags | recipe | output |
|---|---|---|---|
| **Format comparison** (`Format::DEFAULT_SET`) — *how do the formats a city model can ship as compare?* | `citygml`, `cityjson`, `cityjsonseq`, `flatcitybuf`, `cityparquet-hilbert` | `just bench <folder>` (no `FORMATS`) | `bench/read_results/` |
| **Ordering comparison** (`Format::ORDERING_SET`) — *does Hilbert-curve ordering pay for itself?* | `cityparquet`, `cityparquet-hilbert` | `just ordering-bench <folder>` | `bench/ordering_results/` |

Three deliberate choices in that first row:

- **One tag per format family.** The format axis carries exactly one
  CityParquet row, so the chart compares formats and nothing else.
- **CityParquet is represented by its BEST configuration**,
  `cityparquet-hilbert` — the configuration we would actually ship. Entering
  the source-ordered package instead would handicap the format comparison
  with an ordering choice no other format in the set faces, and entering both
  would confound the axes. The ordering choice is a real question, and it is
  asked separately, on its own axis, by the second row.
- **`cityjsonseq-gz` and `duckdb-parquet` are opt-in, not default.** Neither
  is a *format*: `cityjsonseq-gz` is a compression variant of a format
  already in the set, and `duckdb-parquet` is an SQL-**engine** baseline over
  a file already in the set. Neither belongs on a format axis unasked, so
  **a bare `just bench <folder>` produces a CSV with exactly the five
  `DEFAULT_SET` series in it** — no sixth, non-format row. Both are measured
  on request, by naming them:

  ```sh
  just bench <folder> bench/read_results \
      "citygml,cityjson,cityjsonseq,cityjsonseq-gz,flatcitybuf,cityparquet-hilbert,duckdb-parquet"
  ```

  `duckdb-parquet` is the *only* thing that triggers the
  `scripts/readbench_duckdb.sh` append step. This is pinned in both
  directions — a bare run must not append it, naming it must — by
  `scripts/tests/bench_recipe_test.sh`, which extracts the `bench` recipe's
  own format-selection block from the justfile and runs it. The justfile used
  to disagree with `Format::DEFAULT_SET` here, appending the baseline on every
  default run; the test exists so that cannot come back.

`duckdb-parquet` is **not** the M5 write-benchmark's `duckdb-copy` baseline.
`duckdb-copy` there reads CityJSON through the community `cityjson`
extension's `read_cityjson`/`read_cityjsonseq` table functions and re-writes
it via `COPY ... TO (FORMAT PARQUET)` — a baseline with well-documented
partial-geometry gaps (see `bench/README.md`'s "Baseline geometry
coverage"). `duckdb-parquet` here instead runs `read_parquet()` straight
over a `cityparquet-rs`-**written** package: it carries our full geometry
and our typed `bbox` STRUCT column, so none of that write-side coverage
caveat applies to it (see Caveat 5 below for what *does* apply).

## HTTP transport

Every scenario above can also run against **real cloud object storage over
HTTP** instead of a local file — `--transport local|http` (default `local`,
identical to everything above) and `--base-url <url>` on both
`cityparquet-readbench run` and its own `--child` protocol. This measures
each format's actual **cloud-native access pattern**: how many bytes and
HTTP requests a scenario costs when the file lives behind a network, not
just how long it takes locally.

- **Real cloud storage, not a bundled server.** `--base-url` must point at a
  real HTTPS endpoint (S3, Cloudflare R2, or any static host serving `Range`
  requests) hosting a `just readbench-prepare`d directory uploaded wholesale
  — see `scripts/readbench_upload.md`. There is no local HTTP server this
  repo spins up for a real run (only test-only in-process servers inside
  `cargo test`, never part of the measured path).
- **Network variance is real and disclosed, not hidden.** Unlike the local,
  same-machine `time_s`/`time_mad_s`, an http-transport row's timing
  variance includes real network latency/jitter — the MAD (`time_mad_s`)
  column now also captures that, not just OS/filesystem-cache noise. A
  committed http-transport run is a snapshot of one network path at one
  time, not a reproducible local benchmark.
- **Two extra metrics, per scenario: bytes transferred and HTTP request
  count — successful, LOGICAL reads, not raw wire traffic.** The CSV's
  trailing `bytes_read`/`http_requests` columns (see the CSV contract above)
  are empty for every `local`-transport row and populated for every
  `http`-transport row, straight from each format's own transport-agnostic
  reader. Both tallies count successful logical range/GET calls the reader
  itself makes — a failed attempt is not counted, and any retry the
  underlying HTTP client performs internally (connection resets, transient
  5xx, etc.) is invisible to this tally. On a lossy real network the
  reported numbers can therefore be a lower bound on actual wire traffic,
  not an exact packet count — still exactly the right level to compare
  formats' *access patterns* against each other, which is what this
  benchmark is for.
  - `cityparquet`: an `object_store`/`ParquetObjectReader`-based async
    reader (`crates/cityparquet/src/query_async.rs`) shares the exact same
    row-group-pruning/projection/predicate logic as the local sync reader
    (`crates/cityparquet/src/query.rs`) — same query, same pruning
    decisions, only the I/O source differs. A `CountingObjectStore`
    decorator (`crates/cityparquet/src/counting_store.rs`) tallies every
    range request the reader actually makes.
  - `flatcitybuf`: `fcb_core`'s own `HttpFcbReader`
    (`fcb_core::http_reader`) drives its native R-tree/B+-tree indexes over
    HTTP range requests, tallied by a `CountingRangeClient` wrapper
    (`crates/cityparquet-readbench/src/formats/flatcitybuf.rs`).
  - `citygml`, `cityjson`, `cityjsonseq`(+gz) — every unindexed format: a
    single **whole-object GET** — exactly 1 request, the whole file's byte
    length, by construction, regardless of scenario (there is no index to
    prune with, so there is nothing smaller to fetch). All four route through
    the same `CountingObjectStore` as `cityparquet`, so the tally is measured
    rather than asserted.
  - `duckdb-parquet` has no HTTP-transport row; it is a local-only SQL
    baseline (`scripts/readbench_duckdb.sh`), unaffected by `--transport`.

  This is the benchmark's headline cloud-native argument: CityParquet and
  FlatCityBuf pull kilobytes via a handful of range requests for a selective
  query; CityGML, CityJSON and CityJSONSeq pull the entire file over the
  network every time — and on this corpus "the entire file" runs to 1.86 GB.
- **The coordinator's own `QueryParams` derivation stays local regardless of
  `--transport`.** The dataset bbox, the sampled `object_type`/numeric
  attribute/id, and the shared CityObject total are always read directly
  from the local `--prepared-dir` (see `crates/cityparquet-readbench/src/
  coordinator.rs`'s own module doc). This means an http-transport run still
  needs the prepared artefacts present *locally* too (to derive query
  parameters), in addition to uploaded to the served URL.
- **One untimed `Count` preflight per *resolved* format also goes over HTTP
  under `--transport http`, not just the timed measurement rows.** For each
  format actually being benchmarked, the coordinator issues one untimed
  `Count` child call to establish that format's own total (used as the
  `bbox-query` selectivity denominator) — under `--transport http` this
  preflight uses the same http `Source` as every other row for that format,
  so it *does* touch the network, but its own bytes/requests are not folded
  into any CSV row (only the timed rows below it are reported). This is a
  small, fixed amount of extra untimed traffic per format per run (one
  `Count`, the cheapest scenario), disclosed here rather than silently
  absent from the reported totals.

## The seven scenarios

Every format implements every scenario via its own natural mechanism —
never a hand-tuned shortcut, never an artificial common code path:

| scenario | common target | `citygml` mechanism | `cityjson` mechanism | `cityjsonseq`(+gz) mechanism | `flatcitybuf` mechanism | `cityparquet`(+`-hilbert`) mechanism | `duckdb-parquet` mechanism |
|---|---|---|---|---|---|---|---|
| `full-read` | decode every feature's geometry; `(feature_count, boundary_count)` | stream every `cityObjectMember` (quick-xml), decoding every `gml:pos`/`posList`, resolving every `xlink:href` surface reference and rebuilding a feature-local vertex pool, then walk each geometry's `boundaries` tree | parse the whole document, then **resolve every boundary leaf** through the shared `vertices` + `transform` — *not* the same operation as `cityjsonseq`'s (Caveat 13) | parse every line, walk each feature's own `boundaries` tree | `select_all` + `cur_cj_feature`, walked to completion | scan all row groups, decode WKB | `SELECT sum(hash(COLUMNS(*)))` — forces every column decoded |
| `count` | total feature/object count | count `cityObjectMember`s (full parse) | size of the `CityObjects` map (full parse) | count parsed lines (full parse) | `features_count()` header field (O(1)) | Parquet file metadata `num_rows` (O(1), no scan) | `SELECT count(*)` |
| `bbox-query` (1%/5%/25%) | ids/count of objects whose bbox intersects a query window | parse all, test each member's own unioned bbox | parse all, test each CityObject's bbox (min/max over the vertices its geometries reference, resolved through `transform`) | parse all, test each feature's own unioned bbox | `select_query(Query::BBox)` — R-tree, **2D only** (see Caveat 4) | row-group prune (`with_bbox_row_groups`) + row-level bbox test — **exact** | `WHERE bbox.xmax>=.. AND bbox.xmin<=.. AND bbox.ymax>=.. AND bbox.ymin<=..` (full z window, so no z clause needed) |
| `attr-filter` | count of objects matching `attr == v` (or a numeric range) | parse all, test each CityObject's `attributes` | parse all, test each CityObject's `attributes` | parse all, test each CityObject's `attributes` | B+-tree attribute index (`select_attr_query`) | `RowFilter` (`ArrowPredicateFn`) + row-group statistics prune | `WHERE object_type = '<v>'` |
| `attr-stats` | `(min, max, sum, count)` of a numeric attribute | parse all, aggregate | parse all, aggregate | parse all, aggregate | full walk, aggregate (no numeric-range index) | min/max from Parquet column-chunk statistics (near-free); sum/count from a 1-column projected scan | `SELECT min(c), max(c), sum(c), count(c)` |
| `id-lookup` | the single object with a given id, materialised | parse to EOF — **no early exit**, deliberately (see Caveat 9) | parse the whole document, then one map lookup | parse until found (early exit) | B+-tree attribute index on the id field | `RowFilter` on `id` + decode of the one surviving row | not run (id lookup is not a distinct DuckDB SQL pattern worth timing separately from `attr-filter`'s `WHERE` plan; the coordinator's own rows carry it) |
| `project` | one attribute column read across every row; non-null count | parse all, read that attribute | parse all, read that attribute | parse all, read that attribute | full walk, read that one attribute | single-column `ProjectionMask` | `SELECT count(object_type)` |

`cityparquet` and `cityparquet-hilbert` share one runner and one column here:
a Hilbert-ordered package is still a plain CityParquet package on disk, and
the only thing that differs between the two tags is which artefact path
resolves (`Format::artefact`). That is exactly why the ordering question gets
its own single-axis run rather than an extra column.

The three unindexed formats' `attr-filter`/`attr-stats`/`project`/`id-lookup`
mechanisms are not merely *similar*: `citygml` and `cityjson` reuse the
`cityjsonseq` runner's own attribute helpers **verbatim**, so all three agree
on what a column name and an `--attr-eq` predicate mean by construction
rather than by coincidence.

`bbox-query` is measured at **three** selectivity targets — windows sized to
~1%, ~5%, and ~25% of the dataset's own x/y bbox extent, anchored at its
lower-left corner (the same window construction `bench/README.md`'s M5
harness uses for its own single window) — one CSV row per target, tagged
`bbox-1pct`/`bbox-5pct`/`bbox-25pct` in `notes`.

## Metrics and the CSV contract

`bench/read_results/*.csv`, one row per (dataset, format, scenario
[, selectivity target]):

```
dataset,format,scenario,selectivity,result_count,time_s,time_mad_s,peak_heap_bytes,peak_rss_bytes,repeat,notes,bytes_read,http_requests
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
  around the same query (`duckdb-parquet`). **Normalised to bytes on every
  platform** by `rss_to_bytes` in `crates/cityparquet-readbench/src/main.rs`
  (`ru_maxrss` is natively KiB on Linux per `getrusage(2)`, bytes on
  macOS/BSD). CSVs produced by builds *older* than this fix reported the
  raw Linux value, i.e. KiB, under the same column name — see the erratum
  in the parent repo's `benchmarking/README.md` before combining old and
  new runs.
- `selectivity` = `result_count / total_object_count`, empty where N/A
  (`count`, `full-read`). See Caveat 2 for what `total_object_count` means
  per scenario.
- `notes` — a `;`-separated tag list (never a comma: it is one CSV field):
  the `bbox-*pct` selectivity tag, the attribute name/predicate used for
  `attr-filter`/`attr-stats`/`project`, the sampled id for `id-lookup`, or
  `cold` (always first) for the one cold-cache row — plus any DISCLOSURE the
  run made about that row:
  - `no-attr-index` / `attr-index-failed` — FlatCityBuf answered this row by
    a full scan, not by its B+-tree (Caveat 11);
  - `attr-filter-count-mismatch` — the resolved formats disagreed on
    `attr-filter`'s `result_count`, so this run's object-level rows are not
    all measuring the same query (see "Self-consistency" in
    `crates/cityparquet-readbench/src/coordinator.rs`).
- `bytes_read` / `http_requests` — **empty for every `--transport local`
  row** (no HTTP concept locally); for a `--transport http` row, the total
  bytes transferred and HTTP request count that scenario's own
  transport-agnostic reader made (see "HTTP transport" below).

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
`just bench` run. The resulting row is tagged `cold` in `notes` and
is never averaged, medianed, or otherwise mixed with the warm samples —
each cold number stands alone, one per format, one `full-read` only.

## Fairness caveats (read before citing a number)

1. **Counting granularity differs by scenario, not just by format.**
   CityParquet's `count`/`full-read` count **one row per CityObject** —
   parents *and* children each get a row — and `cityjson` matches that grain
   (its `CityObjects` map is flat: a `Building` and its `BuildingPart`s are
   sibling entries linked only by `parents`/`children`). `cityjsonseq`(+gz),
   `flatcitybuf` and `citygml` instead count top-level **features/members**
   for `count`/`full-read`/`bbox-query` (a CityJSONSeq/FCB feature bundles
   one top-level CityObject with all its children inline, exactly as a
   CityGML `cityObjectMember` nests its `BuildingPart`s). So the
   `count`/`full-read`/`bbox-query` rows split into **two grains**:

   | grain | formats |
   |---|---|
   | one row per **CityObject** (children counted separately) | `cityparquet`, `cityparquet-hilbert`, `cityjson`, `duckdb-parquet` |
   | one row per **top-level feature/member** (children inline) | `citygml`, `cityjsonseq`, `cityjsonseq-gz`, `flatcitybuf` |

   But `attr-filter`, `attr-stats`, `project`, and `id-lookup` are
   **CityObject-granular in every format** — `citygml`/`cityjsonseq`/
   `flatcitybuf` deliberately flatten to per-CityObject counting for exactly
   these four scenarios (`flatcitybuf` because that is what its B+-tree
   attribute index naturally returns per entry; the two parsing formats by
   explicit choice, to match) — so these four are directly, honestly
   comparable across every format; `count`/`full-read`/`bbox-query` are not.
   Empirically: `lod3_railway.city.json` is 121 CityObjects / 38 top-level
   features; `delft.city.jsonl`'s `object_type == "BuildingPart"` count is
   1116 CityObjects (out of 2231 total CityObjects / 1115 features);
   `railway_lod3_fragment.gml` is 6 CityObjects / 4 members. Each of those is
   asserted in the runners' own tests, not merely claimed here.

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

   **The guard against a grain mismatch WARNS; it never fails the run.**
   After `attr-filter` has run for every resolved format, the coordinator
   compares their `result_count`s — `object_type` equality is CityObject-level
   in every format, so a healthy run sees them agree exactly — and prints
   either `self-consistency OK: …` or `WARNING: formats disagree on
   AttrFilter(object_type) result_count: …` on **stderr**. It is a diagnostic,
   not a correctness gate: a run whose formats disagreed still writes a
   complete-looking CSV, with nothing in the CSV itself recording that they
   did. It also covers `attr-filter` **only** — never `id-lookup`, and never
   the three feature-grain scenarios. **So the stderr log has to be kept with
   the run**, and a `WARNING: formats disagree` line must be reproduced beside
   any number quoted from that run. CityGML's nested `cityObjectMember`
   hierarchy is the likeliest source of such a disagreement.

3. **`full-read`'s materialisation is honestly different work per format,
   not identical work in different clothes.** CityGML streams and decodes
   every `gml:pos`/`posList`, resolves every `xlink:href` surface reference
   and rebuilds a feature-local vertex pool before it can walk a boundary
   tree at all; plain CityJSON parses one whole document and then resolves
   every boundary leaf through its shared vertex array (Caveat 13);
   CityJSONSeq(+gz) serde-parses every JSON line and traverses its geometry;
   FlatCityBuf decodes its own FlatBuffers representation; CityParquet
   decodes every row's WKB geometry and counts surfaces; DuckDB runs
   `SELECT sum(hash(COLUMNS(*)))` (forcing every column, including every
   geometry column, to be decoded — the same "force full decode" pattern
   `bench/README.md`'s M5 baseline uses). Each is that format's own honest
   full-read cost — reported per-format, never normalised into a shared
   unit of work that doesn't actually exist across six different
   encodings. Where two of them are *labelled* the same but are not the same
   operation, that is called out explicitly rather than left to the label
   (Caveat 13).

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

9. **`id-lookup`'s sampled id is table-order-first — a bias that favours
   *some* scanning formats, but no longer all of them.** The coordinator
   samples the lookup target as the first non-null id in source/table order,
   so a format that answers `id-lookup` by scanning-until-found hits its
   unrepresentative **best case** — an early exit on the first record —
   rather than an average or worst case. That applies to `cityjsonseq`(+gz)
   and to FlatCityBuf's walk when the id is not in its attribute index; for
   those, treat the absolute `id-lookup` time as a *lower bound*, and read
   the mechanism column rather than the raw number.

   **It does NOT apply to `citygml` or `cityjson`, and this is a correction
   to what this caveat said before the two were added.** Neither format can
   take the early exit:

   - **`citygml` deliberately drains the document to EOF, even after a
     hit.** Its skipped-member guard — the check that refuses a document
     containing a `cityObjectMember` this reader does not map (Caveat 12) —
     only becomes authoritative *at EOF*. An early exit would let a document
     whose first member happens to be mapped publish a number while its
     later, unmapped members went unnoticed, i.e. exactly the silently-wrong
     row the guard exists to prevent. Draining is in any case what an
     unindexed format must do to know an id is *absent*, so the cost is
     honest rather than added. Pinned in
     `crates/cityparquet-readbench/src/formats/citygml.rs` (`stream_members`,
     and the `Scenario::IdLookup` arm's own "deliberately no early exit"
     comment).
   - **`cityjson` cannot exit early in principle.** A whole-document
     CityJSON must be parsed in one piece before *any* object is addressable,
     so the map lookup that follows is free and the parse is unavoidable —
     there is no prefix of the file that answers the question.

   So `citygml`'s and `cityjson`'s `id-lookup` rows are already
   representative full-cost numbers, while `cityjsonseq`(+gz)'s are
   best-case. **The three are not comparable to each other as raw times**,
   even though they sit in the same column of the same CSV. A representative
   measurement for the early-exiting formats (a mid/last-order id, or the
   median over several sampled ids) remains future work.

10. **`time_s` is end-to-end read latency, not isolated query compute.** The
    timed window is the whole per-format `run()` call, which INCLUDES opening
    the file, reading Parquet/FlatCityBuf metadata or the CityJSONSeq header,
    and (for CityParquet full-read/id-lookup) a metadata open — not only the
    query kernel. This is deliberate and consistent across every format (each
    pays its own open+read), and it is what a caller issuing a one-shot query
    against a file actually experiences; but it means a sub-millisecond
    `time_s` for a metadata-only scenario (`count`) is dominated by file-open,
    not query work. Interpret the numbers as end-to-end single-query latency,
    not a pure in-memory kernel micro-benchmark.

11. **FlatCityBuf index assumptions and gzip scope (known limitations).** The
    FlatCityBuf runner uses FCB's native indexes (`select_query` for bbox,
    `select_attr_query` for attribute/id), which requires the `.fcb` to carry
    a spatial index (default) and an attribute index (`fcb ser -A`, which
    `readbench-prepare` always passes). If an index query errors, the runner
    falls back to a full scan — and **says so in the CSV `notes`**
    (`no-attr-index` when the column carries no B+-tree at all,
    `attr-index-failed` when the index query itself errored), so an
    index-vs-scan measurement is never silently mislabelled. It used to say
    so on stderr only, which meant a fallback was invisible to everyone
    reading the artefact. The `cityjsonseq-gz` runner fully supports
    CityJSONSeq and single-line whole-document `.city.json.gz` (the form the
    committed fixtures use); a *pretty-printed* multi-line whole-document
    `.city.json.gz` is not yet handled (it needs the fuller sniff
    `cityparquet::source::Source::open` already implements). External review
    (Codex, 2026-07-08) confirmed the query primitives, bbox prune + row-level
    filter, and allocator placement correct; its two flagged "dictionary"
    criticals were verified FALSE POSITIVES — `TypedDictionaryArray::value(i)`
    resolves the row's key, and the committed `attr-filter(object_type)` run
    over 2231 rows with ~4 distinct types would have panicked at row 4 had the
    alleged raw-index reading been real.

12. **`citygml` measures THIS REPOSITORY'S reader, not CityGML's ceiling.**
    This is the single most important caveat on the `citygml` row, and it cuts
    both ways.

    The row answers exactly one question: *what does it cost to answer this
    query against the format the data actually ships in, using the same
    codebase as every other row?* It is **not** a claim about what CityGML
    could achieve in principle. A different parser — a streaming SAX filter
    tuned to one query, an XML database with a pre-built index, a commercial
    CityGML engine — would give different numbers, and nothing measured here
    bounds them. What *is* structural rather than implementation-specific is
    the absence of an index: a published `.gml` carries no offsets, no object
    directory and no spatial or attribute tree, so *any* reader must traverse
    the document to answer any of the seven scenarios. The constant factor is
    ours; the linear term is the format's.

    Two further disclosures about that row, neither folded silently into it:

    - **The appearance pre-pass is skipped.** `FeatureReader::open` re-reads
      the whole document up front to index CityModel-level appearance; not one
      of the seven scenarios consults appearance, so the runner uses
      `open_without_appearance`. On a real 117 MB PLATEAU tile that pre-pass
      was ~35–45% of `count`'s elapsed time and ~20× its peak heap — both
      published CSV columns, and both measuring this harness rather than
      CityGML. Leaving it in would have inflated the `citygml` row with work
      no scenario asks for.
    - **The row describes this reader's supported profile of CityGML** — see
      Caveat 16 for which modules that rules out of the corpus, and why a
      document exercising the gap is refused outright rather than measured.

13. **`cityjson`'s `full-read` is NOT the same operation as
    `cityjsonseq`'s**, even though both rows wear the same scenario label.

    `cityjsonseq` walks each geometry's `boundaries` index tree and stops
    there. `cityjson` additionally **resolves every boundary leaf** through
    the document-level `vertices` array and `transform` into a real-world
    coordinate — on the `lod3_railway` fixture that is **245,137 leaf
    resolutions against 73,554 unique vertices** (the leaves outnumber the
    vertices more than threefold, so this is not a per-vertex pass that could
    be hoisted).

    That extra work is measured, not assumed. *Within the `cityjson` runner*,
    on the same fixture and machine, `full-read` costs roughly a fifth more
    elapsed time than `count` in release mode — median of 9 runs, 0.199 s for
    `count` against 0.243 s for `full-read` — so the leaf resolution is real
    work rather than something the optimiser elides, and
    `std::hint::black_box` pins that rather than trusting it to stay true.
    (That ~20% is a `cityjson`-internal figure, **not** the
    `cityjson`-vs-`cityjsonseq` gap; the cross-format gap also carries the
    whole-document-vs-line-oriented parse difference on top of it.)

    This is **defensible**: resolving coordinates against a shared,
    document-level vertex array *is* the honest cost of that design, and a
    CityJSONSeq feature genuinely does not pay it because it carries its own
    local vertices instead. Neither side is bent to match the other. **But a
    row labelled `full-read` implies parity of work, and here there is none**
    — so a `cityjson`-vs-`cityjsonseq` `full-read` delta must not be read as
    "the same job, one format slower". Part of it is a different job.

14. **Conversion provenance: the chain runs FORWARDS ONLY, and nothing
    derives from CityParquet.** Every measured artefact is derived from the
    published source document by `scripts/readbench_prepare.sh`, in one
    direction:

    ```
    CityGML --citygml-tools 2.5.0 to-cityjson--> CityJSON --cjseq 0.3.1 cat--> CityJSONSeq
                                                                          |--fcb ser -A---------> FlatCityBuf
                                                                          |--cityparquet convert-> CityParquet
    ```

    Each artefact derives from the one before it, and **FlatCityBuf and
    CityParquet derive from the SAME CityJSONSeq bytes** — that is what makes
    their comparison fair. `cityparquet export` could emit the CityJSON
    artefacts and it would be convenient, but **deriving a competitor's input
    from the format under test would favour that format**, so it is never
    done. For the same reason **CityGML is never synthesised**: from a
    CityJSON input the `citygml` artefact is reported as not derivable and
    skipped, because a reverse-converted round-trip artefact is not the source
    data and measuring it would be dishonest.

    **Losslessness is asserted, not assumed.** The prepare script counts
    top-level objects at each hop — CityGML members by a tag-oriented `awk`
    pass, CityJSON via `jq`, CityJSONSeq by counting `CityJSONFeature` lines,
    FCB via `fcb info` — and reports any drift across a conversion, because a
    CityGML row and a CityParquet row are only comparable where the
    conversion between them was lossless. Drift is **reported, not fatal**:
    real CityGML routinely carries ADE content citygml-tools skips, and that
    is evidence for the write-up rather than a reason to abort. **A run whose
    stderr carried a `conversion loss:` warning must have that warning
    reproduced beside any number quoted from it.**

15. **A known INPUT RESTRICTION: CityGML input whose objects lack `gml:id`
    is refused outright.** citygml-tools mints a **fresh random UUID** for any
    top-level object with no `gml:id` — a different one on every run. Two
    consequences, both fatal to a fair measurement:

    - The derived artefacts are **not reproducible**: re-running the chain
      produces different ids for the same objects.
    - The id the coordinator samples out of the derived CityJSONSeq is
      **absent from the `.gml` entirely**, so `citygml`'s `id-lookup` scores a
      **miss** (`result_count = 0`) beside every other format's **hit**. That
      is a *different query*, not a slower one, and nothing downstream catches
      it — the coordinator's cross-format self-consistency check covers
      `attr-filter(object_type)` only, never `id-lookup`.

    `scripts/readbench_prepare.sh` therefore refuses such input in its
    preflight, before anything is written, whenever `citygml` is in the format
    set. Real example, measured 2026-08-16: **Riga's published
    `atgazene_lod2.gml` has 703 top-level objects and 703 of them carry no
    `gml:id`** (identity lives in a `gen:intAttribute` named `OBJECTID`). It
    is kept in the corpus table and annotated rather than deleted — every
    *other* format prepares and measures it fine — and is fetched only by
    `just fetch-data DEST no-citygml`, to be run with an explicit `--formats`
    list that omits `citygml`.

16. **Corpus restrictions — what can be measured is narrower than what can be
    converted.** Three independent filters apply, all recorded per entry in
    `bench/catalogue_benchmark_urls.txt`:

    - **Single-family datasets only.** The coordinator derives every query
      parameter (bbox window, sampled id, attribute predicate) from one
      CityParquet package, and refuses a package listing more than one object
      table (`locate_cityparquet_table`). A dataset spanning two CityGML
      modules therefore cannot be measured at all. Cost, measured 2026-08-16:
      **The Hague tile 01 was excluded for a single `TINRelief` among 844
      Buildings and 1653 BuildingParts** — one terrain object yields a second
      object table (`relief.parquet`) and disqualifies the whole dataset. The
      Hague's terrain-free tiles are not published separately.
    - **CityGML 2.0 only.** The reader supports 2.0; citygml-tools converts
      1.0 happily, so without an explicit version check the whole chain would
      go green around a `.gml` artefact that can never be read. The prepare
      script's preflight refuses a non-2.0 declaration.
    - **Five PLATEAU modules are excluded because their 1st-level types are
      unmapped by this reader**: `dem`, `trk`, `lsld`, `urf` and `ubld`. This
      is a *benchmark* problem, not merely a converter gap: the `citygml`
      runner used to report `count = 0`, exit status 0, in a fraction of a
      real read's time, while every other format's artefact for the same tile
      — produced by citygml-tools, which maps `dem:ReliefFeature` to CityJSON
      `TINRelief` — reported thousands. A silent zero beside everyone else's
      thousands is the worst possible row, so the runner now **refuses** such
      a document outright, naming the offending types. Measured tallies:
      `lsld` 134/134 members unmapped, `trk` 11/11, `urf` 2158/2158, `ubld`
      1 of 2 (the partial case, which used to report a plausible, plausibly
      *wrong* `count = 1`); `dem` is excluded by inference from the reader's
      own type map, never measured (the tile is 599 MB and was never
      downloaded), and no member tally is claimed for it.

    One further entry serves every format **except** `citygml`: PLATEAU's
    `brid` tile, on which this repository's CityGML reader hard-errors over
    cross-building shared geometry. Like Riga (Caveat 15) it is fetched only
    by `--only no-citygml`, because either one would **abort a default-set run
    rather than merely lose a row**.

17. **Two corpus datasets are DEGENERATE for every selectivity-based
    scenario.** Measured 2026-08-16:

    | dataset | top-level objects | why it is in the corpus |
    |---|---:|---|
    | `plateau_yokohama_squr.gml` | **1** | the only Square/plaza dataset in the catalogue; the corpus samples PLATEAU by *module*, not by volume |
    | `plateau_chuo_brid.gml` | **13** | the only Bridge dataset; `no-citygml` set only (see Caveat 16) |

    The squr tile is 5.3 MB because that one plaza is finely triangulated,
    not because it holds many features. On both datasets `bbox-query`,
    `attr-filter` and `id-lookup` match either everything or nothing, so
    **their selectivity ratios carry no information and their timings are
    dominated by fixed open/parse cost**. Read their `count`/`full-read`
    rows; **do not quote their filter rows as selectivity evidence, and do
    not average them into any cross-dataset selectivity figure.**

18. **Disk size ≫ wire size for the archived datasets — the two are different
    numbers and the fetcher records both.** Several corpus entries are
    published as `.zip`/`.gz`, and `scripts/fetch_benchmark.sh` normalises
    them to a plain file on arrival (these archives ship the model beside up
    to 57,150 texture images). The size a benchmark reads is therefore not
    the size that was downloaded:

    | dataset | on the wire | on disk, as measured |
    |---|---:|---:|
    | Estonia national LoD1 canopies | 11 MB (`.zip`) | **323 MB** (`.gml`) |
    | Kuopio LoD2.2 textured | 1.5 GB (`.zip`) | **982 MB** (`building.gml`, the one non-image member) |

    Quote the **disk** figure when relating a dataset's size to a read time
    or a peak-RSS number, and the **wire** figure only when discussing
    download cost. A fetch receipt records both, so "skip if present" means
    "came from the pinned bytes and has not been truncated since" rather than
    "a file with that name exists".

## Environment

**Two halves, and both must be recorded for a run to be reproducible**: the
machine the measurement ran on, and the pinned external converters that
produced the artefacts it measured. Since the conversion chain (Caveat 14)
sits upstream of every row, a run made with a different citygml-tools is a
run against different bytes — not merely a different machine.

### Conversion-chain tools (pinned)

Owned by `scripts/fetch_tools.sh` (`just fetch-tools`), which hardcodes the
version, download URL and archive sha256 rather than resolving "latest", and
retries once before hard-failing on a mismatch. The versions actually used
are written to `bench/tools/tool_versions.txt` on every fetch; the values
below are that file's contents:

```
citygml-tools = citygml-tools 2.5.0     # CityGML -> CityJSON
cjseq         = cjseq 0.3.1             # CityJSON <-> CityJSONSeq
java          = openjdk 21.0.11 (2026-04-21)   # citygml-tools 2.x needs 17+
```

`fcb` (FlatCityBuf serialisation) and `duckdb` are listed with the machine
below, because unlike the two above they are not pinned by a fetch script.

### Machine

Captured 2026-07-08 — **the machine the now-deleted `bench/read_results/*.csv`
were produced on, NOT a machine any current result came from** (see the
results-status banner at the top of this file). Replace this block wholesale
as part of the re-run; the `peak_rss_bytes` unit note above applies to it:

```
uname -a: Darwin F19WYJD2P7 25.5.0 Darwin Kernel Version 25.5.0: Tue Jun  9 22:28:34 PDT 2026; root:xnu-12377.121.10~1/RELEASE_ARM64_T6041 arm64
CPU:      Apple M4 Max
RAM:      38654705664 bytes (36 GiB)
duckdb:   v1.5.3 (Variegata) 14eca11bd9
cargo:    cargo 1.93.1 (083ac5135 2025-12-15)
rustc:    rustc 1.93.1 (01f6ddf75 2026-02-11)
fcb:      fcb 0.7.4
```

The tool versions above were captured on 2026-08-16, i.e. **after** that
machine capture and after every figure this document currently quotes: no
published number here was produced by the pinned chain. That is one more
reason the re-run is mandatory.

`peak_rss_bytes` is in **bytes** on every platform since the
`rss_to_bytes` fix — the macOS numbers recorded above were already bytes
and are unaffected by it.

## Reproduce

This is also the procedure that must be run before any number in this
document may be quoted (see the results-status banner at the top).

```sh
just fetch-tools                     # pinned citygml-tools + cjseq (network, needs java 17+)
just fetch-data                      # the 30-dataset catalogue corpus -> bench/data/benchmark (network, 6.5 GB)
just bench bench/data/benchmark      # FORMAT comparison  -> bench/read_results/ + charts
just ordering-bench bench/data/benchmark   # ORDERING comparison -> bench/ordering_results/ + charts
```

The two runs are deliberately separate and land in separate directories —
see "The two benchmark sets" above for why merging them would answer neither
question. `just bench` removes each `OUT/<name>.csv` before writing it (a
fresh `rm -f` precedes each; it never appends across runs), so a committed
run is always one machine, one sitting, per dataset.

**Clear `bench/data/readbench/` first if it predates commit `fb5e3de`.**
Artefacts built before that commit derive from the wrong stage — for a
`.city.json` input no `.city.jsonl` was cut at all, so its `<name>.jsonl.gz`
is a gzip of the whole CityJSON document (measured: 0.254909 s / 61,192,614 B
against the real seq-gz's 0.092799 s / 1,798,710 B — 2.75x too slow, 34x too
heavy) and its `.fcb`/`.parquet` were serialised from the document rather
than from the seq. The prepare script skips an artefact that already exists,
so those would be reused silently. It does not rely on this paragraph being
read: each dataset's artefacts carry the version of the chain that built them
in `bench/data/readbench/.readbench-chain/<name>`, and a stale or absent
stamp makes `readbench_prepare.sh` REFUSE the dataset, printing the exact
`rm -rf` that clears it (`CHAIN_VERSION` in that script owns the version and
the history of what each one changed).

`fetch-data` defaults to `--only default`, which deliberately **omits** the
two datasets that cannot serve a default-set run — Riga (no `gml:id`,
Caveat 15) and PLATEAU `brid` (Caveat 16). Neither fails gracefully: either
would abort the whole folder loop rather than lose its own row. Measure them
with an explicit format list that omits `citygml`:

```sh
just fetch-data bench/data/benchmark-nocitygml no-citygml
just bench bench/data/benchmark-nocitygml bench/read_results \
    "cityjson,cityjsonseq,flatcitybuf,cityparquet-hilbert"
```

Per-dataset manual invocation (what `just bench` itself calls, one input at a
time — useful when a single dataset needs re-measuring):

```sh
just readbench-prepare <input> bench/data/readbench        # artefacts only, no measurement
cargo run --release -p cityparquet-readbench -- run \
    --input <input> --prepared-dir bench/data/readbench \
    --out bench/read_results/<name>.csv --repeat 7
./scripts/readbench_duckdb.sh bench/data/readbench/<name>.parquet \
    bench/read_results/<name>.csv --numeric-column <col>
```

`just readbench-prepare` takes an optional third argument, a comma-separated
format list, when only some artefacts are wanted (e.g.
`just readbench-prepare <input> bench/data/readbench "cityparquet,flatcitybuf"`);
`duckdb-parquet` is not accepted there, because it has no artefact of its own.

The `readbench_duckdb.sh` step is only needed when the `duckdb-parquet`
baseline is wanted — it is opt-in, and `just bench` appends it **only when
`duckdb-parquet` is explicitly named in `FORMATS`**, never on a bare run.
`--numeric-column` is in turn only needed to enable that baseline's
`attr-stats` row; omit it for datasets with no numeric attribute (e.g.
`lod3_railway.city.json`, where `attr-stats` is skipped for every format —
see `crates/cityparquet-readbench/src/coordinator.rs`'s own
`pick_numeric_attribute`, logged on stderr, never fabricated).

**Continuity with the published figures.** The 11 CityJSONSeq datasets every
number in `bench/CORPUS_REPORT.md` was measured on are now fetched by
`just fetch-seq-data`, into `bench/data/benchmark_seq` — a *different*
directory, because `just bench FOLDER` measures everything under FOLDER.
They are CityJSONSeq only, so they cannot serve the `citygml`/`cityjson` rows
of the format comparison; they remain the ordering benchmark's input and the
link back to the older results.
