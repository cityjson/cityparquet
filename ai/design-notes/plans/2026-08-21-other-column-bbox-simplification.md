# `other` column, `bbox` and LoD0-provenance simplification — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Collapse `other_attributes` into a single `other` column with one reader rule, move `geographicalExtent` out of `other` and into `bbox` as a union, and stop writing the per-row LoD0 provenance marker that leaks into exported CityJSON.

**Architecture:** Three independent format changes landed in dependency order across three areas. `cityparquet-rs` is the reference implementation and changes first (encode → decode → compare); the normative spec in `documents/` is updated to match; `duckdb-cityjson`, the second implementation of the same encoding, follows. Each task ends with a runnable test and a commit.

**Tech Stack:** Rust (cargo workspace, arrow-rs/parquet), C++ (DuckDB extension, sqllogictest), MDX (Blume docs site).

**Spec:** `docs/superpowers/specs/2026-08-21-other-column-bbox-simplification-design.md`

## Global Constraints

- **Branch:** work on `develop` in each repo; commit and push in both the paper repo and the `duckdb-cityjson` submodule. No feature branches.
- **Never stage pre-existing dirty files.** `git add` only the paths a task names. The paper repo has unrelated modifications (`.obsidian/*`, `apm.yml`, deleted spec files) that must stay untouched.
- **TDD, strict red-green.** Write the failing test, run it, watch it fail for the expected reason, then implement. A test that never failed has not been shown to test anything.
- **Real fixtures only.** `cityparquet-rs/tests/fixtures/delft.city.jsonl` and `lod3_railway.city.json`. No inline artificial CityJSON.
- **British English** in all prose and spec text.
- **Breaking is allowed.** Existing CityParquet packages get rewritten; no back-compat shims, no migration notes in the spec.
- **Spec prose is format-level and reader-facing.** No implementation status, no measured numbers, no "formerly X" notes in `documents/`.
- **duckdb-cityjson build:** `just rebuild` before `make test` — `make test` does **not** rebuild the unittest binary and will report green on a stale one.

---

### Task 1: Remove the per-row LoD0 provenance marker (D3)

Smallest independent change, and it removes noise from every later round-trip test.

**Files:**
- Modify: `cityparquet-rs/crates/cityparquet/src/encode.rs:1029-1037` (doc comment), `:1051` (return type), `:1610-1650` (call site and `other` assembly)
- Modify: `cityparquet-rs/crates/cityparquet/tests/lod0_synthesis.rs:330-375`
- Modify: `cityparquet-rs/docs/design.md:176-182`, `cityparquet-rs/TESTING.md:505-520`

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: `synthesize_footprint` returns `Result<Option<(GeometrySlotData, [f64; 6])>>` — the trailing `String` source-column element is gone. Task 3 edits the same `push_object` region and must assume the two-element tuple.

- [ ] **Step 1: Replace the provenance test with one asserting the key is absent**

In `crates/cityparquet/tests/lod0_synthesis.rs`, replace the test at lines 330-375 (the one asserting `parsed.get("cityparquet:lod0_0_source")`) with:

```rust
/// A synthesised footprint records no per-row provenance: the `other` column
/// is for source members the format cannot map, and a marker written there
/// leaves as a foreign top-level member of the exported CityObject.
#[test]
fn synthesised_footprint_writes_no_provenance_marker() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("pkg");
    convert(
        &Source::from_path("tests/fixtures/delft.city.jsonl").unwrap(),
        &out,
        &ConvertOptions {
            generate_lod0: true,
            ..Default::default()
        },
    )
    .unwrap();

    let batches = read_table(&out.join("building.parquet"));
    let other = batches
        .iter()
        .flat_map(|b| {
            let col = b.column_by_name("other").unwrap();
            let col = col.as_any().downcast_ref::<StringArray>().unwrap();
            (0..col.len())
                .filter(|r| !col.is_null(*r))
                .map(|r| col.value(r).to_string())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();

    assert!(
        other.iter().all(|cell| !cell.contains("cityparquet:")),
        "no row may carry a cityparquet: provenance key, got: {other:?}"
    );
}
```

- [ ] **Step 2: Run it and watch it fail**

```bash
cd cityparquet-rs && cargo test -p cityparquet --test lod0_synthesis synthesised_footprint_writes_no_provenance_marker
```

Expected: FAIL — the assertion trips, because rows carry `{"cityparquet:lod0_0_source":"geometry_lod1_2"}`.

- [ ] **Step 3: Drop the source column from `synthesize_footprint`**

In `crates/cityparquet/src/encode.rs`, change the signature at line ~1051 and its return sites:

```rust
fn synthesize_footprint(
    co: &CityObject,
    pool: &VertexPool,
    opts: &crate::lod0::Lod0Options,
    encoding: GeometryEncoding,
) -> Result<Option<(GeometrySlotData, [f64; 6])>> {
```

Delete the `String` from every `Ok(Some((...)))` return in that function. In its doc comment (lines 1029-1037), delete the sentence beginning "plus the SOURCE column the footprint was derived from" and the clause "the caller records that as the row's `other.cityparquet:lod0_0_source` provenance".

- [ ] **Step 4: Drop the call-site threading**

In `push_object`, replace the synthesis block (lines ~1612-1626):

```rust
        // Synthesise an LoD0 footprint into the `geometry_lod0_0` slot when
        // enabled and the object has no source LoD0 (spec "LoD0 synthesis").
        // The footprint slot key is LoD0's column suffix (`lod0_0`), the same
        // key `accumulate_geometry` would use for a real LoD0.
        if let Some(opts) = &self.synthesize_lod0 {
            let key = Lod::parse("0")
                .expect("literal 0 is a valid LoD")
                .column_suffix();
            if !acc.slots.contains_key(&key)
                && let Some((data, bbox)) = synthesize_footprint(co, &pool, opts, self.encoding)?
            {
                union_bbox(&mut acc.own_bbox, bbox);
                acc.slots.insert(key, data);
                stats.synthesized_lod0_footprints += 1;
            }
        }
```

Then delete the `if let Some(source_column) = lod0_source_column { ... }` block that inserts into `unmapped` (lines ~1644-1649), leaving:

```rust
        let mut unmapped = unmapped_from_json(co_json);
        if unmapped.is_empty() {
            self.other.append_null();
        } else {
            self.other
                .append_value(serde_json::to_string(&Value::Object(unmapped))?);
        }
```

Change `let mut unmapped` to `let unmapped` — nothing mutates it any more.

- [ ] **Step 5: Run the test and the crate suite**

```bash
cd cityparquet-rs && cargo test -p cityparquet --test lod0_synthesis
cargo test -p cityparquet
```

Expected: PASS.

- [ ] **Step 6: Update the two docs that describe the marker**

In `cityparquet-rs/docs/design.md`, in the **LoD0 synthesis** paragraph, delete the sentence starting "`geometry_properties`'s struct shape has no field for provenance, so the row's `other` column instead gets a `cityparquet:lod0_0_source` key naming the source geometry column the footprint was derived from (e.g. `"geometry_lod2_2"`)."

In `cityparquet-rs/TESTING.md`, replace the "Gotcha — LoD0 synthesis breaks a naive round-trip" block's diff sample with the geometry-only difference:

```
> object NL.IMBAG.Pand.0503100000000010-0: geometry at lod Some("0.0")
>   present in B, missing in A
```

and delete the clause "the synthesised footprint even records its own provenance".

- [ ] **Step 7: Commit**

```bash
cd cityparquet-rs
git add crates/cityparquet/src/encode.rs crates/cityparquet/tests/lod0_synthesis.rs docs/design.md TESTING.md
git commit -m "fix(encode): stop writing per-row LoD0 synthesis provenance

The cityparquet:lod0_0_source key was written into every synthesised row's
other column and read by nothing. decode splices other's keys straight into
the rebuilt CityObject, so it left as a foreign top-level member of the
exported CityJSON. The dataset-level city.other synthesis flag is unaffected."
```

---

### Task 2: Fix `bbox` to union the whole subtree (D2, prerequisite)

A pre-existing bug, and the base D2's source-extent union sits on. §2 of the spec promises "a consumer pruning on a parent's `bbox` never misses geometry held by its descendants"; `resolve_bbox` breaks that promise by returning the object's own bbox whenever it has one. Verified on `delft.city.jsonl`: `NL.IMBAG.Pand.0503100000030621` (a `Building` with an LoD0 footprint and two solid `BuildingPart`s) gets `zmin == zmax == -0.44`, excluding parts that rise to 16.19 m. duckdb-cityjson already unions (`scan_function.cpp:203-211`).

**Files:**
- Modify: `cityparquet-rs/crates/cityparquet/src/encode.rs:179-222` (`descendant_bbox`, `resolve_bbox`)
- Test: `cityparquet-rs/crates/cityparquet/tests/bbox_subtree.rs` (create)

**Interfaces:**
- Consumes: `union_bbox(acc: &mut Option<[f64; 6]>, bbox: [f64; 6])` and `own_geometry_bbox(co: &CityObject, pool: &VertexPool) -> Result<Option<[f64; 6]>>`, both already in `encode.rs`.
- Produces: `resolve_bbox` keeps its signature but now returns the union of own **and** descendant geometry. Task 3 wraps this result.

- [ ] **Step 1: Write the failing test**

Create `cityparquet-rs/crates/cityparquet/tests/bbox_subtree.rs`:

```rust
//! `bbox` is the union over the object's whole subtree, not its own geometry
//! alone (spec "Object table schema" — "a consumer pruning on a parent's
//! `bbox` never misses geometry held by its descendants").

use arrow_array::{Array, StructArray, Float64Array, RecordBatch, StringArray};
use cityparquet::package::{ConvertOptions, convert};
use cityparquet::source::Source;

mod common;
use common::read_table;

/// A 3DBAG `Building` carries only a flat LoD0 footprint while its
/// `BuildingPart`s carry the solids. Its `bbox` must still span the parts'
/// full z-range, or a z-filtered range query prunes the building away.
#[test]
fn parent_bbox_spans_descendant_solids() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("pkg");
    convert(
        &Source::from_path("tests/fixtures/delft.city.jsonl").unwrap(),
        &out,
        &ConvertOptions {
            generate_lod0: false,
            ..Default::default()
        },
    )
    .unwrap();

    let batches = read_table(&out.join("building.parquet"));
    let (parent_zmax, child_zmax) = zmax_for_pair(
        &batches,
        "NL.IMBAG.Pand.0503100000030621",
        "NL.IMBAG.Pand.0503100000030621-0",
    );

    assert!(
        parent_zmax >= child_zmax,
        "parent bbox zmax {parent_zmax} must cover its part's {child_zmax}"
    );
}

/// `(zmax of parent_id, zmax of child_id)` from the `bbox` struct column.
fn zmax_for_pair(batches: &[RecordBatch], parent_id: &str, child_id: &str) -> (f64, f64) {
    let mut parent = None;
    let mut child = None;
    for batch in batches {
        let ids = batch
            .column_by_name("id")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        let bbox = batch
            .column_by_name("bbox")
            .unwrap()
            .as_any()
            .downcast_ref::<StructArray>()
            .unwrap();
        let zmax = bbox
            .column_by_name("zmax")
            .unwrap()
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        for row in 0..ids.len() {
            if bbox.is_null(row) {
                continue;
            }
            if ids.value(row) == parent_id {
                parent = Some(zmax.value(row));
            } else if ids.value(row) == child_id {
                child = Some(zmax.value(row));
            }
        }
    }
    (
        parent.expect("parent row present"),
        child.expect("child row present"),
    )
}
```

If `tests/common/mod.rs` has no `read_table` helper, add one that opens a Parquet file and collects every `RecordBatch` into a `Vec<RecordBatch>`.

- [ ] **Step 2: Run it and watch it fail**

```bash
cd cityparquet-rs && cargo test -p cityparquet --test bbox_subtree
```

Expected: FAIL — `parent bbox zmax -0.44 must cover its part's 16.19`.

- [ ] **Step 3: Union descendants into the parent's own bbox**

In `crates/cityparquet/src/encode.rs`, `descendant_bbox` currently recurses into a child only when that child has no geometry of its own, which stops the walk at the first geometry-bearing generation. Replace its body so it takes both:

```rust
/// Recursive descendant-bbox union: every descendant's own bbox, unioned
/// over the whole subtree (a child's own geometry does not stop the walk —
/// its own children may extend further), cycle guarded with a visited set.
fn descendant_bbox(
    co: &CityObject,
    feature: &CityJSONFeature,
    pool: &VertexPool,
    visited: &mut HashSet<String>,
) -> Result<Option<[f64; 6]>> {
    let mut acc = None;
    if let Some(children) = &co.children {
        for child_id in children {
            if !visited.insert(child_id.clone()) {
                continue;
            }
            let Some(child) = feature.city_objects.get(child_id) else {
                continue;
            };
            if let Some(bbox) = own_geometry_bbox(child, pool)? {
                union_bbox(&mut acc, bbox);
            }
            if let Some(bbox) = descendant_bbox(child, feature, pool, visited)? {
                union_bbox(&mut acc, bbox);
            }
        }
    }
    Ok(acc)
}
```

Then make `resolve_bbox` union rather than short-circuit:

```rust
/// `bbox` binding rule: the union of the object's own geometry bboxes and a
/// cycle-guarded recursive union over its whole descendant subtree (spec
/// "Spatial metadata"). `None` only when nothing in the subtree has
/// geometry. Unioned, not own-first: a `Building` carrying a flat LoD0
/// footprint over solid `BuildingPart`s would otherwise get a z-flat box
/// that prunes the building away from any query above ground.
fn resolve_bbox(
    own_bbox: Option<[f64; 6]>,
    id: &str,
    co: &CityObject,
    feature: &CityJSONFeature,
    pool: &VertexPool,
) -> Result<Option<[f64; 6]>> {
    let mut acc = own_bbox;
    let mut visited = HashSet::new();
    visited.insert(id.to_string());
    if let Some(bbox) = descendant_bbox(co, feature, pool, &mut visited)? {
        union_bbox(&mut acc, bbox);
    }
    Ok(acc)
}
```

- [ ] **Step 4: Run the test and the crate suite**

```bash
cd cityparquet-rs && cargo test -p cityparquet --test bbox_subtree
cargo test -p cityparquet
```

Expected: PASS. Any existing test asserting a z-flat parent bbox is asserting the bug — update it and note the change in the commit body.

- [ ] **Step 5: Commit**

```bash
cd cityparquet-rs
git add crates/cityparquet/src/encode.rs crates/cityparquet/tests/bbox_subtree.rs
git commit -m "fix(encode): union the whole subtree into a parent's bbox

resolve_bbox returned the object's own geometry bbox whenever it had one,
so a 3DBAG Building carrying a flat LoD0 footprint over solid BuildingParts
got a z-flat box that excluded its own parts -- a z-filtered range query
pruned the building away. The spec's object-table-schema section already
promises the opposite, and duckdb-cityjson already unions."
```

---

### Task 3: Union the source `geographicalExtent` into `bbox` (D2)

**Files:**
- Modify: `cityparquet-rs/crates/cityparquet/src/encode.rs:47-56` (`OTHER_RESERVED_MEMBERS`), `:1710` (`push_object` bbox site), plus a new helper
- Modify: `cityparquet-rs/crates/cityparquet/src/decode.rs:100-132` (drop the extent shape check)
- Test: `cityparquet-rs/crates/cityparquet/tests/bbox_subtree.rs` (extend)

**Interfaces:**
- Consumes: `union_bbox`, and `resolve_bbox`'s subtree-union behaviour from Task 2.
- Produces: `fn source_extent(co: &CityObject) -> Option<[f64; 6]>` in `encode.rs`. `OTHER_RESERVED_MEMBERS` becomes 8 entries including `geographicalExtent`; Task 5 deletes the constant's decode-guard role but keeps the encode strip set.

- [ ] **Step 1: Write the failing tests**

Append to `crates/cityparquet/tests/bbox_subtree.rs`:

```rust
/// A declared `geographicalExtent` may only ever widen `bbox`, never narrow
/// it. 3DBAG declares an extent that fails to contain its own geometry, so
/// the computed box must win on every bound it is larger on.
#[test]
fn declared_extent_never_narrows_bbox() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("pkg");
    convert(
        &Source::from_path("tests/fixtures/delft.city.jsonl").unwrap(),
        &out,
        &ConvertOptions {
            generate_lod0: false,
            ..Default::default()
        },
    )
    .unwrap();

    let batches = read_table(&out.join("building.parquet"));
    let (parent_zmax, child_zmax) = zmax_for_pair(
        &batches,
        "NL.IMBAG.Pand.0503100000030621",
        "NL.IMBAG.Pand.0503100000030621-0",
    );
    // The source declares an extent for this Building whose zmax is 16.191,
    // and its part reaches 16.19; the union must cover both.
    assert!(parent_zmax >= child_zmax);
    assert!(
        parent_zmax >= 16.19,
        "declared extent must be unioned in, got {parent_zmax}"
    );
}

/// `geographicalExtent` is carried by `bbox`, not by `other`.
#[test]
fn geographical_extent_does_not_ride_other() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("pkg");
    convert(
        &Source::from_path("tests/fixtures/delft.city.jsonl").unwrap(),
        &out,
        &ConvertOptions {
            generate_lod0: false,
            ..Default::default()
        },
    )
    .unwrap();

    let batches = read_table(&out.join("building.parquet"));
    for batch in &batches {
        let col = batch.column_by_name("other").unwrap();
        let col = col.as_any().downcast_ref::<StringArray>().unwrap();
        for row in 0..col.len() {
            assert!(
                col.is_null(row),
                "delft has no unmapped members, got: {}",
                col.value(row)
            );
        }
    }
}
```

- [ ] **Step 2: Run and watch both fail**

```bash
cd cityparquet-rs && cargo test -p cityparquet --test bbox_subtree
```

Expected: `geographical_extent_does_not_ride_other` FAILS with a cell like `{"geographicalExtent":[84593.24,...]}`. `declared_extent_never_narrows_bbox` may already pass from Task 2 — that is fine, it pins the union once Step 3 lands.

- [ ] **Step 3: Strip `geographicalExtent` from `other` and union it into `bbox`**

In `crates/cityparquet/src/encode.rs`, extend the constant to 8 entries and rewrite its doc comment:

```rust
/// Members carried by a dedicated column, stripped from the `other` payload
/// (§5.1). `children_roles` rides the flatten member but has its own column
/// (G5); `address` has its own reserved struct column; `geographicalExtent`
/// is carried by `bbox`, which unions it with the object's computed subtree
/// extent; the rest are cjseq's typed fields.
pub(crate) const OTHER_RESERVED_MEMBERS: [&str; 8] = [
    "type",
    "attributes",
    "geometry",
    "children",
    "parents",
    "children_roles",
    "address",
    "geographicalExtent",
];
```

Add the helper next to `union_bbox`:

```rust
/// The source's declared per-object `geographicalExtent` as a bbox, when it
/// is well-formed — exactly six finite numbers (CityJSON 2.0.1 §2). A
/// malformed extent is ignored rather than fatal: it is an optional,
/// derivable source member, and `bbox` is fully recoverable from geometry.
fn source_extent(co: &CityObject) -> Option<[f64; 6]> {
    let extent: [f64; 6] = co.geographical_extent.as_ref()?.as_slice().try_into().ok()?;
    extent.iter().all(|v| v.is_finite()).then_some(extent)
}
```

At the `push_object` bbox site (line ~1710), union it in:

```rust
        // `bbox` is the union of the object's stored geometry (whole subtree,
        // all LoDs) and the source's declared extent. A declared extent may
        // only widen the box: sources routinely declare one that does not
        // contain their own geometry, and a box narrower than the geometry
        // silently prunes the row out of spatial queries.
        let mut bbox = resolve_bbox(acc.own_bbox, id, co, feature, &pool)?;
        if let Some(extent) = source_extent(co) {
            union_bbox(&mut bbox, extent);
        }
        self.push_bbox(bbox);
```

- [ ] **Step 4: Drop the now-dead extent shape check in decode**

In `crates/cityparquet/src/decode.rs`, delete from `merge_other_members` the block:

```rust
        if key == "geographicalExtent" && !is_geographical_extent(&value) {
            return Err(err(format!(
                "object '{id}': 'other' geographicalExtent must be an array of exactly six \
                 numbers, got: {value}"
            )));
        }
```

and delete the now-unused `is_geographical_extent` function.

- [ ] **Step 5: Run tests**

```bash
cd cityparquet-rs && cargo test -p cityparquet --test bbox_subtree
cargo test -p cityparquet
```

Expected: PASS. `cargo clippy --all-targets -- -D warnings` must also be clean (the deleted helper leaves no dead code).

- [ ] **Step 6: Commit**

```bash
cd cityparquet-rs
git add crates/cityparquet/src/encode.rs crates/cityparquet/src/decode.rs crates/cityparquet/tests/bbox_subtree.rs
git commit -m "feat(encode): carry geographicalExtent in bbox, not other

The source's declared per-object extent is unioned into the computed
subtree bbox rather than stored verbatim in the other column. Measured on
the delft fixture, the declared extent fails to contain the object's own
geometry on 100% of sampled 3DBAG rows, by up to 36 m, so it may widen the
box but never narrow it."
```

---

### Task 4: Export `geographicalExtent` from `bbox`; exclude it in `compare` (D2)

**Files:**
- Modify: `cityparquet-rs/crates/cityparquet/src/decode.rs` (read the `bbox` column, set the member)
- Modify: `cityparquet-rs/crates/cityparquet/src/compare.rs:98-103` (`Exclusions`), `:1719-1723` (`other` diff)
- Test: `cityparquet-rs/crates/cityparquet/tests/roundtrip_extent.rs` (create)

**Interfaces:**
- Consumes: `bbox` written by Task 3; `bbox_data_type()` from `cityparquet_schema::model` (a `Struct` of six non-null `Float64` fields named `xmin, ymin, zmin, xmax, ymax, zmax`).
- Produces: exported CityObjects carry `geographicalExtent` on every row with a non-null `bbox`. `compare` no longer reports a `geographicalExtent` difference.

- [ ] **Step 1: Write the failing test**

Create `cityparquet-rs/crates/cityparquet/tests/roundtrip_extent.rs`:

```rust
//! `geographicalExtent` is reconstructed from `bbox` on export.

use cityparquet::export::{ExportOptions, export};
use cityparquet::package::{ConvertOptions, convert};
use cityparquet::source::Source;

#[test]
fn export_emits_geographical_extent_from_bbox() {
    let dir = tempfile::tempdir().unwrap();
    let pkg = dir.path().join("pkg");
    let out = dir.path().join("out.city.jsonl");
    convert(
        &Source::from_path("tests/fixtures/delft.city.jsonl").unwrap(),
        &pkg,
        &ConvertOptions {
            generate_lod0: false,
            ..Default::default()
        },
    )
    .unwrap();
    export(&pkg, &out, &ExportOptions::default()).unwrap();

    let text = std::fs::read_to_string(&out).unwrap();
    let mut checked = 0;
    for line in text.lines().skip(1) {
        let feature: serde_json::Value = serde_json::from_str(line).unwrap();
        for (id, obj) in feature["CityObjects"].as_object().unwrap() {
            let extent = &obj["geographicalExtent"];
            assert!(
                extent.is_array() && extent.as_array().unwrap().len() == 6,
                "object {id} must carry a six-number geographicalExtent, got {extent}"
            );
            checked += 1;
        }
        if checked > 50 {
            break;
        }
    }
    assert!(checked > 0, "exported at least one object");
}
```

- [ ] **Step 2: Run and watch it fail**

```bash
cd cityparquet-rs && cargo test -p cityparquet --test roundtrip_extent
```

Expected: FAIL — `geographicalExtent` is `null`; nothing sets it.

- [ ] **Step 3: Read `bbox` in decode and set the member**

In `crates/cityparquet/src/decode.rs`, alongside the other column look-ups (near line 548 where `other_attributes` is resolved), add:

```rust
    let bbox_array = optional_column(batch, "bbox", &cityparquet_schema::model::bbox_data_type());
    let bbox_col = downcast::<StructArray>(bbox_array.as_ref(), "bbox")?;
```

Then in the row loop, immediately before `let object: cjseq::CityObject = serde_json::from_value(...)`, insert:

```rust
        // `geographicalExtent` is derived from `bbox` on export: the format
        // stores one spatial extent per row, and `bbox` is it (spec
        // "Spatial metadata"). Emitted for every row that has a bbox.
        if let Some(extent) = bbox_extent(bbox_col, row) {
            json.insert(
                "geographicalExtent".to_string(),
                Value::Array(extent.iter().map(|v| json_number(*v)).collect()),
            );
        }
```

Add both helpers near `decode_address_column`:

```rust
/// The row's `bbox` as CityJSON's six-number `geographicalExtent`, in the
/// struct's field order (`xmin, ymin, zmin, xmax, ymax, zmax`). `None` when
/// the column is absent or the row's cell is null.
fn bbox_extent(col: &StructArray, row: usize) -> Option<[f64; 6]> {
    if col.is_null(row) {
        return None;
    }
    let mut out = [0.0f64; 6];
    for (i, name) in ["xmin", "ymin", "zmin", "xmax", "ymax", "zmax"]
        .into_iter()
        .enumerate()
    {
        let field = col.column_by_name(name)?;
        let field = field.as_any().downcast_ref::<Float64Array>()?;
        if field.is_null(row) {
            return None;
        }
        out[i] = field.value(row);
    }
    Some(out)
}

/// A finite `f64` as a JSON number; non-finite coordinates cannot appear in
/// a bbox written by this crate, so they degrade to `null` rather than
/// panicking on an unrepresentable value.
fn json_number(v: f64) -> Value {
    serde_json::Number::from_f64(v).map_or(Value::Null, Value::Number)
}
```

- [ ] **Step 4: Exclude the derived field in `compare`**

In `crates/cityparquet/src/compare.rs`, the source side's `other` is built by `unmapped_object_members`, which now strips `geographicalExtent` (Task 3), so both sides agree without further change. Confirm by reading the `ObjectData` construction at line ~1604 and add a note above the `other` diff at line ~1715:

```rust
    // `geographicalExtent` is not compared: it is derived from `bbox` on
    // export, so the exported value is the stored spatial extent rather than
    // a reproduction of the source member. Neither side's `other` carries it
    // (`unmapped_object_members` strips it).
```

Then append to the report's `excluded` vector, wherever exclusions are recorded for the run:

```rust
    report
        .excluded
        .push("geographicalExtent (derived from bbox)".to_string());
```

- [ ] **Step 5: Run tests**

```bash
cd cityparquet-rs && cargo test -p cityparquet --test roundtrip_extent
cargo test -p cityparquet
```

Expected: PASS.

- [ ] **Step 6: Verify the round trip end to end**

```bash
cd cityparquet-rs
rm -rf /tmp/cp-rt && ./target/debug/cityparquet convert tests/fixtures/delft.city.jsonl -o /tmp/cp-rt --no-lod0 \
  && ./target/debug/cityparquet export /tmp/cp-rt /tmp/cp-rt.city.jsonl \
  && ./target/debug/cityparquet compare tests/fixtures/delft.city.jsonl /tmp/cp-rt.city.jsonl
```

Expected: `equal (excluded: N)` and exit 0. If it reports a `geographicalExtent` difference, Step 4's exclusion is not wired into the path `compare` actually takes — fix before committing.

- [ ] **Step 7: Commit**

```bash
cd cityparquet-rs
git add crates/cityparquet/src/decode.rs crates/cityparquet/src/compare.rs crates/cityparquet/tests/roundtrip_extent.rs
git commit -m "feat(export): reconstruct geographicalExtent from bbox

bbox is the row's one spatial extent, so export derives the CityJSON member
from it rather than from a stored copy. compare treats the field as derived."
```

---

### Task 5: Collapse `other_attributes` into `other` (D1)

The schema, encoder, decoder, scanner, module-column list and comparator must change together — the build does not compile with the column half-removed.

**Files:**
- Modify: `cityparquet-rs/crates/cityparquet-schema/src/model.rs:221-222`, `:429-434`, `:528-529`, `:827-828`, `:942`, `:1055-1067`
- Modify: `cityparquet-rs/crates/cityparquet/src/encode.rs:93-97`, `:136-141`, `:1318-1326`, `:1392`, `:1654-1665`, `:1816`, `:2087`
- Modify: `cityparquet-rs/crates/cityparquet/src/decode.rs:90-132`, `:135-178`, `:534-555`, `:653-656`, `:890-925` (unit tests)
- Modify: `cityparquet-rs/crates/cityparquet/src/scan.rs:503-534`
- Modify: `cityparquet-rs/crates/cityparquet/src/package.rs:582`, `:621`

**Interfaces:**
- Consumes: `OTHER_RESERVED_MEMBERS` (8 entries) from Task 3 — it remains the **encode** strip set and stops being a decode guard.
- Produces: no `other_attributes` column anywhere. `merge_other_members(json: &mut Map<String, Value>, cell: Option<&str>, id: &str) -> Result<()>` now merges into `attributes`. `merge_other_attributes` is deleted.

- [ ] **Step 1: Write the failing test**

Create `cityparquet-rs/crates/cityparquet/tests/other_single_column.rs`:

```rust
//! `other` is the single escape hatch: one column, one reader rule — every
//! entry is restored into the object's `attributes` on export.

use cityparquet::export::{ExportOptions, export};
use cityparquet::package::{ConvertOptions, convert};
use cityparquet::source::Source;

/// A source attribute whose name collides with a reserved column survives a
/// full round trip, back inside `attributes`.
#[test]
fn colliding_attribute_round_trips_through_other() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("collide.city.jsonl");
    // Real Delft objects with one reserved-named attribute injected.
    let text = std::fs::read_to_string("tests/fixtures/delft.city.jsonl").unwrap();
    let mut lines = text.lines();
    let header = lines.next().unwrap().to_string();
    let mut feature: serde_json::Value = serde_json::from_str(lines.next().unwrap()).unwrap();
    for (_id, obj) in feature["CityObjects"].as_object_mut().unwrap() {
        obj["attributes"]["bbox"] = serde_json::json!("collides-with-reserved");
    }
    std::fs::write(&src, format!("{header}\n{feature}\n")).unwrap();

    let pkg = dir.path().join("pkg");
    let out = dir.path().join("out.city.jsonl");
    convert(
        &Source::from_path(&src).unwrap(),
        &pkg,
        &ConvertOptions {
            generate_lod0: false,
            ..Default::default()
        },
    )
    .unwrap();
    export(&pkg, &out, &ExportOptions::default()).unwrap();

    let exported = std::fs::read_to_string(&out).unwrap();
    let line = exported.lines().nth(1).expect("one feature line");
    let feature: serde_json::Value = serde_json::from_str(line).unwrap();
    for (id, obj) in feature["CityObjects"].as_object().unwrap() {
        assert_eq!(
            obj["attributes"]["bbox"], "collides-with-reserved",
            "object {id} must recover its diverted attribute"
        );
    }
}

/// The table carries no `other_attributes` column at all.
#[test]
fn no_other_attributes_column_is_written() {
    let dir = tempfile::tempdir().unwrap();
    let pkg = dir.path().join("pkg");
    convert(
        &Source::from_path("tests/fixtures/delft.city.jsonl").unwrap(),
        &pkg,
        &ConvertOptions {
            generate_lod0: false,
            ..Default::default()
        },
    )
    .unwrap();

    let file = std::fs::File::open(pkg.join("building.parquet")).unwrap();
    let reader = parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder::try_new(file)
        .unwrap();
    let names: Vec<&str> = reader
        .schema()
        .fields()
        .iter()
        .map(|f| f.name().as_str())
        .collect();
    assert!(
        !names.contains(&"other_attributes"),
        "other_attributes must be gone, schema: {names:?}"
    );
    assert!(names.contains(&"other"), "other must remain: {names:?}");
}
```

- [ ] **Step 2: Run and watch both fail**

```bash
cd cityparquet-rs && cargo test -p cityparquet --test other_single_column
```

Expected: `no_other_attributes_column_is_written` FAILS (the column is present). `colliding_attribute_round_trips_through_other` should PASS already — it pins behaviour that must survive the refactor.

- [ ] **Step 3: Remove the column from the schema**

In `crates/cityparquet-schema/src/model.rs`: delete `"other_attributes"` from the reserved-name array at :221-222, from the field list at :429-434 (the `json_field("other_attributes", true)` entry), and from the name lists at :528-529 and :827-828. At :942 replace `"other_attributes"` in the bad-name loop with another reserved name that still exists, e.g. `"template"`. Delete the `other_attributes_is_reserved_json_after_other` test at :1055-1067.

- [ ] **Step 4: Merge the encoder's two builders into one**

In `crates/cityparquet/src/encode.rs`: delete the `other_attributes: StringBuilder` field (:1323), its initialiser (:1392), and its `arrays.push` (:1816). Replace the `other` / `other_attributes` assembly block (:1637-1665) with:

```rust
        // `other`: the single escape hatch (§5.1). Two kinds of entry share
        // it — a source member with no dedicated column, and an attribute
        // whose name collides with a reserved column — and they are
        // deliberately not distinguished: the column's whole contract is
        // that a reader restores every entry into `attributes`. Null when
        // the object has neither.
        let mut unmapped = unmapped_from_json(co_json);
        let diverted = collect_diverted_attributes(co, &self.diverted_attributes);
        stats.diverted_attribute_values += diverted.as_ref().map_or(0, serde_json::Map::len);
        if let Some(diverted) = diverted {
            unmapped.extend(diverted);
        }
        if unmapped.is_empty() {
            self.other.append_null();
        } else {
            self.other
                .append_value(serde_json::to_string(&Value::Object(unmapped))?);
        }
```

Update the doc comments at :93-97 and :136-141 to say the values land in `other`, not `other_attributes`.

- [ ] **Step 5: Make `other` decode into `attributes`**

In `crates/cityparquet/src/decode.rs`, replace `merge_other_members` (:90-132) with the merge-into-attributes logic, and delete `merge_other_attributes` (:135-178) entirely:

```rust
/// Merge the `other` column into the object's `attributes` (§5.1). `other`
/// is the format's single escape hatch, and this is its whole reader
/// contract: every entry is restored as an attribute, keyed by its map key.
/// A reader never tries to infer why a writer put an entry there — a
/// CityParquet file may come from any writer, and a foreign writer's
/// encoding of an unmapped source member is outside this specification.
/// A `None`/absent cell contributes nothing. Errors on a non-object cell,
/// or an entry duplicating an attribute decoded from its own column — both
/// mean a corrupt or foreign file, and dropping either would mask it.
fn merge_other_members(json: &mut Map<String, Value>, cell: Option<&str>, id: &str) -> Result<()> {
    let Some(cell) = cell else {
        return Ok(());
    };
    let Value::Object(entries) = serde_json::from_str::<Value>(cell).map_err(|e| {
        err(format!(
            "object '{id}': 'other' column is not valid JSON: {e}"
        ))
    })?
    else {
        return Err(err(format!(
            "object '{id}': 'other' column must be a JSON object, got: {cell}"
        )));
    };
    if entries.is_empty() {
        return Ok(());
    }
    let attrs = json
        .entry("attributes")
        .or_insert_with(|| Value::Object(Map::new()));
    let Value::Object(attrs_map) = attrs else {
        return Err(err(format!(
            "object '{id}': 'attributes' is not a JSON object"
        )));
    };
    for (key, value) in entries {
        if attrs_map.contains_key(&key) {
            return Err(err(format!(
                "object '{id}': 'other' entry '{key}' duplicates a column attribute"
            )));
        }
        attrs_map.insert(key, value);
    }
    Ok(())
}
```

Delete the `other_attributes` column look-up (:548-550) and its call site (:654-656). Update the `optional_column` doc comment at :195-201 to drop the `other_attributes` example.

Rewrite the unit tests at :890-925 to exercise `merge_other_members` instead of `merge_other_attributes` — same assertions, same fixtures, new function name.

- [ ] **Step 6: Stop reserving the name in scan and package**

In `crates/cityparquet/src/scan.rs`, delete the `if name == "other_attributes" { return Err(...) }` guard (:524-528) and rewrite the comment block at :503-521 to describe diverting into `other`, noting that `other` itself is still reserved and rejected outright by `reserved_and_geometry_column_names`.

In `crates/cityparquet/src/package.rs`, delete `names.push("other_attributes".to_string());` (:621) and the mention in the doc comment (:582).

- [ ] **Step 7: Run the whole workspace suite**

```bash
cd cityparquet-rs
cargo test --workspace
cargo clippy --all-targets -- -D warnings
```

Expected: PASS. Fix every reference the compiler flags; the column name appears in doc comments across `decode.rs` and `package.rs` that the compiler will not catch — grep for stragglers:

```bash
grep -rn "other_attributes" crates/ --include="*.rs"
```

Expected: no output.

- [ ] **Step 8: Verify the round trip**

```bash
cd cityparquet-rs
rm -rf /tmp/cp-d1 && ./target/debug/cityparquet convert tests/fixtures/delft.city.jsonl -o /tmp/cp-d1 --no-lod0 \
  && ./target/debug/cityparquet export /tmp/cp-d1 /tmp/cp-d1.city.jsonl \
  && ./target/debug/cityparquet compare tests/fixtures/delft.city.jsonl /tmp/cp-d1.city.jsonl
rm -rf /tmp/cp-rail && ./target/debug/cityparquet convert tests/fixtures/lod3_railway.city.json -o /tmp/cp-rail --no-lod0 \
  && ./target/debug/cityparquet export /tmp/cp-rail /tmp/cp-rail.city.json \
  && ./target/debug/cityparquet compare tests/fixtures/lod3_railway.city.json /tmp/cp-rail.city.json
```

Expected: `equal` and exit 0 for both.

- [ ] **Step 9: Commit**

```bash
cd cityparquet-rs
git add crates/cityparquet-schema/src/model.rs crates/cityparquet/src/encode.rs \
        crates/cityparquet/src/decode.rs crates/cityparquet/src/scan.rs \
        crates/cityparquet/src/package.rs crates/cityparquet/tests/other_single_column.rs
git commit -m "feat!: collapse other_attributes into a single other column

other is now the format's one escape hatch, with one reader rule: every
entry is restored into the object's attributes on export. Writers put both
reserved-name-colliding attributes and unmapped source members there, and
readers never try to tell them apart -- a CityParquet file may come from any
writer. Breaking: packages written by earlier versions must be rewritten."
```

---

### Task 6: Update the normative specification

**Files:**
- Modify: `documents/docs/03-specification/02-object-table-schema.mdx:33-34`, `:77-79`, `:111-118`
- Modify: `documents/docs/03-specification/03-geometry-semantics.mdx:338-366`, `:379-394`
- Modify: `documents/docs/03-specification/07-mapping-cityjson.mdx:59`, `:70`

**Interfaces:**
- Consumes: the behaviour Tasks 1-5 implement. The spec is the normative description of it; the wording must match what the code now does.
- Produces: no code interface.

- [ ] **Step 1: Rewrite the reserved-column table rows**

In `02-object-table-schema.mdx`, delete the `other_attributes` row (:34) and replace the `other` row (:33) with:

```
| `other` | `JSON` |  | nullable | Source data with no column of its own — a member the format does not map, or an attribute whose name collides with a reserved column. A reader restores every entry into the object's `attributes`; see [the escape-hatch rule](#the-other-column) |
```

- [ ] **Step 2: Add the escape-hatch section**

In the same file, after the reserved-column table's notes, add:

```markdown
### The `other` column

`other` is the single escape hatch for source data no column carries, and its
reader contract is one rule: **every entry is restored into the object's
`attributes`**, keyed by its map key.

Two kinds of entry share the column — a source member the format does not map, and
an attribute whose name collides with a reserved column — and a reader **MUST NOT**
attempt to distinguish them. A CityParquet file may come from any writer, and how a
writer chose to encode an unmapped source member is outside this specification; a
reader that inferred intent from a key would be reading one writer's convention
rather than the format.

A source attribute named `other` is **invalid input**: it has nowhere to divert to,
since diverting it into the column named after it would be circular.
```

- [ ] **Step 3: Delete the single-exception paragraph and the reservation bullet**

In the "Optional data is `NULL`" paragraph (:77-79), delete the sentence beginning "`other_attributes` is the single exception" through "MUST tolerate a table that does not carry it." — `other` is always present and nullable like every other optional column.

In "Column naming and reservation rules", delete the whole `other_attributes` bullet (:111-118) and replace it with:

```markdown
- **`other` is itself a reserved name.** A source attribute literally named `other`
  is invalid input, exactly as one named `children` or `bbox` would be: it is the
  column collisions divert *into*, so it has nowhere further to go.
```

- [ ] **Step 4: Update the LoD0 and bbox sections**

In `03-geometry-semantics.mdx`, in "LoD0 synthesis (writer feature)", delete the entire "**Provenance is recorded per row**" bullet (:355-360), and in the "Synthesis provenance is distinct from degenerate-geometry diagnostics" bullet delete the parenthetical "(outside the per-row `other` provenance above, which is written only for rows that *did* receive a synthesised footprint)". Keep the sentence about the dataset-level `city.other` flag by folding it into the preceding bullet:

```markdown
- A dataset that performs any synthesis notes the fact once in `city.other` (e.g.
  `cityparquet:lod0_synthesis: true`), so a consumer can detect the possibility
  without scanning every row.
```

In "Spatial metadata", replace the first bullet (:383-385) with two:

```markdown
- `bbox` is the **union** over the object's whole subtree — its own geometry and
  every descendant's, not merely the nearest geometry-bearing generation. A
  `Building` carrying only an LoD0 footprint over solid `BuildingPart`s therefore
  still spans its parts' full z-range.
- Where the source declares a per-object extent of its own, that extent is
  **unioned into** `bbox`. It may only widen the box: a source extent is not
  guaranteed to contain the geometry it describes, and a box narrower than the
  geometry would prune the row out of queries that intersect it.
```

- [ ] **Step 5: Update the CityJSON mapping table**

In `07-mapping-cityjson.mdx`, replace the `geographicalExtent` row (:59):

```
| `CityObjects.{id}.geographicalExtent`     | `bbox` (unioned with the extent computed from the object's subtree geometry; reconstructed from `bbox` on export) |
```

and the trailing row (:70):

```
| other object members                      | the object-table `other` column, restored into `attributes` on export (JSON; null when the object has none) |
```

- [ ] **Step 6: Build the docs site and check the links resolve**

```bash
cd documents && npm run build
```

Expected: build succeeds with no broken-anchor warnings. The `#the-other-column` anchor added in Step 2 must resolve from the table row in Step 1.

- [ ] **Step 7: Commit**

```bash
git add documents/docs/03-specification/02-object-table-schema.mdx \
        documents/docs/03-specification/03-geometry-semantics.mdx \
        documents/docs/03-specification/07-mapping-cityjson.mdx
git commit -m "docs(spec): one other column, bbox carries geographicalExtent

other_attributes is removed and other becomes the single escape hatch with
one reader rule. geographicalExtent is unioned into bbox rather than stored
in other, and bbox is stated as a whole-subtree union. The per-row LoD0
synthesis provenance marker is gone."
```

---

### Task 7: Stop duplicating attributes into `other` in duckdb-cityjson

**Files:**
- Modify: `duckdb-cityjson/src/cityjson/city_object_utils.cpp:64-84`
- Modify: `duckdb-cityjson/src/include/cityjson/city_object_utils.hpp:20-42`
- Modify: `duckdb-cityjson/src/cityjson/column_types.cpp:501-506`
- Modify: `duckdb-cityjson/src/cityjson/scan_function.cpp:213`
- Test: `duckdb-cityjson/test/sql/cityjson_other_column.test` (create)

**Interfaces:**
- Consumes: nothing from earlier tasks (a separate repo and build).
- Produces: `CityObjectUtils::GetAttributeValue(const CityObject &obj, const Column &col, const std::set<std::string> &emitted_columns)` — a third parameter naming every column the bound schema actually emits. Task 8 calls the same signature.

- [ ] **Step 1: Write the failing test**

Create `duckdb-cityjson/test/sql/cityjson_other_column.test`:

```
# name: test/sql/cityjson_other_column.test
# description: other carries only data with no column of its own
# group: [cityjson]

require cityjson

# Every ordinary attribute has its own column, so nothing is left for other.
query I
SELECT count(*) FROM read_cityjsonseq('test/data/delft.city.jsonl') WHERE other IS NOT NULL;
----
0

# The same holds for the non-seq and FlatCityBuf readers.
query I
SELECT count(*) FROM read_cityjson('test/data/minimal.city.json') WHERE other IS NOT NULL;
----
0

# An attribute that has its own column is never also copied into other.
query I
SELECT count(*)
FROM read_cityjsonseq('test/data/delft.city.jsonl')
WHERE other IS NOT NULL AND json_extract(other, '$.b3_h_dak_50p') IS NOT NULL;
----
0
```

Adjust the attribute name in the third query to one the fixture actually has — check with:

```bash
cd duckdb-cityjson && ./build/release/duckdb -c \
  "SELECT * FROM read_cityjsonseq('test/data/delft.city.jsonl') LIMIT 1;"
```

- [ ] **Step 2: Run and watch it fail**

```bash
cd duckdb-cityjson && just rebuild && make test ARGS="test/sql/cityjson_other_column.test"
```

Expected: FAIL — every row has a non-null `other` full of duplicated attributes.

- [ ] **Step 3: Pass the emitted column set into `GetAttributeValue`**

In `src/include/cityjson/city_object_utils.hpp`, change the declaration and its doc comment:

```cpp
	/**
	 * Get attribute value from CityObject for a specific column.
	 *
	 * `emitted_columns` names every column the bound schema actually
	 * produces. `other` is assembled from the members that have none, so it
	 * needs the real column set rather than a static predicate: an attribute
	 * that lost a case-insensitive dedup race has no column of its own and
	 * must still survive in `other`.
	 */
	static json GetAttributeValue(const CityObject &obj, const Column &col,
	                              const std::set<std::string> &emitted_columns);
```

Add `#include <set>` to the header.

- [ ] **Step 4: Assemble `other` from unmapped members only**

In `src/cityjson/city_object_utils.cpp`, replace the `other` branch (:64-84):

```cpp
	if (col.name == "other") {
		// `other` carries only what no column of its own carries: an attribute
		// whose name collides with a reserved column, and so never got one.
		// Attributes that DID get a column are not repeated here -- a duplicate
		// costs a JSON blob per row and is written nowhere on COPY TO.
		// geographicalExtent is not included: it is carried by `bbox`, which
		// unions it with the extent computed from the object's geometry.
		json other_attrs = json::object();
		for (const auto &[key, value] : obj.attributes) {
			if (emitted_columns.count(key) == 0) {
				other_attrs[key] = value;
			}
		}
		if (other_attrs.empty()) {
			return json(nullptr);
		}
		return other_attrs;
	}
```

Remove the now-unused `IsPredefinedColumn`/`IsGeometryColumn` calls from this function.

- [ ] **Step 5: Build the emitted set at the call site**

In `src/cityjson/scan_function.cpp`, before the per-row loop that calls `GetAttributeValue` (around :180), build the set once per scan:

```cpp
	// Every column the bound schema emits, for `other`'s "what has no column"
	// test. Built once per scan rather than per row.
	std::set<std::string> emitted_columns;
	for (const auto &c : bind_data.columns) {
		emitted_columns.insert(c.name);
	}
```

and pass it at :213-214:

```cpp
			value = CityObjectUtils::GetAttributeValue(city_obj, col, emitted_columns);
```

- [ ] **Step 6: Un-reserve `other_attributes`**

In `src/cityjson/column_types.cpp`, delete `"other_attributes"` from the `reserved` vector in `IsReservedColumnName` (:504). The column no longer exists in the format, so a source attribute of that name is ordinary.

- [ ] **Step 7: Run the suite**

```bash
cd duckdb-cityjson && just rebuild && make test
```

Expected: PASS, including the new file. Tests asserting the old duplicating behaviour are asserting the bug — update them and say so in the commit body.

- [ ] **Step 8: Commit and push**

```bash
cd duckdb-cityjson
git add src/cityjson/city_object_utils.cpp src/include/cityjson/city_object_utils.hpp \
        src/cityjson/column_types.cpp src/cityjson/scan_function.cpp \
        test/sql/cityjson_other_column.test
git commit -m "fix(scan): stop duplicating attributes into the other column

other filtered with IsPredefinedColumn -- seven names -- so every ordinary
attribute that already had its own column was copied into other as well.
COPY TO reads only geographicalExtent back out, so the duplicates were
written nowhere. other now carries only members with no column of their own,
tested against the schema's real emitted column set so a case-collision
loser is still preserved."
git push origin develop
```

---

### Task 8: Follow the `bbox` and `other` semantics in duckdb-cityjson's writer

**Files:**
- Modify: `duckdb-cityjson/src/cityjson/scan_function.cpp:190-212` (bbox branch)
- Modify: `duckdb-cityjson/src/cityjson/copy_function.cpp:1243-1282`
- Modify: `duckdb-cityjson/src/cityjson/lod_table.cpp:114-122`, `src/include/cityjson/lod_table.hpp:66-82`
- Modify: `duckdb-cityjson/src/cityjson/fcb_selective_convert.cpp:335-355`, `src/cityjson/flatcitybuf_reader.cpp:360-370` (comments only)
- Test: `duckdb-cityjson/test/sql/cityjson_other_column.test` (extend)

**Interfaces:**
- Consumes: `GetAttributeValue(obj, col, emitted_columns)` from Task 7.
- Produces: no new interface; `COPY TO cityjson` output changes shape.

- [ ] **Step 1: Extend the test**

Append to `test/sql/cityjson_other_column.test`:

```
# bbox unions the source's declared extent, so it never sits inside the geometry.
query I
SELECT count(*)
FROM read_cityjsonseq('test/data/delft.city.jsonl')
WHERE bbox.zmax < bbox.zmin;
----
0

# COPY TO restores other's entries as attributes rather than dropping them.
statement ok
COPY (SELECT * FROM read_cityjsonseq('test/data/delft.city.jsonl'))
TO '__TEST_DIR__/other_roundtrip.city.jsonl' (FORMAT cityjsonseq);

query I
SELECT count(*) FROM read_cityjsonseq('__TEST_DIR__/other_roundtrip.city.jsonl');
----
2231
```

- [ ] **Step 2: Run and confirm the current state**

```bash
cd duckdb-cityjson && just rebuild && make test ARGS="test/sql/cityjson_other_column.test"
```

Note which assertions fail; the bbox one may already pass.

- [ ] **Step 3: Union the declared extent into `bbox`**

In `src/cityjson/scan_function.cpp`, in the `bbox` branch, union the object's declared extent into whichever extent was computed:

```cpp
		} else if (col.name == "bbox") {
			std::optional<GeographicalExtent> extent;
			if (vertex_pool == nullptr) {
				extent = std::nullopt;
			} else if (bind_data.target_lod.has_value()) {
				extent = target_geom != nullptr
				             ? CityObjectUtils::GetGeometryExtent(*target_geom, *vertex_pool,
				                                                  bind_data.metadata.transform)
				             : std::nullopt;
			} else {
				extent = CityObjectUtils::GetObjectExtent(city_obj_id, feature.city_objects,
				                                          *vertex_pool, bind_data.metadata.transform);
			}
			// The source's declared per-object extent is unioned in, never
			// substituted: a declared extent is not guaranteed to contain the
			// geometry it describes, and a box narrower than the geometry
			// prunes the row out of queries that intersect it.
			if (city_obj.geographical_extent.has_value()) {
				extent = extent.has_value()
				             ? extent->Union(city_obj.geographical_extent.value())
				             : city_obj.geographical_extent;
			}
			value = extent.has_value() ? extent->ToJson() : json(nullptr);
		}
```

If `GeographicalExtent` has no `Union` member, add one in its header next to `ToJson`:

```cpp
	/** This extent widened to also cover `other`. */
	GeographicalExtent Union(const GeographicalExtent &other) const {
		GeographicalExtent out = *this;
		for (size_t i = 0; i < 3; i++) {
			out.min[i] = std::min(out.min[i], other.min[i]);
			out.max[i] = std::max(out.max[i], other.max[i]);
		}
		return out;
	}
```

Match the struct's real field names — check the definition in `src/include/cityjson/types.hpp` before writing this.

- [ ] **Step 4: Restore `other` into `attributes` on COPY TO**

In `src/cityjson/copy_function.cpp`, in the attribute-assembly block (:1231-1243), after the `CopyColumnRole::Attribute` loop and before the `if (!attributes.empty())` check, merge `other`:

```cpp
		// `other` carries source data with no column of its own; the format's
		// reader rule is that every entry is restored as an attribute. Without
		// this, an attribute whose name collides with a reserved column is read
		// into `other` and then written nowhere.
		if (bind_data.other_col != DConstants::INVALID_INDEX) {
			auto other_val = input.data[bind_data.other_col].GetValue(row);
			if (!other_val.IsNull()) {
				try {
					json other_json = json_utils::ParseJson(other_val.ToString());
					if (other_json.is_object()) {
						for (auto it = other_json.begin(); it != other_json.end(); ++it) {
							if (!attributes.contains(it.key())) {
								attributes[it.key()] = it.value();
							}
						}
					}
				} catch (const CityJSONError &) {
					// Malformed `other` text is bad input, not this
					// reconstruction's problem to diagnose; the attributes it
					// would have contributed are simply absent.
				}
			}
		}
```

- [ ] **Step 5: Rebuild `geographicalExtent` from `bbox` alone**

In the same function, replace the whole `geographical_extent_out` block (:1245-1282) with:

```cpp
		// `geographicalExtent` is derived from `bbox`: the format stores one
		// spatial extent per row and `bbox` is it. `other` is no longer
		// consulted -- it carries attributes now, not the source extent.
		if (bind_data.bbox_col != DConstants::INVALID_INDEX) {
			auto bbox_val = input.data[bind_data.bbox_col].GetValue(row);
			if (!bbox_val.IsNull() && bbox_val.type().id() == LogicalTypeId::STRUCT) {
				auto &children = StructValue::GetChildren(bbox_val);
				if (children.size() >= 6 && !children[0].IsNull() && !children[1].IsNull() &&
				    !children[2].IsNull() && !children[3].IsNull() && !children[4].IsNull() &&
				    !children[5].IsNull()) {
					city_obj["geographicalExtent"] =
					    json::array({children[0].GetValue<double>(), children[1].GetValue<double>(),
					                 children[2].GetValue<double>(), children[3].GetValue<double>(),
					                 children[4].GetValue<double>(), children[5].GetValue<double>()});
				}
			}
		}
```

- [ ] **Step 6: Update the stale comments**

In `src/cityjson/lod_table.cpp` (:114-122) and `src/include/cityjson/lod_table.hpp` (:66-82), delete the sentences about `other_attributes` being deliberately excluded — the column no longer exists in the format, so there is nothing to exclude. In `src/cityjson/fcb_selective_convert.cpp` (:335-345) and `src/cityjson/flatcitybuf_reader.cpp` (:360-370), update the comments that describe `other` as "assembled from every attribute the object has" to say it carries only attributes with no column of their own.

- [ ] **Step 7: Run the full suite**

```bash
cd duckdb-cityjson && just rebuild && make test
```

Expected: PASS.

- [ ] **Step 8: Commit and push**

```bash
cd duckdb-cityjson
git add src/cityjson/scan_function.cpp src/cityjson/copy_function.cpp \
        src/cityjson/lod_table.cpp src/include/cityjson/lod_table.hpp \
        src/include/cityjson/types.hpp src/cityjson/fcb_selective_convert.cpp \
        src/cityjson/flatcitybuf_reader.cpp test/sql/cityjson_other_column.test
git commit -m "feat: bbox unions the declared extent; COPY TO restores other

bbox now unions the source's declared per-object extent rather than ignoring
it, and COPY TO restores other's entries into attributes -- previously an
attribute whose name collided with a reserved column was read into other and
then written nowhere. geographicalExtent is rebuilt from bbox alone."
git push origin develop
```

---

### Task 9: Cross-implementation verification and submodule pointer

**Files:**
- Modify: `cityparquet-rs/TESTING.md` (round-trip section, if the excluded count changed)
- Modify: the paper repo's `duckdb-cityjson` submodule pointer

**Interfaces:**
- Consumes: everything from Tasks 1-8.
- Produces: a verified claim that both implementations agree.

- [ ] **Step 1: Run the bundled interop script**

```bash
cd cityparquet-rs && just interop
```

Expected: `interop ok`. The railway fixture declares no CRS, so a CRS warning on stderr is expected, not a failure.

- [ ] **Step 2: Round-trip a cityparquet-rs package through duckdb-cityjson**

```bash
cd cityparquet-rs
rm -rf /tmp/cp-x && ./target/release/cityparquet convert tests/fixtures/delft.city.jsonl -o /tmp/cp-x --no-lod0
cd ../duckdb-cityjson
./build/release/duckdb -c "
  SELECT count(*) AS rows,
         count(other) AS non_null_other,
         min(bbox.zmin) AS zmin, max(bbox.zmax) AS zmax
  FROM read_parquet('/tmp/cp-x/building.parquet');
"
```

Expected: 2231 rows, `non_null_other` = 0, and a `zmax` well above ground (not a flat z-range). A non-zero `non_null_other` means Task 5's encoder is still writing something into `other`.

- [ ] **Step 3: Confirm no reference to the removed column survives**

```bash
cd /Users/hbbaba/tudelft/papers/citypaquet-paper
grep -rn "other_attributes" cityparquet-rs/crates documents/docs duckdb-cityjson/src || echo "clean"
grep -rn "lod0_0_source" cityparquet-rs/crates documents/docs || echo "clean"
```

Expected: `clean` for both.

- [ ] **Step 4: Update the submodule pointer and push the paper repo**

```bash
cd /Users/hbbaba/tudelft/papers/citypaquet-paper
git add duckdb-cityjson
git commit -m "chore: bump duckdb-cityjson to the other/bbox simplification"
git push origin develop
cd cityparquet-rs && git push origin develop
```

Note: `duckdb-cityjson` was already modified in the working tree before this work began. Check `git diff --submodule` first and only commit the pointer if it reflects the commits from Tasks 7-8.

- [ ] **Step 5: Refresh the memory entries**

Update `~/.claude/projects/-Users-hbbaba-tudelft-papers-citypaquet-paper/memory/other-column-bbox-simplification.md` to say **IMPLEMENTED** with the commit shas, and mark `g9-other-column-design.md` and `g12-collision-divert-design.md` as describing removed designs rather than merely superseded ones.

---

## Self-Review

**Spec coverage:**

| Spec item | Task |
|---|---|
| D1 — `other_attributes` removed, `other` is the single escape hatch | 5 (rs), 6 (spec), 7 (duckdb) |
| D1 — `other` always-present nullable, single-exception paragraph deleted | 5, 6 |
| D1 — decode guard inverts | 5 |
| D2 — `bbox` = subtree union ∪ declared extent | 2, 3 (rs), 8 (duckdb), 6 (spec) |
| D2 — export emits `geographicalExtent` from `bbox` | 4 (rs), 8 (duckdb) |
| D2 — `compare` excludes it as derived | 4 |
| D3 — per-row LoD0 provenance removed | 1 (rs), 6 (spec) |
| D4 — synthesis default unchanged | out of scope, stated in the design doc |
| T2 — undefined members normalise into `attributes` | 5 (the reader rule) |
| T3 — every exported object gains an extent | 4 |

Task 2 was added during planning: it is required by D2's "whole subtree" wording and turned out to be a live bug in `cityparquet-rs`, verified against `delft.city.jsonl`.

**Type consistency:** `synthesize_footprint` returns a two-element tuple after Task 1 and Task 3 assumes that. `GetAttributeValue` gains its `emitted_columns` parameter in Task 7 and Task 8 uses the same signature. `merge_other_members` keeps its signature throughout; `merge_other_attributes` is deleted in Task 5, and Task 4 does not reference it.

**Known unknowns the implementer must check rather than assume:**
- `tests/common/mod.rs` may not expose `read_table` (Task 2, Step 1).
- `GeographicalExtent` may not have a `Union` method or may name its fields differently (Task 8, Step 3).
- The attribute name in Task 7's third query must be one the Delft fixture actually carries.
- `compare`'s `excluded` vector is populated in a specific place that Task 4, Step 4 must locate rather than guess.
