# CityParquet: GeoArrow toggle + by-type default layout — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make CityParquet write plain-BLOB geometry (no GeoArrow/GeoParquet self-description) by default so DuckDB `SELECT *` and the `three_d` extension work with zero setup, while keeping GeoParquet interop available behind an opt-in `--geoarrow` flag; and make the CLI's default table layout one file per object type named `building.parquet` / `bridge.parquet` (dropping the `cityobjects_` prefix), with `--layout single` still producing `cityobjects.parquet`.

**Architecture:** Two orthogonal changes. (A) A `geoarrow: bool` write-time decoration threaded from `ConvertOptions` through the encoder and writer: it gates BOTH the field-level `geoarrow.wkb` Arrow extension type (in `cityparquet-schema::model`) AND the file-level GeoParquet `geo` key-value metadata (in `cityparquet::recipe`). Decode is unaffected because geometry columns are identified by NAME (`geometry_<lod>`), never by the tag. (B) The by-type file-naming rule loses its `cityobjects_` prefix; the CLI's `--layout` default flips to `by-type`. Round-trip is safe because the export path discovers object tables from `metadata.json`'s `manifest.tables`, which records whatever names convert wrote.

**Tech Stack:** Rust (workspace of 4 crates), Arrow/Parquet (`arrow-schema`, `parquet`), `geoarrow-schema`, `cjseq`, `clap`. Strict red-green TDD with real CityJSON fixtures (`tests/fixtures/delft.city.jsonl`, `tests/fixtures/lod3_railway.city.json`). Build/test via `cargo test` (or the repo `justfile`).

## Global Constraints

- **TDD, red-green:** every behavioural change starts with a failing test that uses real fixtures — NO inline artificial CityJSON (repo rule).
- **Round-trip must stay lossless:** the existing `roundtrip_real_data.rs` / `export_real_data.rs` suites must remain green after every task.
- **Writer/batch schema identity:** the Arrow schema handed to the Parquet writer (`package.rs:887`) and the schema the encoded batches carry (`encode.rs:1159`, `encode.rs:1201`) MUST be produced by the same tagged/untagged choice, or Arrow rejects the batches at write time (see the comment at `package.rs:884-886`).
- **Default polarity (decided):** GeoArrow/GeoParquet self-description is OFF by default (`--geoarrow` opts IN). CLI default layout is `by-type` (`--layout single` opts back to the single table).
- **Library `ConvertOptions::new` default stays `TableLayout::Single`** for API/test stability — only the CLI's `--layout` default flips. Only the CLI is the user-facing "default".
- British English in any prose/docs touched (repo rule).
- After the last task, run the Codex CLI external review (repo milestone convention).

---

### Task 1: Schema-level GeoArrow toggle (`cityparquet-schema::model`)

Make the geometry field's `geoarrow.wkb` extension type conditional. Keep the zero-arg `to_arrow_schema()` emitting the tag (all existing callers, tests, and the recipe's internal geometry-column detection depend on it); add a `_tagged(bool)` variant for the write path.

**Files:**
- Modify: `crates/cityparquet-schema/src/model.rs:146-158` (`geometry_field`), `:161` (`to_arrow_schema`)
- Test: `crates/cityparquet-schema/src/model.rs` (`#[cfg(test)] mod tests`, alongside `geometry_field_is_geoarrow_wkb` at ~:316)

**Interfaces:**
- Produces: `CityParquetSchema::to_arrow_schema_tagged(&self, geoarrow: bool) -> Result<Schema>` — renders the schema; geometry columns carry the `geoarrow.wkb` extension type iff `geoarrow`. `to_arrow_schema(&self) -> Result<Schema>` is retained and now equals `self.to_arrow_schema_tagged(true)`.
- Consumes: nothing new.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `model.rs`:

```rust
#[test]
fn geometry_field_tag_is_toggleable() {
    let schema = sample();

    // Tagged (default zero-arg and explicit true): geoarrow.wkb present.
    for tagged in [
        schema.to_arrow_schema().unwrap(),
        schema.to_arrow_schema_tagged(true).unwrap(),
    ] {
        let field = tagged.field_with_name("geometry_lod2").unwrap();
        assert_eq!(
            field.metadata().get("ARROW:extension:name").map(String::as_str),
            Some("geoarrow.wkb"),
            "tagged schema must advertise geoarrow.wkb"
        );
    }

    // Untagged: NO geoarrow extension, but the binary type and the
    // cityparquet role/lod metadata that decode relies on must survive.
    let untagged = schema.to_arrow_schema_tagged(false).unwrap();
    let field = untagged.field_with_name("geometry_lod2").unwrap();
    assert_eq!(field.data_type(), &arrow_schema::DataType::Binary);
    assert!(
        !field.metadata().contains_key("ARROW:extension:name"),
        "untagged geometry field must not advertise any Arrow extension type"
    );
    assert_eq!(
        field.metadata().get("cityparquet:role").map(String::as_str),
        Some("reserved"),
        "role metadata must survive so decode still classifies the column"
    );
    assert_eq!(
        field.metadata().get("cityparquet:lod").map(String::as_str),
        Some("2"),
        "lod metadata must survive"
    );
}
```

> Note: `sample()` already exists in the test module (used by the tests at `:265`/`:293`/`:317`); it builds a `CityParquetSchema` whose LoDs include `lod2` → column `geometry_lod2`. If `sample()`'s LoD set does not include `lod2`, use whatever `geometry_<suffix>` column it does produce (check the existing `geometry_field_is_geoarrow_wkb` test at ~:316 for the exact name it asserts on) and match that name here.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p cityparquet-schema geometry_field_tag_is_toggleable`
Expected: FAIL — `to_arrow_schema_tagged` does not exist (compile error).

- [ ] **Step 3: Write minimal implementation**

Change `geometry_field` (model.rs:146) to take the flag:

```rust
fn geometry_field(&self, name: &str, lod: Option<&Lod>, geoarrow: bool) -> Field {
    let mut field = Field::new(name, DataType::Binary, true);
    if geoarrow {
        let crs = match &self.crs {
            Some(projjson) => Crs::from_projjson(projjson.clone()),
            None => Crs::default(),
        };
        let wkb = WkbType::new(Arc::new(GeoMetadata::new(crs, None)));
        field = field.with_extension_type(wkb);
    }
    let mut field = reserved(field);
    if let Some(lod) = lod {
        field = with_meta(field, &[(LOD_KEY, &lod.to_string())]);
    }
    field
}
```

Rename the current `to_arrow_schema` body to `to_arrow_schema_tagged` and add the delegating wrapper. At model.rs:161 replace the signature and the two `self.geometry_field(...)` calls inside it (`:179` and `:187`):

```rust
/// Render the Arrow schema, columns in spec order. Geometry columns carry
/// the `geoarrow.wkb` extension type (and CRS) iff `geoarrow` — the write
/// path passes the caller's `--geoarrow` choice; every other caller wants
/// the self-describing (tagged) form.
pub fn to_arrow_schema_tagged(&self, geoarrow: bool) -> Result<Schema> {
    self.validate()?;

    let mut fields: Vec<Field> = vec![
        // ... unchanged reserved fields (id, feature_id, object_type,
        // parents, children, children_roles, bbox) ...
    ];

    if self.lods.is_empty() {
        fields.push(self.geometry_field("geometry", None, geoarrow));
        // ... unchanged geometry_properties push ...
    } else {
        for lod in &self.lods {
            let suffix = lod.column_suffix();
            fields.push(self.geometry_field(&format!("geometry_{suffix}"), Some(lod), geoarrow));
            // ... unchanged geometry_properties push ...
        }
    }
    // ... rest of the body unchanged (material, texture, template, other,
    // attributes) ...
    Ok(Schema::new(fields))
}

/// Tagged rendering — the self-describing GeoParquet/GeoArrow form every
/// non-write caller (reader schema rebuild, `column_lists`, recipe
/// geometry-column detection) expects.
pub fn to_arrow_schema(&self) -> Result<Schema> {
    self.to_arrow_schema_tagged(true)
}
```

> Only three lines of the moved body change: the signature, and the two `geometry_field(...)` calls now pass `geoarrow`. Everything between stays byte-for-byte identical.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p cityparquet-schema`
Expected: PASS — the new test passes, and the pre-existing `geometry_field_is_geoarrow_wkb` (which calls the zero-arg `to_arrow_schema()`) still passes.

- [ ] **Step 5: Commit**

```bash
git add crates/cityparquet-schema/src/model.rs
git commit -m "feat(schema): make geoarrow.wkb geometry tag toggleable via to_arrow_schema_tagged"
```

---

### Task 2: Gate the GeoParquet `geo` key in the recipe (`cityparquet::recipe`)

DuckDB's auto-decode trigger is the file-level `geo` key (`enable_geoparquet_conversion`), so the `--geoarrow` flag must also suppress it. `writer_properties` internally keeps using the tagged `to_arrow_schema()` to identify geometry columns for per-column statistics settings — that is correct and must NOT change.

**Files:**
- Modify: `crates/cityparquet/src/recipe.rs:159-174` (`writer_properties`)
- Test: `crates/cityparquet/src/recipe.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Produces: `WriterRecipe::writer_properties(&self, schema: &CityParquetSchema, metadata: &CityParquetMetadata, geoarrow: bool) -> Result<WriterProperties>` — the returned properties include the `geo` KV entry iff `geoarrow`.
- Consumes: `CityParquetSchema` (unchanged), `CityParquetMetadata::geoparquet_geo_value` (unchanged).

- [ ] **Step 1: Write the failing test**

Add to recipe.rs's test module (reuse whatever `CityParquetSchema` + `CityParquetMetadata` builders the existing recipe tests use; grep the module for an existing helper before writing a new one):

```rust
#[test]
fn geo_key_present_only_when_geoarrow_enabled() {
    let (schema, metadata) = sample_schema_and_metadata(); // existing test helper
    let recipe = WriterRecipe::default();

    let has_geo = |geoarrow: bool| {
        recipe
            .writer_properties(&schema, &metadata, geoarrow)
            .unwrap()
            .key_value_metadata()
            .map(|kvs| kvs.iter().any(|kv| kv.key == "geo"))
            .unwrap_or(false)
    };

    assert!(has_geo(true), "geoarrow=true must emit the GeoParquet `geo` key");
    assert!(!has_geo(false), "geoarrow=false must omit the `geo` key entirely");
}
```

> If no `sample_schema_and_metadata` helper exists, build the two values inline from the crate's real fixture via the same path the other recipe tests use (do NOT hand-author CityJSON). Check the top of the test module for the established pattern.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p cityparquet recipe::tests::geo_key_present_only_when_geoarrow_enabled`
Expected: FAIL — `writer_properties` takes 2 args, not 3 (compile error).

- [ ] **Step 3: Write minimal implementation**

In `writer_properties`, add the parameter and gate the `geo` push:

```rust
pub fn writer_properties(
    &self,
    schema: &CityParquetSchema,
    metadata: &CityParquetMetadata,
    geoarrow: bool,
) -> Result<WriterProperties> {
    let arrow_schema = schema.to_arrow_schema()?; // stays TAGGED: used only to
                                                  // detect geometry columns for
                                                  // per-column props below.

    let mut kvs: Vec<KeyValue> = metadata
        .to_key_values()?
        .into_iter()
        .map(|(key, value)| KeyValue::new(key, value))
        .collect();
    if geoarrow {
        kvs.push(KeyValue::new(
            "geo".to_string(),
            metadata.geoparquet_geo_value()?.to_string(),
        ));
    }
    // ... rest unchanged ...
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p cityparquet recipe`
Expected: PASS. (Callers won't compile yet — that's Task 3; run only the `recipe` module here, or expect the crate-wide build to fail at the single call site in `package.rs`, which Task 3 fixes.)

- [ ] **Step 5: Commit**

```bash
git add crates/cityparquet/src/recipe.rs
git commit -m "feat(recipe): gate GeoParquet geo metadata behind geoarrow flag"
```

---

### Task 3: Thread `geoarrow` through encode + `ConvertOptions` + `convert` (`cityparquet::encode`, `cityparquet::package`)

Add the option, thread it to the encoder and both write-path schema sites so writer and batches agree, and pass it to `writer_properties`.

**Files:**
- Modify: `crates/cityparquet/src/package.rs:101-130` (`ConvertOptions` + `new`), `:706`, `:709`, `:887`, `:889` (writer_properties call)
- Modify: `crates/cityparquet/src/encode.rs:1154-1159` (`encode`), `:1195-1201` (`encode_buffered`)
- Test: `crates/cityparquet/tests/convert_real_data.rs`

**Interfaces:**
- Produces: `ConvertOptions.geoarrow: bool` (field; `ConvertOptions::new` sets it `false`). `encode(source, scan, batch_size, geoarrow)` and `encode_buffered(features, header, scan, batch_size, geoarrow)` — both use `scan.schema.to_arrow_schema_tagged(geoarrow)` for their batch schema.
- Consumes: `CityParquetSchema::to_arrow_schema_tagged` (Task 1), `WriterRecipe::writer_properties(.., geoarrow)` (Task 2).

- [ ] **Step 1: Write the failing test**

Add to `convert_real_data.rs` (mirror the file's existing convert-then-open-parquet pattern; it already reads `cityobjects.parquet` and inspects the Arrow schema in several tests):

```rust
#[test]
fn default_convert_writes_plain_blob_geometry_no_geoarrow_no_geo_key() {
    let out = tempfile::tempdir().unwrap();
    let mut opts = ConvertOptions::new(delft_fixture(), out.path().to_path_buf());
    opts.layout = TableLayout::Single; // isolate this test from the layout change
    opts.overwrite = true;
    convert(&opts).unwrap();

    let file = std::fs::File::open(out.path().join("cityobjects.parquet")).unwrap();
    let reader = parquet::arrow::arrow_reader::ArrowReaderMetadata::load(
        &file,
        Default::default(),
    )
    .unwrap();

    // (a) No file-level GeoParquet `geo` key.
    let kv = reader.metadata().file_metadata().key_value_metadata().unwrap();
    assert!(
        !kv.iter().any(|k| k.key == "geo"),
        "default (no --geoarrow) output must not carry the GeoParquet `geo` key"
    );

    // (b) Geometry field is plain Binary with no geoarrow extension.
    let field = reader
        .schema()
        .fields()
        .iter()
        .find(|f| f.name().starts_with("geometry_"))
        .expect("a geometry_<lod> column exists");
    assert!(
        !field.metadata().contains_key("ARROW:extension:name"),
        "default output geometry column must not advertise geoarrow.wkb"
    );
}

#[test]
fn geoarrow_opt_in_restores_tag_and_geo_key() {
    let out = tempfile::tempdir().unwrap();
    let mut opts = ConvertOptions::new(delft_fixture(), out.path().to_path_buf());
    opts.layout = TableLayout::Single;
    opts.geoarrow = true;
    opts.overwrite = true;
    convert(&opts).unwrap();

    let file = std::fs::File::open(out.path().join("cityobjects.parquet")).unwrap();
    let reader = parquet::arrow::arrow_reader::ArrowReaderMetadata::load(
        &file,
        Default::default(),
    )
    .unwrap();
    let kv = reader.metadata().file_metadata().key_value_metadata().unwrap();
    assert!(kv.iter().any(|k| k.key == "geo"), "--geoarrow must write the `geo` key");
    let field = reader
        .schema()
        .fields()
        .iter()
        .find(|f| f.name().starts_with("geometry_"))
        .unwrap();
    assert_eq!(
        field.metadata().get("ARROW:extension:name").map(String::as_str),
        Some("geoarrow.wkb"),
        "--geoarrow must advertise geoarrow.wkb"
    );
}
```

> Use the crate's existing fixture accessor (grep `convert_real_data.rs` for how it currently locates `delft.city.jsonl`; there is already a helper — reuse it instead of `delft_fixture()` if the name differs). Reuse the exact parquet-reader import style already present in the file; the `ArrowReaderMetadata::load` snippet above matches `parquet` 58's API but adapt to whatever the file already uses.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p cityparquet --test convert_real_data default_convert_writes_plain_blob_geometry_no_geoarrow_no_geo_key`
Expected: FAIL — `opts.geoarrow` field does not exist (compile error), and pre-change output still carries `geo`.

- [ ] **Step 3: Write minimal implementation**

`ConvertOptions` (package.rs:101):

```rust
#[derive(Debug, Clone)]
pub struct ConvertOptions {
    pub input: PathBuf,
    pub output_dir: PathBuf,
    pub profile: Profile,
    pub overwrite: bool,
    pub batch_size: usize,
    pub recipe: WriterRecipe,
    pub ordering: RowOrder,
    pub layout: TableLayout,
    /// Write GeoParquet/GeoArrow self-description (the `geoarrow.wkb` field
    /// extension + the file-level `geo` key). OFF by default so DuckDB reads
    /// geometry columns as plain BLOB (works with `SELECT *` and the
    /// `three_d` extension's `ST_3DFromWKB(BLOB)` with zero setup); ON for
    /// GeoPandas/QGIS/GDAL interop.
    pub geoarrow: bool,
}
```

In `ConvertOptions::new` add `geoarrow: false,` to the struct literal (package.rs:127-ish).

`encode` (encode.rs:1154) and `encode_buffered` (encode.rs:1195): add `geoarrow: bool` as the last parameter and change the schema line in each:

```rust
let schema = Arc::new(scan.schema.to_arrow_schema_tagged(geoarrow)?);
```

`convert` call sites in package.rs:
- `:706` → `RowOrder::Source => encode(source, scan_result, opts.batch_size, opts.geoarrow)?,`
- `:709` → `encode_buffered(features, source.header(), scan_result, opts.batch_size, opts.geoarrow)?`
- `:887` → `let arrow_schema = Arc::new(scan_result.schema.to_arrow_schema_tagged(opts.geoarrow)?);`
- writer_properties call (`:889`) → `opts.recipe.writer_properties(&scan_result.schema, &metadata, opts.geoarrow)?;`

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p cityparquet`
Expected: PASS — the two new tests pass; the whole `cityparquet` suite (round-trip, export, decode, reader) stays green because decode is name-based and the manifest/metadata still record CRS via the `crs` key.

- [ ] **Step 5: Commit**

```bash
git add crates/cityparquet/src/package.rs crates/cityparquet/src/encode.rs crates/cityparquet/tests/convert_real_data.rs
git commit -m "feat(convert): default to plain-BLOB geometry; add ConvertOptions.geoarrow opt-in"
```

---

### Task 4: Rename by-type files + reserved-name collision guard (`cityparquet::package`)

Drop the `cityobjects_` prefix so by-type tables are `building.parquet`, `bridge.parquet`, `ext_foo.parquet`. Add a guard: dropping the prefix removes the namespace that made collisions with package sidecar files (`materials.parquet`, `textures.parquet`, `geometry_templates.parquet`, `metadata.json`) and the single-layout `cityobjects.parquet` impossible.

**Files:**
- Modify: `crates/cityparquet/src/package.rs:428-443` (`table_name_for_type`); update the doc comments at `:79-83`, `:424-427`, `:534`, `:769`, `:980`, and the unit tests at `:1061-1082`, `:1012`, `:1032`, and `:986` that assert the old `cityobjects_...` names.
- Test: `crates/cityparquet/src/package.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Produces: `table_name_for_type(object_type: &str) -> String` now returns `"{prefix}{snake}.parquet"` (e.g. `"building.parquet"`, `"ext_foo.parquet"`). A new `const RESERVED_PACKAGE_FILES: &[&str]` names the files a by-type table must never collide with.
- Consumes: nothing new.

- [ ] **Step 1: Write the failing test**

Update the existing naming test (`package.rs:1061-1082`) to expect the un-prefixed names, and add a collision-guard test:

```rust
#[test]
fn by_type_table_name_drops_cityobjects_prefix() {
    assert_eq!(table_name_for_type("Building"), "building.parquet");
    assert_eq!(table_name_for_type("BuildingPart"), "buildingpart.parquet");
    assert_eq!(table_name_for_type("+Foo"), "ext_foo.parquet");
    assert_eq!(
        table_name_for_type("+My Extension Type"),
        "ext_my_extension_type.parquet"
    );
}

#[test]
fn by_type_table_name_never_collides_with_reserved_package_files() {
    // No core object type snakes to a reserved package file name, but the
    // guard proves the invariant holds for the derived name.
    for reserved in RESERVED_PACKAGE_FILES {
        assert_ne!(
            table_name_for_type("Building"),
            *reserved,
            "a by-type object table must never shadow a package sidecar/metadata file"
        );
    }
}
```

Also update the two existing extension-name assertions that reference `cityobjects_ext_a.parquet` (`:1012`, `:1032`) to `ext_a.parquet`, and the `:986` comment/assertion accordingly.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p cityparquet package::tests::by_type_table_name_drops_cityobjects_prefix`
Expected: FAIL — still returns `cityobjects_building.parquet`; and `RESERVED_PACKAGE_FILES` is undefined (compile error).

- [ ] **Step 3: Write minimal implementation**

Add near the other package-file constants (package.rs:37-41):

```rust
/// Files a by-type object table's derived name must never collide with —
/// the single-layout table plus every package sidecar/metadata file. Since
/// `table_name_for_type` no longer namespaces object tables under a
/// `cityobjects_` prefix, this guard is the invariant that keeps a
/// pathological object type from shadowing a reserved file.
const RESERVED_PACKAGE_FILES: &[&str] = &[
    CITYOBJECTS_TABLE,      // "cityobjects.parquet"
    MATERIALS_TABLE,        // "materials.parquet"
    TEXTURES_TABLE,         // "textures.parquet"
    TEMPLATES_TABLE,        // "geometry_templates.parquet"
    "metadata.json",
];
```

Change `table_name_for_type` (package.rs:443):

```rust
format!("{prefix}{snake}.parquet")
```

Where the by-type writer decides the target file (the `TableLayout::ByType` path that calls `table_name_for_type` — grep `table_name_for_type(` in package.rs for the call site inside the writer bookkeeping, near `:534`/`by_type_table_index`), add the guard so a collision is a clear error rather than a silently-overwritten sidecar:

```rust
let name = table_name_for_type(object_type);
if RESERVED_PACKAGE_FILES.contains(&name.as_str()) {
    return Err(err(format!(
        "object type {object_type:?} maps to reserved package file {name:?}; \
         rename the type or use --layout single"
    )));
}
```

> Adapt `err(...)` / the return type to the enclosing function's signature (it already returns `Result<_>`). If the call site is not fallible, thread the check to the nearest fallible caller — the by-type writer open path is fallible.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p cityparquet package`
Expected: PASS. Then run the full crate to catch any other test asserting the old names: `cargo test -p cityparquet`.

- [ ] **Step 5: Commit**

```bash
git add crates/cityparquet/src/package.rs
git commit -m "feat(package): name by-type tables building.parquet (drop cityobjects_ prefix) + reserved-file guard"
```

---

### Task 5: CLI — `--geoarrow` flag + flip `--layout` default to by-type (`cityparquet-cli`)

**Files:**
- Modify: `crates/cityparquet-cli/src/main.rs:64-69` (`--layout` arg), `:141-152` (destructure), `:210-219` (`ConvertOptions` literal); add a `--geoarrow` arg near `:68`.
- Test: `crates/cityparquet-cli/tests/cli.rs`

**Interfaces:**
- Consumes: `ConvertOptions.geoarrow` (Task 3), `TableLayout` (Task 4 rename).
- Produces: CLI flags `--geoarrow` (bool, default false) and `--layout` (default `"by-type"`).

- [ ] **Step 1: Write the failing test**

The 4 existing `cli.rs` references to `cityobjects.parquet` run `convert` and then assert that file exists. After the default flips they must either pass `--layout single` or assert on the new by-type names. Add one new test that pins the new default, and update the existing ones to pass `--layout single` where they specifically want the single table:

```rust
#[test]
fn convert_defaults_to_by_type_layout_named_without_prefix() {
    let out = tempdir().unwrap();
    // run: cityparquet convert <delft fixture> <out>   (no --layout)
    run_cli(&["convert", delft_fixture_path(), out.path().to_str().unwrap()])
        .assert()
        .success();
    // delft contains Building objects → building.parquet, no cityobjects.parquet.
    assert!(out.path().join("building.parquet").exists(), "default layout is by-type");
    assert!(
        !out.path().join("cityobjects.parquet").exists(),
        "by-type default must not emit the single cityobjects.parquet"
    );
}

#[test]
fn convert_layout_single_still_emits_cityobjects_parquet() {
    let out = tempdir().unwrap();
    run_cli(&["convert", delft_fixture_path(), out.path().to_str().unwrap(), "--layout", "single"])
        .assert()
        .success();
    assert!(out.path().join("cityobjects.parquet").exists());
}
```

> Use the file's existing CLI-invocation helper (grep `cli.rs` for how it currently shells out — likely `assert_cmd`'s `Command::cargo_bin`). Match its style; the `run_cli` name above is illustrative. For the existing 4 tests that assert `cityobjects.parquet`, add `"--layout", "single"` to their arg lists.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p cityparquet-cli --test cli convert_defaults_to_by_type_layout_named_without_prefix`
Expected: FAIL — default is still `single`, so `building.parquet` is absent.

- [ ] **Step 3: Write minimal implementation**

Flip the layout default and update help (main.rs:64-69):

```rust
/// Table layout for the main CityObject data: "by-type" (default — one
/// file per object type, e.g. building.parquet / bridge.parquet) or
/// "single" (one cityobjects.parquet holding every type).
#[arg(long, default_value = "by-type")]
layout: String,
```

Add the geoarrow flag just after it:

```rust
/// Emit GeoParquet/GeoArrow self-description (the geoarrow.wkb field
/// extension + the file-level `geo` key). Off by default: default output
/// is plain-BLOB geometry that DuckDB `SELECT *` and the three_d
/// extension read directly. Pass this for GeoPandas/QGIS/GDAL interop.
#[arg(long, default_value_t = false)]
geoarrow: bool,
```

Add `geoarrow,` to the `Commands::Convert { .. }` destructure (main.rs:141-152) and to the `ConvertOptions { .. }` literal (main.rs:210-219):

```rust
let opts = ConvertOptions {
    input,
    output_dir: output,
    profile,
    overwrite,
    batch_size,
    recipe,
    ordering,
    layout,
    geoarrow,
};
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p cityparquet-cli`
Expected: PASS — new tests pass; updated existing tests (now pinned to `--layout single`) pass.

- [ ] **Step 5: Commit**

```bash
git add crates/cityparquet-cli/src/main.rs crates/cityparquet-cli/tests/cli.rs
git commit -m "feat(cli): --geoarrow opt-in; default --layout to by-type (building.parquet)"
```

---

### Task 6: Docs + manual DuckDB/three_d verification (`docs/`, `README.md`)

Update the format docs to state the new defaults and the escape hatch, and verify end-to-end against DuckDB + `three_d` on a real conversion.

**Files:**
- Modify: `docs/design.md` (geometry-encoding note ~:102 and the Solid caveat ~:263-265; the layout section ~:60-68), `docs/architecture.md:57` (the pipeline diagram's output line), `README.md` (add a "Querying in DuckDB / three_d" note).

**Interfaces:** none (docs only).

- [ ] **Step 1: Update `docs/design.md`**

Under the geometry-encoding section (~:102), replace/extend the geoarrow paragraph:

```markdown
Geometry columns hold **ISO WKB** (little-endian, Z types). By default the
columns are plain Parquet `BINARY` with **no** GeoArrow/GeoParquet
self-description, so a reader sees raw WKB bytes — DuckDB `SELECT *` and the
`three_d` extension (`ST_3DFromWKB(BLOB)`) work with zero configuration.
Pass `--geoarrow` to additionally tag geometry columns with the
`geoarrow.wkb` Arrow extension type and write the file-level GeoParquet
`geo` key, for GeoPandas / QGIS / GDAL interop. The CRS is recorded either
way in CityParquet's own `crs` metadata key.
```

Extend the Solid caveat (~:263-265) with the escape hatch:

```markdown
- `Solid` geometry is WKB `PolyhedralSurfaceZ` (type 1015). DuckDB's native
  `GEOMETRY` type and GeoPandas do not support that WKB type, so with
  `--geoarrow` output a solid column fails to auto-decode ("Unsupported
  geometry type in WKB"). Two ways to read solids: (a) the **default**
  output is plain BLOB, so `ST_3DFromWKB(geometry_lod2_2)` just works; or
  (b) for `--geoarrow` files, `SET enable_geoparquet_conversion = false;`
  first, which makes DuckDB read every geometry column as raw BLOB.
  `MultiSurface`-derived columns (`MultiPolygonZ`) auto-decode fine either way.
```

Update the layout section (~:60-68):

```markdown
The **default** layout is **by-type**: one table per object type, named
`building.parquet`, `bridge.parquet`, … (extension types get an `ext_`
prefix, e.g. `ext_foo.parquet`). Pass `--layout single` to instead write one
`cityobjects.parquet` holding every type.
```

- [ ] **Step 2: Update `docs/architecture.md:57` and `README.md`**

`architecture.md:57` — change the pipeline output line to reflect the by-type default:

```
                └─ write ──────► building.parquet, … (or cityobjects.parquet
                                 with --layout single) + sidecars + metadata.json
```

`README.md` — add a short subsection (near the CLI options table):

````markdown
### Querying in DuckDB / three_d

Default output is plain-BLOB WKB, so it works directly:

```sql
LOAD three_d;
SELECT id, ST_3DVolume(ST_3DFromWKB(geometry_lod2_2)) AS volume
FROM 'building.parquet'
WHERE geometry_lod2_2 IS NOT NULL;
```

If the package was written with `--geoarrow`, first tell DuckDB not to
auto-decode geometry (it cannot parse the `PolyhedralSurfaceZ` WKB that
Solids use): `SET enable_geoparquet_conversion = false;`.
````

- [ ] **Step 3: Manual end-to-end verification**

Run (adjust the fixture path/threedbag tile as available):

```bash
cargo run -p cityparquet-cli -- convert tests/fixtures/delft.city.jsonl /tmp/cp_out --overwrite
# default (by-type, plain BLOB): three_d works with no SET
duckdb -unsigned -c "LOAD three_d;
  SELECT id, ST_3DIsClosed(ST_3DFromWKB(geometry_lod2_2)) AS closed
  FROM '/tmp/cp_out/building.parquet'
  WHERE geometry_lod2_2 IS NOT NULL LIMIT 3;"
# plain SELECT * no longer errors (geometry reads as BLOB)
duckdb -c "SELECT count(*) FROM '/tmp/cp_out/building.parquet';"
```

Expected: the `three_d` query returns rows with `closed` booleans; `SELECT count(*)` succeeds with no "Unsupported geometry type in WKB".

- [ ] **Step 4: Commit**

```bash
git add docs/design.md docs/architecture.md README.md
git commit -m "docs: default plain-BLOB geometry + by-type layout; DuckDB/three_d query notes"
```

---

## Post-plan: milestone review & benchmark note

- [ ] **Benchmark harness check (not a code change unless it fails):** the `cityparquet-readbench` crate and any `just interop` / benchmark path that converts via the **library** `ConvertOptions::new` keep `TableLayout::Single` (unchanged default) and the tagged/`geo` behaviour is irrelevant to them (decode is name-based). Confirm `cargo test -p cityparquet-readbench` is green. If any bench path shells out to the CLI and expects `cityobjects.parquet`, pin it with `--layout single`.
- [ ] **Full workspace test:** `cargo test` (all crates) green.
- [ ] **Codex external review** of the milestone diff (repo convention), then bump the `cityparquet-rs` version per the milestone tagging scheme.

## Self-Review notes

- **Spec coverage:** Change A (geoarrow toggle, default off) → Tasks 1-3, 5; Change B (by-type default + rename) → Tasks 4-5; docs/UX → Task 6. The user's four original questions: #1/#3 (DuckDB/three_d errors) are resolved by the default-off behaviour (Task 3) + docs (Task 6); #2 (WKB vs GeoArrow) is documented in Task 6; #4 (per-type files) is Tasks 4-5.
- **Type consistency:** `to_arrow_schema_tagged(bool)` (Task 1) is consumed with the same signature in Tasks 2-3; `writer_properties(.., geoarrow)` (Task 2) is called with `opts.geoarrow` in Task 3; `ConvertOptions.geoarrow` (Task 3) is set by the CLI `geoarrow` flag (Task 5); `table_name_for_type` returns un-prefixed names (Task 4) which the CLI test asserts (Task 5).
- **Decode safety:** verified `decode::geometry_columns` (decode.rs:90) and the export path (`manifest.tables`, export.rs:963-1081) are name/manifest-driven, never tag-driven — so dropping the tag and renaming files cannot break round-trip.
