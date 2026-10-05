# CityParquet size and configuration benchmark methodology

The **size and configuration** side of the benchmark suite: how many bytes each
format occupies on disk, and how a CityParquet package's reads and size move
with bloom filters, the writer option the suite varies. Its read-side
counterpart, and the cross-format comparison, is
`benchmark/formats/READ_BENCHMARK.md`.

The suite does not time writes. Not every format has a native writer, so a
format's file is produced by converting through a common format, and a
conversion time would not compare like with like. Preparing an artefact
(`readbench_prepare.sh`, `convert-all`, the variant packages the bloom family
builds) is never a measurement.

**The committed evidence is `benchmark/runs/formats/results/`**, where the
`formats` family writes its per-dataset read rows and its package `sizes.csv`,
alongside the bloom family's `scaling_bloom_results/`; both carry a
`MACHINE.md` describing the host they were measured on. What the figures cite
is the ratios within a single directory. Nothing in this document quotes a
number, so the methodology here cannot go stale against a re-run; the CSVs
themselves can.

**The committed evidence was measured on packages that carry bloom filters,
and reports one timing statistic.** Both disclosures are
[`READ_BENCHMARK.md`](READ_BENCHMARK.md)'s fairness caveats 30 and 31, which
is where every family's caveats are kept: `benchviz` renders that one numbered
list onto the summary page, so a caveat written only here would never reach a
reader of the figures.

## Running the suite

Use the root entry points:

```sh
just bench-prep --families formats,sizes,bloom
just bench-run --families formats,sizes,bloom
just bench-summary
```

The `formats` family compares read performance across file formats, and the
`sizes` family records each format's file or package size. The `bloom` family
holds the format fixed and changes one configuration dimension over nested
3DBAG slices and the city datasets. Its read scenarios are the identifier
lookups described under "The bloom family" below.

The primary format configuration is Hilbert-ordered CityParquet, displayed as
**CityParquet** in figures. Internal variant IDs retain the ordering and
configuration information needed to reproduce each configuration. The bloom
figures show the largest measured slice, a separate scaling line chart and the
corpus datasets apart from it.
See [`../README.md`](../README.md) for the experimental matrix and figure list.

## Measurement discipline

- **Sizes are on-disk bytes.** A file's size is its length; a CityParquet
  package's is the sum of every file in its directory
  (`benchmark/scripts/measure_sizes.py` for the format comparison, the
  coordinator's `sizes.csv` for the bloom family).
- **Arithmetic mean of `repeat` read samples** — default 7 — reported at
  6-decimal precision. The dispersion column is the **population standard
  deviation** (`time_std_s`): the warm repeats are the whole measured set, not
  a draw used to infer a wider one. `time_s` is the same statistic
  `benchmark/databases` reports, and `READ_BENCHMARK.md` states the same
  contract.
- **Sub-10 ms deltas are noise** at these repeat counts and are not findings on
  their own — the same floor `benchmark/formats/READ_BENCHMARK.md` applies.

## The bloom family

`just bloom-bench` (via `just bench-run --families bloom`) builds two packages
from each input, untimed — `cityparquet`, which carries bloom filters, and
`cityparquet+nobloom`, which carries none — and times `id-lookup` (`id-50pct`,
`id-miss`) and `feature-lookup` (`feature-50pct`, `feature-miss`) against
both. Package bytes go to `sizes.csv`. Every lookup row carries `row_groups_total`, `bloom_pruned` and `filter_bytes` (the
bitset bytes of the filters examined). The scaling curves are drawn from the
nested 3DBAG slices alone; the corpus datasets are other city models, not
larger slices, and are drawn apart, per dataset, in `bloom-corpus`.

Over HTTP, `just bloom-bench-http FOLDER BASE_URL` reads — never builds — the
two packages a local `bloom-bench` run left in the prepared directory, once that
directory is uploaded to `BASE_URL` (`benchmark/scripts/readbench_upload.md`).
Its rows add `bytes_read` and `http_requests`; the results go to
`scaling_bloom_http_results/` and are not part of `bench-run` or the rendered
summary. They are a snapshot of one network path at one time.

The caveats that travel with every one of its numbers are
[`READ_BENCHMARK.md`](READ_BENCHMARK.md)'s fairness caveats **24 to 29** —
the row-group scale a slice has to reach before its pruning means anything, a
positive not being a match, the twice-read footer, the single-table
restriction, what the `feature-*` probes are, and requests being logical. They
live there, not here, because that numbered list is the one `benchviz` renders
onto the summary page beside the figures.

## Reproduce

```sh
just bench-prep --families bloom
just bench-run --families bloom
just bench-summary
```

Add `--smoke` for a small pipeline check. Full experiments use all configured
scaling slices, cut without LoD 1.2 so that every format holds the same
geometry (`READ_BENCHMARK.md`, Caveat 14); actual counts are recorded because
feature boundaries can cross a nominal target. Keep machine metadata, source identity, software
revision, query parameters and repetition settings alongside the results.
Prepared data and result directories have separate responsibilities: preparing
an artefact is never a measurement.
