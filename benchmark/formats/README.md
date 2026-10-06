# CityParquet size and configuration benchmark methodology

The **size and configuration** side of the benchmark suite: how many bytes each
format occupies on disk, and how a CityParquet package's reads and size move
with bloom filters, the writer option the suite varies. Its read-side
counterpart, and the cross-format comparison, is
`benchmark/formats/READ_BENCHMARK.md`.

The suite does not time writes. Not every format has a native writer, so a
format's file is produced by converting through a common format, and a
conversion time would not compare like with like. Preparing an artefact
(`readbench_prepare.sh`, `convert-all`, the variant packages the bloom family
builds) is never a measurement.

**The committed evidence is `benchmark/runs/formats/results/`**, where the
`formats` family writes its per-dataset read rows and its package `sizes.csv`,
alongside the bloom family's `bloom_results/`; both carry a
`MACHINE.md` describing the host they were measured on. What the figures cite
is the ratios within a single directory. Nothing in this document quotes a
number, so the methodology here cannot go stale against a re-run; the CSVs
themselves can.

**The committed evidence was measured on packages that carry bloom filters,
and reports one timing statistic.** Both disclosures are
[`READ_BENCHMARK.md`](READ_BENCHMARK.md)'s fairness caveats 30 and 31, which
is where every family's caveats are kept: `benchviz` renders that one numbered
list onto the summary page, so a caveat written only here would never reach a
reader of the figures.

## The corpus

Seven city datasets and one 3DBAG slice. Each CityJSON source is pinned
by byte size and sha256 in `benchmark/scripts/fetch_benchmark.sh`, and
`corpus_urls.txt` records its provenance; every one is also mirrored at
`https://pub-7aad9a74319741828dbafdbf5e2df201.r2.dev/cityparquet-paper/benchmark/cityjson20/<id>.city.json`,
with the CityGML 2.0 the benchmark synthesises from it at `…/citygml20/<id>.gml`
(a convenience copy: the benchmark synthesises its own with the pinned
citygml-tools, identical once the tool's random `ID_<uuid>`s are masked).

| id                     | Dataset             | Source                                                                                                                    | `attr-filter`                                  | `attr-stats`                                   |
| ---------------------- | ------------------- | ------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------- | ---------------------------------------------- |
| `rotterdam_delfshaven` | Rotterdam           | <https://3d.bk.tudelft.nl/opendata/cityjson/3dcities/v2.0/3-20-DELFSHAVEN.city.json>                                      | `TerrainHeight >=` its 0.75 quantile (2.45)    | `TerrainHeight`                                |
| `ingolstadt`           | Ingolstadt          | <https://3d.bk.tudelft.nl/opendata/cityjson/3dcities/v2.0/Ingolstadt.city.json>                                           | `klumMaterialClass == "Wood"`                  | `materialUncertainty`                          |
| `vienna_102081`        | Vienna              | <https://3d.bk.tudelft.nl/opendata/cityjson/3dcities/v2.0/Vienna_102081.city.json>                                        | `roofType == "FLACHDACH"`                      | `measuredHeight`                               |
| `nyc_da13_buildings`   | New York            | <https://3d.bk.tudelft.nl/opendata/cityjson/3dcities/v2.0/DA13_3D_Buildings_Merged.city.json>                             | `BIN == "1000000"`                             | none (no numeric attribute)                    |
| `zurich_building_lod2` | Zurich              | <https://3d.bk.tudelft.nl/opendata/cityjson/3dcities/v2.0/Zurich_Building_LoD2_V10.city.json>                             | `class == "BB01"`                              | `GebaeudeStatus`                               |
| `tokyo`                | Tokyo (Chiyoda)     | <https://pub-7aad9a74319741828dbafdbf5e2df201.r2.dev/cityparquet-paper/benchmark/cityjson20/tokyo.city.json>              | `usage == "401"`                               | `measuredHeight`, `-9999` placeholder included |
| `montreal`             | Montréal            | <https://pub-7aad9a74319741828dbafdbf5e2df201.r2.dev/cityparquet-paper/benchmark/cityjson20/montreal.city.json>           | `measuredHeight >=` its 0.75 quantile (15.543) | `measuredHeight`                               |
| `3dbag_n1000000`       | 3DBAG (Netherlands) | cut by `just fetch-3dbag` from <https://flatcitybuf.open3d.city/data/3dbag_subset2_all_index.fcb>, without LoD 1.2 | `b3_dak_type == "slanted"`                     | `b3_bag_bag_overlap`                           |

The predicates are declared in `benchmark/readbench/src/params.rs`
(`HAND_PICKED`, `HAND_PICKED_STATS`); a quantile is computed from the data when
the parameters are derived, and the run's parameter sidecar records the value.
Tokyo's `measuredHeight` holds PLATEAU's `-9999` "not measured" placeholder on
1,041 of its 38,743 values, and it is aggregated with the placeholder in: no
runner has an exclusion predicate, and adding one would change the timed work.

Tokyo and Montréal are derived by this project; `corpus_urls.txt` gives every
step, and `benchmark/scripts/cityjson_merge.py` rebuilds either byte for byte
from its inputs:

- **Tokyo** — PLATEAU Chiyoda-ku 2025 `bldg` (CC BY 4.0), 21 CityGML tiles
  (their sha256s in `tokyo_sources.sha256`). Texture `mimeType` `image/jpg` is
  normalised to `image/jpeg`; `citygml-tools to-cityjson --vertex-precision=10`
  converts each tile, skipping the `uro` ADE; the merge re-quantises `z` to
  1e-6 m, because FlatCityBuf stores 32-bit integer vertices and a uniform
  1e-10 overflows on height, giving a transform scale of
  `[1e-10, 1e-10, 1e-6]`. `creationDate` values carry the converting
  machine's UTC offset (`+01:00` or `+02:00`).
- **Montréal** — exported from this project's published CityParquet packages
  of three boroughs (`https://cityparquet.open3d.city/data/montreal-2020/collection.json`,
  `cityparquet:version` 0.1.0-draft) with the CLI at monorepo commit `2f042db`,
  merged with the scale refined to `[1e-5, 1e-3, 1e-6]` and every texture
  given type `JPG` from its file extension. **This dataset derives from
  CityParquet**, by the author's decision — the one exception to the
  preparation chain's rule (`READ_BENCHMARK.md`, Caveat 14).

## Running the suite

Use the root entry points:

```sh
just bench-prep --families formats,sizes,bloom
just bench-run --families formats,sizes,bloom
just bench-summary
```

The `formats` family compares read performance across file formats, and the
`sizes` family records each format's file or package size. The `bloom` family
holds the format fixed and changes one configuration dimension over the
city datasets and the 3DBAG slice. Its read scenarios are the identifier
lookups described under "The bloom family" below.

The format comparison measures five formats — CityGML, CityJSON, CityJSONSeq,
FlatCityBuf and CityParquet — with one artefact each per dataset. The
CityParquet artefact is one package written in Hilbert-curve order
without LoD 0 synthesis (`cityparquet convert --ordering hilbert
--no-lod0`: both the writer's defaults, pinned explicitly so the benchmark
states its configuration and every format holds the same geometries), and the
CityJSON artefact is written without optional whitespace; displayed as
**CityParquet** in figures. The bloom figures are `bloom`,
for the 3DBAG slice, and `bloom-corpus`, for the corpus datasets.
See [`../README.md`](../README.md) for the experimental matrix and figure list.

## Measurement discipline

- **Sizes are on-disk bytes.** A file's size is its length; a CityParquet
  package's is the sum of every file in its directory
  (`benchmark/scripts/measure_sizes.py` for the format comparison, the
  coordinator's `sizes.csv` for the bloom family).
- **Seven timing statistics over `repeat` read samples** — default 25, run back to back after one discarded warm-up; see `READ_BENCHMARK.md` "Sampling" for the optional cell time budget —
  reported at 6-decimal precision: `time_mean_s`, the **population standard
  deviation** `time_std_s` (the warm repeats are the whole measured set, not
  a draw used to infer a wider one), `time_median_s`, `time_min_s`,
  `time_max_s`, `time_q1_s` and `time_q3_s` (median and quartiles by linear
  interpolation at `p * (n - 1)`). The block is the one
  `benchmark/databases` reports, and `READ_BENCHMARK.md` states the same
  contract.
- **Sub-10 ms deltas are noise** at these repeat counts and are not findings on
  their own — the same floor `benchmark/formats/READ_BENCHMARK.md` applies.

## The bloom family

`just bloom-bench` (via `just bench-run --families bloom`) builds two packages
from each input, untimed — `cityparquet`, which carries bloom filters, and
`cityparquet+nobloom`, which carries none — and times `id-lookup` (`id-50pct`,
`id-miss`) and `feature-lookup` (`feature-50pct`, `feature-miss`) against
both. Package bytes go to `sizes.csv`. Every lookup row carries `row_groups_total`, `bloom_pruned` and `filter_bytes` (the
bitset bytes of the filters examined). Both packages are written in Hilbert
order: the coordinator builds every variant package that way and refuses a
`+source` suffix, so the two differ in their bloom filters alone.
The 3DBAG slice is drawn in `bloom`; the corpus datasets are drawn per dataset
in `bloom-corpus`.

Over HTTP, `just bloom-bench-http FOLDER BASE_URL` reads — never builds — the
two packages a local `bloom-bench` run left in the prepared directory, once that
directory is uploaded to `BASE_URL` (`benchmark/scripts/readbench_upload.md`).
Its rows add `bytes_read` and `http_requests`; the results go to
`bloom_http_results/` and are not part of `bench-run` or the rendered
summary. They are a snapshot of one network path at one time.

The caveats that travel with every one of its numbers are
[`READ_BENCHMARK.md`](READ_BENCHMARK.md)'s fairness caveats **24 to 29** —
the row-group scale a table has to reach before its pruning means anything, a
positive not being a match, the twice-read footer, the single-table
restriction, what the `feature-*` probes are, and requests being logical. They
live there, not here, because that numbered list is the one `benchviz` renders
onto the summary page beside the figures.

## Reproduce

```sh
just bench-prep --families bloom
just bench-run --families bloom
just bench-summary
```

Add `--smoke` for a small pipeline check, or `--profile short` for the corpus
without the slice. Full experiments use the corpus datasets and the 3DBAG
slice, cut without LoD 1.2 so that every format holds the same geometry
(`READ_BENCHMARK.md`, Caveat 14); its actual count is recorded because a
feature boundary can cross the nominal target. Keep machine metadata, source identity, software
revision, query parameters and repetition settings alongside the results.
Prepared data and result directories have separate responsibilities: preparing
an artefact is never a measurement.
