# Arrow-native geometry encoding — cityparquet-rs Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Scope note:** this plan covers **`cityparquet-rs` only** — the first leg of the
3-repo `arrow-native-type` experiment described in
`docs/superpowers/specs/2026-07-25-arrow-native-geometry-design.md` (read that design
doc first; this plan does not repeat its reasoning). Per that design's own sequencing,
`cityparquet-rs` lands first as the schema source of truth; `duckdb-cityjson` and
`duckdb-3d` get their own plans once this one has landed, so their tasks can reference
this crate's *actual* signatures rather than guessed ones.

**Goal:** add a second, opt-in geometry encoding (`--geometry-encoding arrow-native`) to
`cityparquet-rs`'s `convert`/`export`/`compare` pipeline — nested Arrow `List`/`Struct`
columns with an indexed, per-row-compacted vertex pool, covering `MultiSurface`,
`CompositeSurface`, `Solid`, `MultiSolid`, `CompositeSolid` — alongside the existing WKB
`BLOB` encoding, which stays untouched and remains the default.

**Architecture:** Reuse `crate::wkb_read::{DecodedGeometry, DecodedKind}` — already the
in-memory shape "vertex-pool coords + nested index structure" that WKB decoding produces
— as the shared encoding-agnostic model for the NEW path too. A new `arrow_geom_write`
module turns `cjseq::Geometry` into a `DecodedGeometry` via **distinct-source-index
compaction** (not coordinate-value dedup — two different source indices are never merged
even if bitwise-identical), reusing `wkb_write`'s existing ring/shell normalisation. A new
`arrow_geom_read` module is the inverse. Because both meet at `DecodedGeometry`, the rest
of the pipeline (`decode.rs`'s `DecodedObject` construction, `export.rs`, `compare.rs`)
needs only a one-line encoding branch, not new logic.

**Tech Stack:** Rust 2024/2021 (this workspace), `arrow-array`/`arrow-buffer`/`arrow-schema`
58.3.0, `cjseq` 0.4.1 (vendored source read directly for this plan — see design doc), the
existing `just fixtures`-downloaded real fixtures.

## Global Constraints

- **Strict red-green TDD**: write the failing test first, smallest change to pass, refactor.
  Never leave the tree red across a commit (this crate's own `CLAUDE.md`).
- **Real fixtures only** (`cityparquet-rs/CLAUDE.md`) — `tests/fixtures/delft.city.jsonl`
  (1115 `MultiSurface` LoD0 footprints + 3348 `Solid` at LoD 1.2/1.3/2.2, real semantics)
  and `tests/fixtures/lod3_railway.city.json` (`MultiSurface`/`CompositeSurface`, **no**
  `Solid` — also has `GeometryInstance`, which stays WKB-`None`/arrow-native-`None`
  regardless of encoding). Run `just fixtures` once if missing.
- **`cityparquet-schema` stays free of `arrow-array`/`parquet`** (`just isolation`) — the
  unified Arrow `DataType` constructors go in `cityparquet-schema` (it already builds
  `DataType`/`Field`/`Fields` via `arrow-schema` only, see `model.rs`), but any code using
  `arrow_array::builder::*` goes in the `cityparquet` crate.
- British English in prose/comments.
- Run `codex exec -m gpt-5.6-sol -s read-only` review at the end of this plan (this repo's
  own convention, and the user explicitly asked for `codex`/`gpt-5.6-sol` as reviewer for
  this whole experiment) before considering the branch done.
- Commit after every task; this repo is already on branch `arrow-native-type` with
  `origin/arrow-native-type` tracking — push after each milestone (every 2-3 tasks).
- **Phase-1 type scope only**: `MultiSurface`, `CompositeSurface`, `Solid`, `MultiSolid`,
  `CompositeSolid`. `MultiPoint`/`MultiLineString`/`GeometryInstance` under the new
  encoding are explicitly **unsupported in phase 1** — the writer errors clearly rather
  than silently mis-encoding (Task 4). `GeometryInstance` continues to produce no geometry
  cell either way (unchanged from WKB).
- **Naming**: sibling vertex-pool column is `geometry_vertices_lod<M>_<m>` (design doc
  open item 1, no better name surfaced).

---

## File Structure

| File | Responsibility |
|---|---|
| `crates/cityparquet-schema/src/types.rs` | Modify: add `GeometryEncoding` enum (`Wkb`/`ArrowNative`). |
| `crates/cityparquet-schema/src/model.rs` | Modify: unified nested `DataType` constructors for the arrow-native geometry + vertex-pool columns; `geometry_field`/`to_arrow_schema_tagged` grow an encoding parameter. |
| `crates/cityparquet-schema/assets/city.schema.json` | Modify: `encoding` field gains a second allowed value; `geometry_types` gains a CityJSON-type-name alternative, conditional on `encoding`. |
| `crates/cityparquet/src/wkb_write.rs` | Modify: make `boundaries`, `Drops`, `normalise_ring`, `normalise_surface`, `normalise_shells` `pub(crate)` (currently private) so the new writer can reuse them — **no other change**, existing WKB behaviour untouched. |
| `crates/cityparquet/src/arrow_geom_write.rs` | Create: `cjseq::Geometry` + `VertexPool` → `Option<DecodedGeometry>` via distinct-index compaction (Task 4); `DecodedGeometry` → Arrow `(ArrayRef, ArrayRef)` (geometry column, vertex-pool column) builders (Task 5). |
| `crates/cityparquet/src/arrow_geom_read.rs` | Create: the inverse — Arrow geometry + vertex-pool columns, one row, plus the row's `geometry_properties.type_name` → `DecodedGeometry` (Task 7). |
| `crates/cityparquet/src/encode.rs` | Modify: `GeometrySlot`/`GeometrySlotData`/`accumulate_geometry`/`RowWriter` branch on encoding (Task 6). |
| `crates/cityparquet/src/decode.rs` | Modify: `decode_batch`'s geometry loop branches on encoding (Task 8). |
| `crates/cityparquet/src/scan.rs` | Modify: `city_geometry_type`/`is_geoparquet_legal_type` become encoding-aware (Task 3). |
| `crates/cityparquet/src/package.rs` | Modify: `ConvertOptions` gains `geometry_encoding: GeometryEncoding` (Task 2). |
| `crates/cityparquet-cli/src/main.rs` | Modify: `convert` subcommand gains `--geometry-encoding` (Task 2). |
| `documents/docs/03-specification/03-geometry-semantics.mdx`, `05-metadata.mdx`, `04-design-decisions/02-geometry-encoding.mdx` | Modify: draft the experimental encoding into the spec, marked draft/under-evaluation (Task 11, per user review decision). |

---

### Task 1: `GeometryEncoding` enum + unified Arrow type constructors

**Files:**
- Modify: `crates/cityparquet-schema/src/types.rs`
- Modify: `crates/cityparquet-schema/src/model.rs`
- Test: `crates/cityparquet-schema/src/model.rs` (inline `#[cfg(test)]`, matching existing style — see `geometry_field_is_geoarrow_wkb` etc. near the file's end)

**Interfaces:**
- Produces: `pub enum GeometryEncoding { Wkb, ArrowNative }` (in `types.rs`, `Default` = `Wkb`); `pub fn arrow_native_geometry_data_type() -> DataType` and `pub fn arrow_native_vertices_data_type() -> DataType` (in `model.rs`); `CityParquetSchema::geometry_field(&self, name, lod, geoarrow, encoding: GeometryEncoding) -> Field` gains the new parameter; `CityParquetSchema::to_arrow_schema_tagged(&self, geoarrow: bool, encoding: GeometryEncoding) -> Result<Schema>` gains it too, and additionally emits the `geometry_vertices_lod<M>_<m>` sibling field whenever `encoding == ArrowNative`.

- [ ] **Step 1: Write the failing test for the unified geometry `DataType` shape**

Add to `crates/cityparquet-schema/src/model.rs`'s test module:

```rust
#[test]
fn arrow_native_geometry_data_type_is_solid_shell_face_ring_index() {
    // solid -> shell -> face -> ring -> vertex-pool index (Int32), matching
    // the design doc's unified shape (padding dimensions for surface types,
    // not a semantic distinction — see design doc "Arrow type definitions").
    let dt = arrow_native_geometry_data_type();
    let solid = match &dt {
        DataType::List(f) => f.data_type().clone(),
        other => panic!("expected outer List (solid), got {other:?}"),
    };
    let shell = match &solid {
        DataType::List(f) => f.data_type().clone(),
        other => panic!("expected List (shell), got {other:?}"),
    };
    let face = match &shell {
        DataType::List(f) => f.data_type().clone(),
        other => panic!("expected List (face), got {other:?}"),
    };
    // face -> List<List<Int32>> (ring -> index)
    let ring_list = match &face {
        DataType::List(f) => f.data_type().clone(),
        other => panic!("expected List (ring), got {other:?}"),
    };
    match &ring_list {
        DataType::List(f) => assert_eq!(f.data_type(), &DataType::Int32, "index type"),
        other => panic!("expected innermost List<Int32>, got {other:?}"),
    }
}

#[test]
fn arrow_native_vertices_data_type_is_list_of_xyz_struct() {
    let dt = arrow_native_vertices_data_type();
    let item = match &dt {
        DataType::List(f) => f.data_type().clone(),
        other => panic!("expected List, got {other:?}"),
    };
    let fields = match &item {
        DataType::Struct(fields) => fields.clone(),
        other => panic!("expected Struct, got {other:?}"),
    };
    let names: Vec<&str> = fields.iter().map(|f| f.name().as_str()).collect();
    assert_eq!(names, vec!["x", "y", "z"]);
    for f in fields.iter() {
        assert_eq!(f.data_type(), &DataType::Float64);
        assert!(!f.is_nullable(), "coordinate fields are non-null (design doc nullability invariants)");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p cityparquet-schema arrow_native_geometry_data_type_is_solid_shell_face_ring_index arrow_native_vertices_data_type_is_list_of_xyz_struct`
Expected: FAIL with "cannot find function `arrow_native_geometry_data_type`" (doesn't exist yet).

- [ ] **Step 3: Add `GeometryEncoding` to `types.rs`**

Add near the top of `crates/cityparquet-schema/src/types.rs` (alongside `Lod`):

```rust
/// Which physical Arrow encoding a `geometry_lod*` column uses. `Wkb` is the
/// only encoding CityParquet supports normatively today; `ArrowNative` is the
/// experimental alternative from the `arrow-native-type` branch (see
/// `docs/superpowers/specs/2026-07-25-arrow-native-geometry-design.md`) —
/// nested indexed `List`/`Struct` columns instead of a WKB `BLOB`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GeometryEncoding {
    #[default]
    Wkb,
    ArrowNative,
}
```

- [ ] **Step 4: Add the two unified `DataType` constructors to `model.rs`**

Add near `geometry_properties_data_type` in `crates/cityparquet-schema/src/model.rs`:

```rust
/// The unified arrow-native `geometry_lod*` shape (design doc "Arrow type
/// definitions"): `List<solid><List<shell><List<face><List<List<Int32>>>>>>`
/// — solid -> shell -> face -> ring -> vertex-pool index. `MultiSurface`/
/// `CompositeSurface` pad the two outer dimensions to length 1; this is a
/// PHYSICAL shape only — a reader dispatches on `geometry_properties.type`,
/// never on nesting depth (design doc "Critical invariant").
pub fn arrow_native_geometry_data_type() -> DataType {
    let index = Arc::new(Field::new("item", DataType::Int32, false));
    let ring = DataType::List(index);
    let rings = Arc::new(Field::new("item", ring, false)); // ring, non-null once populated
    let face = DataType::List(rings);
    let faces = Arc::new(Field::new("item", face, false));
    let shell = DataType::List(faces);
    let shells = Arc::new(Field::new("item", shell, false));
    let solid = DataType::List(shells);
    DataType::List(Arc::new(Field::new("item", solid, false)))
}

/// The arrow-native `geometry_vertices_lod*` sibling: this row's
/// distinct-source-index-compacted vertex pool (design doc "Approaches
/// considered" — NOT coordinate-value dedup; two different source indices
/// with identical coordinates are two separate entries). `Struct<x,y,z>`,
/// not `FixedSizeList<Float64,3>` (design doc "Arrow type definitions" —
/// Parquet shreds struct fields into independent leaf columns).
pub fn arrow_native_vertices_data_type() -> DataType {
    let coord = DataType::Struct(Fields::from(
        ["x", "y", "z"].map(|n| Field::new(n, DataType::Float64, false)).to_vec(),
    ));
    DataType::List(Arc::new(Field::new("item", coord, true)))
}
```

(The vertex-pool item field stays nullable at the `Field` level only because
`arrow-array`'s `ListBuilder` always allows null list items structurally; the
*value* invariant — no null vertex-pool entries within a non-null cell — is a
content contract enforced by the writer/reader, documented in the design doc's
nullability section, not something the `DataType` alone can express. This
mirrors the existing `shells` field's own `Field::new(..., false)` vs. the
outer cell's separate nullability — see `geometry_properties_data_type`
immediately above this addition for the precedent.)

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p cityparquet-schema arrow_native_geometry_data_type_is_solid_shell_face_ring_index arrow_native_vertices_data_type_is_list_of_xyz_struct`
Expected: PASS

- [ ] **Step 6: Write the failing test for `geometry_field`/`to_arrow_schema_tagged` gaining the encoding parameter**

```rust
#[test]
fn arrow_native_encoding_adds_vertices_sibling_and_arrow_native_geometry_type() {
    let schema = CityParquetSchema {
        lods: vec![Lod::parse("2.2").unwrap()],
        attributes: vec![],
        crs: None,
    };
    let arrow_schema = schema
        .to_arrow_schema_tagged(false, GeometryEncoding::ArrowNative)
        .unwrap();
    let geom = arrow_schema.field_with_name("geometry_lod2_2").unwrap();
    assert_eq!(geom.data_type(), &arrow_native_geometry_data_type());
    let vertices = arrow_schema
        .field_with_name("geometry_vertices_lod2_2")
        .expect("arrow-native encoding must add a geometry_vertices_lod* sibling column");
    assert_eq!(vertices.data_type(), &arrow_native_vertices_data_type());

    // WKB encoding (default) must NOT gain a vertices sibling.
    let wkb_schema = schema
        .to_arrow_schema_tagged(false, GeometryEncoding::Wkb)
        .unwrap();
    assert!(wkb_schema.field_with_name("geometry_vertices_lod2_2").is_err());
}
```

- [ ] **Step 7: Run test to verify it fails**

Run: `cargo test -p cityparquet-schema arrow_native_encoding_adds_vertices_sibling`
Expected: FAIL — `to_arrow_schema_tagged` doesn't take a `GeometryEncoding` argument yet (compile error).

- [ ] **Step 8: Thread `GeometryEncoding` through `geometry_field` and `to_arrow_schema_tagged`**

In `crates/cityparquet-schema/src/model.rs`, change `geometry_field`'s signature and body:

```rust
fn geometry_field(&self, name: &str, lod: Option<&Lod>, geoarrow: bool, encoding: GeometryEncoding) -> Field {
    let mut field = match encoding {
        GeometryEncoding::Wkb => {
            let mut field = Field::new(name, DataType::Binary, true);
            if geoarrow {
                let crs = match &self.crs {
                    Some(projjson) => Crs::from_projjson(projjson.clone()),
                    None => Crs::default(),
                };
                let wkb = WkbType::new(Arc::new(GeoMetadata::new(crs, None)));
                field = field.with_extension_type(wkb);
            }
            field
        }
        GeometryEncoding::ArrowNative => Field::new(name, arrow_native_geometry_data_type(), true),
    };
    field = reserved(field);
    if let Some(lod) = lod {
        field = with_meta(field, &[(LOD_KEY, &lod.to_string())]);
    }
    field
}

fn vertices_field(&self, geometry_column_name: &str, lod: Option<&Lod>) -> Field {
    let name = format!("{geometry_column_name}_vertices");
    // Renamed below to the correct `geometry_vertices_lod*` shape — see call site.
    let mut field = Field::new(name, arrow_native_vertices_data_type(), true);
    field = reserved(field);
    if let Some(lod) = lod {
        field = with_meta(field, &[(LOD_KEY, &lod.to_string())]);
    }
    field
}
```

Then update `to_arrow_schema_tagged`'s signature to `pub fn to_arrow_schema_tagged(&self, geoarrow: bool, encoding: GeometryEncoding) -> Result<Schema>`, pass `encoding` through every `self.geometry_field(...)` call site (there are two — the per-LoD loop and the empty-`lods` fallback; grep `geometry_field(` in this file to find both), and — **only for the per-LoD loop, only when `encoding == GeometryEncoding::ArrowNative`** — insert a `geometry_vertices_lod<M>_<m>` field right after each `geometry_lod<M>_<m>` field using the *correct* naming (not the placeholder in `vertices_field` above — see next paragraph):

```rust
if encoding == GeometryEncoding::ArrowNative {
    fields.push(reserved(with_meta(
        Field::new(
            geometry_column_name("geometry_vertices", lod),
            arrow_native_vertices_data_type(),
            true,
        ),
        &[(LOD_KEY, &lod.to_string())],
    )));
}
```

placed immediately after the existing `fields.push(self.geometry_field("geometry", Some(lod), geoarrow, encoding));` line inside the per-LoD loop (find it by grepping `geometry_field("geometry"`). Delete the placeholder `vertices_field` helper written above — it was scaffolding to think through the shape; the inline block above is the real implementation, using the existing `geometry_column_name` helper from `types.rs` exactly like the `geometry`/`geometry_properties` columns already do, so `geometry_vertices_lod2_2` is produced consistently with the suffix grammar. Add `geometry_vertices_lod<M>_<m>` to `RESERVED_COLUMN_NAMES`'s per-LoD equivalent in `reserved_and_geometry_column_names` too (same function, same pattern as the existing `geometry`/`geometry_properties`/`material`/`texture` entries), so an attribute column can never collide with it.

Every other existing caller of `to_arrow_schema_tagged`/`geometry_field` (grep both across the whole crate — `package.rs`, `encode.rs`, `reader.rs` per the earlier design-doc research) passes `GeometryEncoding::Wkb` explicitly at every call site **except** the one new call site Task 2 adds — do not default this silently via `Default::default()` at call sites that should always be WKB (e.g. the reader's schema-inference path, which reads whatever the file declares, not this crate's own write intent) — grep first, decide per call site whether it should read the file's actual encoding or hardcode `Wkb`, and leave a one-line comment at each explaining why.

- [ ] **Step 9: Run test to verify it passes**

Run: `cargo test -p cityparquet-schema arrow_native_encoding_adds_vertices_sibling`
Expected: PASS. Also run `cargo test -p cityparquet-schema` (full crate) to confirm no existing test broke from the signature change — fix any call site the compiler flags.

- [ ] **Step 10: Commit**

```bash
cd cityparquet-rs
git add crates/cityparquet-schema/src/types.rs crates/cityparquet-schema/src/model.rs
git commit -m "feat(schema): unified arrow-native geometry DataType + GeometryEncoding"
```

---

### Task 2: `ConvertOptions.geometry_encoding` + `--geometry-encoding` CLI flag

**Files:**
- Modify: `crates/cityparquet/src/package.rs`
- Modify: `crates/cityparquet-cli/src/main.rs`
- Test: `crates/cityparquet/tests/convert_real_data.rs` (mirror the existing `geoarrow_opt_in_restores_tag_and_geo_key` test named near line 1367 of that file, per earlier research)

**Interfaces:**
- Consumes: `cityparquet_schema::types::GeometryEncoding` (Task 1).
- Produces: `ConvertOptions.geometry_encoding: GeometryEncoding` (default `Wkb`, mirroring `geoarrow: bool`'s default-`false` precedent exactly); CLI `--geometry-encoding <wkb|arrow-native>` on `convert`, default `wkb`.

- [ ] **Step 1: Write the failing test**

Add to `crates/cityparquet/tests/convert_real_data.rs` (same file/style as
`geoarrow_opt_in_restores_tag_and_geo_key`):

```rust
#[test]
fn geometry_encoding_arrow_native_writes_nested_geometry_column() {
    let dir = tempdir().unwrap();
    let mut opts = ConvertOptions::new(fixture_path("delft.city.jsonl"), dir.path().to_path_buf());
    opts.geometry_encoding = cityparquet_schema::types::GeometryEncoding::ArrowNative;
    let report = convert(&opts).unwrap();
    assert!(report.object_count > 0);

    let building_parquet = dir.path().join("building.parquet"); // adjust to this repo's real per-family output name if different — confirm via `report.files`
    let file = std::fs::File::open(&building_parquet).unwrap();
    let reader = parquet::arrow::arrow_reader::ArrowReaderBuilder::try_new(file).unwrap();
    let schema = reader.schema();
    let geom_field = schema.field_with_name("geometry_lod1_2").expect("Solid LoD column"); // adjust suffix to a real LoD delft.city.jsonl carries per Task 1's fixture note ("Solid at LoD 1.2/1.3/2.2")
    assert_eq!(
        geom_field.data_type(),
        &cityparquet_schema::model::arrow_native_geometry_data_type()
    );
    assert!(
        schema.field_with_name("geometry_vertices_lod1_2").is_ok(),
        "arrow-native encoding must write the vertices sibling column"
    );
}
```

(This test's exact output filename/LoD suffix needs confirming against this
repo's real per-family output naming — grep an existing passing test in the
same file for the real `building.parquet`/`geometry_lod1_2`-or-whichever
convention rather than guessing twice.)

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p cityparquet geometry_encoding_arrow_native_writes_nested_geometry_column`
Expected: FAIL — `ConvertOptions` has no `geometry_encoding` field (compile error).

- [ ] **Step 3: Add the field to `ConvertOptions`**

In `crates/cityparquet/src/package.rs`, add to the `ConvertOptions` struct (next to `geoarrow`):

```rust
/// Which physical Arrow encoding `geometry_lod*` columns use. `Wkb` (the
/// default) is normative; `ArrowNative` is the experimental
/// `arrow-native-type` branch encoding — see
/// `docs/superpowers/specs/2026-07-25-arrow-native-geometry-design.md`.
pub geometry_encoding: cityparquet_schema::types::GeometryEncoding,
```

and in `ConvertOptions::new`'s body: `geometry_encoding: Default::default(),`
(which is `GeometryEncoding::Wkb` per Task 1's `#[default]`).

Then find every place `package.rs`/`encode.rs` calls `to_arrow_schema_tagged(opts.geoarrow, ...)` or
similar (per the earlier design-doc research, `encode.rs:1745,1788` and
`package.rs:1226`) and pass `opts.geometry_encoding` (or the equivalent field
on whatever struct carries it at that call site — trace it through) as the
new second argument.

- [ ] **Step 4: Add the CLI flag**

In `crates/cityparquet-cli/src/main.rs`, in the `Convert` subcommand's arg list, next to the existing `geoarrow: bool` field:

```rust
/// Physical geometry column encoding: "wkb" (default, normative) or
/// "arrow-native" (experimental nested Arrow List/Struct columns — see
/// docs/superpowers/specs/2026-07-25-arrow-native-geometry-design.md).
#[arg(long, default_value = "wkb")]
geometry_encoding: String,
```

Then where the CLI builds `ConvertOptions` from parsed args (grep `geoarrow,` in `main.rs` — the struct-literal call site around the existing lines 279/338 noted in earlier research), parse the string into the enum and set the field:

```rust
let geometry_encoding = match geometry_encoding.as_str() {
    "wkb" => cityparquet_schema::types::GeometryEncoding::Wkb,
    "arrow-native" => cityparquet_schema::types::GeometryEncoding::ArrowNative,
    other => {
        eprintln!("error: --geometry-encoding must be \"wkb\" or \"arrow-native\", got {other:?}");
        std::process::exit(2);
    }
};
```

and add `geometry_encoding,` to the `ConvertOptions { ... }` struct literal
next to the existing `geoarrow,` field.

- [ ] **Step 4b: Populate the real `city.columns[].encoding` footer field**

Confirmed by reading source directly: `CityColumnEntry::new` in
`crates/cityparquet-schema/src/metadata.rs:105-119` — the constructor for
`city.columns[]` entries (the spec-required, decoding-critical field the
design doc's "File-level provenance" section targets) — **hardcodes**
`encoding: "WKB".to_string()` regardless of caller. Its one real (non-test)
call site is `crates/cityparquet/src/scan.rs:571`,
`CityColumnEntry::new(name.clone(), geometry_types.clone())`, inside the
loop this plan's Task 3 also touches. This is separate from Task 1's Arrow
`Schema`/`DataType` work — that controls the physical column, this controls
the metadata *declaring* it, and both must agree.

Change `CityColumnEntry::new`'s signature to
`pub fn new(name: impl Into<String>, geometry_types: Vec<String>, encoding: GeometryEncoding) -> Self`,
setting `encoding: match encoding { GeometryEncoding::Wkb => "WKB", GeometryEncoding::ArrowNative => "CityParquetArrowNative-v1" }.to_string()`
internally. Update the one real call site (`scan.rs:571`) to pass the
encoding the surrounding scan is being performed for (the same value Task 3
threads into `is_geoparquet_legal_type`, from the same scope — thread both
from one parameter, don't duplicate the plumbing). Update the test-only
call site at `metadata.rs`'s own test module (`sample_city()`'s construction,
distinct from the `sample_geo()`/`GeoColumnEntry` one at `metadata.rs:290-299`
which is correctly WKB-only and untouched) to pass `GeometryEncoding::Wkb`
explicitly, preserving that test's existing behaviour.

Add a focused test first, per this plan's TDD discipline:

```rust
#[test]
fn city_column_entry_records_the_real_encoding_not_always_wkb() {
    let entry = CityColumnEntry::new(
        "geometry_lod2_2".to_string(),
        vec!["Solid".to_string()],
        GeometryEncoding::ArrowNative,
    );
    assert_eq!(entry.encoding, "CityParquetArrowNative-v1");

    let wkb_entry = CityColumnEntry::new(
        "geometry_lod2_2".to_string(),
        vec!["MultiPolygon Z".to_string()],
        GeometryEncoding::Wkb,
    );
    assert_eq!(wkb_entry.encoding, "WKB");
}
```

Run: `cargo test -p cityparquet-schema city_column_entry_records_the_real_encoding_not_always_wkb`
— confirm it fails first (old signature doesn't take an `encoding` arg),
then implement, then confirm it passes, then run the full
`cargo test -p cityparquet-schema -p cityparquet` to catch every call site
the signature change touches.

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p cityparquet geometry_encoding_arrow_native_writes_nested_geometry_column`
Expected: still FAILS at this point — Task 1 only defined the `DataType`
shape, nothing yet actually populates the columns with real data (Tasks 4-6
do that). Expected failure mode now: either a panic from `encode.rs` not
knowing how to build an `arrow-native`-shaped `RecordBatch` yet, or (if
`encode.rs` silently still writes WKB bytes into a column now typed as
nested `List` — a type mismatch) an Arrow/Parquet type error. **This is
expected and correct** — do not try to make this test pass yet. Note the
failure mode you saw, move to Task 3, and revisit this exact test at the end
of Task 6.

- [ ] **Step 6: Commit**

```bash
git add crates/cityparquet/src/package.rs crates/cityparquet-cli/src/main.rs crates/cityparquet/tests/convert_real_data.rs
git commit -m "feat(cli): --geometry-encoding flag (schema-only so far, encode.rs wiring in Task 6)"
```

---

### Task 3: Encoding-aware `geo` object emission

**Files:**
- Modify: `crates/cityparquet/src/scan.rs`
- Test: `crates/cityparquet/src/scan.rs` (inline, near existing tests for `is_geoparquet_legal_type` if any exist — grep first; else add a new `#[cfg(test)] mod tests` block following this file's existing style)

**Interfaces:**
- Consumes: `GeometryEncoding` (Task 1).
- Produces: `is_geoparquet_legal_type(type_name: &str, encoding: GeometryEncoding) -> bool` (signature change — was `(type_name: &str)`).

**Why:** confirmed by the design doc's round-2 review — `city_geometry_type`/
`is_geoparquet_legal_type` currently decide GeoParquet-legality from the CM
geometry type alone (`scan.rs:113-131`). An arrow-native `MultiSurface`
column would today be incorrectly declared GeoParquet-legal (it maps to
`"MultiPolygon Z"`, which `is_geoparquet_legal_type` accepts) even though its
physical encoding isn't WKB at all.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn arrow_native_multisurface_is_never_geoparquet_legal() {
    // A WKB MultiSurface (-> "MultiPolygon Z") IS legal.
    assert!(is_geoparquet_legal_type("MultiPolygon Z", GeometryEncoding::Wkb));
    // The SAME CM type, under the arrow-native encoding, MUST NOT be
    // declared GeoParquet-legal — its physical column isn't WKB at all.
    assert!(!is_geoparquet_legal_type("MultiPolygon Z", GeometryEncoding::ArrowNative));
    // Solid-family stays illegal either way (unchanged behaviour).
    assert!(!is_geoparquet_legal_type("PolyhedralSurface Z", GeometryEncoding::Wkb));
    assert!(!is_geoparquet_legal_type("PolyhedralSurface Z", GeometryEncoding::ArrowNative));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p cityparquet arrow_native_multisurface_is_never_geoparquet_legal`
Expected: FAIL — `is_geoparquet_legal_type` doesn't take an `encoding` argument yet (compile error).

- [ ] **Step 3: Make it encoding-aware**

```rust
fn is_geoparquet_legal_type(type_name: &str, encoding: GeometryEncoding) -> bool {
    encoding == GeometryEncoding::Wkb
        && !matches!(type_name, "PolyhedralSurface Z" | "GeometryCollection Z")
}
```

Update every call site (`scan.rs:283,573` per earlier research, both inside
`fn scan`) to pass the encoding this scan is being performed for. **This
requires `scan`/`ScanResult` to know which encoding the eventual write will
use** — trace `pub fn scan(source: &Source) -> Result<ScanResult>`'s callers
(likely `convert` in a top-level module, passing `ConvertOptions`) and either
thread a new `encoding: GeometryEncoding` parameter through `scan()` itself,
or compute `geoparquet_columns`/`module_geo` unconditionally for `Wkb`
assumptions and have the **caller** (wherever `ConvertOptions.geometry_encoding`
is known) re-filter `geoparquet_columns` down to empty when
`geometry_encoding == ArrowNative` before it reaches `city.columns`/`geo`
construction in `package.rs`. Prefer threading the real parameter through
`scan()` — it's the more honest fix and avoids a second place that has to
remember to filter — but check `scan()`'s existing call sites first (grep
`scan(`) to judge which is the smaller, more consistent change in context;
if `scan()` is called from more than one place with different encodings
imaginable, threading the parameter is correct, not optional.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p cityparquet arrow_native_multisurface_is_never_geoparquet_legal`
Expected: PASS. Also run `cargo test -p cityparquet scan` (or the full suite) to catch any call site the signature change missed.

- [ ] **Step 5: Commit**

```bash
git add crates/cityparquet/src/scan.rs
git commit -m "fix(geo): geo object omits arrow-native-encoded columns regardless of CM type"
```

---

### Task 4: `arrow_geom_write.rs` — geometry → compacted `DecodedGeometry`

**Files:**
- Modify: `crates/cityparquet/src/wkb_write.rs` (visibility only)
- Create: `crates/cityparquet/src/arrow_geom_write.rs`
- Modify: `crates/cityparquet/src/lib.rs` (add `mod arrow_geom_write;`)
- Test: `crates/cityparquet/src/arrow_geom_write.rs` (inline `#[cfg(test)]`, mirroring `wkb_write.rs`'s own inline test style)

**Interfaces:**
- Consumes: `cjseq::{Geometry, GeometryType}`, `crate::wkb_write::VertexPool` (Task 4 makes `boundaries`/`Drops`/`normalise_ring`/`normalise_surface`/`normalise_shells` `pub(crate)`), `crate::wkb_read::{DecodedGeometry, DecodedKind}` (both already exist, unmodified).
- Produces: `pub(crate) fn geometry_to_compacted(geom: &cjseq::Geometry, pool: &VertexPool) -> Result<Option<DecodedGeometry>>` — Task 6 calls this from `encode.rs` wherever it currently calls `wkb_write::geometry_to_wkb`.

- [ ] **Step 1: Make the reused `wkb_write.rs` helpers `pub(crate)`**

In `crates/cityparquet/src/wkb_write.rs`, change these five items' visibility from private to `pub(crate)` — **no other change to this file**:
- `struct Drops` → `pub(crate) struct Drops` (and its two fields `rings`/`surfaces` → `pub(crate) rings`/`pub(crate) surfaces`, since `arrow_geom_write.rs` needs to read them)
- `fn boundaries<T: ...>` → `pub(crate) fn boundaries<T: ...>`
- `fn normalise_ring` → `pub(crate) fn normalise_ring` (used transitively by `normalise_surface`, not called directly by the new module, but harmless/consistent to expose)
- `fn normalise_surface` → `pub(crate) fn normalise_surface`
- `fn normalise_shells` → `pub(crate) fn normalise_shells`

Run `cargo build -p cityparquet` after this step alone — it must still
compile cleanly with zero behaviour change (visibility-only edit); if
anything breaks, you widened the wrong thing.

- [ ] **Step 2: Write the failing test for a `MultiSurface`**

Create `crates/cityparquet/src/arrow_geom_write.rs`:

```rust
//! `cjseq::Geometry` -> compacted `DecodedGeometry` for the arrow-native
//! encoding (design doc "Approaches considered", Option B). Reuses
//! `wkb_write`'s ring/shell normalisation so degenerate-geometry handling
//! matches the WKB path exactly; differs only in the target shape (indexed
//! `DecodedGeometry` instead of WKB bytes) and in how the vertex pool is
//! built: **distinct-source-index compaction**, never coordinate-value
//! dedup (design doc round-2 correction — two different source indices
//! with identical coordinates stay two separate pool entries).

use std::collections::HashMap;

use cityparquet_schema::{CityParquetError, Result};
use cjseq::{Geometry, GeometryType, Transform};

use crate::wkb_read::{DecodedGeometry, DecodedKind};
use crate::wkb_write::{Drops, VertexPool, boundaries, normalise_shells, normalise_surface};

#[cfg(test)]
mod tests {
    use super::*;

    fn transform_identity() -> Transform {
        Transform { scale: vec![1.0, 1.0, 1.0], translate: vec![0.0, 0.0, 0.0] }
    }

    fn multisurface_geom(boundaries: serde_json::Value) -> Geometry {
        Geometry {
            thetype: GeometryType::MultiSurface,
            lod: Some("2".to_string()),
            boundaries,
            semantics: None,
            material: None,
            texture: None,
            template: None,
            transformation_matrix: None,
        }
    }

    #[test]
    fn multisurface_two_triangles_sharing_an_edge_compacts_the_shared_pair() {
        // Two triangles sharing edge (1,2): 4 distinct vertices total, not 6.
        let vertices: Vec<Vec<i64>> = vec![
            vec![0, 0, 0], vec![1, 0, 0], vec![1, 1, 0], vec![0, 1, 0],
        ];
        let pool = VertexPool::new(&vertices, &transform_identity());
        let geom = multisurface_geom(serde_json::json!([
            [[0, 1, 2]],
            [[0, 2, 3]]
        ]));
        let decoded = geometry_to_compacted(&geom, &pool).unwrap().unwrap();
        assert_eq!(decoded.coords.len(), 4, "shared indices 0 and 2 must be compacted, not duplicated");
        match &decoded.kind {
            DecodedKind::MultiPolygon(surfaces) => {
                assert_eq!(surfaces.len(), 2);
                assert_eq!(surfaces[0], vec![vec![0, 1, 2]]);
                assert_eq!(surfaces[1], vec![vec![0, 2, 3]]);
            }
            other => panic!("expected MultiPolygon, got {other:?}"),
        }
    }

    #[test]
    fn distinct_indices_with_equal_coordinates_are_never_merged() {
        // Two source indices, SAME coordinate value — must stay two pool entries.
        let vertices: Vec<Vec<i64>> = vec![vec![0, 0, 0], vec![0, 0, 0], vec![1, 0, 0]];
        let pool = VertexPool::new(&vertices, &transform_identity());
        let geom = multisurface_geom(serde_json::json!([[[0, 1, 2]]]));
        let decoded = geometry_to_compacted(&geom, &pool).unwrap().unwrap();
        assert_eq!(
            decoded.coords.len(), 3,
            "indices 0 and 1 have identical coordinates but are DISTINCT source vertices \
             (design doc: index-identity compaction, not coordinate-value dedup)"
        );
    }
}
```

- [ ] **Step 2b: Run test to verify it fails**

Run: `cargo test -p cityparquet arrow_geom_write::`
Expected: FAIL — `geometry_to_compacted` doesn't exist yet.

- [ ] **Step 3: Implement `geometry_to_compacted`**

Add above the `#[cfg(test)]` module in `arrow_geom_write.rs`:

```rust
/// Per-geometry, distinct-source-index vertex-pool compactor. Maps each
/// FIRST-SEEN raw source index to a dense local index and remembers its
/// dereferenced coordinate; a repeat occurrence of the SAME raw index reuses
/// its local index. Two different raw indices are never merged even if
/// bitwise-identical coordinates (design doc round-2 correction).
struct Compactor<'a, 'p> {
    pool: &'a VertexPool<'p>,
    seen: HashMap<usize, usize>,
    coords: Vec<[f64; 3]>,
}

impl<'a, 'p> Compactor<'a, 'p> {
    fn new(pool: &'a VertexPool<'p>) -> Self {
        Self { pool, seen: HashMap::new(), coords: Vec::new() }
    }

    fn local_index(&mut self, raw: usize) -> Result<usize> {
        if let Some(&local) = self.seen.get(&raw) {
            return Ok(local);
        }
        let local = self.coords.len();
        self.coords.push(self.pool.coord(raw)?);
        self.seen.insert(raw, local);
        Ok(local)
    }

    fn ring(&mut self, ring: &[usize]) -> Result<Vec<usize>> {
        ring.iter().map(|&raw| self.local_index(raw)).collect()
    }

    fn surface(&mut self, rings: &[&[usize]]) -> Result<Vec<Vec<usize>>> {
        rings.iter().map(|r| self.ring(r)).collect()
    }
}

/// `cjseq::Geometry` -> `Option<DecodedGeometry>`, phase-1 types only
/// (`MultiSurface`/`CompositeSurface`/`Solid`/`MultiSolid`/`CompositeSolid`
/// — design doc "Type coverage (v1)"). Mirrors `wkb_write::geometry_to_wkb`'s
/// dispatch and degenerate-ring/-surface handling exactly (same `Drops`
/// tracking, same `normalise_surface`/`normalise_shells` calls) — differs
/// only in the output shape. Returns `Ok(None)` for `GeometryInstance`
/// (no geometry cell, same as WKB) and for an empty/fully-degenerate result
/// (same "no coordinates written" rule as `wkb_write`).
pub(crate) fn geometry_to_compacted(
    geom: &Geometry,
    pool: &VertexPool,
) -> Result<Option<DecodedGeometry>> {
    let mut drops = Drops::default();
    let mut c = Compactor::new(pool);
    let kind = match geom.thetype {
        GeometryType::GeometryInstance => return Ok(None),
        GeometryType::MultiPoint | GeometryType::MultiLineString => {
            return Err(CityParquetError::Geometry(format!(
                "{:?} is not supported by the arrow-native encoding in phase 1 \
                 (design doc \"Type coverage (v1)\") — use --geometry-encoding wkb for this source",
                geom.thetype
            )));
        }
        GeometryType::MultiSurface | GeometryType::CompositeSurface => {
            let surfaces: Vec<Vec<Vec<usize>>> = boundaries(geom)?;
            let kept: Vec<Vec<&[usize]>> = surfaces
                .iter()
                .enumerate()
                .filter_map(|(pos, s)| normalise_surface(s, pos, &mut drops))
                .collect();
            let mut out = Vec::with_capacity(kept.len());
            for surface in &kept {
                out.push(c.surface(surface)?);
            }
            DecodedKind::MultiPolygon(out)
        }
        GeometryType::Solid => {
            let shells: Vec<Vec<Vec<Vec<usize>>>> = boundaries(geom)?;
            let mut pos = 0;
            let kept = normalise_shells(&shells, &mut pos, &mut drops);
            let mut out = Vec::with_capacity(kept.len());
            for face in &kept {
                out.push(c.surface(face)?);
            }
            DecodedKind::PolyhedralSurface(out)
        }
        GeometryType::MultiSolid | GeometryType::CompositeSolid => {
            let solids: Vec<Vec<Vec<Vec<Vec<usize>>>>> = boundaries(geom)?;
            let mut pos = 0;
            let mut members = Vec::with_capacity(solids.len());
            for solid in &solids {
                let kept = normalise_shells(solid, &mut pos, &mut drops);
                let mut out = Vec::with_capacity(kept.len());
                for face in &kept {
                    out.push(c.surface(face)?);
                }
                members.push(DecodedKind::PolyhedralSurface(out));
            }
            DecodedKind::GeometryCollection(members)
        }
    };
    if c.coords.is_empty() {
        return Ok(None);
    }
    Ok(Some(DecodedGeometry { coords: c.coords, kind }))
}
```

Register the module: add `mod arrow_geom_write;` to `crates/cityparquet/src/lib.rs` next to the existing `mod wkb_write;`/`mod wkb_read;` lines.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p cityparquet arrow_geom_write::`
Expected: PASS (both tests from Step 2).

- [ ] **Step 5: Write + pass a `Solid` test (shell-flattening + drops parity with WKB)**

Add to the same test module:

```rust
#[test]
fn solid_two_shells_flattens_faces_like_wkb_and_reports_no_shell_distinction() {
    // A minimal 2-shell Solid: exterior (1 face, a triangle) + one interior
    // cavity face (also a triangle) sharing no vertices with the exterior.
    let vertices: Vec<Vec<i64>> = vec![
        vec![0, 0, 0], vec![1, 0, 0], vec![0, 1, 0],
        vec![0, 0, 1], vec![1, 0, 1], vec![0, 1, 1],
    ];
    let pool = VertexPool::new(&vertices, &transform_identity());
    let geom = Geometry {
        thetype: GeometryType::Solid,
        lod: Some("2".to_string()),
        boundaries: serde_json::json!([
            [ [[0, 1, 2]] ],
            [ [[3, 4, 5]] ]
        ]),
        semantics: None, material: None, texture: None, template: None, transformation_matrix: None,
    };
    let decoded = geometry_to_compacted(&geom, &pool).unwrap().unwrap();
    assert_eq!(decoded.coords.len(), 6);
    match &decoded.kind {
        // Flattened to 2 faces, shell boundary NOT represented here —
        // exactly mirroring wkb_write::geometry_to_wkb's PolyhedralSurfaceZ
        // output (shell structure lives only in geometry_properties.shells,
        // unchanged, design doc "Face traversal order").
        DecodedKind::PolyhedralSurface(faces) => assert_eq!(faces.len(), 2),
        other => panic!("expected PolyhedralSurface, got {other:?}"),
    }
}
```

Run: `cargo test -p cityparquet arrow_geom_write::` — expect PASS immediately
(no implementation change needed; this test exercises the `Solid` branch
already written in Step 3). If it fails, the `Solid` branch has a bug — fix
it before proceeding, this is a real regression, not an expected-fail step.

- [ ] **Step 6: Commit**

```bash
git add crates/cityparquet/src/wkb_write.rs crates/cityparquet/src/arrow_geom_write.rs crates/cityparquet/src/lib.rs
git commit -m "feat(arrow-geom): geometry_to_compacted — distinct-index vertex pool compaction"
```

---

### Task 5: `arrow_geom_write.rs` — `DecodedGeometry` → Arrow builders

**Files:**
- Modify: `crates/cityparquet/src/arrow_geom_write.rs`

**Interfaces:**
- Consumes: `DecodedGeometry`/`DecodedKind` (Task 4's output), `arrow_native_geometry_data_type`/`arrow_native_vertices_data_type` (Task 1).
- Produces: `pub(crate) struct ArrowGeomBuilders { geometry: ListBuilder<...>, vertices: ListBuilder<StructBuilder> }` with `fn new() -> Self`, `fn append_value(&mut self, decoded: &DecodedGeometry)`, `fn append_null(&mut self)`, `fn finish(self) -> (ArrayRef, ArrayRef)` — Task 6 owns one of these per `GeometrySlot`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn arrow_geom_builders_round_trip_a_multisurface_through_arrow_arrays() {
    use arrow_array::{Array, ListArray, StructArray};

    let decoded = DecodedGeometry {
        coords: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]],
        kind: DecodedKind::MultiPolygon(vec![vec![vec![0, 1, 2]], vec![vec![0, 2, 3]]]),
    };
    let mut b = ArrowGeomBuilders::new();
    b.append_value(&decoded);
    b.append_null();
    let (geometry, vertices) = b.finish();

    let geom_list = geometry.as_any().downcast_ref::<ListArray>().unwrap();
    assert_eq!(geom_list.len(), 2);
    assert!(geom_list.is_valid(0));
    assert!(geom_list.is_null(1));

    let vert_list = vertices.as_any().downcast_ref::<ListArray>().unwrap();
    assert!(vert_list.is_valid(0));
    let row0_vertices = vert_list.value(0);
    let structs = row0_vertices.as_any().downcast_ref::<StructArray>().unwrap();
    assert_eq!(structs.len(), 4, "4 distinct vertices, matching the Task-4 compaction test");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p cityparquet arrow_geom_builders_round_trip`
Expected: FAIL — `ArrowGeomBuilders` doesn't exist.

- [ ] **Step 3: Implement `ArrowGeomBuilders`**

Add to `arrow_geom_write.rs` (imports go at the top of the file, alongside the existing ones):

```rust
use std::sync::Arc;

use arrow_array::builder::{ArrayBuilder, Float64Builder, Int32Builder, ListBuilder, StructBuilder};
use arrow_array::ArrayRef;
use arrow_schema::Fields;

use cityparquet_schema::model::{arrow_native_geometry_data_type, arrow_native_vertices_data_type};

/// The Arrow builder pair for one `geometry_lod*`/`geometry_vertices_lod*`
/// column pair, one call site per `GeometrySlot` (Task 6). Nesting mirrors
/// `arrow_native_geometry_data_type`'s shape exactly — this is the
/// `ListBuilder<StructBuilder>`-of-nested-lists pattern this crate already
/// uses for the `address` reserved column (`encode.rs`'s
/// `ListBuilder::new(StructBuilder::from_fields(...))`), extended to five
/// nesting levels (solid/shell/face/ring/index) via `ListBuilder<Box<dyn
/// ArrayBuilder>>` at each intermediate level, downcast back with
/// `.values().as_any_mut().downcast_mut()` at each level going in.
pub(crate) struct ArrowGeomBuilders {
    geometry: ListBuilder<Box<dyn ArrayBuilder>>, // solid
    vertices: ListBuilder<StructBuilder>,
}

fn vertex_struct_fields() -> Fields {
    match arrow_native_vertices_data_type() {
        arrow_schema::DataType::List(item) => match item.data_type() {
            arrow_schema::DataType::Struct(fields) => fields.clone(),
            _ => unreachable!("arrow_native_vertices_data_type item must be Struct"),
        },
        _ => unreachable!("arrow_native_vertices_data_type must be List"),
    }
}

impl ArrowGeomBuilders {
    pub(crate) fn new() -> Self {
        // Build the nested child builder by hand, innermost-out, since
        // `make_builder` alone can't express "ListBuilder<StructBuilder>"
        // for the vertex-pool column (it would give ListBuilder<Box<dyn
        // ArrayBuilder>> requiring the same downcast dance as `geometry`
        // below) — being explicit here keeps `append_value` simpler.
        let vertices = ListBuilder::new(StructBuilder::new(
            vertex_struct_fields(),
            vec![
                Box::new(Float64Builder::new()),
                Box::new(Float64Builder::new()),
                Box::new(Float64Builder::new()),
            ],
        ));

        // geometry: solid -> shell -> face -> ring -> Int32Builder, built via
        // `make_builder`-equivalent boxed builders at every level.
        let ring: Box<dyn ArrayBuilder> = Box::new(ListBuilder::new(Int32Builder::new()));
        let face: Box<dyn ArrayBuilder> = Box::new(ListBuilder::new(ring));
        let shell: Box<dyn ArrayBuilder> = Box::new(ListBuilder::new(face));
        let geometry = ListBuilder::new(shell);

        Self { geometry, vertices }
    }

    pub(crate) fn append_null(&mut self) {
        self.geometry.append(false);
        self.vertices.append(false);
    }

    pub(crate) fn append_value(&mut self, decoded: &DecodedGeometry) {
        // --- vertices ---
        let vb = &mut self.vertices;
        for c in &decoded.coords {
            let sb = vb.values();
            sb.field_builder::<Float64Builder>(0).unwrap().append_value(c[0]);
            sb.field_builder::<Float64Builder>(1).unwrap().append_value(c[1]);
            sb.field_builder::<Float64Builder>(2).unwrap().append_value(c[2]);
            sb.append(true);
        }
        vb.append(true);

        // --- geometry: pad to the unified solid/shell/face/ring/index shape ---
        self.append_kind(&decoded.kind);
        self.geometry.append(true);
    }

    /// Appends one `DecodedKind` at the "solid" level of `self.geometry`,
    /// padding MultiPolygon (surface types) to solid-count=1/shell-count=1
    /// (design doc "padding dimensions" — no shell/cavity meaning).
    fn append_kind(&mut self, kind: &DecodedKind) {
        match kind {
            DecodedKind::MultiPolygon(faces) => {
                self.push_solid(std::slice::from_ref(&PaddedShell::One(faces)));
            }
            DecodedKind::PolyhedralSurface(faces) => {
                self.push_solid(&[PaddedShell::One(faces)]);
            }
            DecodedKind::GeometryCollection(members) => {
                for member in members {
                    match member {
                        DecodedKind::PolyhedralSurface(faces) => {
                            self.push_one_solid_one_shell(faces);
                        }
                        other => unreachable!(
                            "GeometryCollection member must be PolyhedralSurface \
                             (MultiSolid/CompositeSolid invariant) — got {other:?}"
                        ),
                    }
                }
            }
            DecodedKind::MultiPoint(_) | DecodedKind::MultiLineString(_) => unreachable!(
                "phase-1 scope excludes MultiPoint/MultiLineString (Task 4 already \
                 rejects them before a DecodedGeometry with this kind can exist)"
            ),
        }
    }

    // NOTE: the two helper shapes below (`PaddedShell`, `push_solid`,
    // `push_one_solid_one_shell`) exist only to keep `append_kind` readable;
    // an implementer should feel free to inline/simplify once this compiles
    // and the tests pass, per this repo's normal refactor-while-green step.
    fn push_one_solid_one_shell(&mut self, faces: &[Vec<Vec<usize>>]) {
        self.push_solid(&[PaddedShell::One(faces)]);
    }

    fn push_solid(&mut self, shells: &[PaddedShell<'_>]) {
        let solid_values = self.geometry.values(); // &mut Box<dyn ArrayBuilder> = shell-level ListBuilder
        let shell_builder = solid_values
            .as_any_mut()
            .downcast_mut::<ListBuilder<Box<dyn ArrayBuilder>>>()
            .expect("geometry builder's solid level must hold a shell-level ListBuilder");
        for shell in shells {
            let PaddedShell::One(faces) = shell;
            let shell_values = shell_builder.values(); // &mut Box<dyn ArrayBuilder> = face-level ListBuilder
            let face_builder = shell_values
                .as_any_mut()
                .downcast_mut::<ListBuilder<Box<dyn ArrayBuilder>>>()
                .expect("shell level must hold a face-level ListBuilder");
            for rings in faces.iter() {
                let face_values = face_builder.values(); // &mut Box<dyn ArrayBuilder> = ring-level ListBuilder
                let ring_builder = face_values
                    .as_any_mut()
                    .downcast_mut::<ListBuilder<Box<dyn ArrayBuilder>>>()
                    .expect("face level must hold a ring-level ListBuilder");
                for ring in rings {
                    let index_builder = ring_builder
                        .values()
                        .as_any_mut()
                        .downcast_mut::<Int32Builder>()
                        .expect("ring level must hold an Int32Builder");
                    for &idx in ring {
                        index_builder.append_value(idx as i32);
                    }
                    ring_builder.append(true); // close one ring
                }
                face_builder.append(true); // close one face
            }
            shell_builder.append(true); // close one shell
        }
        solid_values
            .as_any_mut()
            .downcast_mut::<ListBuilder<Box<dyn ArrayBuilder>>>()
            .unwrap(); // (already borrowed above; kept for readability of the pairing)
    }

    pub(crate) fn finish(mut self) -> (ArrayRef, ArrayRef) {
        (Arc::new(self.geometry.finish()), Arc::new(self.vertices.finish()))
    }
}

enum PaddedShell<'a> {
    One(&'a [Vec<Vec<usize>>]),
}
```

**Expect real compile friction here.** The `push_solid` helper above almost
certainly needs adjustment once it actually compiles against `arrow-array`
58.3.0's exact `ListBuilder<Box<dyn ArrayBuilder>>::values()` return type and
`ArrayBuilder`'s `as_any_mut` trait bound — this is genuinely the trickiest
code in this whole plan (5 levels of boxed nested builders). Treat Step 3 as
a **first attempt to compile**, not a guarantee: run `cargo build -p
cityparquet` repeatedly, fix type errors one at a time (the compiler's
"expected `X`, found `Y`" messages plus `arrow-array-58.3.0`'s own module docs
on `struct_builder.rs` — already confirmed in this plan's research to contain
a full worked doctest for exactly this `List<Struct<List<Struct>>>`-shaped
nesting pattern — are the two references to lean on), and only move to Step 4
once it compiles.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p cityparquet arrow_geom_builders_round_trip`
Expected: PASS.

- [ ] **Step 5: Extend the test to a `Solid` (padding dimensions applied) and a `MultiSolid` (2 members)**

```rust
#[test]
fn arrow_geom_builders_pad_solid_and_flatten_multisolid_members() {
    use arrow_array::{Array, ListArray};

    let solid = DecodedGeometry {
        coords: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        kind: DecodedKind::PolyhedralSurface(vec![vec![vec![0, 1, 2]]]),
    };
    let mut b = ArrowGeomBuilders::new();
    b.append_value(&solid);
    let (geometry, _vertices) = b.finish();
    let outer = geometry.as_any().downcast_ref::<ListArray>().unwrap();
    let solids_row0 = outer.value(0); // List<shell>
    let shells = solids_row0.as_any().downcast_ref::<ListArray>().unwrap();
    assert_eq!(shells.len(), 1, "a bare Solid pads to solid-count 1 (design doc: not a semantic distinction)");

    let multisolid = DecodedGeometry {
        coords: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [5.0, 5.0, 5.0], [6.0, 5.0, 5.0], [5.0, 6.0, 5.0]],
        kind: DecodedKind::GeometryCollection(vec![
            DecodedKind::PolyhedralSurface(vec![vec![vec![0, 1, 2]]]),
            DecodedKind::PolyhedralSurface(vec![vec![vec![3, 4, 5]]]),
        ]),
    };
    let mut b2 = ArrowGeomBuilders::new();
    b2.append_value(&multisolid);
    let (geometry2, _) = b2.finish();
    let outer2 = geometry2.as_any().downcast_ref::<ListArray>().unwrap();
    let solids_row0_2 = outer2.value(0);
    let solids2 = solids_row0_2.as_any().downcast_ref::<ListArray>().unwrap();
    assert_eq!(solids2.len(), 2, "MultiSolid with 2 members -> solid-count 2");
}
```

Run: `cargo test -p cityparquet arrow_geom_builders_pad_solid_and_flatten_multisolid_members` — expect PASS without further implementation change (exercises the already-written `append_kind` branches); if it fails, fix the bug before proceeding.

- [ ] **Step 6: Commit**

```bash
git add crates/cityparquet/src/arrow_geom_write.rs
git commit -m "feat(arrow-geom): ArrowGeomBuilders — DecodedGeometry to nested Arrow arrays"
```

---

### Task 6: Wire the writer into `encode.rs`

**Files:**
- Modify: `crates/cityparquet/src/encode.rs`

**Interfaces:**
- Consumes: `arrow_geom_write::{geometry_to_compacted, ArrowGeomBuilders}` (Tasks 4-5), `ConvertOptions.geometry_encoding` (Task 2).
- Produces: `convert`/`convert_buffered` (whichever function ultimately owns `RowWriter`) writes real, non-empty `geometry_lod*`/`geometry_vertices_lod*` columns when `geometry_encoding == ArrowNative`.

- [ ] **Step 1: Extend `GeometrySlotData`/`GeometrySlot` to hold either encoding's payload**

In `crates/cityparquet/src/encode.rs`, change:

```rust
struct GeometrySlotData {
    bytes: Vec<u8>,
    properties: GeometryProperties,
    material: Option<Value>,
    texture: Option<Value>,
}
```

to:

```rust
enum GeometryPayload {
    Wkb(Vec<u8>),
    ArrowNative(crate::wkb_read::DecodedGeometry),
}

struct GeometrySlotData {
    payload: GeometryPayload,
    properties: GeometryProperties,
    material: Option<Value>,
    texture: Option<Value>,
}
```

and:

```rust
struct GeometrySlot {
    key: String,
    geometry: GeometryBuilder,      // was: BinaryBuilder
    properties: GeometryPropertiesBuilder,
    material: StringBuilder,
    texture: StringBuilder,
}

enum GeometryBuilder {
    Wkb(BinaryBuilder),
    ArrowNative(crate::arrow_geom_write::ArrowGeomBuilders),
}
```

- [ ] **Step 2: Update `accumulate_geometry` to branch on encoding**

Change the line noted in this plan's research (`let Some(outcome) = geometry_to_wkb(geom, pool)? else { continue };`) to branch:

```rust
let payload = match encoding {
    GeometryEncoding::Wkb => {
        let Some(outcome) = geometry_to_wkb(geom, pool)? else { continue };
        union_bbox(&mut acc.own_bbox, outcome.bbox);
        stats.degenerate_rings_dropped += outcome.dropped_rings;
        stats.degenerate_surfaces_dropped += outcome.dropped_surfaces.len();
        // ... existing rewrite_geometry_appearance call, unchanged ...
        GeometryPayload::Wkb(outcome.bytes)
    }
    GeometryEncoding::ArrowNative => {
        let Some(decoded) = crate::arrow_geom_write::geometry_to_compacted(geom, pool)? else { continue };
        let bbox = bbox_of(&decoded); // new small helper, see Step 2b
        union_bbox(&mut acc.own_bbox, bbox);
        // No WKB-specific "dropped_rings"/"dropped_surfaces" from THIS call —
        // geometry_to_compacted reuses the same normalise_* helpers
        // internally but doesn't currently surface their Drops externally.
        // If the round-trip test (Task 10) needs these stats for parity
        // with the WKB path's ConvertReport fields, extend
        // geometry_to_compacted's return type to also expose a `Drops`
        // (mirroring WkbOutcome) rather than silently under-reporting —
        // decide based on what Task 10's assertions actually need.
        GeometryPayload::ArrowNative(decoded)
    }
};
```

where `encoding: GeometryEncoding` needs threading into `accumulate_geometry`'s
own signature from whatever caller already has `ConvertOptions`/`ScanResult`
in scope (trace the existing call chain from `RowWriter::push_object`
upward — grep `accumulate_geometry(`).

- [ ] **Step 2b: Add `bbox_of` for a `DecodedGeometry`**

```rust
/// Computes the same `[xmin,ymin,zmin,xmax,ymax,zmax]` shape
/// `WkbOutcome::bbox` provides, from a `DecodedGeometry`'s coords —
/// used by the arrow-native path in place of `wkb_write::Bbox`, which is
/// private to `wkb_write.rs` and WKB-shaped (accumulates while writing
/// bytes) rather than reusable standalone.
fn bbox_of(decoded: &crate::wkb_read::DecodedGeometry) -> [f64; 6] {
    let mut b = [f64::INFINITY, f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for c in &decoded.coords {
        for i in 0..3 {
            b[i] = b[i].min(c[i]);
            b[i + 3] = b[i + 3].max(c[i]);
        }
    }
    b
}
```

- [ ] **Step 3: Update `RowWriter::new`, `push_object`, `finish_arrays`**

`new_slot` (mirrors `encode.rs:1151-1168` per this plan's research) picks the
builder variant from `encoding`:

```rust
let new_slot = |key: String| GeometrySlot {
    key,
    geometry: match encoding {
        GeometryEncoding::Wkb => GeometryBuilder::Wkb(BinaryBuilder::new()),
        GeometryEncoding::ArrowNative => GeometryBuilder::ArrowNative(crate::arrow_geom_write::ArrowGeomBuilders::new()),
    },
    properties: GeometryPropertiesBuilder::new(),
    material: StringBuilder::new(),
    texture: StringBuilder::new(),
};
```

`push_object`'s append branch (mirrors `encode.rs:1463-1484`):

```rust
match acc.slots.get(&slot.key) {
    Some(data) => {
        match (&mut slot.geometry, &data.payload) {
            (GeometryBuilder::Wkb(b), GeometryPayload::Wkb(bytes)) => b.append_value(bytes),
            (GeometryBuilder::ArrowNative(b), GeometryPayload::ArrowNative(decoded)) => b.append_value(decoded),
            _ => unreachable!("GeometryBuilder/GeometryPayload variant mismatch — encoding is fixed per RowWriter, cannot happen"),
        }
        slot.properties.append_value(&data.properties)?;
        // material/texture handling unchanged
    }
    None => {
        match &mut slot.geometry {
            GeometryBuilder::Wkb(b) => b.append_null(),
            GeometryBuilder::ArrowNative(b) => b.append_null(),
        }
        slot.properties.append_null();
        slot.material.append_null();
        slot.texture.append_null();
    }
}
```

`finish_arrays` (mirrors `encode.rs:1536-1561`) — the arrow-native branch
pushes **two** arrays (geometry + vertices) where the WKB branch pushes one,
matching the schema Task 1 built (a `geometry_lod*` immediately followed by
`geometry_vertices_lod*` only for arrow-native):

```rust
for slot in &mut self.geometry_slots {
    match &mut slot.geometry {
        GeometryBuilder::Wkb(b) => arrays.push(Arc::new(b.finish())),
        GeometryBuilder::ArrowNative(b) => {
            // b is owned here via mem::replace or by restructuring GeometryBuilder
            // to hold an Option<ArrowGeomBuilders> if `finish(self)`'s by-value
            // signature (Task 5) doesn't fit a `&mut self` loop cleanly — pick
            // whichever is less invasive once this compiles.
            let (geometry_array, vertices_array) = std::mem::replace(
                b,
                crate::arrow_geom_write::ArrowGeomBuilders::new(),
            ).finish();
            arrays.push(geometry_array);
            arrays.push(vertices_array); // schema order must match Task 1's field order exactly
        }
    }
    arrays.push(slot.properties.finish());
    arrays.push(Arc::new(slot.material.finish()));
    arrays.push(Arc::new(slot.texture.finish()));
}
```

- [ ] **Step 4: Revisit Task 2's end-to-end test**

Run: `cargo test -p cityparquet geometry_encoding_arrow_native_writes_nested_geometry_column`
Expected: PASS now. If the LoD suffix/output filename guessed in Task 2 was
wrong, fix the test's literals against the real output (`report.files` from
the `ConvertReport`, and `delft.city.jsonl`'s real `Solid` LoDs confirmed by
this plan's research as `1.2`/`1.3`/`2.2`).

- [ ] **Step 5: Run the FULL existing test suite**

Run: `cargo test -p cityparquet`
Expected: PASS — confirms the WKB path (default `GeometryEncoding::Wkb`,
`GeometryBuilder::Wkb`/`GeometryPayload::Wkb` branches) is behaviourally
identical to before this task; every pre-existing test exercises only that
path and must be completely unaffected.

- [ ] **Step 6: Commit**

```bash
git add crates/cityparquet/src/encode.rs
git commit -m "feat(encode): wire arrow-native geometry writer into RowWriter"
```

---

### Task 7: `arrow_geom_read.rs` — Arrow columns → `DecodedGeometry`

**Files:**
- Create: `crates/cityparquet/src/arrow_geom_read.rs`
- Modify: `crates/cityparquet/src/lib.rs` (add `mod arrow_geom_read;`)

**Interfaces:**
- Consumes: one row of the `geometry_lod*` (`ListArray`) and `geometry_vertices_lod*` (`ListArray`) columns, plus that row's `geometry_properties.type_name: &str` (Task 8 supplies it — this is the dispatch-by-type invariant, design doc "Critical invariant": never infer the CM type from nesting shape).
- Produces: `pub(crate) fn decode_row(geometry: &ListArray, vertices: &ListArray, row: usize, type_name: &str) -> Result<DecodedGeometry>`.

- [ ] **Step 1: Write the failing test — round-trips Task 5's own output**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::arrow_geom_write::ArrowGeomBuilders; // test-only cross-module use is fine within one crate

    #[test]
    fn decode_row_inverts_arrow_geom_builders_for_a_solid() {
        let decoded_in = DecodedGeometry {
            coords: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            kind: DecodedKind::PolyhedralSurface(vec![vec![vec![0, 1, 2]]]),
        };
        let mut b = ArrowGeomBuilders::new();
        b.append_value(&decoded_in);
        let (geometry, vertices) = b.finish();
        let geometry = geometry.as_any().downcast_ref::<arrow_array::ListArray>().unwrap();
        let vertices = vertices.as_any().downcast_ref::<arrow_array::ListArray>().unwrap();

        let decoded_out = decode_row(geometry, vertices, 0, "Solid").unwrap();
        assert_eq!(decoded_out, decoded_in);
    }

    #[test]
    fn decode_row_strips_padding_dimensions_for_multisurface() {
        let decoded_in = DecodedGeometry {
            coords: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            kind: DecodedKind::MultiPolygon(vec![vec![vec![0, 1, 2]]]),
        };
        let mut b = ArrowGeomBuilders::new();
        b.append_value(&decoded_in);
        let (geometry, vertices) = b.finish();
        let geometry = geometry.as_any().downcast_ref::<arrow_array::ListArray>().unwrap();
        let vertices = vertices.as_any().downcast_ref::<arrow_array::ListArray>().unwrap();

        let decoded_out = decode_row(geometry, vertices, 0, "MultiSurface").unwrap();
        assert_eq!(decoded_out, decoded_in, "type_name=\"MultiSurface\" must strip the 2 padding dimensions ArrowGeomBuilders added, recovering the original MultiPolygon shape");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p cityparquet arrow_geom_read::`
Expected: FAIL — `decode_row` doesn't exist.

- [ ] **Step 3: Implement `decode_row`**

```rust
//! Arrow `geometry_lod*`/`geometry_vertices_lod*` columns -> `DecodedGeometry`,
//! the inverse of `arrow_geom_write::ArrowGeomBuilders`. The row's
//! `geometry_properties.type_name` decides how to interpret/strip the
//! physical shape's padding dimensions (design doc "Critical invariant" —
//! never infer the CM type from nesting depth).

use arrow_array::{Array, Int32Array, ListArray, StructArray};
use cityparquet_schema::Result;

use crate::wkb_read::{DecodedGeometry, DecodedKind};

fn read_vertices(vertices: &ListArray, row: usize) -> Vec<[f64; 3]> {
    let row_values = vertices.value(row);
    let structs = row_values.as_any().downcast_ref::<StructArray>().unwrap();
    let x = structs.column(0).as_any().downcast_ref::<arrow_array::Float64Array>().unwrap();
    let y = structs.column(1).as_any().downcast_ref::<arrow_array::Float64Array>().unwrap();
    let z = structs.column(2).as_any().downcast_ref::<arrow_array::Float64Array>().unwrap();
    (0..structs.len()).map(|i| [x.value(i), y.value(i), z.value(i)]).collect()
}

fn read_ring(ring_list: &ListArray, i: usize) -> Vec<usize> {
    let row = ring_list.value(i);
    let ints = row.as_any().downcast_ref::<Int32Array>().unwrap();
    (0..ints.len()).map(|j| ints.value(j) as usize).collect()
}

fn read_face(face_list: &ListArray, i: usize) -> Vec<Vec<usize>> {
    let row = face_list.value(i);
    let rings = row.as_any().downcast_ref::<ListArray>().unwrap();
    (0..rings.len()).map(|j| read_ring(rings, j)).collect()
}

fn read_shell(shell_list: &ListArray, i: usize) -> Vec<Vec<Vec<usize>>> {
    let row = shell_list.value(i);
    let faces = row.as_any().downcast_ref::<ListArray>().unwrap();
    (0..faces.len()).map(|j| read_face(faces, j)).collect()
}

pub(crate) fn decode_row(
    geometry: &ListArray,
    vertices: &ListArray,
    row: usize,
    type_name: &str,
) -> Result<DecodedGeometry> {
    let coords = read_vertices(vertices, row);

    let solids_row = geometry.value(row);
    let solids = solids_row.as_any().downcast_ref::<ListArray>().unwrap(); // shell-level ListArray

    let kind = match type_name {
        "MultiSurface" | "CompositeSurface" => {
            // Padded to solid-count=1/shell-count=1 — strip both.
            debug_assert_eq!(solids.len(), 1, "MultiSurface/CompositeSurface must be padded to solid-count 1");
            let shells = read_shell(solids, 0);
            debug_assert_eq!(shells.len(), 1, "MultiSurface/CompositeSurface must be padded to shell-count 1");
            DecodedKind::MultiPolygon(shells.into_iter().next().unwrap())
        }
        "Solid" => {
            debug_assert_eq!(solids.len(), 1, "Solid must be padded to solid-count 1");
            let shell_list = solids.value(0);
            let faces_per_shell = shell_list.as_any().downcast_ref::<ListArray>().unwrap();
            let mut faces = Vec::new();
            for shell_idx in 0..faces_per_shell.len() {
                faces.extend(read_face(faces_per_shell, shell_idx));
            }
            DecodedKind::PolyhedralSurface(faces)
        }
        "MultiSolid" | "CompositeSolid" => {
            let mut members = Vec::with_capacity(solids.len());
            for solid_idx in 0..solids.len() {
                let shell_list = solids.value(solid_idx);
                let faces_per_shell = shell_list.as_any().downcast_ref::<ListArray>().unwrap();
                let mut faces = Vec::new();
                for shell_idx in 0..faces_per_shell.len() {
                    faces.extend(read_face(faces_per_shell, shell_idx));
                }
                members.push(DecodedKind::PolyhedralSurface(faces));
            }
            DecodedKind::GeometryCollection(members)
        }
        other => {
            return Err(cityparquet_schema::CityParquetError::Geometry(format!(
                "arrow-native decode: unsupported geometry_properties.type {other:?} \
                 (phase-1 scope: MultiSurface/CompositeSurface/Solid/MultiSolid/CompositeSolid)"
            )));
        }
    };
    Ok(DecodedGeometry { coords, kind })
}
```

Register: add `mod arrow_geom_read;` to `crates/cityparquet/src/lib.rs`.

**Note on the `Solid` case above**: it currently flattens across shells
assuming exactly one shell per solid slot in the physical column when
`type_name == "Solid"`. A real `Solid` written by Task 5/6 can have
**multiple shells** (exterior + interior cavities — see Task 4's own
`solid_two_shells_flattens_faces_like_wkb_and_reports_no_shell_distinction`
test, which has 2 shells in one `Solid`). Verify the loop above (`for
shell_idx in 0..faces_per_shell.len() { faces.extend(...) }`) actually
handles that multi-shell case correctly against a real test before trusting
it — Step 4 below adds exactly that test; if it fails, this is the bug to
fix, not a sign the test is wrong.

- [ ] **Step 4: Run the two Step-1 tests, then add a multi-shell `Solid` round-trip test**

Run: `cargo test -p cityparquet arrow_geom_read::`
Expected: PASS for both Step-1 tests.

Add:

```rust
#[test]
fn decode_row_flattens_a_two_shell_solid_correctly() {
    let decoded_in = DecodedGeometry {
        coords: vec![[0.,0.,0.], [1.,0.,0.], [0.,1.,0.], [0.,0.,1.], [1.,0.,1.], [0.,1.,1.]],
        kind: DecodedKind::PolyhedralSurface(vec![vec![vec![0, 1, 2]], vec![vec![3, 4, 5]]]), // 2 faces, no shell tag (WKB-flattened shape, matching Task 4's own Solid test)
    };
    let mut b = ArrowGeomBuilders::new();
    b.append_value(&decoded_in);
    let (geometry, vertices) = b.finish();
    let geometry = geometry.as_any().downcast_ref::<arrow_array::ListArray>().unwrap();
    let vertices = vertices.as_any().downcast_ref::<arrow_array::ListArray>().unwrap();
    let decoded_out = decode_row(geometry, vertices, 0, "Solid").unwrap();
    assert_eq!(decoded_out, decoded_in);
}
```

Run: `cargo test -p cityparquet arrow_geom_read::decode_row_flattens_a_two_shell_solid_correctly`
Expected: PASS. If not, fix `decode_row`'s `"Solid"` branch — this is exactly
the case the note above Step 4 flagged as needing real verification, not
assumption.

- [ ] **Step 5: Commit**

```bash
git add crates/cityparquet/src/arrow_geom_read.rs crates/cityparquet/src/lib.rs
git commit -m "feat(arrow-geom): decode_row — Arrow columns to DecodedGeometry"
```

---

### Task 8: Wire the reader into `decode.rs`

**Files:**
- Modify: `crates/cityparquet/src/decode.rs`

**Interfaces:**
- Consumes: `arrow_geom_read::decode_row` (Task 7).
- Produces: `decode_batch` correctly decodes an arrow-native-encoded `RecordBatch`, producing the same `DecodedObject` shape as the WKB path (unchanged downstream — `export.rs`/`compare.rs` need zero changes, confirmed by this plan's research: `compare.rs` never sees Parquet-level encoding, only `cjseq::Geometry`/`boundaries`, which `export.rs` builds from `DecodedGeometry` either way).

- [ ] **Step 1: Write the failing test**

This needs a real arrow-native-encoded `RecordBatch` to decode — the
simplest source is Task 2's own test fixture output. Add to
`crates/cityparquet/tests/convert_real_data.rs` (same file, extends the Task
2 test rather than duplicating its setup):

```rust
#[test]
fn arrow_native_roundtrip_decodes_back_to_the_same_semantic_geometry() {
    let dir = tempdir().unwrap();
    let mut opts = ConvertOptions::new(fixture_path("delft.city.jsonl"), dir.path().to_path_buf());
    opts.geometry_encoding = cityparquet_schema::types::GeometryEncoding::ArrowNative;
    convert(&opts).unwrap();

    let exported = dir.path().join("roundtrip.city.jsonl");
    export(&dir, &exported).unwrap(); // adjust to this crate's real `export` signature — grep `pub fn export` in export.rs

    let report = compare(
        &fixture_path("delft.city.jsonl"),
        &exported,
        CompareOptions::default(), // adjust to real options struct/defaults
    ).unwrap();
    assert!(report.is_semantically_equal(), "{report:?}"); // adjust to this crate's real CompareReport API — grep the `Compare` CLI subcommand's success-path output construction for the real success predicate/field name
}
```

(Multiple call sites in this test are marked "adjust" because `export`'s and
`compare`'s exact public signatures weren't pulled into this plan's research
in full — confirm against `crates/cityparquet/src/export.rs`'s and
`compare.rs`'s actual `pub fn` signatures before writing this test for real,
rather than guessing twice. This is the ONE step in this plan most likely to
need real API discovery at implementation time — treat it as such, not as a
sign the plan is wrong.)

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p cityparquet arrow_native_roundtrip_decodes_back_to_the_same_semantic_geometry`
Expected: FAIL — `decode_batch` doesn't know how to read an arrow-native
`geometry_lod*` column yet (either a downcast panic expecting `BinaryArray`,
or silently wrong output).

- [ ] **Step 3: Branch `decode_batch`'s geometry loop on encoding**

The exact line from this plan's research (`decode.rs:502`):
`let decoded = wkb_read::wkb_to_geometry(geom_arr.value(row))?;` sits inside
a loop over `geometry_arrays: Vec<(Option<Lod>, &BinaryArray, &StructArray)>`
(built earlier in `decode_batch` via `geometry_columns(schema)` +
`downcast::<BinaryArray>`, per `decode.rs:216,502` in this plan's research).
This needs to become encoding-aware at the point `geometry_arrays` is built,
not just at the point it's consumed, since a `BinaryArray` downcast is
already wrong for an arrow-native column. Change `geometry_columns`'s return
type (or add a sibling function) to also report each column's encoding
(readable from the `Schema`'s field `DataType` directly — a `DataType::List`
field is arrow-native, `DataType::Binary` is WKB, no separate metadata flag
needed since Task 1 made the two encodings' `DataType`s structurally
distinguishable), and thread that through so the per-row loop can do:

```rust
let decoded = match column_encoding {
    GeometryEncoding::Wkb => wkb_read::wkb_to_geometry(geom_binary_arr.value(row))?,
    GeometryEncoding::ArrowNative => {
        let type_name = geometry_properties_type_name(props_arr, row)?; // new small helper — read geometry_properties.type_name for this row, needed as decode_row's dispatch key; may already exist in some form in geometry_properties.rs, check before adding a duplicate
        crate::arrow_geom_read::decode_row(geom_list_arr, vertices_arr, row, &type_name)?
    }
};
```

where `vertices_arr` is the corresponding `geometry_vertices_lod*` column's
`ListArray`, looked up by the naming convention from Task 1
(`geometry_vertices_lod<M>_<m>`) the same way `geometry_lod*`/
`geometry_properties_lod*` are already paired up in `geometry_columns`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p cityparquet arrow_native_roundtrip_decodes_back_to_the_same_semantic_geometry`
Expected: PASS. This is the plan's main correctness gate — a real,
~1100-building fixture converting to arrow-native encoding, exporting back to
CityJSONSeq, and comparing semantically equal to the original source.

- [ ] **Step 5: Add the equivalent round-trip test for `lod3_railway.city.json`** (MultiSurface/CompositeSurface only, no Solid — confirms the padding-dimension path independently of the Solid path)

Mirror Step 1's test with `fixture_path("lod3_railway.city.json")` and
`--exclude-instances` passed to `compare` (this fixture has `GeometryInstance`s,
which the arrow-native encoding doesn't touch either way — same as WKB).

Run it, expect PASS.

- [ ] **Step 6: Run the FULL existing test suite one more time**

Run: `cargo test -p cityparquet -p cityparquet-schema -p cityparquet-cli`
Expected: PASS — confirms zero regression to the WKB path across the whole workspace.

- [ ] **Step 7: Commit**

```bash
git add crates/cityparquet/src/decode.rs crates/cityparquet/tests/convert_real_data.rs
git commit -m "feat(decode): wire arrow-native geometry reader into decode_batch; round-trip proof"
```

---

### Task 9: `city.schema.json` — extend `encoding`/`geometry_types` for the new value

**Files:**
- Modify: `crates/cityparquet-schema/assets/city.schema.json`
- Test: `crates/cityparquet/tests/metadata_schema_real_data.rs` (existing file — extends it, per this plan's research confirming it's the only consumer of this JSON Schema, test-only via `jsonschema::validator_for`)

**Interfaces:**
- Consumes: nothing new.
- Produces: `city.schema.json`'s `columns[].encoding`/`columns[].geometry_types` validate a real arrow-native `city` footer.

- [ ] **Step 1: Write the failing test**

Add to `crates/cityparquet/tests/metadata_schema_real_data.rs`, mirroring
its existing pattern (convert a fixture, load the schema, validate the real
`city` JSON) but with `ConvertOptions::geometry_encoding =
GeometryEncoding::ArrowNative`:

```rust
#[test]
fn arrow_native_footer_validates_against_city_schema_json() {
    let dir = tempdir().unwrap();
    let mut opts = ConvertOptions::new(fixture_path("delft.city.jsonl"), dir.path().to_path_buf());
    opts.geometry_encoding = cityparquet_schema::types::GeometryEncoding::ArrowNative;
    convert(&opts).unwrap();
    let metadata = std::fs::read_to_string(dir.path().join("metadata.json")).unwrap();
    let metadata: serde_json::Value = serde_json::from_str(&metadata).unwrap();
    let city = &metadata["assets"] /* adjust to this repo's real STAC-Item-shaped metadata.json structure — mirror an existing passing test in this same file for the real JSON path to the per-file `city` object rather than guessing */;
    let schema_text = std::fs::read_to_string(
        "crates/cityparquet-schema/assets/city.schema.json"
    ).unwrap();
    let schema_json: serde_json::Value = serde_json::from_str(&schema_text).unwrap();
    let validator = jsonschema::validator_for(&schema_json).unwrap();
    let errors: Vec<_> = validator.iter_errors(city).collect();
    assert!(errors.is_empty(), "{errors:?}");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p cityparquet arrow_native_footer_validates_against_city_schema_json`
Expected: FAIL — `geometry_types: ["Solid"]`/`["MultiSurface"]` (CityJSON
type names, no `" Z"` suffix) don't match the current regex
`^(GeometryCollection|(Multi)?(Point|LineString|Polygon)|PolyhedralSurface|TIN) Z$`,
and `encoding: "CityParquetArrowNative-v1"` doesn't match the current
`"type": "string"` (which is actually unconstrained today per this plan's
research — re-check: if `encoding` truly has no `enum`/`pattern` constraint
today, only `geometry_types` needs fixing; confirm by reading the schema
file directly in Step 3 before assuming both need a fix).

- [ ] **Step 3: Fix the schema**

Read `crates/cityparquet-schema/assets/city.schema.json` in full first (this
plan's earlier research only confirmed the `geometry_types` regex precisely;
confirm `encoding`'s exact current constraint before editing it). Extend
`geometry_types`'s `items` to accept either the existing WKB-type-name
pattern OR a CityJSON-type-name pattern, e.g.:

```json
"geometry_types": {
  "type": "array",
  "minItems": 1,
  "uniqueItems": true,
  "items": {
    "type": "string",
    "pattern": "^((GeometryCollection|(Multi)?(Point|LineString|Polygon)|PolyhedralSurface|TIN) Z|MultiSurface|CompositeSurface|Solid|MultiSolid|CompositeSolid)$"
  }
}
```

and, if `encoding` currently has no constraint, add one that permits both
values without over-constraining future encodings this schema doesn't know
about yet:

```json
"encoding": {
  "type": "string",
  "minLength": 1,
  "description": "\"WKB\" (normative) or \"CityParquetArrowNative-v1\" (experimental, arrow-native-type branch)."
}
```

(no `enum` — an unconstrained-but-documented string, matching this field's
apparent current looseness; tighten only if Step 3's read of the real file
shows it was already an `enum: ["WKB"]` that needs literally extending
instead.)

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p cityparquet arrow_native_footer_validates_against_city_schema_json`
Expected: PASS. Also run the pre-existing tests in this same file (WKB path) to confirm no regression.

- [ ] **Step 5: Commit**

```bash
git add crates/cityparquet-schema/assets/city.schema.json crates/cityparquet/tests/metadata_schema_real_data.rs
git commit -m "fix(schema): city.schema.json accepts the arrow-native encoding + CityJSON type names"
```

---

### Task 10: `compare` cross-check (confirm, don't change — per user review decision)

**Files:** none (verification-only task; see below for the one case where a code change IS warranted).

Per the design doc's resolved open item 2: no cross-encoding `compare`
support is needed, each encoding is compared to the original source
independently — Task 8 already does exactly this (Steps 1 and 5 both compare
an arrow-native-encoded round trip against the *original* fixture, never
WKB-output against arrow-native-output directly).

- [ ] **Step 1: Confirm `compare.rs` needed zero changes**

Run: `git diff --stat HEAD~9..HEAD -- crates/cityparquet/src/compare.rs` (or
equivalent — count commits back to before Task 4). Expected: **empty diff**.
If `compare.rs` needed ANY change to make Task 8's tests pass, that
contradicts this plan's Task 8 research finding (`compare.rs` operates on
`cjseq::Geometry`/`boundaries`, never on Parquet-level encoding) — stop and
investigate why before proceeding, since it means either the research was
wrong or something upstream (`export.rs`) is leaking encoding-specific shape
into `compare`'s input in a way it shouldn't.

- [ ] **Step 2: No commit needed for this task** (verification-only).

---

### Task 11: Draft the experimental encoding into `documents/`

**Files:**
- Modify: `documents/docs/04-design-decisions/02-geometry-encoding.mdx`
- Modify: `documents/docs/03-specification/03-geometry-semantics.mdx`
- Modify: `documents/docs/03-specification/05-metadata.mdx`

Per the design doc's resolved open item 3 (user review, 2026-07-25): spec
drafting happens now, not deferred to after the benchmark. `documents/` is
not a submodule — these edits land on the same root-repo `arrow-native-type`
branch already pushed, in the **root repo** (`/data2/hideba/cityparquet-paper`,
not `cityparquet-rs`) — this task's commits happen there, once the schema
shape is stable (i.e., after Task 9, when nothing about the type shapes is
expected to change further).

- [ ] **Step 1: Update the design-decision doc's "open" WKB section**

In `documents/docs/04-design-decisions/02-geometry-encoding.mdx`, find the
`<Badge variant="warning">open</Badge>` section titled *"WKB geometry
(including 3D), not native geometry or EWKB"* (confirmed present by this
plan's earlier research). Add a paragraph after the existing "Status" line:

```mdx
**Update (2026-07-25).** Option (d) — a custom Arrow/GeoArrow packed
extension type — is now under active evaluation on the `arrow-native-type`
branch (`cityparquet-rs`, `duckdb-cityjson`, `duckdb-3d`; see
`docs/superpowers/specs/2026-07-25-arrow-native-geometry-design.md` in the
parent workspace repo for the full design and rationale). Covers
`MultiSurface`/`CompositeSurface`/`Solid`/`MultiSolid`/`CompositeSolid` as an
opt-in alternative to WKB, using an indexed per-row vertex pool (not
coordinate-inline GeoArrow-style storage — CityParquet's shells/solids need a
deeper nesting than GeoArrow's own native types support, and the indexed
approach matches both `cityparquet-rs`'s and `duckdb-3d`'s existing in-memory
models more closely). This section will be updated with results once the
DuckDB query/scan performance benchmark (the branch's stated success
criterion) concludes, either promoting this to a decided alternative
encoding or recording why it wasn't adopted.
```

- [ ] **Step 2: Add the schema definition, marked experimental**

In `documents/docs/03-specification/03-geometry-semantics.mdx`, after the
existing "Geometry-type mapping" table, add a new subsection:

```mdx
### Arrow-native geometry encoding (experimental, `arrow-native-type` branch)

:::caution[Not part of the normative encoding yet]
This section documents an **experimental, opt-in alternative** to the WKB
encoding above, developed on the `arrow-native-type` branch. It is not
decided — see [the design-decision doc](/design-decisions/geometry-encoding)
for status. A file using it declares so explicitly via
`city.columns[].encoding` (see [metadata](/specification/metadata)); absent
that declaration, WKB is assumed as normal.
:::

When `city.columns[].encoding` is `"CityParquetArrowNative-v1"` rather than
`"WKB"`, `MultiSurface`, `CompositeSurface`, `Solid`, `MultiSolid`, and
`CompositeSolid` share one unified physical shape — `geometry_lod<M>_<m>` is

```text
List<                   -- "solid" position (padding, length 1, for surface types)
  List<                   -- "shell" position (padding, length 1, for surface types)
    List<                  -- Face
      List<List<Int32>>      -- Ring -> vertex-pool index (ring 0 = exterior, rest = holes)
    >
  >
>
```

paired with a sibling `geometry_vertices_lod<M>_<m>`:
`List<Struct<x: Float64, y: Float64, z: Float64>>` — this row's
distinct-source-index-compacted vertex pool (not coordinate-value
deduplicated: two different source indices with identical coordinates
remain two separate pool entries). Rings do not repeat the closing vertex
(implicitly closed, unlike WKB). Face traversal order matches the WKB
encoding exactly, so `geometry_properties_lod*` (`face_semantics`, `shells`)
is reused completely unmodified — a consumer dispatches which CM type a row
holds via `geometry_properties_lod*.type`, never by inspecting the nesting
shape (a padded `MultiSurface` and a real single-shell `Solid` have the
same shape signature).

`Point`/`MultiPoint`/`MultiLineString`/`GeometryInstance` are not supported
under this encoding yet.
:::
```

- [ ] **Step 3: Add the `encoding` value to the metadata spec**

In `documents/docs/03-specification/05-metadata.mdx`'s `city.columns`
entries table, change the `encoding` row's description from `` `"WKB"` — the
only encoding CityParquet supports at present. `` to:

```mdx
| `encoding` | ✔ | `"WKB"` — the only *normative* encoding. `"CityParquetArrowNative-v1"` is an experimental alternative under evaluation — see [the geometry-encoding design decision](/design-decisions/geometry-encoding) and [its draft schema](/specification/geometry-semantics#arrow-native-geometry-encoding-experimental-arrow-native-type-branch). |
```

- [ ] **Step 4: Verify the docs site builds**

Run whatever this repo's `documents/` build command is (check
`documents/blume.config.ts`/`package.json` — likely `npm run build` or
similar via the `blume` doc-site skill mentioned in the parent repo's
`CLAUDE.md`) and confirm no MDX/link errors from the new content.

- [ ] **Step 5: Commit (in the ROOT repo, not `cityparquet-rs`)**

```bash
cd /data2/hideba/cityparquet-paper
git add documents/docs/04-design-decisions/02-geometry-encoding.mdx documents/docs/03-specification/03-geometry-semantics.mdx documents/docs/03-specification/05-metadata.mdx
git commit -m "docs(spec): draft arrow-native geometry encoding as experimental (arrow-native-type)"
git push origin arrow-native-type
```

---

## Final steps (after Task 11)

- [ ] Bump the root repo's `cityparquet-rs` submodule pointer on the root's
  `arrow-native-type` branch to this crate's final commit, and push (matches
  this repo's own established multi-repo submodule convention — see
  `.superpowers/sdd/progress.md` for the precedent pattern).
- [ ] Run `codex exec -m gpt-5.6-sol -s read-only` over the full diff (this
  repo's own review convention, and the user's explicit request for
  `gpt-5.6-sol` as reviewer for this whole experiment) before considering
  `cityparquet-rs`'s leg of the branch done. Address findings the same way
  the design doc itself was reviewed (verify claims against source before
  accepting them — `superpowers:receiving-code-review` discipline).
- [ ] Only once this is done and reviewed: write the `duckdb-cityjson` and
  `duckdb-3d` implementation plans, now able to reference this crate's real,
  merged `arrow_native_geometry_data_type()`/`arrow_native_vertices_data_type()`
  shapes and real fixture Parquet files instead of the design doc's
  description of them.

---

## Self-Review

**Spec coverage** (against `docs/superpowers/specs/2026-07-25-arrow-native-geometry-design.md`):
- Unified physical shape, padding dimensions, dispatch-by-type invariant: Tasks 1, 5, 7. ✓
- Distinct-source-index compaction (not coordinate dedup): Task 4. ✓
- `Struct<x,y,z>` not `FixedSizeList`: Task 1. ✓
- Ring-closure convention (no repeated closing vertex): inherited for free — Task 4 reuses `wkb_write`'s `normalise_ring`, which already produces open rings; `wkb_write`'s own closure-repeat happens only in `write_polygon`, which the new path never calls. ✓
- Nullability/pairing invariants: Task 1 (field-level `false`/`true` choices matching the design doc's "non-null once populated" rule) + Task 6 (geometry/vertices/properties append together or not at all, inherited from the existing per-slot loop structure). ✓
- Encoding declared via `city.columns[].encoding`, not `city.other`: Task 9 (JSON Schema validation) + Task 2 Step 4b (the actual footer field — found by reading `metadata.rs`/`scan.rs` directly during self-review: `CityColumnEntry::new` hardcoded `"WKB"` regardless of caller; fixed inline rather than left as a gap).
- `geo` omission: Task 3. ✓
- Benchmark harness: explicitly out of scope for this plan (design doc: needs a new DuckDB-side harness, which only makes sense once `duckdb-cityjson`/`duckdb-3d` exist) — correctly deferred, not a gap.
- `Point` dropped from phase 1: Task 4 errors clearly rather than mis-encoding. ✓
- `documents/` spec drafting alongside code: Task 11. ✓

**Placeholder scan:** no "TBD"/"handle appropriately" left in; every step has real code or an explicit, named reason a literal needs confirming against the real codebase at implementation time (Task 2's fixture LoD suffix, Task 8's `export`/`compare` signatures, Task 9's `encoding` field constraint) — these are flagged as "confirm against real source", which is honest given this plan was written by reading source, not by compiling it, not a disguised placeholder.

**Type consistency:** `DecodedGeometry`/`DecodedKind` (Task 4's target) match `GeometryPayload::ArrowNative`'s inner type (Task 6) and `decode_row`'s return type (Task 7) throughout. `GeometryEncoding::{Wkb, ArrowNative}` used consistently from Task 1 through Task 9. `ArrowGeomBuilders::{new, append_value, append_null, finish}` used identically in Tasks 5 and 6.
