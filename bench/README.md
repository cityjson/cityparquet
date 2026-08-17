# CityParquet write & compression benchmark methodology

The **write** side of the benchmark suite: how long a CityParquet package takes
to write, how many bytes it occupies, and how those two move with the writer's
own knobs — codec, row-group size, row ordering. Its read-side counterpart, and
the cross-format comparison, is `bench/READ_BENCHMARK.md`.

**No write-side CSVs are committed.** The earlier ones were deleted once the
inputs they measured (three pinned 3DBAG tiles fetched by a script this repo no
longer carries) stopped being reproducible, and the geometry-encoding default
changed underneath them. Run the recipes below to produce them; nothing in this
document quotes a number, so nothing in it can go stale in that particular way
again.

## Two benchmarks, two questions

| recipe | what it varies | output |
|---|---|---|
| `just write-bench FOLDER [OUT]` | the writer's variant matrix over every CityJSON/CityJSONSeq file under FOLDER — codecs, row-group sizes, ordering, plus the DuckDB `COPY … TO (FORMAT PARQUET)` baseline | `OUT/<name>.csv` (default `bench/results/`) |
| `just compression-bench FOLDER [OUT]` | codec and row-group variants only, with a full scan and a window query re-read per variant, and a round-trip check per variant | `OUT/<name>.csv` (default `bench/compression_results/`) |

`just compression-bench`'s CSVs are what the summary page's compression view and
its static figure read (`bench/plot/benchviz`); with none committed, that view
says so rather than showing an empty grid.

Neither is `just bench` — that is the cross-format **read** benchmark, writes to
`bench/read_results/`, and shares nothing with these two but the corpus.

## Measurement discipline

- **Median of `repeat` samples** (default 5 for `write-bench`), reported at
  6-decimal precision, alongside the byte counts.
- **Every `write_s` sample times a CLEAN write.** A fresh, empty directory is
  created — and later removed — *outside* the timed window for every repeat, so
  no sample pays for unlinking the previous one's files. This matches the
  baseline script's own `mktemp -d`-per-sample discipline, so the two sides are
  timed the same way. (An earlier revision timed repeats 2..n *with* the purge
  of the previous repeat, which inflated everything but the first sample.)
- **Sub-10 ms deltas are noise** at these repeat counts and are not findings on
  their own — the same floor `bench/READ_BENCHMARK.md` applies.
- **`roundtrip_equal`** is written per variant: the package is exported back to
  CityJSONSeq and compared against the source with `cityparquet compare`. A
  `false` there invalidates that variant's bytes as a *lossless* encoding, so
  the summary page greys out and badges any dataset whose variants all fail.

## The codec levels are NOT matched

The codec variants are written at the `parquet-rs` defaults carried by
`crates/cityparquet/src/recipe.rs`: **zstd at level 3, gzip at level 6, brotli
at level 1**. They are therefore a comparison of *implementation defaults*, not
of codecs at equal effort, and **"the smallest codec" is not a citable claim
from this benchmark**. Anyone wanting a codec ranking has to re-run with levels
chosen deliberately. The summary page states this inline above its codec panels
and cites this section.

## Baseline geometry coverage — the DuckDB `duckdb-copy` rows

`scripts/bench_duckdb.sh` appends `duckdb-copy` / `duckdb-copy-zstd` rows to the
same CSV at the same precision and sample count: it reads the source through the
community `cityjson` extension's `read_cityjson`/`read_cityjsonseq` and re-writes
it with `COPY … TO (FORMAT PARQUET)`.

**That extension does not populate every geometry column it declares**, so those
rows do not encode the same content ours do. Verified empirically (2026-07-07,
`duckdb` v1.5.3, `cityjson` community extension) against the two committed
fixtures — re-check it the same way after any extension upgrade:

```sql
LOAD cityjson;
SELECT count(geom_lod0), count(geom_lod1_2), count(geom_lod1_3), count(geom_lod2_2), count(*)
FROM read_cityjsonseq('tests/fixtures/delft.city.jsonl');
-- 0 | 1116 | 1116 | 1116 | 2231

SELECT count(geom_lod), count(geom_lod3), count(*)
FROM read_cityjson('tests/fixtures/lod3_railway.city.json');
-- 0 | 0 | 121
```

Two distinct patterns, both checked against the source CityJSON:

- **`geom_lod0` comes back fully NULL** even where the source genuinely carries
  LoD0 geometries — verified against the raw source JSON, not only through
  DuckDB. It is an extension limitation specific to LoD `"0"`, uniform across
  datasets, not a per-file data quirk. The **50 %**-populated
  `geom_lod1_2`/`geom_lod1_3`/`geom_lod2_2` pattern beside it is *not* a bug:
  a 3DBAG/delft `Building`'s geometry lives on its `BuildingPart` children, so
  half of those datasets' CityObjects (the parents) legitimately carry none.
- **`lod3_railway.city.json` comes back fully NULL on BOTH its geometry
  columns** — `geom_lod` (for objects whose source `lod` is absent, e.g.
  `SolitaryVegetationObject`) and `geom_lod3` (the 105 objects with a real
  `lod: "3"` `MultiSurface`/`Solid`, confirmed against the raw source). The
  extension writes **zero** geometry for that file.

**Consequence: a `duckdb-copy` row for a dataset the extension under-reads is
not comparable to any CityParquet variant's bytes or write time** — at the
limit it measures writing attribute columns and no geometry at all. Keep the
rows (deleting data from a measurement artefact is worse than disclosing it),
and draw no cross-encoder byte or time comparison from them without saying
which geometry each side actually wrote.

## Reproduce

```sh
just fixtures                          # the two CityJSON fixtures (network)
just write-bench tests/fixtures        # variant matrix + DuckDB baseline -> bench/results/
just compression-bench tests/fixtures  # codec / row-group matrix -> bench/compression_results/
```

Any folder of CityJSON/CityJSONSeq works — `just fetch-data` fetches the
30-dataset catalogue corpus the read benchmark uses, and both recipes walk a
folder recursively. Each removes `OUT/<name>.csv` before writing it, never
appends, so a committed run is one machine, one sitting, per dataset.

Per-dataset, without the recipes:

```sh
cargo run --release -p cityparquet-cli -- bench --input <file> --out <csv>
./scripts/bench_duckdb.sh <file> <csv>
```

**Record the machine with the run.** Nothing in the CSVs carries machine
metadata, so a committed run without a recorded host is internally comparable
and externally unquotable:

```sh
uname -a
# Linux: lscpu | sed -n '1,15p'; free -b | head -2
# macOS: sysctl -n machdep.cpu.brand_string hw.memsize
duckdb --version; cargo --version; rustc --version
```
