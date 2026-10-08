# benchviz test fixture — the database family's CSV shape

`results/3dbag_n10000.csv`, its `.params.json` and its `.manifest.json` are
renderer fixture values,
NOT measurements. They are written in `citybench run`'s current header (`citybench/report.py` `COLUMNS`,
pinned by `tests/test_csv_contract.py`)
(`benchmark/databases/README.md`, "Metrics and the CSV contract"), with the
notes, tags and sidecar layout of a real smoke run, so the loader and the
figures are exercised on rows of the right shape:

- every read scenario under both `threads=single` and `threads=parallel`;
- `bbox-query` windows carrying `achieved=<fraction>`, the four `id-lookup`
  probes, and the DuckDB-only `parts-per-building-join`;
- one `ok-deviation` window (a `count-mismatch: … spread=…` decomposition below
  the tolerance), one `mismatch`, one `id-mismatch`, one `skipped`, and the upstream
  `error: BinderException` on DuckDB's `append-object`;
- the write tier, including `duckdb-cityparquet-writeback`;
- `peak_working_mem_bytes` with the current memory-scope tags, and a manifest
  declaring that metric and splitting each system's storage into data and
  indexes (`index_bytes`, plus CityParquet's Bloom-filter, page-index and
  footer bytes).

The sidecar's id probes and windows are copied from a 1,000-object smoke run
and rescaled; the times and memory figures are invented. Replace nothing here
with measured rows; the tests assert on these values.
