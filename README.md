# cityparquet-rs

Rust reference implementation of **CityParquet** — a cloud-native columnar
encoding for 3D city models (CityJSON / CityJSONSeq), with an Arrow in-memory
representation. Part of the CityParquet + CityLake research stack (TU Delft
3D Geoinformation).

Workspace crates:

| Crate | Purpose |
|---|---|
| `cityparquet-schema` | Type system, CityGML CM taxonomy, Arrow schema (spec-as-code) — **M1 complete** |
| `cityparquet` | Parquet writer/reader — **M2 writer complete (core profile)** |
| `cityparquet-cli` | `cityparquet` CLI (WIP) |

## Usage

Convert a CityJSON or CityJSONSeq file to CityParquet:

```bash
cargo run -p cityparquet-cli -- convert tests/fixtures/delft.city.jsonl /tmp/delft-cityparquet --overwrite
```

The output is a directory containing `cityobjects.parquet` (Arrow geometry and semantics) and `metadata.json` (schema and provenance), readable by any Parquet reader (DuckDB, pyarrow).

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
