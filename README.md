# cityparquet-rs

Rust reference implementation of **CityParquet** — a cloud-native columnar
encoding for 3D city models (CityJSON / CityJSONSeq), with an Arrow in-memory
representation. Part of the CityParquet + CityLake research stack (TU Delft
3D Geoinformation).

Workspace crates:

| Crate | Purpose |
|---|---|
| `cityparquet-schema` | Type system, CityGML CM taxonomy, Arrow schema (spec-as-code) — **M1 complete** |
| `cityparquet` | Parquet writer/reader — **M2 writer (core profile)** / **M3 reader & round-trip** / **M4 Compatibility profile complete** |
| `cityparquet-cli` | `cityparquet` CLI (WIP) |

## Usage

Convert a CityJSON or CityJSONSeq file to CityParquet:

```bash
cargo run -p cityparquet-cli -- convert tests/fixtures/delft.city.jsonl /tmp/delft-cityparquet --overwrite
```

The output is a directory containing `cityobjects.parquet` (Arrow geometry and semantics) and `metadata.json` (schema and provenance), readable by any Parquet reader (DuckDB, pyarrow). Passing `--overwrite` into a directory that already holds a package purges every file that package wrote (`metadata.json` plus every `*.parquet` it left behind) before writing the new one, so a stale sidecar from a prior run can never survive alongside a manifest that no longer lists it.

Pass `--profile compatibility` to also write `materials.parquet`, `textures.parquet`, and `geometry_templates.parquet` sidecars (each skipped when the dataset carries none of that kind of data) — the main table's `material`/`texture` columns and geometry-template references become dataset-global ids into these sidecars instead of being dropped:

```bash
cargo run -p cityparquet-cli -- convert tests/fixtures/lod3_railway.city.json /tmp/railway-cityparquet --profile compatibility --overwrite
```

The command prints: `object_count files_count skipped_same_lod_geometries attribute_coercion_nulls degenerate_rings_dropped degenerate_surfaces_dropped materials_written textures_written templates_written` (the last three are always `0` under the Core profile).

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

Every profile's round-trips maintain strict semantic equality for:
- Object IDs, hierarchy (parents/children as sets), and type
- Typed attributes with timestamps compared as UTC instants
- Per-LoD geometry within quantisation tolerance (coordinate space per axis)
- Geometry semantics (boundary trees and per-surface material/texture references)

The Compatibility profile additionally round-trips losslessly:
- Appearance definitions (materials, textures, and their UV coordinates) and every geometry's material/texture references, resolved against the sidecars rather than compared by raw index (feature-local index numbering is an implementation detail, not part of a CityJSON's semantics — exactly like vertex indices are compared as real coordinates, not by identity)
- Geometry-template definitions and every object's `GeometryInstance` reference into them

Documented exclusions, both profiles:
- Structurally degenerate rings (<3 vertices after closure normalisation — dropped at write with realigned semantics)
- `children_roles` and unknown per-object members (outside the CityJSONSeq data model on both sides)

Documented exclusions, Core profile only (present under Compatibility, above):
- Appearance definitions (dropped, counted at export)
- Per-geometry material/texture references beyond what fits in semantics (dropped, counted)
- Geometry-template definitions and their instance references (dropped, counted)

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

`just check` runs lint, the full test suite, the `cityparquet-schema`
arrow/parquet isolation check, and a `cargo fmt --check`. `just interop` runs
`scripts/interop.sh`, which converts both fixtures (delft Core, railway
Compatibility) and has DuckDB read the main table plus every Compatibility
sidecar natively as plain Parquet — skipped if `duckdb` is not on `PATH`.

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
