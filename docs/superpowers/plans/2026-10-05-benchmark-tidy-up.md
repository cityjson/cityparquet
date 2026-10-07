# Plan: tidy up the benchmark for the paper (§5)

A high-level overview of the work that brings the benchmark in line with the paper's evaluation chapter. It complements the handout (`2026-10-05-benchmark-paper-handout.md`), which lists what the paper expects; this document says what we do and in which order. Each task gets its own short design before it starts.

## How we work

- The commander session plans, briefs and verifies; an Opus subagent implements.
- Small commits on `develop` in this repository, nothing pushed until the author has looked. The parent paper repository's submodule pointer is bumped once, when a task is finished.
- Code only. The author edits the paper text separately.
- Citable numbers come from the benchmark host. On a laptop we change code, run the test suites and smoke runs, and re-render from committed evidence.

## Tasks, in order

### 0. Remove the codec and row-group axes — done

Recipes, evidence, figures, tests and documentation are gone (`22fc32b`..`5c71188`). The bloom axis is the only configuration axis left.

### 1. Remove the write benchmark — done

Drop the measurement of how fast each format is written: the timed-write script and recipe, the write rows in the heatmaps, their evidence, tests and documentation. The reason is fairness: not every format has a native writer, so the files are converted through a common format and the timings do not compare like with like.

Decided:

- The bloom axis loses its write time as well; it keeps its read scenarios and its file sizes.
- The database comparison keeps its write tier (add, update, delete, append): there it is one of the queries.

### 2. Make CityGML the baseline — done

Every relative value answers "CityParquet is x times faster, or smaller, than CityGML". Today the size plot and the heatmaps divide by CityJSONSeq.

- Switch the baseline in the size plot and the heatmaps, with their titles and captions.
- Emit the ratios as data, not only as colours: the size table of the handout (factor against CityGML, best and worst dataset) and the per-query ratios against CityGML. CSV only; the author converts it to Typst.
- One figure per dataset and metric, as SVG and PNG, in a folder per dataset: `formats/<dataset>/time.{svg,png}` and `formats/<dataset>/rss.{svg,png}`.
- A missing, skipped or failed CityGML cell leaves the ratio explicit as unavailable, never zero.
- The database figure keeps its own baseline (3DCityDB).

Two small fixes ride along: the specification site's benchmark page says timings are medians where the evidence reports means, and `just bench-summary` calls bare `python3`, which fails on Python older than 3.11, so it runs through `uv`.

### 2a. Five formats, one CityParquet, one 3DBAG slice — done

The format comparison measures five formats: the gzipped CityJSONSeq and the `duckdb-parquet` baseline are gone, and CityParquet is one Hilbert-ordered package per dataset, named `cityparquet` (`<x>.parquet/`). The source-order package is gone from the format benchmark and from the database comparison, which keeps one DuckDB configuration. The scaling series is gone: 3DBAG is the one slice `3dbag_n1000000`, cut by `fcb-slice --drop-lod 1.2` (`just fetch-3dbag`), and the bloom axis runs on the corpus and that slice into `bloom_results/`. The committed evidence is relabelled and the removed rows and slices deleted, with no measured value changed (`9902be9`..`6547da0`).

### 3. Interactive review of the benchmark code — done

The author reviewed the pipeline stage by stage; each finding became a change. What the review settled:

- **Corpus.** Six city datasets (Rotterdam, Vienna, New York, Zurich, Tokyo, Montréal) and the 1,000,000-object 3DBAG slice. Every source is normalised to one geometry per LoD and object; the slice is cut without LoD 1.2; CityParquet packages are written in Hilbert order without LoD 0 synthesis. The corpus holds only content CityGML 2.0, the baseline, can express: Ingolstadt is excluded because its window and door faces cannot be written to CityGML 2.0 (`READ_BENCHMARK.md`, Caveat 43). Tokyo and Montréal are derived sources kept in the R2 bucket.
- **Measurement.** Warm runs, one fresh process per sample, 25 repetitions (a `quick` profile runs 7), single-threaded readers. The CSVs carry mean, standard deviation, median, minimum, maximum and quartiles; summaries default to the median. Measured processes are pinned to one NUMA node under a 64 GB ceiling, and load is recorded per sample.
- **Return rule.** Read all returns every record with all fields in the format's native form; the spatial window returns the count, the identifiers and each match's highest-LoD geometry, visited in place; the attribute filter returns the count and the identifiers; the identifier lookup returns the whole object. The spatial window matches city objects in all five formats.
- **Cross-format check.** A run fails when the formats disagree on identifier sets, object, geometry and semantic-face totals, extents or returned-geometry counts.
- **Sizes.** Each size includes what the format needs for the queries; compact text formats; decimal units; `just bench-compression` reports the share and compression ratio per column group.
- **Bloom axis.** The slice only, on identifier, feature and attribute-equality lookups; the package without filters still prunes by row-group statistics, and both counts are recorded.
- **Databases.** One container-engine layer (`container`, `docker`, `podman`); every corpus dataset by parameter, with the format benchmark's windows and predicates; native binary results; whole-object identifier lookup; indexes on every queried predicate, with sizes reported with and without them; working memory in place of process memory; counts and identifier sets compared across the three systems.

### 3a. Before the host run

Nothing Linux-specific has run on Linux, and all committed evidence under `benchmark/runs/` predates the current harness. The first run on the host is a `quick` run and checks, in this order:

1. Choose the preparation mode (benchmark/README.md, "The hosted corpus"). `v8/` holds the six city datasets, so `just bench-prep` downloads them, verified against the manifest. The 3DBAG slice needs `just bench-prep --rebuild-sources --datasets 3dbag_n1000000` once, on a machine with the R2 token and enough memory: its CityGML synthesis takes hours, and its log shows `0 geometries dropped` and no LoD 1.2. Everything else downloads.
2. `MACHINE.md` and the manifests record the isolation actually applied: NUMA pinning, the memory ceiling, the load gate. A step the host refuses is recorded as not applied.
3. Warm runs stay warm under the 64 GB ceiling: the CityGML and CityJSON cells of the slice show no cold outliers.
4. Every format run prints `cross-format consistency OK`; Montréal, New York and Zurich have not been run through the current check.
5. `just bloom-columns` on the slice's package confirms that the configured attribute columns carry a Bloom filter.
6. The database run passes its count and identifier checks on the slice, including the LoD 2.2 query, which no local dataset can exercise, and on a dataset without a declared CRS.
7. `just bench-compression` gives the geometry share and ratio for the slice.
8. The `network` family runs on the host as part of the full run, under the profiles the author settles (open below: all three on the corpus; on the slice, `fast` and `typical`, or all three, given its run time); every network result's server and client totals agree.

Then the full run, `just bench-summary`, and the paper's placeholders.

### 4. Add the benchmark over the network

Run the format read benchmark over HTTP against object storage and report time, bytes read and request count per query (paper §5.4). The harness already has an HTTP transport (`--transport http`), used so far only by `bloom-bench-http`.

- A recipe that runs the format benchmark over HTTP for the corpus, and an upload step for the prepared artefacts.
- Bytes read and request count in the results, and a figure or table for them.
- The run records the storage location and the network path, and states that it is a snapshot of one path at one time.

The author supplies the URLs where the whole corpus is hosted when this task starts. To decide then: which formats take part.

Status. The primary measurement runs against a local simulated network (`net-sim`; profiles `fast`, `typical`, `slow` in `benchmark/manifest.toml`), the secondary against the hosted corpus; see `benchmark/README.md` "The network family".

- Done: the simulated server and its tests; the `network` family in `bench_suite.py` (`bench-run --families network`, `--network-profile`, a custom profile, `--network-target real`); the profile, target and the server's totals against the clients' in the params sidecar; each row's model time in `<dataset>.model.csv`; the README section and the `READ_BENCHMARK.md` caveat.
- Done: the `network` figure (time, bytes, requests per format and query), `network/network_factors.csv` and the bloom-over-network table in `benchviz`, with the "not measured" state when no results exist; the suite entry point run end to end on Rotterdam and Vienna under all three profiles; the FlatCityBuf attribute-filter caveat (a `fcb_core` 0.7.6 client artefact, not the format).
- Done: which cells the text formats run. They are measured on read all and the four identifier lookups (5 of 11 queries), and the other six are derived rows that copy read all's bytes and requests once the run has proven one whole-object request per measured sample. `[network] whole_file_scenarios` / `--network-whole-file-scenarios` (`full-read,id-lookup`, `full-read`, `all`) sets this. The text formats' transfer alone (4 samples per cell, parsing excluded) under this default: six city datasets 0.4 h `fast`, 4.0 h `typical`, 20 h `slow`; the 1M slice 0.8 h, 7.6 h, 38 h (all 11 queries: 0.9/8.8/44 h and 1.7/16.7/83 h). The table is in `benchmark/README.md`, "Run time".
- Done: the real target records the resolved host and addresses and, per object, the `cf-cache-status`/`age` headers before and after the measured requests (a one-byte ranged `GET`), and sends the corpus downloader's User-Agent.
- The host run: `just bench-run --families network --profile full --network-profile all` over the corpus and the slice, then the real snapshot.

## Afterwards

A full re-run on the host with the final harness, then the figures and tables are regenerated and the paper's placeholders filled.
