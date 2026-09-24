# CityParquet write and configuration benchmark methodology

The **write** side of the benchmark suite: how long a CityParquet package takes
to write, how many bytes it occupies, and how those two move with the writer's
own knobs — codec, row-group size, bloom filters. Its read-side counterpart, and
the cross-format comparison, is `benchmark/formats/READ_BENCHMARK.md`.

**The committed write-side evidence is `read_results/`**, where the `formats`
family writes its per-dataset read and `write` rows and its package `sizes.csv`,
alongside the configuration axes under `scaling_codec_results/` and
`scaling_rowgroup_results/` and the bloom family's `scaling_bloom_results/`,
which carry a `MACHINE.md` describing the host they
were measured on. The `read_results/` runs carry no such record, so whether they
ran on the same host cannot be established from what is committed. Absolute
times are therefore not comparable across a directory that has a machine record
and one that does not; what the figures cite is the ratios within a single
directory. Nothing in this document quotes a number, so the methodology here
cannot go stale against a re-run; the CSVs themselves can, and two caveats
already apply.

**The committed `read_results/` CSVs predate the typed appearance columns.**
Their byte counts were measured while `material_lod*` / `texture_lod*` were JSON
text cells; those columns are typed Arrow/Parquet `MAP`s, which the writer
leaves at parquet's own defaults for dictionary encoding and statistics, so the
committed bytes do not describe the current writer until the family is re-run.
The codec and row-group runs postdate that change: each directory's `MACHINE.md`
names the commit, and the 3DBAG slices carry no appearance data, so those
columns are empty in every package they measured.

**The committed codec and row-group CSVs predate bloom filters, and so does
every other committed CSV, `read_results/` included.** Both disclosures are
[`READ_BENCHMARK.md`](READ_BENCHMARK.md)'s fairness caveats 30 and 31, which is
where every family's caveats are kept: `benchviz` renders that one numbered
list onto the summary page, so a caveat written only here would never reach a
reader of the figures.

## Running the suite

Use the root entry points:

```sh
just bench-prep --families formats,codec,rowgroup,bloom
just bench-run --families formats,codec,rowgroup,bloom
just bench-summary
```

The `formats` family compares write and read performance across file formats.
The `codec` and `rowgroup` families hold the format fixed and change one
configuration dimension over nested 3DBAG slices. Their write and read rows
share a dataset/configuration identity; write measurements are independent of
the subsequent read queries. Read scenarios for configuration experiments are
full read, the bbox windows and the middle-position ID lookup.

The primary format configuration is Hilbert-ordered CityParquet, displayed as
**CityParquet** in figures. Internal variant IDs retain the ordering and codec
information needed to reproduce each configuration. The codec and row-group
figures show the largest measured slice and a separate scaling line chart.
See [`../README.md`](../README.md) for the experimental matrix and figure list.

## Measurement discipline

- **Arithmetic mean of `repeat` samples** — default 7 for reads and 3 for
  writes (`--write-repeat`) — reported at 6-decimal precision, alongside the
  byte counts. The dispersion column is the **population standard deviation**
  (`time_std_s`): the warm repeats are the whole measured set, not a draw used
  to infer a wider one. `time_s` is the same statistic
  `benchmark/databases` reports, and `READ_BENCHMARK.md` states the same
  contract.
- **Every write sample times a CLEAN write.** A fresh, empty directory is
  created — and later removed — _outside_ the timed window for every repeat, so
  no sample pays for unlinking the previous one's files.
- **Sub-10 ms deltas are noise** at these repeat counts and are not findings on
  their own — the same floor `benchmark/formats/READ_BENCHMARK.md` applies.

## The codec levels are NOT matched

`just codec-bench` sweeps zstd at levels **1, 3, 9 and 19**. Gzip and brotli
stay at the `parquet-rs` defaults carried by `crates/core/src/recipe.rs` —
**gzip at level 6, brotli at level 1** — and are reference points, not swept
axes. **"The smallest codec" remains a non-citable claim across codecs**: gzip,
brotli and zstd are different implementations at different effort levels, so a
byte or time difference between them says nothing about the codec family in
general. **"Zstd level N versus level M" is a measured, citable claim**: it is
the same codec swept deliberately, and `just codec-bench`'s CSVs are what back
it. The summary page states this inline above its codec panels and cites this
section.

## The retired `duckdb-copy` writer baseline

The writer experiment that appended `duckdb-copy` / `duckdb-copy-zstd` rows is
out of the suite, and its rows with it. It read through the community `cityjson`
extension's `read_cityjson`/`read_cityjsonseq`, which does not populate every
geometry column it declares — at the limit it writes attribute columns and no
geometry at all — so its rows measured a different amount of geometry from every
CityParquet variant and are **not comparable** to any of them. Keep the
knowledge: a `duckdb-copy` byte or write-time figure is never a citable
cross-encoder comparison without saying which geometry each side actually
wrote. `READ_BENCHMARK.md`'s `duckdb-parquet` rows are a different thing — DuckDB
`read_parquet()` straight over a CityParquet package, full geometry, every LoD
column — and that file's fairness caveat 5 keeps the two apart.

## The bloom family

`just bloom-bench` (via `just bench-run --families bloom`) writes each input
twice — `cityparquet`, which carries bloom filters, and `cityparquet+nobloom`,
which carries none — and times `id-lookup` (`id-50pct`, `id-miss`) and
`feature-lookup` (`feature-50pct`, `feature-miss`) against both. Package bytes
go to `sizes.csv`; the write rows carry write time and peak RSS, where the
filters' memory shows (every filter is held until its file is closed). Every
lookup row carries `row_groups_total`, `bloom_pruned` and `filter_bytes` (the
bitset bytes of the filters examined). The scaling curves are drawn from the
nested 3DBAG slices alone; the corpus datasets are other city models, not
larger slices, and are drawn apart, per dataset, in `bloom-corpus`.

Over HTTP, `just bloom-bench-http FOLDER BASE_URL` reads — never writes — the
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
just bench-prep --families codec,rowgroup,bloom
just bench-run --families codec,rowgroup,bloom
just bench-summary
```

Add `--smoke` for a small pipeline check. Full experiments use all configured
scaling slices; actual counts are recorded because feature boundaries can
cross a nominal target. Keep machine metadata, source identity, software
revision, query parameters and repetition settings alongside the results.
Prepared data and result directories have separate responsibilities: preparing
an artefact does not constitute a timed write measurement.
