# Read-benchmark query design Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the read benchmark's degenerate `bbox-query` windows and its single position-dependent `id-lookup` target with windows binary-searched to a real row fraction and four id probes per format, deriving both in one shared, unit-testable place.

**Architecture:** Query-parameter derivation moves out of `coordinator.rs` into a new `cityparquet_readbench::params` library module, so the window search and probe selection are testable as pure functions with no child processes and no corpus. The coordinator keeps orchestration only and additionally writes the resolved parameters as a JSON sidecar; `readbench_duckdb.sh` reads that sidecar instead of re-deriving the same choices in bash.

**Tech Stack:** Rust 2024, `arrow-array`/`parquet` 58, `anyhow`, `serde`/`serde_json`, `cityparquet` (path dependency), bash + `jq` + DuckDB for the SQL baseline.

**Spec:** `docs/superpowers/specs/2026-08-25-readbench-query-design-design.md`

## Global Constraints

- **British English in prose** (comments, doc comments, Markdown). Repo-wide rule from `CLAUDE.md`.
- **Breaking changes are welcome.** No shims, no deprecation paths — update every call site instead.
- **Document the present, never the past.** No changelog voice in doc comments or reference docs.
- `benchmark/readbench` is **its own Cargo workspace**. Build and test with `cargo test --manifest-path benchmark/readbench/Cargo.toml`, never from the repository root's other workspaces.
- Dependency versions in `benchmark/readbench/Cargo.toml` must be spelled the same as `lib/cityparquet-rs/Cargo.toml`'s `[workspace.dependencies]`. For serde that is exactly `serde = { version = "1", features = ["derive"] }`.
- `fcb_core = "=0.7.6"` and `cjseq2 = "=0.1.0"` are **exact pins** and must stay exact.
- Benchmark caveats are part of the artefact. Never drop one to make a number look better.
- Every level keeps an `AGENTS.md` byte-identical to its `CLAUDE.md`. This plan touches neither, but do not let an edit drift them.
- Markdown is formatted with `npx --yes prettier@3.9.6 --write <file>` (the pinned version in `.githooks/pre-commit`).

---

### Task 1: Row-bbox scan in a new `params` module

Creates the module and moves the bbox column scan into it, returning **every row's** bbox rather than only the union. Later tasks search over that vector.

**Files:**

- Create: `benchmark/readbench/src/params.rs`
- Create: `benchmark/readbench/tests/params.rs`
- Modify: `benchmark/readbench/src/lib.rs` (add the module declaration)
- Modify: `benchmark/readbench/Cargo.toml` (add `serde`)

**Interfaces:**

- Consumes: nothing from earlier tasks.
- Produces:
  - `pub struct RowBoxes { pub boxes: Vec<[f64; 6]>, pub dataset: [f64; 6] }`
  - `pub fn scan_row_bboxes(table: &std::path::Path) -> anyhow::Result<RowBoxes>`

- [ ] **Step 1: Add the serde dependency**

In `benchmark/readbench/Cargo.toml`, under `[dependencies]`, directly after the `serde_json = "1"` line, add:

```toml
serde = { version = "1", features = ["derive"] }
```

- [ ] **Step 2: Declare the module**

In `benchmark/readbench/src/lib.rs`, append after `pub mod naming;`:

```rust
pub mod params;
```

Then extend the module doc comment's final sentence so it names the new member. Replace:

```rust
//! identically: [`format`], the set of formats measured, and [`naming`],
//! the input-extension convention every artefact path is derived through.
```

with:

```rust
//! identically: [`format`], the set of formats measured, [`naming`], the
//! input-extension convention every artefact path is derived through, and
//! [`params`], the query parameters every measurement is driven with.
```

- [ ] **Step 3: Write the failing test**

Create `benchmark/readbench/tests/params.rs`:

```rust
//! `cityparquet_readbench::params` against a real converted CityParquet
//! package, built here with `cityparquet::package::convert` from a committed
//! fixture — no network, no external tool, no prepared corpus.

use std::path::PathBuf;

use cityparquet::package::{ConvertOptions, convert};
use cityparquet_readbench::params::scan_row_bboxes;

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../lib/cityparquet-rs/tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

/// Converts `delft.city.jsonl` into a package in a fresh temp dir and returns
/// its single main table. Delft is the fixture that by-type-converts to
/// exactly ONE table (Building + BuildingPart both map to the "Building"
/// family), which is what these tests need.
fn delft_table() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("temp dir");
    let out = dir.path().join("delft.parquet");
    convert(&ConvertOptions {
        input: fixture("delft.city.jsonl"),
        output: out.clone(),
        ..Default::default()
    })
    .expect("converting delft");
    let table = out.join("building.parquet");
    assert!(table.exists(), "expected {} to exist", table.display());
    (dir, table)
}

#[test]
fn scan_row_bboxes_returns_one_box_per_row_and_their_union() {
    let (_dir, table) = delft_table();
    let scanned = scan_row_bboxes(&table).expect("scanning row bboxes");

    assert!(
        !scanned.boxes.is_empty(),
        "delft's table has rows, so it has row bboxes"
    );

    // The union must contain every row box, on every axis.
    for row in &scanned.boxes {
        for axis in 0..3 {
            assert!(
                scanned.dataset[axis] <= row[axis],
                "dataset min on axis {axis} must not exceed a row's min"
            );
            assert!(
                scanned.dataset[axis + 3] >= row[axis + 3],
                "dataset max on axis {axis} must not be below a row's max"
            );
        }
    }
}
```

- [ ] **Step 4: Run the test to verify it fails**

Run: `cargo test --manifest-path benchmark/readbench/Cargo.toml --test params`
Expected: FAIL to compile — `unresolved import cityparquet_readbench::params`.

- [ ] **Step 5: Write the implementation**

Create `benchmark/readbench/src/params.rs`. This lifts `union_batch_bbox`'s column plumbing out of `coordinator.rs` and keeps every row instead of folding it away:

```rust
//! Every query parameter a read-benchmark measurement is driven with,
//! derived once per dataset from the prepared artefacts themselves — never
//! hardcoded, never fabricated.
//!
//! This lives in the library rather than beside the coordinator so the
//! choices it makes are testable without spawning a single child process:
//! [`window_for_target`] is a pure function over an in-memory slice, and the
//! integration tests reach the rest directly.

use std::path::Path;

use anyhow::{Context, Result};
use arrow_array::{Array, Float64Array, StructArray};
use parquet::arrow::ProjectionMask;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

/// Every row's bbox, plus their union.
///
/// The whole vector is kept — not just the union — because
/// [`window_for_target`] searches for a window that intersects a target
/// FRACTION of rows, which cannot be answered from the extent alone. One
/// `[f64; 6]` per row is 48 bytes; the largest corpus dataset holds roughly
/// 199,000 rows, so under 10 MB.
pub struct RowBoxes {
    pub boxes: Vec<[f64; 6]>,
    pub dataset: [f64; 6],
}

/// Appends every row's bbox in `batch` to `out`. A row with a null bbox
/// contributes nothing — it has no extent, so no window can intersect it.
fn collect_batch_bboxes(batch: &arrow_array::RecordBatch, out: &mut Vec<[f64; 6]>) {
    let Some(bbox_col) = batch.column_by_name("bbox") else {
        return;
    };
    let Some(bbox_col) = bbox_col.as_any().downcast_ref::<StructArray>() else {
        return;
    };
    let leaf = |name: &str| {
        bbox_col
            .column_by_name(name)
            .and_then(|c| c.as_any().downcast_ref::<Float64Array>())
    };
    let (Some(xmin), Some(ymin), Some(zmin), Some(xmax), Some(ymax), Some(zmax)) = (
        leaf("xmin"),
        leaf("ymin"),
        leaf("zmin"),
        leaf("xmax"),
        leaf("ymax"),
        leaf("zmax"),
    ) else {
        return;
    };

    for row in 0..batch.num_rows() {
        if bbox_col.is_null(row) {
            continue;
        }
        out.push([
            xmin.value(row),
            ymin.value(row),
            zmin.value(row),
            xmax.value(row),
            ymax.value(row),
            zmax.value(row),
        ]);
    }
}

/// Scans the whole `bbox` column of `table` (a single-column projection),
/// keeping every row's own box and unioning them into the dataset extent.
pub fn scan_row_bboxes(table: &Path) -> Result<RowBoxes> {
    let file = std::fs::File::open(table).with_context(|| format!("opening {}", table.display()))?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)
        .with_context(|| format!("reading {}", table.display()))?;
    let projection = ProjectionMask::columns(builder.parquet_schema(), ["bbox"]);
    let reader = builder
        .with_projection(projection)
        .build()
        .with_context(|| format!("scanning bbox column of {}", table.display()))?;

    let mut boxes: Vec<[f64; 6]> = Vec::new();
    for batch in reader {
        let batch = batch.with_context(|| format!("reading a batch of {}", table.display()))?;
        collect_batch_bboxes(&batch, &mut boxes);
    }

    let mut iter = boxes.iter();
    let first = *iter.next().ok_or_else(|| {
        anyhow::anyhow!(
            "no row in {} has a bbox — cannot derive a query window",
            table.display()
        )
    })?;
    let dataset = iter.fold(first, |acc, row| {
        [
            acc[0].min(row[0]),
            acc[1].min(row[1]),
            acc[2].min(row[2]),
            acc[3].max(row[3]),
            acc[4].max(row[4]),
            acc[5].max(row[5]),
        ]
    });

    Ok(RowBoxes { boxes, dataset })
}
```

- [ ] **Step 6: Run the test to verify it passes**

Run: `cargo test --manifest-path benchmark/readbench/Cargo.toml --test params`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add benchmark/readbench/Cargo.toml benchmark/readbench/Cargo.lock \
        benchmark/readbench/src/lib.rs benchmark/readbench/src/params.rs \
        benchmark/readbench/tests/params.rs
git commit -m "feat(readbench): scan every row's bbox into a params module

The window search needs the per-row boxes, not only their union."
```

---

### Task 2: `window_for_target` — the binary search

The defect this whole plan exists for. Ten of the eighteen bbox measurements in the read results timed an empty window; this function is what replaces the lower-left anchor.

**Files:**

- Modify: `benchmark/readbench/src/params.rs`

**Interfaces:**

- Consumes: `RowBoxes` from Task 1.
- Produces:
  - `pub struct BboxWindow { pub tag: String, pub target: f64, pub achieved: f64, pub window: [f64; 6], pub approx: bool }`
  - `pub const BBOX_TARGETS: [(f64, &str); 3]`
  - `pub fn window_for_target(boxes: &[[f64; 6]], dataset: [f64; 6], target: f64, tag: &str) -> BboxWindow`

- [ ] **Step 1: Write the failing tests**

Append to `benchmark/readbench/src/params.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// A `rows x cols` grid of unit boxes on a 100 x 100 field.
    fn grid(rows: usize, cols: usize) -> Vec<[f64; 6]> {
        let mut out = Vec::with_capacity(rows * cols);
        for r in 0..rows {
            for c in 0..cols {
                let x = c as f64 * (100.0 / cols as f64);
                let y = r as f64 * (100.0 / rows as f64);
                out.push([x, y, 0.0, x + 0.1, y + 0.1, 1.0]);
            }
        }
        out
    }

    const FIELD: [f64; 6] = [0.0, 0.0, 0.0, 100.0, 100.0, 10.0];

    #[test]
    fn hits_every_target_on_a_uniform_grid() {
        let boxes = grid(100, 100); // 10,000 boxes
        for (target, tag) in BBOX_TARGETS {
            let w = window_for_target(&boxes, FIELD, target, tag);
            assert!(
                !w.approx,
                "{tag}: a uniform 10,000-box grid can hit {target} exactly, got {}",
                w.achieved
            );
            assert!(
                (w.achieved - target).abs() <= 0.1 * target,
                "{tag}: achieved {} is outside the tolerance around {target}",
                w.achieved
            );
        }
    }

    #[test]
    fn never_returns_an_empty_window() {
        let boxes = grid(100, 100);
        for (target, tag) in BBOX_TARGETS {
            let w = window_for_target(&boxes, FIELD, target, tag);
            assert!(w.achieved > 0.0, "{tag} selected no rows at all");
        }
    }

    /// The median centroid of a bimodal cloud falls in the gap between the
    /// two clusters. The search must still find a populated window rather
    /// than converging on the empty middle.
    #[test]
    fn finds_rows_when_the_median_falls_between_two_clusters() {
        let mut boxes = Vec::new();
        for i in 0..500 {
            let x = i as f64 * 0.02; // 0..10
            boxes.push([x, x, 0.0, x + 0.1, x + 0.1, 1.0]);
        }
        for i in 0..500 {
            let x = 90.0 + i as f64 * 0.02; // 90..100
            boxes.push([x, x, 0.0, x + 0.1, x + 0.1, 1.0]);
        }
        let w = window_for_target(&boxes, FIELD, 0.05, "bbox-5pct");
        assert!(
            w.achieved > 0.0,
            "a bimodal cloud must still yield a populated window, got {w:?}"
        );
    }

    /// A long, thin dataset must not receive a window whose y half-extent is
    /// so small it selects nothing: the half-extents scale with each axis's
    /// own span.
    #[test]
    fn scales_the_window_to_the_datasets_aspect_ratio() {
        let mut boxes = Vec::new();
        for i in 0..1000 {
            let x = i as f64; // 0..1000
            boxes.push([x, 0.0, 0.0, x + 0.5, 1.0, 1.0]);
        }
        let thin: [f64; 6] = [0.0, 0.0, 0.0, 1000.0, 1.0, 10.0];
        let w = window_for_target(&boxes, thin, 0.25, "bbox-25pct");
        assert!(w.achieved > 0.0, "thin dataset selected nothing: {w:?}");
        assert!(
            (w.achieved - 0.25).abs() <= 0.1 * 0.25,
            "thin dataset achieved {}, expected near 0.25",
            w.achieved
        );
    }

    /// 1% of 10 rows is 0.1 rows — unreachable. The search must disclose that
    /// with `approx` rather than silently reporting a missed target as met.
    #[test]
    fn flags_approx_when_the_target_is_unreachable() {
        let boxes = grid(2, 5); // 10 boxes
        let w = window_for_target(&boxes, FIELD, 0.01, "bbox-1pct");
        assert!(
            w.approx,
            "1% of 10 rows cannot be hit within tolerance; expected approx, got {w:?}"
        );
        assert!(
            w.achieved > 0.0,
            "even an unreachable target must yield a populated window, got {w:?}"
        );
    }

    #[test]
    fn the_window_always_spans_the_datasets_full_z_range() {
        let boxes = grid(50, 50);
        for (target, tag) in BBOX_TARGETS {
            let w = window_for_target(&boxes, FIELD, target, tag);
            assert_eq!(w.window[2], FIELD[2], "{tag} must keep the dataset zmin");
            assert_eq!(w.window[5], FIELD[5], "{tag} must keep the dataset zmax");
        }
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --manifest-path benchmark/readbench/Cargo.toml --lib params`
Expected: FAIL to compile — `cannot find function window_for_target`, `cannot find value BBOX_TARGETS`.

- [ ] **Step 3: Write the implementation**

Insert into `benchmark/readbench/src/params.rs`, above the `#[cfg(test)]` module:

```rust
/// `(target fraction of rows, notes tag)` for the three bbox windows — one
/// CSV row per entry.
///
/// The targets are fractions of ROWS, not of the dataset's area. An
/// area-anchored window says nothing about how many objects it selects: the
/// retired lower-left construction returned zero rows for `bbox-1pct` on
/// every dataset in the corpus, on every format.
pub const BBOX_TARGETS: [(f64, &str); 3] = [
    (0.01, "bbox-1pct"),
    (0.05, "bbox-5pct"),
    (0.25, "bbox-25pct"),
];

/// How far the achieved fraction may sit from the target before the window
/// is disclosed as `approx`, as a fraction OF THE TARGET (so 10% of 1% is
/// one part in a thousand, not one in ten).
const BBOX_TOLERANCE: f64 = 0.1;

/// Bisection steps. The count of intersecting rows is a step function of the
/// half-extent, so the search converges on a jump rather than a point; 60
/// halvings take the bracket well below one row's width on any real extent.
const BBOX_SEARCH_STEPS: u32 = 60;

/// One resolved bbox window: which target it was searched for, what fraction
/// of rows it actually selects, and whether those two agree.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BboxWindow {
    /// The `notes` tag this window's CSV rows carry, e.g. `bbox-1pct`.
    pub tag: String,
    /// The fraction of rows the search aimed at.
    pub target: f64,
    /// The fraction of rows the returned window actually intersects.
    pub achieved: f64,
    /// `[minx, miny, minz, maxx, maxy, maxz]`.
    pub window: [f64; 6],
    /// `achieved` is outside [`BBOX_TOLERANCE`] of `target` — the target was
    /// not reachable on this data. Disclosed in `notes`, never silent.
    pub approx: bool,
}

/// The same 3D overlap test every format runner applies row-by-row
/// (`formats::cityjsonseq::intersects`), so `achieved` is exactly what the
/// CityParquet runner will report for this window rather than an estimate.
fn intersects(row: &[f64; 6], window: &[f64; 6]) -> bool {
    for axis in 0..3 {
        if row[axis + 3] < window[axis] || row[axis] > window[axis + 3] {
            return false;
        }
    }
    true
}

/// The median of `values` (must be non-empty).
fn median_of(values: &mut [f64]) -> f64 {
    values.sort_by(|a, b| a.partial_cmp(b).expect("bbox coordinates are finite"));
    let mid = values.len() / 2;
    if values.len().is_multiple_of(2) {
        (values[mid - 1] + values[mid]) / 2.0
    } else {
        values[mid]
    }
}

/// A window centred on `centre`, extending `half` of each of the dataset's
/// own x/y spans, and always covering the dataset's FULL z range — a query
/// window's z must never exclude a row, because `readbench_duckdb.sh` tests
/// x/y overlap only and the two must agree.
fn window_at(centre: (f64, f64), half: f64, dataset: [f64; 6]) -> [f64; 6] {
    let span_x = dataset[3] - dataset[0];
    let span_y = dataset[4] - dataset[1];
    [
        centre.0 - half * span_x,
        centre.1 - half * span_y,
        dataset[2],
        centre.0 + half * span_x,
        centre.1 + half * span_y,
        dataset[5],
    ]
}

/// Searches for a window intersecting `target` (a fraction in `(0, 1]`) of
/// `boxes`, centred on the median row centre so it lands where the data is
/// rather than at a bounding-box corner.
///
/// The half-extent scales with each axis's OWN span, so a long, thin tile
/// receives a long, thin window instead of a square one that misses on the
/// short axis. The row count is monotonically non-decreasing in the
/// half-extent, which is what makes bisection valid.
///
/// A target that cannot be reached — 1% of a 10-row dataset is 0.1 rows —
/// returns the nearest achievable window with `approx` set, never a silently
/// missed target.
pub fn window_for_target(
    boxes: &[[f64; 6]],
    dataset: [f64; 6],
    target: f64,
    tag: &str,
) -> BboxWindow {
    let total = boxes.len();
    assert!(total > 0, "window_for_target needs at least one row box");

    let mut xs: Vec<f64> = boxes.iter().map(|b| (b[0] + b[3]) / 2.0).collect();
    let mut ys: Vec<f64> = boxes.iter().map(|b| (b[1] + b[4]) / 2.0).collect();
    let centre = (median_of(&mut xs), median_of(&mut ys));

    let count_at = |half: f64| -> usize {
        let w = window_at(centre, half, dataset);
        boxes.iter().filter(|b| intersects(b, &w)).count()
    };

    let fraction = |count: usize| count as f64 / total as f64;
    let wanted = target * total as f64;

    // `hi` must select everything: half = 1.0 spans the full extent either
    // side of the centre, which covers the dataset whatever the centre is.
    let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
    for _ in 0..BBOX_SEARCH_STEPS {
        let mid = (lo + hi) / 2.0;
        if (count_at(mid) as f64) < wanted {
            lo = mid;
        } else {
            hi = mid;
        }
    }

    // `hi` is the smallest searched half-extent reaching the target; `lo` the
    // largest falling short. Whichever lands closer to the target wins, but
    // never an empty window — a zero-row window is the defect this function
    // replaces.
    let mut best = hi;
    let mut best_count = count_at(hi);
    let lo_count = count_at(lo);
    if lo_count > 0 && (fraction(lo_count) - target).abs() < (fraction(best_count) - target).abs() {
        best = lo;
        best_count = lo_count;
    }

    let achieved = fraction(best_count);
    BboxWindow {
        tag: tag.to_string(),
        target,
        achieved,
        window: window_at(centre, best, dataset),
        approx: (achieved - target).abs() > BBOX_TOLERANCE * target,
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --manifest-path benchmark/readbench/Cargo.toml --lib params`
Expected: PASS, six tests.

- [ ] **Step 5: Commit**

```bash
git add benchmark/readbench/src/params.rs
git commit -m "feat(readbench): search bbox windows to a target row fraction

Centred on the median row centre and scaled to each axis's own span, so a
window lands where the data is rather than at an empty bounding-box corner.
An unreachable target is disclosed as approx, never silently missed."
```

---

### Task 3: Id probes — deciles, the verified miss, and presence checks

**Files:**

- Modify: `benchmark/readbench/src/params.rs`
- Modify: `benchmark/readbench/tests/params.rs`

**Interfaces:**

- Consumes: nothing from Tasks 1-2.
- Produces:
  - `pub struct IdProbe { pub tag: String, pub id: String, pub present: bool, pub substituted: bool }`
  - `pub const ID_DECILES: [(f64, &str); 3]`
  - `pub fn seq_feature_ids(seq_path: &Path) -> Result<Vec<String>>`
  - `pub fn citygml_ids(gml_path: &Path) -> Result<std::collections::HashSet<String>>`
  - `pub fn miss_id(seed: &str, taken: &std::collections::HashSet<String>) -> String`
  - `pub fn id_probes(seq_ids: &[String], verifiable: &std::collections::HashSet<String>) -> Vec<IdProbe>`

- [ ] **Step 1: Write the failing unit tests**

Add inside the existing `#[cfg(test)] mod tests` in `benchmark/readbench/src/params.rs`:

```rust
    use std::collections::HashSet;

    fn ids(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("obj-{i}")).collect()
    }

    #[test]
    fn deciles_land_at_their_nominal_positions() {
        let all = ids(100);
        let present: HashSet<String> = all.iter().cloned().collect();
        let probes = id_probes(&all, &present);

        let hit = |tag: &str| {
            probes
                .iter()
                .find(|p| p.tag == tag)
                .unwrap_or_else(|| panic!("no probe tagged {tag}"))
                .id
                .clone()
        };
        assert_eq!(hit("id-10pct"), "obj-10");
        assert_eq!(hit("id-50pct"), "obj-50");
        assert_eq!(hit("id-90pct"), "obj-90");
    }

    #[test]
    fn every_decile_probe_is_present_and_the_miss_probe_is_not() {
        let all = ids(100);
        let present: HashSet<String> = all.iter().cloned().collect();
        let probes = id_probes(&all, &present);

        assert_eq!(probes.len(), 4, "three deciles plus one miss");
        for probe in &probes {
            if probe.tag == "id-miss" {
                assert!(!probe.present, "the miss probe must be absent");
                assert!(
                    !present.contains(&probe.id),
                    "the miss id must not be in the dataset"
                );
            } else {
                assert!(probe.present, "{} must be a real id", probe.tag);
            }
        }
    }

    /// A decile id missing from the CityGML artefact would be timed as a hit
    /// and recorded as a miss. It must be replaced by the nearest verifiable
    /// feature, and the substitution disclosed.
    #[test]
    fn substitutes_the_nearest_verifiable_id_and_says_so() {
        let all = ids(100);
        let mut verifiable: HashSet<String> = all.iter().cloned().collect();
        verifiable.remove("obj-50");
        let probes = id_probes(&all, &verifiable);

        let mid = probes.iter().find(|p| p.tag == "id-50pct").expect("id-50pct");
        assert_ne!(mid.id, "obj-50", "the unverifiable id must be replaced");
        assert!(mid.present, "the replacement must itself be verifiable");
        assert!(mid.substituted, "the substitution must be disclosed");
        assert!(
            verifiable.contains(&mid.id),
            "the replacement must be in the verifiable set"
        );
    }

    #[test]
    fn miss_id_avoids_a_collision_with_an_existing_id() {
        let mut taken = HashSet::new();
        taken.insert("obj-1".to_string());
        taken.insert("obj-1-readbench-absent".to_string());
        taken.insert("obj-1-readbench-absent-2".to_string());

        let miss = miss_id("obj-1", &taken);
        assert!(!taken.contains(&miss), "miss_id returned a taken id: {miss}");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --manifest-path benchmark/readbench/Cargo.toml --lib params`
Expected: FAIL to compile — `cannot find function id_probes`, `cannot find function miss_id`.

- [ ] **Step 3: Write the pure implementation**

Insert into `benchmark/readbench/src/params.rs`, above the `#[cfg(test)]` module:

```rust
/// `(position in the canonical order, notes tag)` for the three id-lookup
/// hit probes. A single target would make the published time a function of
/// where that one id happened to sit in the stream.
pub const ID_DECILES: [(f64, &str); 3] = [
    (0.10, "id-10pct"),
    (0.50, "id-50pct"),
    (0.90, "id-90pct"),
];

/// The tag of the fourth probe: an id verified absent from the dataset.
/// Position-free, and the number that actually separates a format with an id
/// index from one without.
pub const ID_MISS_TAG: &str = "id-miss";

/// One resolved id-lookup target.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct IdProbe {
    /// The `notes` tag this probe's CSV row carries.
    pub tag: String,
    pub id: String,
    /// Whether this id is expected to be found. False only for
    /// [`ID_MISS_TAG`].
    pub present: bool,
    /// The nominal decile id was not verifiable in every artefact and the
    /// nearest one that was has been used instead.
    pub substituted: bool,
}

/// An id guaranteed absent from `taken`, derived from `seed` so it is
/// reproducible across runs rather than random.
pub fn miss_id(seed: &str, taken: &std::collections::HashSet<String>) -> String {
    let base = format!("{seed}-readbench-absent");
    if !taken.contains(&base) {
        return base;
    }
    for suffix in 2u32.. {
        let candidate = format!("{base}-{suffix}");
        if !taken.contains(&candidate) {
            return candidate;
        }
    }
    unreachable!("u32 exhausted while avoiding an id collision")
}

/// The four id probes for a dataset: three positioned hits plus a verified
/// miss.
///
/// `seq_ids` is the canonical order — the CityJSONSeq stream every other
/// artefact is cut from. `verifiable` is the set of ids confirmed to exist in
/// EVERY artefact that will be asked for them; a nominal decile id outside it
/// is replaced by the nearest index that is inside, with `substituted` set,
/// because timing a lookup for an id one artefact does not contain would
/// record a hit as a miss.
pub fn id_probes(
    seq_ids: &[String],
    verifiable: &std::collections::HashSet<String>,
) -> Vec<IdProbe> {
    assert!(!seq_ids.is_empty(), "id_probes needs at least one feature");

    let nearest_verifiable = |from: usize| -> Option<(usize, String)> {
        for offset in 0..seq_ids.len() {
            for index in [from.saturating_sub(offset), (from + offset).min(seq_ids.len() - 1)] {
                if verifiable.contains(&seq_ids[index]) {
                    return Some((index, seq_ids[index].clone()));
                }
            }
        }
        None
    };

    let mut probes: Vec<IdProbe> = Vec::with_capacity(ID_DECILES.len() + 1);
    for (position, tag) in ID_DECILES {
        let nominal = ((position * seq_ids.len() as f64) as usize).min(seq_ids.len() - 1);
        let Some((index, id)) = nearest_verifiable(nominal) else {
            continue;
        };
        probes.push(IdProbe {
            tag: tag.to_string(),
            id,
            present: true,
            substituted: index != nominal,
        });
    }

    let taken: std::collections::HashSet<String> = seq_ids.iter().cloned().collect();
    let seed = probes
        .iter()
        .find(|p| p.tag == "id-50pct")
        .map(|p| p.id.clone())
        .unwrap_or_else(|| seq_ids[0].clone());
    probes.push(IdProbe {
        tag: ID_MISS_TAG.to_string(),
        id: miss_id(&seed, &taken),
        present: false,
        substituted: false,
    });

    probes
}
```

- [ ] **Step 4: Run the unit tests to verify they pass**

Run: `cargo test --manifest-path benchmark/readbench/Cargo.toml --lib params`
Expected: PASS, ten tests.

- [ ] **Step 5: Write the failing artefact-reader test**

Append to `benchmark/readbench/tests/params.rs`:

```rust
use cityparquet_readbench::params::{citygml_ids, seq_feature_ids};

#[test]
fn seq_feature_ids_reads_the_stream_in_order_and_skips_the_metadata_line() {
    let ids = seq_feature_ids(&fixture("delft.city.jsonl")).expect("reading seq ids");
    assert!(!ids.is_empty(), "delft has features");
    assert!(
        !ids.iter().any(|id| id.is_empty()),
        "no feature id may be empty"
    );
    // The first line is the CityJSON metadata object, not a feature, so the
    // count is the feature count rather than the line count.
    let lines = std::fs::read_to_string(fixture("delft.city.jsonl"))
        .expect("reading the fixture")
        .lines()
        .filter(|l| !l.trim().is_empty())
        .count();
    assert_eq!(
        ids.len(),
        lines - 1,
        "one id per feature line, the metadata line excluded"
    );
}

/// `b1_lod2_cs_w_sem.gml` is one of the two CityGML 2.0 files `just
/// fixtures` fetches — a single semantically-decomposed building.
#[test]
fn citygml_ids_collects_every_city_object_key() {
    let ids = citygml_ids(&fixture("b1_lod2_cs_w_sem.gml")).expect("reading citygml ids");
    assert!(!ids.is_empty(), "the CityGML fixture has city objects");
}
```

- [ ] **Step 6: Run it to verify it fails**

Run: `cargo test --manifest-path benchmark/readbench/Cargo.toml --test params`
Expected: FAIL to compile — `cannot find function seq_feature_ids`.

There is no `delft.gml`; `lib/cityparquet-rs/tests/fixtures` holds `b1_lod2_cs_w_sem.gml`, `b1_lod2_s.gml`, `berlin_citygml1.gml` (CityGML **1.0**, which this reader rejects) and `freiburg_no_preamble_srs.gml`. Use `b1_lod2_cs_w_sem.gml` as written.

- [ ] **Step 7: Write the artefact readers**

Insert into `benchmark/readbench/src/params.rs`, above the `#[cfg(test)]` module:

```rust
/// Every feature's own top-level `id`, in the CityJSONSeq stream's order —
/// the canonical order the id deciles are cut from, because
/// `readbench_prepare.sh` builds the gzipped, FlatCityBuf and CityParquet
/// artefacts from this one file.
///
/// The first line of a `.city.jsonl` is the CityJSON metadata object, not a
/// feature; it is skipped.
pub fn seq_feature_ids(seq_path: &Path) -> Result<Vec<String>> {
    use std::io::BufRead as _;

    let file = std::fs::File::open(seq_path)
        .with_context(|| format!("opening {}", seq_path.display()))?;
    let reader = std::io::BufReader::new(file);

    let mut ids = Vec::new();
    for (index, line) in reader.lines().enumerate() {
        let line = line.with_context(|| format!("reading {}", seq_path.display()))?;
        if line.trim().is_empty() || index == 0 {
            continue;
        }
        let feature: cityparquet::cjseq::CityJSONFeature = serde_json::from_str(&line)
            .with_context(|| format!("parsing feature on line {} of {}", index + 1, seq_path.display()))?;
        ids.push(feature.id);
    }

    if ids.is_empty() {
        anyhow::bail!(
            "{} holds no features — cannot derive id probes",
            seq_path.display()
        );
    }
    Ok(ids)
}

/// Every CityObject key in a CityGML artefact.
///
/// The CityGML is synthesised from the source CityJSON by `citygml-tools`
/// rather than cut from the seq stream, so its member set is not guaranteed
/// to match: `benchmark/README.md` records that `3dbag_9-284-556` loses an
/// LoD in that round trip. An id probe absent here would be timed as a hit
/// and recorded as a miss, which is why every probe is checked against this
/// set.
pub fn citygml_ids(gml_path: &Path) -> Result<std::collections::HashSet<String>> {
    let source = cityparquet::source::Source::open(gml_path, cityparquet::source::SourceFormat::CityGml)
        .with_context(|| format!("opening {}", gml_path.display()))?;
    let transform = source.transform().clone();
    let mut reader = cityparquet::citygml::FeatureReader::open_without_appearance(gml_path, &transform)
        .map_err(|e| anyhow::anyhow!(e))
        .with_context(|| format!("streaming {}", gml_path.display()))?;

    let mut ids = std::collections::HashSet::new();
    for feature in reader.by_ref() {
        let feature = feature
            .map_err(|e| anyhow::anyhow!(e))
            .with_context(|| format!("reading a member of {}", gml_path.display()))?;
        ids.extend(feature.city_objects.keys().cloned());
    }
    Ok(ids)
}
```

**Note for the implementer:** `Source::open`/`transform()` and
`FeatureReader::open_without_appearance` are used exactly as
`benchmark/readbench/src/formats/citygml.rs` uses them — read that file's
`Document::open` before writing this, and mirror its calls rather than
guessing the signatures. If they differ, follow the runner, not this snippet.

- [ ] **Step 8: Run the tests to verify they pass**

Run: `cargo test --manifest-path benchmark/readbench/Cargo.toml --test params`
Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add benchmark/readbench/src/params.rs benchmark/readbench/tests/params.rs
git commit -m "feat(readbench): derive four id probes per dataset

Three deciles of the CityJSONSeq stream plus a verified-absent miss, each
hit probe checked against the CityGML artefact so a synthesis gap cannot
turn a hit into a silent miss."
```

---

### Task 4: `ResolvedParams::resolve` and the coordinator switching over

Moves the remaining derivation helpers out of `coordinator.rs` and makes the coordinator drive `bbox-query` from the searched windows.

**Files:**

- Modify: `benchmark/readbench/src/params.rs`
- Modify: `benchmark/readbench/src/coordinator.rs` (delete `scan_dataset_bbox`, `bbox_window`, `BBOX_FRACTIONS`, `most_frequent_object_type`, `pick_numeric_attribute`, `sample_object_id`, `union_batch_bbox`, `utf8_values`; call `params::resolve`)
- Modify: `benchmark/readbench/tests/params.rs`

**Interfaces:**

- Consumes: `scan_row_bboxes`, `window_for_target`, `BBOX_TARGETS`, `seq_feature_ids`, `citygml_ids`, `id_probes` from Tasks 1-3.
- Produces:
  - `pub struct ResolvedParams { pub dataset: String, pub windows: Vec<BboxWindow>, pub id_probes: Vec<IdProbe>, pub object_type: String, pub object_type_count: u64, pub numeric_attr: Option<String>, pub cp_object_total: u64 }`
  - `pub fn resolve(dataset: &str, cp_table: &Path, seq_path: &Path, gml_path: Option<&Path>) -> Result<ResolvedParams>`

- [ ] **Step 1: Write the failing test**

Append to `benchmark/readbench/tests/params.rs`:

```rust
use cityparquet_readbench::params::resolve;

#[test]
fn resolve_produces_three_populated_windows_and_four_id_probes() {
    let (_dir, table) = delft_table();
    let resolved = resolve(
        "delft.city.jsonl",
        &table,
        &fixture("delft.city.jsonl"),
        None,
    )
    .expect("resolving params");

    assert_eq!(resolved.windows.len(), 3, "three bbox windows");
    for window in &resolved.windows {
        assert!(
            window.achieved > 0.0,
            "{} selected no rows — the defect this replaces",
            window.tag
        );
    }

    assert_eq!(resolved.id_probes.len(), 4, "three deciles plus a miss");
    assert!(!resolved.object_type.is_empty(), "an object_type was chosen");
    assert!(resolved.cp_object_total > 0, "a non-zero denominator");
}

#[test]
fn resolve_fails_loudly_when_the_seq_artefact_is_missing() {
    let (_dir, table) = delft_table();
    let err = resolve(
        "delft.city.jsonl",
        &table,
        Path::new("/nonexistent/delft.city.jsonl"),
        None,
    )
    .expect_err("a missing seq artefact must be a hard failure");
    let message = format!("{err:#}");
    assert!(
        message.contains("delft.city.jsonl"),
        "the error must name the missing artefact, got: {message}"
    );
}
```

Add `use std::path::Path;` to that file's imports if it is not already there.

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test --manifest-path benchmark/readbench/Cargo.toml --test params`
Expected: FAIL to compile — `cannot find function resolve`.

- [ ] **Step 3: Move the remaining derivation helpers**

Cut these four functions from `benchmark/readbench/src/coordinator.rs` and paste them into `benchmark/readbench/src/params.rs`, changing each from private to `pub(crate)` where the coordinator still needs it and `pub` where `resolve` returns its result:

- `most_frequent_object_type` (`coordinator.rs:756`) — make it `fn`, private to `params`
- `pick_numeric_attribute` (`coordinator.rs:784`) — private to `params`
- `utf8_values` (`coordinator.rs:720`) — private to `params`, it exists only for `most_frequent_object_type`
- `open_metadata` and `open_arrow_schema` (`coordinator.rs:604`, `:611`) — **copy**, do not move: `params::resolve` needs them and so does the coordinator's `cityparquet` runner plumbing. Keep the coordinator's copies as they are.

Delete outright, with no replacement:

- `scan_dataset_bbox` (`coordinator.rs:673`) — superseded by `params::scan_row_bboxes`
- `union_batch_bbox` (`coordinator.rs:~630`) — superseded by `params::collect_batch_bboxes`
- `bbox_window` (`coordinator.rs:700`) — superseded by `params::window_for_target`
- `BBOX_FRACTIONS` (`coordinator.rs:121`) — superseded by `params::BBOX_TARGETS`
- `sample_object_id` (`coordinator.rs:802`) — superseded by `params::id_probes`

Move the imports each moved function needs (`StringArray`, `DictionaryArray`, `Int32Type`, `ArrayAccessor`, `DataType`, `Schema`, `CityMetadata`) into `params.rs`, and delete any that `coordinator.rs` no longer uses. `cargo build` names every one it got wrong.

- [ ] **Step 4: Write `ResolvedParams` and `resolve`**

Insert into `benchmark/readbench/src/params.rs`:

```rust
/// Every query parameter one dataset's whole (format x scenario) matrix is
/// driven with, derived once from the prepared artefacts.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ResolvedParams {
    /// The dataset name as it appears in the results CSV's `dataset` column.
    pub dataset: String,
    pub windows: Vec<BboxWindow>,
    pub id_probes: Vec<IdProbe>,
    /// The most-frequent `object_type` value — the `attr-filter` predicate.
    pub object_type: String,
    pub object_type_count: u64,
    /// The alphabetically-first Int64/Float64 attribute column, or `None`
    /// when the dataset has no numeric attribute at all. Never fabricated:
    /// `attr-stats` and `project` are skipped when this is `None`.
    pub numeric_attr: Option<String>,
    /// The dataset-global CityObject total — the SHARED selectivity
    /// denominator for every CityObject-level scenario.
    pub cp_object_total: u64,
}

/// Derives every query parameter for `dataset`.
///
/// Requires BOTH the CityParquet package's main table (`cp_table`, the
/// source of the row bboxes, the attribute choices and the denominator) and
/// the CityJSONSeq artefact (`seq_path`, the canonical order the id deciles
/// are cut from). `gml_path` is checked when present so an id probe absent
/// from the synthesised CityGML is substituted rather than silently timed as
/// a miss; pass `None` when the CityGML artefact is not part of the run.
///
/// A missing required artefact is a hard failure. A benchmark that quietly
/// fabricated a parameter would publish a number nobody could check.
pub fn resolve(
    dataset: &str,
    cp_table: &Path,
    seq_path: &Path,
    gml_path: Option<&Path>,
) -> Result<ResolvedParams> {
    let rows = scan_row_bboxes(cp_table)?;
    let windows = BBOX_TARGETS
        .iter()
        .map(|(target, tag)| window_for_target(&rows.boxes, rows.dataset, *target, tag))
        .collect();

    let seq_ids = seq_feature_ids(seq_path)?;
    let mut verifiable: std::collections::HashSet<String> = seq_ids.iter().cloned().collect();
    if let Some(gml) = gml_path {
        let gml_ids = citygml_ids(gml)?;
        verifiable.retain(|id| gml_ids.contains(id));
        if verifiable.is_empty() {
            anyhow::bail!(
                "no feature id in {} also appears in {} — the two artefacts \
                 describe different data",
                seq_path.display(),
                gml.display()
            );
        }
    }
    let id_probes = id_probes(&seq_ids, &verifiable);

    let meta = open_metadata(cp_table)?;
    let schema = open_arrow_schema(cp_table)?;
    let (object_type, object_type_count) = most_frequent_object_type(cp_table)?;
    let numeric_attr = pick_numeric_attribute(&meta, &schema);

    let file = std::fs::File::open(cp_table)
        .with_context(|| format!("opening {}", cp_table.display()))?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)
        .with_context(|| format!("reading {}", cp_table.display()))?;
    let cp_object_total = builder.metadata().file_metadata().num_rows() as u64;

    Ok(ResolvedParams {
        dataset: dataset.to_string(),
        windows,
        id_probes,
        object_type,
        object_type_count,
        numeric_attr,
        cp_object_total,
    })
}
```

- [ ] **Step 5: Rewire the coordinator's derivation block**

In `benchmark/readbench/src/coordinator.rs`'s `run`, replace the derivation block at `coordinator.rs:250-268` (from `let meta = open_metadata(&cp_table)?;` down to and including the `total_count_for(Format::CityParquet, ...)` call) with:

```rust
    // Every QueryParams for this dataset, derived once from the prepared
    // artefacts (see `cityparquet_readbench::params`) — no hardcoded ids,
    // attributes or windows anywhere in this function.
    let seq_path = match Format::CityJsonSeq.artefact(base) {
        Artefact::Prepared(name) => opts.prepared_dir.join(name),
        Artefact::NotCoordinated => unreachable!("cityjsonseq is always a prepared artefact"),
    };
    let gml_path = match Format::CityGml.artefact(base) {
        Artefact::Prepared(name) => {
            let path = opts.prepared_dir.join(name);
            path.exists().then_some(path)
        }
        Artefact::NotCoordinated => None,
    };
    let resolved = params::resolve(&dataset, &cp_table, &seq_path, gml_path.as_deref())?;
```

Add `use cityparquet_readbench::params;` to the file's imports, and make sure `Artefact` is imported (it already is, alongside `Format`).

Then update every reader of the old variables:

- `windows` becomes `resolved.windows`; the loop `for (window, tag) in &windows` becomes `for window in &resolved.windows`, the params build uses `bbox: Some(window.window)`, and the `notes` argument becomes a tag that discloses an approximate window:

```rust
                Scenario::BBoxQuery => {
                    for window in &resolved.windows {
                        let params = QueryParams {
                            bbox: Some(window.window),
                            ..Default::default()
                        };
                        let notes = if window.approx {
                            format!("{};approx", window.tag)
                        } else {
                            window.tag.clone()
                        };
                        run_measurement(
                            &mut rows,
                            &dataset,
                            format,
                            source,
                            *scenario,
                            &params,
                            opts.repeat,
                            Some(total),
                            &notes,
                        )?;
                    }
                }
```

- `object_type_value` becomes `resolved.object_type`, `object_type_count` becomes `resolved.object_type_count`
- `numeric_attr` becomes `resolved.numeric_attr`
- `cp_object_total` becomes `resolved.cp_object_total`
- `sample_id` has no replacement here — Task 5 rewrites the `IdLookup` arm. **Until Task 5 lands, keep the arm compiling** by driving it from the first probe:

```rust
                Scenario::IdLookup => {
                    let probe = &resolved.id_probes[0];
                    let params = QueryParams {
                        target_id: Some(probe.id.clone()),
                        ..Default::default()
                    };
                    let notes = probe.tag.clone();
```

Also update the log line at `coordinator.rs:275` that prints the derived choices so it reads from `resolved`.

- [ ] **Step 6: Build and run the whole suite**

Run: `cargo test --manifest-path benchmark/readbench/Cargo.toml`
Expected: PASS. `tests/coordinator.rs` exercises the coordinator end-to-end through the built binary, so a rewiring mistake surfaces there.

- [ ] **Step 7: Commit**

```bash
git add benchmark/readbench/src/params.rs benchmark/readbench/src/coordinator.rs \
        benchmark/readbench/tests/params.rs
git commit -m "refactor(readbench): derive query parameters in params::resolve

The coordinator keeps orchestration; the choices it used to make inline are
now testable without spawning a child process. bbox-query switches to the
searched windows, and an approximate window says so in notes."
```

---

### Task 5: Four id-lookup rows, and CityGML's early exit

**Files:**

- Modify: `benchmark/readbench/src/coordinator.rs` (the `Scenario::IdLookup` arm)
- Modify: `benchmark/readbench/src/formats/citygml.rs` (`stream_members`, the `IdLookup` arm, and its existing `#[cfg(test)] mod tests` at `citygml.rs:434`)

**Interfaces:**

- Consumes: `ResolvedParams::id_probes` from Task 4.
- Produces: four CSV rows per format for `id-lookup`, tagged `id-10pct`, `id-50pct`, `id-90pct`, `id-miss`.

- [ ] **Step 1: Write the failing CityGML early-exit test**

The assertion counts **members visited**, not elapsed time: `railway_lod3_fragment.gml` holds four `cityObjectMember`s, so a timing comparison would be pure noise, while the visit count is exact and deterministic.

Add to the existing `#[cfg(test)] mod tests` in `benchmark/readbench/src/formats/citygml.rs`:

```rust
    /// A committed real fragment under `crates/core/tests/data/` — the same
    /// one `tests/citygml_runner.rs` uses for its grain assertions. It holds
    /// four `cityObjectMember`s: a `bldg:Building` (first), a `brid:Bridge`,
    /// a `veg:SolitaryVegetationObject` and a `grp:CityObjectGroup`.
    fn railway_fragment() -> std::path::PathBuf {
        let p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../lib/cityparquet-rs/crates/core/tests/data")
            .join("railway_lod3_fragment.gml");
        assert!(p.exists(), "missing committed fixture {}", p.display());
        p
    }

    /// A hit in the FIRST member must stop there; an absent id must walk all
    /// four. Without the early exit both visit four.
    #[test]
    fn id_lookup_stops_at_the_hit_instead_of_draining_the_document() {
        let doc = Document::open(&railway_fragment()).expect("opening the fragment");

        // The first member's own CityObject key, whatever it is named.
        let mut first_key = None;
        stream_members_until(&doc, |feature| {
            if first_key.is_none() {
                first_key = feature.city_objects.keys().next().cloned();
            }
            Ok(true)
        })
        .expect("streaming the first member");
        let first_key = first_key.expect("the first member has a CityObject");

        let (found, visited_on_hit) =
            count_members_until_id(&doc, &first_key).expect("hit lookup");
        assert!(found, "the first member's own id must be found");
        assert_eq!(visited_on_hit, 1, "a first-member hit must visit one member");

        let (found, visited_on_miss) =
            count_members_until_id(&doc, "definitely-not-in-this-document").expect("miss lookup");
        assert!(!found, "an absent id must not be found");
        assert_eq!(visited_on_miss, 4, "a miss must walk every member");
    }

    /// The `IdLookup` traversal, with the member count the scenario itself
    /// discards — the observable this test needs.
    fn count_members_until_id(doc: &Document, id: &str) -> Result<(bool, u64)> {
        let mut visited = 0u64;
        let found = stream_members_until(doc, |feature| {
            visited += 1;
            Ok(feature.city_objects.contains_key(id))
        })?;
        Ok((found, visited))
    }
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test --manifest-path benchmark/readbench/Cargo.toml --lib citygml`
Expected: FAIL to compile — `cannot find function stream_members_until`.

- [ ] **Step 3: Give `stream_members` an early exit**

In `benchmark/readbench/src/formats/citygml.rs`, add a stopping variant beside `stream_members` (keep `stream_members` itself unchanged — every other scenario needs the full pass and its guard):

```rust
/// Streams members until `visit` returns `true` (a hit), then stops.
///
/// Deliberately does NOT run `ensure_every_member_was_mapped`: that guard is
/// a property of the ARTEFACT, not of any one scenario, and the coordinator
/// already spawns an untimed `Count` child per format per dataset whose full
/// pass exercises it. Draining the document after the answer is known would
/// measure work a reader with an index would never do, and this scenario
/// exists to compare mechanisms.
fn stream_members_until<F>(doc: &Document, mut visit: F) -> Result<bool>
where
    F: FnMut(CityJSONFeature) -> Result<bool>,
{
    let mut reader = FeatureReader::open_without_appearance(&doc.path, &doc.transform)
        .map_err(|e| anyhow!(e))
        .with_context(|| format!("streaming {}", doc.origin))?;
    for feature in reader.by_ref() {
        let feature = feature.map_err(|e| anyhow!(e))?;
        if visit(feature)? {
            return Ok(true);
        }
    }
    Ok(false)
}
```

Then replace the `Scenario::IdLookup` arm in `run_scenario`:

```rust
        // Stops at the hit, the best a document with no index can do. The
        // skipped-member guard is covered by the untimed `Count` pass — see
        // `stream_members_until`.
        Scenario::IdLookup => {
            let id = require(&params.target_id, "target-id", scenario)?;
            let found = stream_members_until(doc, |feature| {
                Ok(feature.city_objects.contains_key(id))
            })?;
            Ok(found as u64)
        }
```

Update this module's doc comment where it states that `IdLookup` deliberately does not stop early, so the file documents the present behaviour.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --manifest-path benchmark/readbench/Cargo.toml --lib citygml`
Then: `cargo test --manifest-path benchmark/readbench/Cargo.toml --test citygml_runner`
Expected: PASS, including the existing integration tests — `count` and every other scenario must still go through `stream_members` and its guard.

- [ ] **Step 5: Emit four id-lookup rows**

In `benchmark/readbench/src/coordinator.rs`, replace the whole `Scenario::IdLookup` arm with:

```rust
                Scenario::IdLookup => {
                    for probe in &resolved.id_probes {
                        let params = QueryParams {
                            target_id: Some(probe.id.clone()),
                            ..Default::default()
                        };
                        let mut notes = probe.tag.clone();
                        if probe.substituted {
                            notes.push_str(";id-substituted");
                        }
                        run_measurement(
                            &mut rows,
                            &dataset,
                            format,
                            source,
                            *scenario,
                            &params,
                            opts.repeat,
                            Some(resolved.cp_object_total),
                            &notes,
                        )?;
                    }
                }
```

- [ ] **Step 6: Assert the four rows in the coordinator test**

Append to `benchmark/readbench/tests/coordinator.rs`, matching that file's existing helpers for running the binary and reading the CSV:

```rust
#[test]
fn id_lookup_emits_three_positioned_probes_and_a_verified_miss() {
    let (_dir, prepared, input) = prepared_delft();
    let out = prepared.join("results.csv");
    run_coordinator(&input, &prepared, &out, &["cityparquet"]);

    let rows = read_rows(&out);
    let id_rows: Vec<_> = rows.iter().filter(|r| r.scenario == "id-lookup").collect();
    assert_eq!(id_rows.len(), 4, "three deciles plus a miss, got {id_rows:?}");

    let tags: Vec<&str> = id_rows.iter().map(|r| r.notes.as_str()).collect();
    for expected in ["id-10pct", "id-50pct", "id-90pct", "id-miss"] {
        assert!(
            tags.iter().any(|t| t.starts_with(expected)),
            "missing a row tagged {expected}; got {tags:?}"
        );
    }

    let miss = id_rows
        .iter()
        .find(|r| r.notes.starts_with("id-miss"))
        .expect("a miss row");
    assert_eq!(miss.result_count, 0, "the miss probe must find nothing");
    for hit in id_rows.iter().filter(|r| !r.notes.starts_with("id-miss")) {
        assert_eq!(hit.result_count, 1, "{} must find its object", hit.notes);
    }
}
```

**Implementer's note:** `prepared_delft`, `run_coordinator`, `read_rows` and
the row struct's field names are placeholders for whatever
`tests/coordinator.rs` already calls its equivalents. Read that file and use
its names; do not add duplicates.

- [ ] **Step 7: Run the suite**

Run: `cargo test --manifest-path benchmark/readbench/Cargo.toml`
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add benchmark/readbench/src/coordinator.rs \
        benchmark/readbench/src/formats/citygml.rs \
        benchmark/readbench/tests/coordinator.rs
git commit -m "feat(readbench): four id-lookup probes, and let CityGML stop at the hit

Three deciles plus a verified miss, so the published time stops being a
function of where one id happened to sit. CityGML now uses the best
mechanism its format affords, like every other runner; the skipped-member
guard rides on the untimed Count pass."
```

---

### Task 6: The resolved-parameters sidecar

**Files:**

- Modify: `benchmark/readbench/src/coordinator.rs` (write the sidecar)
- Modify: `benchmark/readbench/tests/coordinator.rs`

**Interfaces:**

- Consumes: `ResolvedParams` from Task 4 (already `Serialize`).
- Produces: `<out>.params.json`, a `ResolvedParams` serialised with `serde_json::to_string_pretty`. Task 7 reads it from bash.

- [ ] **Step 1: Write the failing test**

Append to `benchmark/readbench/tests/coordinator.rs`:

```rust
#[test]
fn the_run_writes_a_resolved_params_sidecar_beside_the_csv() {
    let (_dir, prepared, input) = prepared_delft();
    let out = prepared.join("results.csv");
    run_coordinator(&input, &prepared, &out, &["cityparquet"]);

    let sidecar = prepared.join("results.csv.params.json");
    assert!(
        sidecar.exists(),
        "expected a sidecar at {}",
        sidecar.display()
    );

    let text = std::fs::read_to_string(&sidecar).expect("reading the sidecar");
    let parsed: serde_json::Value = serde_json::from_str(&text).expect("valid JSON");

    assert_eq!(
        parsed["windows"].as_array().expect("windows").len(),
        3,
        "three windows in the sidecar"
    );
    assert_eq!(
        parsed["id_probes"].as_array().expect("id_probes").len(),
        4,
        "four id probes in the sidecar"
    );
    assert!(
        !parsed["object_type"].as_str().expect("object_type").is_empty(),
        "the sidecar carries the attr-filter predicate"
    );
    assert!(
        parsed["cp_object_total"].as_u64().expect("cp_object_total") > 0,
        "the sidecar carries the shared denominator"
    );

    // Every window in the sidecar must be populated — the whole point.
    for window in parsed["windows"].as_array().unwrap() {
        assert!(
            window["achieved"].as_f64().expect("achieved") > 0.0,
            "sidecar window {} selects no rows",
            window["tag"]
        );
    }
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test --manifest-path benchmark/readbench/Cargo.toml --test coordinator the_run_writes_a_resolved_params_sidecar`
Expected: FAIL — no such file.

- [ ] **Step 3: Write the sidecar**

In `benchmark/readbench/src/coordinator.rs`, immediately after the `params::resolve` call added in Task 4, add:

```rust
    // The resolved parameters, beside the CSV this run owns. It is the ONE
    // description of which windows, ids and attributes this run measured —
    // `benchmark/scripts/readbench_duckdb.sh` reads it rather than
    // re-deriving the same choices in bash, so the two cannot drift.
    let sidecar = params_sidecar_path(&opts.out);
    fs::write(
        &sidecar,
        serde_json::to_string_pretty(&resolved)
            .context("serialising the resolved query parameters")?,
    )
    .with_context(|| format!("writing {}", sidecar.display()))?;
```

And add, beside the other free functions:

```rust
/// The resolved-parameters sidecar for a results CSV: the CSV's own path
/// with `.params.json` appended, so the two travel together and a run cannot
/// leave a stale sidecar behind for a different CSV.
pub fn params_sidecar_path(out: &Path) -> PathBuf {
    let mut name = out.as_os_str().to_os_string();
    name.push(".params.json");
    PathBuf::from(name)
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test --manifest-path benchmark/readbench/Cargo.toml --test coordinator`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add benchmark/readbench/src/coordinator.rs benchmark/readbench/tests/coordinator.rs
git commit -m "feat(readbench): write the resolved query parameters beside the CSV

One description of what a run measured, for the SQL baseline to read
instead of re-deriving the same choices in bash."
```

---

### Task 7: `readbench_duckdb.sh` reads the sidecar

**Files:**

- Modify: `benchmark/scripts/readbench_duckdb.sh`
- Create: `benchmark/scripts/tests/readbench_duckdb_test.sh`
- Modify: `justfile` (the `bench` recipe, `justfile:424` and `justfile:427`)

**Interfaces:**

- Consumes: `<out>.params.json` from Task 6.
- Produces: `duckdb-parquet` CSV rows whose windows and attribute choices are the coordinator's, not the script's.

- [ ] **Step 1: Write the failing bash test**

Create `benchmark/scripts/tests/readbench_duckdb_test.sh`, following the structure of the sibling `readbench_prepare_test.sh` (read it first — reuse its assertion helpers and its exit convention):

```bash
#!/usr/bin/env bash
# `readbench_duckdb.sh`'s contract with the coordinator's resolved-parameters
# sidecar: it reads the windows and the attribute choices from that file, and
# refuses to run without it. A silent fall back to its own bash derivation is
# exactly the drift the sidecar exists to remove.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DUCKDB_SH="$SCRIPT_DIR/../readbench_duckdb.sh"

fail() { echo "FAIL: $*" >&2; exit 1; }

# --- A missing --params is a hard failure, never a fallback ---
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

if "$DUCKDB_SH" "$tmp/nonexistent.parquet" "$tmp/out.csv" \
     --params "$tmp/nonexistent.params.json" >"$tmp/log" 2>&1; then
  fail "the script succeeded with no params file; it must refuse"
fi
grep -qi "params" "$tmp/log" \
  || fail "the failure message must name the missing params file; got: $(cat "$tmp/log")"

# --- The script must not carry its own window construction any more ---
if grep -qE '^\s*(function\s+)?bbox_window\s*\(\)' "$DUCKDB_SH"; then
  fail "readbench_duckdb.sh still defines bbox_window; it must read the sidecar"
fi
if grep -q "most_frequent" "$DUCKDB_SH"; then
  fail "readbench_duckdb.sh still derives object_type; it must read the sidecar"
fi

echo "PASS: readbench_duckdb.sh reads the resolved-parameters sidecar"
```

Make it executable: `chmod +x benchmark/scripts/tests/readbench_duckdb_test.sh`

- [ ] **Step 2: Run it to verify it fails**

Run: `./benchmark/scripts/tests/readbench_duckdb_test.sh`
Expected: FAIL — the script neither accepts `--params` nor has had its bash derivation removed.

- [ ] **Step 3: Rewrite the script's parameter sourcing**

In `benchmark/scripts/readbench_duckdb.sh`:

1. Add a `--params <file>` argument to its option parsing, alongside the existing `--numeric-column` and `--repeat`. Make it **required**: if absent or unreadable, print an error naming the expected path and `exit 1`.
2. Delete the `bbox_window` bash function and the `object_type` most-frequent query entirely.
3. Read the windows with `jq`, one CSV row per entry, using the sidecar's own `tag` (appending `;approx` when `.approx` is true, matching the coordinator's own `notes` construction):

```bash
readarray -t WINDOWS < <(jq -r '.windows[] | [.tag, .approx, .window[0], .window[1], .window[2], .window[3], .window[4], .window[5]] | @tsv' "$PARAMS")
```

4. Read the attribute choices:

```bash
OBJECT_TYPE="$(jq -r '.object_type' "$PARAMS")"
NUMERIC_COLUMN="$(jq -r '.numeric_attr // empty' "$PARAMS")"
```

5. **Fix the `project` divergence**: the scenario currently runs `SELECT count(object_type)`. It must use `"$NUMERIC_COLUMN"`, and skip the scenario with a note on stderr when `NUMERIC_COLUMN` is empty — exactly what the coordinator does. Retire the `--numeric-column` flag; the sidecar is now the source. Update the recipe call sites in Step 5.
6. Update the script's header comment: the "Window construction and the AttrFilter tie-break MATCH `benchmark/readbench/src/coordinator.rs` exactly" paragraph is now wrong in its mechanism. Replace it with a statement that both are READ from the coordinator's `<out>.params.json`, so they cannot drift, and that the script must therefore run after the coordinator.

- [ ] **Step 4: Run the bash test to verify it passes**

Run: `./benchmark/scripts/tests/readbench_duckdb_test.sh`
Expected: PASS.

- [ ] **Step 5: Update the recipe call sites**

In `justfile`, the `bench` recipe calls the script at `justfile:424` and `justfile:427` with `--numeric-column "$numeric_col"` and without. Replace both with a single call passing the sidecar:

```bash
                ./{{BENCH_SCRIPTS}}/readbench_duckdb.sh "$pkg" "$out" --params "$out.params.json" --repeat 7
```

Delete the now-dead `numeric_col` detection block above it (`justfile:405` and its surrounding lines) — the sidecar carries that choice.

- [ ] **Step 6: Run the shell suite**

Run: `just scripts-test`
Expected: PASS, including the new file. If the suite enumerates test scripts by an explicit list rather than a glob, add the new script to that list.

- [ ] **Step 7: Commit**

```bash
git add benchmark/scripts/readbench_duckdb.sh \
        benchmark/scripts/tests/readbench_duckdb_test.sh justfile
git commit -m "feat(readbench): drive the DuckDB baseline from the resolved params

The script read its windows and attribute choices from its own bash copy of
the coordinator's logic while claiming exact parity. It now reads the
coordinator's sidecar, which makes the claim structural, and its project
scenario stops using object_type where the coordinator uses the numeric
column."
```

---

### Task 8: Re-run the corpora and bring the methodology up to date

The benchmark artefacts and the prose are part of the deliverable: a caveat that no longer matches the code is a defect.

**Files:**

- Modify: `benchmark/formats/READ_BENCHMARK.md`
- Modify: `benchmark/README.md`
- Modify: `benchmark/formats/archive/2026-08-17-catalogue-corpus/README.md` (or that directory's own top-level note — check what is there)
- Regenerate: `benchmark/formats/read_results/*.csv` (not committed)
- Regenerate: `benchmark/formats/scaling_read_results/*.csv` (**committed**)

- [ ] **Step 1: Re-run the read corpus**

The six-dataset corpus is already prepared under `benchmark/formats/data/readbench`, so no fetch is needed.

Run: `just bench benchmark/formats/data/benchmark`

**This is multi-hour.** Run it in the background and check on it rather than blocking. `zurich_building_lod2`'s CityGML alone is roughly 20 s per sample, and the matrix now emits 12 rows per format instead of 9.

- [ ] **Step 2: Verify no window is empty**

Run:

```bash
awk -F, 'NR>1 && $3=="bbox-query" && $5=="0" {print FILENAME": "$2" "$11}' \
  benchmark/formats/read_results/*.csv
```

Expected: **no output.** Any line is a window that still selects nothing — stop and fix the search before going further.

- [ ] **Step 3: Verify the id probes**

Run:

```bash
awk -F, 'NR>1 && $3=="id-lookup" {print $2, $11, $5}' \
  benchmark/formats/read_results/ingolstadt.csv
```

Expected: four rows per format; `result_count` 1 for the three decile tags and 0 for `id-miss`.

- [ ] **Step 4: Fetch, prepare and re-run the scaling corpus**

```bash
just fetch-scaling-data
just readbench-prepare <each prepared scaling slice>   # see the scaling recipes in justfile
just scaling-bench                                      # or the four per-cardinality recipes
```

Read `justfile:195-220` and the four per-dataset recipes before running: they are in one file on purpose, because `benchmark/readbench/tests/strip_extension.rs` extracts all four and runs them. Use them as written rather than hand-rolling the invocation.

- [ ] **Step 5: Confirm the committed scaling results changed as intended**

Run:

```bash
awk -F, 'NR>1 && $3=="bbox-query" {print FILENAME": "$4" "$5" "$11}' \
  benchmark/formats/scaling_read_results/*.csv
```

Expected: no zero `result_count`, and `selectivity` near 0.01/0.05/0.25 for the three tags on each cardinality.

- [ ] **Step 6: Rewrite the methodology**

In `benchmark/formats/READ_BENCHMARK.md`:

- Rewrite the numbered caveat describing the lower-left window construction so it describes the target-row-fraction search: the window is centred on the median row centre, scales with each axis's own span, and covers the full z range.
- Rewrite the numbered caveat describing `id-lookup`'s single target so it describes the four probes and states which formats stop early.
- Add four caveats, in the file's existing numbered style:
  1. the CityGML artefact's member order is derived independently through `citygml-tools` and need not match the seq stream the deciles are cut from, so a probe's position within the CityGML is only approximately its decile — presence is verified, position is not
  2. FlatCityBuf's `id` is a CityObject map key rather than a member of the `attributes` map its schema indexes, so `id-lookup` is a full walk regardless of `fcb ser -A`
  3. a bbox row's target and its achieved selectivity can differ on a small dataset, and `approx` in `notes` marks the rows where they do
  4. bbox targets are expressed in CityParquet row space, so feature-grained formats report a different achieved selectivity for the identical window

Keep the file's own numbering scheme and cross-references intact; the summary page quotes these verbatim, so edit here rather than paraphrasing downstream.

- [ ] **Step 7: Update the benchmark README**

In `benchmark/README.md`, update the "Caveats that are load-bearing" section and the committed-artefacts table so both describe the present construction. Add a line to the archived corpus note recording that `archive/2026-08-17-catalogue-corpus/` predates this window construction and its bbox rows are not comparable with current runs.

- [ ] **Step 8: Format the Markdown**

```bash
npx --yes prettier@3.9.6 --log-level warn --write \
  benchmark/README.md benchmark/formats/READ_BENCHMARK.md
```

- [ ] **Step 9: Run the full gate**

Run: `just check`
Expected: PASS — both Cargo workspaces plus the two harness suites.

- [ ] **Step 10: Commit**

```bash
git add benchmark/formats/scaling_read_results benchmark/README.md \
        benchmark/formats/READ_BENCHMARK.md benchmark/formats/archive
git commit -m "bench(readbench): re-run both corpora on the new query design

Every bbox window now selects rows; id-lookup reports three positions and a
verified miss. The methodology's numbered caveats describe the present
construction, with four added for the disclosures the new design creates."
```

---

## Self-Review

**Spec coverage.** Every section of the spec maps to a task: the id-lookup probe design to Tasks 3 and 5; the best-mechanism table to Task 5 (CityGML's early exit) with FlatCityBuf and CityParquet already correct and unchanged; the bbox window search to Task 2; `params.rs` and the module split to Tasks 1-4; the sidecar and the DuckDB rewiring to Tasks 6-7; the `project` divergence fix to Task 7 Step 3.5; the testing section to the tests embedded in Tasks 1-7; the re-runs and every documentation edit to Task 8. The spec's "derivation requires the seq artefact as well" appears as Task 4's `resolve` signature and its missing-artefact test. DuckDB's absent `id-lookup` scenario is out of scope in the spec and is not added here.

**Type consistency.** `BboxWindow`, `IdProbe` and `ResolvedParams` field names are used identically in Tasks 2-7 and in the sidecar JSON the bash test and script read (`windows`, `id_probes`, `object_type`, `numeric_attr`, `cp_object_total`, `tag`, `achieved`, `approx`, `window`). `window_for_target` keeps the same four-parameter signature everywhere. `params_sidecar_path` is defined once in Task 6 and its output path (`<out>.params.json`) is what Task 7's recipe passes.

**Fixture names verified.** There is no `delft.gml`; Task 3 uses `b1_lod2_cs_w_sem.gml`, one of the two CityGML 2.0 files `just fixtures` fetches. Task 5's early-exit test uses the committed `railway_lod3_fragment.gml` (four members, first is the Building) and asserts **members visited**, not elapsed time — four members would make a timing comparison noise. Both live where `tests/citygml_runner.rs` already reaches for them.

**Known softness, flagged rather than hidden.** Tasks 5 and 6's coordinator tests name helpers (`prepared_delft`, `run_coordinator`, `read_rows`) standing in for whatever `tests/coordinator.rs` already calls its equivalents; both steps say so and instruct the implementer to read the file and reuse its names rather than adding duplicates. Task 3's `citygml_ids` mirrors calls made in `formats/citygml.rs`'s `Document::open` and instructs the implementer to follow that file if the signatures differ.
