# CityParquet write and configuration benchmark methodology

The **write** side of the benchmark suite: how long a CityParquet package takes
to write, how many bytes it occupies, and how those two move with the writer's
own knobs — codec, row-group size, row ordering. Its read-side counterpart, and
the cross-format comparison, is `benchmark/formats/READ_BENCHMARK.md`.

**The committed write-side CSVs are `results/` and `scaling_write_results/`**
(the writer's variant matrix over the six-dataset cityjson.org corpus and over
the 3DBAG scaling slices), alongside the configuration axes under
`scaling_codec_results/` and `scaling_rowgroup_results/`, which carry a
`MACHINE.md` describing the host they were measured on. The corpus runs —
`read_results/`, `scaling_read_results/` and `ordering_results/` — carry no
such record, so whether they ran on the same host cannot be established from
what is committed. Absolute times are therefore not comparable across a
directory that has a machine record and one that does not; what the figures
cite is the ratios within a single directory. Nothing in this document quotes
a number, so the methodology here cannot go stale against a re-run; the CSVs
themselves can, and one caveat already applies.

**Every committed write-side and configuration-axis CSV predates the typed
appearance columns.** They were measured while `material_lod*` /
`texture_lod*` were JSON text cells; those columns are now typed Arrow/Parquet
`MAP`s, which the writer leaves at parquet's own defaults for dictionary
encoding and statistics. For `results/` and `scaling_write_results/` that
means neither the committed bytes nor the committed write times describe the
current writer until both families are re-run. The codec and row-group runs
were measured at the branch point before that change; the 3DBAG slices carry
no appearance data, so those columns are empty in every package measured and
the effect on their bytes and times is expected to be negligible, but the two
axes have not been repeated on the current writer either.

## Three recipes

| recipe                                        | what it varies                                                                                                                                                           | output                                                              |
| ---------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------- |
| `just write-bench FOLDER [OUT]`                | the writer's variant matrix over every CityJSON/CityJSONSeq file under FOLDER — codecs, row-group sizes, ordering, plus the DuckDB `COPY … TO (FORMAT PARQUET)` baseline | `OUT/<name>.csv` (default `benchmark/formats/results/`)             |
| `just codec-bench FOLDER [OUT] [PREPARED]`    | codec axis over every input under `FOLDER` on the read harness: a timed write per variant (peak RSS), then a full read and the bbox windows                             | `OUT` (default `benchmark/formats/scaling_codec_results/`)          |
| `just rowgroup-bench FOLDER [OUT] [PREPARED]` | the same for the row-group axis                                                                                                                                          | `OUT` (default `benchmark/formats/scaling_rowgroup_results/`)       |

`just codec-bench`'s and `just rowgroup-bench`'s CSVs are in the READ run's
shape (`READ_BENCHMARK.md`), with a `write` row per variant and the variant id
in the `format` column; they feed the summary page's section 3b and the
`codec`/`rowgroup` print figures (`benchmark/plot/benchviz`).

`just bench` is a third thing again — the cross-format **read** benchmark,
writing to `benchmark/formats/read_results/`. `just codec-bench` and `just
rowgroup-bench` run on the same read harness and share its CSV shape, but hold
the format fixed at `cityparquet` and vary the codec or row-group size
instead of the format.

## Measurement discipline

- **Median of `repeat` samples** (default 5 for `write-bench`), reported at
  6-decimal precision, alongside the byte counts.
- **Every `write_s` sample times a CLEAN write.** A fresh, empty directory is
  created — and later removed — _outside_ the timed window for every repeat, so
  no sample pays for unlinking the previous one's files. This matches the
  baseline script's own `mktemp -d`-per-sample discipline, so the two sides are
  timed the same way. (An earlier revision timed repeats 2..n _with_ the purge
  of the previous repeat, which inflated everything but the first sample.)
- **Sub-10 ms deltas are noise** at these repeat counts and are not findings on
  their own — the same floor `benchmark/formats/READ_BENCHMARK.md` applies.
- **`roundtrip_equal`** is written per variant by `write-bench`: the package is
  exported back to CityJSONSeq and compared against the source with
  `cityparquet compare`. A `false` there invalidates that variant's bytes as a
  _lossless_ encoding. `codec-bench` and `rowgroup-bench` run on the read
  harness and carry no such column.

## The codec levels are NOT matched

`just codec-bench` sweeps zstd at levels **1, 3, 9 and 19**. Gzip and brotli
stay at the `parquet-rs` defaults carried by `crates/core/src/recipe.rs` —
**gzip at level 6, brotli at level 1** — and are reference points, not swept
axes. **"The smallest codec" remains a non-citable claim across codecs**: gzip,
brotli and zstd are different implementations at different effort levels, so a
byte or time difference between them says nothing about the codec family in
general. **"Zstd level N versus level M" is a measured, citable claim**: it is
the same codec swept deliberately, and `just codec-bench`'s CSVs are what back
it. The summary page states this inline above its codec panels and cites this
section.

## Baseline geometry coverage — the DuckDB `duckdb-copy` rows

`benchmark/scripts/bench_duckdb.sh` appends `duckdb-copy` / `duckdb-copy-zstd` rows to the
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
  `geom_lod1_2`/`geom_lod1_3`/`geom_lod2_2` pattern beside it is _not_ a bug:
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
just write-bench tests/fixtures        # variant matrix + DuckDB baseline -> benchmark/formats/results/
just codec-bench    tests/fixtures     # codec axis -> benchmark/formats/scaling_codec_results/
just rowgroup-bench tests/fixtures     # row-group axis -> benchmark/formats/scaling_rowgroup_results/
```

Any folder of CityJSON/CityJSONSeq works — `just fetch-data` fetches the
six-dataset cityjson.org corpus the read benchmark uses, and all three recipes
walk a folder recursively. Each removes `OUT/<name>.csv` before writing it, never
appends, so a committed run is one machine, one sitting, per dataset.

Per-dataset, without the recipes:

```sh
cargo run --release -p cityparquet-cli -- bench --input <file> --out <csv>
./benchmark/scripts/bench_duckdb.sh <file> <csv>
```

**Record the machine with the run.** `codec-bench` and `rowgroup-bench` write
their own `MACHINE.md` beside the CSVs; `write-bench`'s CSVs carry no machine
metadata, so a committed `write-bench` run without a recorded host is
internally comparable and externally unquotable. `benchmark/scripts/machine_record.sh`
is the canonical capture — the two axis recipes call it, and a `write-bench`
run should too:

```sh
uname -srm     # kernel, release and architecture; NOT `uname -a`, whose node
               # name is the host's address and these files are published
# Linux: lscpu | sed -n '1,15p'; free -b | head -2
# macOS: sysctl -n machdep.cpu.brand_string hw.memsize
duckdb --version; cargo --version; rustc --version
```
