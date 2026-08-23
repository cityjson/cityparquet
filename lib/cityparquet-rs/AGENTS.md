# cityparquet-rs

**Rust reference implementation of CityParquet** — the reader/writer that _owns_ the
encoding. Part of the CityParquet + CityLake research stack (TU Delft 3D
Geoinformation). It stores a 3D city model as a **directory of Parquet files** (one row
per city object, WKB geometry per LoD, typed attribute columns, optional
material/texture/geometry-template sidecars), with an Arrow in-memory representation,
and round-trips back to CityJSON / CityJSONSeq with **semantic** losslessness.

## Where you are

`lib/cityparquet-rs/` inside the CityParquet monorepo. Two things follow:

- **The benchmark harness is split on purpose.** Its code is here
  (`crates/cityparquet-readbench`, `scripts/`); its corpora, results and plotting
  project are evidence and live in `../../benchmark/`. Every recipe that reaches both
  — `bench`, `convert-all`, `write-bench`, `compression-bench`, the fetchers, the
  renderers, `plot-test`, `scripts-test` — is in the **root** `justfile` and is run
  from the repository root. `$BENCH_ROOT` (default `../../benchmark/formats`) is the
  seam the scripts use.
- **This `justfile` keeps only what belongs to the crate**, and `just check` here is
  self-contained: no `uv`, no `jq`, no corpus.

## Relationship to the specification

The **normative CityParquet specification** lives in the monorepo at
`../../documents/docs/03-specification/` (reasoning in `04-design-decisions/`,
undecided items in `05-open-questions/`). This crate _implements_ that spec. Its own
`docs/design.md` is the local design doc; where it and the spec disagree, the **spec is
authoritative on the format**. Track any divergence in the spec's
implementation-status table (`../../documents/docs/06-resources/02-software.mdx`).

## Components

Four crates in the Cargo workspace, plus one Python driver that is deliberately outside
it (it orchestrates the binaries rather than linking against them):

| Component                   | Purpose                                                                                                                                                                                                                                                                                                                                                                    |
| --------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `cityparquet-schema`        | Type system, CityGML taxonomy, Arrow schema, sidecar schemas, manifest — **the spec as code**. Kept free of `arrow-array`/`parquet` (enforced by `just isolation`) so it stays an executable specification.                                                                                                                                                                |
| `cityparquet`               | Parquet writer/reader, WKB, appearance interning, sidecars, export, comparator, Hilbert ordering, recipe presets.                                                                                                                                                                                                                                                          |
| `cityparquet-cli`           | The `cityparquet` binary and the benchmark harness.                                                                                                                                                                                                                                                                                                                        |
| `cityparquet-readbench`     | Read-path benchmark harness.                                                                                                                                                                                                                                                                                                                                               |
| `tools/catalog2cityparquet` | Python driver converting the published City3D STAC catalogue (53 collections, ~74k items) into a CityParquet mirror, and ledgering **why** each item did or did not convert. Shells out to `cityparquet` and to the vendored `city3dstac`; interprets no format itself. Its own suite is `just catalog-test`, not `just check`. See `tools/catalog2cityparquet/README.md`. |

## Commands

```bash
just fixtures     # download the real CityJSON test fixtures (one-off, network)
just check        # clippy -D warnings + tests + schema isolation + fmt --check (Rust and Markdown)
just test         # tests only
just lint         # clippy -D warnings
just fmt          # rustfmt + Prettier over the Markdown
just interop      # convert both fixtures and have DuckDB read the Parquet natively
```

Catalogue driver (`tools/catalog2cityparquet`; all but `catalog-test` need the network,
and none of them is part of `just check`):

```bash
just catalog-tools                       # build the two binaries the driver shells out to
just catalog-convert [OUT] [ARGS...]     # whole catalogue -> CityParquet mirror (hours; resumable)
just catalog-convert-collection ID [OUT] # one collection, the way to prove a change on real data
just catalog-aggregate [OUT]             # rebuild the mirror's root catalog.json, no downloads
just catalog-test                        # the driver's own suite (261 tests, no network)
```

CLI (add `--release` for realistic timing):

```bash
cargo run -p cityparquet-cli -- convert INPUT --output OUTPUT_DIR --overwrite  # CityJSON/Seq/CityGML → package
cargo run -p cityparquet-cli -- export  PACKAGE_DIR  OUT.city.jsonl    # package → CityJSON/Seq
cargo run -p cityparquet-cli -- compare A.city.jsonl B.city.jsonl      # semantic equality (round-trip proof)
cargo run --release -p cityparquet-cli -- bench --input INPUT --out results.csv
```

`convert` takes the output directory as the **required `-o`/`--output` flag**, not
positionally. Other flags include `--recipe`, `--ordering source|hilbert`,
`--row-group-size`, `--zstd-level`, `--geometry-encoding wkb|arrow-native`,
`--partition`, `--no-lod0`, `--crs` (an operator-supplied CRS for a source that
declares none — without it such a source still converts, writing `city.crs: null`), and
`--tolerate-invalid-appearance` (drop a dangling material/texture index instead of
aborting; off by default — strict is the oracle). The
round-trip is proven by `convert` → `export` → `compare` against the source. See
`README.md` for the full flag tables and the per-command stdout report formats, and
`../../benchmark/formats/README.md` for benchmark methodology and comparability caveats.

## Development discipline

- **Strict red-green TDD** — write the failing test first, then the smallest change to pass, then refactor. No implementation code before a failing test.
- **Tests read real CityJSON fixtures** (`just fixtures` first; e.g. `tests/fixtures/delft.city.jsonl`, `tests/fixtures/lod3_railway.city.json`) — **never** inline hand-written CityJSON.
- **Keep `cityparquet-schema` free of `arrow-array`/`parquet`** (`just isolation`) so it remains an executable spec.
- **Breaking changes are welcome** — pick the right design, do not carry compatibility shims, deprecation paths, or legacy branches for the old one. Update every call site instead.
- **The pre-commit hook formats source and docs on every commit** — the monorepo's `.githooks/pre-commit` runs rustfmt and Prettier over the staged files; activate it once per clone with `just hooks` **from the repository root**, and never bypass it with `--no-verify`.
- **`just check` is the gate** before declaring work done or opening a PR.
- Run an external Codex CLI review at the end of each milestone.

## Writing docs

- **Document the present, never the past.** No "fixed", "was broken", "now uses", "previously", no strikethrough deltas, no changelog voice. A reader wants how it is, not how it got here. History belongs in git; write it into a doc only when explicitly asked.
- **British English** in prose, consistent with the paper.
- Each level keeps an `AGENTS.md` byte-identical to its `CLAUDE.md` — edit one, copy it to the other.

## Delegating to models

- **Fable is the advisor** — use it to review a plan, weigh a design decision, or check finished work.
- **Sonnet or Opus is the executor** — they write the code, run the tests, and make the edits.

## Docs

- `docs/design.md` — data model & format (package layout, columns, geometry/appearance encoding, round-trip semantics).
- `docs/architecture.md` — the code: crates, the two-pass conversion pipeline, reader/export/compare, the benchmark harness.
- `../../benchmark/README.md` — what the three benchmark families measure, which evidence is committed and which is re-measured, and the caveats that are load-bearing.
- `../../benchmark/formats/README.md` — the write/compression benchmark's methodology and comparability caveats (no CSVs committed).
- `../../benchmark/formats/READ_BENCHMARK.md` — the cross-format read benchmark: methodology, the six-dataset cityjson.org corpus, the two benchmark sets, and 18 fairness caveats. No CSVs are committed; `benchmark/formats/read_results/` and `benchmark/formats/ordering_results/` are populated by `just bench` / `just ordering-bench` from the repository root.
