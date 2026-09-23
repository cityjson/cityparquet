# benchviz test fixture — the database family's CSV shape

`results/3dbag_n10000.csv` and its `.params.json` are renderer fixture values,
NOT measurements. They are written in `citybench run`'s nineteen-column shape
(`benchmark/databases/README.md`, "Metrics and the CSV contract"), with the
notes, tags and sidecar layout of a real smoke run, so the loader and the
figures are exercised on rows of the right shape:

- every read scenario under both `threads=single` and `threads=parallel`;
- `bbox-query` windows carrying `achieved=<fraction>`, the four `id-lookup`
  probes, the `duckdb-cityparquet-source` control on the windows only, and the
  DuckDB-only `parts-per-building-join`;
- one `ok-deviation` window (a `count-mismatch: … spread=…` decomposition below
  the tolerance), one `mismatch`, one `skipped`, and the upstream
  `error: BinderException` on DuckDB's `append-object`;
- the write tier, including `duckdb-cityparquet-writeback`.

The sidecar's id probes and windows are copied from a 1,000-object smoke run
and rescaled; the times and memory figures are invented. Replace nothing here
with measured rows; the tests assert on these values.
