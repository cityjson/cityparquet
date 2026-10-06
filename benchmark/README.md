# Benchmarks

The suite compares file formats, CityParquet configurations and database query
systems. Run the three public commands from the repository root:

```sh
just bench-prep
just bench-run
just bench-summary
```

Preparation downloads and prepares inputs; running measures them; summarising
renders existing results. Rendering never starts a benchmark.

On this machine the data and output root is
`benchmark/runs/`. Its layout is:

| Path                                          | Contents                                          |
| --------------------------------------------- | ------------------------------------------------- |
| `data/benchmark/`, `data/3dbag/`              | Source corpus and the 3DBAG slice                 |
| `data/readbench/`                             | Prepared format artefacts                         |
| `formats/results/`, `formats/bloom_results/`  | Full format, size and bloom measurements          |
| `formats/{short,smoke}/`                      | The same families under the `short` and `smoke` profiles |
| `databases/{prepared,results,short,smoke}/`   | Database lifecycle inputs and measurements        |
| `summary/{full,short,smoke}/`                 | Rendered figures and combined HTML                |

Benchmark inputs, derived artefacts, results and rendered summaries are generated
beneath this ignored directory. The paper checkout may explicitly export figures
to `paper/assets/bench/`.

## Selecting work

```sh
just bench-prep --families formats
just bench-run --families bloom
just bench-run --profile short
just bench-run --datasets 3dbag --families formats
just bench-summary --data-root benchmark/runs
```

The family names are `sizes`, `formats`, `bloom` and `databases`. With no
selection, the suite includes all four. Use each command's `--help` for its
selection and output options. Smoke runs validate the pipeline with small
inputs and fewer repetitions; their results are not publication runs.

Three run profiles decide which datasets are measured, how many repetitions
and where results land, so a test run can never overwrite the paper's
evidence: `--profile full` (the default; the seven corpus datasets and the
3DBAG slice, 25 read repetitions, each family's own results directory, and
the database family measures the slice), `--profile short` (the corpus
without the slice, the same repetitions, results under `<family>/short/`; for
iterating on the harness in about an hour rather than a day) and `--profile
smoke` (`--smoke`: Rotterdam alone, one repetition, `<family>/smoke/`; a
pipeline check, not a measurement). Under `short` and `smoke` the database
family measures Rotterdam (the manifest's `small_database_dataset`) through
its prepared `rotterdam_delfshaven.city.jsonl`. Every run manifest records its
profile.

Each measured cell takes one discarded warm-up and then 25 timed samples by
default, run back to back; samples are not interleaved across formats or
systems. For the formats and bloom families, `just bench-run
--cell-budget-s <seconds>` (off by default) stops a cell's sampling once its
runs, warm-up included, have taken that long and at least `--min-repeat`
samples exist (default 7); such a row records the samples taken in `repeat`
and carries the `budget` tag in `notes`. The database family always takes
`--repeat` samples. `benchmark/formats/READ_BENCHMARK.md` ("Sampling") has
the rule.

## Full run

Run the full matrix only after the smoke run succeeds:

```sh
just bench-prep --data-root benchmark/runs
just bench-run --data-root benchmark/runs
just bench-summary --data-root benchmark/runs
```

Preparation fetches the seven-file corpus (about 1.2 GB), cuts the 3DBAG slice
from the pinned 7.6 GB FlatCityBuf source (`just fetch-3dbag`), prepares all
required format artefacts (`readbench_prepare.sh`), builds the
release CityParquet CLI, and prepares the database environment. It needs Rust
and Cargo, Java 17 or later, `fcb`, `cjseq`, the pinned citygml-tools archive,
Python with `uv`, and rootless Podman plus the database images for the database
family. The full run writes substantial prepared packages and result files
under the data root and takes hours; the database family also needs its own
container storage and available local ports. `bench-summary` only reads those
results.

Use `--families` to run one family after preparing it. `--datasets` accepts
manifest IDs, and `3dbag` for the slice (`[suite] slice_dataset` in
`manifest.toml`). The database family's dataset is the slice under `full` and
Rotterdam under `short` and `smoke`. The selector rejects data roots outside
`benchmark/runs/`.

## Running on a shared host

The citable numbers come from a shared Linux host without root, so the read
families (`formats`, `bloom`, the variant runs) isolate their measured child
processes as far as an ordinary user can, and record every setting as applied
or "not applied: <reason>" in each CSV's `.params.json` (`isolation`) and in
the results directory's `MACHINE.md`. Off Linux nothing is applied and the
records say why.

| Setting | Flag / recipe parameter | Default | Applied by |
| --- | --- | --- | --- |
| NUMA node | `--numa-node`, `NUMA_NODE` (env `BENCH_NUMA_NODE`) | `auto`: the node with the most free memory at start | `numactl --physcpubind=<node cores minus the first> --membind=N`; else `taskset -c` (CPU only); the coordinator pins itself to the node's first core |
| Memory ceiling | `--memory-max`, `MEMORY_MAX` (bytes) | off | `systemd-run --user --scope -p MemoryMax=`, probed once; a refusal is recorded, never fatal |
| Load gate | `--max-load`, `MAX_LOAD` | `auto`: half the pinned node's cores | before every sample, `load1 * node_cores / total_cores` above the threshold waits in 10 s steps |
| Longest wait | `--max-load-wait-s`, `MAX_LOAD_WAIT_S` | 600 | a cell that proceeds while still contended is tagged `busy` in `notes` |

One node rather than the whole machine keeps memory local and leaves the other
node to co-tenants. The load threshold is half the node because our own child
adds about one runnable task: half leaves room for it and light co-tenancy
while still catching a node that is genuinely contended. Each sample's
`load1`, runnable count and `MemAvailable` go to `.samples.json`, and each
cell's maxima to `.params.json`. `just bench-run` forwards the same settings
(`--numa-node`, `--memory-max`, `--max-load`, `--max-load-wait-s`) and records
them in the run manifest.

The samples of one cell run back to back after its warm-up; formats are not
interleaved. What needs root, and is therefore not done: changing the CPU
governor, disabling SMT or turbo, dropping page caches, `isolcpus`, and a
cgroup `cpuset` for the database family's podman containers without user
delegation (`MACHINE.md` records whether the user cgroup delegates `cpuset`
and `memory`).

## Experimental matrix

| Family      | Data                             | Measurements                                               | Read queries                                                                            |
| ----------- | -------------------------------- | ---------------------------------------------------------- | --------------------------------------------------------------------------------------- |
| `sizes`     | Corpus with the 3DBAG slice      | Complete file or package size                              | None                                                                                    |
| `formats`   | Same corpus                      | Read time and peak memory                                  | All format queries                                                                      |
| `bloom`     | Same corpus                      | Size; lookup time, memory and row-group counters           | `id-lookup` at `id-50pct`/`id-miss`; `feature-lookup` at `feature-50pct`/`feature-miss` |
| `databases` | The 3DBAG slice                  | Storage including indexes; mean query time and peak memory | Database query suite                                                                    |

The corpus is Rotterdam, Ingolstadt, Vienna, New York, Zurich, Tokyo (Chiyoda)
and Montréal, with the 3DBAG slice
([`formats/README.md`](formats/README.md) lists each with its source and query
predicates; Tokyo and Montréal are derived by this project, and Montréal is
exported from CityParquet). Dataset IDs identify artefacts;
figures use readable display names. The slicer (`fcb-slice`, driven by `just
fetch-3dbag [DEST] [SIZES]`) takes whole features from a pinned FlatCityBuf
source in source order until it reaches 1,000,000 CityObjects; because a
feature is indivisible, the slice holds 1,000,001, and the recorded count is
the one reported. Another `SIZES` cuts a smaller prefix for trying the harness
out, which no profile measures. The slice is treated like a corpus dataset in the
format comparison, the size table and the bloom axis, and it is the database
family's dataset. It is cut without LoD 1.2: CityGML 2.0 has integer LoDs only and
cannot carry LoD 1.2 beside LoD 1.3, so removing it at the source gives all
five formats the same geometry, LoD 0, 1.3 and 2.2
([`formats/READ_BENCHMARK.md`](formats/READ_BENCHMARK.md), Caveat 14).

The format comparison measures five formats: `citygml`, `cityjson`,
`cityjsonseq`, `flatcitybuf` and `cityparquet`. CityParquet is one package per
dataset, its rows written in Hilbert-curve order (`cityparquet convert
--ordering hilbert`: the writer's default, pinned explicitly so the benchmark
states its configuration), displayed as **CityParquet**. The bloom experiment compares `cityparquet` with
`cityparquet+nobloom`, both Hilbert-ordered, and holds ordering, codec and
row-group size fixed.

## Figures

`just bench-summary` produces individual SVG and PNG files, the format
comparison's ratio tables as CSV, and a self-contained `index.html` collecting
the same figures, tables and conditions. The format comparison lives in its own
`formats/` folder of the figures directory, one sub-folder per dataset id.

| Figure                                                  | Content                                                                                                                                                                    |
| ------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `formats/sizes`                                         | Vertical size bars, one subplot per dataset; actual sizes and factors against CityGML                                                                                      |
| `formats/<dataset>/time`, `formats/<dataset>/rss`       | One heatmap per dataset and metric: read time or read peak memory per query and format, with factors against CityGML                                                       |
| `formats/size_factors.csv`, `formats/size_extremes.csv` | Bytes and size factors against CityGML per dataset; the best and worst dataset by CityParquet's factor                                                                     |
| `formats/query_factors.csv`                             | Time and peak memory per dataset, query and format, with both factors against CityGML                                                                                      |
| `bloom`                                                 | Package size and the two read heatmaps for the 3DBAG slice                                                                                                                 |
| `bloom-corpus`                                          | The bloom pair per corpus dataset                                                                                                                                          |
| `databases`                                             | Storage bars; time/memory heatmaps, `threads=single` and `threads=parallel` apart; the write tier as rows below the reads (`threads=single` only, Caveat 19 as a footnote) |

The format comparison's baseline is **CityGML**, and every relative value is a
factor: CityGML's value divided by the format's, so **higher is better** ("CityParquet
is 5× faster, or smaller, than CityGML"), with 1× neutral. The configuration and
database comparisons keep a ratio of measurement to baseline, **lower is
better**: configuration comparisons use their fixed default and database
comparisons use **3DCityDB**. Cell labels show actual values and units. A
missing, skipped or failed baseline cell leaves its factor or ratio unavailable
and is not replaced by another system. Missing, unsupported or failed measurements remain explicit rather
than becoming zeros. The combined HTML reports incomplete coverage.

## Evidence and interpretation

Keep source identity, query parameters, software revisions, machine information
and repetition settings with measurements. Do not combine results from
different runs as if they were one experiment. Database memory must cover the
execution system, not only its client process; the database methodology defines
the measurement boundary and sampling limitations.

Read the detailed methodology before citing a result:

- [Format queries and fairness caveats](formats/READ_BENCHMARK.md)
- [File sizes and configuration experiments](formats/README.md)
- [Database measurements](databases/README.md)

Relevant qualifications include CityGML synthesis and possible information
loss, feature-versus-CityObject counting grain, source-position-dependent ID
lookups, and small timing differences relative to repetition spread. Preserve
these qualifications in captions and reports. Database ingest time is separate
from the query comparison.

## Layout and checks

`readbench/` is a separate Rust workspace for the format harness. `scripts/`
holds preparation and orchestration. `formats/` and `databases/` own their
measurement artefacts. `plot/` renders them; `summary/` holds generated output.
Large source files and prepared packages are fetched or generated, not committed.

Run `just plot-test`, `just scripts-test`, the readbench Rust tests and the
database unit tests when changing the corresponding components. Benchmarks are
separate from CI checks: complete runs require the corpus, external converters
and database containers, and can take hours.
