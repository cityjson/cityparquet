# Configuration-axes benchmark: codec and row-group size measured by the read harness

**Date:** 2026-09-04
**Status:** approved design, not yet implemented
**Branch:** `bench/configuration-axes` (from `develop` at 75d0ca6)

## 1. Why

The paper needs two configuration figures beside the cross-format read figure:
*which compression codec, and when*, and *which row-group size, and when*. Both
must read like `formats.png`: per dataset, per measure, a bar per variant
against a baseline at 1x, read time on the left and peak memory on the right.

Today those two axes are measured by `cityparquet bench` (`crates/cli/src/bench.rs`)
through `just compression-bench`. That harness records write time, bytes, one
raw Arrow full scan, one bbox window through the pruning reader, row-group
counts and a round-trip flag. It records **no peak memory**, its reads are not
the query primitives the read benchmark measures, and it runs every variant
in one process so a memory figure could not be added without a high-water-mark
bleed between variants. The figure it feeds (`compression.png`) is a write-time
vs size scatter and its own headline says it is not citable.

The read benchmark (`benchmark/readbench`, `just bench`) already has what the
figures need: one child process per measurement, a discarded warmup and `N`
warm repeats, median and MAD, `getrusage` peak RSS, and the query primitives
the paper cites. The Hilbert ordering axis is already measured this way, as a
pseudo-format (`Format::CityParquetHilbert`). This design extends the same
harness to codec and row-group variants.

## 2. Decisions taken with the author

| Question | Decision |
|---|---|
| Which two queries | `full-read` and `bbox-query` (the 5 % window is drawn; 1 % and 25 % are kept in the data) |
| Corpus | The 3DBAG scaling slices at 1k, 5k, 10k, 50k, 100k, 500k, 1M CityObjects |
| Row-group sizes | 512, 2048, 8192, 32768 and the 65536 default |
| Codec levels | Library defaults for none, snappy, lz4, gzip (6), brotli (1); zstd swept at 1, 3 (default), 9, 19 |
| Codec and row group | **Two separate benchmarks**, two runs, two result directories, two figures, each against its own default at 1x |
| Which harness | The read coordinator grows a `--variants` axis and a write child (approach C below) |

Approaches rejected: (A) keep `cityparquet bench` for writes with a child per
repeat and teach the coordinator only the read side, which leaves two harnesses,
two grammars and two child spawners to keep in step; (B) put child processes
and both reads inside `cityparquet bench`, which re-implements the coordinator's
methodology and still measures a different read path from the read figure.

## 3. Variant grammar and the two lists

### 3.1 Grammar

The parser for `<preset>[+hilbert][+rg<N>][+<codec>]` moves from
`crates/cli/src/bench.rs` into the core crate beside `WriterRecipe`
(`crates/core/src/recipe.rs`) as a `Variant` type:

```rust
pub struct Variant { preset: RecipePreset, ordering: RowOrder,
                     row_group_size: Option<usize>, compression: Option<Codec>,
                     zstd_level: Option<i32> }
impl Variant {
    pub fn parse(id: &str) -> Result<Variant>;   // the grammar below
    pub fn id(&self) -> String;                  // canonical spelling, round-trips parse
    pub fn recipe(&self) -> WriterRecipe;        // row_group_size, codec and zstd_level applied
    pub fn ordering(&self) -> RowOrder;
}
```

One extension: a `zstd` token may carry a level, `zstd1`, `zstd9`, `zstd19`.
Only `zstd` takes a level; `gzip<N>` or `brotli<N>` is a grammar error, so the
level-matching question stays closed. `zstd` without digits is the recipe
default (3). The level flows into `WriterRecipe.zstd_level`, which exists.
Each suffix at most once; `rg0` and a zstd level outside `ZstdLevel`'s range are
errors. `cityparquet bench` and the coordinator both import `Variant`; the
CLI keeps no parser of its own.

### 3.2 The codec list (all at 65536 rows per group), in figure order

```
cityparquet                 zstd 3 - the default and the 1x baseline
cityparquet+zstd1
cityparquet+zstd9
cityparquet+zstd19
cityparquet+lz4
cityparquet+snappy
cityparquet+gzip
cityparquet+brotli
cityparquet+uncompressed
```

### 3.3 The row-group list (all zstd 3)

```
cityparquet                 65536 - the default and the 1x baseline
cityparquet+rg32768
cityparquet+rg8192
cityparquet+rg2048
cityparquet+rg512
```

### 3.4 Naming on disk

A variant package is written to `<prepared_dir>/<base>.<variant-id>.parquet`,
e.g. `3dbag_n50000.cityparquet+rg512.parquet`. The default is written by the
run too, as `<base>.cityparquet.parquet`, so baseline and variants come from
one sitting. The prepare script's own `<base>.parquet` is untouched: it is
what the coordinator derives the query parameters from, exactly as for every
other run, so the windows are shared with the read run on the same slice.

The `hilbert` suffix keeps parsing; neither list uses it.

## 4. What the coordinator learns

### 4.1 `--variants`

`cityparquet-readbench run --variants <comma-list>` is exclusive with
`--formats`; passing both is an error, so a CSV is always either a format
comparison or a configuration run. `--input`, `--prepared-dir`, `--out`,
`--repeat`, `--scenarios` keep their meaning. New: `--write-repeat` (default 3).

Rejected loudly: a duplicate id, an unknown suffix, a list without the bare
`cityparquet` baseline, or `--variants` together with `--transport http`
(there is no server-side write).

### 4.2 Per variant, in order: write, then read

1. **Write child.** The coordinator spawns itself as
   `--child --write --variant <id> --input <base>.city.jsonl --out <dir>`.
   The child calls the library's `convert` with `Variant::recipe()` and
   `Variant::ordering()`, measures wall time around it, and reports its own
   peak RSS through the existing `max_rss_bytes` and its heap peak through
   `alloc::peak_heap_bytes`, printing the same line shape the read children
   print (`time_s peak_heap peak_rss result_count`, where `result_count` is
   the conversion's `object_count`). Each invocation converts into a fresh
   directory, so no repeat times the deletion of the previous one. Run
   `1 + write_repeat` times: the first is a discarded warmup, the rest are
   warm samples. The **last** repeat's directory is renamed to
   `<prepared_dir>/<base>.<id>.parquet` (an existing one is removed first);
   no extra conversion is spent on the kept artefact.
2. **Read children.** The ordinary read path runs with `Format::CityParquet`'s
   runner against that package for every requested scenario, unchanged in
   warmup, repeats, medians and RSS. The recipes pass
   `--scenarios full-read,bbox-query`; bbox emits the 1, 5 and 25 % windows
   as today.

The child spawn for reads passes `--format cityparquet`; the variant id is only
a label on the coordinator side.

### 4.3 CSV shape: unchanged

A write measurement is one more row in the existing thirteen columns:

| column | write row |
|---|---|
| `dataset` | the slice file name |
| `format` | the variant id, e.g. `cityparquet+zstd9` |
| `scenario` | `write` |
| `selectivity` | empty |
| `result_count` | `object_count` reported by `convert` |
| `time_s`, `time_mad_s` | median and MAD over the warm write repeats |
| `peak_heap_bytes`, `peak_rss_bytes` | maxima over the warm repeats |
| `repeat` | `write_repeat` |
| `notes`, `bytes_read`, `http_requests` | empty |

Read rows differ from today only in `format` carrying a variant id.
Internally the coordinator gains `enum Measure { Write, Read(Scenario) }` and
`Row.format` becomes a label type that is either a `Format` or a variant id;
`write` never enters `Scenario::ALL` and `--scenarios write` is rejected.

### 4.4 Sizes

After a package is kept, the coordinator appends one row to `sizes.csv`
beside `--out`, in the read run's columns:
`dataset, format (variant id), bytes, mb, ratio_vs_cityjsonseq, baseline_format (cityparquet), ratio_vs_baseline`.
`bytes` is the sum of every regular file in the package directory (the same
rule `bench.rs` uses for `total_bytes`); the CityJSONSeq size is the prepared
`<base>.city.jsonl`. Rows for the run's own dataset that already exist in the
file are replaced, so a re-run of one slice never duplicates.

### 4.5 Query parameters and the sidecar

Unchanged: windows, predicate and id probes derive from the prepare script's
`<base>.parquet`, and `<out>.params.json` is written as today.

## 5. Recipes, corpus and the run

### 5.1 Corpus

The seven slices exist on the measurement host, cut on 2026-08-26 from the
7.6 GB source, together with their prepared artefacts (`<base>.parquet`,
`<base>.city.jsonl`, and the rest):

```
/data2/hideba/cityparquet/benchmark/formats/data/scaling/        n1000 .. n100000
/data2/hideba/cityparquet/benchmark/formats/data/scaling-large/  n500000, n1000000
/data2/hideba/cityparquet/benchmark/formats/data/readbench/      prepared artefacts
```

The worktree symlinks `benchmark/formats/data` at that directory (`data/` is
gitignored). `just fetch-scaling-data` stays the reproducible path for another
host.

### 5.2 Two recipes, one shape

```
codec-bench    SLICES OUT=(BENCH / "scaling_codec_results")    PREPARED=(BENCH / "data/readbench")
rowgroup-bench SLICES OUT=(BENCH / "scaling_rowgroup_results") PREPARED=(BENCH / "data/readbench")
```

Both walk `SLICES` for `.city.jsonl` files (the shared extension convention)
and, per slice:

1. `just readbench-prepare <slice> PREPARED cityparquet` - materialises
   `<base>.city.jsonl` and `<base>.parquet` if missing; no CityGML, no
   FlatCityBuf.
2. `cargo run --release <readbench> -- run --input <slice> --prepared-dir PREPARED --variants <list> --scenarios full-read,bbox-query --out OUT/<base>.csv`.

`codec-bench` passes the nine ids of 3.2; `rowgroup-bench` the five of 3.3.
Those two lines are the only place the two benchmarks meet. Each recipe
removes `OUT/sizes.csv` once at its start and `OUT/<base>.csv` before each
slice; each slice's coordinator run then appends its own rows to `sizes.csv`
(4.4). Both recipes end by writing `OUT/MACHINE.md` (5.3).

`strip_extension.rs` extracts the per-dataset recipes out of the justfile and
runs them; `compression-bench` leaves that list and the two new recipes join
it. `bench_recipe_test.sh` gains a check that `codec-bench` passes exactly the
codec list and `rowgroup-bench` exactly the row-group list, both starting with
the bare `cityparquet` baseline.

### 5.3 Machine record

Each recipe writes `OUT/MACHINE.md` from the capture block
`READ_BENCHMARK.md` asks for: `uname -a`, the first 15 lines of `lscpu`,
`free -b | head -2`, `rustc --version`, `cargo --version`, and
`git rev-parse HEAD`. The results README states that these two directories
were measured on a different host from the corpus runs (128-core AMD EPYC,
503 GB, Linux x86-64), which have no machine record at all.

### 5.4 Cost and order

From the committed 1M-object write (138 s at the default), per axis at 1M:
codec 9 x 4 writes, 2 to 3 h (brotli and zstd 19 dominate); row group 5 x 4
writes, under 1 h; every smaller slice together, under 1 h. Reads are cheap
against that. The two runs go under `nohup` with one log each, one after the
other, never concurrently.

### 5.5 Removed

`compression-bench`, `compression-plot`, `benchmark/formats/compression_results/`,
`benchmark/formats/scaling_compression_results/`, and
`benchmark/plot/readbench_plot/compression.py`. `write-bench`, its
`results/` and `scaling_write_results/` stay: they answer the encoding-preset
and DuckDB-baseline questions, which are out of scope here.

### 5.6 Committed

Both results directories: the per-slice CSVs, their `.params.json` sidecars,
`sizes.csv`, `MACHINE.md`.

## 6. Data contract (`bench_data.json`)

`prep.py` gains `load_scaling_axis(dir, baseline="cityparquet")`, called
once per directory. It replaces `load_compression` and the scaling compression
block. `compression` and `compression_gaps` leave the file; `scaling` gains:

```jsonc
"scaling": {
  "read": [...], "sizes": [...], "ordering": [...],     // unchanged
  "codec":    { "records": [...], "sizes": [...], "gaps": [...] },
  "rowgroup": { "records": [...], "sizes": [...], "gaps": [...] }
}
```

A **record** is one (slice, variant, measure):

```jsonc
{ "dataset": "3dbag_n50000", "objects": 50001,
  "variant": "cityparquet+rg512", "kind": "variant",     // "default" for the baseline
  "measure": "write",              // write | full-read | bbox-1pct | bbox-5pct | bbox-25pct
  "time_s": 1.92, "rss_b": 412000000,
  "base_time_s": 1.80, "base_rss_b": 398000000,
  "time_ratio": 0.94, "rss_ratio": 0.97,              // baseline / variant: > 1 is faster / leaner
  "below_floor": false }                              // |base - variant| < citation_floor_s
```

`objects` is the write row's `result_count`. Ratios follow the ordering
records' convention. Absolute values stay because the trend strip plots them.

A **size** entry is one (slice, variant): `bytes`, `mb`, `ratio_vs_cityjsonseq`
from `sizes.csv`, and `size_ratio` = baseline bytes / variant bytes (> 1 is
smaller).

**Gaps** are named, never dropped: a slice CSV without its baseline rows, a
variant missing a measure the others have, a header-only CSV, a `sizes.csv`
row without a matching record. Each is `{dataset, issue}`.

**Meta.** `meta.sources` gains the two directories. `meta.codec_level_note`
is rewritten: zstd swept at 1, 3, 9, 19; other codecs at library defaults
(gzip 6, brotli 1), drawn as reference points, not ranked against each other.
`meta.machine` carries the parsed `MACHINE.md` per directory.

**HTML.** Section 3b of the summary page is repointed at the two keys with
the same table shape it has (bytes per variant per slice, plus write time).
No new interactive view.

**Tests.** `test_csv_contract.py` gains the two directories with one fixture
CSV each, including a `write` row and a `sizes.csv`, and pins ratio direction,
`objects`, and the gap list.

## 7. The two figures

One function, `axis_sheet(data, key, order, palette, headline, out_dir)`,
draws both `codec` and `rowgroup`. Layout:

```
<headline computed from the run>
<subtitle: N slices, 3 measures, K variants, each against the default at 1x>

3dbag_n1000                3dbag_n10000              3dbag_n100000             3dbag_n1000000
1,000 objects · 1.2 MB seq  ...                       ...                       ...
           time (x faster)   peak memory (x leaner)
write      0.9 s ▌▌▌▌▌       ▌▌▌▌▌
full read  12 ms ▌▌▌▌▌       ▌▌▌▌▌
spatial 5%  3 ms ▌▌▌▌▌       ▌▌▌▌▌
bytes      1.2 MB ▌▌▌▌▌      (right column empty; axis label says "bytes row: x smaller")

key strip: one swatch per variant, "same order in every group"

trend, all seven slices, log-log, absolute, one line per variant, default heavier:
[ bytes on disk ]  [ write time ]  [ write peak RSS ]  [ spatial 5% time ]

footer: codec-level note · machine line · gap list
```

**Ratio sheet.** Four slice panels picked by the formats sheet's rule (spread
across the run by object count: 1k, 10k, 100k, 1M). Rows: write, full read,
spatial 5 %, bytes on disk. The default is the 1x rule, not a bar (the read
figure's convention for CityJSONSeq). Every bar prints its ratio; a faded bar
is under the 10 ms floor; the grey figure beside each row label is the
default's own absolute. The size row uses the left column only. Reuses
`_ratio_bar`, `_format_panel_axis`, `_panel_heading`, `_bar_value_label`,
`_headline`, `_footer`.

**Trend strip.** All seven cardinalities, log-log, absolute values, one line
per variant with the default drawn heavier. Panels: bytes on disk, write
time, write peak RSS, spatial 5 % time. Full-read time is omitted here (it is
in the sheet and adds little as a trend).

**Palette.** Codec: the zstd sweep in one hue at four lightness steps; the
five other codecs in distinct muted colours. Row group: one sequential hue
from small to large. Both from the module's existing colour constants.

**Headline and footer.** Computed, never typed. Codec: the size and
write-time span of the zstd sweep, and the fastest-reading codec at the
largest slice. Row group: the smallest group size that clears the floor on
the spatial window, and its write cost. Footer: codec-level note, machine
line, gap list.

**Files.** `codec.png/svg`, `rowgroup.png/svg`; `compression.*` deleted;
`configuration.*` (ordering) unchanged. `_check_capacity` refuses more than
five slices in the ratio sheet, as for datasets. One fixture-driven render
test per figure in `test_benchviz.py`.

In the paper repository, `just bench-summary` renders these into
`paper/assets/bench/` unchanged in mechanism; `compression.png/svg` are
removed there.

## 8. Out of scope

- Ordering (`ordering-bench`, `configuration.png`) - unchanged.
- The encoding-preset matrix and the DuckDB `COPY` baseline (`write-bench`).
- HTTP transport for variant runs.
- Level sweeps for codecs other than zstd.
- A new interactive HTML view.
