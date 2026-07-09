# CityParquet corpus benchmark report

A full-corpus run of the three benchmarks — cross-format **read** performance
(`just bench`), **file size / compression ratio** (`just sizes`), and the
**compression-codec / row-group-size** sweep (`just compression-bench`) —
over the CityJSON benchmark corpus. This aggregates the per-dataset artefacts
in `bench/read_results/*.csv`, `bench/read_results/sizes.csv`, and
`bench/compression_results/*.csv`; the referenced charts are regenerable with
`just plot` / `just sizes` / `just compression-plot`.

Captured 2026-07-09 on the machine in `bench/READ_BENCHMARK.md`'s environment
block (Apple M4 Max, macOS, duckdb 1.5.3). Read the caveats there — they all
apply here — plus the corpus-specific ones at the end of this document.

## Datasets

Eight corpus datasets spanning three orders of magnitude, plus the two
committed fixtures and one 3DBAG tile already benchmarked:

| dataset | raw CityJSONSeq | CityObjects |
|---|---:|---:|
| Rotterdam | 2.7 MB | 853 |
| Ingolstadt | 3.8 MB | 379 |
| Railway | 4.3 MB | — |
| Montreal | 4.6 MB | — |
| Vienna | 4.8 MB | 1,322 |
| 3DBAG | 5.9 MB | 2,221 |
| NYC | 95 MB | 23,777 |
| Zurich | 247 MB | 198,699 |

**Excluded (scalability limit, not a bug):** 3DBV (333 MB), Helsinki (432 MB),
and Helsinki_tex (675 MB) are omitted from the cross-format read matrix. The
row/record formats (CityJSONSeq, gzipped CityJSONSeq) must fully parse the
document for **every** scenario and repeat, so at 300 MB+ a single run costs
minutes and gigabytes of RAM — which is itself the finding in §1: line-
delimited CityJSONSeq does not scale to large tiles for repeated querying.

## 1. Cross-format read performance

Per-dataset charts: `bench/read_results/plots/<dataset>-time.png` (median time
per scenario, grouped by format, log-scaled) and `<dataset>-mem.png` (peak
heap). Cross-dataset full-read summary: `summary-full-read.png`.

### The headline: metadata queries are O(1) for CityParquet, O(n) for CityJSONSeq

`count` returns the object count. CityParquet and FlatCityBuf read it from
file/index metadata in constant time; CityJSONSeq must scan and parse every
feature. The cost gap **grows linearly with dataset size**:

| dataset | CityObjects | CityJSONSeq `count` | CityParquet `count` | gap |
|---|---:|---:|---:|---:|
| Rotterdam | 853 | 0.013 s | 0.00021 s | ~62× |
| Ingolstadt | 379 | 0.019 s | 0.00025 s | ~75× |
| Vienna | 1,322 | 0.022 s | 0.00020 s | ~112× |
| 3DBAG | 2,221 | 0.027 s | 0.00024 s | ~110× |
| NYC | 23,777 | 0.475 s | 0.00022 s | ~2,160× |
| **Zurich** | **198,699** | **1.076 s** | **0.00022 s** | **~4,900×** |

CityParquet's `count` is flat (~0.0002 s) across a 230× range of object
counts; CityJSONSeq's climbs from 13 ms to over a second. The same holds for
`attr-stats` and `project` (columnar stats / single-column reads vs full
parse). This is the core cloud-native argument: a discovery/aggregation query
over a national dataset is instant on CityParquet and linear on CityJSONSeq.

### Full read (materialise every feature)

Where every format must touch all data, the gap narrows and the tradeoffs
differ (NYC, 23,777 objects):

| format | full-read time | peak heap |
|---|---:|---:|
| FlatCityBuf | 0.31 s | 1.6 MB (streams) |
| DuckDB-over-Parquet | 0.17 s | — (out of process) |
| CityJSONSeq | 0.48 s | 3.5 MB |
| CityParquet | 0.55 s | 63 MB (decodes geometry) |

FlatCityBuf streams features with a tiny heap; CityParquet's columnar full
decode materialises more in memory. `time_s` is end-to-end (includes file
open) — see the caveats.

### Spatial & attribute queries

`bbox-query`, `attr-filter`, and `id-lookup` are where the indexed formats
pull ahead: FlatCityBuf uses its packed R-tree (bbox) and B-tree (attribute /
id) to seek; CityParquet prunes via bbox row-group statistics; CityJSONSeq
scans. See each dataset's `-time.png`. `attr-filter` result counts agree
across all formats at CityObject granularity (a cross-format consistency
check the harness enforces).

## 2. Storage & compression

Chart: `bench/read_results/plots/sizes.png` (MB per format) and
`compression-ratio.png` (ratio vs raw CityJSONSeq, higher = smaller).

CityParquet is **2.6–5.5× smaller than raw CityJSONSeq while remaining
queryable**:

| dataset | CityParquet size | ratio vs raw |
|---|---:|---:|
| Ingolstadt | 0.9 MB | 4.18× |
| Rotterdam | 0.8 MB | 3.53× |
| Vienna | 1.3 MB | 3.60× |
| Montreal | 1.7 MB | 2.74× |
| delft | 2.2 MB | 2.83× |
| 3DBAG | 2.2 MB | 2.64× |
| Zurich | 70.7 MB | 3.50× |
| **NYC** | **17.2 MB** | **5.54×** |

For comparison per dataset (see `sizes.csv`): gzipped CityJSONSeq is smaller
still (~4–5×) but is an **opaque blob** — no random access, no projection;
FlatCityBuf is at or above raw size (it carries spatial + attribute indexes).
So CityParquet captures most of gzip's size win *without* giving up queryability,
which is the storage tradeoff the paper argues for.

## 3. Compression codec & row-group size

Charts: `bench/compression_results/plots/<dataset>-codec-size.png`,
`-codec-time.png`, `-rowgroup.png`. Ran on the fixtures + 3DBAG, Ingolstadt,
Montreal, NYC, Railway, Vienna, Zurich (Rotterdam excluded — see caveats).

### Codec (NYC, 23,777 objects — the size↔speed tradeoff)

| codec | size | full read |
|---|---:|---:|
| uncompressed | 150.0 MB | 0.017 s (fastest read) |
| snappy | 30.2 MB | 0.042 s |
| lz4 | 30.1 MB | 0.034 s |
| brotli | 19.0 MB | 0.198 s (slowest read) |
| gzip | 17.9 MB | 0.076 s |
| **zstd (default)** | **17.2 MB** | **0.063 s** |

zstd (the CityParquet default) gives the best size here with a moderate read
cost; snappy/lz4 trade ~1.7× more bytes for ~1.5–2× faster reads; brotli
matches zstd's size but reads slowest. Write time barely moves between codecs
at these sizes. The default zstd is a good balance; snappy/lz4 suit read-hot
workloads, gzip/brotli/zstd suit storage-constrained ones.

### Row-group size (NYC — pruning vs overhead)

| row-group size | groups | window query touched | window-query time |
|---|---:|---:|---:|
| 65536 (default) | 1 | 1 / 1 (no pruning) | 0.061 s |
| 4096 | 6 | 6 / 6 (no pruning) | 0.066 s |
| **512** | **47** | **16 / 47 (34%)** | **0.030 s** |

At NYC scale, smaller row groups give real bbox pruning: `+rg512` splits into
47 groups, the 5% window touches only 16, and the windowed read is ~2× faster
than the single-group default — at a small size/scan cost (see `-rowgroup.png`).
Row-group size is the lever for spatially-selective query performance.

## Corpus-specific caveats

- **Three giants excluded** (3DBV/Helsinki/Helsinki_tex, ≥ 333 MB) from the
  read matrix — the row-format full-parse cost is prohibitive; documented in
  §Datasets as a scalability finding, not skipped silently.
- **Rotterdam is excluded from the compression benchmark.** Its Compatibility-
  profile conversion fails: a feature's geometry-template `material` map
  references index 2 while that feature declares only 2 local material
  definitions (`material index 2 out of range, local defs len 2`) — an
  out-of-range reference in the source appearance data. The Core-profile
  **read** benchmark (which does not resolve template appearance) runs
  Rotterdam fine; only the Compatibility path trips. Worth confirming whether
  this is malformed source data or a writer slicing bug before it reaches the
  paper.
- All the `bench/READ_BENCHMARK.md` caveats apply: counting granularity
  (feature vs CityObject for count/full-read/bbox), `time_s` is end-to-end
  read latency, `id-lookup` samples a table-order-first id (a lower bound for
  scan formats), FlatCityBuf `bbox` is 2D, `duckdb-parquet` has no heap
  figure.

## Reproduce

```sh
just fetch-data                       # corpus → bench/data/benchmark (network)
just bench <folder>                   # cross-format read benchmark + charts
just compression-bench <folder>       # codec / row-group sweep + charts
just sizes                            # size + compression-ratio report + charts
```

The committed CSVs under `bench/read_results/` and `bench/compression_results/`
are this run's measurement artefacts; the PNGs under `*/plots/` are gitignored
and regenerated by the plot recipes.
