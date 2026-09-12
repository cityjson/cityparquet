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

**The committed `results/` and `scaling_write_results/` CSVs predate the
typed appearance columns.** They were measured while `material_lod*` /
`texture_lod*` were JSON text cells; those columns are now typed Arrow/Parquet
`MAP`s, which the writer leaves at parquet's own defaults for dictionary
encoding and statistics, so neither the committed bytes nor the committed
write times describe the current writer until both families are re-run. The
codec and row-group runs are measured on the current writer: each directory's
`MACHINE.md` names the commit, and the 3DBAG slices carry no appearance data,
so those columns are empty in every package they measured.

## Running the suite

Use the root entry points:

```sh
just bench-prep --families formats,codec,rowgroup
just bench-run --families formats,codec,rowgroup
just bench-summary
```

The `formats` family compares write and read performance across file formats.
The `codec` and `rowgroup` families hold the format fixed and change one
configuration dimension over nested 3DBAG slices. Their write and read rows
share a dataset/configuration identity; write measurements are independent of
the subsequent read queries. Read scenarios for configuration experiments are
full read, the bbox windows and the middle-position ID lookup.

The primary format configuration is Hilbert-ordered CityParquet, displayed as
**CityParquet** in figures. Internal variant IDs retain the ordering and codec
information needed to reproduce each configuration. The codec and row-group
figures show the largest measured slice and a separate scaling line chart.
See [`../README.md`](../README.md) for the experimental matrix and figure list.

The older `results/` and `scaling_write_results/` writer matrices use a
separate schema and are not a substitute for the format family's write
measurements. Their provenance and geometry qualifications below still apply
when inspecting those files.

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
just bench-prep --families codec,rowgroup
just bench-run --families codec,rowgroup
just bench-summary
```

Add `--smoke` for a small pipeline check. Full experiments use all configured
scaling slices; actual counts are recorded because feature boundaries can
cross a nominal target. Keep machine metadata, source identity, software
revision, query parameters and repetition settings alongside the results.
Prepared data and result directories have separate responsibilities: preparing
an artefact does not constitute a timed write measurement.
