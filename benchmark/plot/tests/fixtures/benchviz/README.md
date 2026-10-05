# benchviz test fixture — three datasets of a real run

Result CSVs extracted verbatim from commit `cc7f2f7`
("bench(corpus): full-corpus read/size/compression results + report"), the
full-corpus run whose numbers the first `bench-summary.html` reported. Nothing
here is hand-written: they are measured rows, trimmed to three datasets.

Why a pinned copy rather than the live `benchmark/formats/read_results`: a benchmark run
replaces those CSVs with whatever corpus, formats and columns it measured, so a
test reading them asserts something different after every run. The three
datasets kept here span the range the views have to handle —

- **Zurich** — 198,699 objects, the largest, so it must sort first;
- **delft** — 2,231 objects: an ordinary, complete dataset in the middle;
- **Ingolstadt** — 379 objects, the smallest, so it sorts last, and 18 of its
  53 read rows fall inside the citation floor.

`scaling_bloom_results/` is the one exception to "nothing is edited by hand":
its `delft.csv`, `delft.csv.params.json` (the `cp_object_total` the axis takes
its object count from) and `sizes.csv` are renderer fixture values, NOT measurements,
written in the coordinator's 16-column `--variants` shape (`cityparquet`
against `cityparquet+nobloom`, `id-lookup` and `feature-lookup` at their middle
and miss probes, with the three lookup counters) so the bloom axis's keying and
counter pass-through are exercised on rows of the right shape. One dataset
only, and `delft` is a corpus model rather than one of the nested 3DBAG
slices, so `prep` classifies its rows `series: corpus`: the `bloom` main panel
draws from it, and `bloom-scaling` renders its "no 3DBAG scaling slice was
measured" placeholder instead of a trend strip. No measured
`bloom` run is committed under `benchmark/formats/`; these stand in for one,
and are to be replaced by a measured run's rows rather than kept beside them.

The methodology document is deliberately NOT copied here. `fixture_bench` in
`tests/test_benchviz.py` takes `READ_BENCHMARK.md` from the live
`benchmark/formats/` directory because the page quotes its fairness caveats
verbatim and the extraction is supposed to fail when that document changes
shape.
