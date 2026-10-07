# Benchmarks

The suite compares file formats, CityParquet configurations and database query
systems. Run the three public commands from the repository root:

```sh
just bench-prep
just bench-run
just bench-summary
```

Preparation obtains the prepared inputs (see [The hosted corpus](#the-hosted-corpus)); running measures them; summarising
renders existing results. Rendering never starts a benchmark.

On this machine the data and output root is
`benchmark/runs/`. Its layout is:

| Path                                              | Contents                                                          |
| ------------------------------------------------- | ----------------------------------------------------------------- |
| `data/benchmark/`, `data/3dbag/`                  | Source corpus and the 3DBAG slice                                 |
| `data/readbench/`                                 | Prepared format artefacts                                         |
| `formats/results/`, `formats/bloom_results/`      | Full format, size and bloom measurements                          |
| `formats/{quick,short,smoke}/`                    | The same families under the `quick`, `short` and `smoke` profiles |
| `databases/{prepared,results,quick,short,smoke}/` | Database lifecycle inputs and measurements                        |
| `summary/{full,short,smoke}/`                     | Rendered figures and combined HTML                                |

Benchmark inputs, derived artefacts, results and rendered summaries are generated
beneath this ignored directory. The paper checkout may explicitly export figures
to `paper/assets/bench/`.

## The hosted corpus

The prepared corpus is published on Cloudflare R2, so that a machine obtains
it without converting anything, the network benchmark reads the very same
files over HTTP, and a run's evidence names the exact bytes it measured. It
is public at `https://other-data.open3d.city`, under
`cityparquet-paper/benchmark/v<chain>/`, one folder per preparation-chain
version (`CHAIN_VERSION` in `scripts/readbench_prepare.sh`, currently 8):

```
v<chain>/manifest.json
v<chain>/citygml/<id>.gml
v<chain>/cityjson/<id>.city.json          the normalised, compact source every artefact derives from
v<chain>/cityjsonseq/<id>.city.jsonl
v<chain>/flatcitybuf/<id>.fcb
v<chain>/cityparquet/<id>.parquet/...     every file of the package
v<chain>/cityparquet-nobloom/<id>.parquet/...  the bloom axis's package without Bloom filters (see below)
```

`<id>` is the dataset's file stem (`rotterdam_delfshaven`). The folders exist
only in the keys: locally every artefact keeps its flat name in
`data/readbench/`, which is what `bench-run` and the database family read.
`scripts/corpus_bucket.py` owns the layout, the manifest, the download and
the upload; the read harness maps a format to its folder in `Format::key`
(`--key-layout bucket`), and a test holds the two in step.

`just bench-prep` has four modes:

| Mode                | Downloads                                           | Builds locally                                                              | Uploads                            |
| ------------------- | --------------------------------------------------- | --------------------------------------------------------------------------- | ---------------------------------- |
| default             | every artefact of the selected datasets             | nothing                                                                     | nothing                            |
| `--no-cache`        | `cityjson/` and `citygml/`                          | CityJSONSeq, FlatCityBuf and the CityParquet package, with the current code | those artefacts, then the manifest |
| `--rebuild-sources` | the published sources (`fetch-data`, `fetch-3dbag`) | everything: normalisation, compaction, CityGML synthesis, every artefact    | everything, then the manifest      |
| `--local`           | the published sources                               | everything                                                                  | nothing; no bucket access          |

The `smoke` profile always prepares locally. The default mode checks every
file against the manifest (bytes and sha256), resumes an interrupted download
with a range request, skips a file that already matches, and refuses, naming
`--no-cache` and `--rebuild-sources`, when the bucket holds no `v<chain>/`
for the code's chain version, when the manifest lacks a selected dataset or
artefact, or when a hash does not match. Only the two uploading modes need
credentials: an `rclone` remote with write access to the bucket. The token
cannot list the bucket root, so every call passes `--s3-no-check-bucket`.

An upload never silently replaces: identical content is skipped, and a key
that exists with different content stops the upload before anything is sent,
naming every differing key, unless `--force-upload` is given. The object
store keeps no sha256, so an existing key is compared with the manifest's
recorded hash, or, when the manifest does not record it, downloaded through
`rclone cat` and hashed. After the upload every key is checked by size with
`rclone lsjson`; the content relies on rclone's own transfer checksum.

`manifest.json` is written last, so a partial upload shows as keys the
manifest does not list. It holds `schema`, `chain_version`, `created_at`,
`updated_at` and, per dataset, `source` (the published source's `url` and
`sha256`), `built` (`created_at`, `monorepo_commit`, `monorepo_tree_dirty`,
`cityparquet_rs_commit` and the `tools`: citygml-tools, cjseq, the fcb CLI,
the pinned `fcb_core`, the `cityparquet` CLI) and `artefacts` (per artefact
its `key`, `bytes` and `sha256`, or for a package directory `files`, each
with `bytes` and `sha256`). Datasets are merged into the manifest, so one
machine can add a dataset without re-uploading the others. The provenance
chain is published source, then normalised CityJSON (`cityjson/`), then
every other artefact; `cityjson20/` and `citygml20/` beside `v<chain>/` are
an earlier upload and stay the published home of the Tokyo and Montréal
sources.

The prepared directory records where its artefacts came from in
`data/readbench/.corpus-origin.json`: the manifest's URL and sha256, or
`local`. Each run manifest (`*.run.json`) copies it into its `corpus` field.

The configuration is read from the environment, with these defaults:
`CITYPARQUET_CORPUS_REMOTE=r2`, `CITYPARQUET_CORPUS_BUCKET=other-data`,
`CITYPARQUET_CORPUS_PREFIX=cityparquet-paper/benchmark`,
`CITYPARQUET_CORPUS_BASE_URL=https://other-data.open3d.city`. Use the custom
domain, not the rate-limited `r2.dev` URL.

The chain is deterministic apart from the CityGML. The CityJSONSeq follows
the source document's object order (`seq-order` fixes it before `cjseq cat`
writes the stream), and the STAC `datetime` of every benchmark package is
the fixed `CORPUS_DATETIME` of `scripts/readbench_prepare.sh`, so two builds
of the same chain from the same source agree byte for byte. The `.gml` does
not: citygml-tools mints random surface identifiers, so it is not
reproducible by hash, and the hosted file is the reference. `--no-cache`
therefore downloads it and reuses it rather than synthesising a new one, and
a `--no-cache` run that reproduces every hosted artefact uploads nothing, not
even the manifest. The custom domain does not cache the objects at the edge
(`cf-cache-status: DYNAMIC`), so a read over HTTP reaches the bucket.

`cityparquet-nobloom/` holds the bloom axis's package without Bloom filters,
for the datasets whose manifest entry lists it in `variant_artefacts` (the
3DBAG slice). Preparation builds it through the code path a `--variants` run
uses and keeps it locally as `<id>.cityparquet+nobloom.parquet/`; the bloom
run reuses that prepared variant while its chain stamp is current, rather
than converting it again.

The bucket holds chain 8 for the six city datasets (Rotterdam, Vienna, New
York, Zurich, Tokyo and Montréal). The 3DBAG slice is added by
`just bench-prep --rebuild-sources --datasets 3dbag_n1000000`, once, on a
machine with the R2 token and enough memory for its CityGML synthesis; every
other machine then downloads it with the default mode.

## The network family

`bench-run --families network` is the format read benchmark over HTTP: per
query it records the time, the bytes read and the number of HTTP requests, for
the five formats and, on the 3DBAG slice, for the `cityparquet` /
`cityparquet-nobloom` pair on the identifier, feature and attribute-equality
lookups (the prepared no-bloom package is reused, never rebuilt). Bytes read
and requests are properties of the format and the query; the time is what a
network turns them into.

**Two targets.** The primary measurement (`--network-target simulated`, the
default) reads the prepared directory through `net-sim`, a local HTTP/1.1
server in `readbench/src/netsim.rs` that imposes a deterministic profile:

| Profile             | Bandwidth  | Latency per request |
| ------------------- | ---------- | ------------------- |
| `fast`              | 1,000 Mbps | 5 ms                |
| `typical` (default) | 100 Mbps   | 20 ms               |
| `slow`              | 20 Mbps    | 50 ms               |

The profiles are data in `manifest.toml` (`[network_profiles.*]`); select them
with `--network-profile fast,slow` or `all`, or give a custom one with
`--network-bandwidth-mbps` and `--network-latency-ms`. Bandwidth penalises
formats that download whole files; latency penalises formats that make many
range requests. The secondary target (`--network-target real --base-url <url>
--key-layout bucket`) reads real object storage, such as the hosted corpus at
`https://other-data.open3d.city/cityparquet-paper/benchmark/v8`; its result is a
snapshot of one network path at one time, not a repeatable measurement, and
its params sidecar's `network` block (copied into the run manifest) says so
(`snapshot`). That block records the base URL, the host and the addresses it
resolved to at run start, and, per object read, the `cf-cache-status` and `age`
response headers before the first and after the last measured request.
object_store does not expose those two headers, so they come from a one-byte
ranged `GET` per object, outside the measured section; the first of these
probes can itself fill the CDN's cache. Every request, measured or probe, sends
the corpus downloader's User-Agent (`cityparquet-bench-prep/1`), so the
benchmark reads with the identity that prepared its inputs.

**Measured and derived cells.** The indexed formats (`flatcitybuf`,
`cityparquet`) are measured on every query. The three text formats
(`citygml`, `cityjson`, `cityjsonseq`) fetch the whole object with one `GET`
for every query, so their bytes read and request count are the same for
every query, and so is most of their time: in the end-to-end runs on
Rotterdam and Vienna, their time for any query was within 0.92–1.15× of
read all, and the transfer was 83–94 % of it at `typical`. By default they
are therefore measured on read all and on the four identifier lookups, and
the other six queries (count, the three spatial windows, the attribute
filter, the attribute statistics) are emitted as derived rows. A derived
row is written only after the run has proven the premise for that format
and dataset: every sample of its read all and identifier miss, warm-ups
included, made exactly one request and read exactly the artefact's size
(an identifier hit cannot prove it: over HTTP, `cityjsonseq` and `citygml`
read the body as a stream and abandon the transfer at the hit, reporting
the bytes received; `cityjson` cannot stop early) (the served
file's length on the simulated network, the `Content-Length` of a `HEAD`
against a real target). A derived row copies bytes read and the request
count from the format's read all, leaves every time and memory field and
`result_count` empty, has `repeat` 0, and carries
`derived-from=full-read;status=derived` in `notes`. When the premise fails,
the run prints `WARNING: NOT DERIVED for <format>`, writes no derived rows
for that format and records the reason in the params sidecar's
`whole_file_derivation` block, which otherwise lists the derived cells.
The cross-format consistency check skips derived cells and says how many it
skipped. Which queries the text formats measure is set by
`[network] whole_file_scenarios` in `manifest.toml` (`full-read,id-lookup`
by default), overridden with `--network-whole-file-scenarios`:
`full-read,id-lookup` measures read all and the identifier lookups,
`full-read` derives the identifier lookups too, and `all` measures every
query. Over HTTP an identifier lookup in CityGML or CityJSONSeq still
downloads the whole object before it parses, so its early exit at the hit
saves parse time, not transfer; a single CityJSON document cannot stop
early at all.

**Run time.** The text formats' transfers dominate the family's run time.
With 4 samples per measured cell (one warm-up and 3 timed), the transfer
alone (`bytes * 8 / bandwidth + latency` per sample, parsing and the
indexed formats excluded) of the three text formats comes to:

| Dataset          | `fast`, all 11 queries → default 5 | `typical`     | `slow`        |
| ---------------- | ---------------------------------- | ------------- | ------------- |
| Rotterdam        | 0.1 → 0.1 min                      | 1.2 → 0.5 min | 5.9 → 2.7 min |
| Vienna           | 0.2 → 0.1 min                      | 1.8 → 0.8 min | 9.0 → 4.1 min |
| New York         | 4.6 → 2.1 min                      | 46 → 21 min   | 3.8 → 1.7 h   |
| Zurich           | 12 → 5.3 min                       | 1.9 → 0.9 h   | 9.6 → 4.4 h   |
| Tokyo            | 12 → 5.6 min                       | 2.1 → 0.9 h   | 10.3 → 4.7 h  |
| Montréal         | 24 → 11 min                        | 4.0 → 1.8 h   | 20 → 9.2 h    |
| 3DBAG slice (1M) | 1.7 → 0.8 h                        | 16.7 → 7.6 h  | 83 → 38 h     |
| Total            | 2.6 → 1.2 h                        | 25.5 → 11.6 h | 128 → 58 h    |

The slice's CityGML alone (10.8 GB) takes about 14 minutes per sample at
`typical`. Parsing adds about 6–20 % on top at `typical`, judging by the
transfer share above, and proportionally more on `fast`.

**Rendering.** `just bench-summary` discovers the network profile directories
and renders, per profile and dataset, a figure with three panels (time, bytes
read, HTTP requests; formats as columns, queries as rows, with the display
labels of the format figures and each value's factor against CityGML
coloured as in the format heatmaps), `network/network_factors.csv` with the
model time beside the measured statistic, and `network_bloom.csv` per profile
for the Bloom pair. A derived cell shows its bytes and requests with a `†`
and, in the time panel, "= read all transfer †" instead of a time; the
caption explains the mark. `network_factors.csv` marks such a row in its
`derived` column and leaves its time empty, and `bench_data.json` carries a
`derived` flag on every network record. A summary with no network results
states that the family was not measured.

**The simulation.** Each request waits the profile's latency before its first
response byte. Response bodies leave through ONE bandwidth budget shared by
every connection, so concurrent requests divide the bandwidth rather than
multiply it. The limiter is a virtual clock that paces 16 KiB pieces; an idle
link earns at most 2 ms of credit (the burst), which absorbs the timer's
resolution: 25 kB at 100 Mbps, 250 kB at 1 Gbps, 5 kB at 20 Mbps, sent at once
after an idle spell. Headers are not charged. The simulation leaves out TLS, HTTP/2,
connection set-up, jitter, packet loss and any throttling a storage service
applies, so the simulated times are a model of the transfer, not of a
particular provider.

**What is recorded.** Results go to
`runs/network/<suite-profile>/<network-profile>/<dataset>.csv` (the bloom pair
under `bloom/`), with the read CSV's columns. The params sidecar's `network`
block records the profile, the target, the bandwidth and latency, the server's
own request and body-byte totals and the clients' totals over every sample
(warm-up included); the two must be equal. `<dataset>.model.csv` puts each
row's model time, `bytes_read * 8 / bandwidth + http_requests * latency`,
beside the measured median. The simulated network is deterministic, so a cell
takes 3 timed samples after one discarded warm-up (`--network-repeat`), under
the usual per-cell time budget.

```sh
just bench-run --families network --profile short                     # typical
just bench-run --families network --profile short --network-profile all
just bench-run --families network --profile short --network-whole-file-scenarios all
just bench-run --families network --datasets rotterdam --network-target real \
  --base-url https://other-data.open3d.city/cityparquet-paper/benchmark/v8
```

## Selecting work

```sh
just bench-prep --families formats
just bench-run --families bloom
just bench-run --profile quick
just bench-run --profile short
just bench-run --datasets 3dbag --families formats
just bench-summary --data-root benchmark/runs
```

The family names are `sizes`, `formats`, `bloom` and `databases`. With no
selection, the suite includes all four. Use each command's `--help` for its
selection and output options. Smoke runs validate the pipeline with small
inputs and fewer repetitions; their results are not publication runs.

The bloom family's attribute lookups probe the text columns listed as
`bloom_attributes` under the slice's entry in `manifest.toml`; `just bench-run
--bloom-attributes COLUMNS` and the `ATTRIBUTES=` parameter of the
`bloom-bench` and `bloom-bench-http` recipes override the list, and a run
fails, naming the column, when one carries no Bloom filter. `just
bloom-columns PACKAGE` lists the columns of a package that do, with their
non-null and distinct counts and filter bytes ("The bloom family" in
`formats/README.md`).

Four run profiles decide which datasets are measured, how many repetitions
and where results land, so a test run can never overwrite the paper's
evidence: `--profile full` (the default; the six corpus datasets and the
3DBAG slice, 25 read repetitions, each family's own results directory, and
the database family measures the slice), `--profile quick` (the same
datasets as `full`, the database family's slice included, at 7 repetitions,
results under `<family>/quick/` and `databases/quick/`; a complete run in
about a third of the time, not the paper's evidence), `--profile short` (the corpus
without the slice, the same repetitions, results under `<family>/short/`; for
iterating on the harness in about an hour rather than a day) and `--profile
smoke` (`--smoke`: Rotterdam alone, one repetition, `<family>/smoke/`; a
pipeline check, not a measurement). Under `short` and `smoke` the database
family measures Rotterdam (the manifest's `small_database_dataset`) through
its prepared `rotterdam_delfshaven.city.jsonl`. Every run manifest records its
profile and its repetitions (`measurement.read_repeat` for the read families,
`execution.repeat` for the database family), every CSV row its samples in
`repeat`, and every timing figure's caption prints the warm runs it plots
("7 warm runs" for a quick run), so a quick run cannot be mistaken for the
25-repetition one. `bench-summary --profile quick` renders it into
`summary/quick/`.

On the earlier, smaller corpus a full run took about 23 h for the read
families and 8 h for the database family; at 7 repetitions the same work is
estimated at about 7 h and 2.5 h. Neither figure has been re-measured on the
current corpus.

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

Preparation fetches the six-file corpus (about 1.2 GB), cuts the 3DBAG slice
from the pinned 7.6 GB FlatCityBuf source (`just fetch-3dbag`), prepares all
required format artefacts (`readbench_prepare.sh`), builds the
release CityParquet CLI, and prepares the database environment. It needs Rust
and Cargo, Java 17 or later, `fcb`, `cjseq`, the pinned citygml-tools archive,
Python with `uv`, and a container engine (Apple `container`, `docker` or
`podman`, picked in that order) plus the database images for the database
family. The full run writes substantial prepared packages and result files
under the data root and takes hours; the database family also needs its own
container storage and available local ports. `bench-summary` only reads those
results.

Use `--families` to run one family after preparing it. `--datasets` accepts
manifest IDs, and `3dbag` for the slice (`[suite] slice_dataset` in
`manifest.toml`). The database family's dataset is the slice under `full` and
`quick` and Rotterdam under `short` and `smoke`; `--database-datasets
all|<ids>` selects its datasets explicitly. The selector rejects data roots outside
`benchmark/runs/`.

The `sizes` family also writes `compression.csv`, the breakdown of each
CityParquet package by column group, column and non-column part. `just
bench-compression [PREPARED] [OUT]` writes it for every package in a prepared
directory on its own, with a readable table on stdout. It reads Parquet
footers with DuckDB from `benchmark/databases`' uv project and measures no
time ([`formats/README.md`](formats/README.md#the-compression-breakdown)).

## Running on a shared host

The citable numbers come from a shared Linux host without root, so the read
families (`formats`, `bloom`, the variant runs) isolate their measured child
processes as far as an ordinary user can, and record every setting as applied
or "not applied: <reason>" in each CSV's `.params.json` (`isolation`) and in
the results directory's `MACHINE.md`. Off Linux nothing is applied and the
records say why.

| Setting        | Flag / recipe parameter                                | Default                                                                     | Applied by                                                                                                                                           |
| -------------- | ------------------------------------------------------ | --------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------- |
| NUMA node      | `--numa-node`, `NUMA_NODE` (env `BENCH_NUMA_NODE`)     | `auto`: the node with the most free memory at start                         | `numactl --physcpubind=<node cores minus the first> --membind=N`; else `taskset -c` (CPU only); the coordinator pins itself to the node's first core |
| Memory ceiling | `--memory-max`, `MEMORY_MAX` (decimal bytes, or `off`) | 64,000,000,000 (64 GB) under `full`, `quick` and `short`; off under `smoke` | `systemd-run --user --scope -p MemoryMax=`, probed once; a refusal is recorded, never fatal                                                          |
| Load gate      | `--max-load`, `MAX_LOAD`                               | `auto`: half the pinned node's cores                                        | before every sample, `load1 * node_cores / total_cores` above the threshold waits in 10 s steps                                                      |
| Longest wait   | `--max-load-wait-s`, `MAX_LOAD_WAIT_S`                 | 600                                                                         | a cell that proceeds while still contended is tagged `busy` in `notes`                                                                               |

One node rather than the whole machine keeps memory local and leaves the other
node to co-tenants. The load threshold is half the node because our own child
adds about one runnable task: half leaves room for it and light co-tenancy
while still catching a node that is genuinely contended. Each sample's
`load1`, runnable count and `MemAvailable` go to `.samples.json`, and each
cell's maxima to `.params.json`. `just bench-run` forwards the same settings
(`--numa-node`, `--memory-max`, `--max-load`, `--max-load-wait-s`) and records
them in the run manifest. Sizes in this repository are decimal: the ceiling's
64 GB is 64,000,000,000 bytes, and `--memory-max off` (`MEMORY_MAX=off` on the
recipes) runs without one. The database family receives the ceiling and
records it in its manifest, but does not apply it (see
[`databases/README.md`](databases/README.md), "Host isolation").

On Linux the page-cache pages a child reads are charged to its cgroup, so
the ceiling must stay above the largest artefact a reader opens plus the
largest reader's peak RSS: today about 10.8 GB for the slice's CityGML
artefact and about 35.5 GB peak RSS for the CityJSON reader on the slice,
measured in different cells. A ceiling below that would evict the warm
cache mid-cell or kill the reader. The first host run under the default must
confirm that warm runs stay warm under the ceiling.

The samples of one cell run back to back after its warm-up; formats are not
interleaved. What needs root, and is therefore not done: changing the CPU
governor, disabling SMT or turbo, dropping page caches, `isolcpus`, and a
cgroup `cpuset` for the database family's podman containers without user
delegation (`MACHINE.md` records whether the user cgroup delegates `cpuset`
and `memory`).

## Experimental matrix

| Family      | Data                        | Measurements                                                         | Read queries                                                                                                                                                                       |
| ----------- | --------------------------- | -------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `sizes`     | Corpus with the 3DBAG slice | Complete file or package size; CityParquet bytes by column           | None                                                                                                                                                                               |
| `formats`   | Same corpus                 | Read time and peak memory                                            | All format queries                                                                                                                                                                 |
| `bloom`     | The 3DBAG slice alone       | Size; lookup time, memory and row-group counters                     | `id-lookup` at `id-50pct`/`id-miss`; `feature-lookup` at `feature-50pct`/`feature-miss`; `attr-lookup` at `attr-<column>-50pct`/`attr-<column>-miss` per `bloom_attributes` column |
| `databases` | The 3DBAG slice             | Storage with and without indexes; query time and peak working memory | Database query suite                                                                                                                                                               |

The corpus is Rotterdam, Vienna, New York, Zurich, Tokyo (Chiyoda) and
Montréal, with the 3DBAG slice
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
([`formats/READ_BENCHMARK.md`](formats/READ_BENCHMARK.md), Caveat 14). The
corpus as a whole holds only content CityGML 2.0 can express (Caveat 43), so
it has no LoD 3 dataset with openings; Tokyo keeps 21 LoD 3 solids and 103
LoD 3 installation geometries.
For the same reason, preparation first normalises every CityJSON and
CityJSONSeq source so that each CityObject holds at most one geometry per LoD,
keeping the first in source order. CityParquet stores one geometry column per
LoD, so this is a property of the corpus, and all five artefacts derive from
the normalised source. Of the six corpus sources it changes Vienna only, and
the other five pass through byte for byte; the 3DBAG
slice is normalised the same way, and its preparation log reports how many
geometries were dropped (Caveat 41).

The format comparison measures five formats: `citygml`, `cityjson`,
`cityjsonseq`, `flatcitybuf` and `cityparquet`. CityParquet is one package per
dataset, its rows written in Hilbert-curve order and without LoD 0 synthesis
(`cityparquet convert --ordering hilbert --no-lod0`: Hilbert order is the
writer's default, pinned explicitly so the benchmark states its configuration,
and `--no-lod0` turns off the CLI's default LoD 0 synthesis so every format
holds the same geometries), displayed as **CityParquet**. The CityJSON
artefact is written without optional whitespace, like the CityGML one. The bloom experiment compares `cityparquet` with
`cityparquet+nobloom`, both Hilbert-ordered, and holds ordering, codec and
row-group size fixed. It measures the 3DBAG slice alone: a filter rules out
whole row groups, and at the default 65,536 rows per group every corpus
dataset but Zurich is a single row group, so a hit there can skip nothing.
The suite runs it under `full` and `quick`, the profiles that include the
slice.

## Figures

`just bench-summary` produces individual SVG and PNG files, the format
comparison's ratio tables as CSV, and a self-contained `index.html` collecting
the same figures, tables and conditions. The format comparison lives in its own
`formats/` folder of the figures directory, one sub-folder per dataset id.

File sizes and memory are in decimal units throughout: 1 MB = 10^6 bytes and
1 GB = 10^9 bytes, in figure labels, captions, tables and the derived CSV
columns alike. The CSVs keep the raw byte counts as the measurement; one helper,
`benchmark/plot/benchviz/units.py`, derives MB and GB from them for the
renderer and the scripts.

| Figure                                                  | Content                                                                                                                                                                        |
| ------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `formats/sizes`                                         | Vertical size bars, one subplot per dataset; actual sizes and factors against CityGML                                                                                          |
| `formats/<dataset>/time`, `formats/<dataset>/rss`       | One heatmap per dataset and metric: read time or read peak memory per query and format, with factors against CityGML                                                           |
| `formats/size_factors.csv`, `formats/size_extremes.csv` | Bytes and size factors against CityGML per dataset; the best and worst dataset by CityParquet's factor                                                                         |
| `formats/query_factors.csv`                             | Time and peak memory per dataset, query and format, with both factors against CityGML                                                                                          |
| `formats/compression.csv`                               | CityParquet bytes per dataset by column group, column and non-column part, copied from the `sizes` family's results ([breakdown](formats/README.md#the-compression-breakdown)) |
| `bloom`                                                 | Package size, the two read heatmaps and the row groups each lookup pruned for the 3DBAG slice; a configured lookup the run lacks reads "not measured"                          |
| `databases`                                             | Storage bars; time/memory heatmaps, `threads=single` and `threads=parallel` apart; the write tier as rows below the reads (`threads=single` only, Caveat 19 as a footnote)     |

Every results CSV carries the same seven-column timing block,
`time_mean_s,time_std_s,time_median_s,time_min_s,time_max_s,time_q1_s,time_q3_s`
(the database family adds a parallel `server_time_*` block). The summary plots
the median with the interquartile range by default; `just bench-summary
--statistic mean` plots the mean ± the population standard deviation instead,
and each caption names the statistic.

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
