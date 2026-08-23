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
  layout, columns, geometry/appearance encoding, sidecars, round-trip
  semantics.
- **[docs/architecture.md](docs/architecture.md)** — the code: crates, the
  two-pass conversion pipeline, reader/export/compare, the benchmark harness.
- **[benchmark/formats/README.md](../../benchmark/formats/README.md)** — the
  write/compression benchmark's methodology and comparability caveats.
- **[benchmark/formats/READ_BENCHMARK.md](../../benchmark/formats/READ_BENCHMARK.md)**
  — the cross-format read benchmark's methodology, fairness caveats, and the
  CSVs' provenance.

The benchmark does not live here at all — harness crate, scripts, corpora,
results and renderers are all in the monorepo's
[`benchmark/`](../../benchmark/README.md) tree, and so are the `just` recipes
that drive it. Run those from the repository root. What is here is the library
they measure.

## Crates

| Crate                | Purpose                                                                                                            |
| -------------------- | ------------------------------------------------------------------------------------------------------------------ |
| `cityparquet-schema` | Type system, CityGML taxonomy, Arrow schema, sidecar schemas, manifest — the spec as code (no Parquet/buffer deps) |
| `cityparquet`        | Parquet writer/reader, WKB, appearance interning, sidecars, export, comparator, Hilbert ordering, recipe presets   |
| `cityparquet-cli`    | The `cityparquet` binary and the benchmark harness                                                                 |

Status: milestones **M1–M5 complete** — schema, native writer, reader
& round-trip, content-gated appearance/template sidecars, and the benchmark
suite. Every LoD,
**including LoD0, is a suffixed geometry column** (`geometry_lod0_0`, `geometry_lod2_2`, …);
the writer can **synthesise** an LoD0 footprint from higher-LoD geometry when
the source lacks it (CLI default; `--no-lod0` to disable) — see `src/lod0.rs`.
Async / object-store I/O, native (non-WKB) geometry, and Python bindings are
post-1.0 future work.

## Install & build

```bash
just fixtures     # download the CityJSON test fixtures (one-off, network)
just check        # clippy -D warnings, tests, schema isolation, fmt --check
```

Requires Rust 1.93.1 (pinned in `rust-toolchain.toml`, so rustup selects and
installs it for you) and, for the `just` recipes,
[`just`](https://github.com/casey/just). `just interop` and the benchmark
baseline additionally use `duckdb` if it's on `PATH`.

## CLI usage

Run via `cargo run -p cityparquet-cli -- <command>` (add `--release` for
realistic timing). Four subcommands:

### convert — CityJSON/Seq → CityParquet package

```bash
cargo run -p cityparquet-cli -- convert INPUT --output OUTPUT_DIR --overwrite
```

Writes `OUTPUT_DIR/` containing one `<snake>.parquet` table per 1st-level
CityObject family (e.g. `building.parquet`, `bridge.parquet`) + `metadata.json`,
readable by any Parquet reader (DuckDB, pyarrow, …). `metadata.json` is a STAC
Item (the `city3d:*` extension) describing that one package; a dataset-level
`collection.json` aggregating multiple packages/tiles into one STAC
Collection is **not yet implemented** — it needs a multi-package workflow
this CLI doesn't have, so it's tracked as a follow-up rather than emitted as
a meaningless single-Item Collection.

| Flag                            | Default       | Meaning                                                                                                                                                                |
| ------------------------------- | ------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `--overwrite`                   | off           | purge an existing package in the target dir first                                                                                                                      |
| `--recipe`                      | `cityparquet` | writer preset: `cityparquet`, `parquet-defaults`, `no-dictionary`, `no-bss`, `no-delta`, `snappy`                                                                      |
| `--ordering`                    | `source`      | `source` or `hilbert` (spatial row ordering for better bbox pruning)                                                                                                   |
| `--row-group-size`              | `65536`       | Parquet row-group size                                                                                                                                                 |
| `--zstd-level`                  | `3`           | zstd level (ignored by `--recipe snappy`)                                                                                                                              |
| `--batch-size`                  | `4096`        | encode batch size                                                                                                                                                      |
| `--crs`                         | unset         | operator-supplied CRS (`EPSG:25832` or bare `25832`) for a source that declares none; ignored for a source that declares its own                                       |
| `--tolerate-invalid-appearance` | off           | drop a material/texture index that falls outside its local definitions array instead of aborting; counted in the report's trailing field, never silent                 |
| `--partition`                   | unset         | split the output into one package per partition: `count` (+`--number N`), `features` (+`--feature-num M`), or `box` (+`--cell-size METRES`). Omit to write one package |

`city.crs` is **tri-state**, exactly as in GeoParquet (spec "CRS rules"): a
PROJJSON object when the CRS is known, an explicit **`null`** when the file
holds CRS-bearing coordinates whose CRS is unknown or unresolvable, and absent
only when it holds no CRS-bearing coordinate at all. So a source that carries
CRS-bearing coordinates but declares no CRS this writer can resolve still
converts — with `city.crs: null`, a matching `geo.columns[].crs: null`, no
`proj:*` STAC fields, no `referenceSystem` on export, and a `warning:` line
naming the problem. The writer never guesses, and never omits the key over
CRS-bearing coordinates (per GeoParquet an absent `crs` asserts OGC:CRS84,
which would silently mis-georeference a projected national city model).
`--crs` is the operator's way to turn that unknown into a real CRS — an
explicit declaration is neither a guess nor an absent CRS, so it makes the CRS
resolvable before the writer runs, exactly as an EPSG code in the source
would. When it is actually applied, the footer records
`city.other.crs_source = "operator-supplied"`, so the output never implies the
source declared a CRS it did not carry. A geographic (degree-valued) code is
refused: nothing in this pipeline reprojects, and coordinates are quantised at
millimetre scale.

With `--partition`, `-o` becomes the _parent_ of one self-contained package per
partition (`count-00000/`, `features-00003/`, `box_x93_y44/`, …), all sharing
one canonical schema so `read_parquet('OUT/*/building.parquet')` sees a uniform
layout. Partitions are assigned per **feature**, and a `CityJSONFeature` is a
top-level city object plus its children, so a parent and its children always
land in the same package — never split across two. Where the input breaks that
assumption (a CityJSONSeq file whose lines are not self-contained, or a merge
of neighbouring tiles that separates a building from its parts), the features
that reference each other are pulled onto a shared partition so the references
stay resolvable, and a `warning:` line reports how many were moved. This is the
one thing that can push a `features` partition past `--feature-num`. References
naming an object absent from the input altogether cannot be repaired; they are
counted and warned about, not refused, since a partial-area extract carries
them legitimately.

Within a package, objects are still split across module tables by CityGML
module, so a `CityObjectGroup` in `generics.parquet` may have its members in
`vegetation.parquet`. That is the by-module layout, not a partition boundary:
the guarantee is that the references resolve inside the package.

Sidecars (`materials.parquet`/`textures.parquet`/`geometry_templates.parquet`)
are written automatically whenever the source has that kind of content —
there is no profile flag to opt into them:

```bash
cargo run -p cityparquet-cli -- convert tests/fixtures/lod3_railway.city.json \
    --output /tmp/railway --overwrite
```

`convert` prints a space-separated report: `object_count files_count
skipped_same_lod_geometries attribute_coercion_nulls degenerate_rings_dropped
degenerate_surfaces_dropped materials_written textures_written
templates_written invalid_appearance_refs_dropped` (`materials_written`
through `templates_written` are `0` when the source has no appearance/
templates for that sidecar to write; `invalid_appearance_refs_dropped` is `0`
unless `--tolerate-invalid-appearance` actually dropped a dangling
material/texture reference).

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
skip appearance/`GeometryInstance` comparison, for a package whose sidecars
were left out of the comparison on purpose. This is how a round-trip is
proven: `convert` → `export` → `compare` against the source.

### bench — variant-matrix benchmark

```bash
cargo run --release -p cityparquet-cli -- bench --input INPUT --out results.csv
```

Appends one CSV row per variant. `--variants` takes a comma-separated list in
the grammar `<preset>[+hilbert][+rg<N>]` (omit for the default
9-variant set); `--repeat` (default 5) reports the median; `--window-frac`
(default 0.05) sizes the spatial window query; `--skip-roundtrip` skips the
export+compare check. See
[benchmark/formats/README.md](../../benchmark/formats/README.md).

## `just` recipes

**From this directory** — the crate's own gate, self-contained (no `uv`, no
`jq`, no corpus):

| Recipe                                 | What it does                                                     |
| -------------------------------------- | ---------------------------------------------------------------- |
| `just fixtures`                        | download the CityJSON test fixtures into `tests/fixtures/`       |
| `just check`                           | clippy, tests, schema/Parquet isolation, `fmt --check`, prettier |
| `just test` / `just lint` / `just fmt` | the individual gates                                             |
| `just vendor-check`                    | fmt + clippy + test the vendored `city3d-stac-tool`              |
| `just interop`                         | convert both fixtures and have DuckDB read the Parquet natively  |

**From the repository root** — everything that reaches both this crate and the
`benchmark/` tree:

| Recipe                                 | What it does                                                                                                                                                                                                                        |
| -------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `just convert-all FOLDER [OUT]`        | convert every city-model input under `FOLDER` into a package under `OUT` (default `out/cityparquet`)                                                                                                                                |
| `just fetch-data [DEST] [ONLY]`        | fetch the read benchmark's corpus (six real CityJSON datasets, 423 MB) into `DEST` (default `benchmark/formats/data/benchmark/`); `ONLY` picks the entries serving one benchmark set — `default` (the default), `no-citygml`, `all` |
| `just fetch-tools`                     | fetch the pinned external converters the read benchmark's conversion chain needs (citygml-tools, cjseq)                                                                                                                             |
| `just bench FOLDER [OUT] [FORMATS]`    | cross-format READ benchmark over every input under `FOLDER`, one CSV per input under `OUT` (default `benchmark/formats/read_results`); `FORMATS` is a comma-separated format list, empty for the default format-comparison set      |
| `just ordering-bench FOLDER [OUT]`     | the same run restricted to the ordering axis (source-order vs Hilbert CityParquet), into `OUT` (default `benchmark/formats/ordering_results`)                                                                                       |
| `just write-bench FOLDER [OUT]`        | encoding-variant WRITE benchmark + the DuckDB `COPY` baseline, one CSV per input                                                                                                                                                    |
| `just compression-bench FOLDER [OUT]`  | codec + row-group WRITE-bench matrix, one CSV per input, plus charts                                                                                                                                                                |
| `just plot` / `just plot-pretty`       | render charts and the cross-dataset summary page from CSVs already measured                                                                                                                                                         |
| `just plot-test` / `just scripts-test` | the harness's two non-Rust test suites (`benchmark/plot`'s pytest, `benchmark/scripts/`'s bash suite) — outside `just check`, which is the Rust gate                                                                                |

Every recipe that walks a `FOLDER` discovers and names its inputs through the
one input-extension convention at the top of the **root** `justfile`
(`KNOWN_INPUT_EXTENSIONS`/`KNOWN_INPUT_FIND`), which CityGML inputs are part
of; `benchmark/readbench/tests/strip_extension.rs` holds it in
lockstep with the Rust and shell implementations of the same rule.

Downloaded benchmark data (`benchmark/formats/data/`) and generated packages
(`out/`) are gitignored. The committed measurement artefacts are the
configuration-axis CSVs (`benchmark/formats/scaling_*_results/`) and the two
methodology documents beside them
(`benchmark/formats/READ_BENCHMARK.md`, `benchmark/formats/README.md`); the
read-side CSVs are not currently committed — see
[benchmark/README.md](../../benchmark/README.md) for which evidence is in git
and which is re-measured.

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
