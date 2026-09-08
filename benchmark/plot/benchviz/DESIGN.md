# Benchmark figure contract

`benchviz` renders benchmark evidence; it does not run measurements. The canonical
data root is `benchmark/runs/`:

- `formats/results/` holds the format-comparison CSVs and `sizes.csv`.
- `formats/scaling_codec_results/` and `formats/scaling_rowgroup_results/` hold
  configuration experiments over the scaling corpus.
- `databases/results/` holds the database summary CSVs and size summaries.
- `summary/` holds prepared JSON, the self-contained `bench-summary.html`, and
  individual paper figures. `--figures` can export those figures elsewhere.

Run `python -m benchviz summary --data-root ROOT`. A smoke run uses its own
result and summary directories; it must never be combined with a full run.

The static output set is `sizes`, `heatmap`, `codec`, `codec-scaling`,
`rowgroup`, `rowgroup-scaling`, and `databases`, each as SVG and 300 dpi PNG.
The HTML index embeds the same SVGs and has no external dependencies.

Format size panels use actual on-disk bytes, with a CityJSONSeq ratio in each
bar label. Heatmap cells print actual values and colour the logarithmic ratio to
the stated baseline; lower values are green. Missing, unsupported, failed, and
unverified measurements remain labelled cells. CityParquet's Hilbert package is
displayed as **CityParquet** while retaining `cityparquet-hilbert` internally.

Configuration panels use the default CityParquet configuration as baseline and
show the largest scaling dataset plus trends over observed object counts.
Database panels use 3DCityDB as baseline; storage includes indexes and memory
means peak execution-process RSS, not total database-server memory.

Read comparisons retain the counting-grain caveat for full-read and bbox
queries. Timings at or below 10 ms are shown but must not support a ranking
claim. Repetitions and raw result artefacts remain the evidence for uncertainty.
