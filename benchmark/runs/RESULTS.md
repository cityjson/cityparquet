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
  not to re-run them, as bloom filters are not part of those axes.

## Coverage and limitations

- The 1M slice's **CityJSON read rows are absent**: the whole-document
  parse (about 35 GB resident) was refused twice by the host's strict
  memory-overcommit limit while other users' processes held the commit
  budget. They are re-measured separately once the budget allows and
  merged into `formats/results/3dbag_n1000000.csv`; until then every 1M
  figure shows CityJSON as not measured.
- The 1M slice's **CityGML write row** was not re-measured (nine hours on
  the previous run and unaffected by any fix); the 11 September sample is
  kept in the paper repository's research notes and is re-attached, tagged,
  when the CityJSON rows are merged.
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
