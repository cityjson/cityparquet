# CLI Multi-Input & Spatial Partitioning — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development (recommended) or superpowers:executing-plans. Steps use `- [ ]`.

**Goal:** `convert` accepts multiple/wildcard inputs merged into one dataset, and can partition the result into N self-contained CityParquet packages by `count`/`features`/`box`.

**Architecture:** Two seams over the existing single-`Source` pipeline. (M-A) input globbing + an in-memory `Source` variant + `merge_sources` that requantises heterogeneous inputs onto one transform; `convert` factored into `convert_source`. (M-B) `assign_partitions` (pure) + `convert_partitioned` driver that runs `convert_source` per partition group + CLI flags. Reuses `encode_buffered` and the per-package crash-safe temp-dir swap.

**Tech Stack:** Rust 2024, clap 4 (derive), cjseq 0.4, arrow 58, `glob` crate (new workspace dep).

## Global Constraints

- British English in prose and doc-comments.
- Strict red-green TDD. **Real fixtures only** — `tests/fixtures/{delft.city.jsonl, lod3_railway.city.json}` and the crate `tests/data/*.gml` hand fixtures. NO inline artificial CityJSON.
- `just check` (clippy `-D warnings` + `cargo fmt --check` + workspace tests + schema isolation) is the green gate for every commit.
- `delft.city.jsonl` has **2231** CityObjects (asserted in existing tests) — the partition-completeness invariant reuses this.
- Do not edit other submodules.
- Consult Fable (`Agent` subagent, `model: fable`) at hard design points. At each milestone boundary run a Codex external review with the **`sol`** model (`codex exec --cd "$(pwd)" -m sol --sandbox read-only "..."`); triage + fix Critical/Important.
- `Transform { scale: Vec<f64>, translate: Vec<f64> }` (cjseq). `CityJSONFeature { id: String, vertices: Vec<Vec<i64>>, city_objects, appearance }`.
- Test fixture helper (copy into each new `tests/*.rs`):
  ```rust
  fn fixture(name: &str) -> std::path::PathBuf {
      let p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
          .join("../../tests/fixtures").join(name);
      assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
      p
  }
  ```

---

# Milestone M-A: Multiple / wildcard inputs merged into one package

Deliverable: `convert IN1 IN2 ... -o OUT` (dir/glob-aware) writes ONE merged package; identical output to today for a single input.

### Task A1: `resolve_inputs` — expand files / directories / globs

**Files:**
- Create: `crates/cityparquet/src/inputs.rs`
- Modify: `crates/cityparquet/src/lib.rs` (add `pub mod inputs;`)
- Modify: `Cargo.toml` (workspace dep `glob = "0.3"`), `crates/cityparquet/Cargo.toml` (`glob = { workspace = true }`)
- Test: unit tests in `inputs.rs`

**Interfaces:**
- Produces: `pub fn resolve_inputs(patterns: &[PathBuf]) -> Result<Vec<PathBuf>>` — expands each pattern (file → itself; directory → immediate children with extension `json`/`jsonl`/`gml`; glob string → `glob::glob` matches that are files), then canonicalises, de-duplicates, and sorts. Empty result → `Err(CityParquetError::Io(...))`.

- [ ] **Step 1: Add the `glob` dependency.** In root `Cargo.toml` under `[workspace.dependencies]` add `glob = "0.3"`; in `crates/cityparquet/Cargo.toml` under `[dependencies]` add `glob = { workspace = true }`. Run `cargo metadata >/dev/null` to confirm it resolves.

- [ ] **Step 2: Write the failing test.** In `crates/cityparquet/src/inputs.rs`:
```rust
//! Resolve CLI input patterns (files, directories, globs) into a concrete,
//! de-duplicated, sorted list of source files.
use std::path::PathBuf;
use cityparquet_schema::{CityParquetError, Result};

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn touch(dir: &std::path::Path, name: &str) -> PathBuf {
        let p = dir.join(name);
        fs::write(&p, "{}").unwrap();
        p
    }

    #[test]
    fn resolves_directory_to_recognised_children_sorted() {
        let d = tempfile::tempdir().unwrap();
        touch(d.path(), "b.city.jsonl");
        touch(d.path(), "a.city.json");
        touch(d.path(), "c.gml");
        touch(d.path(), "ignore.txt");
        let sub = d.path().join("nested");
        fs::create_dir(&sub).unwrap();
        touch(&sub, "deep.city.json"); // non-recursive: excluded

        let got = resolve_inputs(&[d.path().to_path_buf()]).unwrap();
        let names: Vec<_> = got.iter().map(|p| p.file_name().unwrap().to_str().unwrap().to_string()).collect();
        assert_eq!(names, vec!["a.city.json", "b.city.jsonl", "c.gml"]);
    }

    #[test]
    fn glob_and_explicit_file_are_deduped() {
        let d = tempfile::tempdir().unwrap();
        let a = touch(d.path(), "a.city.json");
        touch(d.path(), "b.city.json");
        let pat = d.path().join("*.city.json");
        let got = resolve_inputs(&[pat, a.clone()]).unwrap();
        assert_eq!(got.len(), 2, "duplicate a.city.json must collapse");
    }

    #[test]
    fn empty_resolution_is_error() {
        let d = tempfile::tempdir().unwrap();
        assert!(resolve_inputs(&[d.path().to_path_buf()]).is_err());
    }
}
```

- [ ] **Step 3: Run test to verify it fails.** `cargo test -p cityparquet --lib inputs::` → FAIL (`resolve_inputs` not found). Also add `pub mod inputs;` to `lib.rs` so it compiles to the failure.

- [ ] **Step 4: Implement.** In `inputs.rs`:
```rust
const RECOGNISED_EXTS: [&str; 3] = ["json", "jsonl", "gml"];

fn is_recognised(p: &std::path::Path) -> bool {
    p.is_file()
        && p.extension()
            .and_then(|e| e.to_str())
            .map(|e| RECOGNISED_EXTS.contains(&e))
            .unwrap_or(false)
}

fn looks_like_glob(s: &str) -> bool {
    s.contains(['*', '?', '['])
}

pub fn resolve_inputs(patterns: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut out: Vec<PathBuf> = Vec::new();
    for pat in patterns {
        let s = pat.to_string_lossy();
        if looks_like_glob(&s) {
            let entries = glob::glob(&s)
                .map_err(|e| CityParquetError::Io(format!("bad glob {s}: {e}")))?;
            for entry in entries {
                let p = entry.map_err(|e| CityParquetError::Io(format!("glob error: {e}")))?;
                if p.is_file() {
                    out.push(p);
                } else {
                    eprintln!("warning: glob match {} is not a file; skipping", p.display());
                }
            }
        } else if pat.is_dir() {
            let mut children: Vec<PathBuf> = std::fs::read_dir(pat)
                .map_err(|e| CityParquetError::Io(format!("cannot read dir {}: {e}", pat.display())))?
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| is_recognised(p))
                .collect();
            children.sort();
            out.append(&mut children);
        } else if pat.is_file() {
            out.push(pat.clone());
        } else {
            return Err(CityParquetError::Io(format!("input not found: {}", pat.display())));
        }
    }
    // Canonicalise for de-dup; fall back to the raw path if canonicalisation fails.
    let mut seen = std::collections::BTreeSet::new();
    let mut deduped = Vec::new();
    for p in out {
        let key = std::fs::canonicalize(&p).unwrap_or_else(|_| p.clone());
        if seen.insert(key) {
            deduped.push(p);
        }
    }
    deduped.sort();
    if deduped.is_empty() {
        return Err(CityParquetError::Io("no input files resolved".to_string()));
    }
    Ok(deduped)
}
```

- [ ] **Step 5: Run test to verify it passes.** `cargo test -p cityparquet --lib inputs::` → PASS.

- [ ] **Step 6: `just check`, then commit.**
```bash
git add -A && git commit -m "feat(inputs): resolve files/dirs/globs into a source list"
```

### Task A2: In-memory `Source` variant

**Files:**
- Modify: `crates/cityparquet/src/source.rs`
- Test: unit tests in `source.rs`

**Interfaces:**
- Consumes: nothing new.
- Produces:
  - `pub fn Source::from_parts(header: CityJSON, features: Vec<CityJSONFeature>, doc_appearance: Option<cjseq::Appearance>, format: SourceFormat) -> Source`
  - `Source::features()`, `header()`, `format()`, `doc_appearance()` all work for the in-memory variant.

Add an internal `buffered: Option<BufferedSource>` where `struct BufferedSource { features: Vec<CityJSONFeature>, doc_appearance: Option<cjseq::Appearance> }`; when set, `features()` returns a new `FeatureIter::Buffered(std::slice::Iter)` yielding `Ok(feature.clone())`, and `doc_appearance()` returns `buffered.doc_appearance.as_ref()`. Keep `path`/`doc` as `Option`. Guard: `from_parts` sets `format` to the passed value (callers use `CityJsonSeq`, the feature-local-appearance convention).

- [ ] **Step 1: Write the failing test.** In `source.rs` tests:
```rust
#[test]
fn buffered_source_round_trips_features_and_header() {
    // Build a tiny in-memory source from a real fixture's header + its features.
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/delft.city.jsonl");
    let disk = Source::open(&path).unwrap();
    let feats: Vec<_> = disk.features().unwrap().map(|f| f.unwrap()).take(3).collect();
    let mem = Source::from_parts(disk.header().clone(), feats.clone(), None, SourceFormat::CityJsonSeq);
    let got: Vec<_> = mem.features().unwrap().map(|f| f.unwrap()).collect();
    assert_eq!(got.len(), 3);
    assert_eq!(got[0].id, feats[0].id);
    assert_eq!(mem.format(), SourceFormat::CityJsonSeq);
}
```

- [ ] **Step 2: Run to verify it fails.** `cargo test -p cityparquet --lib source::tests::buffered_source` → FAIL (`from_parts` missing).

- [ ] **Step 3: Implement** the `buffered` field, `from_parts`, the `FeatureIter::Buffered` arm (clone-on-yield), and route `features()`/`doc_appearance()`. Make `Source`'s fields keep working for the file paths (existing `open` sets `buffered: None`).

- [ ] **Step 4: Run to verify it passes**, and run the whole `source::` module + `cargo test -p cityparquet --lib` to confirm no regression.

- [ ] **Step 5: `just check`, commit.** `git commit -m "feat(source): in-memory Source::from_parts variant"`

### Task A3: Extract `convert_source` from `convert`

**Files:**
- Modify: `crates/cityparquet/src/package.rs`
- Test: existing `tests/convert_real_data.rs` is the characterization guard (must stay green).

**Interfaces:**
- Produces: `pub fn convert_source(source: &Source, opts: &ConvertOptions) -> Result<ConvertReport>` — everything today's `convert` does after `Source::open`, using `opts.output_dir`/`overwrite`/etc. and ignoring `opts.input`.
- `pub fn convert(opts: &ConvertOptions)` becomes: `let source = Source::open(&opts.input)?; convert_source(&source, opts)`.

- [ ] **Step 1: Write the failing test** in `tests/convert_real_data.rs`:
```rust
#[test]
fn convert_source_matches_convert_object_count() {
    let out = tempfile::tempdir().unwrap();
    let opts = ConvertOptions::new(fixture("delft.city.jsonl"), out.path().to_path_buf());
    let src = cityparquet::source::Source::open(&opts.input).unwrap();
    let report = cityparquet::package::convert_source(&src, &opts).unwrap();
    assert_eq!(report.object_count, 2231);
}
```
(Ensure `source` and `package` items are `pub`/`pub(crate)` as needed; add `pub mod source;`/`pub use` if not already exported.)

- [ ] **Step 2: Run to verify it fails.** FAIL (`convert_source` missing).

- [ ] **Step 3: Implement** by moving the body of `convert` (from `fs::create_dir_all(&opts.output_dir)` through the `Ok(ConvertReport{..})`) into `convert_source(source, opts)`, replacing the `let source = Source::open(&opts.input)?;` line with the `source` parameter. `convert` becomes the two-line wrapper.

- [ ] **Step 4: Run to verify it passes** AND run `cargo test -p cityparquet --test convert_real_data` — every existing convert test must stay green (characterization).

- [ ] **Step 5: `just check`, commit.** `git commit -m "refactor(package): extract convert_source from convert"`

### Task A4: `merge_sources` — CRS check, transform requantise, header merge

**Files:**
- Create: `crates/cityparquet/src/merge.rs`
- Modify: `crates/cityparquet/src/lib.rs` (`pub mod merge;`), `crates/cityparquet/src/order.rs` (`vertices_minmax` → `pub(crate)`)
- Test: `tests/merge_real_data.rs`

**Interfaces:**
- Consumes: `Source` (A2), `order::vertices_minmax`.
- Produces:
  - `pub struct MergedDataset { pub header: CityJSON, pub features: Vec<CityJSONFeature>, pub doc_appearance: Option<cjseq::Appearance>, pub duplicate_ids: usize }`
  - `pub fn merge_sources(sources: &[Source]) -> Result<MergedDataset>`

Rules: CRS (`metadata.reference_system` serialised) must be equal across sources → else `Err`. If every `header.transform` is equal → adopt it, features untouched. Else merged transform `= Transform { translate: vec![0.0,0.0,0.0], scale: componentwise-min of scales }`, and requantise every feature's `vertices`: `real = v[i]*srcScale[i]+srcTranslate[i]; v'[i] = ((real - 0.0)/mergedScale[i]).round() as i64`. Merged header = first source's header with `transform` replaced. Doc-level templates/appearance: if more than one source has non-empty `geometry_templates` OR a doc `appearance` → `Err` (documented single-carrier limit); otherwise carry the sole carrier's through (`doc_appearance` from `source.doc_appearance()`). Duplicate feature ids across the merged set → count into `duplicate_ids` (BTreeSet/HashSet of ids; increment on re-insert) and `eprintln!` a warning once with the count.

- [ ] **Step 1: Write the failing tests** in `tests/merge_real_data.rs`:
```rust
// helper: fixture() as in Global Constraints
#[test]
fn single_source_merge_preserves_transform_and_count() {
    let src = cityparquet::source::Source::open(&fixture("delft.city.jsonl")).unwrap();
    let merged = cityparquet::merge::merge_sources(std::slice::from_ref(&src)).unwrap();
    assert_eq!(merged.header.transform.scale, src.header().transform.scale);
    let n = src.features().unwrap().count();
    assert_eq!(merged.features.len(), n);
}

#[test]
fn heterogeneous_transforms_requantise_to_same_real_coords() {
    // Same fixture opened twice, but the second copy is rewritten with a
    // coarser transform on disk so transforms differ; real coords must match
    // after requantise (compare via vertices_minmax through the MERGED transform).
    // (Implementer: build the second temp file by scaling transform.scale x10
    //  and dividing vertices by 10 — a real re-quantisation of delft.)
    // Assert: merged real bbox ~= original real bbox within 1e-3.
    // NOTE: keep this test minimal — one feature slice is enough.
    todo_placeholder_do_not_ship();
}

#[test]
fn crs_mismatch_is_error() {
    // Two temp CityJSONSeq built from delft's first line, one with a mutated
    // metadata.referenceSystem, must fail to merge.
    todo_placeholder_do_not_ship();
}
```
> Implementer note: replace the two `todo_placeholder_do_not_ship()` bodies with concrete constructions (write temp `.city.jsonl` files derived from `delft.city.jsonl`'s header + a few feature lines, mutating transform/CRS as described). Do NOT hand-author synthetic CityJSON from scratch — derive from the real fixture's lines. This placeholder MUST NOT remain in the committed test.

- [ ] **Step 2: Run to verify it fails.** `cargo test -p cityparquet --test merge_real_data` → FAIL.

- [ ] **Step 3: Implement** `merge.rs` per the rules; make `vertices_minmax` `pub(crate)`.

- [ ] **Step 4: Run to verify it passes.**

- [ ] **Step 5: `just check`, commit.** `git commit -m "feat(merge): merge_sources with requantise + CRS guard"`

### Task A5: CLI — variadic inputs + `-o`, merged single package

**Files:**
- Modify: `crates/cityparquet-cli/src/main.rs`
- Test: `crates/cityparquet-cli/tests/cli.rs` (update existing calls to `-o`; add a multi-input test)

**Interfaces:**
- Consumes: `resolve_inputs` (A1), `merge_sources` (A4), `Source::from_parts` (A2), `convert_source` (A3).

`Convert` struct: `inputs: Vec<PathBuf>` (`#[arg(value_name="INPUTS", required=true, num_args=1..)]`), `output: PathBuf` (`#[arg(short='o', long="output")]`). Handler: `resolve_inputs(&inputs)?` → open each `Source` → if exactly one, `convert_source(&src, &opts)` as today; if more than one, `merge_sources` → `Source::from_parts(merged.header, merged.features, merged.doc_appearance, SourceFormat::CityJsonSeq)` → `convert_source`. `opts.input` set to the first input (cosmetic; unused by `convert_source`). Existing single-input tests updated to pass `-o OUT`.

- [ ] **Step 1: Update existing CLI tests** to the new surface (`convert IN -o OUT`) and **add the failing test**:
```rust
#[test]
fn convert_two_inputs_merges_into_one_package() {
    let out = tempfile::tempdir().unwrap();
    let binary = env!("CARGO_BIN_EXE_cityparquet");
    // delft twice = merged object count doubles (2231*2), single package.
    let status = Command::new(binary)
        .arg("convert").arg(fixture("delft.city.jsonl")).arg(fixture("delft.city.jsonl"))
        .arg("-o").arg(out.path()).arg("--layout").arg("single")
        .output().expect("run");
    assert!(status.status.success());
    let so = String::from_utf8_lossy(&status.stdout);
    assert!(so.contains("4462"), "merged object count 2231*2; got {so}");
    assert!(out.path().join("cityobjects.parquet").exists());
}
```

- [ ] **Step 2: Run to verify it fails** (arg parse changes + merge not wired). `cargo test -p cityparquet-cli --test cli`.

- [ ] **Step 3: Implement** the clap surface change + handler wiring.

- [ ] **Step 4: Run to verify it passes**, plus the full CLI test file.

- [ ] **Step 5: `just check`, commit.** `git commit -m "feat(cli): variadic inputs + -o, merge multiple into one package"`

### M-A close-out

- [ ] `just check` green. Whole-milestone Codex review: `codex exec --cd "$(pwd)" -m sol --sandbox read-only "Review the multi-input merge feature on this branch vs main: crates cityparquet/src/{inputs,merge,source,package}.rs and cityparquet-cli/src/main.rs. Focus on correctness of requantise, CRS guard, de-dup, and the convert_source refactor."`. Triage Critical/Important, fix with TDD.
- [ ] Update milestone memory (M-A done).

---

# Milestone M-B: Spatial partitioning into N packages

Deliverable: `--partition count|features|box` with `--number|--feature-num|--cell-size` writes N self-contained packages under `-o` dir.

### Task B1: `PartitionSpec` + `assign_partitions` (pure)

**Files:**
- Create: `crates/cityparquet/src/partition.rs`
- Modify: `crates/cityparquet/src/lib.rs` (`pub mod partition;`)
- Test: unit tests in `partition.rs`

**Interfaces:**
- Consumes: `order::vertices_minmax` (A4 made it `pub(crate)`), `cjseq::{CityJSONFeature, Transform}`.
- Produces:
  - `pub enum PartitionSpec { Count(usize), Features(usize), Box { cell: f64 } }`
  - `pub fn assign_partitions(features: &[CityJSONFeature], spec: &PartitionSpec, transform: &Transform) -> Vec<(String, Vec<usize>)>` — groups feature indices by partition key label, returned sorted by label. `count`: `floor(i*N/total)` → `count-{:05}`; `features`: `floor(i/M)` → `features-{:05}`; `box`: centroid cell `(floor(cx/cell), floor(cy/cell))` → `box_x{ix}_y{iy}`; vertexless feature → `box_none`. Never drops an index.

- [ ] **Step 1: Write the failing tests** (pure, small synthetic index math is fine here — these are NOT CityJSON, just feature vectors built from a real fixture's features):
```rust
#[cfg(test)]
mod tests {
    use super::*;
    fn load_feats() -> (Vec<cjseq::CityJSONFeature>, cjseq::Transform) {
        let p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/delft.city.jsonl");
        let s = crate::source::Source::open(&p).unwrap();
        (s.features().unwrap().map(|f| f.unwrap()).collect(), s.header().transform.clone())
    }

    #[test]
    fn count_splits_into_contiguous_chunks_covering_all() {
        let (feats, t) = load_feats();
        let groups = assign_partitions(&feats, &PartitionSpec::Count(3), &t);
        assert_eq!(groups.len(), 3);
        let total: usize = groups.iter().map(|(_, v)| v.len()).sum();
        assert_eq!(total, feats.len());
        // contiguous + disjoint
        let mut all: Vec<usize> = groups.iter().flat_map(|(_, v)| v.clone()).collect();
        all.sort();
        assert_eq!(all, (0..feats.len()).collect::<Vec<_>>());
    }

    #[test]
    fn features_caps_each_group() {
        let (feats, t) = load_feats();
        let groups = assign_partitions(&feats, &PartitionSpec::Features(1000), &t);
        assert!(groups.iter().all(|(_, v)| v.len() <= 1000));
        assert_eq!(groups.iter().map(|(_, v)| v.len()).sum::<usize>(), feats.len());
    }

    #[test]
    fn box_groups_are_disjoint_and_complete() {
        let (feats, t) = load_feats();
        let groups = assign_partitions(&feats, &PartitionSpec::Box { cell: 500.0 }, &t);
        assert!(groups.len() > 1, "delft should span >1 500m cell");
        assert_eq!(groups.iter().map(|(_, v)| v.len()).sum::<usize>(), feats.len());
    }
}
```

- [ ] **Step 2: Run to verify it fails.** `cargo test -p cityparquet --lib partition::` → FAIL.

- [ ] **Step 3: Implement** `assign_partitions` using a `BTreeMap<String, Vec<usize>>` (auto-sorted keys), centroid from `vertices_minmax`.

- [ ] **Step 4: Run to verify it passes.**

- [ ] **Step 5: `just check`, commit.** `git commit -m "feat(partition): PartitionSpec + assign_partitions"`

### Task B2: `convert_partitioned` driver

**Files:**
- Modify: `crates/cityparquet/src/partition.rs`
- Test: `tests/partition_real_data.rs`

**Interfaces:**
- Consumes: `merge_sources` (A4), `assign_partitions` (B1), `Source::from_parts` (A2), `convert_source` (A3), `ConvertOptions`.
- Produces:
  - `pub struct PartitionReport { pub partitions: Vec<(String, crate::package::ConvertReport)>, pub duplicate_ids: usize }`
  - `pub fn convert_partitioned(sources: &[Source], spec: &PartitionSpec, opts: &ConvertOptions) -> Result<PartitionReport>` — `merge_sources` → `assign_partitions` → for each `(label, idxs)`: clone the merged features at `idxs` into a `Vec`, `Source::from_parts(merged.header.clone(), subset, merged.doc_appearance.clone(), CityJsonSeq)`, clone `opts` with `output_dir = opts.output_dir.join(&label)`, `convert_source`. Sequential. Collect reports.

- [ ] **Step 1: Write the failing test** in `tests/partition_real_data.rs`:
```rust
#[test]
fn partitioned_convert_is_lossless_over_delft() {
    let out = tempfile::tempdir().unwrap();
    let src = cityparquet::source::Source::open(&fixture("delft.city.jsonl")).unwrap();
    let mut opts = cityparquet::package::ConvertOptions::new(fixture("delft.city.jsonl"), out.path().to_path_buf());
    opts.layout = cityparquet::package::TableLayout::Single;
    let spec = cityparquet::partition::PartitionSpec::Count(4);
    let rep = cityparquet::partition::convert_partitioned(std::slice::from_ref(&src), &spec, &opts).unwrap();
    assert_eq!(rep.partitions.len(), 4);
    // completeness: union of per-partition object counts == single-package count
    let total: usize = rep.partitions.iter().map(|(_, r)| r.object_count).sum();
    assert_eq!(total, 2231);
    for (label, _) in &rep.partitions {
        assert!(out.path().join(label).join("metadata.json").exists(), "{label} package incomplete");
    }
}
```

- [ ] **Step 2: Run to verify it fails.** FAIL (`convert_partitioned` missing).

- [ ] **Step 3: Implement** the driver.

- [ ] **Step 4: Run to verify it passes.**

- [ ] **Step 5: `just check`, commit.** `git commit -m "feat(partition): convert_partitioned driver"`

### Task B3: CLI — `--partition` flags + validation + summary

**Files:**
- Modify: `crates/cityparquet-cli/src/main.rs`
- Test: `crates/cityparquet-cli/tests/cli.rs`

**Interfaces:**
- Consumes: `convert_partitioned` (B2), `PartitionSpec` (B1).

Add to `Convert`: `partition: Option<String>`, `number: Option<usize>`, `feature_num: Option<usize>`, `cell_size: Option<f64>`. Build `PartitionSpec`: `count`→require `--number`; `features`→require `--feature-num`; `box`→require `--cell-size`; unknown method or wrong/missing sizing flag → error+FAILURE. A sizing flag without `--partition` → error. `Some(spec)` → `convert_partitioned` (open all sources first) and print `"<K> partitions written"` + per-label object counts; `None` → the M-A single/merged path.

- [ ] **Step 1: Write the failing test:**
```rust
#[test]
fn partition_count_writes_n_package_dirs() {
    let out = tempfile::tempdir().unwrap();
    let binary = env!("CARGO_BIN_EXE_cityparquet");
    let o = Command::new(binary)
        .arg("convert").arg(fixture("delft.city.jsonl"))
        .arg("-o").arg(out.path())
        .arg("--partition").arg("count").arg("--number").arg("3")
        .arg("--layout").arg("single")
        .output().expect("run");
    assert!(o.status.success(), "stderr: {}", String::from_utf8_lossy(&o.stderr));
    for i in 0..3 {
        assert!(out.path().join(format!("count-{i:05}")).join("cityobjects.parquet").exists());
    }
}

#[test]
fn partition_box_requires_cell_size() {
    let out = tempfile::tempdir().unwrap();
    let binary = env!("CARGO_BIN_EXE_cityparquet");
    let o = Command::new(binary)
        .arg("convert").arg(fixture("delft.city.jsonl")).arg("-o").arg(out.path())
        .arg("--partition").arg("box") // no --cell-size
        .output().expect("run");
    assert!(!o.status.success(), "box without --cell-size must fail");
}
```

- [ ] **Step 2: Run to verify it fails.**

- [ ] **Step 3: Implement** the flag parsing/validation + summary print.

- [ ] **Step 4: Run to verify it passes**, plus full CLI suite.

- [ ] **Step 5: `just check`, commit.** `git commit -m "feat(cli): --partition count/features/box"`

### Task B4: End-to-end per-partition round-trip (box)

**Files:**
- Modify: `tests/partition_real_data.rs`

- [ ] **Step 1: Write the failing test** — convert delft with `box cell=1000`, then for each partition package `export` back to CityJSONSeq and `compare` it clean (semantic equality of each self-contained package):
```rust
#[test]
fn box_partitions_each_round_trip_clean() {
    let out = tempfile::tempdir().unwrap();
    let src = cityparquet::source::Source::open(&fixture("delft.city.jsonl")).unwrap();
    let mut opts = cityparquet::package::ConvertOptions::new(fixture("delft.city.jsonl"), out.path().to_path_buf());
    opts.layout = cityparquet::package::TableLayout::Single;
    let spec = cityparquet::partition::PartitionSpec::Box { cell: 1000.0 };
    let rep = cityparquet::partition::convert_partitioned(std::slice::from_ref(&src), &spec, &opts).unwrap();
    for (label, _) in &rep.partitions {
        let pkg = out.path().join(label);
        let dst = out.path().join(format!("{label}.city.jsonl"));
        cityparquet::export::export(&cityparquet::export::ExportOptions {
            package_dir: pkg.clone(), output: dst.clone(),
        }).unwrap();
        let rc = cityparquet::compare::compare_datasets(&pkg, &dst, &cityparquet::compare::CompareOptions {
            coord_tolerance: [0.0;3],
            exclusions: cityparquet::compare::Exclusions { appearance:false, geometry_instances:false },
        }).unwrap();
        assert!(rc.equal, "partition {label} not round-trip clean: {:?}", rc.differences);
    }
}
```
> Implementer: adapt the `compare_datasets` first argument to whatever the existing exported-package-vs-source comparison expects (see how `tests/convert_real_data.rs`/`bench` round-trips); the intent is "each partition package is independently valid + round-trips". If comparing a package dir directly is unsupported, export both the partition package and re-encode its own feature subset and compare those.

- [ ] **Step 2–4:** run → (implement any support needed) → pass.

- [ ] **Step 5: `just check`, commit.** `git commit -m "test(partition): per-partition export+compare round-trip"`

### M-B close-out

- [ ] `just check` green. Codex review with `sol`: `codex exec --cd "$(pwd)" -m sol --sandbox read-only "Review the partitioning feature on this branch: cityparquet/src/partition.rs + the CLI --partition wiring. Focus on completeness (no feature lost/duplicated across partitions), box cell math incl. negatives + vertexless, and per-partition package self-containment."`. Triage + fix.
- [ ] Update milestone memory (M-B done; H3/S2 + reprojection = next follow-up).
- [ ] `superpowers:finishing-a-development-branch`: `just check`, present merge options.

## Self-Review

- **Spec coverage:** multi-input (A1,A2,A4,A5), CRS guard (A4), requantise (A4), convert_source refactor (A3), count/features/box (B1), N packages + per-partition metadata (B2), CLI flags+validation (B3), completeness + round-trip tests (B2,B4). H3/S2 correctly absent (deferred).
- **Type consistency:** `resolve_inputs`, `Source::from_parts`, `convert_source`, `merge_sources`→`MergedDataset`, `assign_partitions`→`Vec<(String,Vec<usize>)>`, `convert_partitioned`→`PartitionReport` used consistently across tasks.
- **Placeholder honesty:** A4/B4 contain explicit "replace this placeholder / adapt to existing API" notes for constructions that depend on the real fixture and existing compare API the implementer must read first — flagged, not silently vague. Every other step ships real code.
- **TDD:** every task is failing-test → implement → pass → `just check` → commit.
