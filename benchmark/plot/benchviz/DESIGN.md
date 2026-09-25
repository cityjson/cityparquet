# Benchmark figure contract

`benchviz` renders benchmark evidence; it does not run measurements. The canonical
data root is `benchmark/runs/`:

- `formats/results/` holds the format-comparison CSVs and `sizes.csv`.
- `formats/scaling_codec_results/`, `formats/scaling_rowgroup_results/` and
  `formats/scaling_bloom_results/` hold configuration experiments over the
  scaling corpus.
- `databases/results/` holds the database summary CSVs and size summaries.
- `summary/` holds prepared JSON, the self-contained `bench-summary.html`, and
  individual paper figures. `--figures` can export those figures elsewhere.

Run `python -m benchviz summary --data-root ROOT`. A smoke run uses its own
result and summary directories; it must never be combined with a full run.

The static output set is `sizes`, `heatmap`, `codec`, `codec-scaling`,
`rowgroup`, `rowgroup-scaling`, `bloom`, `bloom-scaling`, `bloom-corpus` and
`databases`, each as SVG and 300 dpi PNG. The HTML index
embeds the same SVGs, each followed by the conditions it was measured under
(`meta.conditions`: attribute predicates, achieved window selectivities, the
write baseline, thread configurations), and has no external dependencies.

Format size panels use actual on-disk bytes (a GB unit once a bar clears a
gigabyte, a MB unit otherwise), with a CityJSONSeq ratio in each bar label, and
run from CityGML to CityParquet so the subject is the last bar. Heatmap cells
print the absolute value over its ×ratio to the stated baseline, and colour the
logarithmic ratio;
teal is better, the warm accent is worse, and each metric carries its own colour
scale — a read ratio range cannot bound a write time, which is orders of
magnitude larger, and one shared bound would paint the write column a single
colour. Write metrics use a one-sided ramp because they never beat the streaming
baseline; read metrics stay diverging. The scales are drawn once, beneath the
grid, one per metric. Missing, unsupported, failed, and unverified measurements
remain labelled cells. Both the size and heatmap sheets share the same format
order; the format-comparison heatmap puts the formats across the top and the
queries down the side. CityParquet's Hilbert package is displayed as
**CityParquet** while retaining `cityparquet-hilbert` internally.

Configuration panels use the default CityParquet configuration as baseline and
show the largest scaling dataset plus trends over observed object counts.
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
