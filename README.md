# CityParquet

**A columnar Parquet encoding for 3D city models.**

CityParquet stores a CityJSON or CityGML model as a **directory of Parquet
files, split by CityGML module** — `building.parquet`, `transportation.parquet`,
… — with one row per city object. Geometry is WKB in a per-LoD
`geometry_lod*` column, paired with a `geometry_properties_lod*` struct carrying
what WKB cannot hold (semantic surfaces, shell structure). Materials, textures
and geometry templates go into sidecar tables, written only when the source has
them.

The point is **pruning**. A national 3D city model is tens of gigabytes; a
question about one neighbourhood at one level of detail touches a few megabytes
of it. Row-group statistics prune by bounding box, column projection prunes by
LoD and by attribute, and both work over plain HTTP range requests — so the
model can stay on object storage and be queried where it lies, without a server
in front of it.

At LoD0 a CityParquet footprint **is** conformant GeoParquet: existing readers
open it and see a normal geospatial table. Solids step beyond GeoParquet's WKB
vocabulary and are declared in CityParquet's own `city` footer metadata, which
strict GeoParquet readers correctly ignore.

> **Status: experimental.** The specification is unversioned and still moving,
> the implementations are catching up to it, and on-disk output may change
> without notice. Do not build anything load-bearing on it yet.

## Quickstart

```sh
git clone --recurse-submodules https://github.com/cityjson/cityparquet.git
cd cityparquet/lib/cityparquet-rs

cargo run --release -p cityparquet-cli -- convert delft.city.jsonl --output delft/
cargo run --release -p cityparquet-cli -- export  delft/ roundtrip.city.jsonl
cargo run --release -p cityparquet-cli -- compare delft.city.jsonl roundtrip.city.jsonl
```

`convert` writes `delft/building.parquet`, `delft/metadata.json` (a STAC Item)
and any sidecars the source needs. `compare` is the round-trip proof: it checks
semantic equality, not byte equality, and it is what every claim of losslessness
in this repository rests on.

Or in SQL, with no Rust at all:

```sql
INSTALL cityjson FROM community; LOAD cityjson;
COPY (SELECT * FROM read_cityjsonseq('delft.city.jsonl')) TO 'delft.parquet' (FORMAT PARQUET);
```

## What is in this repository

```
documents/     the normative specification — and the reasoning behind it
lib/           the implementations
  cityparquet-rs/    Rust reference reader/writer; owns the encoding
  citylake/          DuckLake-based lakehouse runtime (work in progress)
  duckdb-cityjson/   DuckDB extension: CityJSON/Seq/FlatCityBuf in SQL  (submodule)
  duckdb-3d/         DuckDB extension: 3D solid processing              (submodule)
benchmark/     three benchmark families, their corpora, results and caveats
test/          the cross-module manual walkthrough, and a script that runs it
example/       small inputs to try things on
ai/            design notes; the record of decisions that shaped the above
```

- **The specification** is in [`documents/docs/03-specification/`](documents/docs/03-specification/),
  with the reasoning in `04-design-decisions/` and the genuinely unsettled parts
  in `05-open-questions/`. It is the authority on the format; the
  implementations are catching up to it, and where they disagree with it, they
  are wrong. Rendered at **https://cityjson.github.io/cityparquet/**.
- **The benchmarks** are in [`benchmark/`](benchmark/README.md). Read its README
  before quoting a number — several of the results are deliberately
  non-citable as rankings, and it says which and why.
- **`lib/cityparquet-rs`** is the reference implementation and the place a
  format question gets settled in code.

## Setup

```sh
just setup            # every submodule, recursively (~1.2 GB — see below)
just setup-shallow    # the same at --depth 1: enough for the spec and the Rust library
just --list           # everything you can run from here
```

**A full checkout is large.** The repository itself is small (~5 MB packed), but
the two DuckDB extensions each vendor DuckDB, `extension-ci-tools` and `vcpkg`
as their own submodules, which is where roughly 1.2 GB of `.git/modules` comes
from. If you only want to read or build the specification, or to work on the
Rust library, `just setup-shallow` — or simply cloning without
`--recurse-submodules` — is enough.

## Development

```sh
cd lib/cityparquet-rs && just check   # clippy, tests, schema isolation, fmt, prettier
just plot-test                        # the benchmark plotting suite      (needs uv)
just scripts-test                     # the benchmark shell suites        (needs jq)
just check                            # all three, from the repository root
just docs-build                       # build the specification site      (needs pnpm)
```

The benchmark harness spans two trees on purpose — its code is a crate in
`lib/cityparquet-rs`, its corpora and results are evidence under `benchmark/`
— so every recipe that reaches both lives in the **root** `justfile` and runs
from the repository root. `benchmark/README.md` explains the split.

Contributions are welcome; see [CONTRIBUTING.md](CONTRIBUTING.md) for where a
specification change goes and what has to pass before it lands.

## Related work

CityParquet is the storage layer of a small stack. Beside it sit
[CityJSON](https://www.cityjson.org/) (the model it encodes),
[GeoParquet](https://geoparquet.org/) (which it is a superset of, as far as
their shared WKB vocabulary reaches),
[FlatCityBuf](https://github.com/cityjson/flatcitybuf) (a cloud-optimised binary
CityJSON encoding, and the closest comparison in the benchmarks), and the
[City3D STAC extension](https://github.com/cityjson/city3d-stac-tool) (the
discovery layer that sits in front of a collection of packages).

## Licence

The **software** — everything under `lib/`, `benchmark/`, `test/` and the root
tooling — is dual-licensed under [MIT](LICENSE-MIT) OR
[Apache-2.0](LICENSE-APACHE), at your option.

The **specification and documentation** in [`documents/`](documents/LICENSE) are
licensed under **CC BY 4.0**. A format specification exists to be quoted,
reproduced and built on, which a content licence permits plainly and a software
licence does not.

The submodules under `lib/` carry their own licences.

## Citing

See [CITATION.cff](CITATION.cff). A paper describing the encoding and these
benchmarks is in preparation; this section will point at it once it is
published.
