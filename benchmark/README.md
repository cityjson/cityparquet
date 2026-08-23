# Benchmarks

Three benchmark families, each answering a different question, each with its own
corpus and its own caveats. This page is the map: what each one measures, what
it will and will not support as a claim, and which caveats are load-bearing. The
methodology lives with each family; nothing here restates a number.

| Directory                           | Question                                                                       | Compared against                                                 |
| ----------------------------------- | ------------------------------------------------------------------------------ | ---------------------------------------------------------------- |
| [`formats/`](formats/README.md)     | How does CityParquet read, write and compress against the other file formats?  | CityGML, CityJSON, CityJSONSeq, gzipped CityJSONSeq, FlatCityBuf |
| [`databases/`](databases/README.md) | How does reading CityParquet compare with querying a 3D city model _database_? | cjdb, 3DCityDB v5 (both PostgreSQL), DuckDB over CityParquet     |
| [`plot/`](plot/)                    | Neither — it renders. A uv project holding the two chart packages.             | —                                                                |

## What is in here

Everything the benchmark is:

|              |                                                                    |
| ------------ | ------------------------------------------------------------------ |
| `readbench/` | the cross-format read harness — its **own Cargo workspace**        |
| `scripts/`   | the conversion chain, the fetchers, and their own bash test suites |
| `formats/`   | the read/write/compression corpora, results and methodology        |
| `databases/` | the database comparison — its own uv project and justfile          |
| `plot/`      | the renderers; measures nothing                                    |

**The recipes are not here, and that is deliberate.** `just bench`,
`just fetch-data`, `just plot-pretty` and the rest live in the **root**
`justfile` and run from the repository root, because a benchmark run has to
reach the _library_ as well — it builds the `cityparquet` converter it measures,
from `lib/cityparquet-rs`. A recipe that spans both belongs above both.

`readbench/` is a separate workspace from the library on purpose: it keeps
`cd lib/cityparquet-rs && just check` a gate on the **library alone**, runnable
with no `uv`, no `jq` and no corpus. The root `just check` runs both.

Two things in `readbench/Cargo.toml` are load-bearing and easy to "tidy" away:
it repeats the `[patch.crates-io] cjseq` line (a `[patch]` is honoured only in
the workspace root being built, so without it the benchmark would silently use
the unpatched upstream), and its `fcb_core`/`cjseq2` pins are **exact** — they
are a measured format's reader, and a caret range would let a later release
change what the published numbers mean.

## What is in git and what is not

| Artefact                                                     | Committed?                                                                                                                                                                                                                                                     |
| ------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `formats/scaling_{read,write,compression,ordering}_results/` | **yes** — the configuration-axis evidence, four cardinalities of one 3DBAG slice                                                                                                                                                                               |
| `formats/archive/2026-08-17-catalogue-corpus/`               | **yes** — the retired 30-dataset corpus and its results, kept so the earlier claims stay checkable                                                                                                                                                             |
| `formats/READ_BENCHMARK.md`, `formats/README.md`             | **yes** — the methodology, including all 18 fairness caveats                                                                                                                                                                                                   |
| `databases/results/`                                         | **no, currently** — the CSVs were removed with the read benchmark's, for the same reason and a second one: on Linux the harness recorded `peak_rss_bytes` in KiB, not bytes, so five result files were 1024× wrong. `just db bench <dataset>` repopulates them |
| `formats/read_results/`, `formats/ordering_results/`         | **no, currently** — the CSVs were removed when the read corpus was replaced (six fully-comparable cityjson.org datasets in place of thirty partly-comparable ones). `just bench` repopulates them                                                              |
| `formats/data/` (~24 GB), `formats/tools/`                   | **no** — fetched from pinned URLs with pinned byte sizes by `just fetch-data` / `just fetch-scaling-data` / `just fetch-tools`                                                                                                                                 |
| `summary/`, `**/plots/`                                      | **no** — derived from the CSVs in seconds by `just plot-pretty` / `just plot`                                                                                                                                                                                  |

Because the read CSVs are not currently committed, `just plot-pretty` produces a
summary page whose read views say so rather than showing an empty grid. That is
the intended behaviour, not a failure.

**`databases/README.md` still cites "the currently committed `results/…`"** in
half a dozen places, and quotes numbers from them. Those files were removed in
the same sweep and the prose has not caught up; the figures it quotes were
measured, but you cannot check them against a file in this repository until a
re-run puts one back. Treat every such number there as provisional.

## Caveats that are load-bearing

Read the family's own methodology before citing anything from it. Three caveats
are quoted here because they are the ones most likely to be dropped in
translation:

1. **"Smallest codec" is not a citable claim from `formats/`.** The codec
   variants are written at the `parquet-rs` implementation defaults — zstd at
   level 3, gzip at 6, brotli at 1. That is a comparison of defaults, not of
   codecs at equal effort. A codec ranking needs a re-run with levels chosen
   deliberately. (`formats/README.md`, "The codec levels are NOT matched".)

2. **Ingest is deliberately not compared in `databases/`.** Encoding a
   CityParquet package and populating an indexed relational schema are different
   operations, not two points on one scale. Ingest wall-clock is recorded in
   each dataset's manifest, never in the results CSV, and carries an explicit
   caveat where it appears. (`databases/README.md`, "Purpose and claim".)

3. **The `citygml` row measures a _synthesised_ artefact.** Every entry in the
   read corpus is fetched as CityJSON and its CityGML is derived from it by
   `citygml-tools`, because not one of the `.gml` files published beside those
   datasets is CityGML 2.0 — six are 1.0 and two are 3.0, and this repository's
   reader accepts only 2.0. What that costs is stated in full in
   `formats/READ_BENCHMARK.md`'s "CityGML synthesis" section, and one entry
   (`3dbag_9-284-556`) additionally loses an LoD in the round trip, so its
   `citygml` row is not content-equivalent to its other seven.

`formats/READ_BENCHMARK.md` carries eighteen such caveats. They are numbered and
cross-referenced from the tables they qualify; the summary page quotes them
verbatim rather than paraphrasing, so a page and its methodology cannot drift.

## Running them

```sh
just fetch-tools                              # pinned citygml-tools + cjseq (one-off)
just fetch-data                               # the six-dataset read corpus, 423 MB
just bench benchmark/formats/data/benchmark   # the cross-format read comparison

just fetch-scaling-data                       # the configuration-axis corpus (7.6 GB source)
just compression-bench benchmark/formats/data/scaling
just ordering-bench    benchmark/formats/data/scaling

just plot-pretty                              # the cross-dataset summary page + print figures

just db --list                                # the database comparison's own recipes
```

None of these is in CI. They are multi-hour, corpus-dependent, and in the
database family's case need podman. `just plot-test` and `just scripts-test` —
the harness's own suites, which need neither a corpus nor a network — are the
gates that run cheaply.
