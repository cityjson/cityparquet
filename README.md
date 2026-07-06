# cityparquet-rs

Rust reference implementation of **CityParquet** — a cloud-native columnar
encoding for 3D city models (CityJSON / CityJSONSeq), with an Arrow in-memory
representation. Part of the CityParquet + CityLake research stack (TU Delft
3D Geoinformation).

Workspace crates:

| Crate | Purpose |
|---|---|
| `cityparquet-schema` | Type system, CityGML CM taxonomy, Arrow schema (spec-as-code) — **M1 complete** |
| `cityparquet` | Parquet writer/reader — **M2 writer complete (core profile)** / **M3 reader & round-trip complete** |
| `cityparquet-cli` | `cityparquet` CLI (WIP) |

## Usage

Convert a CityJSON or CityJSONSeq file to CityParquet:

```bash
cargo run -p cityparquet-cli -- convert tests/fixtures/delft.city.jsonl /tmp/delft-cityparquet --overwrite
```

The output is a directory containing `cityobjects.parquet` (Arrow geometry and semantics) and `metadata.json` (schema and provenance), readable by any Parquet reader (DuckDB, pyarrow).

Export a CityParquet package back to CityJSON/CityJSONSeq:

```bash
cargo run -p cityparquet-cli -- export /tmp/delft-cityparquet /tmp/delft-export.city.jsonl
```

Output format is auto-detected from the file extension (`.city.jsonl` → Seq, `.city.json` → Doc). The command prints: `feature_count object_count instance_geometries_dropped appearance_refs_dropped`.

Compare source and exported datasets for semantic equality:

```bash
cargo run -p cityparquet-cli -- compare tests/fixtures/delft.city.jsonl /tmp/delft-export.city.jsonl
```

Exit code 0 when equal (prints "equal"), exit code 2 when different (prints up to 20 differences). Supports `--exclude-appearance` and `--exclude-instances` flags to skip deliberate drops.

### Round-trip guarantees

Core-profile round-trips maintain strict semantic equality for:
- Object IDs, hierarchy (parents/children as sets), and type
- Typed attributes with timestamps compared as UTC instants
- Per-LoD geometry within quantisation tolerance (coordinate space per axis)
- Geometry semantics (boundary trees and per-surface material/texture references)

Documented exclusions deferred until M4 sidecars:
- Appearance definitions (dropped, counted at export)
- Per-geometry material/texture references beyond what fits in semantics (dropped, counted)
- Geometry-template definitions and their instance references (dropped, counted)
- Structurally degenerate rings (<3 vertices after closure normalisation — dropped at write with realigned semantics)
- `children_roles` and unknown per-object members (outside the CityJSONSeq data model on both sides)

### Known limitations (M2)

- Solid geometry is encoded as WKB `PolyhedralSurfaceZ` (type 1015), which
  GeoParquet readers that auto-decode geometry (GeoPandas, DuckDB's `spatial`
  extension) do not support yet. `MultiSurface`-derived columns (e.g. the LoD0
  `MultiPolygonZ` column) decode fine in those readers.
- The `geo` key-value metadata omits `crs` until PROJJSON support lands, so
  GeoParquet readers that consult it assume `OGC:CRS84` while the coordinates
  are actually in the source CRS. The source CRS is still recorded, as an OGC
  CRS URL, in the file's `crs` key-value metadata entry.
- `children_roles` and `other` columns are always null. Second geometries per
  `(object, LoD)` and LoD-less geometries in mixed datasets are skipped (both
  are counted in the convert report).

## Development

Integration tests read real CityJSON fixtures that are not checked into the
repo. Run `just fixtures` once to download them into `tests/fixtures/` before
running `cargo test --workspace`.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or
  http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or
  http://opensource.org/licenses/MIT)

at your option.

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in the work by you, as defined in the Apache-2.0
license, shall be dual licensed as above, without any additional terms or
conditions.
