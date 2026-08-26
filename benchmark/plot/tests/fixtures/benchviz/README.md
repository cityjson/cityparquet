# benchviz test fixture — three datasets of a real run

Result CSVs extracted verbatim from commit `cc7f2f7`
("bench(corpus): full-corpus read/size/compression results + report"), the
full-corpus run whose numbers the first `bench-summary.html` reported. Nothing
here is hand-written: they are measured rows, trimmed to three datasets.

Why a pinned copy rather than the live `benchmark/formats/read_results`: a benchmark run
replaces those CSVs with whatever corpus, formats and columns it measured, so a
test reading them asserts something different after every run. The three
datasets kept here are the ones that exercise the awkward cases —

- **Zurich** — largest object count, so it must sort first;
- **delft** — an ordinary, complete dataset;
- **Ingolstadt** — every compression row has `roundtrip_equal=false`;
- **Railway** (compression only) — a header-only CSV.

`ordering_results/` holds two datasets from the row-ordering run instead, and
deliberately shares none of its names with `read_results/`: the ordering
benchmark is a separate pass with its own corpus, and the views must not assume
a `datasets` entry exists for a dataset only it measured. One of the two has
scenarios that clear the 10 ms citation floor and the other has none, which is
the contrast the configuration figure is built to show.

The methodology documents are deliberately NOT copied here. `tests/test_benchviz.py`
takes `READ_BENCHMARK.md` and `README.md` from the live `benchmark/formats/` directory,
because the page quotes their caveats verbatim and extraction is supposed to
fail when they change shape.
