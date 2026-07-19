# cityparquet-rs

Rust reference implementation of **CityParquet** — a cloud-native, columnar
Parquet encoding for 3D city models (CityJSON / CityJSONSeq), with an Arrow
in-memory representation. Part of the CityParquet + CityLake research stack
(TU Delft 3D Geoinformation).

CityParquet stores a city model as a **directory of Parquet files** — one row
per city object, WKB geometry per LoD, typed attribute columns, and optional
sidecar tables for materials, textures, and geometry templates — so national-
to-global 3D city models can be filtered, pruned, and queried directly from
cloud object storage. It round-trips back to CityJSON/CityJSONSeq with
semantic losslessness.

- **[docs/design.md](docs/design.md)** — the data model & format: package
  layout, columns, geometry/appearance encoding, profiles, round-trip
  semantics.
- **[docs/architecture.md](docs/architecture.md)** — the code: crates, the
  two-pass conversion pipeline, reader/export/compare, the benchmark harness.
- **[bench/README.md](bench/README.md)** — benchmark methodology, results, and
  comparability caveats.

## Crates

| Crate | Purpose |
|---|---|
| `cityparquet-schema` | Type system, CityGML taxonomy, Arrow schema, profiles, manifest — the spec as code (no Parquet/buffer deps) |
| `cityparquet` | Parquet writer/reader, WKB, appearance interning, sidecars, export, comparator, Hilbert ordering, recipe presets |
| `cityparquet-cli` | The `cityparquet` binary and the benchmark harness |

Status: milestones **M1–M5 complete** — schema, native writer (Core), reader
& round-trip, Compatibility profile, and the benchmark suite. The **LoD0
footprint is the un-suffixed, GeoParquet-legal primary `geometry` column**, and
the writer can **synthesise** one from higher-LoD geometry when the source lacks
it (CLI default; `--no-lod0` to disable) — see `src/lod0.rs`. Async /
object-store I/O, native (non-WKB) geometry, and Python bindings are post-1.0
future work.

## Install & build

```bash
just fixtures     # download the CityJSON test fixtures (one-off, network)
just check        # clippy -D warnings, tests, schema isolation, fmt --check
```

Requires a recent stable Rust toolchain (pinned in `rust-toolchain.toml`) and,
for the `just` recipes, [`just`](https://github.com/casey/just). `just
interop` and the benchmark baseline additionally use `duckdb` if it's on
`PATH`.

## CLI usage

Run via `cargo run -p cityparquet-cli -- <command>` (add `--release` for
realistic timing). Four subcommands:

### convert — CityJSON/Seq → CityParquet package

```bash
cargo run -p cityparquet-cli -- convert INPUT OUTPUT_DIR --overwrite
```

Writes `OUTPUT_DIR/` containing `cityobjects.parquet` + `metadata.json`,
readable by any Parquet reader (DuckDB, pyarrow, …).

| Flag | Default | Meaning |
|---|---|---|
| `--profile` | `core` | `core` or `compatibility` (adds material/texture/template sidecars) |
| `--overwrite` | off | purge an existing package in the target dir first |
| `--recipe` | `cityparquet` | writer preset: `cityparquet`, `parquet-defaults`, `no-dictionary`, `no-bss`, `no-delta`, `snappy` |
| `--ordering` | `source` | `source` or `hilbert` (spatial row ordering for better bbox pruning) |
| `--layout` | `single` | `single` (one table) or `by-type` (one table per object type) |
| `--row-group-size` | `65536` | Parquet row-group size |
| `--zstd-level` | `3` | zstd level (ignored by `--recipe snappy`) |
| `--batch-size` | `4096` | encode batch size |

Compatibility adds the sidecars (each skipped when the dataset has none of
that data):

```bash
cargo run -p cityparquet-cli -- convert tests/fixtures/lod3_railway.city.json \
    /tmp/railway --profile compatibility --overwrite
```

`convert` prints a space-separated report: `object_count files_count
skipped_same_lod_geometries attribute_coercion_nulls degenerate_rings_dropped
degenerate_surfaces_dropped materials_written textures_written
templates_written` (the last three are always `0` under Core).

### export — package → CityJSON/Seq

```bash
cargo run -p cityparquet-cli -- export PACKAGE_DIR OUTPUT.city.jsonl
```

Format is auto-detected from the extension (`.city.jsonl` → Seq, `.city.json`
→ document). Prints `feature_count object_count instance_geometries_dropped
appearance_refs_dropped appearance_lod_misses`.

### compare — semantic equality of two datasets

```bash
cargo run -p cityparquet-cli -- compare A.city.jsonl B.city.jsonl
```

Exit `0` and prints `equal` when semantically equal; exit `2` and prints up to
20 differences otherwise. `--exclude-appearance` and `--exclude-instances`
skip the Core profile's deliberate drops. This is how a round-trip is proven:
`convert` → `export` → `compare` against the source.

### bench — variant-matrix benchmark

```bash
cargo run --release -p cityparquet-cli -- bench --input INPUT --out results.csv
```

Appends one CSV row per variant. `--variants` takes a comma-separated list in
the grammar `<preset>[+hilbert][+by-type][+rg<N>]` (omit for the default
10-variant set); `--repeat` (default 5) reports the median; `--window-frac`
(default 0.05) sizes the spatial window query; `--skip-roundtrip` skips the
export+compare check. See [bench/README.md](bench/README.md).

## `just` recipes

| Recipe | What it does |
|---|---|
| `just fixtures` | download the CityJSON test fixtures into `tests/fixtures/` |
| `just check` | clippy, tests, schema/Parquet isolation, `fmt --check` |
| `just test` / `just lint` / `just fmt` | the individual gates |
| `just interop` | convert both fixtures and have DuckDB read the Parquet natively |
| `just convert-all [FOLDER] [OUT] [PROFILE]` | convert every CityJSON/Seq file under `FOLDER` (default `tests/fixtures`) into a package under `OUT` (default `out/cityparquet`) |
| `just bench-fixtures [FOLDER]` | run the benchmark on every CityJSON/Seq file under `FOLDER` (default `tests/fixtures`), one CSV per input |
| `just bench-data` | fetch the 3 pinned 3DBAG benchmark tiles into `bench/data/` |
| `just bench-corpus` | fetch the CityJSON benchmark corpus (11 datasets) into `bench/data/benchmark/` |
| `just bench-all` | the full M5 benchmark run (fixtures + 3DBAG tiles, cityparquet + DuckDB baseline) |
| `just bench-baseline INPUT CSV` | append the DuckDB `COPY` baseline rows for one dataset |

Downloaded benchmark data (`bench/data/`) and generated packages (`out/`) are
gitignored; `bench/results/*.csv` and `bench/README.md` are committed as the
paper's measurement artefacts.

## Development

Tests read **real CityJSON fixtures** (run `just fixtures` first), never
inline hand-written CityJSON. Development follows strict red-green TDD, and
`cityparquet-schema` is kept free of `arrow-array`/`parquet` (enforced by
`just isolation`) so it can serve as an executable specification. See
[docs/architecture.md](docs/architecture.md) for the module map.

## License

Licensed under either of Apache License 2.0
([LICENSE-APACHE](LICENSE-APACHE)) or MIT ([LICENSE-MIT](LICENSE-MIT)) at your
option. Unless you state otherwise, any contribution you submit for inclusion
shall be dual licensed as above, without additional terms.
