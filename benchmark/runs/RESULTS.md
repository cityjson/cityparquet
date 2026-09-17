# September 2026 benchmark evidence

Completed format, file-size, compression, row-group and database measurements, with raw
samples, query parameters, available run manifests and the rendered report.
The large 3DBAG format-write retry and rendering finished on 11 September 2026
at 08:47 UTC. Inputs and temporary packages are deliberately not tracked.

## Coverage and limitations

- Format results cover the five city datasets and 1,000,001-object 3DBAG input.
- Compression and row-group results cover all seven scaling sizes.
- CityGML large-input writing completed one warm-up and three measured runs.
  The measured median is 32,786.302283 seconds (about 9 hours 6 minutes).
- The first database run failed during cjdb ingestion with
  `cannot allocate memory for output buffer`. A retry on 12 September 2026,
  from about 11:58 to 15:14 CEST, completed all three systems (DuckDB over
  CityParquet, cjdb, 3DCityDB) on the 1,000,001-object 3DBAG input only:
  `databases/results/3dbag_n1000000.*`. It exited non-zero because the three
  bounding-box rows per system carry `status=mismatch`: cjdb returns slightly
  fewer objects than the other two systems, and CityParquet and 3DCityDB
  differ by three objects at 25 %. Those nine rows are not citable until the
  counts are reconciled. The run's manifest records no Git revision or
  timestamp; the times above come from file timestamps. The committed report
  and its database figure predate this run.
- Read measurements preceded the large-input write retry. Existing provenance
  files and their original absolute paths are retained as recorded; the snapshot
  does not imply every family was measured at a single Git revision.
- Missing query/metric coverage must not be interpreted as zero cost. Consult
  the CSVs, parameter sidecars and report caveats before citing comparisons.

`summary/full/index.html` is the self-contained report. Individual SVG and PNG
figures are in `summary/full/figures/`. `EVIDENCE_SHA256SUMS` verifies the saved
measurement and report files; it is not a replacement for run provenance.
