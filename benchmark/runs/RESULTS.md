# September 2026 benchmark evidence

Format, file-size, bloom-filter and database measurements from the run of
23 September 2026 on the fairness-fixed harness, with raw samples, query
parameters, run manifests, the machine record and the rendered report.
Inputs and prepared packages are not tracked. This evidence describes an
earlier corpus and harness, and all of it is to be re-measured: the corpus
has since gained Tokyo (Chiyoda) and Montréal, which have no results yet; the
committed 3DBAG results were measured on slices that still carried LoD 1.2,
which the slices no longer do; and every `bbox-*` row predates the axis-order
fix, the tie-free windows and the cross-format consistency check
(`benchmark/formats/READ_BENCHMARK.md`, Caveats 1, 2, 21 and 22). The 11 and 12 September
evidence this replaces is in git history (`5b80835` and before) and was
measured on a harness with the defects `benchmark/formats/READ_BENCHMARK.md`
Caveats 32–35 and `benchmark/databases/README.md` Caveat 21 describe; do not
mix the two.

## What was measured

- **Formats and sizes** (`formats/results/`): the five city datasets and the
  1,000,001-object 3DBAG slice, every artefact written by the chain-3
  preparation (CityParquet packages carry bloom filters; `MACHINE.md`
  records the host, the commit and the tool versions). Seven read
  repetitions.
- **Bloom** (`formats/bloom_results/`): the 1,000,001-object 3DBAG slice
  and the five city datasets, `cityparquet` against `cityparquet+nobloom`:
  lookup reads and package sizes. These two packages were written in source
  order; the present harness writes every bloom package in Hilbert order.
- **Databases** (`databases/results/`): the 1,000,001-object slice, both
  thread configurations (`threads=single` is the primary figure), the four
  write scenarios, the 0.1 % explained-deviation tolerance. The DuckDB
  system reads the Hilbert package.

`time_mean_s` in every CSV above is the arithmetic mean of the warm samples
and `time_std_s` their population standard deviation. The run itself reported a
median with `time_mad_s`; on 25 September both columns were recomputed from
the committed raw samples (`*.csv.samples.json`, `raw_time_samples_s`)
without re-measuring, every other column unchanged.

The median, minimum, maximum and quartile columns (`time_median_s`,
`time_min_s`, `time_max_s`, `time_q1_s`, `time_q3_s`) of the format and bloom
CSVs (`formats/results/*.csv`, `formats/bloom_results/*.csv`; not
`sizes.csv`) and of the database CSV (`databases/results/3dbag_n1000000.csv`)
were computed from the same committed raw samples by
`benchmark/scripts/migrate_timing_columns.py`, which also checked that every
recomputed mean and standard deviation equals the committed value exactly. In
the database CSV, `time_s` became `time_mean_s` and `server_time_s` became
`server_time_mean_s`, and the parallel `server_time_*` block was computed the
same way from `raw_server_time_samples_s`. No existing value changed.

The run also timed writes. The suite no longer measures them, so the
`write` rows were removed from the format and bloom CSVs and the bloom
samples, and the format family's `*.write.samples.csv` files were deleted;
every read row, read sample and size is unchanged.

The run also measured what the suite no longer has: the bloom axis on six
smaller 3DBAG slices (1,000 to 500,000 objects), whose files and size rows
were deleted, and a DuckDB configuration over the source-order package,
whose six `duckdb-cityparquet-source` rows were deleted from the database
CSV. The `count-mismatch` note on each remaining `bbox-query` row still
lists that configuration's count beside the other systems', as the run
wrote it; its count equals `duckdb-cityparquet`'s, so the recorded spread
holds without it. The format CSVs, their samples and `sizes.csv` carried the
Hilbert-ordered package as `cityparquet-hilbert`; it is relabelled
`cityparquet`, the benchmark's one CityParquet format, with no value
changed. Both `sizes.csv` files hold `dataset,format,bytes,mb_decimal`. The runs
wrote an `mb` column in units of 2^20 bytes and ratio columns against
CityJSONSeq; the ratio columns are dropped and `mb_decimal` is recomputed as
bytes / 10^6, with every byte value as the run wrote it.

The `*.run.json` manifests and the database `3dbag_n1000000.manifest.json`
record the run as it was made, and stay as that record: the run manifests'
`write_repeat`, `fixed_configuration` (which names the format by its old
label, `cityparquet-hilbert`), source paths (`data/scaling/`), result paths
(`formats/scaling_bloom_results/`) and `files_sha256` (the CSVs as the run
wrote them, and the deleted write-samples files), and the database
manifest's `duckdb-cityparquet-source` entries, no longer match the
committed files.
`EVIDENCE_SHA256SUMS` is current.

## Coverage and limitations

- The database CSV's memory column, `peak_rss_bytes`, holds each executing
  process's **resident set size** (`memory-scope: duckdb-process-rss` and
  `postgresql-backend-rss` in `notes`), which for the PostgreSQL backends
  is dominated by the shared buffer pool. It is **not** the working memory
  (`peak_working_mem_bytes`, summed `RssAnon`) the current database harness
  reports, and the summary labels it as process RSS; the two must not be
  compared.
- The 1M slice's **CityJSON read rows were measured one scenario per run**
  on 24 September: the whole-document parse (about 35 GB resident) was
  refused three times by the host's strict memory-overcommit limit while
  other users' processes held the commit budget, and the coordinator
  truncates its CSV per run, so each scenario ran as its own invocation
  with the identical parameter sidecar and the rows were merged into
  `formats/results/3dbag_n1000000.csv` (the run manifest says so). Same
  binary, same artefacts, same host; timings are single-scenario runs on a
  shared machine and carry the usual noise floor.
- The database run exited non-zero for two rows only: `append-object` on
  the two CityParquet tags errors with a DuckDB `BinderException`, because
  the extension types an empty `material_lod*` column as `VARCHAR` while the
  package declares a `MAP` (`benchmark/databases/README.md`, write tier).
  Every other row is `ok` or `ok-deviation` (the spatial counts differ by at
  most 0.02 %, with the decomposition in `notes`).
- The CityParquet `full-read` row measures this repository's reader
  rebuilding a CityJSON object per row (Caveat 3); it is slower than the
  CityJSONSeq parse on the 1M slice for that reason.
- Missing coverage must not be read as zero cost. Read the CSVs with their
  parameter sidecars, run manifests and the README caveats before citing a
  comparison.

`summary/full/index.html` is the self-contained report; the figures are in
`summary/full/figures/`. `EVIDENCE_SHA256SUMS` verifies the saved
measurement and report files; it is not a replacement for run provenance.
