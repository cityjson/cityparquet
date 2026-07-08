# CityParquet M5 benchmark artefacts

Results of the M5 variant-matrix benchmark: two CityJSON fixtures plus three
pinned 3DBAG tiles, each run through `cityparquet bench` (the 10-variant
default set, repeat = 5, median reported at 6-decimal precision) and through
the DuckDB `COPY ... TO (FORMAT PARQUET)` baseline (`scripts/bench_duckdb.sh`,
which appends `duckdb-copy` / `duckdb-copy-zstd` rows to the same CSVs at the
same precision and sample count).

Post-Codex-review measurement semantics (this run supersedes the original
M5 run): every `write_s` sample now times a CLEAN write — a fresh, empty
directory is created (and later deleted) OUTSIDE the timed window for every
repeat, matching the baseline script's `mktemp -d`-per-sample discipline —
where the original run's repeats 2..n also timed the purge/unlink of the
previous repeat's files. The default variant set gained
`cityparquet+rg512` and `cityparquet+hilbert+rg512` (the new `+rg<N>`
row-group-size override; initially ruled as `+rg4096`, but every committed
dataset has fewer than 4,096 rows, so rg4096 wrote a single row group
everywhere and demonstrated nothing — re-ruled to rg512 after the first
re-run exposed that). See "Row-group pruning" under Observations for what
the committed rg512 rows show.

`bench/results/*.csv` and this README are committed (they are the paper's
measurement artefacts); `bench/data/` (the downloaded 3DBAG tiles) is
gitignored and reproducible via `just bench-data`.

## Datasets

| dataset | source | objects | source bytes | gzipped download bytes |
|---|---|---:|---:|---:|
| `delft.city.jsonl` | CityJSON fixture (cityjson.org) | 2231 | 6,605,724 | — |
| `lod3_railway.city.json` | CityJSON fixture (cityjson.org) | 121 | 4,522,415 | — |
| `9-284-556.city.json` | 3DBAG v20250903, tile `9/284/556` — dense urban (Delft historic centre), RD bbox [84593, 445890]–[85593, 446890] | 2423 | 9,689,576 | 1,900,526 |
| `9-304-532.city.json` | 3DBAG v20250903, tile `9/304/532` — suburban (between Delft and Rotterdam), RD bbox [89593, 439890]–[90593, 440890] | 930 | 2,965,520 | 525,507 |
| `9-196-328.city.json` | 3DBAG v20250903, tile `9/196/328` — rural (Zeeland farmland), RD bbox [62593, 388890]–[63593, 389890] | 60 | 236,387 | 52,103 |

Object counts are the `object_count` column of the CSVs. The three 3DBAG
tiles were selected on 2026-07-07 from the live tile index
(`https://data.3dbag.nl/latest/tile_index.fgb`, queried with DuckDB
spatial); all three are zoom-level-9 (~1 km × 1 km) cells so gzipped
content-length is a like-for-like density proxy. Selection rationale and
the exact index queries are documented in `scripts/fetch_3dbag.sh`. All
tiles are gzipped whole-document CityJSON (EPSG:7415, quantised vertices).

## Environment

Captured 2026-07-08 (the machine the committed CSVs were produced on;
re-verified identical to the 2026-07-07 original-run capture):

```
uname -a: Darwin F19WYJD2P7 25.5.0 Darwin Kernel Version 25.5.0: Tue Jun  9 22:28:34 PDT 2026; root:xnu-12377.121.10~1/RELEASE_ARM64_T6041 arm64
CPU:      Apple M4 Max
RAM:      38654705664 bytes (36 GiB)
duckdb:   v1.5.3 (Variegata) 14eca11bd9
cargo:    cargo 1.93.1 (083ac5135 2025-12-15)
rustc:    rustc 1.93.1 (01f6ddf75 2026-02-11)
```

Full `just bench-all` wall time on this machine: ~120 s (excluding the
one-off release build and tile downloads; the original 8-variant repeat-3
run took 71 s — the increase is the two extra variants and repeat 3 → 5).

## Reproduce

```sh
just fixtures     # CityJSON fixtures into tests/fixtures/ (network)
just bench-data   # pinned 3DBAG tiles into bench/data/ (network)
just bench-all    # rewrites bench/results/*.csv (network: duckdb community ext)
```

`bench-all` deletes and rewrites the five result CSVs, so a run is always
one machine, one sitting. Per-dataset runs:
`cityparquet bench --input <file> --out <csv>` and
`./scripts/bench_duckdb.sh <file> <csv>`.

## Resolved: 9-284-556 round-trip

The dense-urban tile used to be benchmarked with `--skip-roundtrip`
(`roundtrip_equal` was left empty in its CSV): `cityparquet compare` on the
exported CityJSONSeq reported, verbatim:

```
object NL.IMBAG.Pand.0503100000025101-0: geometry at lod Some("2.2"): boundary/coordinates differ
object NL.IMBAG.Pand.0503100000025101-0: geometry at lod Some("2.2"): semantics differ: ...
```

(The `semantics differ` line carried both sides' full semantics JSON,
~48 KB; the two sides' `surfaces` arrays were identical element-wise and the
flattened `values` arrays differed in length by one — 736 vs 735 — with the
kept entries shifted from that point on.)

Diagnosis: face 497 of that Solid's LoD 2.2 shell has a 3-entry exterior
ring whose three *distinct* vertex indices `[49590, 49127, 49595]` all
quantise to the *same* vertex coordinate `(31653, 359040, -33533)`. Traced
end-to-end: the writer's ring normalisation (`crate::wkb_write::normalise_ring`)
is deliberately INDEX-based and does NOT drop this ring (3 distinct indices,
no writer-side drop); it is written as a real, if degenerate, WKB ring. The
WKB reader's coordinate interner then dedupes its 3 written points (bitwise
`f64::to_bits`, since all 3 dequantise identically) down to a single,
3-times-repeated pool index — and the comparator's OLD, also INDEX-based
normalisation treated that repeated-index shape as closed-to-nothing and
dropped it, but only on the round-tripped (exported) side, since the
SOURCE side's 3 distinct indices are untouched by an index-only check. That
one-sided drop (736 vs 735 faces) is what surfaced as the compare failure.

Fixed by extending the comparator's normalisation (`crates/cityparquet/src/compare.rs`)
to also drop a ring whose surviving indices dequantise to fewer than 3
DISTINCT coordinates (bitwise, exact — both sides quantise identically),
applied uniformly to BOTH sides and reusing the existing exterior-ring/
surface-drop and semantics/material/texture realignment machinery. The
writer is unchanged. All 2423 objects on this tile (including the
previously-failing one), and every object on the other four datasets, now
round-trip `true` — see `bench/results/9-284-556.csv`.

This fix additionally surfaced (and correctly drops) 8 previously-invisible
coordinate-degenerate rings in the `delft.city.jsonl` fixture and 20 more in
`lod3_railway.city.json` (real production data quirks the writer's
index-only check could never have caught either) — the M4/M5 round-trip
gates were updated to pin these newly-detected drops rather than their
former, incomplete zero count; see `crates/cityparquet/tests/roundtrip_real_data.rs`
and `crates/cityparquet/tests/convert_real_data.rs`.

## Baseline geometry coverage (duckdb-copy / duckdb-copy-zstd)

**The `duckdb-cityjson` extension does not populate every geometry column it
declares**, on every dataset benchmarked here, and on `lod3_railway.city.json`
it populates NONE at all. This was verified empirically (2026-07-07,
`duckdb` v1.5.3, `cityjson` community extension) with, for each geometry
column `read_cityjson`/`read_cityjsonseq` reports in its schema:

```sql
LOAD cityjson;
SELECT count(geom_lod0), count(geom_lod1_2), count(geom_lod1_3), count(geom_lod2_2), count(*)
FROM read_cityjsonseq('tests/fixtures/delft.city.jsonl');
-- 0 | 1116 | 1116 | 1116 | 2231

SELECT count(geom_lod), count(geom_lod3), count(*)
FROM read_cityjson('tests/fixtures/lod3_railway.city.json');
-- 0 | 0 | 121

SELECT count(geom_lod0), count(geom_lod1_2), count(geom_lod1_3), count(geom_lod2_2), count(*)
FROM read_cityjson('bench/data/9-284-556.city.json');
-- 0 | 1212 | 1212 | 1212 | 2423

SELECT count(geom_lod0), count(geom_lod1_2), count(geom_lod1_3), count(geom_lod2_2), count(*)
FROM read_cityjson('bench/data/9-304-532.city.json');
-- 0 | 465 | 465 | 465 | 930

SELECT count(geom_lod0), count(geom_lod1_2), count(geom_lod1_3), count(geom_lod2_2), count(*)
FROM read_cityjson('bench/data/9-196-328.city.json');
-- 0 | 30 | 30 | 30 | 60
```

| dataset | geometry column | non-null rows | total rows | coverage |
|---|---|---:|---:|---:|
| delft.city.jsonl | `geom_lod0` | 0 | 2231 | 0% |
| delft.city.jsonl | `geom_lod1_2` | 1116 | 2231 | 50% |
| delft.city.jsonl | `geom_lod1_3` | 1116 | 2231 | 50% |
| delft.city.jsonl | `geom_lod2_2` | 1116 | 2231 | 50% |
| **lod3_railway.city.json** | `geom_lod` | **0** | 121 | **0%** |
| **lod3_railway.city.json** | `geom_lod3` | **0** | 121 | **0%** |
| 9-284-556.city.json | `geom_lod0` | 0 | 2423 | 0% |
| 9-284-556.city.json | `geom_lod1_2` | 1212 | 2423 | 50% |
| 9-284-556.city.json | `geom_lod1_3` | 1212 | 2423 | 50% |
| 9-284-556.city.json | `geom_lod2_2` | 1212 | 2423 | 50% |
| 9-304-532.city.json | `geom_lod0` | 0 | 930 | 0% |
| 9-304-532.city.json | `geom_lod1_2` | 465 | 930 | 50% |
| 9-304-532.city.json | `geom_lod1_3` | 465 | 930 | 50% |
| 9-304-532.city.json | `geom_lod2_2` | 465 | 930 | 50% |
| 9-196-328.city.json | `geom_lod0` | 0 | 60 | 0% |
| 9-196-328.city.json | `geom_lod1_2` | 30 | 60 | 50% |
| 9-196-328.city.json | `geom_lod1_3` | 30 | 60 | 50% |
| 9-196-328.city.json | `geom_lod2_2` | 30 | 60 | 50% |

Two distinct patterns, both checked against the source CityJSON:

- **All four real datasets (delft + all three 3DBAG tiles) have `geom_lod0`
  fully NULL**, even though the source genuinely carries LoD0 geometries in
  every case — verified against the raw source JSON directly (not just
  DuckDB): 9-196-328's source carries exactly 30 LoD0 geometries (parsing
  every `CityObject`'s `geometry[].lod`), matching the 30 non-null rows
  `geom_lod1_2`/`geom_lod1_3`/`geom_lod2_2` each report for the SAME
  objects, yet DuckDB's own `geom_lod0` column for that identical data is
  0/60 non-null. This is a `duckdb-cityjson` extension limitation/bug
  specific to LoD `"0"`, uniform across every dataset here, NOT a per-tile
  data quirk. The `geom_lod1_2`/`geom_lod1_3`/
  `geom_lod2_2` 50%-populated pattern, by contrast, is expected and legitimate:
  each 3DBAG/delft Building's geometry lives on its `BuildingPart` children,
  not the parent `Building` object itself, so exactly half of each dataset's
  CityObjects (the parents) carry no geometry at all — this is a real,
  correct CityJSON data-model split, not a coverage bug.
- **`lod3_railway.city.json` is fully NULL on BOTH its geometry columns** —
  `geom_lod` (the generic column for objects whose source `lod` is absent,
  e.g. `SolitaryVegetationObject`) and `geom_lod3` (the column for the 105
  objects with a real `lod: "3"` `MultiSurface`/`Solid` geometry, confirmed
  against the raw source JSON). The extension writes **zero** geometry for
  this file: every `duckdb-copy`/`duckdb-copy-zstd` row in
  `bench/results/railway.csv` is a Parquet file containing attribute
  columns only, no boundaries, no vertices.

**Consequence: railway's `duckdb-copy`/`duckdb-copy-zstd` rows in
`bench/results/railway.csv` are NOT COMPARABLE to any cityparquet-rs
variant's bytes or write time** — they measure writing zero geometry for
121 objects, not a real competing encoding of the same data. The rows are
kept in the CSV (removing data from a committed measurement artefact would
be worse than disclosing it), but no cross-format byte/time comparison is
drawn from them anywhere in this document. The 3DBAG-tile `duckdb-copy`
comparisons that DO appear below are against baselines missing LoD0
entirely (see above) — a partial-geometry comparison, not a full one.

## Observations (numbers only)

Sub-10ms deltas are within scheduler noise at repeat=5 and are not cited.

Smallest `total_bytes` per dataset, cityparquet-rs variants
(vs the `cityparquet` preset):

| dataset | winner | bytes | `cityparquet` bytes |
|---|---|---:|---:|
| delft | `cityparquet+hilbert` | 2,295,504 | 2,337,653 |
| railway | `no-bss` | 1,377,944 | 1,378,783 |
| 9-284-556 | `no-dictionary` | 2,706,579 | 2,747,936 |
| 9-304-532 | `no-dictionary` | 782,329 | 797,336 |
| 9-196-328 | `no-dictionary` | 115,931 | 118,992 |

The harness's own `write_s` (every cityparquet-rs row below) times one
whole `convert()` call into a fresh, empty directory created outside the
timed window: input scan, `RowOrder::Hilbert`'s buffer-and-sort pass when
that variant enables it, the Parquet/sidecar encode, and the crash-safe
`commit_package` rename swap — uniformly, for every variant, so
variant-to-variant deltas below isolate the codec/layout/ordering choice
itself rather than a measurement artefact.

- `snappy` is the largest on every dataset (delft 3,720,228; railway
  2,544,962; 9-284-556 4,424,207; 9-304-532 1,195,769; 9-196-328 159,096)
  and records the lowest cityparquet-rs `write_s` on 4 of 5 datasets — but
  its margin over the ZSTD variants is under 10 ms everywhere (largest:
  delft, 0.118409 s vs the ZSTD variants' 0.124565–0.132224 s), so no
  speed ranking is cited from it.
- `write_s` spread across the nine ZSTD-based cityparquet-rs variants is
  under 12 ms on every dataset (delft 0.124565–0.132224 s; railway
  0.145628–0.157061 s; 9-284-556 0.179308–0.186732 s; 9-304-532
  0.053040–0.057616 s; 9-196-328 0.006265–0.008268 s) — i.e. within or
  barely above noise; the honest summary is that the
  recipe/ordering/layout choice does not measurably change write time at
  these dataset sizes.
- Row-group pruning (`+rg512` rows; the review's original `+rg4096`
  ruling wrote a single group everywhere — every committed dataset has
  fewer than 4,096 rows — and was re-ruled to rg512, at which the two
  larger datasets genuinely split into 5 groups):
  - delft (5 groups): the 5% window touches **2 of 5 in source order but
    1 of 5 under Hilbert** (`window_query_s` 0.005681 s vs 0.002508 s) —
    Hilbert ordering genuinely improves pruning here.
  - 9-284-556 (5 groups): the window touches **1 of 5 both with and
    without Hilbert** (0.002808 s vs 0.002572 s) — a 5x pruning win over
    the single-group layout's full-file read, but Hilbert adds nothing on
    this tile: 3DBAG's source feature order is already spatially
    coherent, itself a citable observation.
  - 9-304-532 (930 rows, 2 groups): source order touches **2 of 2** (no
    pruning) but Hilbert touches **0 of 2** — the tighter per-group bbox
    statistics Hilbert produces exclude the lower-left 5% window
    entirely (`window_query_s` 0.004424 s vs 0.000168 s).
  - railway (121 rows) and 9-196-328 (60 rows) are below the 512-row
    group size, so their `+rg512` rows stay single-group and
    byte-identical to their unsuffixed counterparts — grammar/parity
    only.
  - Cost disclosure: smaller row groups cost bytes and full-scan time —
    delft `cityparquet+rg512` is 2,456,530 B vs `cityparquet`'s
    2,337,653 B (+5.1%) with `full_scan_s` 0.012029 s vs 0.008864 s;
    9-284-556 is 2,876,525 B vs 2,747,936 B (+4.7%) with 0.013451 s vs
    0.008660 s. Pruning gains are bought with compression/scan overhead.
  - The other multi-row-group rows remain `cityparquet+by-type` (delft
    2/2, railway 14 total / 4 touched, tiles 2/2).
- `cityparquet+by-type` is larger than `cityparquet` on every dataset
  (delft +28,755 B; railway +119,832 B; 9-284-556 +28,501 B; 9-304-532
  +35,590 B; 9-196-328 +34,730 B).
- duckdb-copy baseline (schema differs: boundaries as JSON text, no bbox
  column, no window query — see `scripts/bench_duckdb.sh` header for what
  is and is not comparable, and the "Baseline geometry coverage" section
  above for what geometry it does and does not actually contain): `write_s`
  0.390594/0.666161/0.271854/0.097547 s
  (delft/9-284-556/9-304-532/9-196-328), each sample carrying ~0.076 s
  process + `LOAD cityjson` overhead (disclosed, undeducted; see the
  `# calibration:` stderr line). Baseline-vs-harness write deltas are well
  above the 10 ms noise floor on every dataset (e.g. delft 0.390594 s vs
  0.132224 s). ZSTD baseline bytes: delft 838,144; 9-284-556 1,663,803;
  9-304-532 517,762; 9-196-328 69,628. SNAPPY baseline bytes on 9-284-556
  (2,777,385) exceed every cityparquet-rs variant except `snappy`. **All
  four of these numbers are against a baseline missing LoD0 entirely**
  (every tile's `geom_lod0` is 0% covered — see above), so they compare
  cityparquet-rs's FULL geometry (all LoDs) against a baseline carrying
  only 3 of 4 LoDs; a hypothetical complete baseline would be larger
  still, so this is not an overstatement in cityparquet-rs's favour, but
  it is not a like-for-like byte comparison either. **Railway's `write_s`
  (0.314682 s) and ZSTD bytes (8,450 B) are excluded from this list and
  from all comparison above: the baseline coverage table shows the
  extension writes zero geometry for railway, so those numbers describe an
  attribute-only Parquet file, not a competing encoding of the same data —
  see "Baseline geometry coverage" above.**
- Baseline `full_scan_s` (0.058130–0.073517 s) is dominated by the same
  per-process overhead; the harness's in-process `full_scan_s` is
  0.001229–0.013451 s across all variants and datasets.
