# CityParquet read-benchmark methodology

The format family's query definitions live here, and so does the **one
numbered fairness-caveat list for every family** — the format family's own,
the configuration families' and the `bloom` family's. It is single because
`benchviz` renders exactly this list onto the summary page (`prep.read_caveats`
extracts it verbatim, `html.py` prints it under "Measurement caveats"), so a
caveat kept anywhere else never reaches a reader of the figures. The
suite entry points, dataset selection and figure layout are described in
[`../README.md`](../README.md). Run `just bench-prep --families formats`,
`just bench-run --families formats`, then `just bench-summary` from the
monorepo root. The format family measures writes as well as these reads.

Result files must be interpreted with their own query-parameter sidecars and
run provenance. Existing `read_results/` CSVs describe
the datasets and configurations named in those files; they do not establish
measurements for the replacement large 3DBAG dataset. Detailed caveats below
include observations on those datasets and remain qualifications on that
evidence until equivalent checks have been made on a new run.

## Purpose

Compare **read** performance — wall-clock time and memory — of the formats a
3D city model can actually be published in, across six access-pattern
scenarios that mirror how a consumer of that data actually reads it: a full
scan, a metadata-only count, a spatial window query at three selectivities,
an attribute-equality filter, a numeric-attribute aggregate, and a single-id
lookup. The read side is the geometry- and
query-facing half of the CityParquet argument; the write side (encoding
size, write time, row-group pruning) is already covered by `benchmark/formats/README.md`.

## Formats

Eight format tags. The canonical vocabulary, spelling and order are owned by
`Format::ALL` in `benchmark/readbench/src/format.rs` — this table is
a copy of it, and the CSV's `format` column, the `--formats` flag, the
`readbench_prepare.sh` artefact names and the plotter's ordering all use the
same eight strings. They run left-to-right from "what the data ships as
today" through "what we propose" to "a different engine over the same file":

| format tag            | what it is                                                                                                                                                                                                                                                                                         | index available                                                                                                                                      |
| --------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------- |
| `citygml`             | CityGML 2.0 XML (`.gml`) — the format most national datasets are published in, read through **this repository's own reader** (`cityparquet::citygml`). On the current corpus this artefact is **synthesised** from the CityJSON source; see "CityGML synthesis" below                              | **none** — no offsets, no object directory, no spatial or attribute tree; every scenario is a full XML parse and an in-memory filter (see Caveat 12) |
| `cityjson`            | plain, whole-document CityJSON (`.city.json`): one JSON document, one `CityObjects` map, one shared document-level `vertices` array                                                                                                                                                                | **none** — the document must be parsed in one piece before any object is readable, so every scenario is a full parse (see Caveat 13)                 |
| `cityjsonseq`         | CityJSONSeq, one self-contained JSON feature per line, feature-local vertices. Read from the PREPARED `<base>.city.jsonl` — `readbench_prepare.sh` always materialises one (copied from a `.city.jsonl` input, `cjseq cat` from anything else), and the runner refuses a CityGML document outright | **none** — every scenario is a full parse                                                                                                            |
| `cityjsonseq-gz`      | the same stream, `gzip -9`'d                                                                                                                                                                                                                                                                       | **none** — full parse, plus gzip inflate                                                                                                             |
| `flatcitybuf`         | FlatCityBuf, written `fcb ser -A` and NOTHING else — every other index knob at its `fcb ser` default (attribute B+-tree branching factor 256, R-tree node size 16). One configuration, measured once, not a swept axis; see Caveat 33                                                              | R-tree spatial index (**2D only**, see Caveat 4) + B+-tree index over **every** attribute (`-A`)                                                     |
| `cityparquet`         | our CityParquet package, **source row order** (`cityparquet convert --overwrite`)                                                                                                                                                                                                                  | Parquet row-group min/max statistics + column projection                                                                                             |
| `cityparquet-hilbert` | the same package, rows written in Hilbert-curve order (`--ordering hilbert`)                                                                                                                                                                                                                       | the same Parquet statistics, but tighter per-row-group bboxes from spatial clustering                                                                |
| `duckdb-parquet`      | DuckDB (v1.5.x) SQL, `read_parquet()` directly over our `cityparquet` package's own table                                                                                                                                                                                                          | whatever Parquet statistics DuckDB's own scan uses — same file as `cityparquet`, different engine                                                    |

The first four are **unindexed by construction**: a published `.gml`,
`.city.json` or `.city.jsonl` carries no way to answer any question without
reading all of it. That is not an omission in the harness — it is the finding
the benchmark exists to quantify, and the reason a `count` gap grows linearly
with dataset size while CityParquet's stays flat.

## Format and ordering configurations

The suite's format comparison uses one configuration per format family:
`citygml`, `cityjson`, `cityjsonseq`, `flatcitybuf` and
`cityparquet-hilbert`. The last is displayed as **CityParquet** in figures;
Hilbert ordering is an experimental condition, not a different format name.
The harness retains separate internal identifiers for source order
(`cityparquet`) and Hilbert order (`cityparquet-hilbert`). They must not be
merged when interpreting configuration measurements.

`Format::DEFAULT_SET` defines these five reader identifiers. The lower-level
harness also supports `Format::ORDERING_SET` for source-order versus Hilbert
experiments, gzipped CityJSONSeq, and DuckDB over Parquet. These are not extra
series in the default format comparison. The database family compares DuckDB
with cjdb and 3DCityDB separately.

The reader's `duckdb-parquet` identifier means DuckDB reading a CityParquet
package. It does not mean the older writer experiment's `duckdb-copy`, which
uses the community CityJSON extension to encode a different Parquet table and
has separate geometry-coverage qualifications in `README.md`.

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
  — see `benchmark/scripts/readbench_upload.md`. There is no local HTTP server this
  repo spins up for a real run (only test-only in-process servers inside
  `cargo test`, never part of the measured path).
- **Network variance is real and disclosed, not hidden.** Unlike the local,
  same-machine `time_s`/`time_std_s`, an http-transport row's timing
  variance includes real network latency/jitter — the standard deviation
  (`time_std_s`) column now also captures that, not just OS/filesystem-cache
  noise. A committed http-transport run is a snapshot of one network path at
  one time, not a reproducible local benchmark.
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
  formats' _access patterns_ against each other, which is what this
  benchmark is for.
  - `cityparquet`: an `object_store`/`ParquetObjectReader`-based async
    reader (`crates/core/src/query_async.rs`) shares the exact same
    row-group-pruning/projection/predicate logic as the local sync reader
    (`crates/core/src/query.rs`) — same query, same pruning
    decisions, only the I/O source differs. A `CountingObjectStore`
    decorator (`crates/core/src/counting_store.rs`) tallies every
    range request the reader actually makes.
  - `flatcitybuf`: `fcb_core`'s own `HttpFcbReader`
    (`fcb_core::http_reader`) drives its native R-tree/B+-tree indexes over
    HTTP range requests, tallied by a `CountingRangeClient` wrapper
    (`benchmark/readbench/src/formats/flatcitybuf.rs`).
  - `citygml`, `cityjson`, `cityjsonseq`(+gz) — every unindexed format: a
    single **whole-object GET** — exactly 1 request, the whole file's byte
    length, by construction, regardless of scenario (there is no index to
    prune with, so there is nothing smaller to fetch). All four route through
    the same `CountingObjectStore` as `cityparquet`, so the tally is measured
    rather than asserted.
  - `duckdb-parquet` has no HTTP-transport row; it is a local-only SQL
    baseline (`benchmark/scripts/readbench_duckdb.sh`), unaffected by `--transport`.

  This is the benchmark's headline cloud-native argument: CityParquet and
  FlatCityBuf pull kilobytes via a handful of range requests for a selective
  query; CityGML, CityJSON and CityJSONSeq pull the entire file over the
  network every time — and on this corpus "the entire file" runs to 293 MB for
  Zurich's CityJSON, and larger again for its CityGML (`sizes.csv` carries the
  measured per-format bytes; no figure is quoted here).

- **The coordinator's own `QueryParams` derivation stays local regardless of
  `--transport`.** The dataset bbox, the derived attribute predicate, the
  numeric attribute, the sampled ids, and the shared CityObject total are
  always read directly
  from the local `--prepared-dir` (see `benchmark/readbench/src/
coordinator.rs`'s own module doc). This means an http-transport run still
  needs the prepared artefacts present _locally_ too (to derive query
  parameters), in addition to uploaded to the served URL.
- **One untimed `Count` preflight per _resolved_ format also goes over HTTP
  under `--transport http`, not just the timed measurement rows.** For each
  format actually being benchmarked, the coordinator issues one untimed
  `Count` child call to establish that format's own total (used as the
  `bbox-query` selectivity denominator) — under `--transport http` this
  preflight uses the same http `Source` as every other row for that format,
  so it _does_ touch the network, but its own bytes/requests are not folded
  into any CSV row (only the timed rows below it are reported). This is a
  small, fixed amount of extra untimed traffic per format per run (one
  `Count`, the cheapest scenario), disclosed here rather than silently
  absent from the reported totals.

## The corpus

The format and size families use Rotterdam, Ingolstadt, Vienna, New York,
Zurich and the largest 3DBAG scaling slice. Published CityJSON inputs are
listed with their provenance in `corpus_urls.txt`. The suite manifest selects
five of those inputs and replaces the small 3DBAG tile with the scaling
source. The source list remains a download inventory, not the experimental
matrix.

The 3DBAG slices are nested prefixes of a pinned FlatCityBuf source, cut at
whole-feature boundaries. Their names give nominal targets; recorded actual
CityObject counts determine the scaling axis. All families that request the
largest slice must use the same source bytes and derived query parameters.

The corpus is building-focused and does not establish coverage of all CityGML
modules. Ingolstadt provides LoD3 data; 3DBAG provides multiple LoDs. Synthesised
CityGML must be checked for information loss, including collapse of fractional
LoDs (Caveat 14). This limitation also needs checking on the large 3DBAG slice;
a successful conversion alone does not prove equivalent content.

## The six scenarios

Every format implements every scenario via its own natural mechanism —
never a hand-tuned shortcut, never an artificial common code path:

| scenario                         | common target                                                                                                                | `citygml` mechanism                                                                                                                                                                                                     | `cityjson` mechanism                                                                                                                                                 | `cityjsonseq`(+gz) mechanism                                | `flatcitybuf` mechanism                                                                                                                                                                                                  | `cityparquet`(+`-hilbert`) mechanism                                                               | `duckdb-parquet` mechanism                                                                                                                              |
| -------------------------------- | ---------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `full-read`                      | decode every feature's geometry; `(feature_count, boundary_count)`                                                           | stream every `cityObjectMember` (quick-xml), decoding every `gml:pos`/`posList`, resolving every `xlink:href` surface reference and rebuilding a feature-local vertex pool, then walk each geometry's `boundaries` tree | parse the whole document, then **resolve every boundary leaf** through the shared `vertices` + `transform` — _not_ the same operation as `cityjsonseq`'s (Caveat 13) | parse every line, walk each feature's own `boundaries` tree | `select_all` + a RAW `CityFeature` walk (`cur_feature`): every geometry's five flattened index arrays and its semantics indices, every template instance's boundaries, every vertex — no CityJSON conversion (Caveat 32) | scan all row groups, decode WKB                                                                    | `SELECT sum(hash(COLUMNS(*)))` — forces every column decoded                                                                                            |
| `count`                          | total feature/object count                                                                                                   | count `cityObjectMember`s (full parse)                                                                                                                                                                                  | size of the `CityObjects` map (full parse)                                                                                                                           | count parsed lines (full parse)                             | `features_count()` header field (O(1))                                                                                                                                                                                   | Parquet file metadata `num_rows` (O(1), no scan)                                                   | `SELECT count(*)`                                                                                                                                       |
| `bbox-query` (1%/5%/25% of rows) | ids/count of objects whose bbox intersects a query window                                                                    | parse all, test each member's own unioned bbox                                                                                                                                                                          | parse all, test each CityObject's bbox (min/max over the vertices its geometries reference, resolved through `transform`)                                            | parse all, test each feature's own unioned bbox             | `select_query(Query::BBox)` — R-tree, **2D only** (see Caveat 4); the R-tree's hit count is returned and no feature is read or decoded, which is what its index affords                                                  | row-group prune (`with_bbox_row_groups`) + row-level bbox test — **exact**                         | `WHERE bbox.xmax>=.. AND bbox.xmin<=.. AND bbox.ymax>=.. AND bbox.ymin<=..` (full z window, so no z clause needed)                                      |
| `attr-filter`                    | count of objects matching an ATTRIBUTE predicate — `attr == v`, or `attr >= q` (see "Which attribute the predicate runs on") | parse all, test each CityObject's `attributes`                                                                                                                                                                          | parse all, test each CityObject's `attributes`                                                                                                                       | parse all, test each CityObject's `attributes`              | B+-tree attribute index (`select_attr_query`) when the column is in the CityJSON `attributes` map; otherwise a raw `select_all` walk that decodes only that one column (Caveats 11, 19, 24)                              | `RowFilter` (`ArrowPredicateFn`) + row-group statistics prune                                      | `WHERE "<attr>" = '<v>'` / `WHERE "<attr>" >= <q>`, read from the sidecar                                                                               |
| `attr-stats`                     | `(min, max, sum, count)` of a numeric attribute                                                                              | parse all, fold `(min, max, sum, count)` over every numeric value (Caveat 35)                                                                                                                                           | parse all, fold `(min, max, sum, count)` over every numeric value (Caveat 35)                                                                                        | parse all, fold `(min, max, sum, count)` (Caveat 35)        | full walk decoding only that one attribute column, no geometry, folding `(min, max, sum, count)` (no numeric-range index; Caveat 35)                                                                                     | min/max from Parquet column-chunk statistics (near-free); sum/count from a 1-column projected scan | `SELECT min(c), max(c), sum(c), count(c)`                                                                                                               |
| `id-lookup` (x4)                 | the single object with a given id, materialised                                                                              | parse until found (early exit); a miss drains to EOF                                                                                                                                                                    | parse the whole document, then one map lookup                                                                                                                        | parse until found (early exit)                              | the B+-tree is tried and never has the field, so in practice a raw `select_all` walk comparing each CityObject's borrowed `id()`, exiting at the hit (Caveats 19, 24)                                                    | `RowFilter` on `id` + decode of the one surviving row                                              | not run (id lookup is not a distinct DuckDB SQL pattern worth timing separately from `attr-filter`'s `WHERE` plan; the coordinator's own rows carry it) |

`cityparquet` and `cityparquet-hilbert` share one runner and one column here:
a Hilbert-ordered package is still a plain CityParquet package on disk, and
the only thing that differs between the two tags is which artefact path
resolves (`Format::artefact`). That is exactly why the ordering question gets
its own single-axis run rather than an extra column.

The three unindexed formats' `attr-filter`/`attr-stats`/`id-lookup`
mechanisms are not merely _similar_: `citygml` and `cityjson` reuse the
`cityjsonseq` runner's own attribute helpers **verbatim**, so all three agree
on what a column name and an `--attr-eq` predicate mean by construction
rather than by coincidence.

### Which attribute the predicate runs on

`attr-filter` compares **indexed attribute access**, so its predicate must run
on a real CityJSON ATTRIBUTE — a member of a CityObject's `attributes` map.
That is exactly what FlatCityBuf's B+-tree covers (`fcb ser -A` indexes every
attribute), and it is why this scenario used to measure nothing on that
format: it was driven by the reserved `object_type` column, which is
structural and never in the `attributes` map, so every FlatCityBuf row of
every earlier run carried `no-attr-index` and answered by a full walk.

The predicate is derived once per dataset by
`benchmark/readbench/src/params.rs` (`pick_attr_filter`), recorded in the
`<out>.csv.params.json` sidecar as `attr_filter` (column, predicate, matched
count, share), and read from there by `readbench_duckdb.sh` rather than
re-derived. It appears in `notes` as `attr=<column>=<value>` or
`attr=<column>>=<q>`.

**Hand-picked, for the datasets this benchmark actually measures.** These are
the queries a reader of the corpus would recognise, chosen once from a survey
of the prepared packages; the share is of all CityObject rows:

| dataset                         | predicate                     | share |
| ------------------------------- | ----------------------------- | ----- |
| `3dbag_*` (every scaling slice) | `b3_dak_type == "slanted"`    | ~35%  |
| `zurich_building_lod2`          | `class == "BB01"`             | 19.4% |
| `vienna_102081`                 | `roofType == "FLACHDACH"`     | 45.4% |
| `ingolstadt`                    | `klumMaterialClass == "Wood"` | 6.9%  |
| `nyc_da13_buildings`            | `BIN == "1000000"`            | 0.7%  |
| `rotterdam_delfshaven`          | `TerrainHeight >= 2.45`       | 25.4% |

NYC's only categorical attributes are identifiers, so its entry is the
placeholder `BIN` — a legitimate low-selectivity equality. Rotterdam's string
attributes are constant, so a numeric range is the only selective predicate it
has; the bound is the column's own 0.75 quantile (linear interpolation, i.e.
DuckDB's `quantile_cont`), computed from the data at derivation time rather
than written down here.

**Derived, for any other dataset.** The string attribute column with between 2
and 1000 distinct values whose most frequent value's share of rows lies
closest to 0.25 (ties by column name); its predicate is equality with that
value. Failing that, the alphabetically-first numeric attribute, thresholded
at `>=` its own 0.75 quantile. Failing that, `attr-filter` is **skipped** and
the coordinator says so on stderr — the same "never fabricated" treatment
`attr-stats` already gets on a dataset with no numeric attribute.

A hand-picked entry whose column the package does not carry, or whose value
matches no row, falls back to the derived rule with a line on stderr rather
than measuring a query that returns nothing.

`bbox-query` is measured at **three** selectivity targets — windows selecting
~1%, ~5%, and ~25% of the dataset's **rows** — one CSV row per target, tagged
`bbox-1pct`/`bbox-5pct`/`bbox-25pct` in `notes`.

The target is a fraction of rows, not of area. Each window is centred on the
dataset's **median row centre** and its half-extent binary-searched, scaled to
each axis's own span, until the fraction of rows it intersects reaches the
target (`benchmark/readbench/src/params.rs`, `window_for_target`). A window
whose target was not reachable on the data carries `approx` in `notes`
alongside its tag; the achieved fraction is always what the `selectivity`
column records, so target against achieved is checkable per row.

The target is a fraction of rows rather than of area by decision. A fraction
of the objects is comparable across datasets; a fraction of the area is not,
because how many objects an area holds depends on where the data sits in its
own extent. The alternative — a lower-left window covering the target fraction
of the x/y area — was measured and rejected: on the 1M 3DBAG slice the 1 %
area window selected 0.49 % of the objects (and the 5 % and 25 % windows
6.37 % and 22.1 %). `benchmark/databases` builds its windows with the
identical construction (`citybench/config.py`, `citybench/params.py`, ported
from `window_for_target`), so `bbox-1pct` means the same thing in both
families.

`feature-lookup` (CityParquet only, run by name — the `bloom` family) returns
every object of one `feature_id`, probed at `feature-50pct` (the `id-50pct`
feature) and `feature-miss`; it is not one of the six comparison scenarios,
because no other format stores the column.

A seventh scenario, `project` (one attribute column read across every row,
reporting its non-null count), was retired on 2026-09-23. A single-attribute
projection is not a real-world workload, and for every format without a
columnar layout it cost what a full parse costs, so it added a row without
adding a question. The runner rejects the name like any unknown scenario.
Result CSVs written before that date still carry `project` rows; they are not
part of the comparison set and must not be read as one.

## Metrics and the CSV contract

`benchmark/formats/read_results/*.csv`, one row per (dataset, format, scenario
[, selectivity target]):

```
dataset,format,scenario,selectivity,result_count,time_s,time_std_s,peak_heap_bytes,peak_rss_bytes,repeat,notes,bytes_read,http_requests,row_groups_total,bloom_pruned,filter_bytes
```

- `time_s` / `time_std_s` — **warm-cache** arithmetic mean and population
  standard deviation of `repeat` samples (default 7; one further, discarded
  warmup precedes them), 6-decimal precision. The mean is the statistic
  `benchmark/databases` reports too, so a timing quoted from either CSV is the
  same statistic; the standard deviation is the population one because the
  warm repeats are the whole measured set, not a draw used to infer a wider
  one. A fresh child process is spawned per
  sample (see "Warm vs cold" below) — independent OS page-cache and
  independent `peak_alloc` state per sample, never reused across repeats.
- `peak_heap_bytes` — the `peak_alloc` global-allocator high-water mark for
  that one in-process scenario call. **Empty for `duckdb-parquet`**: DuckDB
  runs out-of-process, so there is no allocator hook into it the way the
  `--child` protocol has one into `cityparquet-readbench` itself.
- `peak_rss_bytes` — the child's own peak resident set size
  (`cityparquet`/`cityparquet-hilbert`/`flatcitybuf`/`cityjsonseq`/
  `cityjsonseq-gz`), or a separate untimed `/usr/bin/time -l`/`-v` capture
  around the same query (`duckdb-parquet`). On Linux the child reads
  `VmHWM` from `/proc/self/status`; elsewhere it reads
  `getrusage(RUSAGE_SELF).ru_maxrss`, **normalised to bytes** by
  `rss_to_bytes` in `benchmark/readbench/src/main.rs` (`ru_maxrss` is
  natively KiB on Linux per `getrusage(2)`, bytes on macOS/BSD). The column
  is bytes throughout. **Every CSV committed before the `VmHWM` change is
  floored at the coordinator's own RSS** — see Caveat 34 — so its small
  values are not the child's.
- `selectivity` = `result_count / total_object_count`, empty where N/A
  (`count`, `full-read`). See Caveat 2 for what `total_object_count` means
  per scenario.
- `notes` — a `;`-separated tag list (never a comma: it is one CSV field):
  the `bbox-*pct` selectivity tag, the attribute predicate used for
  `attr-filter` (`attr=<column>=<value>` or `attr=<column>>=<q>`), the
  attribute name for `attr-stats` (`attr=<column>`), the sampled
  id for `id-lookup`, or
  `cold` (always first) for the one cold-cache row — plus any DISCLOSURE the
  run made about that row:
  - `no-attr-index` / `attr-index-failed` — FlatCityBuf answered this row by
    a full scan, not by its B+-tree (Caveat 11);
  - `attr-filter-count-mismatch` — the resolved formats disagreed on
    `attr-filter`'s `result_count`, so this run's object-level rows are not
    all measuring the same query (see "Self-consistency" in
    `benchmark/readbench/src/coordinator.rs`).
- `bytes_read` / `http_requests` — **empty for every `--transport local`
  row** (no HTTP concept locally); for a `--transport http` row, the total
  bytes transferred and HTTP request count that scenario's own
  transport-agnostic reader made (see "HTTP transport" below).
- `row_groups_total` / `bloom_pruned` / `filter_bytes` — **empty on every
  row but a CityParquet `id-lookup` or `feature-lookup`**: the table's row
  groups, those its bloom filters ruled out, and the bitset bytes of every
  filter examined (32 per block; header bytes and transport overhead are not
  counted — `bytes_read` carries the latter over HTTP). The kept row groups are
  read by one reader, as without filters; an `id-lookup` hit stops at its
  first match. Deterministic across repeats; the first warm sample's values
  are recorded.

## Warm vs cold protocol

The **headline number is the warm-cache mean**: `repeat` fresh child
processes (default 7), a further discarded warmup beforehand, OS page cache
and (for the in-process formats) allocator state left however the previous
sample left them — i.e. "warm" describes the OS/filesystem cache, not a
long-lived process, since every sample is already a brand-new process (see
`benchmark/readbench/src/coordinator.rs`'s own module doc on why:
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
   parents _and_ children each get a row — and `cityjson` matches that grain
   (its `CityObjects` map is flat: a `Building` and its `BuildingPart`s are
   sibling entries linked only by `parents`/`children`). `cityjsonseq`(+gz),
   `flatcitybuf` and `citygml` instead count top-level **features/members**
   for `count`/`full-read`/`bbox-query` (a CityJSONSeq/FCB feature bundles
   one top-level CityObject with all its children inline, exactly as a
   CityGML `cityObjectMember` nests its `BuildingPart`s). So the
   `count`/`full-read`/`bbox-query` rows split into **two grains**:

   | grain                                                      | formats                                                            |
   | ---------------------------------------------------------- | ------------------------------------------------------------------ |
   | one row per **CityObject** (children counted separately)   | `cityparquet`, `cityparquet-hilbert`, `cityjson`, `duckdb-parquet` |
   | one row per **top-level feature/member** (children inline) | `citygml`, `cityjsonseq`, `cityjsonseq-gz`, `flatcitybuf`          |

   But `attr-filter`, `attr-stats` and `id-lookup` are
   **CityObject-granular in every format** — `citygml`/`cityjsonseq`/
   `flatcitybuf` deliberately flatten to per-CityObject counting for exactly
   these three scenarios (`flatcitybuf` because that is what its B+-tree
   attribute index naturally returns per entry: `fcb_core` indexes every
   value of a feature's `city_objects` map, not only the root object, so on
   Vienna an indexed `attr-filter` returns 600 entries from 307 features,
   and the raw-accessor walks of Caveat 32 count the same way; the two
   parsing formats by explicit choice, to match) — so these three are directly, honestly
   comparable across every format; `count`/`full-read`/`bbox-query` are not.
   Empirically: `lod3_railway.city.json` is 121 CityObjects / 38 top-level
   features; `delft.city.jsonl`'s `object_type == "BuildingPart"` count is
   1116 CityObjects (out of 2231 total CityObjects / 1115 features);
   `railway_lod3_fragment.gml` is 6 CityObjects / 4 members. Each of those is
   asserted in the runners' own tests, not merely claimed here.

2. **Selectivity's denominator differs by scenario, on purpose.** The three
   CityObject-granular scenarios (`attr-filter`/`attr-stats`/`id-lookup`)
   divide by the **dataset-global CityObject total** — the
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
   compares their `result_count`s — the derived attribute predicate is
   CityObject-level in every format, so a healthy run sees them agree exactly
   — and prints either `self-consistency OK: …` or `WARNING: formats disagree
on AttrFilter(attr=<column>=<value>) result_count: …` on **stderr**, naming
   the predicate that was measured. It is a diagnostic,
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
   the write benchmark's baseline uses). Each is that format's own honest
   full-read cost — reported per-format, never normalised into a shared
   unit of work that doesn't actually exist across six different
   encodings. Where two of them are _labelled_ the same but are not the same
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

5. **`duckdb-parquet` reads OUR CityParquet package directly — the write-side
   geometry-coverage caveats do NOT carry over.** Unlike
   `benchmark/formats/README.md`'s `duckdb-copy`/`duckdb-copy-zstd` rows (which go
   through the community `cityjson` extension's `read_cityjson`/
   `read_cityjsonseq`, documented there to write 0% `geom_lod0` coverage
   everywhere and 0% of _everything_ on `lod3_railway.city.json`),
   `duckdb-parquet` here runs `read_parquet()` straight over a
   `cityparquet-rs`-written package — full geometry, every LoD column.
   **However**: our packages' WKB geometry columns carry GeoParquet "geo"
   file metadata, and DuckDB's spatial extension (autoloaded) eagerly
   tries to decode them into its own native `GEOMETRY` type the instant a
   query references the column — even a bare `SELECT` with no function
   applied — and that native decode does not support the multi-surface/
   solid WKB shapes our LoD1.2/LoD1.3/LoD2 geometries use, failing with
   `Invalid Input Error: Unsupported geometry type in WKB`.
   `benchmark/scripts/readbench_duckdb.sh` sets `enable_geoparquet_conversion=false`
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
   extension loading). `benchmark/scripts/readbench_duckdb.sh` measures this via 5
   timed `SELECT 1;` calls and prints it as a `# calibration:` stderr line
   before every run — it is **disclosed, never subtracted**, from any
   reported `time_s`/`time_std_s`.

7. **Warm vs cold — never silently mixed.** The headline numbers everywhere
   in this document and in `benchmark/formats/read_results/*.csv` are warm-cache
   means; the single `cold`-tagged row per format (see "Warm vs cold"
   above) is a distinct, separately-reported measurement, always
   `full-read` only, never averaged into or compared unlabelled against the
   warm rows.

8. **Sub-millisecond deltas are noise; single-threaded reads are pinned.**
   As in `benchmark/formats/README.md`'s own methodology, deltas under roughly 10 ms at
   `repeat = 7` are within scheduler/filesystem-cache noise and are not
   cited as a finding by themselves. Every format's reads here run
   single-threaded (no Parquet multi-threaded row-group decode, no
   DuckDB multi-threaded query execution) — a deliberate, disclosed choice
   so timing differences reflect the format/mechanism, not thread-count
   parallelism a production deployment might or might not enable.

9. **`id-lookup` is FOUR rows per format, and the three hit rows are not
   comparable across formats as raw times.** A single target made the
   published number a function of where that one id happened to sit in the
   file — a property of the sample, not of the format. Each format is now
   measured against four targets, tagged in `notes`:

   | tag        | target                                 |
   | ---------- | -------------------------------------- |
   | `id-10pct` | the id at 10% of the canonical order   |
   | `id-50pct` | the id at 50%                          |
   | `id-90pct` | the id at 90%                          |
   | `id-miss`  | an id verified absent from the dataset |

   The **canonical order is the CityJSONSeq stream**, because
   `readbench_prepare.sh` cuts the gzipped, FlatCityBuf and CityParquet
   artefacts from that one file. Each probe is verified present in both the
   CityParquet table and the CityGML artefact before it is used; a probe that
   fails verification is replaced by the nearest feature that passes and the
   row carries `id-substituted`.

   **Every format now takes the best mechanism its encoding affords**, which
   is the change that makes the column mean one thing:

   - `cityjsonseq`(+gz) and `citygml` stop at the hit. CityGML did not
     before, because its skipped-member guard (Caveat 12) is only
     authoritative at EOF; that guard now rides on the **untimed `count`
     pass** the coordinator runs per format per dataset before any scenario,
     whose failure aborts the run, so a document with unmapped members still
     cannot reach a published row. An `id-lookup` **miss** drains by
     necessity and still consults the guard directly.
   - `cityjson` cannot exit early in principle: a whole-document CityJSON
     must be parsed in one piece before any object is addressable, so its
     four rows are near-identical and position-independent.
   - `flatcitybuf` tries its B+-tree first and falls back to a full walk;
     see the new Caveat 19 for why the fallback is structural.
   - `cityparquet` applies `id` as a `RowFilter`, which evaluates the whole
     id column regardless of position.

   **`id-miss` is the row to compare across formats.** It is position-free,
   and it is what actually separates a format carrying an id index from one
   that does not.

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
    reading the artefact. That fallback is a raw-flatbuffer walk, not a
    CityJSON conversion — see Caveat 32. The `cityjsonseq-gz` runner fully supports
    CityJSONSeq and single-line whole-document `.city.json.gz` (the form the
    committed fixtures use); a _pretty-printed_ multi-line whole-document
    `.city.json.gz` is not yet handled (it needs the fuller sniff
    `cityparquet::source::Source::open` already implements). External review
    (Codex, 2026-07-08) confirmed the query primitives, bbox prune + row-level
    filter, and allocator placement correct; its two flagged "dictionary"
    criticals were verified FALSE POSITIVES — `TypedDictionaryArray::value(i)`
    resolves the row's key, and the then-committed `attr-filter(object_type)` run
    over 2231 rows with ~4 distinct types would have panicked at row 4 had the
    alleged raw-index reading been real.

12. **`citygml` measures THIS REPOSITORY'S reader, not CityGML's ceiling.**
    This is the single most important caveat on the `citygml` row, and it cuts
    both ways.

    The row answers exactly one question: _what does it cost to answer this
    query against the format the data actually ships in, using the same
    codebase as every other row?_ It is **not** a claim about what CityGML
    could achieve in principle. A different parser — a streaming SAX filter
    tuned to one query, an XML database with a pre-built index, a commercial
    CityGML engine — would give different numbers, and nothing measured here
    bounds them. What _is_ structural rather than implementation-specific is
    the absence of an index: a published `.gml` carries no offsets, no object
    directory and no spatial or attribute tree, so _any_ reader must traverse
    the document to answer any of the six scenarios. The constant factor is
    ours; the linear term is the format's.

    Two further disclosures about that row, neither folded silently into it:

    - **The appearance pre-pass is skipped.** `FeatureReader::open` re-reads
      the whole document up front to index CityModel-level appearance; not one
      of the six scenarios consults appearance, so the runner uses
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

    That extra work is measured, not assumed. _Within the `cityjson` runner_,
    on the same fixture and machine, `full-read` costs roughly a fifth more
    elapsed time than `count` in release mode — median of 9 runs, 0.199 s for
    `count` against 0.243 s for `full-read` — so the leaf resolution is real
    work rather than something the optimiser elides, and
    `std::hint::black_box` pins that rather than trusting it to stay true.
    (That ~20% is a `cityjson`-internal figure, **not** the
    `cityjson`-vs-`cityjsonseq` gap; the cross-format gap also carries the
    whole-document-vs-line-oriented parse difference on top of it.)

    This is **defensible**: resolving coordinates against a shared,
    document-level vertex array _is_ the honest cost of that design, and a
    CityJSONSeq feature genuinely does not pay it because it carries its own
    local vertices instead. Neither side is bent to match the other. **But a
    row labelled `full-read` implies parity of work, and here there is none**
    — so a `cityjson`-vs-`cityjsonseq` `full-read` delta must not be read as
    "the same job, one format slower". Part of it is a different job.

14. **Conversion provenance: the chain runs FORWARDS ONLY, and nothing
    derives from CityParquet.** Every measured artefact is derived from the
    published source document by `benchmark/scripts/readbench_prepare.sh`, in one
    direction:

    ```
    CityGML --citygml-tools 2.5.0 to-cityjson--> CityJSON --cjseq 0.3.1 cat--> CityJSONSeq
                                                     |                    |--fcb ser -A---------> FlatCityBuf
                                                     |                    |--cityparquet convert-> CityParquet
                                                     |
                                                     |--citygml-tools from-cityjson -v 2.0--> CityGML
                                                        (only when the source is not itself CityGML)
    ```

    Each artefact derives from the one before it, and **FlatCityBuf and
    CityParquet derive from the SAME CityJSONSeq bytes** — that is what makes
    their comparison fair. `cityparquet export` could emit the CityJSON
    artefacts and it would be convenient, but **deriving a competitor's input
    from the format under test would favour that format**, so it is never
    done. **CityGML is the one artefact derived backwards**, and the next
    caveat is entirely about what that costs.

    **CityGML synthesis — the one backwards hop, and its cost.** Where the
    source document is not itself CityGML, the `citygml` artefact is produced
    by `citygml-tools from-cityjson -v 2.0 --no-pretty-print` from the
    CityJSON stage. This REVERSES an earlier rule of this benchmark, which
    reported such an artefact as "not derivable" and skipped it on the grounds
    that a round-trip product is not the source data. Three things about that
    reversal, in the order a sceptical reader will raise them:

    - **Why it changed.** This benchmark's claim is a comparison BETWEEN
      formats, so a dataset that produces seven artefacts and skips the eighth
      does not weaken the comparison, it removes the baseline from it. Under
      the retired corpus the skipped ones were precisely the datasets a reader
      recognises — 3DBAG, Rotterdam, Vienna, NYC and Zurich all ship as
      CityJSON. Nor is the published `.gml` beside a `.city.json` usually a
      way out: of the nine on cityjson.org, six are CityGML **1.0** and two
      are **3.0**, and this reader accepts only 2.0 (verified 2026-08-23; the
      per-file versions are tabulated in `benchmark/formats/corpus_urls.txt`).
    - **What it costs.** The `citygml` row measures **citygml-tools'
      serialisation**, not a published file. State this beside any CityGML
      number quoted from this corpus. Two things bound the cost. First, size:
      on Rotterdam the synthesised document is **14.0 MB** against a published
      original of **16.5 MB** — the same magnitude, so a "CityGML is bulky"
      finding is not an artefact of the synthesis. Second, formatting:
      `--no-pretty-print` is a measurement decision, not a tidiness one. The
      same content serialises to **18.8 MB** indented and **14.0 MB** compact,
      so indentation alone would move the row by a third. Compact is the
      conservative choice — it gives the baseline this benchmark argues
      against its **best** case, so no size or parse-time gap can be dismissed
      as whitespace.
    - **What it cannot preserve.** CityGML 2.0 has only integer LoDs, so a
      source carrying fractional ones loses some. Measured 2026-08-23 on
      `3dbag_9-284-556`, whose CityJSON holds LoD 0, 1.2, 1.3 and 2.2: the
      synthesised `.gml` collapses 1.2 and 1.3 into a single `lod1Solid` and
      carries **three** LoDs where every other artefact carries four
      (`lod0FootPrint` 1,110 / `lod1Solid` 1,111 / `lod2Solid` 1,111 /
      `lod2MultiSurface` 4,031). **That dataset's `citygml` row is therefore
      not content-equivalent to its other seven, and its bytes and parse time
      must not be quoted against another format's without saying so.** It is
      the only corpus entry affected, it is kept deliberately — it is also the
      only entry exercising the per-LoD `geometry_lod*` columns — and the
      collapse is arguably a finding about CityGML rather than a defect in the
      measurement.

    The synthesised artefact is verified after it is written, not trusted:
    `readbench_prepare.sh` re-reads it for a 2.0 declaration and for a
    `gml:id` on every top-level member (Caveat 15's check, on the other side
    of the conversion), and refuses it on either failure.

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

    No drift check runs across the **synthesis** hop, and running one would be
    wrong: CityGML nests a `BuildingPart` inside its parent `Building` where
    CityJSON lists both at top level, so the counts legitimately differ — on
    `3dbag_9-284-556`, 1,110 top-level GML members against 2,221 CityObjects.
    Comparing them would report a 50 % "loss" that did not happen.

15. **A known INPUT RESTRICTION: CityGML input whose objects lack `gml:id`
    is refused outright.** citygml-tools mints a **fresh random UUID** for any
    top-level object with no `gml:id` — a different one on every run. Two
    consequences, both fatal to a fair measurement:

    - The derived artefacts are **not reproducible**: re-running the chain
      produces different ids for the same objects.
    - The id the coordinator samples out of the derived CityJSONSeq is
      **absent from the `.gml` entirely**, so `citygml`'s `id-lookup` scores a
      **miss** (`result_count = 0`) beside every other format's **hit**. That
      is a _different query_, not a slower one, and nothing downstream catches
      it — the coordinator's cross-format self-consistency check covers
      `attr-filter` only, never `id-lookup`.

    `benchmark/scripts/readbench_prepare.sh` therefore refuses such a document whenever
    `citygml` is in the format set — in its preflight for a CityGML input,
    before anything is written, and immediately after writing for a
    synthesised one (Caveat 14). Real example, measured 2026-08-16: **Riga's
    published `atgazene_lod2.gml` has 703 top-level objects and 703 of them
    carry no `gml:id`** (identity lives in a `gen:intAttribute` named
    `OBJECTID`).

    **No entry of the current corpus trips this check**, and none can trip it
    accidentally: every source is CityJSON, where an object's key is what
    becomes the `gml:id`. The check is retained and exercised by
    `scripts/tests/readbench_prepare_test.sh` precisely because it is not
    expected to fire — an unexercised guard is one that stops working at the
    next citygml-tools upgrade without anyone noticing.

16. **Corpus restrictions — what can be measured is narrower than what can be
    converted.** The filters below are recorded per entry in
    `benchmark/formats/corpus_urls.txt`, including for the two cityjson.org datasets they
    exclude:

    - **Single-family datasets only.** The coordinator derives every query
      parameter (bbox window, sampled id, attribute predicate) from one
      CityParquet package, and refuses a package listing more than one object
      table (`locate_cityparquet_table`). A dataset spanning two CityGML
      modules therefore cannot be measured at all. Cost, re-verified
      2026-08-23 on the current corpus's own source page: **Den Haag tile 01
      is excluded for a single `TINRelief` among 844 Buildings and 1,653
      BuildingParts** — one terrain object yields a second object table
      (`relief.parquet`) and disqualifies the whole dataset — and
      **`LoD3_Railway` is excluded for spanning fourteen types across ten
      modules**. The Hague's terrain-free tiles are not published separately.
      Both would otherwise be good fits, and both would return the moment the
      coordinator learns to pick a table from a multi-table package.
    - **CityGML 2.0 only.** The reader supports 2.0; citygml-tools reads 1.0
      and writes 3.0 by default, so without an explicit version check the
      chain would go green around a `.gml` artefact that can never be read.
      The prepare script refuses a non-2.0 declaration — in its preflight for
      a CityGML input, and after writing for a synthesised one. **This filter
      is the reason the corpus is fetched as CityJSON at all**: every CityGML
      file cityjson.org publishes beside it is 1.0 or 3.0 (Caveat 14).
    - **Unmapped 1st-level types are refused, not counted as zero.** No entry
      of the current corpus is affected (all six are Building module), so the
      evidence below is from the retired catalogue corpus, where five PLATEAU
      modules were excluded on these grounds: `dem`, `trk`, `lsld`, `urf` and
      `ubld`. It is kept because the runner behaviour it describes is live.
      This
      is a _benchmark_ problem, not merely a converter gap: the `citygml`
      runner used to report `count = 0`, exit status 0, in a fraction of a
      real read's time, while every other format's artefact for the same tile
      — produced by citygml-tools, which maps `dem:ReliefFeature` to CityJSON
      `TINRelief` — reported thousands. A silent zero beside everyone else's
      thousands is the worst possible row, so the runner now **refuses** such
      a document outright, naming the offending types. Measured tallies:
      `lsld` 134/134 members unmapped, `trk` 11/11, `urf` 2158/2158, `ubld`
      1 of 2 (the partial case, which used to report a plausible, plausibly
      _wrong_ `count = 1`); `dem` is excluded by inference from the reader's
      own type map, never measured (the tile is 599 MB and was never
      downloaded), and no member tally is claimed for it.

    **Every entry of the current corpus serves every format**, which is the
    property it was selected for and what `scripts/tests/fetch_benchmark_test.sh`
    asserts about the pinned table. The retired corpus had two entries that did
    not — Riga (Caveat 15) and PLATEAU's `brid` tile — and both had to be
    fetched by `--only no-citygml`, because either would **abort a default-set
    run rather than merely lose a row** (`just bench`'s folder loop runs under
    `set -e`). That escape hatch still exists for `$CORPUS_MANIFEST` inputs.

    The `brid` tile reads now, and converts to a single-table package: a
    `brid:Bridge`'s `lod2Solid` composes its faces by `xlink:href` from
    polygons defined in the Bridge's own `brid:boundedBy` semantic surfaces,
    and the generic reader harvests those as xlink targets
    (`citygml_generic_object_boundedby.rs`). Nothing in this caveat rules it
    out any more; whether to restore it to the corpus is a corpus-selection
    question, and it is not measured until it is.

17. **The `attr-stats` scenario is not uniform across the corpus.** Measured
    2026-08-23. `nyc_da13_buildings` (23,777 objects) carries **no numeric
    attribute at all**, so it has no `attr-stats` row. `ingolstadt`'s
    `measuredHeight` covers only **55 of its 379** objects (the `Building`s,
    not the 323 `BuildingInstallation`s), so its `attr-stats` row aggregates a
    minority of rows rather than the dataset. The other four are clean:
    `TerrainHeight` on all 853 of Rotterdam, `measuredHeight` on 1,102 of
    Vienna, `b3_h_dak_50p` on all 1,110 3DBAG parents, `Geomtype` on 145,862
    of Zurich.

    `just bench` detects the column per dataset and omits the row where there
    is none, so an absent `attr-stats` row is expected rather than a failed
    measurement. **Do not read a missing row as a zero, and do not average
    Ingolstadt's into a cross-dataset aggregate figure.** Every other
    scenario is unaffected: they derive from geometry, object type or id,
    which all six datasets have.

    The retired corpus had a different version of this problem — two datasets
    with 1 and 13 top-level objects, whose every filter matched either
    everything or nothing — and the current corpus has none: its smallest
    entry holds 379 objects and its largest 198,699, so every selectivity
    ratio carries information.

18. **Wire size equals disk size on this corpus, but the SOURCE size is not
    the size of what gets measured.** Every current entry is served
    uncompressed, so the pinned wire bytes are also the bytes on disk — unlike
    the retired corpus, where `.zip`/`.gz` entries differed by up to 30x
    (Estonia's 11 MB archive expanded to 323 MB) and the disk figure was the
    one to quote. That distinction no longer applies here.

    What does apply: **a dataset's pinned size is its CityJSON size, and seven
    of the eight measured artefacts are not that file.** The synthesised
    CityGML is several times larger (Rotterdam: 2.7 MB CityJSON, 14.0 MB
    CityGML), the CityParquet package is smaller, and so on. When relating "a
    dataset's size" to a read time or a peak-RSS number, use the per-format
    bytes in `sizes.csv`, never the corpus table's wire figure — that one
    answers download cost for the source only.

19. **FlatCityBuf's `id` is structurally unindexable, so its `id-lookup` is a
    full walk.** The `.fcb` artefacts are built `fcb ser -A`, which indexes
    **every attribute**, and the runner does try the B+-tree first. It always
    falls back: `id` is a CityObject's map KEY, never a member of the
    CityJSON `attributes` map FCB's schema covers, so there is no entry to
    find. The `no-attr-index` tag on those rows records a property of the
    format as it stands, not a gap in this harness. `attr-filter` uses the
    index whenever its predicate column IS a member of that `attributes`
    map, and falls back to the same full walk — carrying the same
    `no-attr-index` tag — whenever it is not, which is the case for the
    reserved `object_type`. The tag in the CSV, not the scenario name, says
    which of the two a given row measured.

20. **A probe's POSITION is only nominal outside the CityJSONSeq stream.**
    The deciles are cut from the seq order, and the gzipped, FlatCityBuf and
    CityParquet artefacts are cut from that same file — but neither the
    CityGML nor the FlatCityBuf artefact preserves it. The CityGML is
    synthesised independently by `citygml-tools`; the `.fcb` is ordered by
    its own R-tree. Both therefore show **non-monotonic** times across
    `id-10pct`/`id-50pct`/`id-90pct`, and on `ingolstadt` FlatCityBuf's
    `id-90pct` is roughly 40x faster than its `id-10pct`. Presence is
    verified for every probe; position is not. Read the three hit rows for
    those two formats as three samples of the distribution, never as a
    position curve.

21. **A bbox row's target and its achieved selectivity can differ, and
    `approx` says where.** Row counts are discrete, so a target is not always
    reachable: 1% of `ingolstadt`'s 379 rows is 3.79 rows. The search accepts
    a relative tolerance of ±10% and, when it cannot converge inside that,
    takes the nearest achievable window and appends `approx` to the row's
    `notes`. A missed target is disclosed in the artefact, never silent.

22. **bbox targets are expressed in CityParquet ROW space.** The search runs
    over the CityParquet package's per-row bboxes, so `cityparquet`'s own
    achieved selectivity is the one that lands on the target. Feature-grained
    formats (`citygml`, `cityjsonseq`, `flatcitybuf`) count a different unit
    and therefore report a different achieved fraction for the **identical**
    window — on `ingolstadt`, 0.055 against CityParquet's 0.011 for
    `bbox-1pct`. The window is the same for every format, which is what makes
    the timings comparable; the `selectivity` column is not comparable across
    grains. See Caveat 3 on counting grain.

23. **The scaling corpus's format set is NOT uniform across its seven
    cardinalities.** The scaling slices measure 1k, 5k, 10k, 50k and
    100k CityObjects with all five formats, but 500k and 1M with **four** —
    `cityjson`, `cityjsonseq`, `flatcitybuf`, `cityparquet-hilbert`. There is
    no `citygml` row at the two largest sizes.

    The reason is the artefact, not the reader. These slices carry no `.gml`
    of their own, so `readbench_prepare.sh` synthesises one with
    `citygml-tools`, and the synthesised file is roughly 4x the CityJSONSeq
    it came from. On the 1M slice — a 2.75 GB stream — that pass ran for over
    four hours at 30 GB resident and had written 6.6 GB of an estimated 10 GB
    when it was abandoned. The two large cardinalities are therefore measured
    without it deliberately.

    **What this forbids:** reading a `citygml` scaling curve across the whole
    axis, or comparing a 1M-row figure against a `citygml` number that does
    not exist. What it still supports: the four-format curve across all seven
    cardinalities, and the five-format comparison up to 100k. CityGML's
    cross-format cost is measured properly in `read_results/`, on six real
    published datasets, which is what that family is for.

24. **The `bloom` family's larger slices show pruning across many row
    groups.** At the default 65 536-row groups the small slices are one row
    group, which a filter can still prune on a miss but which says nothing
    about how pruning scales; the slices from `3dbag_n100000` upward, at two
    or more groups, are the informative ones.

25. **In the `bloom` family, a filter's positive is not a match.** At FPP 0.01
    a miss can still keep a row group; `row_groups_total − bloom_pruned` on
    the `*-miss` rows is how many the reader still scanned.

26. **The `bloom` family reads the footer twice per lookup** — once for the
    decode metadata, once inside the lookup — equally for both variants.

27. **The `bloom` family measures single-table packages only.** The runner
    queries one object table; a multi-table corpus package is refused, never
    partially read.

28. **In the `bloom` family, `feature-50pct` is the `id-50pct` feature**, and
    `feature-miss` the same verified-absent string as `id-miss` (absent from
    both columns).

29. **The `bloom` family's requests are logical.** Over `--transport http`,
    `CountingObjectStore` counts the requests the reader made after
    object_store coalesced nearby ranges — not raw wire traffic, retries or
    connection reuse.

30. **The committed codec and row-group CSVs predate bloom filters.** Every
    package they measured, the `cityparquet` baseline included, carries none,
    and their `id-50pct` rows read the `id` column without bloom pruning. The
    current writer puts filters on `id`, `feature_id` and high-cardinality
    string attributes by default, so its packages are larger and its lookups
    prune: re-run both families before comparing them with the `bloom`
    family, or with each other across that change.

31. **Only the codec and row-group CSVs predate bloom filters now.** The
    `formats`, `sizes` and `bloom` evidence under `benchmark/runs/formats/`
    was measured on 23–24 September 2026 on packages the bloom-enabled
    writer produced (chain version 3, `MACHINE.md` beside the results), in
    the 16-column shape with the three lookup counters. The legacy
    `read_results/` directory and the `scaling_codec_results` /
    `scaling_rowgroup_results` directories still describe packages with no
    filters; their `LEGACY.md` says so, and an `id-lookup` row from them must
    not be compared with one from the current evidence or with the `bloom`
    family.

32. **FlatCityBuf is read through the raw FlatBuffers accessors, not
    `cur_cj_feature`.** Every FCB walk — `full-read`, the `attr-filter`
    fallback, `id-lookup`, `attr-stats` — reads
    `FeatureIter::cur_feature()`, the zero-copy `CityFeature` table, and
    touches only what the scenario's answer needs: one flatbuffer enum for
    `object_type`, one borrowed `&str` for an id, one targeted decode of a
    single column out of the packed attribute blob, and — for `full-read`
    alone — every geometry's flattened `solids`/`shells`/`surfaces`/
    `strings`/`boundaries` arrays, its per-surface `semantics` indices,
    every template instance's boundaries and every vertex. Not read by
    `full-read`: `semantics_objects`, `material`, `texture` and the feature
    `appearance`.

    These rows used to call `cur_cj_feature()`, which runs `fcb_core`'s
    `to_cj_feature`: a whole CityJSON feature — nested `serde_json` boundary
    arrays, every vertex converted, every attribute into a
    `serde_json::Map`, a `String` id per CityObject — built to answer a
    question needing one comparison. That made FCB's attribute/id rows
    cost the same as its `full-read` row, and measured the conversion rather
    than the format. Measured on this machine, 3 repeats, medians, identical
    `result_count` in every pair: on `3dbag_n10000` `full-read` 0.404 s ->
    0.037 s, `attr-filter` on `object_type` 0.405 s -> 0.034 s, `id-miss`
    0.407 s -> 0.035 s, `attr-stats` 0.404 s -> 0.045 s; on
    `zurich_building_lod2` the same four rows 2.22/2.05/2.22/2.22 s ->
    0.55/0.50/0.50/0.51 s. Peak heap falls with
    them (1.3 MB -> 0.3 MB on zurich). The INDEXED paths
    (`select_attr_query`, `select_query`) were already native and are
    unchanged — `b3_dak_type == slanted` on `3dbag_n10000` sits at ~1 ms
    either way.

    FlatCityBuf is a comparison baseline here, not this repository's
    subject, so it is owed its best natural implementation. Published
    FlatCityBuf figures produced before this change overstate its
    attribute-, id- and full-read costs and must not be mixed with figures
    produced after it.

33. **One FlatCityBuf configuration, chosen by measurement.** The `.fcb`
    artefacts are written `fcb ser -A` with every other index knob left at
    its default — attribute B+-tree branching factor 256, R-tree node size 16. That is a decision, not an oversight, and it is not a swept axis:
    the paper's configuration axes are CityParquet's own.

    `3dbag_n100000` was written at attribute branching factors 16, 64, 128
    and 256 and read back with the benchmark's own child (3 repeats,
    medians): `count` 0.1 ms, `bbox-1pct` 0.3 ms, `bbox-5pct` 1.0 ms,
    `bbox-25pct` 3.4-4.0 ms, the indexed `attr-filter` (`b3_dak_type ==
slanted`) 5.2-5.4 ms and the `id-lookup` miss 0.31-0.32 s — every
    scenario within noise of every other factor, with identical
    `result_count`s and file sizes inside 0.5%. `zurich_building_lod2`
    agrees at 16/128/256, including on an indexed `attr-filter` that really
    matches (`GebaeudeStatus` in [1, 1], 52 834 rows): 0.605/0.605/0.607 s
    over 7 interleaved repeats. No factor wins, so the default stands.

    `--index-node-size` must be left alone for a different reason: it is
    **unreadable**. `fcb_core` 0.7.6's `FcbReader::select_query` — the
    seekable path this benchmark reads through — passes
    `PackedRTree::DEFAULT_NODE_SIZE` where its streaming sibling
    `select_query_seq` passes the header's own `index_node_size`, so a file
    written `fcb ser -A --index-node-size 64` panics with a capacity
    overflow on every `bbox-query`. Reproduced here, not inferred.

34. **`peak_rss_bytes` in every CSV committed before 2026-09-22 is floored
    at the coordinator's own RSS, and the floor hides every format that
    used less.** The child used to report `getrusage(RUSAGE_SELF).ru_maxrss`.
    Linux's `exec_mmap` folds the high-water mark of the address space a
    task had BEFORE `exec` into `signal->maxrss`, and under
    `posix_spawn`/`vfork` that address space is the parent's, so a freshly
    exec'd child can never report less than the coordinator's peak. On the
    committed 1M 3DBAG run 36 of the 43 read rows — every `citygml`,
    `cityjson`, `flatcitybuf` and `cityparquet-hilbert` row — carry the
    identical value 269 963 264 B, the coordinator's RSS after deriving the
    query parameters from the package; only `cityjsonseq` (4.3 GB) rose
    above it. On Zurich 35 rows share 54 816 768 B. Those numbers are the
    coordinator's memory, not the format's, and any read-memory ratio
    computed from them (the `rss_b` heatmap panel) is a ratio of floors.
    The write rows had the same defect through Python's `os.wait4` (the
    constant 14 680 064 B on every `cityjsonseq` write row was the launcher).

    Fixed by reading `VmHWM` from `/proc/self/status` in the child (its own
    `mm`, created by `exec`, is not inherited; measured 10.5 MB for a child
    under a 420 MB parent, against 419 MB from `ru_maxrss`) and by running
    every write converter under `/usr/bin/time -f %M` (floor ≈ 1 MB). The
    fix changes no timing. **Read-memory figures from before and after this
    change must not be mixed**, and the pre-change CSVs' `peak_rss_bytes`
    must not be quoted for any format whose value equals the run's floor.

35. **Before 2026-09-23, only CityParquet's `attr-stats` computed the
    aggregate.** The scenario's contract is `(min, max, sum, count)` of a
    numeric attribute, but the `citygml`, `cityjson`, `cityjsonseq`(+gz) and
    `flatcitybuf` runners only COUNTED the CityObjects carrying a numeric
    value: the same `result_count`, a cheaper question. Every runner now
    folds each value into all four aggregates — `as_f64()` of the JSON (or
    FCB-decoded) value, integers and floats alike, `f64` `min`/`max`/`+=` —
    and pins them with `std::hint::black_box`, so the optimiser cannot reduce
    the arithmetic to the count. CityParquet is unchanged: min and max from
    column-chunk statistics, sum and count from a one-column projected scan,
    which is the format's own mechanism. The child reports the four values on
    stderr (`cityparquet-readbench: attr-stats <min> <max> <sum> <count>`,
    after its timed line; the CSV is unchanged), and
    `tests/attr_consistency.rs` holds every format to DuckDB's
    `min/max/sum/count` on `delft.city.jsonl` over two integer and two float
    columns — they agree exactly, not merely within the test's 1e-6
    (CityParquet's statistics-derived minimum of `b3_bag_bag_overlap` prints
    as `-0` where the folding runners print `0`: Parquet writes a zero
    minimum as `-0.0`, and the two are equal under IEEE comparison). The
    `result_count`s did not change. **`attr-stats` timings of the
    non-CityParquet formats from before and after this change must not be
    mixed**; the added work is one comparison pair and one addition per
    value, next to a full parse, so the difference is expected to be small
    but it has not been measured.

## Environment

**Two halves, and both must be recorded for a run to be reproducible**: the
machine the measurement ran on, and the pinned external converters that
produced the artefacts it measured. Since the conversion chain (Caveat 14)
sits upstream of every row, a run made with a different citygml-tools is a
run against different bytes — not merely a different machine.

### Conversion-chain tools (pinned)

Owned by `benchmark/scripts/fetch_tools.sh` (`just fetch-tools`), which hardcodes the
version, download URL and archive sha256 rather than resolving "latest", and
retries once before hard-failing on a mismatch. The versions actually used
are written to `benchmark/formats/tools/tool_versions.txt` on every fetch; the values
below are that file's contents:

```
citygml-tools = citygml-tools 2.5.0     # CityGML -> CityJSON
cjseq         = cjseq 0.3.1             # CityJSON <-> CityJSONSeq
java          = openjdk 21.0.11 (2026-04-21)   # citygml-tools 2.x needs 17+
```

`fcb` (FlatCityBuf serialisation) and `duckdb` are listed with the machine
below, because unlike the two above they are not pinned by a fetch script.

**FlatCityBuf is written and read by different versions, and that is
deliberate.** `readbench_prepare.sh` writes the `.fcb` with whatever `fcb`
CLI is on PATH (`fcb 0.7.8` on this host); the harness reads it with
`fcb_core` pinned **exactly** to `0.7.6` in
`benchmark/readbench/Cargo.toml`, because that is the release the published
FlatCityBuf figures were produced by and a caret range would let a later
one silently change what they mean. The skew is real and it bites: Caveat
25's `--index-node-size` panic is a 0.7.8-written index the 0.7.6 reader
mis-reads. Record BOTH versions with a run — the writing CLI's `fcb
--version` and the manifest's `fcb_core` pin — because neither alone
identifies the bytes that were measured.

### Machine

**Not captured for the committed run.** What the commit that added
`benchmark/formats/read_results/*.csv` records is the date and the platform family — a Linux
x86-64 host, AMD EPYC, 2026-08-17 — and nothing in the CSVs carries machine
metadata, so no CPU model, RAM figure or toolchain version can be recovered from
the artefacts. Treat the committed numbers as internally comparable (one machine,
one sitting, per dataset) but do not quote an absolute time against another
paper's hardware.

`benchmark/scripts/machine_record.sh` is the canonical capture: it runs the
`uname`, `lscpu`/`sysctl` and `free` lines below, plus `rustc`, `cargo` and the
commit hash, into a results directory's `MACHINE.md` — how `codec-bench` and
`rowgroup-bench` record their host. Run it as part of the next read run, add
the two lines it does not cover, and paste the result here:

```sh
uname -srm     # kernel, release and architecture; NOT `uname -a`, whose node
               # name is the host's address and these files are published
# Linux: lscpu | sed -n '1,15p'; free -b | head -2
# macOS: sysctl -n machdep.cpu.brand_string hw.memsize
duckdb --version; cargo --version; rustc --version; fcb --version
cat benchmark/formats/tools/tool_versions.txt      # the pinned conversion chain
```

`peak_rss_bytes` is in **bytes** on every platform (Linux reads `VmHWM`
in kB from `/proc/self/status`; `rss_to_bytes` in
`benchmark/readbench/src/main.rs` normalises the `ru_maxrss` other
platforms report), so a Linux run and a macOS run are directly comparable in
that column, subject to Caveat 34 for runs made before the `VmHWM` change.

## Reproduce

From the monorepo root:

```sh
just bench-prep --families formats
just bench-run --families formats
just bench-summary
```

Use `--datasets` to select datasets and `--smoke` for a small validation run.
Run each command with `--help` for the available options. The three commands
separate preparation, measurement and rendering; a summary never measures data.

Prepared artefacts carry conversion-chain stamps. Inputs whose stamps do not
match the active preparation chain must be rebuilt before measurement; an
existing filename is not evidence of a valid cache. Keep the source identity,
query-parameter sidecars and run configuration with the output. Do not append
repetitions from a different source or software revision to an existing run.
