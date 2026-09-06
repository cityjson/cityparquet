# benchviz test fixture — three datasets of a real run

Result CSVs extracted verbatim from commit `cc7f2f7`
("bench(corpus): full-corpus read/size/compression results + report"), the
full-corpus run whose numbers the first `bench-summary.html` reported. Nothing
here is hand-written: they are measured rows, trimmed to three datasets.

Why a pinned copy rather than the live `benchmark/formats/read_results`: a benchmark run
replaces those CSVs with whatever corpus, formats and columns it measured, so a
test reading them asserts something different after every run. The three
datasets kept here span the range the views have to handle —

- **Zurich** — 198,699 objects, the largest, so it must sort first, and the one
  dataset whose scenarios almost all clear the 10 ms citation floor;
- **delft** — 2,231 objects: an ordinary, complete dataset in the middle;
- **Ingolstadt** — 379 objects, the smallest, so it sorts last, and 18 of its
  53 read rows fall inside the citation floor — the muted, "≈"-prefixed cells
  and the widest noise band on the Pareto panels.

`ordering_results/` holds two datasets from the row-ordering run instead, and
deliberately shares none of its names with `read_results/`: the ordering
benchmark is a separate pass with its own corpus, and the views must not assume
a `datasets` entry exists for a dataset only it measured. One of the two has
scenarios that clear the 10 ms citation floor and the other has none, which is
the contrast the configuration figure is built to show.

`scaling_codec_results/` and `scaling_rowgroup_results/` each hold one
`--variants` run of the coordinator over the `delft.city.jsonl` fixture
(`--repeat 2 --write-repeat 2 --scenarios full-read,bbox-query`), produced
by the command in `ai/design-notes/plans/2026-09-05-configuration-axes-benchmark.md`,
Task 5. Three variants each (`cityparquet` plus `+zstd1`/`+lz4`, and
`cityparquet` plus `+rg512`/`+rg2048`), so the loader's baseline, ratio
direction, variant order and `sizes.csv` join are all exercised on measured
rows. One slice only: the trend strip is drawn from one point, which is a
valid degenerate case, and nothing here is edited by hand.

The methodology documents are deliberately NOT copied here. `_bench_dir` in
`tests/test_benchviz.py` takes them from the live `benchmark/formats/`
directory: `READ_BENCHMARK.md` because the page quotes its fairness caveats
verbatim and the extraction is supposed to fail when that document changes
shape, and `README.md` for no remaining reason — nothing in `benchviz` reads it
since the compression axis went, and the copy stays because
`prep.Inputs` names one `benchmark/formats/` directory and a faithful stand-in
for it costs a line.
