# CityParquet M5 benchmark artefacts

Results of the M5 variant-matrix benchmark: two CityJSON fixtures plus three
pinned 3DBAG tiles, each run through `cityparquet bench` (the 8-variant
default set, repeat = 3, median reported) and through the DuckDB
`COPY ... TO (FORMAT PARQUET)` baseline (`scripts/bench_duckdb.sh`, which
appends `duckdb-copy` / `duckdb-copy-zstd` rows to the same CSVs).

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

Captured 2026-07-07 (the machine the committed CSVs were produced on):

```
uname -a: Darwin F19WYJD2P7 25.5.0 Darwin Kernel Version 25.5.0: Tue Jun  9 22:28:34 PDT 2026; root:xnu-12377.121.10~1/RELEASE_ARM64_T6041 arm64
CPU:      Apple M4 Max
RAM:      38654705664 bytes (36 GiB)
duckdb:   v1.5.3 (Variegata) 14eca11bd9
cargo:    cargo 1.93.1 (083ac5135 2025-12-15)
rustc:    rustc 1.93.1 (01f6ddf75 2026-02-11)
```

Full `just bench-all` wall time on this machine: 71 s (excluding the
one-off release build and tile downloads).

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

## Observations (numbers only)

Smallest `total_bytes` per dataset, cityparquet-rs variants
(vs the `cityparquet` preset):

| dataset | winner | bytes | `cityparquet` bytes |
|---|---|---:|---:|
| delft | `cityparquet+hilbert` | 2,295,504 | 2,337,653 |
| railway | `no-bss` | 1,377,944 | 1,378,783 |
| 9-284-556 | `no-dictionary` | 2,706,579 | 2,747,936 |
| 9-304-532 | `no-dictionary` | 782,329 | 797,336 |
| 9-196-328 | `no-dictionary` | 115,931 | 118,992 |

- `snappy` is the largest on every dataset (delft 3,720,228; railway
  2,544,962; 9-284-556 4,424,207; 9-304-532 1,195,769; 9-196-328 159,096)
  and the fastest cityparquet-rs writer on 4 of 5 datasets (delft 0.112 s;
  railway 0.139 s; 9-284-556 0.174 s; 9-304-532 0.052 s; 9-196-328 0.006 s,
  where five other variants also record 0.006 s).
- `write_s` spread across the seven ZSTD-based cityparquet-rs variants is
  small on every dataset (delft 0.119–0.124 s; 9-284-556 0.182–0.194 s;
  9-304-532 0.055–0.060 s).
- Hilbert row-group pruning: no effect at these sizes — every single-table
  variant writes `row_groups_total = 1` on every dataset (even the largest
  tile, 2423 objects), so `row_groups_touched` is 1 with or without
  Hilbert ordering. The only multi-row-group runs are `cityparquet+by-type`
  (delft 2/2, railway 14 total / 4 touched, tiles 2/2).
- `cityparquet+by-type` is larger than `cityparquet` on every dataset
  (delft +28,755 B; railway +119,832 B; 9-284-556 +28,501 B; 9-304-532
  +35,590 B; 9-196-328 +34,730 B).
- duckdb-copy baseline (schema differs: boundaries as JSON text, no bbox
  column, no window query — see `scripts/bench_duckdb.sh` header for what
  is and is not comparable): `write_s` 0.381/0.316/0.662/0.270/0.097 s
  (delft/railway/9-284-556/9-304-532/9-196-328), each sample carrying
  ~0.07 s process + `LOAD cityjson` overhead (disclosed, undeducted; see
  the `# calibration:` stderr line). ZSTD baseline bytes: delft 838,144;
  railway 8,450; 9-284-556 1,663,803; 9-304-532 517,762; 9-196-328 69,628.
  SNAPPY baseline bytes on 9-284-556 (2,777,385) exceed every
  cityparquet-rs variant except `snappy`.
- Baseline `full_scan_s` (0.058–0.071 s) is dominated by the same
  per-process overhead; the harness's in-process `full_scan_s` is
  0.001–0.010 s across all variants and datasets.
