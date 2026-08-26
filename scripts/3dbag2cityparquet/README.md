# 3dbag2cityparquet

Encode the whole of **3DBAG** — every tile, ~18.3 million rows — as **one
CityParquet package**.

`cityparquet convert` already merges many inputs into one package, so the
obvious command is `cityparquet convert tile1.json tile2.json … -o out`. It
does not work at this scale, and the reason is worth stating: `merge_sources`
materialises every input's features at once. Measured on 24 real tiles, peak
resident memory is **~20× the input JSON**, which over 3DBAG's ~53 GB is
**~1.1 TB**. No machine here has that.

The way through is that **CityJSONSeq sources stream**. `Source::features()`
reopens the file and yields line by line, and the writer makes two such passes
(scan for the schema, then encode) — only `--ordering hilbert` buffers. So the
whole dataset can be handed to the reference writer as **one CityJSONSeq file**,
and the writer computes the footer itself. Nothing here grafts metadata onto a
Parquet file after the fact; `cityparquet` remains the only thing that decides
what a CityParquet package contains.

## The one piece of arithmetic

3DBAG's 8,941 tiles are each quantised against their own `transform`. Turning
them into one CityJSONSeq means requantising onto a shared grid — exactly what
`merge_sources` does in memory. This reimplements **its** arithmetic so the
result is the same artefact by a different route:

```
merged.scale     = componentwise min of the inputs' scale
merged.translate = componentwise min of the inputs' translate
v' = round(v·(src.scale/merged.scale) + (src.translate−merged.translate)/merged.scale)
```

3DBAG makes it cheap: every tile declares `scale [0.001, 0.001, 0.001]`, so the
ratio is 1 and requantisation collapses to a per-tile, per-axis **integer add**.
`plan` asserts the shared scale rather than assuming it.

The subtlety is rounding. Rust rounds ties **away from zero**, and roughly one
3DBAG axis-shift in fourteen lands exactly on `.5` — the translates are dyadic,
so `Δtranslate/0.001` often is too. Writing `offset = m + f` with
`m = floor(offset)`, the add is `v + m` plus: `f < 0.5` → nothing; `f > 0.5` →
one more; `f == 0.5` → one more only where the result is non-negative. That last
case is the only per-vertex branch, and it is what keeps the output identical to
`merge_sources` rather than 1 mm away from it.

## The run, measured

3DBAG **v20250903**, 8,941 tiles, on a 128-core / 503 GB host:

| Stage     | Wall clock   | Output                                      |
| --------- | ------------ | ------------------------------------------- |
| `fetch`   | 1.5 min      | 11.2 GB of `.gz`                            |
| `plan`    | < 1 min      | shared scale, one CRS, merged transform     |
| `seq`     | 1.5 min      | 10,771,547 features, 62 GB, no empty tile   |
| `merge`   | 6 min        | one 65.9 GB `3dbag.city.jsonl`              |
| `convert` | **53.5 min** | 21,555,522 rows, 16.4 GB `building.parquet` |

**Peak resident memory during `convert`: 56 GB** — against the ~1.1 TB the
monolithic merge extrapolates to for the same data. That is the whole
justification for this pipeline, and it is a measured figure, not a code read.
Note it is not constant-memory: 56 GB is ~0.86× the input file, so the writer
still accumulates (schema scan, dictionaries, row-group buffers). It is
sub-linear enough to fit, which is what was needed.

The result:

```
rows           21,555,522   (10,771,547 Building + 10,783,975 BuildingPart)
distinct ids   21,555,522   — unique, so no tile duplicates another's objects
row groups     329
extent (RD)    x 13603.3 … 277924.3   y 306900.4 … 612658.0   z −2147483.6 … 345.6
LoD columns    lod0_0 21,554,891 · lod1_2 10,783,939 · lod1_3 10,783,956 · lod2_2 10,783,975
footer         city 0.1.0-draft, 62 attributes, CRS 'Amersfoort / RD New + NAP height'
               geo 1.1.0, primary_column geometry_lod0_0
```

`zmax 345.61` and the `−2147483.75` sentinel both match the published
`netherlands-3d-bag` STAC collection extent exactly — independent evidence that
the conversion carries the source faithfully.

## Equivalence, checked

Against a direct `cityparquet convert` of the same 24 tiles — all 88 columns,
geometry WKB included, compared as row multisets:

```
columns 88  rows A=49134 B=49134  A-only=0  B-only=0
```

Reproduce it by converting a handful of tiles both ways and diffing; the tie
handling above is what that test is really checking.

Getting there required one fix in the library. `serde_json` was used **without
its `float_roundtrip` feature**, so its fast float parser is not correctly
rounded: an attribute written `28.184951782226562` was stored as the double one
ULP below it. That is a conversion silently not preserving the value it was
handed, and it affects `cityparquet convert` generally — this pipeline only
surfaced it, by re-serialising the numbers and changing which side of the fast
path they fell on. The feature is now enabled in `lib/cityparquet-rs/Cargo.toml`
and in `vendor/cjseq`, which parses and re-serialises the same numbers.

## Running it

```sh
python3 3dbag2cityparquet.py all \
  --manifest /path/to/3dbag_urls.txt \
  --work    /scratch/3dbag-work \
  --dest    ../cityparquet_data/3dbag \
  --jobs    64
```

Needs `duckdb` (the `verify` stage only) and both binaries built:
`lib/cityparquet-rs/target/release/cityparquet` and
`vendor/cjseq/target/release/cjseq`.

| Stage     | What it does                                                  |
| --------- | ------------------------------------------------------------- |
| `fetch`   | download every tile `.gz` named by the manifest               |
| `plan`    | read each tile's transform + CRS; derive the merged transform |
| `seq`     | `gunzip \| cjseq cat \| shift` → one `.jsonl` per tile        |
| `merge`   | synthetic header + every tile's feature lines → one file      |
| `convert` | `cityparquet convert <that one file> -o <dest>`               |
| `verify`  | row count, id uniqueness, extent, footer keys                 |
| `all`     | every stage in order                                          |

**Every stage is resumable** — rerunning skips what is already on disk, and
partial output is written to `.part` and renamed, so a crash never leaves a
half-file that looks finished.

Tiles are fed in quadtree order `(z, x, y)`, which gives the writer spatially
coherent row groups for free. `--ordering hilbert` would do better but buffers
every feature — the 1.1 TB this whole approach exists to avoid.

## Caveats, which are part of the artefact

**Rounding ties are not a corner case.** 4,002 of 26,823 axis shifts — 15% —
land exactly on `.5`. A plain integer add would put roughly one axis in seven
1 mm away from what `merge_sources` produces, systematically, across the whole
country. The tie branch above is load-bearing, and the equivalence test is what
proves it.

**One tile declares a far-away quantisation origin.** Tile `8-768-720` has
`translate.z = -1073721.375`; the other 8,940 sit in `[-2.09, 172.81]`. Taking
the componentwise minimum therefore drags the merged z origin down by ~1.07e6 m,
and the merged transform reads `translate [15130.9375, 308058.0, -1073721.375]`
— which looks alarming and is not. That tile's own vertices compensate exactly
(they decode to a sensible 0–19 m), z shifts stay around 1e9, far inside the
range where f64 represents integers exactly, and the output carries absolute
WKB coordinates either way. A direct `cityparquet convert` of all 8,941 tiles
would compute the same origin. It is recorded here only because anyone reading
the merged transform would otherwise reasonably suspect corruption.

**Two rows carry a nodata elevation.** One Building and its BuildingPart have
`bbox.zmin = −2147483.6` (INT32_MIN millimetres). That is a defect in the source
— the published STAC collection advertises the same value — and it is carried
through rather than filtered. Excluding those two, z spans a sane
`[−33.82, 345.61]` m NAP. Anything computing a national extent from this package
must expect the sentinel.

**631 BuildingParts have no LoD0 footprint.** `geometry_lod0_0` is null for
them, so §9 LoD0 synthesis could not derive a footprint from a higher LoD.
Every one is a BuildingPart; all 10,771,547 Buildings have one.

**The header's non-spatial metadata comes from one tile.** `merge_sources`
carries the FIRST input's metadata, and this mirrors it: `title: "3DBAG"`,
`pointOfContact`, `referenceSystem`. That is safe here because every 3DBAG tile
carries the same three, but it is a property of the source, not a guarantee.
`geographicalExtent` is dropped — one tile's extent must never be advertised as
the merged dataset's.

## Scratch

`--work` holds ~11 GB of `.gz`, ~53 GB of per-tile `.jsonl` and the ~53 GB
merged `3dbag.city.jsonl`; budget **~120 GB**. The merged file is a reusable
single-file CityJSONSeq of all of 3DBAG and is worth keeping; the rest is not.

## The manifest

The URL list is not generated here. It comes from the City3D STAC registry
(`manifests/3dbag_urls.txt`), which is also what the published
`netherlands-3d-bag` collection is built from — so the package and the
catalogue entry describe the same 8,941 tiles.
