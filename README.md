# cityparquet-rs

Rust reference implementation of **CityParquet** — a cloud-native columnar
encoding for 3D city models (CityJSON / CityJSONSeq), with an Arrow in-memory
representation. Part of the CityParquet + CityLake research stack (TU Delft
3D Geoinformation).

Workspace crates:

| Crate | Purpose |
|---|---|
| `cityparquet-schema` | Type system, CityGML CM taxonomy, Arrow schema (spec-as-code) — **M1 complete** |
| `cityparquet` | Parquet writer/reader (WIP) |
| `cityparquet-cli` | `cityparquet` CLI (WIP) |
