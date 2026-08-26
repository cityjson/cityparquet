# Cross-stack test pass + benchmark — 2026-08-16 (Linux)

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development.
> Executors are Opus subagents; Fable coordinates. Steps use checkbox syntax.

**Goal:** Verify every feature of the CityParquet stack (cityparquet-rs, duckdb-cityjson, duckdb-3d) builds and works at latest `develop`, including all cross-component chains, by executing `TESTING.md` on Linux x86_64; then, once the parallel session in `cityparquet-rs` finishes its benchmark-harness work, run the benchmark and commit results.

**Architecture:** `TESTING.md` (repo root, commit 5fa54b3) is the authoritative script — it was verified on macOS 2026-08-16 at cityparquet-rs 84a2b38 / duckdb-cityjson f59d324(+fixes→b9d90ea) / duckdb-3d 9f19ae1. This pass re-executes it on Linux at: cityparquet-rs **3b2b586** (origin/develop, isolated worktree), duckdb-cityjson **b9d90ea**, duckdb-3d **9f19ae1**. Three parallel per-repo executors, then one integration executor, then a monitored wait, then the benchmark phase.

## Global constraints (every executor)

- **NEVER touch `/data2/hideba/cityparquet-paper/cityparquet-rs/`** (checkout of an active parallel session on `feat/format-comparison-benchmark`, claude pid 1621509). cityparquet-rs work happens ONLY in the worktree `/data2/hideba/cp-testpass-20260816/cityparquet-rs`. Reading files from the main checkout's `tests/fixtures/` is allowed; writing anywhere under it is not.
- A second idle claude session (pid 3807230) sits in `duckdb-3d`. Its working tree is clean and at latest develop; in-place incremental builds/tests there are fine, but do not switch branches, commit, or discard anything.
- PATH `duckdb` is **v1.3.2 — too old**. For every guide query use the snapshot shells `/data2/hideba/cp-testpass-20260816/bin/duckdb3d` / `duckdbcj` (v1.5.4) or the freshly rebuilt `build/release/duckdb` shells.
- Prefix heavy builds with `nice -n 10`; cap parallelism at `-j32` (128-core box shared with the parallel session).
- Expected values in TESTING.md are macOS observations: **row counts, assertion counts, exit codes, schemas, and footer keys must match exactly; byte sizes, timings, and DISTINCT-aggregation orderings may differ.** Assertion counts may exceed the guide's where commits landed after it (duckdb-cityjson b9d90ea added tests) — more passing assertions is fine, any failure is not.
- Report per-step: section number, PASS/FAIL/DEVIATION/SKIPPED, observed vs expected. Never claim green without pasting the actual final counts.

## Task 1 — cityparquet-rs (worktree) build + full Part 1 [Opus executor A]

- [ ] Worktree sanity: `git -C /data2/hideba/cp-testpass-20260816/cityparquet-rs log --oneline -1` → 3b2b586; `vendor/city3d-stac-tool` populated (done by commander).
- [ ] Fixtures (§0.2): `just fixtures`; if network fails, copy from `/data2/hideba/cityparquet-paper/cityparquet-rs/tests/fixtures/` (read-only source). Verify the 7 expected fixture files.
- [ ] Build (§0.4): `nice -n 10 cargo build --release -p cityparquet-cli` in the worktree.
- [ ] Suite (§1.1): `just check` (5 gates). Expected core: **714 passed / 0 failed / 1 ignored** across 51 targets (the ignored one is readbench `attr_consistency`). Use the aggregate awk line from §1.1.
- [ ] §1.2–1.5: convert Delft (report `2231 2 0 0 0 0 0 0 0`), verify Parquet schema + footer (`ARROW:schema`/`city`/`geo`; geo primary geometry_lod0_0; city primary geometry_lod2_2), STAC metadata.json (roles `['data','cityparquet-objects']`, lods `['0.0','1.2','1.3','2.2']`), railway multi-module convert with tri-state CRS (warning + `121 13 0 0 6 6 85 34 3`, 9 module tables with expected row counts, crs null/absent/object states per §1.5).
- [ ] §1.6 round-trip: `--no-lod0` convert → export → compare → `equal (excluded: 19)`, exit 0. Also confirm the default (with LoD0) gives exit 2 with the documented provenance diff.
- [ ] §1.7: `just interop` → `interop ok`.
- [ ] §1.9 partitioned output: `partitions=3 duplicate_ids=0`, 1001/1000/230, glob count 2231.
- [ ] §1.10: `just catalog-test` → **261 passed, 11 skipped**.
- [ ] Leave artifacts in `OUT=/data2/hideba/cp-testpass-20260816/cp_test` (delft, delft_arrow, railway, railway_crs, delft_parts, delft_rt…) for the integration executor.

## Task 2 — duckdb-cityjson rebuild + Part 2 [Opus executor B]

- [ ] In `/data2/hideba/cityparquet-paper/duckdb-cityjson` (in place, at b9d90ea): `nice -n 10 just rebuild` (if vcpkg baseline errors appear, apply §0.3: pin `$VCPKG_ROOT` to `84bab45d…` — record prior state first and restore nothing until the whole pass ends; report what you did).
- [ ] §2.1: `./build/release/test/unittest "test/sql/*"` → all pass; guide said 1186 assertions / 42 cases / 1 skip (`FCB_REMOTE_TEST_URL`); counts may now be higher.
- [ ] §2.2 read_cityjsonseq Delft count = 2231 (fixture path: use the MAIN checkout's `tests/fixtures/delft.city.jsonl` read-only, or the worktree copy).
- [ ] §2.3 suffixed per-LoD WKB mode: `geometry_properties_lod2_2` STRUCT shape incl. `shells [[6]]`.
- [ ] §2.6 sidecar readers on lod3_railway: **85 | 34 | 3**; `appearance := 'sidecar'` scan: 121 rows / 24 with material.
- [ ] §2.7 package mutation family: init/validate(0 rows)/insert_cityjsonseq(routes to bridge+building+city_furniture + 3 sidecars)/cityparquet_write with crs (7 files written incl. building 2231) and without crs (`crs: null` in footer)/cityparquet_read of the rs-written delft package → 2231.
- [ ] §2.8 metadata: version 2.0, title 3DBAG, 2231, EPSG:7415 struct.
- [ ] §2.9 FCB local: 3 and 3. Remote test SKIP (network-gated) unless trivially available.
- [ ] §2.10 C++ harnesses (Linux): `FCB_PREFIX="$(pwd)/build/release/vcpkg_installed/x64-linux"` (verify triplet dir name first) `test/cpp/run_encoder_tests.sh` and `run_fcb_selective_tests.sh` → both "All … assertions passed." Wasm: SKIP (heavy toolchain; guide didn't run it either).

## Task 3 — duckdb-3d build + Part 3 [Opus executor C]

- [ ] Confirm pid 3807230's process tree is idle and `git -C duckdb-3d status --porcelain` is empty before starting; report if not and proceed only if clean.
- [ ] `nice -n 10 make -j32 release` (or `just build`) then §3.1: `make test` → 469 assertions / 28 cases / 5 skips; `make test_cpp` → 528 assertions / 187 cases. Rebuild before trusting any failure (guide's stale-extension warning).
- [ ] §3.2 hollow solid: `THREE_D_TEST_FIXTURES=1 ./build/release/test/unittest "test/sql/st_3d_hollow_solid.test"` → 17 assertions, volume 56 semantics.
- [ ] §3.4 typed constructors + fixture gate: `SOLID_3D | 56.0`; st_aswkb count 1 without env, 12 with.

## Task 4 — Cross-component integration, Part 4 [Opus executor D, after 1–3]

- [ ] §4.1 unit cube via cityjson→three_d: `cube | 6 | true | 6.0 | 1.0`.
- [ ] §4.2 hollow solid shells contract: `[[6,6]]`, `2 | true | 56.0`.
- [ ] §4.3 rs package → duckdb-3d WKB chain on `$OUT/delft`: **parsed 1116 | valid 1098 | vol 1915861.2**.
- [ ] §4.5 duckdb-cityjson-written package → rs export: EXPECTED FAIL with `sidecar column 'id' has an unexpected array type` (open decision #2 — geometry_templates.id BIGINT vs VARCHAR). Reproduce §2.7's `pkg_out` first if executor B's artifact is absent. Confirm reverse direction (duckdb reads rs package) works.
- [ ] Use fresh binaries: rebuilt extensions from Tasks 2–3, CLI from Task 1's worktree. Load cityjson extension by absolute path into the three_d shell.

## Task 5 — Wait for the parallel session, then benchmark [commander + Opus executor E]

- [ ] Monitor pid 1621509 + `cityparquet-rs` HEAD. "Finished" = process gone, OR (no new commits AND process tree ~idle) for ≥60 min.
- [ ] Then: inspect final branch state (`feat/format-comparison-benchmark`, possibly merged to develop), read its bench docs/justfile at that commit, and run the benchmark it defines, in the worktree updated to that commit (never their checkout — unless their session has exited, in which case their checkout may be used read-mostly if the recipe demands its data dirs).
- [ ] Note: no `bench/data` corpus exists on this box; the new harness appears catalogue-URL-driven (`bench/catalogue_benchmark_urls.txt`). Follow its own README/recipes; fetch what it needs. `fcb` CLI is not installed — install per repo docs if the recipe requires it.
- [ ] Commit results to the cityparquet-rs repo on the branch the session left (their own convention: results CSVs are committed; e.g. `bench/read_results/*.csv`). Then record the submodule pointer in the paper repo per its `chore(submodules): …` convention.

## Reporting

Final user report must include: build results ×3, suite totals ×4 (rs workspace, catalogue, cityjson SQL, 3d SQL+C++), Part-1/2/3/4 outcomes with any deviations, status of open decisions #2/#7, what benchmark was run + where results were committed, and every decision taken autonomously.
