# cityparquet-rs

**Rust reference implementation of CityParquet** — the reader/writer that *owns* the
encoding. Part of the CityParquet + CityLake research stack (TU Delft 3D
Geoinformation). It stores a 3D city model as a **directory of Parquet files** (one row
per city object, WKB geometry per LoD, typed attribute columns, optional
material/texture/geometry-template sidecars), with an Arrow in-memory representation,
and round-trips back to CityJSON / CityJSONSeq with **semantic** losslessness.

## Relationship to the specification

The **normative CityParquet specification** lives in the parent workspace at
`../documents/docs/03-specification/` (reasoning in `04-design-decisions/`, undecided
items in `05-open-questions/`). This repo *implements* that spec and is still catching
up to it in places — the current gaps are tracked in the spec's implementation-status
table (`../documents/docs/06-resources/02-software.mdx`). This repo's own
`docs/design.md` is the local design doc; where it and the parent spec disagree, the
**parent spec is authoritative on the format**, and any divergence is a tracked
follow-up here. Known current-vs-spec deltas:

- ~~File split per 1st-level CityObject family~~ — **resolved**: the file split now follows the spec's per-CityGML-module layout (`building.parquet`, `transportation.parquet`, …; `cityparquet_schema::module_file`).
- Footer metadata is now the spec's single `city` object (plus a conditional pure-GeoParquet `geo` object), emitted per Parquet file.

## Components

Four crates in the Cargo workspace, plus one Python driver that is deliberately outside
it (it orchestrates the binaries rather than linking against them):

| Component | Purpose |
|---|---|
| `cityparquet-schema` | Type system, CityGML taxonomy, Arrow schema, sidecar schemas, manifest — **the spec as code**. Kept free of `arrow-array`/`parquet` (enforced by `just isolation`) so it stays an executable specification. |
| `cityparquet` | Parquet writer/reader, WKB, appearance interning, sidecars, export, comparator, Hilbert ordering, recipe presets. |
| `cityparquet-cli` | The `cityparquet` binary and the benchmark harness. |
| `cityparquet-readbench` | Read-path benchmark harness. |
| `tools/catalog2cityparquet` | Python driver converting the published City3D STAC catalogue (53 collections, ~74k items) into a CityParquet mirror, and ledgering **why** each item did or did not convert. Shells out to `cityparquet` and to the vendored `city3dstac`; interprets no format itself. Its own suite is `just catalog-test`, not `just check`. See `tools/catalog2cityparquet/README.md`. |

Status: milestones **M1–M5 complete** (schema, native writer, reader & round-trip,
content-gated appearance/template sidecars, benchmarks) plus a native CityGML 2.0 reader/writer stack. Async
/ object-store I/O, native (non-WKB) geometry, and Python bindings are post-1.0.

## Commands

```bash
just fixtures     # download the real CityJSON test fixtures (one-off, network)
just check        # clippy -D warnings + tests + schema isolation + fmt --check
just test         # tests only
just lint         # clippy -D warnings
just fmt          # rustfmt
just interop      # convert both fixtures and have DuckDB read the Parquet natively
```

Catalogue driver (`tools/catalog2cityparquet`; all but `catalog-test` need the network,
and none of them is part of `just check`):

```bash
just catalog-tools                       # build the two binaries the driver shells out to
just catalog-convert [OUT] [ARGS...]     # whole catalogue -> CityParquet mirror (hours; resumable)
just catalog-convert-collection ID [OUT] # one collection, the way to prove a change on real data
just catalog-aggregate [OUT]             # rebuild the mirror's root catalog.json, no downloads
just catalog-test                        # the driver's own suite (219 tests, no network)
```

CLI (add `--release` for realistic timing):

```bash
cargo run -p cityparquet-cli -- convert INPUT OUTPUT_DIR --overwrite   # CityJSON/Seq → package
cargo run -p cityparquet-cli -- export  PACKAGE_DIR  OUT.city.jsonl    # package → CityJSON/Seq
cargo run -p cityparquet-cli -- compare A.city.jsonl B.city.jsonl      # semantic equality (round-trip proof)
cargo run --release -p cityparquet-cli -- bench --input INPUT --out results.csv
```

`convert` flags include `--recipe`, `--ordering
source|hilbert`, `--row-group-size`, `--zstd-level`. The round-trip is proven by
`convert` → `export` → `compare` against the source. See `README.md` for the full flag
tables and the per-command stdout report formats, and `bench/README.md` for benchmark
methodology and comparability caveats.

## Development discipline

- **Strict red-green TDD** — write the failing test first, then the smallest change to pass, then refactor.
- **Tests read real CityJSON fixtures** (`just fixtures` first; e.g. `tests/fixtures/delft.city.jsonl`, `tests/fixtures/lod3_railway.city.json`) — **never** inline hand-written CityJSON.
- **Keep `cityparquet-schema` free of `arrow-array`/`parquet`** (`just isolation`) so it remains an executable spec.
- **British English** in prose, consistent with the paper.
- Run an external Codex CLI review at the end of each milestone.

## Docs

- `docs/design.md` — data model & format (package layout, columns, geometry/appearance encoding, round-trip semantics).
- `docs/architecture.md` — the code: crates, the two-pass conversion pipeline, reader/export/compare, the benchmark harness.
- `bench/README.md` — benchmark methodology, results, comparability caveats (the paper's measurement artefacts).
