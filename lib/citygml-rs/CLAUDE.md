# citygml-rs

**CityGML readers in Rust**, shared by `cityparquet-rs` and (in time) FlatCityBuf.
A Cargo workspace of its own inside the CityParquet monorepo; it may move to a
repository of its own later, so nothing here may depend on `cityparquet-rs`.

## Crates

| Directory         | Package        | What                                                                                                                                                                                                                                                                          |
| ----------------- | -------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `crates/core`     | `citygml-core` | Version-neutral: `GmlObject` and what it holds, GML 3.1/3.2 geometry and xlink assembly (`gml`), the CityGML-to-CityJSON class vocabulary, version sniffing, quick-xml helpers, what an ADE's XML Schema says about its elements (`xsd`), and the conversions out (`convert`) |
| `crates/citygml2` | `citygml2`     | The CityGML 2.0 reader: walks the XML and fills a `GmlObject` per top-level city object                                                                                                                                                                                       |

A `citygml3` crate joins them for CityGML 3.0.

## Design rules

- **`GmlObject` names no target format.** It holds no cjseq type. Conversions
  out are `convert::FromGml` implementations; cjseq's is behind the `cjseq`
  feature (on by default). Another intermediate model is another `FromGml`.
- **`citygml-core` knows no CityGML version.** Namespace URIs, element names
  and content models belong to the per-version crates.
- **No XSD code generation, no per-class structs.** What a version's schema
  says is data: tables of class names and modules, read by one walker.
- **One object subtree in memory at a time.** The readers stream; a document's
  size must not bound what can be read.
- **The conversion is deterministic.** Vertices, materials and textures are
  pushed in a fixed depth-first order, so the same document always yields the
  same bytes.

## Commands

```sh
just check   # clippy (default and no-default features) + tests + fmt --check
just test
just fmt
```

`cityparquet-rs` path-depends on these crates, so its own `just check` and
its CityGML tests also exercise them.

## Conventions

- `cjseq` is pinned to the **same exact version** as `lib/cityparquet-rs/Cargo.toml`;
  bump both together, or the two workspaces build two copies.
- Tests read real CityGML files committed under each crate's `tests/data`, no
  larger than about 100 MB each; never inline hand-written XML.
- **British English** in prose. **Document the present, never the past.**
- `AGENTS.md` is byte-identical to this file — edit one, copy it to the other.
