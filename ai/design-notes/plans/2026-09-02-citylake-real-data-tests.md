# Real-Data Server Tests Implementation Plan (piece A)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prove the CityParquet operations hold at national scale by running them against the published Delft datasets, rather than only against the three-object fixtures the suite has today.

**Architecture:** One new integration suite, `lib/citylake/tests/real_data.rs`, marked `#[ignore]` so it is reported as ignored by a default `cargo test` and run deliberately by its own recipe. Nothing is downloaded and nothing is committed: the cityjson extension auto-loads `httpfs` and resolves an `https://` source as readily as a local path, so the tests pass the URL straight to `create_dataset` and exercise the remote read path for free.

**Tech Stack:** Rust 2021, duckdb-rs `=1.10504.0` (DuckDB v1.5.4), the `cityjson` DuckDB extension v0.4.0, ducklake.

**Spec:** `ai/design-notes/specs/2026-09-02-citylake-real-data-and-ui-e2e-design.md` — read §3 before Task 1.

## Global Constraints

- **The extension must be the locally built one.** The published community build predates v0.4.0 and lacks the `cityparquet_*` pragmas. Export before any test run:
  ```bash
  export CITYLAKE_CITYJSON_EXTENSION=/data2/hideba/cityparquet-paper/cityparquet/lib/duckdb-cityjson/build/release/extension/cityjson/cityjson.duckdb_extension
  ```
- **No CityJSON implementation in Rust.** No parsing, geometry handling, module routing or CRS resolution. These tests call the crate's public API and assert on what comes back.
- **Tests must genuinely assert and be able to fail for the RIGHT reason.** Twelve tests were found unable to do so during the rebuild — assertions holding equally for the behaviour being ruled out, loops over collections empty in the fixture, substring checks two outcomes both satisfy. For every assertion here, ask what would have to break for it to fail.
- **Scope git operations to `lib/citylake/` and the root `justfile`.** Explicit pathspec; never `git add -A`.
- British English; document the present, never the past.
- `cargo fmt` applied and `cargo clippy --all-targets -- -D warnings` clean.

## Verified Ground Truth

Measured against the real feed before this plan was written. Do not re-derive; do not "correct" code that depends on these.

1. `https://cityjson.open3d.city/cityjsonseq/delft.city.jsonl` is 6.6 MB and holds **2231** CityObjects: **1115 `Building`** and **1116 `BuildingPart`**.
2. Its declared CRS resolves to **EPSG:7415**.
3. The extension reads that URL directly — `read_cityjsonseq('https://…')` works with no download step.
4. The whole pipeline — attach DuckLake, seed, init, insert all 2231, validate — takes **about 15 seconds**, and validation reports **zero errors**.
5. A real CityParquet package is published at `https://cityparquet.open3d.city/data/delft/`. It **cannot** be imported by URL today: `create_dataset` chooses between the file bootstrap and `import_package` using `std::path::Path::is_dir()`, which is false for a URL. This plan round-trips through a package written locally and records the URL case as a follow-up. Do not attempt to fix that here.

## A deliberate refinement of the spec

The spec proposed gating on `CITYLAKE_REAL_DATA=1`, with the tests returning early when it is unset. **Use `#[ignore]` instead.** An early return makes a skipped test report as *passed*, which is the false-green this project has spent a whole rebuild eliminating; `#[ignore]` makes the test harness itself print `ignored`, so a skipped run is visibly skipped. It also revives the meaning the crate's `test-integration` recipe originally had — Task 15 of the rebuild found that recipe dead precisely because nothing was `#[ignore]`d any more.

## File Structure

| File | Responsibility |
|---|---|
| `lib/citylake/tests/real_data.rs` | The whole suite. One file: every test shares the same source URL and the same setup, and splitting them would separate things that change together. |
| `lib/citylake/tests/common/mod.rs` | Existing. Gains nothing; `test_service()` and `fixture()` are reused as they are. |
| `lib/citylake/justfile` | Gains `test-real-data`. |
| `justfile` (root) | Gains nothing — the root gate stays fast and offline. |
| `lib/citylake/CLAUDE.md` + `AGENTS.md` | Gain a paragraph in Testing describing the suite and how to run it. |

---

### Task 1: The suite, and what the real feed contains

**Files:**
- Create: `lib/citylake/tests/real_data.rs`

**Interfaces:**
- Consumes: `common::test_service()` → `(DuckLakeService, TempDir)`; `DatasetName::new`, `ModuleName::new`, `QueryParams`; the async `CityLakeRepository` trait (`create_dataset`, `describe_dataset`, `query_objects`).
- Produces: `const DELFT_SEQ: &str` — the source URL later tasks reuse.

- [ ] **Step 1: Write the failing tests**

`lib/citylake/tests/real_data.rs`:

```rust
//! The operations against the published Delft datasets, at their real size.
//!
//! Every test here is `#[ignore]`d: they reach the network, and a gate that
//! needs the network is one people learn to skip. Run them deliberately with
//! `just test-real-data`. Marking them ignored rather than returning early
//! keeps a skipped run visibly skipped — an early return would report as a
//! pass, which is worse than no test.
//!
//! Nothing is downloaded. The extension auto-loads `httpfs` and resolves an
//! `https://` source as readily as a local path, so the URL goes straight to
//! `create_dataset` and the remote read path is exercised too.

mod common;

use citylake::core::interface::repository::CityLakeRepository;
use citylake::core::interface::types::{DatasetName, ModuleName, QueryParams};

/// 6.6 MB, 2231 CityObjects, EPSG:7415.
const DELFT_SEQ: &str = "https://cityjson.open3d.city/cityjsonseq/delft.city.jsonl";

/// Every object in this feed is a Building or a BuildingPart, and the Building
/// module holds both — so one module table carries all 2231 rows.
const TOTAL_OBJECTS: usize = 2231;
const BUILDINGS: usize = 1115;
const BUILDING_PARTS: usize = 1116;

#[tokio::test]
#[ignore = "network: reads the published Delft feed; run with `just test-real-data`"]
async fn the_real_feed_ingests_every_object() {
    let (service, _dir) = common::test_service();
    let name = DatasetName::new("delft_real").unwrap();

    let info = service
        .create_dataset(&name, DELFT_SEQ)
        .await
        .expect("ingest the published Delft feed");

    let rows: usize = info.modules.iter().map(|m| m.rows).sum();
    assert_eq!(
        rows, TOTAL_OBJECTS,
        "every object in the feed must arrive; the fixtures cannot show this"
    );
}

#[tokio::test]
#[ignore = "network: reads the published Delft feed; run with `just test-real-data`"]
async fn the_real_feed_routes_everything_into_the_building_module() {
    let (service, _dir) = common::test_service();
    let name = DatasetName::new("delft_real").unwrap();
    let info = service.create_dataset(&name, DELFT_SEQ).await.unwrap();

    // Building and BuildingPart belong to the same CityGML module, so a
    // correct routing produces exactly one object table. A second object
    // table would mean the extension split a module, or that the empty seed
    // survived when it should not have.
    let object_modules: Vec<&str> = info
        .modules
        .iter()
        .filter(|m| m.role == "object")
        .map(|m| m.name.as_str())
        .collect();
    assert_eq!(
        object_modules,
        vec!["building"],
        "the whole feed is one module; got {object_modules:?}"
    );

    let building = info
        .modules
        .iter()
        .find(|m| m.name == "building")
        .expect("a building module");
    assert_eq!(building.rows, TOTAL_OBJECTS);
}

#[tokio::test]
#[ignore = "network: reads the published Delft feed; run with `just test-real-data`"]
async fn the_real_feed_declares_its_crs() {
    let (service, _dir) = common::test_service();
    let name = DatasetName::new("delft_real").unwrap();
    let info = service.create_dataset(&name, DELFT_SEQ).await.unwrap();

    // The footer is minted by the extension from a probe row. At this size the
    // probe reads one row out of 2231 — that it still lands on the right CRS is
    // the thing worth checking.
    let crs = info.crs.expect("the feed declares EPSG:7415");
    assert!(crs.contains("7415"), "unexpected CRS: {crs}");
}

#[tokio::test]
#[ignore = "network: reads the published Delft feed; run with `just test-real-data`"]
async fn the_object_type_split_matches_the_published_figures() {
    let (service, _dir) = common::test_service();
    let name = DatasetName::new("delft_real").unwrap();
    let module = ModuleName::new("building").unwrap();
    service.create_dataset(&name, DELFT_SEQ).await.unwrap();

    let count_of = |object_type: &'static str| {
        let service = &service;
        let name = &name;
        let module = &module;
        async move {
            service
                .query_objects(
                    name,
                    module,
                    &QueryParams {
                        filter: Some(format!("object_type = '{object_type}'")),
                        limit: 5000,
                        offset: 0,
                    },
                )
                .await
                .unwrap_or_else(|e| panic!("query {object_type}: {e}"))
                .len()
        }
    };

    // Published figures for this dataset. A routing or filter regression moves
    // one of these without moving the total, which the first test would miss.
    assert_eq!(count_of("Building").await, BUILDINGS);
    assert_eq!(count_of("BuildingPart").await, BUILDING_PARTS);
    assert_eq!(BUILDINGS + BUILDING_PARTS, TOTAL_OBJECTS);
}
```

- [ ] **Step 2: Run them and watch them be ignored, then fail**

```bash
cd lib/citylake
cargo test --test real_data
```
Expected: `4 ignored` — proving the gate works and that a default run does not touch the network.

```bash
cargo test --test real_data -- --ignored
```
Expected: they RUN. If `create_dataset` cannot read the URL, that is a real finding — report it rather than switching to a local download.

- [ ] **Step 3: Make them pass**

No production code should be needed: the crate already supports everything these call. If a test fails, read the failure before changing anything — an assertion here encodes a measured fact, so a mismatch means either the published data moved or the crate has a scale-dependent bug. Say which in your report.

- [ ] **Step 4: Verify**

```bash
cargo test --test real_data -- --ignored
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```
Expected: 4 passed, clippy and fmt clean. The whole file should take well under a minute — a single ingest measured about 15 seconds.

- [ ] **Step 5: Commit**

```bash
git add lib/citylake/tests/real_data.rs
git commit -m "test(citylake): assert the real Delft feed at its published size

The suite proves the shape of every operation on three-object fixtures
and says nothing about 2231. These read the published feed directly —
the extension resolves https:// sources, so nothing is downloaded or
committed — and pin the object-type split, the routing into one module
and the CRS the footer probe recovers."
```

---

### Task 2: The package round trip, and a cascade, at real size

**Files:**
- Modify: `lib/citylake/tests/real_data.rs`

**Interfaces:**
- Consumes: `DELFT_SEQ`, `TOTAL_OBJECTS` from Task 1; `write_package`, `validate`, `delete_object`, `query_objects` from the trait.
- Produces: nothing further.

- [ ] **Step 1: Write the failing tests**

Append to `lib/citylake/tests/real_data.rs`:

```rust
#[tokio::test]
#[ignore = "network: reads the published Delft feed; run with `just test-real-data`"]
async fn the_real_dataset_validates_clean_on_arrival() {
    let (service, _dir) = common::test_service();
    let name = DatasetName::new("delft_real").unwrap();
    service.create_dataset(&name, DELFT_SEQ).await.unwrap();

    // 2231 objects with a real parent/child hierarchy: feature_id, the
    // reciprocal arrays and bbox are all derived across the whole set, and a
    // derivation that works on three objects can still break on 2231.
    let findings = service.validate(&name).await.expect("validate");
    let errors: Vec<_> = findings.iter().filter(|f| f.severity == "error").collect();
    assert!(errors.is_empty(), "unexpected errors: {errors:?}");
}

#[tokio::test]
#[ignore = "network: reads the published Delft feed; run with `just test-real-data`"]
async fn a_real_package_round_trips_through_the_lake() {
    let (service, dir) = common::test_service();
    let name = DatasetName::new("delft_real").unwrap();
    let ingested: usize = service
        .create_dataset(&name, DELFT_SEQ)
        .await
        .unwrap()
        .modules
        .iter()
        .map(|m| m.rows)
        .sum();

    let out = dir.path().join("delft_pkg");
    let written = service
        .write_package(&name, out.to_str().unwrap())
        .await
        .expect("write the package");
    assert!(written.iter().any(|f| f.file == "building.parquet"));
    assert!(written.iter().any(|f| f.file == "metadata.json"));

    let reloaded = DatasetName::new("delft_reloaded").unwrap();
    let info = service
        .create_dataset(&reloaded, out.to_str().unwrap())
        .await
        .expect("read the written package back");

    assert_eq!(
        info.modules.iter().map(|m| m.rows).sum::<usize>(),
        ingested,
        "every row must survive the round trip at this size"
    );
    assert!(
        info.crs.expect("a CRS survives the round trip").contains("7415"),
        "the written package must still declare EPSG:7415"
    );
}

#[tokio::test]
#[ignore = "network: reads the published Delft feed; run with `just test-real-data`"]
async fn deleting_a_real_parent_cascades_and_leaves_the_package_valid() {
    let (service, _dir) = common::test_service();
    let name = DatasetName::new("delft_real").unwrap();
    let module = ModuleName::new("building").unwrap();
    let before: usize = service
        .create_dataset(&name, DELFT_SEQ)
        .await
        .unwrap()
        .modules
        .iter()
        .map(|m| m.rows)
        .sum();

    // Find a real parent rather than hardcoding an id: this dataset's
    // Building rows carry BuildingPart children, and picking one from the data
    // keeps the test valid if the published feed is ever regenerated.
    let parents = service
        .query_objects(
            &name,
            &module,
            &QueryParams {
                filter: Some("children IS NOT NULL AND len(children) > 0".into()),
                limit: 1,
                offset: 0,
            },
        )
        .await
        .expect("find a parent");
    let parent = parents
        .first()
        .and_then(|row| row.get("id"))
        .and_then(|id| id.as_str())
        .expect("the feed must contain a Building with children")
        .to_string();

    let removed = service.delete_object(&name, &parent).await.expect("delete");
    assert!(
        removed > 1,
        "deleting a parent must take its children too; removed {removed}"
    );

    let after: usize = service
        .describe_dataset(&name)
        .await
        .unwrap()
        .modules
        .iter()
        .map(|m| m.rows)
        .sum();
    assert_eq!(after, before - removed);

    // The cascade must not leave a dangling reference behind in a set this
    // size — survivor cleanup is the part small fixtures cannot stress.
    let findings = service.validate(&name).await.expect("re-validate");
    let errors: Vec<_> = findings.iter().filter(|f| f.severity == "error").collect();
    assert!(errors.is_empty(), "cascade left the package invalid: {errors:?}");
}
```

- [ ] **Step 2: Run them and watch them fail**

```bash
cd lib/citylake && cargo test --test real_data -- --ignored
```
Expected: they run. The `children IS NOT NULL AND len(children) > 0` predicate is the one part not yet verified against this data — if it errors, inspect the column's actual type with the DuckDB CLI and adjust the predicate, saying in your report what it turned out to be. Do not weaken the test to "delete any object".

- [ ] **Step 3: Make them pass**

Again, no production code should be needed. If `removed > 1` fails, the cascade is not reaching children at this scale, which is a genuine finding — report it, do not relax the assertion.

- [ ] **Step 4: Verify**

```bash
cargo test --test real_data -- --ignored
cargo clippy --all-targets -- -D warnings && cargo fmt --check
```
Expected: 7 passed in the file, clippy and fmt clean.

- [ ] **Step 5: Commit**

```bash
git add lib/citylake/tests/real_data.rs
git commit -m "test(citylake): round-trip and cascade the real feed at 2231 objects

Derived state, survivor cleanup and the package writer all work on
three objects; none of that shows whether they work on a real
hierarchy. The parent is found from the data rather than hardcoded, so
the test survives the published feed being regenerated."
```

---

### Task 3: A recipe, and saying it exists

**Files:**
- Modify: `lib/citylake/justfile`
- Modify: `lib/citylake/CLAUDE.md`, then copy byte-identically to `lib/citylake/AGENTS.md`

**Interfaces:**
- Consumes: the suite from Tasks 1-2.
- Produces: `just test-real-data`.

- [ ] **Step 1: Add the recipe**

In `lib/citylake/justfile`, beside the existing test recipes:

```just
# Run the suite against the published Delft datasets (network, ~1 min).
#
# These are #[ignore]d so a plain `just test` neither reaches the network nor
# reports them as passing. Nothing is downloaded: the cityjson extension
# resolves https:// sources directly.
test-real-data:
    cargo test --test real_data -- --ignored
```

- [ ] **Step 2: Verify it runs**

```bash
cd lib/citylake && just test-real-data
```
Expected: 7 passed. Then confirm the default gate is unaffected and still offline:

```bash
cargo test 2>&1 | grep -c "ignored"
```
Expected: the real-data tests appear as ignored, not as passed.

- [ ] **Step 3: Document it**

Add a paragraph to the Testing section of `lib/citylake/CLAUDE.md` stating: that the suite exists and what it covers (the published Delft feed at 2231 objects — routing, the object-type split, the CRS the probe recovers, the package round trip, a real cascade); that it is `#[ignore]`d because it needs the network, and that this keeps a skipped run visibly skipped rather than falsely green; that nothing is downloaded or committed because the extension resolves `https://` sources; and that the hosted CityParquet package at `https://cityparquet.open3d.city/data/delft/` cannot yet be imported by URL, because `create_dataset` decides between the file bootstrap and `import_package` with `is_dir()`, which is false for a URL.

Then:
```bash
cp lib/citylake/CLAUDE.md lib/citylake/AGENTS.md
diff -q lib/citylake/CLAUDE.md lib/citylake/AGENTS.md
```
Expected: no output.

- [ ] **Step 4: Commit**

```bash
git add lib/citylake/justfile lib/citylake/CLAUDE.md lib/citylake/AGENTS.md
git commit -m "build(citylake): add the real-data recipe, and say what it covers

Also records why a hosted package cannot yet be imported by URL: the
create path chooses import_package with is_dir(), which a URL fails."
```

---

## Self-Review

**Spec coverage.** §3's source and figures → Task 1's constants, all measured. §3's "nothing downloaded, nothing committed" → the URL passed straight to `create_dataset`, with the reason in the module doc comment. §3's gating → Task 1, refined from the spec's env var to `#[ignore]` and argued for above. §3's assertions (routing, split, CRS, round trip, cascade, validation) → Tasks 1 and 2. §3's recorded limitation on URL package import → Task 3's documentation, and stated as ground truth so no task tries to fix it. §6's separate recipe → Task 3.

**Placeholders.** None: every test body is complete, and the one genuinely unverified detail — the `children` predicate — is called out as such with instructions to investigate rather than weaken.

**Type consistency.** `DELFT_SEQ`, `TOTAL_OBJECTS`, `BUILDINGS`, `BUILDING_PARTS` are defined in Task 1 and used in Task 2. `DatasetInfo.modules` is `Vec<ModuleInfo>` with `name`, `role`, `rows`; `ValidationFinding.severity` is a `String` compared against `"error"`; `PackageFile.file` is a `String`. `delete_object` returns `usize`. All match the crate as it stands.

**One thing deliberately not asserted.** No test pins how long the ingest takes. A timing assertion would fail on a slow network and tell nobody anything about the code.
