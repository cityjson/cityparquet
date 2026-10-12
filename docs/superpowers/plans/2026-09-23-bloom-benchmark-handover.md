# Bloom-filter benchmark — handover

For whoever runs the `bloom` benchmark family. The implementation is finished,
reviewed and open as two pull requests; **nothing has been measured yet**.

- Branch: `feat/bloom-filters` (monorepo, 24 commits) and the same branch in the
  `lib/duckdb-cityjson` submodule (3 commits).
- PRs: cityjson/cityparquet#7 and cityjson/duckdb-cityjson#21.
  **Merge the submodule PR first** — the monorepo pins its commit, so a fresh
  clone's `just setup` breaks if the pointer lands first.
- Design: `docs/superpowers/specs/2026-09-22-bloom-filters-design.md`.
  Plan: `docs/superpowers/plans/2026-09-22-bloom-filters.md`.

## What exists

**Writer.** Filters on `id`, `feature_id`, and scalar `Utf8` (non-JSON)
attributes whose HyperLogLog-estimated distinct count reaches
`0.2 × non-null count` with at least one non-null value — decided once per
dataset during the scan. FPP 0.01, NDV unset, `BloomFilterPosition::End`
(after the last row group, before the page indexes and footer). On by default.
`convert --no-bloom` / `--bloom-fpp`; the `+nobloom` variant token writes none.
Sidecars never carry filters. The `ParquetDefaults` preset writes none.

**Reader.** `id_lookup`, `attr_filter` (string equality) and `feature_lookup`
(table, object-store and package level), each with a `_with_stats` form
returning `LookupStats { row_groups_total, bloom_pruned, filter_bytes }`.
`filter_bytes` is bitset bytes (32 × blocks), identical on both transports.
The async path issues **one** `AsyncFileReader::get_byte_ranges` call, with a
per-row-group fallback when `bloom_filter_length` is absent. A positive filter
never decides a row — the exact `RowFilter` still runs.

**duckdb-cityjson.** The same intent through file-wide `COPY` options
(`ROW_GROUP_SIZE`/`DICTIONARY_SIZE_LIMIT` 122880, 8 MiB string dictionary-page
cap, `WRITE_BLOOM_FILTER`, FPP 0.01; sidecars off; a `bloom` option). Its
column set is **wider** than the Rust writer's, because DuckDB decides per
dictionary-encoded chunk: numeric and temporal attributes, `bbox` leaves and
list leaves get filters too. Documented in
`documents/docs/06-resources/02-software.mdx`.

## The family

One axis, one pair — `cityparquet` (filters, the default) against
`cityparquet+nobloom`. No FPP sweep and no row-group cross: those were dropped
deliberately, so do not add variants without asking the author.

```sh
just bloom-bench <FOLDER> [OUT] [PREPARED] [REPEAT] [WRITE_REPEAT]
just bloom-bench-http <FOLDER> <BASE_URL> [OUT] [PREPARED] [REPEAT]
```

- Default output: `benchmark/runs/formats/scaling_bloom_results/`
  (HTTP: `scaling_bloom_http_results/`). Everything generated stays under
  `benchmark/runs/`.
- Or through the suite: `benchmark/scripts/bench_suite.py --families bloom`
  (`--smoke` for the fast path), which stages inputs and calls the recipe.
- Scenarios: `id-lookup` with probes `id-50pct` and `id-miss`, and
  `feature-lookup` with `feature-50pct` and `feature-miss`. `feature-lookup` is
  CityParquet-only and never runs unless named explicitly.
- Results CSV is **16 columns**, ending
  `…,http_requests,row_groups_total,bloom_pruned,filter_bytes`. The last three
  are filled only on CityParquet lookup rows.
- Figures: `bloom` (main), `bloom-scaling` (3DBAG slices only) and
  `bloom-corpus` (per-dataset bars). Render with `just bench-summary`.

## Inputs

- Scaling slices: `benchmark/runs/data/scaling/3dbag_n1000 … n1000000.city.jsonl`.
- Corpus: rotterdam, ingolstadt, vienna, new_york, zurich (all single
  object-table packages — checked, so `bloom-bench`'s `set -e` will not abort
  on a multi-table refusal).
- The HTTP recipe is **read-only**: it reads packages a local run wrote and you
  then uploaded (`benchmark/scripts/readbench_upload.md`). It was verified
  against an in-process `ServeDir`, never a real bucket.

## Before you cite any number

1. **The committed codec, rowgroup, formats and sizes results predate
   default-on bloom filters.** Caveat 31 in `benchmark/formats/READ_BENCHMARK.md`
   says so and it reaches the rendered summary page. Re-run those families
   before comparing them with anything measured now. `format_write.py` refuses
   to append to a 13-column CSV, so delete the old file for an incremental run.
2. **The bloom figures currently in the fixtures are hand-written**, not
   measurements (`benchmark/plot/tests/fixtures/benchviz/README.md` says so).
   Replace them once a real run exists.
3. **Caveats are part of the artefact.** Never drop or soften one to make a
   number look better; add new ones for anything you discover.
4. The 1M-slice acceptance numbers below came from a shared host, alongside
   another benchmark. Check what else is running before you trust latencies.

## Known measurements (acceptance, not the benchmark)

From `lib/cityparquet-rs/crates/core/tests/bloom_corpus.rs`, run with
`CITYPARQUET_SCALING_DIR=benchmark/runs/data/scaling` (they are `#[ignore]`):

- 1M slice at `--row-group-size 512`: a miss prunes 1949/1954 row groups for
  `id`, 1950/1954 for `feature_id`.
- Async: one `get_byte_ranges` call, 1 object-store request, 4,093,632 bytes
  for 1954 filters.
- No false negatives: all 10,000 ids of the 10k slice found, sync and async.
- Cross-writer: `just interop` reads a DuckDB-written package through the Rust
  lookups — 2,231 ids and 1,115 feature ids, none lost.

## Traps

- **3DBAG slices above n10000 cannot be read into DuckDB at all**:
  `documentdatum` holds `210-12-20T`. That blocks any DuckDB-side work on the
  larger slices.
- The in-tree `lib/duckdb-cityjson/build/release` can be stale against the
  pinned submodule commit. Rebuild (`just -f lib/duckdb-cityjson/justfile
build`) before trusting anything DuckDB-side.
- Benchviz writes SVGs to `<out>/figures/`, not `<out>/`.
- Gates: `cd lib/cityparquet-rs && just check`, `just plot-test`,
  `just scripts-test`, `just mcp-check`, `just interop`. Under memory pressure
  the linker can fail; retry with `CARGO_BUILD_JOBS=2`.

## Open, deliberately

- The determinism test for attribute selection
  (`crates/core/tests/scan_real_data.rs`) cannot fail for its stated reason: a
  randomly seeded estimator would still pass. The property holds today
  (`wyhash` default seed 0) and a sibling test pins the expected column set.
- A dozen minor test-strengthening notes, each recorded with its file and line
  in the controller ledger (see below).

## Where the record is

`.superpowers/sdd/2026-09-22-bloom-filters/` in the monorepo checkout —
gitignored, so it exists only on this machine. It holds `progress.md` (every
decision and its cost if wrong), the eleven task reports with their measured
output, and sixteen reviews. Delete it once the PRs are merged; git history is
the record then.
