# CityParquet read-benchmark methodology

The format family's query definitions live here, and so does the **one
numbered fairness-caveat list for every family** — the format family's own
and the `bloom` configuration family's. It is single because
`benchviz` renders exactly this list onto the summary page (`prep.read_caveats`
extracts it verbatim, `html.py` prints it under "Measurement caveats"), so a
caveat kept anywhere else never reaches a reader of the figures. The
suite entry points, dataset selection and figure layout are described in
[`../README.md`](../README.md). Run `just bench-prep --families formats`,
`just bench-run --families formats`, then `just bench-summary` from the
monorepo root. The format family measures reads only; the suite does not
time writes.

Result files must be interpreted with their own query-parameter sidecars and
run provenance. A full run writes its results to
`benchmark/runs/formats/results/`, which describe the datasets and
configurations named in those files; none is committed until the full run on
the benchmark host. Some caveats
below record observations made on an earlier corpus; they remain
qualifications until equivalent checks have been made on a new run.

## Purpose

Compare **read** performance — wall-clock time and memory — of the formats a
3D city model can actually be published in, across six access-pattern
scenarios that mirror how a consumer of that data actually reads it: a full
scan, a metadata-only count, a spatial window query at three selectivities,
an attribute-equality filter, a numeric-attribute aggregate, and a single-id
lookup. The read side is the geometry- and
query-facing half of the CityParquet argument; file sizes and the
bloom-filter configuration are covered by `benchmark/formats/README.md`.

## Formats

Five format tags. The canonical vocabulary, spelling and order are owned by
`Format::ALL` in `benchmark/readbench/src/format.rs` — this table is
a copy of it, and the CSV's `format` column, the `--formats` flag, the
`readbench_prepare.sh` artefact names and the plotter's ordering all use the
same five strings. A run with no `--formats` measures all five. They run
left-to-right from "what the data ships as today" to "what we propose":

| format tag    | what it is                                                                                                                                                                                                                                                                                         | index available                                                                                                                                      |
| ------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------- |
| `citygml`     | CityGML 2.0 XML (`.gml`) — the format most national datasets are published in, read through **this repository's own reader** (`cityparquet::citygml`). On the current corpus, the 3DBAG slice included, this artefact is **synthesised** from the CityJSON source (Caveat 14)                      | **none** — no offsets, no object directory, no spatial or attribute tree; every scenario is a full XML parse and an in-memory filter (see Caveat 12) |
| `cityjson`    | plain, whole-document CityJSON (`.city.json`): one JSON document, one `CityObjects` map, one shared document-level `vertices` array; written without optional whitespace (Caveat 40)                                                                                                               | **none** — the document must be parsed in one piece before any object is readable, so every scenario is a full parse (see Caveat 13)                 |
| `cityjsonseq` | CityJSONSeq, one self-contained JSON feature per line, feature-local vertices. Read from the PREPARED `<base>.city.jsonl` — `readbench_prepare.sh` always materialises one (copied from a `.city.jsonl` input, `cjseq cat` from anything else), and the runner refuses a CityGML document outright | **none** — every scenario is a full parse                                                                                                            |
| `flatcitybuf` | FlatCityBuf, written `fcb ser -A` and NOTHING else — every other index knob at its `fcb ser` default (attribute B+-tree branching factor 256, R-tree node size 16). One configuration, measured once, not a swept axis; see Caveat 33                                                              | R-tree spatial index (**2D only**, see Caveat 4) + B+-tree index over **every** attribute (`-A`)                                                     |
| `cityparquet` | our CityParquet package, one per dataset (`<x>.parquet/`), its rows written in Hilbert-curve order (`cityparquet convert --ordering hilbert --no-lod0`; Caveat 37); displayed as **CityParquet**                                                                                                   | Parquet row-group min/max statistics, tightened by the spatial clustering of Hilbert order, + column projection                                      |

The first three are **unindexed by construction**: a published `.gml`,
`.city.json` or `.city.jsonl` carries no way to answer any question without
reading all of it. That is not an omission in the harness — it is the finding
the benchmark exists to quantify, and the reason a `count` gap grows linearly
with dataset size while CityParquet's stays flat.

## Format configurations

The format comparison measures one configuration per format, so each of the
five tags names exactly one artefact per dataset. For CityParquet that
artefact is a package whose rows `readbench_prepare.sh` writes in Hilbert-curve
order. Hilbert order is the `cityparquet convert` default, and the prepare
script and the coordinator pin it explicitly (`--ordering hilbert`,
`RowOrder::Hilbert`) so the benchmark states its configuration rather than
inheriting it. It costs memory: the writer holds every feature before it
writes the first row, where `--ordering source` streams. Row order changes how
tightly each row group's bbox statistics enclose its rows, and so how much a
`bbox-query` can prune; it changes neither the package's schema nor what a
reader must understand to read it.

The only configuration axis is the `bloom` family's: `cityparquet` against
`cityparquet+nobloom`, both written in Hilbert order (see "The bloom family"
in `README.md`). The coordinator builds every variant package in Hilbert order
and refuses a `+source` suffix on a variant, so the two packages differ in
their bloom filters alone.

DuckDB reading a CityParquet package is not a format in this comparison: it is
a query engine, and the database comparison measures it beside cjdb and
3DCityDB (`benchmark/databases/README.md`; Caveat 5).

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
  same-machine timing block, an http-transport row's timing
  variance includes real network latency/jitter — the standard deviation
  (`time_std_s`) column now also captures that, not just OS/filesystem-cache
  noise. An http-transport run is a snapshot of one network path at
  one time, not a reproducible local benchmark.
- **Two extra metrics, per scenario: bytes transferred and HTTP request
  count — successful, LOGICAL reads, not raw wire traffic.** The CSV's
  trailing `bytes_read`/`http_requests` columns (see the CSV contract below)
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
  - `citygml`, `cityjson`, `cityjsonseq` — every unindexed format: a
    single **whole-object GET** — exactly 1 request, the whole file's byte
    length, by construction (there is no index to prune with, so there is
    nothing smaller to fetch). The one exception is `id-lookup` in
    `citygml` and `cityjsonseq`: the same single GET is read as a stream
    and dropped at the hit, so it reports the bytes received
    (`benchmark/readbench/src/formats/body_stream.rs`). All three route through
    the same `CountingObjectStore` as `cityparquet`, so the tally is measured
    rather than asserted.

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

The format, size and bloom families use six city datasets — Rotterdam,
Vienna, New York, Zurich, Tokyo (Chiyoda) and Montréal — and one
3DBAG slice, `3dbag_n1000000`, which every family treats as one more dataset.
Every CityJSON input is listed with its provenance, byte size and sha256 in
`corpus_urls.txt`; Tokyo and Montréal are derived by this project, and that
file records how, step by step. `README.md` lists every dataset with its
download URL and its query predicates.

The 3DBAG slice is the first 1,000,000 CityObjects of a pinned 7.6 GB
FlatCityBuf source, cut at a whole-feature boundary by `fcb-slice` (`just
fetch-3dbag`), so it holds 1,000,001 CityObjects: its name gives the nominal
target, and the recorded count is the actual one. It is also the database
family's dataset, and every family that reads it uses the same source bytes
and derived query parameters.

The corpus is building-focused and does not establish coverage of all CityGML
modules. It holds only content that CityGML 2.0, the baseline, can express
(Caveat 43), so it has no LoD 3 dataset with openings: the only LoD 3 content
is Tokyo's 21 LoD 3 solids and 103 LoD 3 installation geometries. 3DBAG and
Tokyo provide several LoDs per building; Tokyo and Montréal are textured. Synthesised CityGML must be
checked for information loss, including collapse of fractional LoDs
(Caveat 14); a successful conversion alone does not prove equivalent content.

## The six scenarios

Every format implements every scenario via its own natural mechanism —
never a hand-tuned shortcut, never an artificial common code path — and every
format returns the same thing for a scenario (the "return rule" in the
`returned` column). Each cell says what the format READS, what it VISITS and
what it RETURNS.

| scenario                         | returned (every format)                                                                                                         | `citygml`                                                                                                                                                | `cityjson`                                                                                                                                                                              | `cityjsonseq`                                                                                                                                                    | `flatcitybuf`                                                                                                                                                                                                                                                                                                                                    | `cityparquet`                                                                                                                                                                                                                                   |
| -------------------------------- | ------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `full-read`                      | every record, all fields, in the format's native form                                                                           | reads the stream of `cityObjectMember`s, each converted in full by the library's CityGML reader (appearance excluded); then visits as `cityjsonseq` does | parses the whole document; visits every CityObject's every field, every geometry's coordinates resolved through the shared `vertices` and `transform`, its semantic values and surfaces | parses every line; visits every CityObject as `cityjson` does, with the feature's own `vertices` and the header `transform`; `result_count` is the FEATURE count | `select_all` over every feature; the stored integer vertices are converted to real coordinates and visited, and semantic values, semantic-surface objects, attribute bytes and each template instance's anchor and transform are read (raw accessors, no CityJSON conversion; Caveat 32)                                                         | `full_read_visit`: every row group, every column; Arrow arrays and WKB walked in place, no CityJSON-shaped object built                                                                                                                         |
| `count`                          | the number of objects                                                                                                           | converts every member in full (as `full-read` does), then counts them — not a tag count                                                                  | parses the whole document; size of the `CityObjects` map                                                                                                                                | parses every line; counts the features                                                                                                                           | header feature count (O(1))                                                                                                                                                                                                                                                                                                                      | footer `num_rows` (O(1), no scan)                                                                                                                                                                                                               |
| `bbox-query` (1%/5%/25% of rows) | count and identifiers of the matching CityObjects, plus each match's highest-LoD geometry with its coordinates visited in place | converts every member; then as `cityjsonseq`                                                                                                             | parses the whole document; tests each CityObject's box over its `children` subtree (coordinates resolved through `transform`); visits each match's highest-LoD geometry                 | parses every line; tests each CityObject of the feature against its subtree box with feature-local vertices; visits each match's highest-LoD geometry            | hits from its 2D per-FEATURE R-tree (z dropped; Caveat 4); each hit feature read once, every CityObject in it tested against its subtree box, and each match's highest-LoD geometry walked                                                                                                                                                       | row groups pruned by `bbox` statistics; only `id`, `bbox` and the geometry columns read; a row filter on `bbox` runs first, so a rejected row decodes no geometry; the highest non-null LoD column of each kept row walked in place — **exact** |
| `attr-filter`                    | count and identifiers of the matching CityObjects                                                                               | converts every member; tests each CityObject's `attributes`                                                                                              | parses the whole document; tests each CityObject's `attributes`                                                                                                                         | parses every line; tests each CityObject's `attributes`                                                                                                          | the features its B+-tree attribute index hits are read once each, without decoding geometry, and every CityObject in them is re-tested; a column outside the index, a hit list longer than the file's feature count, or an empty hit list over HTTP (both tagged `attr-index-failed`, Caveat 11), falls back to a `select_all` walk that decodes only that column (Caveats 19, 32) | row groups pruned by the Bloom filter (an `==` predicate), then by min/max statistics; only the predicate column and `id` read; matching ids returned                                                                                           |
| `attr-stats`                     | `(min, max, sum, count)` of a numeric attribute                                                                                 | converts every member; folds over every numeric value (Caveat 35)                                                                                        | parses the whole document; folds over every numeric value (Caveat 35)                                                                                                                   | parses every line; folds over every numeric value (Caveat 35)                                                                                                    | a walk that decodes only that attribute, no geometry, folding the four values (no numeric-range index; Caveat 35)                                                                                                                                                                                                                                | min/max from column-chunk statistics; sum/count from a one-column scan                                                                                                                                                                          |
| `id-lookup` (x4)                 | the whole object, every field                                                                                                   | converts members until the hit (early exit), then visits every field of the object; a miss reads to the end of the document                              | parses the whole document; one map lookup, then every field of the object visited                                                                                                       | parses lines until the feature holding the hit, then visits every field of the object                                                                            | a `select_all` walk that stops at the hit (`fcb ser -A` builds no `id` index; Caveat 19); every field of the object read                                                                                                                                                                                                                         | row groups pruned by the Bloom filter, then by min/max statistics on `id`; a row filter on `id`; every field of the hit visited natively                                                                                                        |

What "returned" means here, for every format:

- **Returned geometry is visited, not handed over.** A spatial-window match's
  geometry has every coordinate visited in place, so a zero-copy format cannot
  win by never touching the geometry it returns.
- **"Highest LoD" is per object**: each match contributes the geometry of its
  own highest LoD, not of a dataset-wide one.
- **Appearance is excluded** for every format, so no format reads or visits
  textures or materials.
- **An implicit geometry counts by its anchor and transform only**; its
  template is not expanded per instance.

The `bloom` family's two packages, `cityparquet` and `cityparquet+nobloom`,
share the `cityparquet` runner and its column here: a package without bloom
filters is still a plain CityParquet package on disk, read by the same code,
and only its row groups' filters differ. That is why the bloom question gets
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
count, share); `benchmark/databases` derives its own with the same rule
(`citybench/params.py`). It appears in `notes` as `attr=<column>=<value>` or
`attr=<column>>=<q>`.

**Hand-picked, for the datasets this benchmark actually measures.** These are
the queries a reader of the corpus would recognise, chosen once from a survey
of the prepared packages; the share is of all CityObject rows:

| dataset                                                        | predicate                  | share |
| -------------------------------------------------------------- | -------------------------- | ----- |
| `3dbag_*` (the slice, and any smaller prefix `fcb-slice` cuts) | `b3_dak_type == "slanted"` | ~35%  |
| `zurich_building_lod2`                                         | `class == "BB01"`          | 19.4% |
| `vienna_102081`                                                | `roofType == "FLACHDACH"`  | 45.4% |
| `nyc_da13_buildings`                                           | `BIN == "1000000"`         | 0.7%  |
| `rotterdam_delfshaven`                                         | `TerrainHeight >= 2.45`    | 25.4% |

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

`benchmark/runs/formats/results/*.csv`, one row per (dataset, format, scenario
[, selectivity target]):

```
dataset,format,scenario,selectivity,result_count,time_mean_s,time_std_s,time_median_s,time_min_s,time_max_s,time_q1_s,time_q3_s,peak_heap_bytes,peak_rss_bytes,repeat,notes,bytes_read,http_requests,row_groups_total,bloom_pruned,stats_pruned,filter_bytes
```

- `time_mean_s` / `time_std_s` / `time_median_s` / `time_min_s` /
  `time_max_s` / `time_q1_s` / `time_q3_s` — **warm-cache** arithmetic mean,
  population standard deviation, median, extremes and quartiles (linear
  interpolation at `p * (n - 1)` on the sorted samples) of `repeat` samples (default 25; one further, discarded
  warmup precedes them; see "Sampling" below), 6-decimal precision. `benchmark/databases` records the same seven
  statistics under the same names, so a timing quoted from either CSV under one
  name is the same statistic. The summaries (`just bench-summary`) report the
  median by default, with the interquartile range (`time_q1_s` to `time_q3_s`)
  as its spread and the extremes recorded beside it; `--statistic mean` reports
  the mean ± the standard deviation instead. The standard deviation is the population one because the
  warm repeats are the whole measured set, not a draw used to infer a wider
  one. A fresh child process is spawned per
  sample (see "Warm vs cold" below) — independent OS page-cache and
  independent `peak_alloc` state per sample, never reused across repeats.
- `peak_heap_bytes` — the `peak_alloc` global-allocator high-water mark for
  that one in-process scenario call. Every format runs in the
  `cityparquet-readbench` child, so every row carries one.
- `peak_rss_bytes` — the child's own peak resident set size, for every
  format. On Linux the child reads
  `VmHWM` from `/proc/self/status`; elsewhere it reads
  `getrusage(RUSAGE_SELF).ru_maxrss`, **normalised to bytes** by
  `rss_to_bytes` in `benchmark/readbench/src/main.rs` (`ru_maxrss` is
  natively KiB on Linux per `getrusage(2)`, bytes on macOS/BSD). The column
  is bytes throughout. On Linux it is the child's own peak, not one
  floored at the coordinator's RSS — see Caveat 34.
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
  - `count-mismatch` — this row's `result_count` disagreed with the
    reference or with the other formats, and the run failed (Caveat 2);
  - `budget` (always last) — the cell time budget stopped sampling before
    `--repeat` samples (see "Sampling" below).
- `bytes_read` / `http_requests` — **empty for every `--transport local`
  row** (no HTTP concept locally); for a `--transport http` row, the total
  bytes transferred and HTTP request count that scenario's own
  transport-agnostic reader made (see "HTTP transport" above).
- `row_groups_total` / `bloom_pruned` / `stats_pruned` / `filter_bytes` —
  **empty on every row but a CityParquet `id-lookup` or `feature-lookup`**:
  the table's row groups, those its bloom filters ruled out, those its
  min/max statistics ruled out among the rest, and the bitset bytes of every
  filter examined (32 per block; header bytes and transport overhead are not
  counted — `bytes_read` carries the latter over HTTP). The kept row groups are
  read by one reader, as without filters; an `id-lookup` hit stops at its
  first match. Deterministic across repeats; the first warm sample's values
  are recorded.

## Sampling

A cell is one (dataset, format, scenario, query) measurement. Its samples
run back to back: one discarded warm-up, then up to `--repeat` timed samples
(default 25), before the next cell starts. Samples are not interleaved
across formats, so a slow drift in the host's state lands on whichever cells
run while it lasts.

`--cell-budget-s <seconds>` (off by default) caps the time a cell spends
sampling. After each timed sample the coordinator stops when the cell's runs
so far, warm-up included, have taken at least the budget and at least
`--min-repeat` timed samples exist (default 7); it always stops at
`--repeat`. A `--min-repeat` above `--repeat` is clamped to `--repeat`. The
row's `repeat` column records the samples actually taken, a cell that stopped
early carries the `budget` tag in `notes`, and both the `<csv>.params.json`
(`sampling`) and the `<csv>.samples.json` sidecars record the budget and the
effective floor. The root recipes `bench`, `bloom-bench` and
`bloom-bench-http` take the two as `CELL_BUDGET_S` (empty: off) and
`MIN_REPEAT`; `just bench-run` takes `--cell-budget-s` and `--min-repeat`.

## Warm vs cold protocol

The **headline numbers are warm-cache statistics**: the CSV records both the
mean and the median of `repeat` fresh child processes (default 25), and the
summaries report the median unless `--statistic mean` is given. A further discarded warmup precedes the samples, and the OS page
cache and (for the in-process formats) allocator state are left however the
previous sample left them — i.e. "warm" describes the OS/filesystem cache, not a
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
   sibling entries linked only by `parents`/`children`). `cityjsonseq`,
   `flatcitybuf` and `citygml` instead count top-level **features/members**
   for `count` and the `full-read` `result_count` (a CityJSONSeq/FCB feature
   bundles one top-level CityObject with all its children inline, exactly as
   a CityGML `cityObjectMember` nests its `BuildingPart`s). So the `count`
   and `full-read` rows split into **two grains**:

   | grain                                                      | formats                                 |
   | ---------------------------------------------------------- | --------------------------------------- |
   | one row per **CityObject** (children counted separately)   | `cityparquet`, `cityjson`               |
   | one row per **top-level feature/member** (children inline) | `citygml`, `cityjsonseq`, `flatcitybuf` |

   The `full-read` totals the coordinator compares (`objects`, `geometries`,
   `semantic_faces`) are per CityObject in every format; only its
   `result_count` keeps the feature grain. `bbox-query`, `attr-filter`,
   `attr-stats` and `id-lookup` are **CityObject-granular in every format**
   — `citygml`/`cityjsonseq`/`flatcitybuf` flatten to per-CityObject
   matching for these four scenarios (`flatcitybuf` because that is what its B+-tree
   attribute index naturally returns per entry: `fcb_core` indexes every
   value of a feature's `city_objects` map, not only the root object, so on
   Vienna an indexed `attr-filter` returns 600 entries from 307 features,
   and the raw-accessor walks of Caveat 32 count the same way; the two
   parsing formats by explicit choice, to match) — so these four are
   directly comparable across every format; the `count` and `full-read`
   `result_count` are not.
   Empirically: `lod3_railway.city.json` is 121 CityObjects / 38 top-level
   features; `delft.city.jsonl`'s `object_type == "BuildingPart"` count is
   1116 CityObjects (out of 2231 total CityObjects / 1115 features);
   `railway_lod3_fragment.gml` is 6 CityObjects / 4 members. Each of those is
   asserted in the runners' own tests, not merely claimed here.

   **What "an object's bbox intersects the window" means, in every format.**
   An object's box is the min/max over every vertex referenced by a geometry
   in its **subtree** — its own geometries and every descendant's, reached
   through `children` — so a `Building` with no geometry of its own matches a
   window its `BuildingPart`s intersect, and a `CityObjectGroup` matches
   through its members. This is the subtree union the CityParquet `bbox`
   column stores (the specification's "Spatial metadata"), and every runner
   computes the same union: `cityjson` over the shared vertices, `citygml`
   and `cityjsonseq` over each feature's own vertices, and `flatcitybuf`
   for every CityObject of each feature its R-tree hits. Every format
   therefore returns the identifiers of the same matching CityObjects, and
   the coordinator compares their digest (Caveat 2). A box intersects the
   window when it overlaps it on every axis, edges included. An object with
   no geometry anywhere in its subtree has no box and matches nothing.

   The CityParquet `bbox` column also covers an object's declared
   `geographicalExtent`, which no runner reads (FlatCityBuf's R-tree holds
   the boxes `fcb ser` computed), so on a dataset that declares one the
   column can reach beyond the vertex box. On this corpus Zurich and
   Montréal declare one on every object. Montréal's reaches beyond the
   vertex box in `x`/`y` on 1,544 objects, by at most 1.5 µm. Zurich's
   reaches beyond it on 108,396 objects, by at most its 1 mm quantisation.
   The untied windows guarantee only that every edge lies more than one
   quantisation step from every `bbox` edge (Caveat 21), which for Zurich is
   the same 1 mm, so the guarantee alone does not keep a window edge out of
   the band between an object's vertex box and its `bbox`. The coordinator's
   check does: a window edge inside that band would make `cityjson`
   disagree with `cityparquet`, and the run would fail. On the current
   Zurich artefacts no window edge is closer than 3 mm to a `bbox` edge.

2. **Selectivity's denominator is the CityObject total, for every format.**
   The four CityObject-granular scenarios (`bbox-query`/`attr-filter`/
   `attr-stats`/`id-lookup`) divide by the **dataset-global CityObject
   total** — the same number as CityParquet's own `count` — as a single
   shared denominator across every format, so their selectivity is always
   in `(0, 1]` and directly comparable format to format. `count` and
   `full-read` carry no selectivity.

   **A count that disagrees fails the run.** Once every format has run, the
   coordinator checks each row's `result_count` at its own counting level.
   `count` and `full-read` must equal the CityParquet table's CityObjects or
   its features (root objects, `id == feature_id`); each `bbox-*` window
   must select exactly the CityObjects whose box intersects it as the
   CityParquet `bbox` column says (the reference count is in the parameter
   sidecar, `windows[].objects`); `attr-filter` must match the predicate's
   `matched`. Beyond the counts, the coordinator compares what each format
   RETURNED (Caveat 42). `attr-stats` and each `id-*` probe have no reference and must
   agree across every format. A disagreeing row is tagged `count-mismatch`
   in `notes`, the CSV is written anyway, and the coordinator exits
   non-zero naming the scenario, the formats and the counts; a clean run
   prints `cross-format consistency OK` on stderr.

3. **`full-read` returns the same thing from every format, by different
   work.** Every format returns every record with all its fields in its
   native form, and every format visits every coordinate as a real
   coordinate: CityGML streams its members through the library's CityGML
   reader, which decodes every `gml:pos`/`posList`, resolves every
   `xlink:href` surface reference and rebuilds a feature-local vertex pool;
   plain CityJSON parses one whole document and resolves every boundary leaf
   through its shared vertex array and `transform`; CityJSONSeq parses every
   line and resolves each feature's own vertices through the header
   `transform`; FlatCityBuf converts its stored integer vertices to real
   coordinates; CityParquet walks its Arrow arrays and WKB in place. Each is
   that format's own full-read cost, reported per format and never
   normalised into a shared unit of work. The coordinator checks that the
   five agree on the CityObjects, geometries, semantic faces and extent they
   visited (Caveat 42).

4. **FlatCityBuf's `bbox-query` is 2D.** Its spatial R-tree indexes x/y
   only; the query window's z component is silently dropped when querying
   FCB (the window itself is still constructed with a full z range, as it
   is for every other format, but FCB's own index simply has no z
   dimension to test against). The index's hits are therefore a superset
   of the matches, never a subset, and the runner's own 3D test of every
   CityObject in a hit feature removes the extra ones, so FlatCityBuf's
   `result_count` equals every other format's.

   The R-tree also indexes FEATURES, not CityObjects: its entry is a
   feature's 2D box. The runner therefore reads each hit feature once and
   tests every CityObject in it against that object's own 3D subtree box,
   so the returned CityObjects agree with every other format, but a hit
   feature's non-matching CityObjects (and every feature whose 2D box
   intersects while no 3D box does) are read and tested as well. That
   extra reading is the cost of a per-feature 2D index, and it is inside
   FlatCityBuf's timing.

5. **DuckDB over CityParquet is measured by the database comparison, not
   here — and an ad-hoc DuckDB query over a package needs one setting.**
   This family times each format through its own reader in a
   `cityparquet-readbench` child. DuckDB reading the same Hilbert-ordered
   package is a query engine rather than a format, so
   `benchmark/databases/README.md` measures it beside cjdb and 3DCityDB,
   with its own query suite, thread configurations and count cross-check.
   Its rows are not a sixth format and must not be set beside this
   family's rows as if they were.
   DuckDB reads the package with `read_parquet()` straight over a
   `cityparquet-rs`-written package — full geometry, every LoD column.
   **However**: the package's WKB geometry columns carry GeoParquet "geo"
   file metadata, and DuckDB's spatial extension (autoloaded) eagerly
   tries to decode them into its own native `GEOMETRY` type the instant a
   query references the column — even a bare `SELECT` with no function
   applied — and that native decode does not support the multi-surface/
   solid WKB shapes our LoD1.2/LoD1.3/LoD2 geometries use, failing with
   `Invalid Input Error: Unsupported geometry type in WKB`. The database
   family sets `enable_geoparquet_conversion=false` on every connection
   (`benchmark/databases/src/citybench/systems/duckdb_cp.py`,
   `citybench/params.py`), which makes
   those columns read back as plain `BLOB`. **Anyone running an ad-hoc
   DuckDB query against a `cityparquet-rs` package that touches a geometry
   column needs the same setting**, or will hit the identical error. The
   LoD0 footprint is the exception: it is written with Parquet's own
   GEOMETRY logical type, which DuckDB decodes to `GEOMETRY` whatever the
   setting.

6. **A DuckDB memory figure and a format memory figure measure different
   processes.** Here `peak_heap_bytes` and `peak_rss_bytes` belong to one
   fresh `cityparquet-readbench` child per sample (see the CSV contract
   above). The database comparison reports no `peak_heap_bytes` for DuckDB,
   and its `peak_rss_bytes` samples the process executing the query — for
   DuckDB the harness's own Python process, which hosts the engine
   in-process, keeps one connection across the scenario sequence and so
   carries the interpreter, the engine's idle baseline and memory retained
   from earlier scenarios (the `peak_rss_bytes` scope caveat in
   `benchmark/databases/README.md`). Quote each within its own family; a
   ratio between the two is a ratio of different scopes.

7. **Warm vs cold — never silently mixed.** The headline numbers everywhere
   in this document and in `benchmark/runs/formats/results/*.csv` are warm-cache
   statistics (the summaries' median by default, the mean under
   `--statistic mean`); the single `cold`-tagged row per format (see "Warm vs cold"
   above) is a distinct, separately-reported measurement, always
   `full-read` only, never averaged into or compared unlabelled against the
   warm rows.

8. **Sub-millisecond deltas are noise; single-threaded reads are pinned.**
   As in `benchmark/formats/README.md`'s own methodology, deltas under roughly 10 ms at
   `repeat = 7` are within scheduler/filesystem-cache noise and are not
   cited as a finding by themselves. That threshold applies to a run at
   `repeat = 7`; the default is 25, and a run at 25 samples is read against
   its own spread (the interquartile range under
   the median default), not against this figure. Every format's reads here run
   single-threaded (no Parquet multi-threaded row-group decode) — a
   deliberate, disclosed choice so timing differences reflect the
   format/mechanism, not thread-count parallelism a production deployment
   might or might not enable. Parallel query execution is measured in the
   database comparison, whose rows carry `threads=single` or
   `threads=parallel`.

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
   `readbench_prepare.sh` cuts the FlatCityBuf and CityParquet artefacts
   from that one file. Each probe is verified present in both the
   CityParquet table and the CityGML artefact before it is used; a probe that
   fails verification is replaced by the nearest feature that passes and the
   row carries `id-substituted`.

   **Every format now takes the best mechanism its encoding affords**, which
   is the change that makes the column mean one thing:

   - `cityjsonseq` and `citygml` stop at the hit. CityGML did not
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

10. **The timing columns are end-to-end read latency, not isolated query compute.** The
    timed window is the whole per-format `run()` call, which INCLUDES opening
    the file, reading Parquet/FlatCityBuf metadata or the CityJSONSeq header,
    and (for CityParquet full-read/id-lookup) a metadata open — not only the
    query kernel. This is deliberate and consistent across every format (each
    pays its own open+read), and it is what a caller issuing a one-shot query
    against a file actually experiences; but it means a sub-millisecond
    the time for a metadata-only scenario (`count`) is dominated by file-open,
    not query work. Interpret the numbers as end-to-end single-query latency,
    not a pure in-memory kernel micro-benchmark.

11. **FlatCityBuf index assumptions and compression scope (known limitations).** The
    FlatCityBuf runner uses FCB's native indexes (`select_query` for bbox,
    `select_attr_query` for the attribute filter), which requires the `.fcb`
    to carry a spatial index (default) and an attribute index (`fcb ser -A`,
    which `readbench-prepare` always passes; `id` is never indexed, Caveat
    19). If the attribute index cannot answer, the runner falls back to a
    full walk and **says so in the CSV `notes`**: `no-attr-index` when the
    column carries no B+-tree at all, `attr-index-failed` when the index
    query errored, returned a hit list longer than the file's feature
    count, or, over HTTP, returned no hits. The second case is a known
    truncation in `fcb_core` 0.7.6: its attribute-index iterator stops
    after as many entries as the file has features, while the index holds
    one entry per matching CityObject, so a predicate that matches more
    CityObjects than there are features would otherwise lose matches
    silently. The third is a defect in the same library's HTTP index
    reader, which answers no hits for a key that names 4,096 or more
    features (see "FlatCityBuf's HTTP attribute index drops every key that
    names 4,096 or more features" later in this section), so the HTTP arm
    verifies an empty answer by the full walk; a walk that also finds
    nothing is a verified empty result. An index-vs-scan measurement is
    therefore never mislabelled. That fallback is a raw-flatbuffer walk, not a
    CityJSON conversion — see Caveat 32. No artefact is read through an
    external compression layer: `citygml`, `cityjson` and `cityjsonseq` are
    read as uncompressed text and `flatcitybuf` as an uncompressed file, so
    their sizes and read times are those of the uncompressed files, while
    CityParquet's only compression is the Parquet page
    compression its writer applies by default. A gzipped text file is not
    one of the five formats, and no number here describes one. External review
    (Codex, 2026-07-08) confirmed the query primitives, bbox prune + row-level
    filter, and allocator placement correct; its two flagged "dictionary"
    criticals were verified FALSE POSITIVES — `TypedDictionaryArray::value(i)`
    resolves the row's key, and an `attr-filter(object_type)` run
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

13. **`cityjson` and `cityjsonseq` resolve coordinates differently in
    `full-read`.** Both resolve every boundary leaf into a real-world
    coordinate, but `cityjson` resolves it through the one document-level
    `vertices` array and `transform`, while `cityjsonseq` resolves it
    through the feature's own local vertices and the header `transform`. On
    the `lod3_railway` fixture `cityjson` performs **245,137 leaf
    resolutions against 73,554 unique vertices** (the leaves outnumber the
    vertices more than threefold, so this is not a per-vertex pass that
    could be hoisted).

    That work is measured, not assumed. _Within the `cityjson` runner_, on
    the same fixture and machine, `full-read` costs roughly a fifth more
    elapsed time than `count` in release mode — median of 9 runs, 0.199 s
    for `count` against 0.243 s for `full-read` — so the leaf resolution is
    real work rather than something the optimiser elides, and
    `std::hint::black_box` pins that rather than trusting it to stay true.
    (That ~20% is a `cityjson`-internal figure, **not** the
    `cityjson`-vs-`cityjsonseq` gap; the cross-format gap also carries the
    whole-document-vs-line-oriented parse difference.)

    The two rows therefore do the same job, return the same totals (Caveat
    42), and differ in where their vertices live: a shared, document-level
    array against a feature-local one. A `cityjson`-vs-`cityjsonseq`
    `full-read` delta measures that layout and the parse difference, not a
    difference in what is visited.

14. **Conversion provenance: the chain runs FORWARDS ONLY, and nothing
    derives from CityParquet.** Every measured artefact is derived from the
    published source document by `benchmark/scripts/readbench_prepare.sh`, in one
    direction:

    ```
    CityGML --citygml-tools 2.5.0 to-cityjson--> CityJSON --cjseq 0.3.1 cat--> CityJSONSeq
                                                     |                    |--fcb ser -A---------> FlatCityBuf
                                                     |                    |--cityparquet convert --no-lod0-> CityParquet
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
    caveat is entirely about what that costs. The `cityjson` artefact is the
    CityJSON stage without its optional whitespace (Caveat 40).

    **One corpus source derives from CityParquet: Montréal.** By the
    author's decision, the Montréal CityJSON was exported from this project's
    own published CityParquet packages of three boroughs (the derivation is
    in `corpus_urls.txt`). The chain above still runs forwards from that
    CityJSON, so within a run every artefact, CityParquet included, derives
    from the same document; but the document itself passed through
    CityParquet first, so whatever the CityParquet encoding could not hold
    is absent from every Montréal artefact. A Montréal number compares the
    formats on what survived that round trip, not on the city's published
    original.

    **CityGML synthesis — the one backwards hop, and its cost.** Where the
    source document is not itself CityGML, the `citygml` artefact is produced
    by `citygml-tools from-cityjson -v 2.0 --no-pretty-print` from the
    CityJSON stage. This REVERSES an earlier rule of this benchmark, which
    reported such an artefact as "not derivable" and skipped it on the grounds
    that a round-trip product is not the source data. Three things about that
    reversal, in the order a sceptical reader will raise them:

    - **Why it changed.** This benchmark's claim is a comparison BETWEEN
      formats, so a dataset that produces four artefacts and skips the fifth
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
    - **What it cannot preserve, and how the corpus avoids it.** CityGML 2.0
      has only integer LoDs. 3DBAG carries LoD 0, 1.2, 1.3 and 2.2, and
      `citygml-tools from-cityjson -v 2.0` writes the LoD 1.3 solid as
      `lod1Solid` and drops the LoD 1.2 one, so a CityGML artefact
      synthesised from it would carry less geometry than the other four. The
      3DBAG slice is therefore cut **without LoD 1.2**
      (`fcb-slice --drop-lod 1.2`, which `just fetch-3dbag`
      passes): all five artefacts hold the same LoD 0, 1.3 and 2.2. The drop
      keeps every CityObject and removes the vertices only LoD 1.2 used, so
      no format carries orphan coordinates;
      `benchmark/readbench/tests/drop_lod.rs` checks that every object then
      carries at most one geometry per integer LoD, which is what CityGML 2.0
      can hold. A source with two fractional LoDs at one integer level needs
      the same treatment before its `citygml` row is content-equivalent to
      the others. Two geometries at the SAME LoD on one object are a
      different matter, and the whole corpus is normalised against them
      before any artefact is built (Caveat 41).

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
    a 3DBAG tile, 1,110 top-level GML members against 2,221 CityObjects.
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

17. **The `attr-stats` scenario is not uniform across the corpus.**
    `nyc_da13_buildings` (23,777 objects) carries **no numeric attribute at
    all**, so it has no `attr-stats` row. Elsewhere the aggregated column
    covers all of a dataset's CityObjects or only part of them:

    | dataset                | column               | CityObjects carrying it |
    | ---------------------- | -------------------- | ----------------------- |
    | `rotterdam_delfshaven` | `TerrainHeight`      | 853 of 853              |
    | `vienna_102081`        | `measuredHeight`     | 1,102 of 1,322          |
    | `zurich_building_lod2` | `GebaeudeStatus`     | 52,834 of 198,699       |
    | `tokyo`                | `measuredHeight`     | 38,743 of 49,915        |
    | `montreal`             | `measuredHeight`     | 31,415 of 31,415        |
    | `3dbag_n1000000`       | `b3_bag_bag_overlap` | 331,363 of 1,000,001    |

    Tokyo's column is on its `Building`s, not its 11,172
    `BuildingInstallation`s, and 1,041 of its 38,743 values are PLATEAU's
    `-9999` "not measured" placeholder. They are aggregated with the rest:
    no runner has an exclusion predicate, and adding one to every runner
    would change the timed work of every format. Tokyo's `min`, `sum` and
    hence its mean are therefore not heights, and its `attr-stats` row is a
    timing, not a statistic of the city.

    `just bench` takes the column from the dataset's hand-picked entry
    (`HAND_PICKED_STATS` in `benchmark/readbench/src/params.rs`: Tokyo and
    Montréal) or derives it, and omits the row where there is none, so an
    absent `attr-stats` row is expected rather than a failed measurement.
    **Do not read a missing row as a zero, and do not average a
    partial-coverage row into a cross-dataset aggregate figure.** Every other
    scenario is unaffected: they derive from geometry, object type or id,
    which every dataset has.

    The retired corpus had a different version of this problem — two datasets
    with 1 and 13 top-level objects, whose every filter matched either
    everything or nothing — and the current corpus has none: its smallest
    entry holds 379 objects, so every selectivity ratio carries information.

18. **Wire size equals disk size on this corpus, but the SOURCE size is not
    the size of what gets measured.** Every current entry is served
    uncompressed, so the pinned wire bytes are also the bytes on disk — unlike
    the retired corpus, where `.zip`/`.gz` entries differed by up to 30x
    (Estonia's 11 MB archive expanded to 323 MB) and the disk figure was the
    one to quote. That distinction no longer applies here.

    What does apply: **a dataset's pinned size is its CityJSON size, and four
    of the five measured artefacts are not that file.** The synthesised
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
    The deciles are cut from the seq order, which is the source CityJSON
    document's own object order, so the CityJSON artefact shares it. The
    FlatCityBuf and CityParquet artefacts are cut from that same file — but none of the
    CityGML, FlatCityBuf and CityParquet artefacts preserves it. The CityGML
    is synthesised independently by `citygml-tools`; the `.fcb` is ordered by
    its own R-tree; the CityParquet package's rows are in Hilbert-curve
    order, so a probe sits wherever its object's Hilbert key puts it. The
    CityGML and FlatCityBuf rows show **non-monotonic** times across
    `id-10pct`/`id-50pct`/`id-90pct`. Presence is
    verified for every probe; position is not. Read the three hit rows for
    any of those three formats as three samples of the distribution, never
    as a position curve.

21. **A bbox row's target and its achieved selectivity can differ, and
    `approx` says where.** Row counts are discrete, so a target is not always
    reachable: 1% of `vienna_102081`'s 1,322 rows is 13.22 rows. The search accepts
    a relative tolerance of ±10% and, when it cannot converge inside that,
    takes the nearest achievable window and appends `approx` to the row's
    `notes`. A missed target is disclosed in the artefact, never silent.

    No window edge lies on an object's edge. The search converges on a jump
    in the row count, so its edges land exactly on object edges, where
    whether an object counts depends on the last digit of how a format
    decodes the coordinate — a CityJSON integer times its scale against the
    Parquet double. Each `x`/`y` edge is therefore moved to the midpoint of
    the gap between two consecutive distinct object-edge values (a lower
    edge competes with the boxes' maxima, an upper edge with their minima);
    the gap must exceed twice the dataset's coordinate quantisation (the
    CityJSONSeq `transform.scale`) on that axis, and when the edge's own gap
    is narrower the nearest wide one is used. `z` spans the dataset's range
    with a margin either side. `achieved` and `approx` describe the moved
    window.

22. **bbox targets are expressed in CityParquet ROW space.** The search runs
    over the CityParquet package's per-row bboxes, so `cityparquet`'s own
    achieved selectivity is the one that lands on the target. Feature-grained
    formats (`citygml`, `cityjsonseq`, `flatcitybuf`) count a different unit
    and therefore report a different achieved fraction for the **identical**
    window — on `vienna_102081`, 0.026 against CityParquet's 0.0098 for
    `bbox-1pct`. The window is the same for every format, which is what makes
    the timings comparable; the `selectivity` column is not comparable across
    grains. See Caveat 1 on counting grain.

    The window is defined in the package's axis order: a CityParquet package
    stores `x` as longitude and `y` as latitude, as GeoParquet requires,
    whatever order the CRS declares. Every other artefact keeps the source's
    order, so for a CRS that declares latitude first (Tokyo's JGD2011,
    EPSG:6697) `citygml`, `cityjson`, `cityjsonseq` and `flatcitybuf` receive
    the window with `x` and `y` swapped. The parameter sidecar records the
    fact as `swap_xy`, read from the CRS's declared axis directions, never
    guessed from coordinate magnitudes; the `cityparquet` runner reads the
    package and uses the window as written.

23. **The format set is uniform across every dataset, the 3DBAG slice
    included.** Every dataset the suite measures gets all five formats,
    so a `citygml` row exists wherever any other row does and every
    factor against CityGML has its baseline. On the slice, as on every
    corpus dataset, that CityGML is a `citygml-tools` serialisation the
    preparation chain synthesises, not a published file (Caveat 14), and it
    is the largest artefact the chain writes, larger than the CityJSONSeq
    it derives from by the factor the run's `sizes.csv` records.
    Synthesising it is the longest step of `just
bench-prep`, and parsing it dominates the slice's `citygml` rows; quote
    those rows as the cost of reading this serialisation of the slice, with
    the same qualification as every other `citygml` number.

24. **The `bloom` family measures the 3DBAG slice alone, because pruning
    needs several row groups.** A filter rules out whole row groups. At the
    default 65 536-row groups a table of fewer rows is one row group, on
    which a hit can skip nothing and which says nothing
    about how pruning scales. The 3DBAG slice's 1,000,001 rows span 16 row groups and
    Zurich's 198,699 span 4; every other corpus dataset holds fewer than
    65 536 CityObjects and is a single row group. The family therefore runs
    on the slice only. Its probes are three lookups, each as a hit and a
    miss: `id-lookup`, `feature-lookup` and `attr-lookup`, an equality
    lookup on each configured text attribute (`bloom_attributes`; a run
    without attribute rows is shown in the summary as not measured). A hit still costs the
    row groups that hold the value, because a filter cannot narrow the
    search inside a group: on a 3DBAG tile written with 9 row groups,
    `documentnummer`'s hit returned 23 rows and its filters pruned 6 of the
    9 groups, so the reader still scanned 3. A miss can still read a group
    through a false positive at the 1 % target rate. The benefit depends on
    the row-group size, which the benchmark does not vary (65 536 rows per
    group, the writer's default). `cityparquet+nobloom` still carries
    row-group statistics, so the axis measures what the filters add on top
    of statistics, and every lookup row records both pruning counts.

    Row groups are pruned in two steps: the Bloom filter first, then the
    column chunk's min/max statistics. `bloom_pruned` counts the row groups
    the filters rejected and `stats_pruned` those the statistics rejected
    among the rest, so the two never count one row group twice. Each miss
    probe is chosen INSIDE the row groups' stored value ranges (a stored
    identifier or attribute value with `-readbench-absent` appended), so
    min/max statistics alone cannot reject it and a miss row measures the
    filters, not the statistics.

25. **In the `bloom` family, a filter's positive is not a match.** At FPP 0.01
    a miss can still keep a row group; `row_groups_total − bloom_pruned −
stats_pruned` on the `*-miss` rows is how many the reader still
    scanned.

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

30. **The format evidence is measured on bloom-enabled packages.** The
    `formats`, `sizes` and `bloom` families read packages the bloom-enabled
    writer produces, and each results directory carries a `MACHINE.md`
    describing the host it was measured on. The writer puts filters on
    `id`, `feature_id` and high-cardinality string attributes by default, so
    its packages are larger and its lookups prune; the `bloom` family's
    `cityparquet+nobloom` variant is the one package measured without them.
    The coordinator writes 22 columns, with `stats_pruned` after
    `bloom_pruned`, and applies the return rule of "The six scenarios".

31. **One generation of results, one timing block, one header.** Every
    results CSV reports the seven-column timing block
    (`time_mean_s` .. `time_q3_s`) over the warm samples, in the
    coordinator's 22-column shape (Caveat 30). Results with a median and
    `time_mad_s`, results measured on packages without default-on bloom
    filters, and results in the 21-column shape that predates `stats_pruned`
    exist only in git history and must not be set beside these: a median
    absolute deviation and an interquartile range are different spreads, and
    an `id-lookup` without filters is a different operation. The shapes keep
    them apart mechanically: the coordinator's
    `CSV_HEADER` (`benchmark/readbench/src/coordinator.rs`) is the only
    writer of a results header, `benchmark/plot/tests/test_csv_contract.py`
    holds the renderer's `READ_COLUMNS` to a leading prefix of it, and the
    summary loader refuses to render a CSV whose leading columns differ
    from `READ_COLUMNS` (a `time_mad_s` header among them).

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
    `result_count` in every pair: on a 10,000-object prefix of the 3DBAG
    stream `full-read` 0.404 s ->
    0.037 s, `attr-filter` on `object_type` 0.405 s -> 0.034 s, `id-miss`
    0.407 s -> 0.035 s, `attr-stats` 0.404 s -> 0.045 s; on
    `zurich_building_lod2` the same four rows 2.22/2.05/2.22/2.22 s ->
    0.55/0.50/0.50/0.51 s. Peak heap falls with
    them (1.3 MB -> 0.3 MB on zurich). The INDEXED paths
    (`select_attr_query`, `select_query`) were already native and are
    unchanged — `b3_dak_type == slanted` on that prefix sits at ~1 ms
    either way.

    FlatCityBuf is a comparison baseline here, not this repository's
    subject, so it is owed its best natural implementation. Published
    FlatCityBuf figures produced before this change overstate its
    attribute-, id- and full-read costs and must not be mixed with figures
    produced after it.

33. **One FlatCityBuf configuration, chosen by measurement.** The `.fcb`
    artefacts are written `fcb ser -A` with every other index knob left at
    its default — attribute B+-tree branching factor 256, R-tree node size 16. That is a decision, not an oversight, and it is not a swept axis:
    the paper's only configuration axis is CityParquet's own (bloom
    filters).

    A 100,000-object prefix of the 3DBAG stream was written at attribute
    branching factors 16, 64, 128 and 256 and read back with the benchmark's own child (3 repeats,
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

34. **`peak_rss_bytes` is the child's own `VmHWM` on Linux, because
    `ru_maxrss` there is floored at the coordinator's RSS.** A child that
    reported `getrusage(RUSAGE_SELF).ru_maxrss` could never report less
    than the coordinator's peak: Linux's `exec_mmap` folds the high-water
    mark of the address space a task had BEFORE `exec` into
    `signal->maxrss`, and under `posix_spawn`/`vfork` that address space is
    the parent's. Every format that used less than the coordinator would
    then carry the coordinator's memory, not its own, and any read-memory
    ratio computed from such values would be a ratio of floors.

    The child therefore reads `VmHWM` from `/proc/self/status` (its own
    `mm`, created by `exec`, is not inherited; measured 10.5 MB for a child
    under a 420 MB parent, against 419 MB from `ru_maxrss`). Reading it
    changes no timing. **Read-memory figures taken with `ru_maxrss` on Linux
    must not be set beside these**, and a value equal to the coordinator's
    own RSS is not a format's memory.

35. **Before 2026-09-23, only CityParquet's `attr-stats` computed the
    aggregate.** The scenario's contract is `(min, max, sum, count)` of a
    numeric attribute, but the `citygml`, `cityjson`, `cityjsonseq` and
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

36. **FlatCityBuf's schema has no CityObject `address`.** Tokyo carries an
    `address` on 35,067 CityObjects; `fcb ser` writes none of them, so the
    `.fcb` holds less than the other four artefacts. No query reads an
    address, so no `result_count` is affected, but a Tokyo size or full-read
    figure compares a FlatCityBuf file without addresses against files with
    them.

37. **The package is written without LoD 0 synthesis (`--no-lod0`), so it
    holds the same geometries as every other artefact.** By default
    `cityparquet convert` synthesises an LoD 0 footprint for every object
    without a source LoD 0, which no other format's artefact holds.
    `readbench_prepare.sh` and the coordinator's variant packages turn it
    off; the library default is unchanged. A source LoD 0 is kept: Tokyo's
    38,743 `Building`s keep theirs, and its 11,172 `BuildingInstallation`s
    (LoD 2 or 3 in the source) gain none. With synthesis on, the package
    gained a footprint for 788 of those installations and for the 12
    Montréal buildings without LoD 0, and on Rotterdam, Vienna, New York and
    Zurich, which are LoD 2 only, for every object. Synthesis changes no
    `result_count` (verified on Tokyo, with and without it), but it adds
    bytes: on Tokyo the package is 80,249,026 B with it and 80,184,081 B
    without, 64,945 B or 0.08 %; on Rotterdam it is 780,544 B with it and
    714,076 B without, 66,468 B or 8.5 %, a footprint for each of its 853
    buildings.

38. **On Tokyo the two counting levels are far apart: 49,915 CityObjects in
    38,743 features.** Each feature is a `Building` with its
    `BuildingInstallation`s inline, so the feature-grained formats
    (`citygml`, `cityjsonseq`, `flatcitybuf`) count 38,743 for `count` and
    the `full-read` `result_count`, while every format returns CityObjects
    for each `bbox-*` window; Caveat 1 states which is which, and the
    coordinator checks each level against its own reference (Caveat 2).
    Vienna (1,322 in 307) and Zurich (198,699 in 52,834) are nested too; Rotterdam, New York and Montréal hold one
    CityObject per feature.

39. **Isolation on a shared host is best effort, and recorded rather than
    assumed.** The citable host is shared and the harness has no root, so it
    pins the measured children to one NUMA node (`numactl`, else `taskset`
    without memory binding), keeps the coordinator on that node's first core,
    can cap the children's memory through `systemd-run --user`, and holds each
    sample while the node's share of the load exceeds `--max-load`. It cannot
    change the CPU governor, disable SMT or turbo, drop page caches or isolate
    cores (`isolcpus`). What a run requested and what was applied is in its
    `.params.json` under `isolation`, with each sample's `load1`, runnable
    count and `MemAvailable` in `.samples.json`; a params sidecar without an
    `isolation` object comes from a run that applied none of it. A row tagged
    `busy` ran after the longest wait with the node still contended and is not
    cited without its spread. The samples of one cell run back to back, with
    no interleaving across formats, so slow drift in co-tenant load lands on
    whole cells rather than averaging out across formats.

40. **The `cityjson` artefact is measured without optional whitespace, like
    the `citygml` artefact (`--no-pretty-print`, Caveat 14).**
    `benchmark/scripts/compact_json.py` writes it: it removes the whitespace
    outside strings and copies every other byte, so string contents and
    number spellings are the published ones and an already-compact document
    is unchanged. A JSON parser and serialiser would not do: `jq -c` (1.7.1)
    respells numbers (Tokyo's `1e-10` scale becomes `1E-10`, which shrinks
    Tokyo by 986 B), though every value survives. All six corpus sources
    are compact as published (montreal 498,371,022 B, nyc_da13_buildings 110,083,137 B,
    rotterdam_delfshaven 2,731,804 B, tokyo 315,968,009 B, vienna_102081
    5,635,634 B, zurich_building_lod2 292,500,409 B), and so is the
    normalised source Caveat 41 derives from one of them, so on the current
    corpus the step is a no-op: each `cityjson` artefact is byte identical
    to the source the chain builds from — the published one for five
    datasets, and for vienna_102081 (4,731,370 B) the normalised one. The
    normalised source is written by `serde_json`, which writes the shortest
    round-trip spelling of each number: every value survives, and
    vienna_102081's spellings are the published ones, but a source that
    spells a float otherwise (`-1.1e-06` for `-1.1e-6`) would be respelled,
    so the published spellings are guaranteed for the five pass-through
    datasets only. A size or
    parse-time gap against CityJSON therefore cannot be dismissed as
    whitespace.

41. **The corpus holds at most one geometry per LoD and object.** Before any
    artefact is built, `readbench_prepare.sh` normalises a CityJSON or
    CityJSONSeq source with `lod-normalise`
    (`benchmark/readbench/src/lod.rs`, `keep_first_per_lod`): each
    CityObject keeps the first geometry at each LoD in source order, the
    vertices nothing references any more are removed and the boundaries
    re-indexed, and every CityObject is kept, with or without geometry. The
    kept geometries' semantics, `material` and `texture` are unchanged;
    texture coordinates (`vertices-texture`) and the `appearance` arrays are
    left as they are, so every kept reference still resolves and a dropped
    geometry's texture coordinates stay in the shared array. A changed
    source is re-serialised compactly; an unchanged one is used byte for
    byte. This is a property of the CORPUS, chosen because CityParquet
    stores one geometry column per LoD: its writer keeps the first geometry
    at a LoD and drops the rest (`skipped_same_lod_geometries`), so without
    the normalisation the package would hold fewer geometries than the other
    four formats. "The same LoD" is the writer's key exactly — the `lod`
    string as `Lod::parse` reads it, so `2` and `2.0` are one LoD — and a
    GeometryInstance never claims a LoD. The CityParquet stage then asserts
    that the writer skipped nothing, and fails the dataset otherwise. On the
    corpus, vienna_102081 loses 1,102 of its 2,204 geometries (each LoD 2
    object carries a MultiSurface, kept, and a Solid, dropped; no vertex is
    orphaned); the other five sources and the 3DBAG slice are unchanged. A
    CityGML source is not normalised; none is in the corpus.

42. **Cross-format consistency covers what is returned, not only how
    many.** After the counts of Caveat 2, the coordinator's
    `check_returned` compares, per scenario and query tag, what every
    format returned: the identifier-set digest (count, sum and xor of the
    identifiers' hashes) of each `bbox-*` window and of `attr-filter`; the
    number of geometries returned by each window and the extent their
    visited coordinates span; and, for `full-read` and each `id-*` probe,
    the CityObjects, geometries and semantic faces visited and their
    extent. Extents are brought into the package's axis order and must
    agree within one quantisation step of the package's `transform` per
    axis; every other value must be equal. A disagreement tags the rows
    `count-mismatch` and fails the run. A row that reports no value for a
    part is named on stderr rather than passed silently.

    A format may be excused from one comparison only through the explicit
    `TOTAL_EXCLUSIONS` list in the coordinator, each entry naming the
    format, the comparison and the reason, and pinned by a test. The list
    holds one entry: **`citygml` is not compared on the `full-read` and
    identifier-lookup `extent`**. CityGML stores real coordinates and no
    `transform`, so the library's CityGML reader quantises them with its
    own step, derived from the CRS's units — a millimetre on a metre axis.
    Where the source's `transform` is finer (Tokyo's height step is
    1e-6 m), the CityGML extent lies up to half a millimetre from the other
    four formats', beyond the one-step tolerance. Its spatial windows'
    returned extent, and every count, digest and total, are still compared.

43. **The corpus holds only content that CityGML 2.0, the baseline, can
    express.** Every artefact derives from one source, and the comparison
    is fair only if all five formats hold the same content, so a dataset
    whose CityGML 2.0 synthesis loses content is excluded. Ingolstadt is
    excluded on this rule. Its 32,670 Window and Door semantic surfaces
    name no parent surface, and CityGML 2.0 can hold an opening only inside
    a wall or roof surface; `citygml-tools from-cityjson -v 2.0` writes the
    references but omits the polygons, and prints no warning, so its
    synthesised CityGML holds about a quarter of the dataset's faces. No
    corpus dataset carries Window or Door semantics, and none has a
    dangling reference; as a consequence the corpus has no LoD 3 dataset
    with openings, and its only LoD 3 content is Tokyo's 21 LoD 3 solids
    and 103 LoD 3 installation geometries.

- **The network family's times come from a simulated network.** The
  primary over-HTTP measurement reads through `net-sim`, a local server
  with a fixed bandwidth (one budget shared by every connection) and a
  fixed latency per request, with no TLS, HTTP/2, connection set-up,
  jitter, loss or service-side throttling. Its times are therefore a
  model of the transfer: a reader should cite the bytes read and the
  request count as the format's properties, and the time as what one
  stated profile makes of them. A run against real object storage is a
  snapshot of one path at one time. How each client issues its requests
  decides what the latency multiplies: the CityGML, CityJSON and
  CityJSONSeq arms fetch the whole object with one `GET` per query, so
  every query costs the whole file's transfer plus one latency, with one
  exception: the identifier lookup of the two streaming formats, CityGML
  and CityJSONSeq, parses the body as it arrives and abandons the
  transfer at the hit, so its `bytes_read` is the bytes received up to
  the hit (chunk-granular, counted on the body stream, not by
  `CountingObjectStore`), and only a miss reads the whole file. CityJSON
  is one JSON document and cannot stop early. On an abandoned transfer
  the simulated server has sent more than the client received (what was
  in flight); the params sidecar reports that gap as
  `server.unreceived_bytes` rather than folding it into either total. The
  CityParquet arm reads `metadata.json`, then each Parquet file through
  `parquet`'s `ParquetObjectReader` over `object_store` (a `HEAD` for the
  size, the footer, then the column chunks of the row groups that
  survive pruning, as ranged `GET`s that `object_store` may coalesce and
  issue concurrently); the FlatCityBuf arm reads through
  `http-range-client`'s buffered client, one ranged `GET` at a time, for
  the header, the spatial or attribute index and then the features.
  Because the text formats' bytes and requests are the same for every
  query, and their time nearly so (within 0.92–1.15× of read all in the
  end-to-end runs on Rotterdam and Vienna, with the transfer 83–94 % of
  it at `typical`), they are measured by default only on read all and the
  four identifier lookups; their other six queries are derived rows.
  A derived row is written only when every sample of the format's read
  all and identifier miss made one request and read exactly the
  artefact's size (an identifier hit cannot prove it: the streaming
  formats stop at the hit),
  copies read all's bytes and request count, and has no time, memory or
  result count (`notes` carries `derived-from=full-read;status=derived`).
  It states that the query costs the same transfer as read all; it is not
  a measured time, the figure marks it with `†`, and the cross-format
  consistency check skips it. `[network] whole_file_scenarios` in
  `manifest.toml` (`--network-whole-file-scenarios`) selects
  `full-read,id-lookup` (the default), `full-read` or `all`, the last
  measuring every query.

- **FlatCityBuf's indexed attribute filter over HTTP reads many times its
  file, and the cause is the pinned library, not the format.** On
  Rotterdam (a 2.95 MB `.fcb`) the `attr-filter` query
  (`TerrainHeight >= 2.45`, 217 matching CityObjects) issues 144 ranged
  `GET`s and reads 113.5 MB, about 38 times the file, while `full-read`
  takes 4 requests and 2.95 MB and the 1 % window 4 requests and 85 kB.
  The runner opens the file once and reads each hit once; the bytes come
  from `fcb_core` 0.7.6 and `http-range-client` 0.9.1. The B+-tree node
  fetch (`static_btree/stree.rs`) sets the buffered client's minimum
  request size to 1 MiB and nothing resets it, and the hit list comes
  back in key order, not file order, so a hit that falls before the
  buffer, or past its end, clears the buffer and fetches 1 MiB (or up
  to the end of the file) from that feature's offset; the next hit
  usually falls outside that buffer again. The spatial window does not
  show this because its iterator sets its own request size and visits
  the features in file order. Neither text equality filter on this
  corpus reaches this path. Vienna's `roofType = FLACHDACH` names 600
  CityObjects in a 307-feature file, so its hit list is longer than the
  feature count and the row falls back to a `select_all` walk (tagged
  `attr-index-failed`; 9 requests, 4.6 MB for a 3.6 MB file). Tokyo's
  `usage = "401"` gets no hits from the HTTP index at all and takes the
  same tagged walk (next item). The
  attribute filter's bytes, requests and time over HTTP are therefore a
  property of this client, and a reader should not cite them as the
  cost of FlatCityBuf's attribute index; the local arm reads a file
  handle and is not affected.

- **FlatCityBuf's HTTP attribute index drops every key that names 4,096 or more features**, and the cause is the pinned library, not the format.
  On Tokyo (a 265 MB `.fcb`), `usage = "401"` names 12,348 CityObjects
  through the local index, but the same query through `fcb_core` 0.7.6's
  `HttpFcbReader` returns an empty hit list after 4 requests and 1.09 MB.
  The same happens to `class = "3001"` (11,599 locally) and to
  `延べ面積換算係数 = "1"` (19,956), while keys with 566 and 84 matches
  come back whole. A key's B+-tree payload entry is 4 bytes plus 8 per
  matching feature. The HTTP reader prefetches 16 KiB of the payload
  section for a column with few distinct values
  (`compute_payload_prefetch_size`). It re-fetches an entry that does not
  fit in that prefetch with a fixed 32 KiB request
  (`batch_resolve_payloads`, `static_btree/stree.rs`). An entry longer
  than 32 KiB, which is any key naming 4,096 or more features, then fails
  to decode, and the reader drops it without an error. The local reader
  reads the whole entry from the file handle and is not affected. The HTTP
  arm therefore never publishes an empty index answer: it verifies it by
  the full `select_all` walk, tagged `attr-index-failed`. On Tokyo that
  walk returns the 12,348 objects, with the same identifier digest as the
  local index, for 254 requests and 261.4 MB. A genuine miss costs the
  same walk. A range predicate is not guarded: one heavy key inside the
  range would be dropped from a non-empty answer, which only the
  cross-format consistency check detects.

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

`fcb` (FlatCityBuf serialisation) is listed with the machine below, because
unlike the two above it is not pinned by a fetch script.

**FlatCityBuf is written and read by different versions, and that is
deliberate.** `readbench_prepare.sh` writes the `.fcb` with whatever `fcb`
CLI is on PATH (`fcb 0.7.8` on this host); the harness reads it with
`fcb_core` pinned **exactly** to `0.7.6` in
`benchmark/readbench/Cargo.toml`, because that is the release the published
FlatCityBuf figures were produced by and a caret range would let a later
one silently change what they mean. The skew is real and it bites: Caveat
33's `--index-node-size` panic is a 0.7.8-written index the 0.7.6 reader
mis-reads. Record BOTH versions with a run — the writing CLI's `fcb
--version` and the manifest's `fcb_core` pin — because neither alone
identifies the bytes that were measured.

### Machine

**Captured beside the results.** `benchmark/runs/formats/results/MACHINE.md`
records a run's host: kernel, CPU, memory, the Rust toolchain and
the commit. Treat a run's numbers as internally comparable (one machine,
one sitting, per dataset) but do not quote an absolute time against another
paper's hardware.

`benchmark/scripts/machine_record.sh` is the canonical capture: it runs the
`uname`, `lscpu`/`sysctl` and `free` lines below, plus `rustc`, `cargo` and the
commit hash, into a results directory's `MACHINE.md` — how `bench` and
`variant-bench` record their host. It adds an isolation table (`numactl` and
`taskset` availability, the chosen NUMA node and its cores, the pinning
command, the memory limit and whether the user cgroup delegates `cpuset` and
`memory`, the CPU governor, SMT, the kernel, other users' CPU and `MemTotal`
against `MemAvailable` at start) and a tool-version table (the
`cityparquet-rs` commit, `cjseq`, the `fcb` CLI and the pinned `fcb_core`,
citygml-tools, DuckDB, the PostGIS and 3DCityDB images, `cjdb`). Off Linux the
Linux-only rows read "not readable" or "not applied: not Linux", and an absent
tool reads "not found". The pinned conversion chain is recorded separately:

```sh
uname -srm     # kernel, release and architecture; NOT `uname -a`, whose node
               # name is the host's address and these files are published
# Linux: lscpu | sed -n '1,15p'; free -b | head -2
# macOS: sysctl -n machdep.cpu.brand_string hw.memsize
cargo --version; rustc --version; fcb --version
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

Use `--datasets` to select datasets (`3dbag` names the slice),
`--profile short` for the corpus without the slice and `--smoke` for a small
validation run. Run each command with `--help` for the available options. The
three commands separate preparation, measurement and rendering; a summary never
measures data. `just bench-prep` fetches the slice's source and cuts it
(`just fetch-3dbag`) and prepares every artefact (`readbench_prepare.sh`);
the `bench` recipe measures prepared artefacts and prepares nothing.

Prepared artefacts carry conversion-chain stamps. Inputs whose stamps do not
match the active preparation chain must be rebuilt before measurement; an
existing filename is not evidence of a valid cache. Keep the source identity,
query-parameter sidecars and run configuration with the output. Do not append
repetitions from a different source or software revision to an existing run.
