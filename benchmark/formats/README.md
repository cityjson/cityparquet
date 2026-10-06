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

Every CityJSON/CityJSONSeq source is normalised before any artefact is built,
so that each CityObject holds at most one geometry per LoD (the first in source
order) and all five formats hold the same geometries (`READ_BENCHMARK.md`,
Caveat 41). Two datasets change: `vienna_102081` loses 1,102 of its 2,204
geometries (each LoD 2 object carries a MultiSurface and a Solid) and
`ingolstadt` 26 of its 405. The other five are used byte for byte, and the 3DBAG
slice, already cut without LoD 1.2, needs no change either. A CityGML source
would not be normalised; none is in the corpus.

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
--no-lod0`: Hilbert order is the writer's default, pinned explicitly so the
benchmark states its configuration, and `--no-lod0` turns off the CLI's
default LoD 0 synthesis so every format holds the same geometries), and the
CityJSON artefact is written without optional whitespace; displayed as
**CityParquet** in figures. The bloom figures are `bloom`,
for the 3DBAG slice, and `bloom-corpus`, for the corpus datasets.
See [`../README.md`](../README.md) for the experimental matrix and figure list.

## Measurement discipline

- **Sizes are on-disk bytes.** A file's size is its length; a CityParquet
  package's is the sum of every file in its directory
  (`benchmark/scripts/measure_sizes.py` for the format comparison, the
  coordinator's `sizes.csv` for the bloom family). Both write one
  `sizes.csv` schema, `dataset,format,bytes,mb_decimal`: the byte column is
  the measurement and `mb_decimal` is bytes / 10^6 (1 MB = 10^6 bytes), for
  reading the file by eye. Ratios between formats are derived from the bytes
  by the renderer, in one place.
- **The compression breakdown says where a package's bytes go.**
  `benchmark/scripts/compression_contribution.py` (`just bench-compression
  [PREPARED] [OUT]`, and part of the `sizes` family) reads every object table
  of each prepared package with DuckDB's `parquet_metadata()` and writes
  `compression.csv`. See [The compression breakdown](#the-compression-breakdown).

## The compression breakdown

The breakdown is measured from the Parquet footers of the packages the
`sizes` family measured, so it changes only when the packages do. DuckDB
comes from `benchmark/databases`' uv project. For each dataset it sums, over
every row group and every object table, the column chunks'
`total_compressed_size` and `total_uncompressed_size`. A nested column (a
struct, list or map) counts towards its top-level column, and each top-level
column belongs to one group by its specification name:

| Group                   | Columns                                                                     |
| ----------------------- | --------------------------------------------------------------------------- |
| `geometry`              | `geometry_lod*`                                                             |
| `geometry_properties`   | `geometry_properties_lod*`                                                  |
| `appearance`            | `material_lod*`, `texture_lod*`                                             |
| `attributes`            | `address` and every column that is not reserved                            |
| `identifiers_structure` | `id`, `feature_id`, `object_type`, `parents`, `children`, `children_roles` |
| `bbox`                  | `bbox`                                                                      |
| `other`                 | `other`, `implicit_geometry`                                                |

**"Uncompressed" is the size after encoding and before the codec.**
`total_uncompressed_size` counts the column's pages once dictionary, delta or
run-length encoding has been applied. For a plain byte-array column, such as
the WKB in `geometry_lod*`, that is close to the raw values. For a
dictionary-encoded column it is already the encoded size, so
`compression_ratio` (uncompressed / compressed) understates the total saving
over the raw values. Each column row lists the encodings its chunks use, so a
dictionary-encoded column is visible as `RLE_DICTIONARY`.

**Everything that is not column data is a part**, so the column groups and
the parts add up to the package size in `sizes.csv`:

| Part                      | Bytes                                                                                  |
| ------------------------- | -------------------------------------------------------------------------------------- |
| `bloom_filters`           | The object tables' Bloom filters (`bloom_filter_length`)                               |
| `footer_and_page_indexes` | The rest of each object table: footer, column and offset indexes, magic bytes          |
| `sidecar_tables`          | The `cityparquet-sidecar` assets of `metadata.json` (materials, textures, templates), whole files |
| `metadata`                | `metadata.json`                                                                        |
| `other_files`             | Any other file in the package directory                                                |

`compression.csv` has the columns
`dataset,status,level,name,group,encodings,compressed_bytes,uncompressed_bytes,compressed_mb_decimal,share_of_column_bytes,share_of_package_bytes,compression_ratio`.
`level` is `group`, `part`, `package` (the total) or `column`; `encodings`
is filled for columns. The two shares are against the package's compressed
column bytes and against the whole package, and parts have no uncompressed
size, ratio or column share. A dataset whose package is missing gets one row
with status `missing` and empty values, and the script exits 1. The `sizes`
family writes the file beside `sizes.csv`, and `just bench-summary` copies it
to `<figures>/formats/compression.csv`, beside `size_factors.csv`.
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

`just bloom-bench` (via `just bench-run --families bloom`) measures the 1M
3DBAG slice alone. A Bloom filter rules out whole row groups, and at the
writer's default 65,536 rows per group only the slice (16 groups) and Zurich
(4) span more than one; on every other corpus dataset the table is a single
row group, so a hit can skip nothing. The suite therefore runs the family
under the profiles that include the slice (`full`, `quick`) and not under
`short` or `smoke`. Even on the slice a hit still reads one whole row group,
because a filter cannot narrow the search inside a group, and a miss can still
read a group through a false positive at the filters' 1 % target rate. The
benefit depends on the row-group size, which the benchmark does not vary: both
packages use the writer's default. `cityparquet+nobloom` still carries
row-group min/max statistics, so the axis measures what the filters add on top
of the statistics, and both pruning counts are recorded.

The recipe builds two packages from the slice, untimed — `cityparquet`, which carries bloom filters, and
`cityparquet+nobloom`, which carries none — and times `id-lookup` (`id-50pct`,
`id-miss`) and `feature-lookup` (`feature-50pct`, `feature-miss`) against
both. Package bytes go to `sizes.csv`. Every lookup row carries `row_groups_total`, `bloom_pruned`, `stats_pruned`
(the row groups min/max statistics ruled out among those the filters kept)
and `filter_bytes` (the bitset bytes of the filters examined), the last four
of the CSV's 22 columns. Each miss probe is a stored identifier with
`-readbench-absent` appended, inside the row groups' identifier ranges, so
statistics alone cannot reject it. Both packages are written in Hilbert
order: the coordinator builds every variant package that way and refuses a
`+source` suffix, so the two differ in their bloom filters alone.
The `bloom` figure draws the slice.

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
