# Benchviz — design spec (final)

Builds a single self-contained `bench-summary.html` (replacing 14 per-dataset PNG
pairs + tables) and the static paper figures, from one data-prep step over this
repository's `bench/` result CSVs. It measures nothing: `just bench`,
`just compression-bench` and `just sizes` produce the CSVs, `just plot-pretty`
renders them.

This package used to live in the paper workspace (`scripts/benchviz/`) and reach
into `cityparquet-rs/` as a read-only submodule, resolving every path by counting
parent directories. It now lives beside the benchmark it reports, as a second
package of the `bench/plot` uv project, and takes its input and output locations
as flags (`benchviz/paths.py` holds the in-repo defaults). The paper workspace
calls the same code with its own `--html`/`--figures` destinations.

## Global conventions

- **Every plotted metric is a unitless ratio vs the CityJSONSeq baseline** for
  the same (dataset, scenario). Lower/left = better, everywhere. The baseline
  sits at 1× and is drawn as a reference cross/line in every panel.
  **One documented exception**: the scaling view plots absolute seconds and
  bytes on log-log axes, because the quantity being read there is the SLOPE of
  cost against cardinality — a flat line is a cost that does not grow with the
  model — and dividing by a baseline that is itself growing hides exactly that.
  It is the only view allowed to; anything else stays a ratio.
- **Per-dataset small multiples** at shared scale, ordered by CityObject count
  descending. The order is computed from the data, so each run's corpus orders
  itself and no dataset name is written down anywhere in this package.
- **The format axis is the harness's, not this package's.** The views plot
  `Format::DEFAULT_SET` (crates/cityparquet-readbench/src/format.rs), carried in
  `meta.format_axis`: `citygml`, `cityjson`, `cityjsonseq`, `flatcitybuf` and
  `cityparquet-hilbert` — one tag per format family, CityParquet represented by
  the configuration it would ship as. `cityjsonseq-gz` (a compression variant of
  a format already on the axis) and `duckdb-parquet` (an SQL-engine baseline) are
  NOT formats: their rows stay in `bench_data.json` when a run opts in, no view
  plots them, and the page says so once. Ordering is a separate question with its
  own run (`Format::ORDERING_SET`, `bench/ordering_results/`), its own baseline
  (the source-order package, not CityJSONSeq) and its own static figure; it is
  never mixed onto the format axis, which is the confound the two sets exist to
  keep apart.
- **Bars and lines carry HUE; markers still carry shape.** The colour rule
  below was written for scatter marks, where shape is the identity channel and
  grey keeps the panel quiet. A filled bar has no shape, and four greys in one
  group are not tellable apart at panel scale — so the bar and line views give
  each format a hue: `#7a4fa3` CityGML, `#00786b` CityJSON, `#1c63a8`
  FlatCityBuf, grey for the CityJSONSeq baseline, accent for CityParquet. The
  hues differ in lightness as well, so a greyscale print still separates them,
  and hue is never the sole channel: the row order is fixed in every group and
  the views print their values. The marker views are unchanged in shape and
  pick up the same hues, so one format looks the same everywhere on the page.
- **Color/markers**: accent `#e41a1c` (light) / `#fc8d62` (dark) for
  `cityparquet` (filled circle); same accent, open circle for
  `cityparquet-hilbert`; gray `#666`/`#999` for the rest with distinct shapes:
  cityjsonseq = ×/cross at (1,1) (it IS the baseline), citygml = star,
  cityjson = plus, flatcitybuf = triangle; cityjsonseq-gz = square and
  duckdb-parquet = diamond keep their marks for an opt-in run's footnotes.
  Shapes carry identity (color-blind safety); no legends — direct labels in the
  first panel, tooltips in HTML.
- Tufte: no top/right spines, range-frame axes, bg `#fffff8`/`#151515`, serif
  titles/labels, sans 11–12 px ticks, no gridlines (hover crosshair in HTML),
  titles assert findings. Log scales on Pareto axes, labelled "log".
- Dark mode first-class in HTML (`prefers-color-scheme` + `data-theme`
  overrides); matplotlib figures light-only (for print).

## Honesty rules (non-negotiable; from the bench docs' own caveats)

1. **10 ms citation floor** (READ_BENCHMARK.md caveat 8): time deltas < 10 ms
   are noise. Pareto panels draw a shaded vertical band around x=1 of width
   `±0.010 / baseline_time_s` (dataset-specific: wide for delft, sliver for
   Zurich). Heatmap cells whose |t − t_baseline| < 10 ms render muted with a
   "≈" prefix. Static figures include the band + a caption note.
2. **Counting-grain marker** (caveat 1): `full-read`, `count`, `bbox-*` compare
   feature-grain vs CityObject-grain formats. These scenario labels carry "†"
   linked to the verbatim caveat. `attr-filter`/`attr-stats`/`project`/
   `id-lookup` are grain-comparable (no marker).
3. **id-lookup bias** (caveat 9): sampled id is table-order-first, favouring
   scanning formats — footnote on the id-lookup scenario wherever shown.
4. **duckdb-parquet**: no `peak_heap_bytes` (blank under heap toggle, marked
   "n/a — out-of-process"); carries ~0.06 s un-subtracted startup overhead —
   footnote + tooltip note.
5. **Compression codec levels are mismatched** — zstd@3 vs gzip@6 vs brotli@1
   (parquet-rs defaults; `crates/cityparquet/src/recipe.rs`). NOT documented in
   bench/README.md. The codec view carries this inline, phrased as sourced from
   implementation defaults, and the section is visually de-emphasized:
   "smallest codec" is not a citable claim.
6. **Round-trip failures and empty inputs are shown, not dropped.** A dataset
   whose compression rows are all `roundtrip_equal=false` renders grayed-out
   with a "roundtrip FAILED — not citable" badge; a header-only CSV becomes an
   explicit empty-panel note. A corpus with no compression run at all (the
   compression benchmark is a separate, slower pass) says so where the panels
   would be, and the compression figure is skipped rather than drawn empty.
7. **Verbatim caveats**: the page quotes every one of READ_BENCHMARK.md's
   fairness caveats (11 when this was written, 18 today) and
   README.md's "Baseline geometry coverage" section verbatim at generation time
   (same policy as the previous page — the page cannot drift from the
   methodology it reports).
8. **Memory metric**: primary = `peak_rss_bytes` ratio (present for every
   format the coordinator measures; platform units cancel in ratios). `peak_heap_bytes` ratio is the
   HTML toggle, annotated: allocator view; FCB streams (tiny heap by design);
   duckdb-parquet absent.
9. Dataset subtitles use only in-repo facts (CityObject count, raw
   CityJSONSeq size). No invented city/LoD descriptions — the repo does not
   document them.

## Data contract — `bench_data.json`

Produced by `prep.py`; consumed by both renderers. All ratios are
`value / baseline_value` within (dataset, scenario_key); baseline = the
`cityjsonseq` row. Missing baseline or missing value ⇒ `null` (renderers show
an explicit gap, never drop silently).

```jsonc
{
  "meta": {
    "baseline": "cityjsonseq",
    "sources": { "read": "...", "sizes": "...", "compression": "..." },
    "caveats_read": ["<verbatim caveat 1>", "..."], // as many as the
    // source lists,
    // numbered 1..n
    "caveats_compression": ["<verbatim baseline-geometry-coverage text>"],
    "codec_level_note": "<the recipe.rs-sourced mismatch note>",
    "citation_floor_s": 0.01,
    "format_axis": [
      "cityparquet-hilbert",
      "citygml",
      "cityjson",
      "cityjsonseq",
      "flatcitybuf",
    ], // = Format::DEFAULT_SET
    "object_grain_formats": [
      "cityparquet",
      "cityparquet-hilbert",
      "cityjson",
      "duckdb-parquet",
    ], // caveat 1's own table
    "feature_grain_formats": [
      "citygml",
      "cityjsonseq",
      "cityjsonseq-gz",
      "flatcitybuf",
    ],
    "excluded_formats": [
      // measured but unplottable: no colour,
      // marker or caption exists for them.
      // Stated in the page's coverage notes,
      // never averaged in, never hidden.
      { "format": "<tag>", "rows": 144, "where": ["read", "sizes"] },
    ],
  },
  "datasets": [
    // ordered by objects desc
    {
      "id": "Zurich",
      "objects": 198699,
      "features": 52834,
      "raw_mb": 247.0,
      "subtitle": "198,699 CityObjects · 247 MB CityJSONSeq",
    },
  ],
  "read": [
    {
      "dataset": "delft",
      "format": "cityparquet",
      "scenario_key": "bbox-5pct",
      "grain_comparable": false, // † scenarios: full-read, count, bbox-*
      "time_s": 0.000975,
      "time_mad_s": 0.000001,
      "heap_b": 358363,
      "rss_b": 9879552,
      "result_count": 2,
      "time_ratio": 0.035,
      "heap_ratio": 0.099,
      "rss_ratio": 0.754,
      "below_floor": false, // |time_s - baseline_time_s| < 0.010
    },
  ],
  "sizes": [
    {
      "dataset": "delft",
      "format": "cityparquet",
      "bytes": 0,
      "frac_of_baseline": 0.38,
    }, // bytes / cityjsonseq bytes; <1 smaller
  ],
  "compression": [
    {
      "dataset": "delft",
      "variant": "cityparquet+gzip",
      "kind": "codec",
      // kind: "default" | "codec" | "rowgroup"  (rg512/rg4096 are not codecs)
      "write_s": 0.179,
      "total_bytes": 2287085,
      "full_scan_s": 0.0104,
      "window_query_s": 0.0101,
      "write_ratio": 1.23,
      "size_ratio": 0.98, // vs the dataset's "cityparquet" default row
      "roundtrip": true,
    },
  ],
  "compression_gaps": [
    // two kinds, both derived from the CSVs themselves:
    { "dataset": "<id>", "issue": "all roundtrip_equal=false (undocumented)" },
    { "dataset": "<id>", "issue": "CSV present but header-only" },
  ],
  "scaling": {
    // One city model cut to N cardinalities: the corpus the CONFIGURATION axes
    // are measured on, because a codec or a row-group size answers "how does
    // this scale", not "how does this compare to Vienna". A separate key, never
    // merged into "read"/"datasets" -- a slice is not a peer of a real city
    // model, and one leaking in grows a synthetic panel onto every grid.
    // Object counts come from the RUN: the slices are named for the cardinality
    // asked for and hold what a strict prefix actually contains (n5000 is 5,001).
    "read": [
      {
        "dataset": "<slice>",
        "objects": 50001,
        "format": "cityparquet-hilbert",
        "scenario_key": "full-read",
        "time_s": 1.9, // absolutes kept: the trend view plots them
        "rss_b": 0,
        "time_ratio": 1.02,
        "rss_ratio": 0.58,
        "below_floor": false,
      },
    ],
    "sizes": [], // slice rows only; the source sweeps a shared directory
    "ordering": [], // source vs hilbert, per slice
    "compression": [], // codec x row-group, per slice, with row_groups_touched
  },
  "ordering": [
    // The row-ordering run, baselined against the SOURCE-ORDER package rather
    // than CityJSONSeq: an ordering run has no cityjsonseq row to divide by.
    // Its corpus is NOT a subset of "datasets" -- it routinely covers datasets
    // the read benchmark never measured -- so each record carries the shape a
    // view needs instead of expecting a "datasets" entry to look it up in.
    {
      "dataset": "<id>",
      "scenario_key": "bbox-5pct",
      "objects": 129738,
      "base_time_s": 0.1039,
      "variant_time_s": 0.0015,
      "base_rss_b": 0,
      "variant_rss_b": 0,
      "time_ratio": 70.79, // source ÷ hilbert; >1 = ordering paid
      "rss_ratio": 1.0,
      "delta_s": 0.1024,
      "below_floor": false,
    },
  ],
}
```

`scenario_key`: `full-read`, `count`, `bbox-1pct`, `bbox-5pct`, `bbox-25pct`
(scenario + `notes` merged), `attr-filter`, `attr-stats`, `id-lookup`,
`project`. A run need not measure every scenario for every dataset — that is a
source fact, rendered as "n/a" per cell and counted in the figures' own
"not measured in this run" footnote, never imputed.

Verbatim caveat texts: prep.py extracts the "## Fairness caveats" section from
`bench/READ_BENCHMARK.md` (lines under that heading until the
next `##`) splitting on the numbered items, and the "## Baseline geometry
coverage" section from `bench/README.md`. Extraction is by
heading match, not hard-coded line numbers.

## View 0 — Overview: format profiles (added 2026-08-04, additive)

Motivation: the per-dataset small multiples answer "what happens on dataset X"
but not the two top-level questions. Chart-type choice follows the data
structure:

- _Few items × few ordered criteria_ → **slopegraph** (parallel log axes).
- _Many repeated observations per category_ → **distribution dot-strips**
  (each dataset an individual dot; median marked; spread = consistency).

Placement: first view section, immediately after "How to read this page".
Section id `view-overview`. All existing views unchanged.

**0a. Trade-off slopegraph — "balance at a glance".** Three parallel vertical
log axes: median time ratio, median peak-RSS ratio, on-disk size (fraction of
baseline; scenario-independent, say so in the axis subtitle). One polyline per
format through its median-across-datasets value; lower = better on every axis;
baseline reference line at 1× spanning all axes. Scenario selector (all 9
keys, † preserved), default `full-read` (the conservative case). Individual
dataset values drawn as faint short ticks on each axis (spread stays visible —
the median hides nothing). Direct labels at the right end (collision-dodged);
no legend. cityparquet/hilbert accent, others gray, global marker shapes.
duckdb-parquet has no size artefact → its line ends at the RSS axis with an
explicit "no artefact" note. Annotate "median of n datasets" (n varies by
scenario coverage). Citation floor: tooltip per format reports how many of its
underlying time deltas are below 10 ms; if ≥ half are, the time axis carries a
"mostly < 10 ms — not citable" note for that scenario.

**0b. Consistency dot-strips — "is the win consistent?"** Nine small-multiple
panels (one per scenario_key, † markers kept; id-lookup footnote kept). Rows =
5 non-baseline formats (baseline is the vertical 1× reference line, not a
row); x = ratio on a shared log scale. One small gray dot per dataset; the
median is a larger marker in the format's global shape/color. `below_floor`
dots render hollow/muted (time metric only). Metric radio: time / RSS / heap
(heap → duckdb-parquet row "n/a — out-of-process"). Row labels are the format
names (left of first column panels). aria-labels state each panel's median
finding.

Both charts read only `bench_data.json` (read ratios + sizes
`frac_of_baseline`); medians computed in the page JS over non-null values.
Static paper figures for View 0 are **not** produced yet (HTML only until
requested).

## The page's sections

The page opens with what a reader looks up before reading anything else, then
the two comparisons, then the trend:

1. **The corpus** — a table, not a chart: which datasets, how many CityObjects,
   and the bytes in each format. The question is a lookup and the answer wants
   exact figures, which is what a table is for.
2. **Read time and peak memory, per dataset** — one chart per dataset, with
   every query a subgroup of format bars inside it and read time beside peak
   memory. Deliberately NOT one mini-plot per (dataset, query, metric): that is
   three hundred fragments carrying three hundred axes, and comparing two of
   them means comparing two rulers. Grouped onto one pair of axes per dataset,
   a format holds the same row position in every query, so reading down a
   column reads that format across all of them. One log scale is shared by
   every panel — a bar means the same thing everywhere — and the bars grow out
   of the 1× rule, per the size grid's reasoning.
3. **Configuration axes** — row ordering across the whole ordering run, then
   codec and row-group size on the scaling corpus. The second is a table: those
   are write-side axes, and the harness reports bytes, write time and row-group
   counts for them but no peak RSS and only two query types, so they cannot take
   the shape used above. `row_groups_touched / row_groups_total` is the honest
   pruning metric — it counts skipping directly, and is immune to the 10 ms floor.
4. **Scaling** — see the ratio-rule exception above.

Then the four corpus-wide views, unchanged:

## The four views

1. **Speed–Memory Pareto grid** (headline), one panel per dataset; x = time_ratio (log),
   y = rss_ratio (log); dot per format; baseline cross at (1,1); quadrant hint
   "below-left = faster AND leaner than CityJSONSeq"; stepped Pareto frontier
   through non-dominated points; citation-floor band per honesty rule 1.
   HTML: scenario selector (all 9 keys) + RSS/heap toggle. Static: two figures
   (`full-read`, `bbox-5pct`), RSS only.
2. **Read speedup heatmap grid**, one panel per dataset; rows = scenario_keys (with †),
   cols = formats; cell = speedup `1/time_ratio`, diverging palette in log2
   space centered 1× (green = faster, red = slower — also encoded by value
   label so color is not the sole channel); in-cell labels "12×"/"0.3×"/"≈".
   Companion `<details>` data table per dataset (accessibility + precision).
3. **On-disk size grid**, one panel per dataset; horizontal bars of
   `frac_of_baseline` sorted ascending, cityparquet accented, value labels
   ("0.38×"). The bars grow OUT OF the 1× baseline on a shared LOG x scale,
   left for smaller and right for larger: the axis spans CityParquet at ~0.3×
   and CityGML at up to ~25× of the same bytes, and a linear 0-to-max scale
   collapses the CityParquet series into a sliver. duckdb-parquet is absent —
   it writes no artefact of its own.
4. **Compression codec grid** (de-emphasized styling), one panel per measured
   dataset; x = write_ratio, y = size_ratio, default variant at (1,1) cross;
   codecs = filled markers, row-group variants = open markers (different
   axis of variation, same plot, distinguished); round-trip failures grayed +
   badged, gaps named in the key. Codec-level caveat inline above the grid.
   Roundtrip status: one sentence + per-dataset ✓/✗ strip, no chart.

Page order: Title (finding-asserting) → How to read this page (conventions,
baseline, floor) → Pareto → Heatmap → Sizes → Compression → Fairness caveats
(verbatim) → Coverage notes. Every view carries an `aria-label` with its key
finding and has a text/table fallback.

## The two print-only figures

The four views above answer "how does the corpus behave". A journal page asks a
narrower question — what happens on THIS dataset, for THIS query, in time and in
memory — and asks it at a size where 21 panels are unreadable. Two figures serve
that, and neither has an HTML counterpart: the page already carries every number
they select from.

5. **`formats`** — one row per dataset, scenarios down the panel, the four
   non-baseline formats per scenario, read time and peak memory side by side.
   Bars grow out of the 1× rule on a LOG axis for the reason the size grid does,
   only more so: within a single scenario the formats span up to five orders of
   magnitude, so a zero-anchored linear bar renders everything but the fastest as
   a sliver. The baseline's own absolute time is printed beside each scenario, so
   a ratio can be read back into seconds and the citation floor is legible in
   context.
6. **`configuration`** — the row-ordering axis: the Hilbert package against the
   same package written in source order, same two metrics, same anchoring. Its
   panels are the dataset where the most scenarios clear the citation floor and
   the one where the largest difference is smallest, because whether ordering is
   measurable at all is a property of the input rather than of the ordering.

Both **derive their panels rather than naming them** — the conventions above
forbid a dataset name in this package, and these two would otherwise smuggle one
in as a "representative" choice. `formats` spreads its panels across the corpus
by CityObject count; `configuration` brackets the axis by floor-clearing count.
Both skip inputs too small to exercise a selective query (the corpus holds a
one-object tile, and every filter on it matches all of it or none), which would
otherwise report fixed open cost as a finding about a format or a configuration.

Both **print what they leave out**: the datasets drawn against the datasets
measured, the windows omitted, and the scenarios a run never measured. A reader
must never have to infer that a sheet is a selection.

## Build layout

```
bench/plot/                 # uv project, shared with readbench_plot
  benchviz/DESIGN.md        # this file
  benchviz/paths.py         # the default input + output locations
  benchviz/prep.py          # CSVs -> bench_data.json (stdlib only)
  benchviz/html.py          # bench_data.json -> bench-summary.html
  benchviz/figures.py       # bench_data.json -> *.{svg,png}
  benchviz/__main__.py      # python -m benchviz [prep|html|figures] [paths]
  tests/test_benchviz.py    # contract + path-flag tests (`just plot-test`)
  tests/fixtures/benchviz/  # pinned real runs: three datasets read +
                            #   compression, two more ordering-only
bench/summary/              # generated: JSON + page + figures (gitignored)
```

- `just plot-pretty [OUT]` runs the three stages; each is also its own
  subcommand, and `--bench-dir` / `--out` / `--data` / `--html` / `--figures`
  move any path. Nothing is written outside the repository by default.
- HTML output is fully self-contained: inline SVG rendered by a small inline
  JS module from embedded JSON; no external requests; works from file://.
- Static figures: `formats`, `configuration`, `pareto-full-read`,
  `pareto-bbox-5pct`, `heatmap`, `sizes`, `compression` as `.svg` + `.png`
  (Typst cannot embed PDF). 300 dpi PNG. The static set is NOT the view set:
  `formats` and `configuration` are print-only, and `compression` and
  `configuration` are each skipped with a printed reason when their run is
  absent.
- **Nothing in a figure is typed by hand about the data.** Every headline
  sentence, count and median is computed from the run being plotted, and the
  comparative words ("faster than", "at parity with") come from the same
  numbers; a format the run did not measure gets no marker slot, no key row and
  no column, and is named in the footer instead. Hand-typed findings held
  exactly until the next benchmark run, which is how the first edition came to
  assert one corpus's medians over another corpus's marks.
- The grids grow to a 5x5 sheet (four columns while the corpus still fits the
  shape they were drawn at) and refuse anything past 24 datasets: past that the
  panels carry neither values nor a pattern, and the answer is a different
  figure. Denser grids scale the panel type down and say on the sheet that the
  panels are to be read as a pattern, with exact values in the HTML tables.
