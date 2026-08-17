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
- **Per-dataset small multiples** at shared scale, ordered by CityObject count
  descending — for the corpus this was written for: Zurich, NYC, 9-284-556,
  delft, 3DBAG, Vienna, Rotterdam, Ingolstadt, Montreal, Railway, lod3_railway.
  The order is computed from the data, so a later run's corpus orders itself.
- **Color/markers**: accent `#e41a1c` (light) / `#fc8d62` (dark) for
  `cityparquet` (filled circle); same accent, open circle for
  `cityparquet-hilbert`; gray `#666`/`#999` for others with distinct shapes:
  cityjsonseq = ×/cross at (1,1) (it IS the baseline), cityjsonseq-gz = square,
  flatcitybuf = triangle, duckdb-parquet = diamond. Shapes carry identity
  (color-blind safety); no legends — direct labels in the first panel, tooltips
  in HTML.
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
8. **Memory metric**: primary = `peak_rss_bytes` ratio (present for all 6
   formats; platform units cancel in ratios). `peak_heap_bytes` ratio is the
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
    "sources": {"read": "...", "sizes": "...", "compression": "..."},
    "caveats_read": ["<verbatim caveat 1>", "..."],          // as many as the
                                                             // source lists,
                                                             // numbered 1..n
    "caveats_compression": ["<verbatim baseline-geometry-coverage text>"],
    "codec_level_note": "<the recipe.rs-sourced mismatch note>",
    "citation_floor_s": 0.010,
    "excluded_formats": [               // measured but unplottable: no colour,
                                        // marker or caption exists for them.
                                        // Stated in the page's coverage notes,
                                        // never averaged in, never hidden.
      {"format": "citygml", "rows": 144, "where": ["read", "sizes"]}
    ]
  },
  "datasets": [                       // ordered by objects desc
    {"id": "Zurich", "objects": 198699, "features": 52834,
     "raw_mb": 247.0, "subtitle": "198,699 CityObjects · 247 MB CityJSONSeq"}
  ],
  "read": [
    {"dataset": "delft", "format": "cityparquet", "scenario_key": "bbox-5pct",
     "grain_comparable": false,       // † scenarios: full-read, count, bbox-*
     "time_s": 0.000975, "time_mad_s": 0.000001,
     "heap_b": 358363, "rss_b": 9879552, "result_count": 2,
     "time_ratio": 0.035, "heap_ratio": 0.099, "rss_ratio": 0.754,
     "below_floor": false             // |time_s - baseline_time_s| < 0.010
    }
  ],
  "sizes": [
    {"dataset": "delft", "format": "cityparquet", "bytes": 0,
     "frac_of_baseline": 0.38}        // bytes / cityjsonseq bytes; <1 smaller
  ],
  "compression": [
    {"dataset": "delft", "variant": "cityparquet+gzip", "kind": "codec",
     // kind: "default" | "codec" | "rowgroup"  (rg512/rg4096 are not codecs)
     "write_s": 0.179, "total_bytes": 2287085,
     "full_scan_s": 0.0104, "window_query_s": 0.0101,
     "write_ratio": 1.23, "size_ratio": 0.98,   // vs the dataset's "cityparquet" default row
     "roundtrip": true}
  ],
  "compression_gaps": [
    {"dataset": "Ingolstadt", "issue": "all roundtrip_equal=false (undocumented)"},
    {"dataset": "Railway", "issue": "CSV present but header-only"},
    {"dataset": "Rotterdam", "issue": "excluded per CORPUS_REPORT (material index error)"}
  ]
}
```

`scenario_key`: `full-read`, `count`, `bbox-1pct`, `bbox-5pct`, `bbox-25pct`
(scenario + `notes` merged), `attr-filter`, `attr-stats`, `id-lookup`,
`project`. `attr-stats` absent for lod3_railway, Montreal, NYC, Railway —
that's a source fact, render as "n/a".

Verbatim caveat texts: prep.py extracts the "## Fairness caveats" section from
`bench/READ_BENCHMARK.md` (lines under that heading until the
next `##`) splitting on the numbered items, and the "## Baseline geometry
coverage" section from `bench/README.md`. Extraction is by
heading match, not hard-coded line numbers.

## View 0 — Overview: format profiles (added 2026-08-04, additive)

Motivation: the per-dataset small multiples answer "what happens on dataset X"
but not the two top-level questions. Chart-type choice follows the data
structure:

- *Few items × few ordered criteria* → **slopegraph** (parallel log axes).
- *Many repeated observations per category* → **distribution dot-strips**
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

## The four views

1. **Speed–Memory Pareto grid** (headline). 11 panels; x = time_ratio (log),
   y = rss_ratio (log); dot per format; baseline cross at (1,1); quadrant hint
   "below-left = faster AND leaner than CityJSONSeq"; stepped Pareto frontier
   through non-dominated points; citation-floor band per honesty rule 1.
   HTML: scenario selector (all 9 keys) + RSS/heap toggle. Static: two figures
   (`full-read`, `bbox-5pct`), RSS only.
2. **Read speedup heatmap grid**. 11 panels; rows = scenario_keys (with †),
   cols = formats; cell = speedup `1/time_ratio`, diverging palette in log2
   space centered 1× (green = faster, red = slower — also encoded by value
   label so color is not the sole channel); in-cell labels "12×"/"0.3×"/"≈".
   Companion `<details>` data table per dataset (accessibility + precision).
3. **On-disk size grid**. 11 panels; horizontal bars of `frac_of_baseline`
   sorted ascending, reference line at 1×, cityparquet accented, shared x
   scale, value labels ("0.38×"). 5 formats (no duckdb-parquet — no artefact).
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
  tests/fixtures/benchviz/  # three datasets of a pinned real run
bench/summary/              # generated: JSON + page + figures (gitignored)
```

- `just plot-pretty [OUT]` runs the three stages; each is also its own
  subcommand, and `--bench-dir` / `--out` / `--data` / `--html` / `--figures`
  move any path. Nothing is written outside the repository by default.
- HTML output is fully self-contained: inline SVG rendered by a small inline
  JS module from embedded JSON; no external requests; works from file://.
- Static figures: `pareto-full-read`, `pareto-bbox-5pct`, `heatmap`, `sizes`,
  `compression` as `.svg` + `.png` (Typst cannot embed PDF). 300 dpi PNG.
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
