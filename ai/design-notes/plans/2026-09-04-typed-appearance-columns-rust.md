# Typed Appearance Columns — cityparquet-rs (phase 2) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the Rust reference implementation write and read `material_lod*` / `texture_lod*` as the specification's typed `MAP` columns, flat per WKB face, and round-trip them to CityJSON and CityGML exactly as before.

**Architecture:** One new shared module, `crates/core/src/appearance_columns.rs`, owns the Arrow-decoupled cell types, the builders and the readers for both columns, mirroring `geometry_properties.rs`. The encoder flattens a CityJSON `material` / `texture` map to per-WKB-face cells by reusing the exact walk that produces `face_semantics` (`flatten_values` / `count_boundary_faces` / `values_nesting_depth` plus the writer-dropped positions). Readers (`export.rs`, the CityGML writer, the template sidecar) get a flat theme map back and re-nest it from `shells` with the same partition helpers `rebuild_semantics` uses. The comparator canonicalises both sides to the flat form, so a source `value` broadcast compares equal to the exporter's expanded `values`.

**Tech Stack:** Rust 2024, arrow-array/arrow-schema/parquet 58 (`MapBuilder`, `ListBuilder::with_field`, `StructBuilder`), serde_json, the workspace at `lib/cityparquet-rs`.

**Spec:** `ai/design-notes/specs/2026-09-04-typed-appearance-columns-design.md` and the normative pages `documents/docs/03-specification/04-appearance-templates.mdx` (section *material / texture columns*, incl. invariants) and `03-geometry-semantics.mdx`. Read both before any task; the spec is the binding authority.

## Global Constraints

- The Arrow types, exactly (Parquet-canonical MAP field names so every Parquet reader sees the standard `key_value`/`key`/`value` groups):
  - `material_lod*` = `Map(Field("key_value", Struct[ Field("key", Utf8, non-null), Field("value", List(Field("item", Int64, nullable)), non-null) ], non-null), keys_sorted=false)`
  - `texture_lod*` = `Map(Field("key_value", Struct[ Field("key", Utf8, non-null), Field("value", List(Field("item", List(Field("item", Struct[ Field("id", Int64, nullable), Field("uv", List(Field("item", List(Field("item", Float64, non-null), non-null)), nullable) ], non-null)), non-null)), non-null) ], non-null), keys_sorted=false)`
  - The whole cell (the map column itself) is nullable. No `arrow.json` metadata on these two columns. The `cityparquet:role = reserved` and `cityparquet:lod` field metadata stay exactly as today.
- Invariants a writer MUST honour (spec): per theme, `len(material) == WKB face count`; `len(texture) == WKB face count`, `len(texture[i]) == face i's ring count`, `len(texture[i][r].uv) == ring r's WKB point count − 1` (one pair per distinct vertex; the closing repeat has no pair); `id` and `uv` null together; map values non-null, maps never empty; a source theme with every entry null stays present as an all-null list; the NULL cell means no appearance at all; a whole-geometry `{"value": n}` is expanded to one entry per face; ids are the sidecar `id` (`i64`).
- **TDD**: every task writes its failing test first and runs it to see it fail for the expected reason.
- The gate is `cd lib/cityparquet-rs && just check` (clippy `-D warnings`, `cargo test --workspace`, isolation, vendor-check, `cargo fmt --check`, prettier over `**/*.md`). `just fixtures` must have been run once (network) — the integration tests read `tests/fixtures/lod3_railway.city.json` and `delft.city.jsonl`.
- Commit per task in the parent repository (`/data2/hideba/cityparquet`) on `develop`, conventional prefix `feat(cityparquet-rs)!:` / `refactor(cityparquet-rs):` / `test(cityparquet-rs):`, British English body, and the trailer `Claude-Session: https://claude.ai/code/session_018spx3RYGD6ZRYC8xaznCLf` as the last line. The pre-commit hook runs rustfmt and Prettier on staged files; never bypass it. Never stage `lib/duckdb-3d` (unrelated pending change) or anything under `lib/duckdb-cityjson`.
- Document the present: doc comments describe today's shape, never "used to be JSON". British English.
- `crates/schema` must not depend on `arrow-array`/`parquet` (the `isolation` recipe enforces it): the data-type functions live in `arrow-schema` terms only.
- Do not regenerate `lib/duckdb-cityjson/test/data/cityparquet_rs_minimal` in this plan — that is phase 3's first task, because the DuckDB tests that read it cannot pass until the extension reads MAP cells.

---

## File map

| File | Responsibility |
| --- | --- |
| `crates/schema/src/model.rs` | `material_data_type()`, `texture_data_type()`; the per-LoD quartet and the un-suffixed pair use them; schema tests |
| `crates/schema/src/sidecar_schemas.rs` | `geometry_templates_schema` uses the same two functions |
| `crates/core/src/appearance_columns.rs` (new) | `MaterialCell`, `TextureRing`, `TextureCell`; `MaterialCellBuilder`, `TextureCellBuilder`; `read_material_cell`, `read_texture_cell`; `to_flat_value()` conversions |
| `crates/core/src/appearance.rs` | `AppearanceInterner::flatten_material_map` / `flatten_texture_map` (CityJSON map → cell, via the `face_semantics` walk, drops applied, UVs inlined) |
| `crates/core/src/encode.rs` | `rewrite_geometry_appearance` returns cells; `GeometrySlot`/`GeometrySlotData` carry cells; the nested realign helpers for appearance go away |
| `crates/core/src/sidecar.rs` | `TemplateRow.material/texture: Option<MaterialCell>/Option<TextureCell>`; `TemplateSlot` uses the builders; `read_templates` uses the readers |
| `crates/core/src/package.rs` | template rows carry cells (no `to_value`) |
| `crates/core/src/export.rs` | `read_lod_keyed_appearance` reads MAP cells into the flat theme map; `nest_by_shells` re-nests before `LocalAppearance` localises |
| `crates/core/src/citygml/writer/appearance.rs` | `material_face_maps` / `texture_face_maps` consume the flat theme map; the nesting walks are deleted |
| `crates/core/src/compare.rs` | `canonical_material` / `canonical_texture` (flat, broadcast expanded, drops applied, resolved by value) replace the four `realigned_*` functions |
| `crates/core/tests/*.rs` | integration assertions read MAP cells |
| `documents/docs/06-resources/02-software.mdx` | the `cityparquet-rs` conformance row flips |

---

### Task 1: The Arrow types in the schema crate

**Files:**
- Modify: `crates/schema/src/model.rs` (add two public functions next to `geometry_properties_data_type`; replace the two `json_field` uses for material/texture in both branches of `to_arrow_schema`; update the tests `json_columns_carry_arrow_json_extension` and add `appearance_columns_are_typed_maps`)
- Modify: `crates/schema/src/sidecar_schemas.rs` (`geometry_templates_schema` uses the new functions)

**Interfaces:**
- Produces: `pub fn material_data_type() -> DataType` and `pub fn texture_data_type() -> DataType` in `cityparquet_schema::model`. Every later task builds arrays whose `data_type()` equals these.

- [ ] **Step 1: Write the failing schema test**

In `crates/schema/src/model.rs`'s `mod tests`, add:

```rust
#[test]
fn appearance_columns_are_typed_maps() {
    use arrow_schema::DataType;
    let schema = CityParquetSchema::new(vec![Lod::parse("1.0").unwrap(), Lod::parse("2.2").unwrap()], vec![]).unwrap();
    let arrow = schema.to_arrow_schema();
    for (name, expected) in [
        ("material_lod1_0", material_data_type()),
        ("texture_lod2_2", texture_data_type()),
    ] {
        let f = arrow.field_with_name(name).unwrap();
        assert_eq!(f.data_type(), &expected, "{name}");
        assert!(f.is_nullable(), "{name} cell is nullable");
        assert!(f.metadata().get("ARROW:extension:name").is_none(), "{name} carries no arrow.json tag");
        assert_eq!(f.metadata().get(ROLE_KEY).map(String::as_str), Some(ROLE_RESERVED));
    }
    // The MAP entries use Parquet's canonical names so any reader sees the standard groups.
    let DataType::Map(entries, false) = material_data_type() else { panic!("material is a Map") };
    assert_eq!(entries.name(), "key_value");
    let DataType::Struct(kv) = entries.data_type() else { panic!("entries are a Struct") };
    assert_eq!((kv[0].name().as_str(), kv[0].is_nullable()), ("key", false));
    assert_eq!((kv[1].name().as_str(), kv[1].is_nullable()), ("value", false));
    assert_eq!(kv[1].data_type(), &DataType::List(Arc::new(Field::new("item", DataType::Int64, true))));
}
```

Adjust the constructor call to whatever `CityParquetSchema`'s existing tests use to build a schema with LoDs (copy from `reserved_columns_in_spec_order`). Also change `json_columns_carry_arrow_json_extension` to assert only `other` (and `surfaces` inside `geometry_properties`, if it does) — remove `material_lod1_0` / `texture_lod2_2` from it.

- [ ] **Step 2: Run it and watch it fail**

Run: `cd lib/cityparquet-rs && cargo test -p cityparquet-schema appearance_columns_are_typed_maps`
Expected: compile error, `material_data_type` not found.

- [ ] **Step 3: Implement the two data-type functions and use them**

Add to `crates/schema/src/model.rs`, after `geometry_properties_data_type`:

```rust
/// Parquet's canonical MAP group names (`key_value` / `key` / `value`), so
/// the column reads as a standard map in every Parquet reader.
fn map_of(value: DataType) -> DataType {
    let key = Field::new("key", DataType::Utf8, false);
    let value = Field::new("value", value, false);
    let entries = Field::new("key_value", DataType::Struct(Fields::from(vec![key, value])), false);
    DataType::Map(Arc::new(entries), false)
}

/// The `material_lod*` Arrow type (spec "material / texture columns"):
///
/// ```text
/// MAP<VARCHAR, LIST<BIGINT>>   -- theme -> one sidecar id (or null) per WKB face
/// ```
///
/// Map values are non-null (a theme is present with a full-length list or
/// absent); list items are nullable (a face with no material in that theme).
pub fn material_data_type() -> DataType {
    map_of(DataType::List(Arc::new(Field::new("item", DataType::Int64, true))))
}

/// The `texture_lod*` Arrow type (spec "material / texture columns"):
///
/// ```text
/// MAP<VARCHAR, LIST<LIST<STRUCT<id BIGINT, uv LIST<LIST<DOUBLE>>>>>>
///   -- theme -> per WKB face -> per ring -> {sidecar id, one [u, v] per distinct vertex}
/// ```
///
/// Face entries and ring structs are non-null; `id` and `uv` are null
/// together for an untextured ring; every `[u, v]` and its two values are
/// non-null.
pub fn texture_data_type() -> DataType {
    let coord = Field::new("item", DataType::Float64, false);
    let pair = Field::new("item", DataType::List(Arc::new(coord)), false);
    let uv = Field::new("uv", DataType::List(Arc::new(pair)), true);
    let id = Field::new("id", DataType::Int64, true);
    let ring = Field::new("item", DataType::Struct(Fields::from(vec![id, uv])), false);
    let face = Field::new("item", DataType::List(Arc::new(ring)), false);
    map_of(DataType::List(Arc::new(face)))
}
```

In `to_arrow_schema`, replace both `json_field(...)` uses for `material`/`texture` (the `lods.is_empty()` branch and the per-LoD loop) with `Field::new(name, material_data_type(), true)` / `Field::new(name, texture_data_type(), true)` wrapped in the same `with_meta(...)` calls (keep `ROLE_KEY`/`LOD_KEY` exactly as they are). Write it as a small helper:

```rust
fn appearance_data_type(prefix: &str) -> DataType {
    match prefix {
        "material" => material_data_type(),
        "texture" => texture_data_type(),
        other => unreachable!("appearance prefix {other}"),
    }
}
```

In `crates/schema/src/sidecar_schemas.rs`, replace the two `json_col(&geometry_column_name("material"|"texture", lod))` lines with `Field::new(geometry_column_name("material", lod), material_data_type(), true)` and the texture equivalent, importing both from `crate::model`. If `json_col` is then unused, delete it. Update the module-level doc comments that call these columns JSON.

- [ ] **Step 4: Run the schema tests**

Run: `cargo test -p cityparquet-schema`
Expected: PASS (including `reserved_columns_in_spec_order`, which only checks names/order).

- [ ] **Step 5: Commit**

```bash
git add lib/cityparquet-rs/crates/schema
git commit -m "feat(cityparquet-rs)!: type the appearance columns as MAP

material_lod* is MAP<VARCHAR, LIST<BIGINT>> and texture_lod* is
MAP<VARCHAR, LIST<LIST<STRUCT<id BIGINT, uv LIST<LIST<DOUBLE>>>>>>,
with Parquet's canonical key_value/key/value names, in the object table
and the geometry-templates sidecar alike. The core crate follows.

Claude-Session: https://claude.ai/code/session_018spx3RYGD6ZRYC8xaznCLf"
```

(The core crate still compiles — it builds these columns with `StringBuilder` — but any test that writes a table now fails at runtime with an Arrow schema/array type mismatch until Task 3 lands. Run only `cargo test -p cityparquet-schema` here and say so in the report; Task 3's gate is the first green `--lib` run and Task 6's the first green `just check`.)

---

### Task 2: The shared cell module — types, builders, readers

**Files:**
- Create: `crates/core/src/appearance_columns.rs`
- Modify: `crates/core/src/lib.rs` (add `mod appearance_columns;` next to `mod geometry_properties;`)

**Interfaces:**
- Produces (all `pub(crate)`):

```rust
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct MaterialCell {
    /// theme -> one sidecar id (or None) per WKB face; insertion order kept.
    pub themes: Vec<(String, Vec<Option<i64>>)>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TextureRing {
    pub id: Option<i64>,
    /// One [u, v] per distinct ring vertex; None exactly when `id` is None.
    pub uv: Option<Vec<[f64; 2]>>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct TextureCell {
    /// theme -> per WKB face -> per ring.
    pub themes: Vec<(String, Vec<Vec<TextureRing>>)>,
}

pub(crate) struct MaterialCellBuilder { /* MapBuilder<StringBuilder, ListBuilder<Int64Builder>> */ }
impl MaterialCellBuilder {
    pub(crate) fn new() -> Self;
    pub(crate) fn append_value(&mut self, cell: &MaterialCell) -> Result<()>;   // error on an empty `themes`
    pub(crate) fn append_null(&mut self);
    pub(crate) fn finish(&mut self) -> ArrayRef;                                // a MapArray whose data_type() == material_data_type()
}
pub(crate) struct TextureCellBuilder { /* MapBuilder<StringBuilder, ListBuilder<ListBuilder<StructBuilder>>> */ }
impl TextureCellBuilder { /* same four methods, over TextureCell; error when a ring has id.is_some() != uv.is_some() or a pair count of 0 with Some */ }

pub(crate) fn read_material_cell(array: &MapArray, row: usize) -> Result<Option<MaterialCell>>;
pub(crate) fn read_texture_cell(array: &MapArray, row: usize) -> Result<Option<TextureCell>>;

impl MaterialCell {
    /// `{"<theme>": {"values": [id|null, …]}}` — the flat CityJSON-shaped map every reader re-nests from `shells`.
    pub(crate) fn to_flat_value(&self) -> Value;
}
impl TextureCell {
    /// `{"<theme>": {"values": [ [ [id, [u,v], …] | [null], … ], … ]}}` — per face, per ring, the inlined ring form.
    pub(crate) fn to_flat_value(&self) -> Value;
}
```

- [ ] **Step 1: Write the failing unit tests**

In the new file's `#[cfg(test)] mod tests`:

```rust
use super::*;
use cityparquet_schema::model::{material_data_type, texture_data_type};

fn ring(id: i64, uv: &[[f64; 2]]) -> TextureRing { TextureRing { id: Some(id), uv: Some(uv.to_vec()) } }
fn bare() -> TextureRing { TextureRing { id: None, uv: None } }

#[test]
fn material_builder_matches_schema_type_and_round_trips() {
    let mut b = MaterialCellBuilder::new();
    let cell = MaterialCell { themes: vec![("".into(), vec![Some(3), None, Some(3)]), ("night".into(), vec![None, None, None])] };
    b.append_value(&cell).unwrap();
    b.append_null();
    let arr = b.finish();
    assert_eq!(arr.data_type(), &material_data_type());
    let map = arr.as_any().downcast_ref::<MapArray>().unwrap();
    assert_eq!(read_material_cell(map, 0).unwrap(), Some(cell));
    assert_eq!(read_material_cell(map, 1).unwrap(), None);
}

#[test]
fn empty_material_map_is_refused() {
    let mut b = MaterialCellBuilder::new();
    assert!(b.append_value(&MaterialCell::default()).is_err());
}

#[test]
fn texture_builder_matches_schema_type_and_round_trips() {
    let mut b = TextureCellBuilder::new();
    let cell = TextureCell { themes: vec![("".into(), vec![
        vec![ring(7, &[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]]), bare()],   // face 0: textured exterior, bare hole
        vec![bare()],                                                   // face 1: untextured
    ])] };
    b.append_value(&cell).unwrap();
    b.append_null();
    let arr = b.finish();
    assert_eq!(arr.data_type(), &texture_data_type());
    let map = arr.as_any().downcast_ref::<MapArray>().unwrap();
    assert_eq!(read_texture_cell(map, 0).unwrap(), Some(cell));
    assert_eq!(read_texture_cell(map, 1).unwrap(), None);
}

#[test]
fn half_null_ring_is_refused() {
    let mut b = TextureCellBuilder::new();
    let cell = TextureCell { themes: vec![("".into(), vec![vec![TextureRing { id: Some(1), uv: None }]])] };
    assert!(b.append_value(&cell).is_err());
}

#[test]
fn flat_values_take_the_cityjson_shapes_readers_expect() {
    let m = MaterialCell { themes: vec![("".into(), vec![Some(3), None])] };
    assert_eq!(m.to_flat_value(), serde_json::json!({"": {"values": [3, null]}}));
    let t = TextureCell { themes: vec![("".into(), vec![vec![ring(7, &[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]]), bare()], vec![bare()]])] };
    assert_eq!(
        t.to_flat_value(),
        serde_json::json!({"": {"values": [ [ [7, [0.0, 0.0], [1.0, 0.0], [1.0, 1.0]], [null] ], [ [null] ] ]}})
    );
}
```

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo test -p cityparquet appearance_columns`
Expected: compile error, module not found.

- [ ] **Step 3: Implement the module**

Mirror `geometry_properties.rs`: derive every list/struct field from the schema crate's data type so the builder can never drift from it. Skeleton for the material builder (the texture builder follows the same pattern one level deeper):

```rust
//! The typed `material_lod*` / `texture_lod*` MAP columns (spec "material /
//! texture columns"): the Arrow-decoupled cell values, the builders the main
//! object table and the geometry-template sidecar both write with, and the
//! readers every consumer reads with — one physical shape, one implementation.
//!
//! A cell is flat per WKB face, in WKB face order, keyed by theme; a reader
//! that needs CityJSON's per-shell nesting re-nests from `shells`
//! (`crate::export::nest_by_shells`).

use std::sync::Arc;

use arrow_array::builder::{Float64Builder, Int64Builder, ListBuilder, MapBuilder, MapFieldNames, StringBuilder, StructBuilder};
use arrow_array::{Array, ArrayRef, Float64Array, Int64Array, ListArray, MapArray, StringArray, StructArray};
use arrow_schema::{DataType, Field};
use serde_json::Value;

use cityparquet_schema::model::{material_data_type, texture_data_type};
use cityparquet_schema::{CityParquetError, Result};

fn err(msg: impl Into<String>) -> CityParquetError { CityParquetError::Schema(msg.into()) }

/// The `key_value`/`key`/`value` names and the key/value fields of a map type.
fn map_parts(map: &DataType) -> (MapFieldNames, Arc<Field>, Arc<Field>) {
    let DataType::Map(entries, _) = map else { unreachable!("appearance columns are Map") };
    let DataType::Struct(kv) = entries.data_type() else { unreachable!("map entries are Struct") };
    let names = MapFieldNames { entry: entries.name().clone(), key: kv[0].name().clone(), value: kv[1].name().clone() };
    (names, Arc::clone(&kv[0]), Arc::clone(&kv[1]))
}

fn list_item(list: &DataType) -> Arc<Field> {
    let DataType::List(item) = list else { unreachable!("expected List") };
    Arc::clone(item)
}

pub(crate) struct MaterialCellBuilder {
    map: MapBuilder<StringBuilder, ListBuilder<Int64Builder>>,
}

impl MaterialCellBuilder {
    pub(crate) fn new() -> Self {
        let (names, key_f, value_f) = map_parts(&material_data_type());
        let ids = ListBuilder::new(Int64Builder::new()).with_field(list_item(value_f.data_type()));
        let map = MapBuilder::new(Some(names), StringBuilder::new(), ids)
            .with_keys_field(key_f)
            .with_values_field(value_f);
        Self { map }
    }

    pub(crate) fn append_value(&mut self, cell: &MaterialCell) -> Result<()> {
        if cell.themes.is_empty() {
            return Err(err("material cell has no themes: write a null cell instead of an empty map"));
        }
        for (theme, ids) in &cell.themes {
            self.map.keys().append_value(theme);
            let list = self.map.values();
            for id in ids {
                list.values().append_option(*id);
            }
            list.append(true);
        }
        self.map.append(true).map_err(|e| err(format!("material map: {e}")))
    }

    pub(crate) fn append_null(&mut self) {
        self.map.append(false).expect("keys and values stay aligned");
    }

    pub(crate) fn finish(&mut self) -> ArrayRef {
        Arc::new(self.map.finish())
    }
}
```

For the texture builder, build `StructBuilder::new(ring_fields, vec![Box::new(Int64Builder::new()), Box::new(uv_builder)])` where `uv_builder = ListBuilder::new(ListBuilder::new(Float64Builder::new()).with_field(coord_item)).with_field(pair_item)`; wrap it in `ListBuilder::new(ring_struct).with_field(ring_item)` (rings of a face) and again `ListBuilder::new(...).with_field(face_item)` (faces). Appending a ring: `id` via `field_builder::<Int64Builder>(0)`, `uv` via `field_builder::<ListBuilder<ListBuilder<Float64Builder>>>(1)` (append the two coordinates, `append(true)` per pair, then `append(true)`/`append_null()` for the ring's uv list), then `struct_builder.append(true)`. Refuse `id.is_some() != uv.is_some()`.

Readers: `read_material_cell` — if `array.is_null(row)` return `Ok(None)`; else `let entries = array.value(row)` (a `StructArray` of the row's entries), downcast column 0 to `StringArray` and column 1 to `ListArray`, and for each entry collect the `Int64Array` items with nulls into `Vec<Option<i64>>`. Error (never panic) on a null map value, a wrong child type, or an empty map. `read_texture_cell` walks face list → ring list → struct (`id` Int64, `uv` List<List<Float64>>), errors on a pair without exactly two values, on `id`/`uv` null-ness disagreeing, or on a null face/ring.

`to_flat_value`: material → `{theme: {"values": [Value::from(id) | Null …]}}`; texture → per face an array of rings, a ring being `[id, [u, v], …]` or `[null]` when `id` is `None`.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p cityparquet appearance_columns`
Expected: PASS (`cargo test -p cityparquet --lib appearance_columns`). Other tests that write a table still fail at runtime until Task 3; do not chase them here.

- [ ] **Step 5: Commit**

```bash
git add lib/cityparquet-rs/crates/core/src/appearance_columns.rs lib/cityparquet-rs/crates/core/src/lib.rs
git commit -m "feat(cityparquet-rs): shared builders and readers for the appearance MAP cells

MaterialCell and TextureCell are the Arrow-decoupled values; one builder
and one reader per column, derived from the schema crate's data types,
so the object table and the template sidecar can never diverge.

Claude-Session: https://claude.ai/code/session_018spx3RYGD6ZRYC8xaznCLf"
```

---

### Task 3: The encoder — flatten CityJSON appearance to cells

**Files:**
- Modify: `crates/core/src/appearance.rs` (new `flatten_material_map` / `flatten_texture_map` on `AppearanceInterner`; delete `rewrite_material_map`, `rewrite_material_tree`, `rewrite_texture_map`, `rewrite_texture_tree`, `rewrite_texture_ring`, `is_texture_ring`, and their tests; keep `rewrite_material_index`-style index resolution as a private helper `resolve_material_index(&mut self, v: &Value, local_defs, theme) -> Result<Option<i64>>` and `resolve_texture_index` likewise)
- Modify: `crates/core/src/encode.rs` (`rewrite_geometry_appearance` returns `(Option<MaterialCell>, Option<TextureCell>, GeometryProperties)`; `GeometrySlotData.material/texture` become the cell types; `GeometrySlot.material/texture` become the builders; delete `realign_appearance_themes`, `realign_nested_appearance_themes`, `realign_nested_values`, `remove_dropped_entries`, `drops_align_with_surface_arrays` if nothing else uses them; make `flatten_values`, `count_boundary_faces`, `values_nesting_depth` `pub(crate)` if they are not already; add `pub(crate) fn face_ring_vertex_counts(boundaries: &Value, depth: usize) -> Vec<Vec<Option<usize>>>` — per face in depth-first order, per ring, `Some(stored distinct vertex count)` or `None` for a ring the WKB writer drops (fewer than 3 source indices — `normalise_ring`'s rule), computed from `boundaries` the same way `count_boundary_faces` counts faces; a face's STORED ring count is the number of `Some` entries)
- Modify: `crates/core/src/package.rs` (`TemplateRow { material, texture, .. }` receive the cells directly)
- Modify: `crates/core/src/lod0.rs` (the three `GeometrySlotData { material: None, texture: None, .. }` literals follow the new field types — a compile error points at each)
- Modify: `crates/core/src/wkb_write.rs` (expose `pub(crate) fn distinct_ring_len(ring: &[usize]) -> Option<usize>` — `None` for a ring `normalise_ring` drops, otherwise the stored vertex count — and make `normalise_ring` use it so the two cannot disagree)
- Modify: `crates/core/src/sidecar.rs` (`TemplateRow.material: Option<MaterialCell>`, `.texture: Option<TextureCell>`; `TemplateSlot` uses `MaterialCellBuilder`/`TextureCellBuilder`; `write_templates` appends cells; `read_templates`' `TemplateCols.material/texture: &MapArray` read with the Task 2 readers; delete `push_opt_json`/`opt_json` if unused)

**Interfaces:**
- Consumes: Task 2's cell types, builders and readers.
- Produces:

```rust
impl AppearanceInterner {
    /// One geometry's CityJSON `material` map -> a flat cell. `boundaries`/`thetype`
    /// size the walk (the same one `face_semantics` uses); `dropped` are the
    /// writer-dropped flat face positions, removed AFTER flattening.
    pub fn flatten_material_map(&mut self, map: &Value, boundaries: &Value, thetype: &GeometryType,
                                dropped: &[usize], local_defs: &[Value]) -> Result<MaterialCell>;
    pub fn flatten_texture_map(&mut self, map: &Value, boundaries: &Value, thetype: &GeometryType,
                               dropped: &[usize], local_defs: &[Value], local_uvs: &[Vec<f64>]) -> Result<TextureCell>;
}
```

- [ ] **Step 1: Write the failing unit tests in `appearance.rs`**

Replace the deleted rewrite tests with these (keep the interning/dedupe tests untouched):

```rust
fn solid_two_shells() -> (Value, GeometryType) {
    // shell 0: 2 faces (one with a hole), shell 1: 1 face
    (serde_json::json!([ [ [[0,1,2,3],[4,5,6]], [[0,1,2]] ], [ [[7,8,9]] ] ]), GeometryType::Solid)
}

#[test]
fn material_values_flatten_per_wkb_face_and_expand_the_broadcast() {
    let (b, t) = solid_two_shells();
    let defs = vec![json!({"name": "a"}), json!({"name": "b"})];
    let mut i = AppearanceInterner::new();
    let cell = i.flatten_material_map(&json!({"": {"values": [[0, null], [1]]}, "night": {"value": 1}}), &b, &t, &[], &defs).unwrap();
    assert_eq!(cell.themes, vec![
        ("".to_string(), vec![Some(0), None, Some(1)]),
        ("night".to_string(), vec![Some(1), Some(1), Some(1)]),
    ]);
}

#[test]
fn material_null_shorthand_and_dropped_face_are_honoured() {
    let (b, t) = solid_two_shells();
    let defs = vec![json!({"name": "a"})];
    let mut i = AppearanceInterner::new();
    // whole first shell null; face 1 (the hole-bearing face's neighbour) dropped by the writer
    let cell = i.flatten_material_map(&json!({"": {"values": [null, [0]]}}), &b, &t, &[1], &defs).unwrap();
    assert_eq!(cell.themes, vec![("".to_string(), vec![None, Some(0)])]);
}

#[test]
fn an_all_null_theme_stays_present() {
    let (b, t) = solid_two_shells();
    let mut i = AppearanceInterner::new();
    let cell = i.flatten_material_map(&json!({"x": {"values": [[null, null], [null]]}}), &b, &t, &[], &[]).unwrap();
    assert_eq!(cell.themes, vec![("x".to_string(), vec![None, None, None])]);
}

#[test]
fn texture_rings_inline_uvs_per_distinct_vertex_and_keep_ring_count() {
    let (b, t) = solid_two_shells();
    let defs = vec![json!({"type": "PNG", "image": "a.png"})];
    let uvs = vec![vec![0.0, 0.0], vec![1.0, 0.0], vec![1.0, 1.0], vec![0.0, 1.0], vec![0.5, 0.5]];
    let mut i = AppearanceInterner::new();
    let map = json!({"": {"values": [ [ [[0, 0, 1, 2, 3], [null]], null ], [ [[0, 4, 4, 4]] ] ]}});
    let cell = i.flatten_texture_map(&map, &b, &t, &[], &defs, &uvs).unwrap();
    let faces = &cell.themes[0].1;
    assert_eq!(faces.len(), 3);
    assert_eq!(faces[0].len(), 2, "exterior + hole");
    assert_eq!(faces[0][0], TextureRing { id: Some(0), uv: Some(vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]) });
    assert_eq!(faces[0][1], TextureRing { id: None, uv: None });
    assert_eq!(faces[1], vec![TextureRing { id: None, uv: None }], "a null face expands to one bare ring per ring");
    assert_eq!(faces[2][0].uv.as_ref().unwrap().len(), 3);
}

#[test]
fn texture_uv_count_mismatch_is_an_error_naming_the_ring() {
    let (b, t) = solid_two_shells();
    let defs = vec![json!({"type": "PNG", "image": "a.png"})];
    let uvs = vec![vec![0.0, 0.0]; 5];
    let mut i = AppearanceInterner::new();
    let map = json!({"": {"values": [ [ [[0, 0, 1]], [null] ], [ [[null]] ] ]}});  // face 0 exterior has 4 vertices, 2 uvs
    let e = i.flatten_texture_map(&map, &b, &t, &[], &defs, &uvs).unwrap_err().to_string();
    assert!(e.contains("face 0") && e.contains("ring 0"), "{e}");
}

#[test]
fn a_degenerate_middle_hole_removes_its_texture_entry_not_the_last_one() {
    // face 0 has an exterior, a 2-index (dropped) hole, and a real hole: the stored face has
    // 2 rings and the texture list must lose entry 1, keeping the real hole's texture.
    let b = json!([ [[0, 1, 2, 3], [4, 5], [6, 7, 8]] ]);
    let t = GeometryType::MultiSurface;
    let defs = vec![json!({"type": "PNG", "image": "a.png"}), json!({"type": "PNG", "image": "b.png"})];
    let uvs = vec![vec![0.0, 0.0]; 9];
    let mut i = AppearanceInterner::new();
    let map = json!({"": {"values": [ [ [0, 0, 1, 2, 3], [1, 4, 5], [1, 6, 7, 8] ] ]}});
    let cell = i.flatten_texture_map(&map, &b, &t, &[], &defs, &uvs).unwrap();
    let rings = &cell.themes[0].1[0];
    assert_eq!(rings.len(), 2);
    assert_eq!(rings[0].id, Some(0));
    assert_eq!((rings[1].id, rings[1].uv.as_ref().map(Vec::len)), (Some(1), Some(3)), "the real hole keeps its own texture");
}

#[test]
fn texture_uv_list_drops_the_closing_repeat_a_source_ring_carried() {
    // boundary ring [0,1,2,0] is closed in the source; the writer strips the repeat, so the
    // stored ring has 3 distinct vertices and the 4th uv (for the repeat) is dropped.
    let b = json!([ [[0, 1, 2, 0]] ]);
    let t = GeometryType::MultiSurface;
    let defs = vec![json!({"type": "PNG", "image": "a.png"})];
    let uvs = vec![vec![0.0, 0.0], vec![1.0, 0.0], vec![1.0, 1.0], vec![0.0, 0.0]];
    let mut i = AppearanceInterner::new();
    let cell = i.flatten_texture_map(&json!({"": {"values": [ [[0, 0, 1, 2, 3]] ]}}), &b, &t, &[], &defs, &uvs).unwrap();
    assert_eq!(cell.themes[0].1[0][0].uv.as_ref().unwrap().len(), 3);
}
```

`face_ring_vertex_counts` must therefore report each ring's *stored* vertex count — the count after `crate::wkb_write::normalise_ring`'s rule (strip trailing repeats of the first index while more than 3 remain). Expose a pure `pub(crate) fn distinct_ring_len(ring: &[usize]) -> usize` next to `normalise_ring` in `wkb_write.rs` and use it from both, so the two can never disagree.

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo test -p cityparquet --lib appearance::`
Expected: compile errors, `flatten_material_map` not found.

- [ ] **Step 3: Implement**

`flatten_material_map`:
1. `depth = values_nesting_depth(thetype)`; `faces = count_boundary_faces(boundaries, depth)`.
2. For each `(theme, inner)` in the map (object required, error otherwise): if `inner` has `"values"`, `flatten_values(values, boundaries, depth, &mut flat)`, then `flat.resize(faces, Null)`; else if it has `"value"`, `flat = vec![value.clone(); faces]`; else error `material theme '{theme}' has neither 'values' nor 'value'`.
3. Remove the `dropped` positions (ascending flat face positions): `flat.into_iter().enumerate().filter(|(i, _)| !dropped.contains(i))`.
4. Map each entry through `resolve_material_index` (`Null → None`, number → `Some(intern_material(&local_defs[idx]) as i64)`, out of range → error or, with `tolerate_invalid_refs`, `None` + count).
5. Push `(theme, ids)`. Return the cell (an empty source map yields an empty `themes`; the caller writes a null cell for it — document that in `rewrite_geometry_appearance`).

`flatten_texture_map`: same walk at the face level (so each flattened entry is a face's ring array, or `Null`), then per face `i` (post-drop index into `face_ring_vertex_counts` after the same drop filter): `Null` → one bare ring per STORED ring; an array → walk the source rings in order against the face's `Vec<Option<usize>>`: a `None` ring (dropped by the writer) consumes its texture entry and emits nothing, a `Some` ring emits one entry (a missing source entry becomes a bare ring; surplus source entries beyond the ring list are ignored); a ring `[null]` → bare; a ring `[t, uv…]` → `id = resolve_texture_index(t)`, `uv` = the first `ring_vertex_count` UV indices resolved through `local_uvs` (error `"... face {i} ring {r}: {n} uv indices for {m} distinct vertices"` when fewer; a surplus beyond the count is the stripped closing repeat and is dropped); a dangling texture index under `tolerate_invalid_refs` → bare ring + count. Only the first two coordinates of a UV entry are used; fewer than two is an error.

`rewrite_geometry_appearance` (encode.rs): drop the realign branches; call the two flatten functions with `&geom.boundaries`, `&geom.thetype`, `dropped_surfaces`; return `Some(cell)` only when `cell.themes` is non-empty. Update `GeometrySlotData`, `GeometrySlot::new` (`MaterialCellBuilder::new()`), the append site (`slot.material.append_value(cell)?` / `append_null()`), and `finish_arrays` (`slot.material.finish()`). Update `package.rs` (`TemplateRow { material, texture, .. }` straight from the tuple) and `sidecar.rs` as listed in Files. The doc comment on `TemplateSlot` that says there is no shared appearance builder is replaced by one sentence naming `crate::appearance_columns`.

- [ ] **Step 4: Fix the existing encode tests**

`dropped_surface_realigns_semantics_material_and_texture`, `solid_single_shell_realigns_semantics_material_and_texture_when_face_dropped` and the multi-shell one (~lines 2077–2400 of `encode.rs`) currently parse JSON strings out of `StringArray`s. Rewrite their material/texture assertions to downcast the column to `MapArray`, call `read_material_cell` / `read_texture_cell`, and assert on `cell.themes` (same scenarios, same expected survivors). Keep the `EncodeStats` assertions.

- [ ] **Step 5: Run the core unit tests**

Run: `cargo test -p cityparquet --lib`
Expected: PASS. (Integration tests under `crates/core/tests` and `export.rs`/`compare.rs`/CityGML still read the old shape and fail to compile or fail — that is Task 4/5/6.)

- [ ] **Step 6: Commit**

```bash
git add lib/cityparquet-rs/crates/core
git commit -m "feat(cityparquet-rs)!: write appearance flat per WKB face

The interner flattens a geometry's material and texture maps with the
same walk that produces face_semantics, expands a whole-geometry value
per face, removes the writer-dropped faces, inlines one uv pair per
distinct ring vertex, and hands typed cells to the object table and the
template sidecar.

Claude-Session: https://claude.ai/code/session_018spx3RYGD6ZRYC8xaznCLf"
```

---

### Task 4: Export — read the cells, re-nest from `shells`, localise as before

**Files:**
- Modify: `crates/core/src/export.rs`
  - `read_lod_keyed_appearance(batch, columns, row, kind: AppearanceKind) -> Result<Option<Value>>` where `pub(crate) enum AppearanceKind { Material, Texture }`: downcast to `MapArray`, read the cell, insert `cell.to_flat_value()` under the LoD key.
  - New `pub(crate) fn nest_by_shells(flat_theme_map: &Value, props: Option<&Value>, gtype: &GeometryType) -> Result<Value>`: for each theme's `values` list, apply exactly the nesting `rebuild_semantics` applies to `face_semantics` (surface types: unchanged; `Solid`: one shell partition via `shell_faces` / `single_solid_shell` / `partition_face_semantics_by_solids`, or a single shell when `shells` is absent; `MultiSolid`/`CompositeSolid`: per solid, `shells` required). Factor the match out of `rebuild_semantics` into `fn nest_faces(flat: Vec<Value>, props: Option<&Value>, gtype) -> Result<Value>` and use it from both.
  - In the restore loop, before `localise_material_map` / `localise_texture_map`, pass the hit through `nest_by_shells(hit, props.as_ref(), &gtype)?`. `LocalAppearance` stays as it is (it already consumes `{theme: {values: nested}}` with `[id, [u,v], …]` / `[null]` rings). Delete `localise_material_map`'s `"value"` branch (no broadcast exists in a cell) and the doc comments that mention it.
  - `rebuild_templates` (around lines 1199–1221): a `TemplateRow`'s `material`/`texture` are now cells; convert with `to_flat_value()` and `nest_by_shells(.., row.geometry_properties.as_ref(), &gtype)` before the existing localisation, exactly as the object rows do.
- Modify: `crates/core/src/citygml/writer/mod.rs` (the two `read_lod_keyed_appearance` calls pass the kind; nothing else — Task 5 handles what the writer does with the flat map)

**Interfaces:**
- Consumes: Task 2's readers and `to_flat_value`.
- Produces: `nest_by_shells`, `AppearanceKind`.

- [ ] **Step 1: Write the failing tests**

In `export.rs`'s test module:

```rust
#[test]
fn nest_by_shells_mirrors_face_semantics_nesting() {
    let props = serde_json::json!({"type": "Solid", "shells": [[2, 1]]});
    let flat = serde_json::json!({"": {"values": [3, null, 4]}});
    let nested = nest_by_shells(&flat, Some(&props), &GeometryType::Solid).unwrap();
    assert_eq!(nested, serde_json::json!({"": {"values": [[3, null], [4]]}}));
    let flat_ms = serde_json::json!({"": {"values": [3, null]}});
    assert_eq!(nest_by_shells(&flat_ms, None, &GeometryType::MultiSurface).unwrap(), flat_ms);
}

#[test]
fn nest_by_shells_rejects_a_length_that_disagrees_with_shells() {
    let props = serde_json::json!({"type": "Solid", "shells": [[2, 1]]});
    let flat = serde_json::json!({"": {"values": [3, null]}});
    assert!(nest_by_shells(&flat, Some(&props), &GeometryType::Solid).is_err());
}
```

And in `crates/core/tests/export_real_data.rs`, the existing railway round-trip tests are the oracle: after this task they must pass unchanged where they compare exported CityJSON against the source (a test that asserted on the *stored* JSON cell shape moves to reading the MAP cell — see Task 6).

- [ ] **Step 2: Run and watch them fail**

Run: `cargo test -p cityparquet --lib export::`
Expected: compile error (`nest_by_shells` missing).

- [ ] **Step 3: Implement** as described in Files.

- [ ] **Step 4: Run the export tests**

Run: `cargo test -p cityparquet --lib export:: && cargo test -p cityparquet --test export_real_data`
Expected: PASS for the unit tests; the integration file compiles once its stored-cell assertions are updated (Task 6) — if it does not compile yet, run only `--lib` here and say so in the report.

- [ ] **Step 5: Commit**

```bash
git add lib/cityparquet-rs/crates/core/src/export.rs lib/cityparquet-rs/crates/core/src/citygml/writer/mod.rs
git commit -m "feat(cityparquet-rs): export re-nests the flat appearance cells from shells

The per-LoD MAP cells are read into a flat theme map and nested exactly
as face_semantics is before the feature-local re-interning runs.

Claude-Session: https://claude.ai/code/session_018spx3RYGD6ZRYC8xaznCLf"
```

---

### Task 5: The CityGML writer consumes the flat map directly

**Files:**
- Modify: `crates/core/src/citygml/writer/appearance.rs`: `material_face_maps(material_map, n_faces, materials)` reads each theme's flat `values` list (length must equal `n_faces`; ids checked against the table by id as today); delete `flatten_leaves` and the `"value"` branch. `texture_face_maps(texture_map, textures)` reads each theme's per-face ring arrays directly (each face an array of ring leaves, parsed by the existing `parse_ring_leaf`); delete `flatten_texture_faces` and `is_ring_leaf`. Doc comments describe the flat shape.
- Modify: `crates/core/src/citygml/writer/building.rs` only if a signature changed.

- [ ] **Step 1: Write the failing test**

In `appearance.rs`'s test module (add one if there is none):

```rust
#[test]
fn face_maps_read_the_flat_per_face_shape() {
    let mut materials = HashMap::new();
    materials.insert(3i64, serde_json::json!({"name": "m"}));
    let m = material_face_maps(&serde_json::json!({"": {"values": [3, null, 3]}}), 3, &materials).unwrap();
    assert_eq!(m[""], vec![Some(3), None, Some(3)]);
    assert!(material_face_maps(&serde_json::json!({"": {"values": [3]}}), 3, &materials).is_err());

    let mut textures = HashMap::new();
    textures.insert(7i64, serde_json::json!({"type": "PNG", "image": "a.png"}));
    let t = texture_face_maps(&serde_json::json!({"": {"values": [ [ [7, [0.0, 0.0], [1.0, 0.0], [1.0, 1.0]], [null] ], [ [null] ] ]}}), &textures).unwrap();
    assert_eq!(t[""].len(), 2);
    assert_eq!(t[""][0][0].as_ref().unwrap().0, 7);
    assert!(t[""][0][1].is_none() && t[""][1][0].is_none());
}
```

- [ ] **Step 2: Run and watch it fail** — `cargo test -p cityparquet --lib citygml::writer::appearance` — the nested walker accepts the flat input for materials, so the first assertion passes but the `values: [3]` length check and the texture face parse fail (or the test fails to compile if signatures differ). Note the actual failure in the report.

- [ ] **Step 3: Implement**, then run `cargo test -p cityparquet --lib citygml:: && cargo test -p cityparquet --test citygml_appearance` (the integration test's stored-cell assertions are updated in Task 6; if it does not compile yet, run `--lib` only and say so).

- [ ] **Step 4: Commit**

```bash
git add lib/cityparquet-rs/crates/core/src/citygml
git commit -m "refactor(cityparquet-rs): the CityGML writer reads flat appearance cells

Claude-Session: https://claude.ai/code/session_018spx3RYGD6ZRYC8xaznCLf"
```

---

### Task 6: The comparator canonicalises both sides to the flat form

**Files:**
- Modify: `crates/core/src/compare.rs`: replace `realigned_material`, `realigned_texture`, `realigned_nested_material`, `realigned_nested_texture` with

```rust
/// A source geometry's `material` map reduced to the stored form — flat per
/// face (the null shorthand and a whole-geometry `value` expanded), writer-
/// dropped positions removed, every index dereferenced to its definition —
/// so a source `value` broadcast compares EQUAL to the exporter's per-face
/// `values` (spec "Round trip": the second canonicalisation).
fn canonical_material(map: &Option<HashMap<String, cjseq::Material>>, boundaries: &Value, thetype: &GeometryType,
                      dropped: &[usize], defs: Option<&AppearanceDefs>) -> Result<Option<Value>>;
/// The texture counterpart: flat per face, each face its ring list, a null
/// face expanded to one `[null]` per ring, rings resolved by value.
fn canonical_texture(map: &Option<HashMap<String, cjseq::Texture>>, boundaries: &Value, thetype: &GeometryType,
                     dropped: &[usize], defs: Option<&AppearanceDefs>) -> Result<Option<Value>>;
```

  implemented with `crate::encode::{flatten_values, count_boundary_faces, values_nesting_depth, face_ring_vertex_counts}` (the comparator deliberately keeps its own ring normalisation for geometry; for appearance alignment it reuses the encoder's walk exactly as `canonical_semantics` already does). `canonical_texture` applies the same dropped-ring rule as the encoder: a source ring with fewer than 3 indices consumes its texture entry and emits nothing, so a source face with a degenerate middle hole compares equal to the exporter's two-ring face. Every `normalise_geometry` arm calls the two new functions with `&geom.boundaries`. Delete `remove_dropped_entries`, `realign_nested_values` from `compare.rs` if unused afterwards, and `resolve_material_map`'s `"value"` branch.

- [ ] **Step 1: Write the failing comparator test**

```rust
#[test]
fn a_source_value_broadcast_compares_equal_to_the_exporters_per_face_values() {
    // Two features identical except that one spells the material as {"value": 0}
    // and the other as {"values": [[0, 0, 0, 0, 0, 0]]} on a six-face Solid.
    // Build them the way `compare_detects_an_added_material_block_when_appearance_not_excluded`
    // builds its fixture; assert `compare_datasets` reports no difference.
}
```

Fill the body by copying that existing test's fixture construction; assert the report's difference count is 0.

- [ ] **Step 2: Run and watch it fail** — `cargo test -p cityparquet --lib compare::a_source_value_broadcast` — expected: a reported difference (the two maps differ as JSON today).

- [ ] **Step 3: Implement**; add a second unit test in which a source `MultiSurface` face has a 2-index middle hole with its own texture entry and the "exported" side has the two-ring face — they must compare equal.

- [ ] **Step 4: Run the comparator tests**

Run: `cargo test -p cityparquet --lib compare::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add lib/cityparquet-rs/crates/core/src/compare.rs
git commit -m "feat(cityparquet-rs): compare canonicalises appearance to the flat form

A source value broadcast and the exporter's per-face values reduce to the
same flat, resolved form, as the round-trip contract states.

Claude-Session: https://claude.ai/code/session_018spx3RYGD6ZRYC8xaznCLf"
```

---

### Task 7: The integration tests read cells; the first green `just check`

**Files:**
- Modify: `crates/core/src/appearance_columns.rs` and `crates/core/src/lib.rs`: make the module `pub mod appearance_columns;`, the two readers and the cell types `pub`, and add `pub fn material_cell_value(array: &MapArray, row: usize) -> Result<Option<Value>>` / `pub fn texture_cell_value(...)` (`read_*_cell(..)?.map(|c| c.to_flat_value())`) so integration tests can read a cell as JSON.
- Modify: `crates/core/tests/encode_real_data.rs`, `convert_real_data.rs`, `citygml_appearance.rs`, `export_real_data.rs`: every place that downcasts a `material_lod*`/`texture_lod*` column to `StringArray` and parses JSON now downcasts to `MapArray` and calls the helpers. Assertions change from nested `["visual"]["values"][0][…]` to the flat list; the mutated-feature test in `encode_real_data.rs` (around line 368) injects `{"visual": {"values": [[0,1,0,1,0,1]]}}` on a Solid — its expectation becomes the flat six ids; `convert_real_data.rs`'s content-shape test (lines ~206–251) asserts the flat `{"<theme>": {"values": [...]}}` shape with a length equal to the face count.

- [ ] **Step 1: Make the integration tests compile and run them to see which fail**

Run: `cd lib/cityparquet-rs && cargo test -p cityparquet --tests 2>&1 | tail -40`
Expected: failures only in the four files above, each at a stored-cell assertion. Record them in the report.

- [ ] **Step 2: Update the assertions** as described in Files.

- [ ] **Step 3: Run the whole gate**

Run: `cd lib/cityparquet-rs && just check`
Expected: PASS. Include the `test result:` summary lines in the report.

- [ ] **Step 4: Commit**

```bash
git add lib/cityparquet-rs/crates/core
git commit -m "test(cityparquet-rs): the integration tests read the appearance MAP cells

Claude-Session: https://claude.ai/code/session_018spx3RYGD6ZRYC8xaznCLf"
```

---

### Task 8: Sweep, docs, the conformance matrix, and the cross-reader check

**Files:**
- Read/modify: `lib/cityparquet-rs/README.md`, `lib/cityparquet-rs/CLAUDE.md` + `AGENTS.md` (byte-identical), crate-level doc comments (`crates/core/src/lib.rs`, `encode.rs`, `export.rs`, `sidecar.rs` module docs) — every sentence that calls the appearance columns JSON, or describes the nested shape, now describes the typed flat cells.
- Modify: `documents/docs/06-resources/02-software.mdx` — the `cityparquet-rs` "Appearance & templates" row: remove the sentence saying the writer emits JSON cells in CityJSON's nesting; the typed per-WKB-face `MAP` columns are done.
- Modify: `lib/cityparquet-rs/crates/cli` only if a help string mentions JSON appearance columns.

- [ ] **Step 1: Sweep**

```bash
cd lib/cityparquet-rs && grep -rn "arrow.json\|JSON map\|JSON columns\|nested.*values\|\"value\"" crates --include=*.rs | grep -iv "other\|surfaces\|test" | head -40
grep -rn "material\|texture" README.md CLAUDE.md docs 2>/dev/null | grep -i json
```

Read each hit in context; rewrite the ones that describe the old shape. `other` and `surfaces` stay JSON and their mentions stay.

- [ ] **Step 2: Prove a non-arrow reader sees a standard MAP**

Every Rust test round-trips through arrow-rs, which stores its own schema in the footer. Convert the railway fixture and read the result with the DuckDB binary the sibling extension built:

```bash
cd lib/cityparquet-rs && cargo run -q -p cityparquet-cli -- convert tests/fixtures/lod3_railway.city.json --output /tmp/claude-1020/-data2-hideba-cityparquet/60ef30a0-0572-40dd-8e68-d18a43dbd3c1/scratchpad/railway_pkg --overwrite
/data2/hideba/cityparquet/lib/duckdb-cityjson/build/release/duckdb -c "DESCRIBE SELECT material_lod3_0, texture_lod3_0 FROM '/tmp/claude-1020/-data2-hideba-cityparquet/60ef30a0-0572-40dd-8e68-d18a43dbd3c1/scratchpad/railway_pkg/building.parquet'"
/data2/hideba/cityparquet/lib/duckdb-cityjson/build/release/duckdb -c "SELECT map_keys(material_lod3_0), len(map_values(material_lod3_0)[1]) FROM '/tmp/claude-1020/-data2-hideba-cityparquet/60ef30a0-0572-40dd-8e68-d18a43dbd3c1/scratchpad/railway_pkg/building.parquet' WHERE material_lod3_0 IS NOT NULL LIMIT 3"
```

Expected column types: `MAP(VARCHAR, BIGINT[])` and `MAP(VARCHAR, STRUCT(id BIGINT, uv DOUBLE[][])[][])`. Anything else (a STRUCT where a MAP is expected, `entries`/`keys`/`values` names) means the map field naming or repetition is wrong — fix it in `crates/schema/src/model.rs` and report what you saw. If the object table has no `material_lod3_0` (the railway fixture's LoD differs), use `DESCRIBE SELECT * ...` and pick the populated pair. Paste both outputs into the report.

- [ ] **Step 3: Gates**

Run: `cd lib/cityparquet-rs && just check` and, from the repository root, `just docs-build`.
Expected: both pass.

- [ ] **Step 4: Commit**

```bash
git add lib/cityparquet-rs documents/docs/06-resources/02-software.mdx
git commit -m "docs(cityparquet-rs): describe the typed appearance columns

Claude-Session: https://claude.ai/code/session_018spx3RYGD6ZRYC8xaznCLf"
```

---

## Not in this plan

- Regenerating `lib/duckdb-cityjson/test/data/cityparquet_rs_minimal` (phase 3, Task 1 — it must be read by an extension that understands MAP cells).
- Any change under `lib/duckdb-cityjson`.
- Pushing; the parent submodule pointer.
