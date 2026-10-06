# Benchmark figure contract

`benchviz` renders benchmark evidence; it does not run measurements. The canonical
data root is `benchmark/runs/`:

- `formats/results/` holds the format-comparison CSVs and `sizes.csv`.
- `formats/bloom_results/` holds the bloom-filter configuration experiment
  over the slice dataset and the corpus datasets.
- `databases/results/` holds the database summary CSVs and size summaries.
- `summary/` holds prepared JSON, the self-contained `bench-summary.html`, and
  individual paper figures. `--figures` can export those figures elsewhere.

Run `python -m benchviz summary --data-root ROOT`. A smoke run uses its own
result and summary directories; it must never be combined with a full run.

The static output set is `formats/sizes`, `formats/<dataset>/time` and
`formats/<dataset>/rss` for every dataset id in the results, `bloom`,
`bloom-corpus` and `databases`, each as SVG and 300 dpi PNG,
plus the format comparison's ratio tables `formats/size_factors.csv`,
`formats/size_extremes.csv` and `formats/query_factors.csv`. `--figures DIR`
moves the whole tree. The HTML index embeds the same SVGs, each followed by the
conditions it was measured under (`meta.conditions`: attribute predicates,
achieved window selectivities, thread configurations), and the three tables; it
has no external dependencies.

The format comparison's baseline is CityGML (`meta.baseline`). Every relative
value is a **factor**, CityGML's value divided by the format's — read time,
read peak RSS and on-disk bytes alike — so a larger factor is better and a
sentence such as "CityParquet is 5× faster than CityGML" reads straight off the
figure and the tables. A cell whose row is absent, or whose `notes` report a
skip, an error or a failed cross-format check, carries no value; a format cell
whose CityGML cell is unavailable keeps its own value and its factor is
unavailable (`× n/a` in a figure, an empty cell with a `note` in a table) —
never zero, and never a factor against another format. Nothing names a
dataset: the figures and tables follow the results.

Format size panels use actual on-disk bytes (a GB unit once a bar clears a
gigabyte, a MB unit otherwise), with the factor against CityGML in each bar
label, and run from CityGML to CityParquet so the subject is the last bar. The
per-dataset heatmaps put the formats across the top and the queries down the
side; each cell prints the absolute value over its factor and colours the
logarithmic factor — teal is better than CityGML, the warm accent worse. Each
metric's colour bound is shared across the datasets, so a colour means the same
factor in every figure, and saturates at 1024× either way; the printed factor
carries the precision. Missing, unsupported, failed and unverified measurements
remain labelled cells. A slice's title names the size it was cut to; its exact
CityObject count, which can exceed that because a feature is indivisible, is
stated beneath the figure. The `cityparquet` format is the Hilbert-ordered
package, displayed as **CityParquet**.

Configuration panels use the default CityParquet configuration as baseline.
The `bloom` figure shows the manifest's slice dataset (`[suite] slice_dataset`,
carried as `meta.slice_dataset`) and is a labelled placeholder when the run did
not measure it; `bloom-corpus` shows every other measured dataset, and is not
written when there is none. The page reports a manifest `corpus` or `slice`
dataset without format results as a missing section.
Database panels use 3DCityDB as baseline; storage includes indexes and memory
means peak execution-process RSS, not total database-server memory. Every read
cell is keyed by system, scenario and thread configuration: `threads=single`
is the primary column, `threads=parallel` a separately labelled second column,
and no ratio crosses the two. `ok-deviation` cells are citable and carry a
`*n` marker whose footnote quotes the count decomposition. The write tier
shares the `databases` figure: its four rows sit below the reads, under a rule
and a "write tier" label, in the `threads=single` panels only (it runs once;
the `threads=parallel` cells say `n/a`), coloured by ratio to 3DCityDB like
the reads. Its footnote keeps Caveat 19 — different operations, not one scale
— with the area expressions and the rows each importer added. The
`duckdb-cityparquet-writeback` tag has no column of its own: each
CityParquet (DuckDB) write cell stacks the in-engine value (upper line and
half) over the value with package write-back (lower line and half), each with
its own ratio and colour.
