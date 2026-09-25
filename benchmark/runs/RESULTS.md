# September 2026 benchmark evidence

Format, file-size, bloom-filter and database measurements from the run of
23 September 2026 on the fairness-fixed harness, with raw samples, query
parameters, run manifests, the machine record and the rendered report.
Inputs and prepared packages are not tracked. The 11 and 12 September
evidence this replaces is in git history (`5b80835` and before) and was
measured on a harness with the defects `benchmark/formats/READ_BENCHMARK.md`
Caveats 32–35 and `benchmark/databases/README.md` Caveat 21 describe; do not
mix the two.

## What was measured

- **Formats and sizes** (`formats/results/`): the five city datasets and the
  1,000,001-object 3DBAG slice, every artefact written by the chain-3
  preparation (CityParquet packages carry bloom filters; `MACHINE.md`
  records the host, the commit and the tool versions). Seven read and three
  write repetitions.
- **Bloom** (`formats/scaling_bloom_results/`): the seven 3DBAG slices and
  the five city datasets, `cityparquet` against `cityparquet+nobloom`.
- **Databases** (`databases/results/`): the 1,000,001-object slice, both
  thread configurations (`threads=single` is the primary figure), the four
  write scenarios, the 0.1 % explained-deviation tolerance. The DuckDB
  system reads the Hilbert package.
- **Codec and row-group results are unchanged** from the 9 September run
  and were measured on packages without bloom filters; the author decided
  not to re-run them, as bloom filters are not part of those axes. They
  still report a median `time_s` with `time_mad_s` and are labelled by
  their directories' `LEGACY.md`; the summary lists them as gaps.

`time_s` in every CSV above is the arithmetic mean of the warm samples and
`time_std_s` their population standard deviation. The run itself reported a
median with `time_mad_s`; on 25 September both columns were recomputed from
the committed raw samples (`*.csv.samples.json`, `*.write.samples.csv`,
`raw_time_samples_s`) without re-measuring, every other column unchanged.
Write rows from `format_write.py` are recomputed from samples rounded to
six decimals, so their mean can differ from one computed at measurement
time in the last digit. The `files_sha256` in each `*.run.json` records the
CSV as the run wrote it and no longer matches the recomputed file;
`EVIDENCE_SHA256SUMS` is current.

## Coverage and limitations

- The 1M slice's **CityJSON read rows were measured one scenario per run**
  on 24 September: the whole-document parse (about 35 GB resident) was
  refused three times by the host's strict memory-overcommit limit while
  other users' processes held the commit budget, and the coordinator
  truncates its CSV per run, so each scenario ran as its own invocation
  with the identical parameter sidecar and the rows were merged into
  `formats/results/3dbag_n1000000.csv` (the run manifest says so). Same
  binary, same artefacts, same host; timings are single-scenario runs on a
  shared machine and carry the usual noise floor.
- The 1M slice's **CityGML write row** was not re-measured (nine hours on
  the previous run and unaffected by any fix): the 11 September sample is
  kept in the CSV with `measured-2026-09-11-pre-fix-run;not-re-measured` in
  its notes, and its three raw samples are in the write-samples file.
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
