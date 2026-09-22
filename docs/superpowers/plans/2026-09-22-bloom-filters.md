# Bloom Filters Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Write standard Parquet bloom filters on every object table's `id`, `feature_id` and high-cardinality string attribute columns (default on, `+nobloom` / `--no-bloom` off), use them to prune row groups in `id_lookup`, a new `feature_lookup` and string-equality `attr_filter` (sync and async, the async path fetching every filter in one coalesced range request), write the same policy from duckdb-cityjson, specify it, and measure it as a one-pair `bloom` benchmark family.

**Architecture:** parquet-rs does all sizing, hashing, folding, serialisation and metadata (`set_column_bloom_filter_*`, `BloomFilterPosition::End`, `Sbbf`). The writer's column set is decided in two places that never drift: `WriterRecipe::writer_properties` (the fixed `id`/`feature_id` rule and the policy switch) and `scan` (a HyperLogLog distinct-count per scalar string attribute, turned into `ScanResult::bloom_attributes`, canonicalised across partitions). The reader prunes with free functions that run on the builder's public metadata before a builder is configured: a pure `bloom_targets` shared by a sync `bloom_keep_row_groups` (local `pread`s) and an async `bloom_keep_row_groups_async` (one `AsyncFileReader::get_byte_ranges` call). Lookups read the surviving row groups one builder per row group (built from shared `ArrowReaderMetadata`), which is what makes `LookupStats::row_groups_read` exact. The benchmark harness gains lookup counters in its CSV, a `feature-lookup` scenario and a `bloom` family on the existing `--variants` machinery.

**Tech Stack:** Rust 2024 (toolchain 1.93.1), `parquet`/`arrow` 58 (58.3.0 resolved), `object_store` 0.13, `cardinality-estimator` 1.0.3, tokio; C++20 DuckDB extension (DuckDB v1.5.4) with sqllogictest; Python 3 (`uv`, pytest, matplotlib) for `benchviz`; bash for the recipe tests; Blume/MDX for the specification site.

**Spec:** docs/superpowers/specs/2026-09-22-bloom-filters-design.md

## Global Constraints

- British English in all prose: comments, doc comments, Markdown, MDX, commit messages.
- Breaking changes are welcome: no shims, no deprecation paths, no legacy branches — update every call site.
- Document the present, never the past: no "now", "previously", "fixed", "no longer" in docs or doc comments.
- Benchmark caveats are part of the artefact: never drop or soften one to make a number look better; every new caveat is written down.
- Generated benchmark inputs, packages, results and summaries go under `benchmark/runs/` only (and never into another user's directory).
- Each level keeps `AGENTS.md` byte-identical to its `CLAUDE.md`; this plan edits neither, so do not let an edit drift them.
- Parquet column paths are built with `ColumnPath::new(vec![..])`, never `ColumnPath::from` a dotted string (which does not split and silently matches nothing for nested paths).
- Bloom-filter FPP default is 0.01; NDV is never set (parquet-rs resolves it to the row-group row count and folds).
- Commit in the repository that owns the change: `lib/duckdb-cityjson` is a submodule — commit there first, then bump the pointer in the monorepo.
- Work on `develop` in the monorepo (already checked out). In `lib/duckdb-cityjson` (detached at `origin/develop`), create a local `develop` before editing (Task 6).
- `benchmark/readbench` is its own Cargo workspace: build/test with `--manifest-path benchmark/readbench/Cargo.toml`; its `fcb_core = "=0.7.6"` / `cjseq2 = "=0.1.0"` pins stay exact and its `[patch.crates-io] cjseq` line stays.
- Tests read real fixtures (`lib/cityparquet-rs/tests/fixtures/`, `just fixtures`), never inline hand-written CityJSON. Corpus-scale checks are `#[ignore]` tests driven by environment variables, so every gate stays corpus-free.
- The pre-commit hook (`just hooks` once, from the repository root) formats Rust and Markdown; never pass `--no-verify`.
- Markdown in the monorepo is formatted by `npx --yes prettier@3.9.6 --write <file>`; the hook does it for staged files.
- Editing `documents/docs/**` or `lib/duckdb-cityjson/docs/FUNCTIONS.md` changes the MCP corpus: regenerate with `just mcp-corpus` and commit `ai/mcp/corpus/` in the same commit, or `just mcp-check` fails.
- Strict red-green TDD: every step that adds behaviour first adds a test and runs it to watch it fail for the stated reason.

## Where this plan resolves something the spec leaves open

These are decisions the implementer must not re-open; each is reported to the author alongside the plan.

1. **HLL crate: `cardinality-estimator` 1.0.3** (Cloudflare; released 2026-02-11; Apache-2.0, compatible with the workspace's `MIT OR Apache-2.0`; default hasher WyHash with a fixed seed, so `bloom_attributes` — and therefore the package layout — is reproducible run to run; exact counting below 129 distinct values, HLL++ with ~1.6 % standard error at the default `P = 12`, ≤ 3 KiB per column). Rejected: `hyperloglogplus` 0.4.1 (MIT, no `unsafe`, but unreleased since 2022-06, and its `new(precision, builder)` invites a `RandomState` hasher, which would make the selected column set vary between runs). Disclosed constraint: `CardinalityEstimator` exists only on 64-bit targets (`#[cfg(target_pointer_width = "64")]`) and uses `unsafe` internally for its tagged-pointer representation.
2. **Presets.** The spec says "`NoDictionary` and `CityParquet` honour the policy". In `recipe.rs` every preset except `ParquetDefaults` runs the same per-column code after the `ParquetDefaults` early return, so `NoByteStreamSplit`, `NoDelta` and `Snappy` honour it too. This plan implements that (no preset-specific opt-out).
3. **Partitioned conversion.** `bloom_attributes` must be dataset-wide, so it joins `CanonicalSchema` (package.rs:1243) and is stamped into every partition's scan (package.rs:1340-1363, partition.rs:402-411). `ScanResult::add_synthesized_lod0_column` diverts attributes after `scan`, so it also drops them from `bloom_attributes`, and `writer_properties` only honours names that are `String` attributes of the schema it renders.
4. **`row_groups_read` "counted in the stream".** A batch carries no row-group index, so each kept row group is read by its own builder, all built from one `ArrowReaderMetadata` (`new_with_metadata`); a hit stops before the next builder. This changes the lookup's read pattern (equally for both variants) and is disclosed in the benchmark caveats.
5. **No CLI lookup.** `cityparquet` has `convert`/`export`/`compare`/`bench` only (`crates/cli/src/main.rs:24`), so no `lookup --feature-id` form is added.
6. **The async gate.** `query_async` is behind the `object-store` feature and `cargo test --workspace` does not enable it, so its tests never run under `cd lib/cityparquet-rs && just check` today (verified: 0 tests without the feature, 7 with it). Task 4 makes the library gate's `test` and `lint` recipes use `--all-features` (clippy with `--all-features` is clean at the plan's base commit).
7. **duckdb-cityjson sidecars.** The spec keeps "Sidecar `COPY`s unchanged", but DuckDB's default writes filters on any dictionary-encoded chunk, so a sidecar could carry one and fail Acceptance 4c ("none on sidecars"). Task 6 adds `WRITE_BLOOM_FILTER false` to the sidecar `COPY`. Its row-group size is stated explicitly as DuckDB's default, 122 880 rows, and `DICTIONARY_SIZE_LIMIT` equals it.
8. **Acceptance 1 for duckdb-cityjson (4c)** is read as: filters present on at least `id`, `feature_id` and the qualifying attributes (a documented superset), every offset past the data, lengths declared, none on sidecars.
9. **HTTP for the bloom family.** `--variants` runs are local-only (coordinator.rs:217-219), yet the spec lists HTTP latency and requests as metrics. Task 10 lets a `--variants` run read already-written, uploaded variant packages over `--transport http` (no write children), mirroring how the format comparison already runs over HTTP against an uploaded prepared directory.
10. **CSV contract.** Lookup counters become four appended CSV columns, `row_groups_total,bloom_pruned,row_groups_read,filter_bytes`, empty on every row that is not a CityParquet lookup. The child reports them on stderr (a marker line), so the timed stdout protocol keeps its 4/6-field shape.

---

### Task 1: Writer bloom policy, `+nobloom` and the CLI flags

Filters on `id` and `feature_id` in every object table, all placed after the last row group, switchable per recipe, per variant and per CLI run. Attribute columns come in Task 2.

**Files:**

- Modify: `lib/cityparquet-rs/crates/core/src/recipe.rs` — imports (lines 13-15); new `BloomPolicy` after `impl Codec` (after line 67); `RecipePreset::recipe` doc (147-148); `WriterRecipe` fields (157-191) and `Default` (193-203); `writer_properties` body, a new block after the `id`/`feature_id` block (after line 343); tests module (404-771).
- Modify: `lib/cityparquet-rs/crates/core/src/variant.rs` — module doc (1-13), `GRAMMAR` (23), `Variant` (26-34), `parse` (37-101), `id` (103-121), `recipe` (123-137), `grammar_err` (158-170), tests (172-305).
- Modify: `lib/cityparquet-rs/crates/cli/src/main.rs` — imports (line 8), `Commands::Convert` fields (after line 88), destructuring (377-395), recipe construction (420-426).
- Modify: `lib/cityparquet-rs/crates/cli/tests/cli.rs` — two new tests after `convert_with_an_invalid_compression_fails` (ends line 612).
- Create: `lib/cityparquet-rs/crates/core/tests/bloom_real_data.rs`.
- Modify: `lib/cityparquet-rs/README.md` — convert flag table (lines 81-91).

**Interfaces:**

- Consumes: nothing from earlier tasks.
- Produces:
  - `pub struct BloomPolicy { pub enabled: bool, pub fpp: f64 }` in `cityparquet::recipe`, `#[derive(Debug, Clone, Copy, PartialEq)]`, `impl Default` = `{ enabled: true, fpp: 0.01 }`, `pub const DEFAULT_FPP: f64 = 0.01`, `pub fn validate(&self) -> cityparquet_schema::Result<()>`.
  - `WriterRecipe` gains `pub bloom: BloomPolicy`.
  - `Variant` gains `pub bloom: bool` (`true` unless `+nobloom`); `GRAMMAR = "<preset>[+hilbert][+rg<N>][+<codec>[<level>]][+nobloom]"`; canonical id appends `+nobloom` last.
  - CLI `convert --no-bloom` and `--bloom-fpp <f64>` (default 0.01).
  - Test helpers in `crates/core/tests/bloom_real_data.rs`: `fn filtered_columns(table: &Path) -> BTreeMap<String, usize>`, `fn assert_filters_follow_the_data(table: &Path)`, `fn convert_with(fixture_name: &str, recipe: WriterRecipe) -> tempfile::TempDir`.

- [ ] **Step 1: Write the failing recipe unit tests**

In `recipe.rs`, inside `mod tests`, extend the imports:

```rust
    use cityparquet_schema::{AttributeType, Lod};
    use parquet::basic::Compression;
    use parquet::file::properties::BloomFilterPosition;
```

and append these tests at the end of the module:

```rust
    #[test]
    fn the_default_recipe_filters_id_and_feature_id_after_the_last_row_group() {
        let props = WriterRecipe::default()
            .writer_properties(&sample_schema())
            .unwrap();
        for name in ["id", "feature_id"] {
            let bloom = props
                .bloom_filter_properties(&ColumnPath::new(vec![name.to_string()]))
                .unwrap_or_else(|| panic!("{name} must carry a bloom filter"));
            assert_eq!(bloom.fpp, 0.01, "{name}");
            // NDV is left to parquet-rs, which resolves it to the row-group
            // row count.
            assert_eq!(bloom.ndv, 65536, "{name}");
        }
        assert_eq!(props.bloom_filter_position(), BloomFilterPosition::End);
        for name in ["object_type", "yoc", "other", "geometry_lod2_2"] {
            assert!(
                props
                    .bloom_filter_properties(&ColumnPath::new(vec![name.to_string()]))
                    .is_none(),
                "{name} must not carry a bloom filter"
            );
        }
        let bbox_xmin = ColumnPath::new(vec!["bbox".to_string(), "xmin".to_string()]);
        assert!(props.bloom_filter_properties(&bbox_xmin).is_none());
    }

    #[test]
    fn a_disabled_policy_and_parquet_defaults_write_no_filter() {
        let disabled = WriterRecipe {
            bloom: BloomPolicy {
                enabled: false,
                ..BloomPolicy::default()
            },
            ..WriterRecipe::default()
        };
        let parquet_defaults = RecipePreset::ParquetDefaults.recipe();
        for recipe in [disabled, parquet_defaults] {
            let props = recipe.writer_properties(&sample_schema()).unwrap();
            for name in ["id", "feature_id"] {
                assert!(
                    props
                        .bloom_filter_properties(&ColumnPath::new(vec![name.to_string()]))
                        .is_none(),
                    "{:?} must not filter {name}",
                    recipe.preset
                );
            }
        }
    }

    #[test]
    fn every_tuned_preset_honours_the_policy() {
        for preset in RecipePreset::ALL {
            let props = preset
                .recipe()
                .writer_properties(&sample_schema())
                .unwrap();
            let filtered = props
                .bloom_filter_properties(&ColumnPath::new(vec!["id".to_string()]))
                .is_some();
            assert_eq!(
                filtered,
                preset != RecipePreset::ParquetDefaults,
                "{preset:?}"
            );
        }
    }

    #[test]
    fn the_fpp_reaches_the_properties_and_an_out_of_range_fpp_is_an_error() {
        let recipe = WriterRecipe {
            bloom: BloomPolicy {
                enabled: true,
                fpp: 0.05,
            },
            ..WriterRecipe::default()
        };
        let props = recipe.writer_properties(&sample_schema()).unwrap();
        assert_eq!(
            props
                .bloom_filter_properties(&ColumnPath::new(vec!["id".to_string()]))
                .unwrap()
                .fpp,
            0.05
        );
        for fpp in [0.0, 1.0, 1.5, -0.1, f64::NAN] {
            let recipe = WriterRecipe {
                bloom: BloomPolicy { enabled: true, fpp },
                ..WriterRecipe::default()
            };
            let err = recipe
                .writer_properties(&sample_schema())
                .unwrap_err()
                .to_string();
            assert!(err.contains("strictly between 0 and 1"), "{fpp}: {err}");
        }
    }
```

- [ ] **Step 2: Run them and watch them fail to compile**

Run: `cd lib/cityparquet-rs && cargo test -p cityparquet --lib recipe::tests`
Expected: compile error — `cannot find type BloomPolicy in this scope` and `no field bloom on type WriterRecipe`.

- [ ] **Step 3: Add `BloomPolicy` and the field**

In `recipe.rs` replace line 14:

```rust
use parquet::file::properties::{EnabledStatistics, WriterProperties};
```

with:

```rust
use parquet::file::properties::{BloomFilterPosition, EnabledStatistics, WriterProperties};
```

Directly after the closing brace of `impl Codec` (line 67), insert:

```rust
/// Parquet bloom filters on the object tables (specification "Physical
/// encoding and conformance"; design decision G). Which columns carry one is
/// decided by [`WriterRecipe::writer_properties`]; this switches them on and
/// sets their target false-positive probability.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BloomPolicy {
    pub enabled: bool,
    /// Target false-positive probability of every filter written, strictly
    /// between 0 and 1. parquet-rs sizes each filter for the row-group row
    /// count and folds it down to the values actually present.
    pub fpp: f64,
}

impl BloomPolicy {
    /// The default target false-positive probability.
    pub const DEFAULT_FPP: f64 = 0.01;

    /// Rejects an `fpp` outside the open interval (0, 1) with a clear error,
    /// where parquet-rs itself would panic.
    pub fn validate(&self) -> Result<()> {
        if self.fpp > 0.0 && self.fpp < 1.0 {
            Ok(())
        } else {
            Err(CityParquetError::Schema(format!(
                "invalid bloom-filter false-positive probability {}: expected a value \
                 strictly between 0 and 1",
                self.fpp
            )))
        }
    }
}

impl Default for BloomPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            fpp: Self::DEFAULT_FPP,
        }
    }
}
```

Replace the `RecipePreset::recipe` doc comment (lines 147-148):

```rust
    /// The default [`WriterRecipe`] for this preset (row-group size 65536,
    /// zstd level 3 — ignored by `Snappy` — `statistics_for_json` off).
```

with:

```rust
    /// The default [`WriterRecipe`] for this preset (row-group size 65536,
    /// zstd level 3 — ignored by `Snappy` — `statistics_for_json` off, bloom
    /// filters on at FPP 0.01).
```

In `pub struct WriterRecipe`, after the `compression` field (line 190), add:

```rust
    /// Bloom filters on `id`, `feature_id` and the dataset's high-cardinality
    /// string attributes. Honoured by every preset except
    /// [`RecipePreset::ParquetDefaults`], which applies no per-column tuning.
    pub bloom: BloomPolicy,
```

and in `impl Default for WriterRecipe`, after `compression: None,` add `bloom: BloomPolicy::default(),`.

- [ ] **Step 4: Render the policy in `writer_properties`**

Directly after the closing brace of the `if self.preset != RecipePreset::NoDelta { ... }` block (line 343), insert:

```rust
        // Bloom filters: `id` and `feature_id` in every object table — the
        // two identifier columns whose min/max statistics cannot prune. Every
        // filter goes after the last row group (`BloomFilterPosition::End`),
        // where one coalesced range read reaches them all. NDV is left to
        // parquet-rs, which resolves it to the row-group row count and folds
        // each filter down to the values actually present. Placed after the
        // `ParquetDefaults` early return, so that preset writes none.
        if self.bloom.enabled {
            self.bloom.validate()?;
            builder = builder.set_bloom_filter_position(BloomFilterPosition::End);
            for name in ["id", "feature_id"] {
                let path = ColumnPath::new(vec![name.to_string()]);
                builder = builder
                    .set_column_bloom_filter_enabled(path.clone(), true)
                    .set_column_bloom_filter_fpp(path, self.bloom.fpp);
            }
        }
```

- [ ] **Step 5: Run the recipe tests**

Run: `cd lib/cityparquet-rs && cargo test -p cityparquet --lib recipe::tests`
Expected: compile error in `crates/cli/src/main.rs` is NOT reached (lib only); all `recipe::tests` pass, including the four new ones.

- [ ] **Step 6: Write the failing variant tests**

In `variant.rs` `mod tests`, add after `ROWGROUP_LIST`:

```rust
    const BLOOM_LIST: [&str; 2] = ["cityparquet", "cityparquet+nobloom"];

    #[test]
    fn nobloom_round_trips_and_switches_the_recipe_policy_off() {
        for id in BLOOM_LIST {
            let v = Variant::parse(id).unwrap();
            assert_eq!(v.id(), id, "canonical spelling must be the list's spelling");
            assert_eq!(Variant::parse(&v.id()).unwrap(), v);
        }
        let off = Variant::parse("cityparquet+nobloom").unwrap();
        assert!(!off.bloom);
        assert!(!off.recipe().bloom.enabled);
        let on = Variant::parse("cityparquet").unwrap();
        assert!(on.bloom);
        assert!(on.recipe().bloom.enabled);

        let mixed = Variant::parse("cityparquet+nobloom+rg512+hilbert+zstd9").unwrap();
        assert_eq!(mixed.id(), "cityparquet+hilbert+rg512+zstd9+nobloom");
        assert_eq!(Variant::parse(&mixed.id()).unwrap(), mixed);
    }
```

and in `duplicates_malformed_suffixes_and_unknown_presets_are_rejected_with_the_grammar`, add `"cityparquet+nobloom+nobloom"` and `"cityparquet+bloom"` to the list of ids.

- [ ] **Step 7: Run and watch them fail**

Run: `cd lib/cityparquet-rs && cargo test -p cityparquet --lib variant::tests`
Expected: compile error — `no field bloom on type Variant`.

- [ ] **Step 8: Implement the token**

In `variant.rs`, replace the module doc's first paragraph (lines 4-7):

```rust
//! An id is `<preset>[+hilbert][+rg<N>][+<codec>[<level>]]`: a
//! [`RecipePreset`] name, then any of three suffixes, each at most once, in
//! any order on input. [`Variant::id`] spells the same variant back in the
//! fixed order above, and that spelling is what the result CSVs carry.
```

with:

```rust
//! An id is `<preset>[+hilbert][+rg<N>][+<codec>[<level>]][+nobloom]`: a
//! [`RecipePreset`] name, then any of four suffixes, each at most once, in
//! any order on input. [`Variant::id`] spells the same variant back in the
//! fixed order above, and that spelling is what the result CSVs carry.
//! `+nobloom` writes no Parquet bloom filter; without it the recipe's
//! default bloom policy applies.
```

Replace line 23:

```rust
pub const GRAMMAR: &str = "<preset>[+hilbert][+rg<N>][+<codec>[<level>]]";
```

with:

```rust
pub const GRAMMAR: &str = "<preset>[+hilbert][+rg<N>][+<codec>[<level>]][+nobloom]";
```

In `pub struct Variant`, after `zstd_level`, add:

```rust
    /// `false` only for `+nobloom`.
    pub bloom: bool,
```

In `parse`, after `let mut seen_hilbert = false;` add `let mut bloom = true;`, and replace the final `match part { ... }` with:

```rust
            match part {
                "hilbert" if !seen_hilbert => {
                    seen_hilbert = true;
                    ordering = RowOrder::Hilbert;
                }
                "nobloom" if bloom => {
                    bloom = false;
                }
                _ => return Err(grammar_err(id, None)),
            }
```

and add `bloom,` to the returned `Variant { .. }`.

In `id`, before the final `id` expression, add:

```rust
        if !self.bloom {
            id.push_str("+nobloom");
        }
```

and update its doc comment to `/// The canonical spelling: preset, then `+hilbert`, `+rg<N>`, `+<codec>[<level>]`, `+nobloom`.`

In `recipe`, before the final `recipe`, add:

```rust
        recipe.bloom.enabled = self.bloom;
```

and extend its doc to `/// The preset's recipe with this variant's row-group size, codec, zstd level and bloom switch applied on top.`

- [ ] **Step 9: Run the variant tests**

Run: `cd lib/cityparquet-rs && cargo test -p cityparquet --lib variant::tests`
Expected: all pass (the existing `a_zstd_level_reaches_the_recipe...` still asserts `default.recipe() == WriterRecipe::default()`).

- [ ] **Step 10: Write the failing package-level test**

Create `lib/cityparquet-rs/crates/core/tests/bloom_real_data.rs`:

```rust
//! Bloom filters in written packages, read back from the Parquet footer:
//! which columns carry one, where the filters sit, and which files carry
//! none. Real fixtures only.

use std::collections::BTreeMap;
use std::fs::File;
use std::path::{Path, PathBuf};

use cityparquet::package::{ConvertOptions, convert};
use cityparquet::recipe::{BloomPolicy, RecipePreset, WriterRecipe};
use cityparquet::stac::properties::PackageTables;
use parquet::file::reader::{FileReader, SerializedFileReader};

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

/// Converts `fixture_name` with `recipe` (source order, LoD0 synthesis on)
/// into a fresh temporary package.
fn convert_with(fixture_name: &str, recipe: WriterRecipe) -> tempfile::TempDir {
    let out = tempfile::tempdir().unwrap();
    let mut opts = ConvertOptions::new(fixture(fixture_name), out.path().to_path_buf());
    opts.recipe = recipe;
    convert(&opts).unwrap();
    out
}

/// Dotted column path -> number of row groups whose chunk declares a filter.
fn filtered_columns(table: &Path) -> BTreeMap<String, usize> {
    let reader = SerializedFileReader::new(File::open(table).unwrap()).unwrap();
    let mut out = BTreeMap::new();
    for rg in reader.metadata().row_groups() {
        for chunk in rg.columns() {
            if chunk.bloom_filter_offset().is_some() {
                *out.entry(chunk.column_path().string()).or_insert(0) += 1;
            }
        }
    }
    out
}

fn row_group_count(table: &Path) -> usize {
    SerializedFileReader::new(File::open(table).unwrap())
        .unwrap()
        .metadata()
        .num_row_groups()
}

/// Every filter starts at or after the last byte of column data, and
/// declares its length.
fn assert_filters_follow_the_data(table: &Path) {
    let reader = SerializedFileReader::new(File::open(table).unwrap()).unwrap();
    let meta = reader.metadata();
    let data_end = meta
        .row_groups()
        .iter()
        .flat_map(|rg| rg.columns())
        .map(|c| {
            let (start, len) = c.byte_range();
            start + len
        })
        .max()
        .expect("a table has at least one column chunk");
    for rg in meta.row_groups() {
        for chunk in rg.columns() {
            if let Some(offset) = chunk.bloom_filter_offset() {
                assert!(
                    offset as u64 >= data_end,
                    "{}: filter of {} at {offset} precedes the end of data {data_end}",
                    table.display(),
                    chunk.column_path().string()
                );
                assert!(
                    chunk.bloom_filter_length().is_some(),
                    "{}: filter of {} declares no length",
                    table.display(),
                    chunk.column_path().string()
                );
            }
        }
    }
}

fn delft_recipe(row_group_size: usize) -> WriterRecipe {
    WriterRecipe {
        row_group_size,
        ..WriterRecipe::default()
    }
}

#[test]
fn the_default_recipe_filters_the_identifiers_in_every_row_group() {
    let out = convert_with("delft.city.jsonl", delft_recipe(512));
    let table = out.path().join("building.parquet");
    let groups = row_group_count(&table);
    assert_eq!(groups, 5, "2231 rows at 512 per group");
    let expected: BTreeMap<String, usize> = [("feature_id", groups), ("id", groups)]
        .into_iter()
        .map(|(name, n)| (name.to_string(), n))
        .collect();
    assert_eq!(filtered_columns(&table), expected);
    assert_filters_follow_the_data(&table);
}

#[test]
fn nobloom_and_parquet_defaults_write_no_filter() {
    let nobloom = WriterRecipe {
        bloom: BloomPolicy {
            enabled: false,
            ..BloomPolicy::default()
        },
        ..delft_recipe(512)
    };
    let parquet_defaults = WriterRecipe {
        row_group_size: 512,
        ..RecipePreset::ParquetDefaults.recipe()
    };
    for recipe in [nobloom, parquet_defaults] {
        let out = convert_with("delft.city.jsonl", recipe);
        assert!(
            filtered_columns(&out.path().join("building.parquet")).is_empty(),
            "{recipe:?}"
        );
    }
}

#[test]
fn sidecars_carry_no_filter_and_every_object_table_filters_its_identifiers() {
    let out = convert_with("lod3_railway.city.json", WriterRecipe::default());
    let tables = PackageTables::open(out.path()).unwrap();
    assert!(
        !tables.sidecar_files.is_empty(),
        "railway must write sidecars, or this test proves nothing"
    );
    for sidecar in &tables.sidecar_files {
        let path = out.path().join(sidecar);
        assert!(
            filtered_columns(&path).is_empty(),
            "sidecar {sidecar} carries a filter"
        );
    }
    for table in &tables.tables {
        let filtered = filtered_columns(table);
        for name in ["id", "feature_id"] {
            assert_eq!(
                filtered.get(name).copied(),
                Some(row_group_count(table)),
                "{}: {name}",
                table.display()
            );
        }
        assert_filters_follow_the_data(table);
    }
}
```

- [ ] **Step 11: Run it**

Run: `cd lib/cityparquet-rs && cargo test -p cityparquet --test bloom_real_data`
Expected: all three pass (the writer change is already in; this pins it on real packages). If `row_group_count` is not 5, the fixture changed — stop and investigate rather than editing the number.

- [ ] **Step 12: Write the failing CLI tests**

In `crates/cli/tests/cli.rs`, after `convert_with_an_invalid_compression_fails`, add:

```rust
/// Dotted paths of every column that declares a bloom filter in `table`.
fn bloom_filtered_columns(table: &std::path::Path) -> std::collections::BTreeSet<String> {
    use parquet::file::reader::{FileReader, SerializedFileReader};
    let reader = SerializedFileReader::new(std::fs::File::open(table).unwrap()).unwrap();
    reader
        .metadata()
        .row_groups()
        .iter()
        .flat_map(|rg| rg.columns())
        .filter(|c| c.bloom_filter_offset().is_some())
        .map(|c| c.column_path().string())
        .collect()
}

/// The default convert writes bloom filters on the identifiers; `--no-bloom`
/// writes none at all.
#[test]
fn convert_writes_bloom_filters_by_default_and_no_bloom_suppresses_them() {
    let binary = env!("CARGO_BIN_EXE_cityparquet");
    let filtered = |extra: &[&str]| {
        let out = tempfile::tempdir().unwrap();
        let status = Command::new(binary)
            .arg("convert")
            .arg(fixture("delft.city.jsonl"))
            .arg("-o")
            .arg(out.path())
            .args(extra)
            .status()
            .expect("failed to run convert");
        assert!(status.success(), "convert {extra:?} failed");
        bloom_filtered_columns(&out.path().join("building.parquet"))
    };
    let default = filtered(&[]);
    assert!(
        default.contains("id") && default.contains("feature_id"),
        "default filters: {default:?}"
    );
    let off = filtered(&["--no-bloom"]);
    assert!(off.is_empty(), "--no-bloom filters: {off:?}");
}

/// `--bloom-fpp` outside (0, 1) is refused with a clear error before any
/// conversion, not a parquet-rs panic.
#[test]
fn convert_rejects_a_bloom_fpp_outside_the_open_unit_interval() {
    let binary = env!("CARGO_BIN_EXE_cityparquet");
    for bad in ["0", "1", "1.5", "-0.1", "NaN"] {
        let out = tempfile::tempdir().unwrap();
        let output = Command::new(binary)
            .arg("convert")
            .arg(fixture("delft.city.jsonl"))
            .arg("-o")
            .arg(out.path())
            .arg(format!("--bloom-fpp={bad}"))
            .output()
            .expect("failed to run convert");
        assert!(!output.status.success(), "--bloom-fpp={bad} was accepted");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("--bloom-fpp") && stderr.contains("strictly between 0 and 1"),
            "--bloom-fpp={bad}: {stderr}"
        );
        assert!(
            !stderr.contains("panicked"),
            "--bloom-fpp={bad} panicked: {stderr}"
        );
    }
}
```

- [ ] **Step 13: Run and watch them fail**

Run: `cd lib/cityparquet-rs && cargo test -p cityparquet-cli --test cli convert_`
Expected: the build fails first — `missing field bloom in initializer of WriterRecipe` at `crates/cli/src/main.rs:420`.

- [ ] **Step 14: Add the flags**

In `crates/cli/src/main.rs`, replace line 8:

```rust
use cityparquet::recipe::{Codec, RecipePreset, WriterRecipe};
```

with:

```rust
use cityparquet::recipe::{BloomPolicy, Codec, RecipePreset, WriterRecipe};
```

In `Commands::Convert`, directly after the `compression: Option<String>,` field (line 88), add:

```rust
        /// Write no Parquet bloom filter. By default every object table
        /// carries filters on `id`, `feature_id` and its high-cardinality
        /// string attributes, placed after the last row group.
        #[arg(long, default_value_t = false)]
        no_bloom: bool,

        /// Target false-positive probability of every bloom filter, strictly
        /// between 0 and 1.
        #[arg(long, default_value_t = BloomPolicy::DEFAULT_FPP)]
        bloom_fpp: f64,
```

Add `no_bloom,` and `bloom_fpp,` to the `Commands::Convert { .. }` destructuring after `compression,`. Replace the recipe construction (lines 420-426):

```rust
            let recipe = WriterRecipe {
                row_group_size,
                zstd_level,
                statistics_for_json: false,
                preset: recipe.preset(),
                compression,
            };
```

with:

```rust
            let bloom = BloomPolicy {
                enabled: !no_bloom,
                fpp: bloom_fpp,
            };
            if let Err(e) = bloom.validate() {
                eprintln!("error: --bloom-fpp: {}", render_error(&e));
                return std::process::ExitCode::FAILURE;
            }
            let recipe = WriterRecipe {
                row_group_size,
                zstd_level,
                statistics_for_json: false,
                preset: recipe.preset(),
                compression,
                bloom,
            };
```

- [ ] **Step 15: Document the flags**

In `lib/cityparquet-rs/README.md`, in the convert flag table, after the `--zstd-level` row add:

```markdown
| `--no-bloom`                    | off           | write no Parquet bloom filter (by default `id`, `feature_id` and high-cardinality string attributes carry one) |
| `--bloom-fpp`                   | `0.01`        | target false-positive probability of every bloom filter, strictly between 0 and 1                               |
```

Then run `npx --yes prettier@3.9.6 --write lib/cityparquet-rs/README.md` from the repository root.

- [ ] **Step 16: Run the library gate**

Run: `cd lib/cityparquet-rs && just check`
Expected: PASS (clippy, all tests including the two new CLI tests, isolation, vendor-check, fmt, Prettier).

- [ ] **Step 17: Commit**

```bash
git add lib/cityparquet-rs/crates/core/src/recipe.rs lib/cityparquet-rs/crates/core/src/variant.rs \
  lib/cityparquet-rs/crates/cli/src/main.rs lib/cityparquet-rs/crates/cli/tests/cli.rs \
  lib/cityparquet-rs/crates/core/tests/bloom_real_data.rs lib/cityparquet-rs/README.md
git commit -m "feat(writer)!: bloom filters on id and feature_id by default

WriterRecipe gains a BloomPolicy (on, FPP 0.01); every preset but
parquet-defaults writes filters on id and feature_id after the last row
group. The variant grammar gains +nobloom and convert gains --no-bloom
and a validated --bloom-fpp. Packages written with the default recipe
now carry bloom filters."
```

---

### Task 2: High-cardinality string attributes

`scan` estimates, per scalar string attribute column, a non-null count and a distinct count, and selects the columns with `estimated_distinct >= 0.2 × non_null_count`. The selection is dataset-wide, survives every diversion, and reaches `writer_properties` as a second argument.

**Files:**

- Modify: `lib/cityparquet-rs/Cargo.toml` — `[workspace.dependencies]` (after `jsonschema = "0.26"`, line 50).
- Modify: `lib/cityparquet-rs/crates/core/Cargo.toml` — `[dependencies]` (after `city3d-stac-types`, line 25).
- Modify: `lib/cityparquet-rs/crates/core/src/scan.rs` — imports (9-23); new sketch type and constant before `pub struct ScanResult` (line 25); new field after `module_geo` (line 121); sketch declaration beside `let mut inferer` (line 190); the attribute loop (264-268); the rule after the diversion loop (after line 484); `Ok(ScanResult { .. })` (505-526); `add_synthesized_lod0_column` (after the `self.schema.attributes = kept;` line, 594).
- Modify: `lib/cityparquet-rs/crates/core/src/recipe.rs` — imports; `writer_properties` signature and doc (257-267); the bloom block from Task 1; tests (every `writer_properties` call).
- Modify: `lib/cityparquet-rs/crates/core/src/package.rs` — `CanonicalSchema` (1243-1291), `convert_source_impl` override (1340-1363), the `writer_properties` call (1396).
- Modify: `lib/cityparquet-rs/crates/core/src/partition.rs` — `CanonicalSchema { .. }` literal (402-411).
- Modify: `lib/cityparquet-rs/crates/core/tests/scan_real_data.rs` — new tests.
- Modify: `lib/cityparquet-rs/crates/core/tests/bloom_real_data.rs` — the expected map of `the_default_recipe_filters_the_identifiers_in_every_row_group`; a partition test.

**Interfaces:**

- Consumes: `BloomPolicy`, `WriterRecipe::bloom` (Task 1); the helpers in `bloom_real_data.rs` (Task 1).
- Produces:
  - `pub const BLOOM_DISTINCT_RATIO: f64 = 0.2;` in `cityparquet::scan`.
  - `ScanResult::bloom_attributes: std::collections::BTreeSet<String>`.
  - `CanonicalSchema::bloom_attributes: std::collections::BTreeSet<String>`.
  - `WriterRecipe::writer_properties(&self, schema: &CityParquetSchema, bloom_attributes: &BTreeSet<String>) -> Result<WriterProperties>`.
  - On `delft.city.jsonl`: `bloom_attributes == {"documentnummer", "identificatie"}` (exact counts 293/1115 and 1115/1115 non-null; `status` 5, `b3_dak_type` 3, `b3_pw_bron` 2, `b3_pw_selectie_reden` 3, `b3_val3dity_lod22` 6; `begingeldigheid`/`documentdatum` infer `Date`, `tijdstipregistratie`/`tijdstipregistratielv` `Timestamp`; all-null columns never qualify).

- [ ] **Step 1: Write the failing scan tests**

In `crates/core/tests/scan_real_data.rs`, change the first import line to:

```rust
use std::collections::BTreeSet;
use std::path::PathBuf;
```

and append:

```rust
/// The high-cardinality rule on delft, whose attributes sit on the 1115
/// Building rows only: `identificatie` (1115 distinct of 1115 non-null) and
/// `documentnummer` (293 of 1115, 0.26) qualify at 0.2; `status` (5 values),
/// the other short vocabularies, the Date/Timestamp-typed strings and the
/// all-null columns do not.
#[test]
fn delft_scan_selects_exactly_the_high_cardinality_string_attributes() {
    let result = scan(&Source::open(&fixture("delft.city.jsonl")).unwrap()).unwrap();
    let expected: BTreeSet<String> = ["documentnummer", "identificatie"]
        .into_iter()
        .map(String::from)
        .collect();
    assert_eq!(result.bloom_attributes, expected);
}

/// The estimator's hasher is seeded deterministically, so the selection —
/// and with it the package layout — is identical run to run.
#[test]
fn the_bloom_attribute_selection_is_deterministic() {
    let first = scan(&Source::open(&fixture("delft.city.jsonl")).unwrap()).unwrap();
    let second = scan(&Source::open(&fixture("delft.city.jsonl")).unwrap()).unwrap();
    assert_eq!(first.bloom_attributes, second.bloom_attributes);
}

/// A copy of delft with every `identificatie` attribute renamed to `to` —
/// a JSON mutation of the real fixture, never hand-written CityJSON.
fn delft_with_identificatie_renamed(to: &str) -> (tempfile::TempDir, PathBuf) {
    let text = std::fs::read_to_string(fixture("delft.city.jsonl")).unwrap();
    let mut out = String::new();
    for (index, line) in text.lines().enumerate() {
        if index == 0 || line.trim().is_empty() {
            out.push_str(line);
            out.push('\n');
            continue;
        }
        let mut feature: serde_json::Value = serde_json::from_str(line).unwrap();
        for (_, object) in feature["CityObjects"].as_object_mut().unwrap() {
            if let Some(attrs) = object
                .get_mut("attributes")
                .and_then(|a| a.as_object_mut())
                && let Some(value) = attrs.remove("identificatie")
            {
                attrs.insert(to.to_string(), value);
            }
        }
        out.push_str(&serde_json::to_string(&feature).unwrap());
        out.push('\n');
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("delft_renamed.city.jsonl");
    std::fs::write(&path, out).unwrap();
    (dir, path)
}

/// delft has no LoD0 of its own, so an attribute named `geometry_lod0_0` is a
/// legal column after `scan` — and a qualifying one — until LoD0 synthesis
/// reserves the name and diverts it into `other`. It must leave the bloom set
/// with its column.
#[test]
fn an_attribute_diverted_by_lod0_synthesis_leaves_the_bloom_set() {
    let (_dir, path) = delft_with_identificatie_renamed("geometry_lod0_0");
    let mut result = scan(&Source::open(&path).unwrap()).unwrap();
    assert!(
        result.bloom_attributes.contains("geometry_lod0_0"),
        "before synthesis: {:?}",
        result.bloom_attributes
    );
    result.add_synthesized_lod0_column();
    assert!(result.diverted_attribute_names.contains("geometry_lod0_0"));
    assert!(
        !result.bloom_attributes.contains("geometry_lod0_0"),
        "after synthesis: {:?}",
        result.bloom_attributes
    );
}
```

- [ ] **Step 2: Run and watch them fail**

Run: `cd lib/cityparquet-rs && cargo test -p cityparquet --test scan_real_data bloom`
Expected: compile error — `no field bloom_attributes on type ScanResult`.

- [ ] **Step 3: Add the dependency**

In `lib/cityparquet-rs/Cargo.toml`, after `jsonschema = "0.26"` add:

```toml
# HyperLogLog++ distinct counts for the bloom-filter high-cardinality rule
# (`scan`). Deterministic WyHash default: the selected columns, and so the
# package layout, are identical run to run.
cardinality-estimator = "1.0.3"
```

In `lib/cityparquet-rs/crates/core/Cargo.toml` `[dependencies]`, after `city3d-stac-types = { workspace = true }` add:

```toml
cardinality-estimator = { workspace = true }
```

- [ ] **Step 4: Implement the sketch in `scan`**

In `scan.rs`, replace the `cityparquet_schema` import block (lines 11-16) with:

```rust
use cityparquet_schema::{
    AttributeInferer, AttributeType, CITYPARQUET_VERSION, CityColumnEntry, CityMetadata,
    CityParquetError, CityParquetSchema, CrsState, ExtensionRegistry, GeoColumnEntry, GeoMetadata,
    GeometryEncoding, Lod, ModuleKey, ModuleKeyResolver, Result, SourceFormat as SchemaSourceFormat,
    geometry_column_name, normalise_attribute_name,
};

use cardinality_estimator::CardinalityEstimator;
```

Before the `/// Outcome of scanning a [`Source`] once` doc comment (line 25), insert:

```rust
/// A scalar string attribute column gets a bloom filter when its estimated
/// distinct count is at least this fraction of its non-null count
/// (specification `05-metadata.mdx`, "Recommended writer defaults").
pub const BLOOM_DISTINCT_RATIO: f64 = 0.2;

/// Distinct-value sketch for one attribute column's JSON-string values: the
/// non-null count and a HyperLogLog++ estimate (precision 12: exact below
/// 129 distinct values, about 1.6 % standard error above, at most ~3 KiB
/// whatever the row count).
#[derive(Default)]
struct StringCardinality {
    non_null: u64,
    distinct: CardinalityEstimator<str>,
}

impl StringCardinality {
    fn observe(&mut self, value: &str) {
        self.non_null += 1;
        self.distinct.insert(value);
    }

    /// `estimated_distinct >= 0.2 × non_null`, never for an all-null column.
    fn qualifies(&self) -> bool {
        self.non_null > 0
            && self.distinct.estimate() as f64 >= BLOOM_DISTINCT_RATIO * self.non_null as f64
    }
}
```

In `pub struct ScanResult`, after the `module_geo` field (line 121), add:

```rust
    /// The `String` attribute columns that get a bloom filter under an
    /// enabled [`crate::recipe::BloomPolicy`]: those whose estimated distinct
    /// count is at least [`BLOOM_DISTINCT_RATIO`] of their non-null count.
    /// Always a subset of `schema.attributes` of type `String`: a diverted
    /// name leaves it, both here and in
    /// [`Self::add_synthesized_lod0_column`]. Decided dataset-wide — a
    /// partitioned conversion stamps the whole-dataset set into every
    /// partition (`crate::package::CanonicalSchema::bloom_attributes`).
    pub bloom_attributes: BTreeSet<String>,
```

After `let mut inferer = AttributeInferer::default();` (line 190), add:

```rust
    let mut string_cardinality: BTreeMap<String, StringCardinality> = BTreeMap::new();
```

Replace the attribute loop (lines 264-268):

```rust
            if let Some(attrs) = co.attributes.as_ref().and_then(|v| v.as_object()) {
                for (name, value) in attrs {
                    inferer.observe(&normalise_attribute_name(name), value);
                }
            }
```

with:

```rust
            if let Some(attrs) = co.attributes.as_ref().and_then(|v| v.as_object()) {
                for (name, value) in attrs {
                    let name = normalise_attribute_name(name);
                    // Only JSON strings: a column that ends up `String`
                    // holds nothing else (numbers, booleans and arrays
                    // promote it to another type).
                    if let serde_json::Value::String(text) = value {
                        match string_cardinality.get_mut(&name) {
                            Some(sketch) => sketch.observe(text),
                            None => {
                                let mut sketch = StringCardinality::default();
                                sketch.observe(text);
                                string_cardinality.insert(name.clone(), sketch);
                            }
                        }
                    }
                    inferer.observe(&name, value);
                }
            }
```

Directly after the diversion loop's closing brace (the `for (name, ty) in inferer.finish() { .. }` block ending at line 484), add:

```rust
    // The high-cardinality rule, decided on the FINAL attribute set: a
    // diverted name has no column, and a column that inferred any type but
    // `String` (a Date or Timestamp, a Json mix) is never a candidate.
    let bloom_attributes: BTreeSet<String> = attributes
        .iter()
        .filter(|(name, ty)| {
            *ty == AttributeType::String
                && string_cardinality
                    .get(name)
                    .is_some_and(StringCardinality::qualifies)
        })
        .map(|(name, _)| name.clone())
        .collect();
```

In the returned `ScanResult { .. }`, after `module_geo,` add `bloom_attributes,`.

In `add_synthesized_lod0_column`, directly after `self.schema.attributes = kept;`, add:

```rust
        // A diverted attribute has no column any more, so no filter either.
        let attributes = &self.schema.attributes;
        self.bloom_attributes
            .retain(|name| attributes.iter().any(|(kept, _)| kept == name));
```

- [ ] **Step 5: Run the scan tests**

Run: `cd lib/cityparquet-rs && cargo test -p cityparquet --test scan_real_data`
Expected: all pass, including the three new tests.

- [ ] **Step 6: Write the failing recipe test and update the existing calls**

In `recipe.rs` tests, first update every existing call to the new signature:

```bash
sed -i \
  -e 's/\.writer_properties(&schema)/.writer_properties(\&schema, \&BTreeSet::new())/' \
  -e 's/\.writer_properties(&sample_schema())/.writer_properties(\&sample_schema(), \&BTreeSet::new())/' \
  lib/cityparquet-rs/crates/core/src/recipe.rs
```

then append:

```rust
    #[test]
    fn bloom_attributes_reach_only_string_columns_by_single_part_path() {
        let schema = CityParquetSchema {
            lods: vec![Lod::parse("2.2").unwrap()],
            geoparquet_lods: vec![Lod::parse("2.2").unwrap()],
            attributes: vec![
                ("identificatie".to_string(), AttributeType::String),
                ("status".to_string(), AttributeType::String),
                ("dotted.name".to_string(), AttributeType::String),
                ("yoc".to_string(), AttributeType::Int64),
                ("props".to_string(), AttributeType::Json),
            ],
            crs: None,
        };
        let selected: BTreeSet<String> = ["identificatie", "dotted.name", "yoc", "props", "absent"]
            .into_iter()
            .map(String::from)
            .collect();
        let props = WriterRecipe::default()
            .writer_properties(&schema, &selected)
            .unwrap();
        let filtered = |parts: &[&str]| {
            props
                .bloom_filter_properties(&ColumnPath::new(
                    parts.iter().map(|p| p.to_string()).collect(),
                ))
                .is_some()
        };
        assert!(filtered(&["identificatie"]));
        assert!(filtered(&["dotted.name"]), "a literal `.` stays one path part");
        assert!(!filtered(&["dotted", "name"]));
        assert!(!filtered(&["status"]), "not selected");
        assert!(!filtered(&["yoc"]), "numeric attributes never carry a filter");
        assert!(!filtered(&["props"]), "JSON attributes never carry a filter");

        let off = WriterRecipe {
            bloom: BloomPolicy {
                enabled: false,
                ..BloomPolicy::default()
            },
            ..WriterRecipe::default()
        }
        .writer_properties(&schema, &selected)
        .unwrap();
        assert!(
            off.bloom_filter_properties(&ColumnPath::new(vec!["identificatie".to_string()]))
                .is_none()
        );
    }
```

- [ ] **Step 7: Run and watch it fail**

Run: `cd lib/cityparquet-rs && cargo test -p cityparquet --lib recipe::tests`
Expected: compile error — `this method takes 1 argument but 2 arguments were supplied`.

- [ ] **Step 8: Change the signature**

In `recipe.rs`, add `use std::collections::BTreeSet;` above the `use arrow_schema::...` imports, and replace line 17:

```rust
use cityparquet_schema::{CityParquetError, CityParquetSchema, Result};
```

with:

```rust
use cityparquet_schema::{AttributeType, CityParquetError, CityParquetSchema, Result};
```

Replace the signature line 267:

```rust
    pub fn writer_properties(&self, schema: &CityParquetSchema) -> Result<WriterProperties> {
```

with:

```rust
    ///
    /// `bloom_attributes` names the scalar string attributes that also get a
    /// bloom filter under an enabled [`BloomPolicy`] —
    /// [`crate::scan::ScanResult::bloom_attributes`]. A name that is not a
    /// `String` attribute of `schema` is ignored.
    pub fn writer_properties(
        &self,
        schema: &CityParquetSchema,
        bloom_attributes: &BTreeSet<String>,
    ) -> Result<WriterProperties> {
```

(the three new `///` lines continue the existing doc comment above it). In the Task 1 bloom block, after the `for name in ["id", "feature_id"] { .. }` loop and still inside `if self.bloom.enabled { .. }`, add:

```rust
            // High-cardinality scalar string attributes, as `scan` decided
            // them. An attribute column is one top-level leaf, so its path is
            // the single-part `[name]` even when the name holds a literal `.`.
            for (name, ty) in &schema.attributes {
                if *ty == AttributeType::String && bloom_attributes.contains(name) {
                    let path = ColumnPath::new(vec![name.clone()]);
                    builder = builder
                        .set_column_bloom_filter_enabled(path.clone(), true)
                        .set_column_bloom_filter_fpp(path, self.bloom.fpp);
                }
            }
```

and update the block's leading comment's first sentence to: `// Bloom filters: `id` and `feature_id` in every object table — the two identifier columns whose min/max statistics cannot prune — plus the high-cardinality string attributes.`

- [ ] **Step 9: Thread the set through the package writer**

In `package.rs`, in `pub struct CanonicalSchema`, after `crs_diagnostic` add:

```rust
    /// The whole-dataset bloom-filtered attribute set (see
    /// [`crate::scan::ScanResult::bloom_attributes`]), stamped into every
    /// partition's scan like the column sets above: the high-cardinality
    /// rule is a dataset property, and a partition's handful of rows would
    /// otherwise select nearly every string column.
    pub bloom_attributes: std::collections::BTreeSet<String>,
```

In `convert_source_impl`, after `scan_result.crs_diagnostic = canon.crs_diagnostic.clone();` add:

```rust
        scan_result.bloom_attributes = canon.bloom_attributes.clone();
```

Replace line 1396:

```rust
    let props = opts.recipe.writer_properties(&scan_result.schema)?;
```

with:

```rust
    let props = opts
        .recipe
        .writer_properties(&scan_result.schema, &scan_result.bloom_attributes)?;
```

In `partition.rs`, in the `CanonicalSchema { .. }` literal (line 402), after `crs_diagnostic: full_scan.crs_diagnostic.clone(),` add:

```rust
        bloom_attributes: full_scan.bloom_attributes.clone(),
```

- [ ] **Step 10: Update the package test and add the partition test**

In `bloom_real_data.rs`, replace the `expected` map in `the_default_recipe_filters_the_identifiers_in_every_row_group` with:

```rust
    let expected: BTreeMap<String, usize> = [
        ("documentnummer", groups),
        ("feature_id", groups),
        ("id", groups),
        ("identificatie", groups),
    ]
    .into_iter()
    .map(|(name, n)| (name.to_string(), n))
    .collect();
```

rename the test to `the_default_recipe_filters_the_identifiers_and_high_cardinality_attributes`, add to the imports:

```rust
use cityparquet::partition::{PartitionSpec, convert_partitioned};
use cityparquet::source::Source;
```

and append:

```rust
/// 1115 features in 300 contiguous chunks leaves 3 or 4 Buildings per
/// partition, where a partition-local rule would select `status` everywhere
/// (one distinct value of at most four is at least 0.2). Every partition
/// must carry the dataset-wide decision instead.
#[test]
fn partitions_share_the_dataset_wide_attribute_decision() {
    let out = tempfile::tempdir().unwrap();
    let src = Source::open(&fixture("delft.city.jsonl")).unwrap();
    let opts = ConvertOptions::new(fixture("delft.city.jsonl"), out.path().to_path_buf());
    let report =
        convert_partitioned(std::slice::from_ref(&src), &PartitionSpec::Count(300), &opts)
            .unwrap();
    assert_eq!(report.partitions.len(), 300);
    for (label, _) in &report.partitions {
        let filtered = filtered_columns(&out.path().join(label).join("building.parquet"));
        for name in ["id", "feature_id", "identificatie", "documentnummer"] {
            assert!(filtered.contains_key(name), "{label} lacks {name}: {filtered:?}");
        }
        assert!(!filtered.contains_key("status"), "{label}: {filtered:?}");
    }
}
```

- [ ] **Step 11: Run the affected suites**

Run: `cd lib/cityparquet-rs && cargo test -p cityparquet --lib recipe::tests && cargo test -p cityparquet --test bloom_real_data && cargo test -p cityparquet --test partition_real_data`
Expected: all pass.

- [ ] **Step 12: Run the library gate**

Run: `cd lib/cityparquet-rs && just check`
Expected: PASS.

- [ ] **Step 13: Commit**

```bash
git add lib/cityparquet-rs/Cargo.toml lib/cityparquet-rs/Cargo.lock lib/cityparquet-rs/crates/core/Cargo.toml \
  lib/cityparquet-rs/crates/core/src/scan.rs lib/cityparquet-rs/crates/core/src/recipe.rs \
  lib/cityparquet-rs/crates/core/src/package.rs lib/cityparquet-rs/crates/core/src/partition.rs \
  lib/cityparquet-rs/crates/core/tests/scan_real_data.rs lib/cityparquet-rs/crates/core/tests/bloom_real_data.rs
git commit -m "feat(writer): bloom filters on high-cardinality string attributes

scan keeps a HyperLogLog distinct count per scalar string attribute
(cardinality-estimator, deterministic hasher) and selects the columns
whose estimate reaches 0.2 of their non-null count. The set is
intersected with the final attribute columns, canonicalised across
partitions, and passed to WriterRecipe::writer_properties."
```

---

### Task 3: Sync bloom pruning — `id_lookup_with_stats` and `attr_filter`

A pure target finder, a sync pruner over any `ChunkReader`, the lookup statistics, and the two sync call sites. `id_lookup` keeps its signature and delegates.

**Files:**

- Modify: `lib/cityparquet-rs/crates/core/src/query_core.rs` — imports (10-21); new types and functions after `BBoxQueryResult` (after line 72); `id_row_filter` (349-367) replaced by `utf8_eq_row_filter`.
- Modify: `lib/cityparquet-rs/crates/core/src/query.rs` — module doc (1-20), imports (22-31), `attr_filter` (106-139), `id_lookup` (190-222), new `bloom_keep_row_groups` and `id_lookup_with_stats`.
- Modify: `lib/cityparquet-rs/crates/core/src/query_async.rs` — `id_lookup_async` (lines 226-230) keeps compiling against `utf8_eq_row_filter` (full async rewrite is Task 4).
- Modify: `lib/cityparquet-rs/crates/core/tests/bloom_real_data.rs` — reader tests.

**Interfaces:**

- Consumes: packages with filters (Tasks 1-2); `delft_recipe`, `convert_with`, `row_group_count`, `filtered_columns` test helpers (Task 1).
- Produces (all `pub` in `query_core`, re-exported from `cityparquet::query`):
  - `pub struct BloomTarget { pub row_group: usize, pub offset: u64, pub length: Option<u64> }` — `#[derive(Debug, Clone, Copy, PartialEq, Eq)]`.
  - `pub struct BloomTargets { pub leaf: Option<usize>, pub with_filter: Vec<BloomTarget>, pub without_filter: Vec<usize> }` — `#[derive(Debug, Clone, Default, PartialEq, Eq)]`.
  - `pub struct BloomPrune { pub keep: Vec<usize>, pub total: usize, pub pruned: usize, pub without_filter: usize, pub filter_bytes: u64 }` — `#[derive(Debug, Clone, Default, PartialEq, Eq)]`; `keep` ascending.
  - `pub struct LookupStats { pub row_groups_total: usize, pub bloom_pruned: usize, pub row_groups_read: usize, pub filter_bytes: u64 }` — `#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]`, `impl std::ops::AddAssign`.
  - `pub fn bloom_targets(meta: &ParquetMetaData, column: &ColumnPath, candidates: &[usize]) -> BloomTargets`.
  - crate-private: `BloomPrune::from_verdicts(targets: &BloomTargets, candidates: &[usize], verdicts: &[bool], filter_bytes: u64) -> BloomPrune`, `LookupStats::from_prune(prune: &BloomPrune) -> LookupStats`, `sbbf_matches_any(sbbf: &Sbbf, values: &[&str]) -> bool`, `top_level_path(name: &str) -> ColumnPath`, `bloom_probe_value(pred: &AttrPredicate) -> Option<&str>`, `utf8_eq_row_filter(parquet_schema: &SchemaDescriptor, column: &str, value: &str) -> RowFilter`.
  - `cityparquet::query::bloom_keep_row_groups<R: ChunkReader>(reader: &R, meta: &ParquetMetaData, column: &ColumnPath, values: &[&str], candidates: &[usize]) -> Result<BloomPrune>`.
  - `cityparquet::query::id_lookup_with_stats(table_path: &Path, meta: &CityMetadata, id: &str) -> Result<(Option<DecodedObject>, LookupStats)>`.
  - Semantics used by every later task: `total = candidates.len()`; `pruned` = candidates with a filter in which no value tested positive; `without_filter` = candidates whose chunk has no filter (kept); `filter_bytes` = sum of `bloom_filter_length` of the filters read (for a filter without a declared length, its bitset size, `32 × num_blocks`); `row_groups_read` = kept row groups actually opened (a hit stops at its first match).

- [ ] **Step 1: Write the failing reader tests**

In `bloom_real_data.rs`, add to the imports:

```rust
use arrow_array::{Array, StringArray};
use cityparquet::query::{
    AttrPredicate, attr_filter, bloom_keep_row_groups, bloom_targets, id_lookup,
    id_lookup_with_stats,
};
use cityparquet::reader::CityParquetReaderBuilder;
use cityparquet::schema::CityMetadata;
use parquet::arrow::ProjectionMask;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::schema::types::ColumnPath;
```

and append:

```rust
/// A miss for every delft `id`/`feature_id`: the 3DBAG prefix with a suffix
/// no BAG identifier carries.
const MISS: &str = "NL.IMBAG.Pand.readbench-absent";

/// Every non-null value of top-level Utf8 `column`, with its row group.
fn values_by_row_group(table: &Path, column: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    for rg in 0..row_group_count(table) {
        let builder = ParquetRecordBatchReaderBuilder::try_new(File::open(table).unwrap()).unwrap();
        let mask = ProjectionMask::columns(builder.parquet_schema(), [column]);
        let reader = builder
            .with_projection(mask)
            .with_row_groups(vec![rg])
            .build()
            .unwrap();
        for batch in reader {
            let batch = batch.unwrap();
            let values = batch
                .column(0)
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap();
            for i in 0..values.len() {
                if !values.is_null(i) {
                    out.push((rg, values.value(i).to_string()));
                }
            }
        }
    }
    out
}

fn table_meta(table: &Path) -> CityMetadata {
    ParquetRecordBatchReaderBuilder::try_new(File::open(table).unwrap())
        .unwrap()
        .cityparquet_metadata()
        .unwrap()
}

fn footer(table: &Path) -> std::sync::Arc<parquet::file::metadata::ParquetMetaData> {
    ParquetRecordBatchReaderBuilder::try_new(File::open(table).unwrap())
        .unwrap()
        .metadata()
        .clone()
}

fn id_path() -> ColumnPath {
    ColumnPath::new(vec!["id".to_string()])
}

fn nobloom(row_group_size: usize) -> WriterRecipe {
    WriterRecipe {
        bloom: BloomPolicy {
            enabled: false,
            ..BloomPolicy::default()
        },
        ..delft_recipe(row_group_size)
    }
}

#[test]
fn bloom_targets_locate_one_filter_per_candidate_in_row_group_order() {
    let out = convert_with("delft.city.jsonl", delft_recipe(64));
    let table = out.path().join("building.parquet");
    let meta = footer(&table);
    assert_eq!(meta.num_row_groups(), 35, "2231 rows at 64 per group");

    let targets = bloom_targets(&meta, &id_path(), &[1, 3, 34]);
    assert!(targets.leaf.is_some());
    assert!(targets.without_filter.is_empty());
    let groups: Vec<usize> = targets.with_filter.iter().map(|t| t.row_group).collect();
    assert_eq!(groups, vec![1, 3, 34]);
    assert!(targets.with_filter.iter().all(|t| t.length.is_some()));
    assert!(
        targets.with_filter.windows(2).all(|w| w[0].offset < w[1].offset),
        "End placement lays filters out row group by row group"
    );

    let absent = bloom_targets(&meta, &ColumnPath::new(vec!["no_such".to_string()]), &[0, 1]);
    assert_eq!(absent.leaf, None);
    assert_eq!(absent.without_filter, vec![0, 1]);

    let off = convert_with("delft.city.jsonl", nobloom(64));
    let off_targets = bloom_targets(
        &footer(&off.path().join("building.parquet")),
        &id_path(),
        &[0, 1, 2],
    );
    assert!(off_targets.leaf.is_some());
    assert!(off_targets.with_filter.is_empty());
    assert_eq!(off_targets.without_filter, vec![0, 1, 2]);
}

/// No false negatives: for every id in the table, the pruner keeps the row
/// group that holds it.
#[test]
fn the_bloom_prune_never_drops_the_row_group_holding_an_id() {
    let out = convert_with("delft.city.jsonl", delft_recipe(64));
    let table = out.path().join("building.parquet");
    let meta = footer(&table);
    let file = File::open(&table).unwrap();
    let all: Vec<usize> = (0..meta.num_row_groups()).collect();
    let ids = values_by_row_group(&table, "id");
    assert_eq!(ids.len(), 2231);
    for (rg, id) in &ids {
        let prune = bloom_keep_row_groups(&file, &meta, &id_path(), &[id.as_str()], &all).unwrap();
        assert!(prune.keep.contains(rg), "{id} lives in row group {rg}: {prune:?}");
        assert_eq!(prune.total, all.len());
        assert_eq!(prune.without_filter, 0);
        assert_eq!(prune.keep.len() + prune.pruned, all.len());
    }
}

#[test]
fn id_lookup_finds_sampled_ids_and_reports_what_it_read() {
    let out = convert_with("delft.city.jsonl", delft_recipe(64));
    let table = out.path().join("building.parquet");
    let meta = table_meta(&table);
    for (_, id) in values_by_row_group(&table, "id").iter().step_by(97) {
        let (found, stats) = id_lookup_with_stats(&table, &meta, id).unwrap();
        assert_eq!(found.as_ref().map(|o| o.id.as_str()), Some(id.as_str()));
        assert_eq!(stats.row_groups_total, 35);
        assert!(stats.row_groups_read >= 1);
        assert!(stats.row_groups_read <= stats.row_groups_total - stats.bloom_pruned);
        assert!(stats.filter_bytes > 0);
        assert_eq!(
            id_lookup(&table, &meta, id).unwrap().map(|o| o.id),
            Some(id.clone())
        );
    }
}

#[test]
fn a_miss_is_pruned_by_the_filters_and_read_in_full_without_them() {
    let on = convert_with("delft.city.jsonl", delft_recipe(64));
    let table = on.path().join("building.parquet");
    let (found, stats) = id_lookup_with_stats(&table, &table_meta(&table), MISS).unwrap();
    assert!(found.is_none());
    assert!(stats.bloom_pruned >= 1, "{stats:?}");
    assert_eq!(stats.row_groups_read, stats.row_groups_total - stats.bloom_pruned);

    let off = convert_with("delft.city.jsonl", nobloom(64));
    let table = off.path().join("building.parquet");
    let (found, stats) = id_lookup_with_stats(&table, &table_meta(&table), MISS).unwrap();
    assert!(found.is_none());
    assert_eq!(stats.bloom_pruned, 0);
    assert_eq!(stats.row_groups_read, 35);
    assert_eq!(stats.filter_bytes, 0);
}

/// String equality on a filtered column (`identificatie`) and on an
/// unfiltered one (`status`) counts exactly what an unfiltered package
/// counts; a miss counts zero.
#[test]
fn attr_filter_on_string_columns_counts_like_an_unfiltered_package() {
    let on = convert_with("delft.city.jsonl", delft_recipe(64));
    let off = convert_with("delft.city.jsonl", nobloom(64));
    let on_table = on.path().join("building.parquet");
    let off_table = off.path().join("building.parquet");
    assert!(filtered_columns(&on_table).contains_key("identificatie"));

    let (_, target) = values_by_row_group(&on_table, "identificatie")
        .into_iter()
        .nth(500)
        .unwrap();
    for (column, value, expected) in [
        ("identificatie", target.as_str(), Some(1)),
        ("identificatie", MISS, Some(0)),
        ("status", "Pand in gebruik", None),
    ] {
        let pred = AttrPredicate::Eq(serde_json::Value::String(value.to_string()));
        let got = attr_filter(&on_table, column, &pred).unwrap();
        assert_eq!(got, attr_filter(&off_table, column, &pred).unwrap(), "{column}={value}");
        if let Some(expected) = expected {
            assert_eq!(got, expected, "{column}={value}");
        }
    }
}
```

- [ ] **Step 2: Run and watch them fail**

Run: `cd lib/cityparquet-rs && cargo test -p cityparquet --test bloom_real_data`
Expected: compile error — `unresolved imports cityparquet::query::bloom_keep_row_groups, bloom_targets, id_lookup_with_stats`.

- [ ] **Step 3: Add the shared types to `query_core`**

In `query_core.rs`, add to the imports:

```rust
use parquet::bloom_filter::Sbbf;
```

After `BBoxQueryResult` (after line 72), insert:

```rust
/// One candidate row group's bloom filter for the target column, located
/// from footer metadata alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BloomTarget {
    pub row_group: usize,
    pub offset: u64,
    /// `bloom_filter_length`, when the writer declared it.
    pub length: Option<u64>,
}

/// Where the target column's filters are, per candidate row group.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BloomTargets {
    /// The column's Parquet leaf index, resolved by path from the file
    /// schema (never an Arrow field ordinal); `None` when the file has no
    /// such leaf.
    pub leaf: Option<usize>,
    pub with_filter: Vec<BloomTarget>,
    /// Candidates whose chunk carries no filter. They are always kept.
    pub without_filter: Vec<usize>,
}

/// The outcome of probing a column's bloom filters for a set of values
/// (IN semantics): which candidate row groups can still hold one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BloomPrune {
    /// Row groups to read, ascending — for `with_row_groups`.
    pub keep: Vec<usize>,
    /// Candidates considered.
    pub total: usize,
    /// Candidates whose filter rejected every value.
    pub pruned: usize,
    /// Candidates kept because their chunk has no filter.
    pub without_filter: usize,
    /// Bytes of filter read: each declared `bloom_filter_length`, or the
    /// bitset size of a filter without one.
    pub filter_bytes: u64,
}

impl BloomPrune {
    /// Assembles the outcome from `targets` and one verdict per
    /// `targets.with_filter` entry (`true`: keep).
    pub(crate) fn from_verdicts(
        targets: &BloomTargets,
        candidates: &[usize],
        verdicts: &[bool],
        filter_bytes: u64,
    ) -> Self {
        let mut keep = targets.without_filter.clone();
        keep.extend(
            targets
                .with_filter
                .iter()
                .zip(verdicts)
                .filter(|(_, keep)| **keep)
                .map(|(target, _)| target.row_group),
        );
        keep.sort_unstable();
        Self {
            keep,
            total: candidates.len(),
            pruned: verdicts.iter().filter(|keep| !**keep).count(),
            without_filter: targets.without_filter.len(),
            filter_bytes,
        }
    }
}

/// What one identifier lookup cost, in row groups and filter bytes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LookupStats {
    pub row_groups_total: usize,
    pub bloom_pruned: usize,
    /// Kept row groups actually opened; a lookup that stops at its first
    /// match reads fewer than it kept.
    pub row_groups_read: usize,
    pub filter_bytes: u64,
}

impl LookupStats {
    /// The prune's share of the statistics; `row_groups_read` starts at 0.
    pub(crate) fn from_prune(prune: &BloomPrune) -> Self {
        Self {
            row_groups_total: prune.total,
            bloom_pruned: prune.pruned,
            row_groups_read: 0,
            filter_bytes: prune.filter_bytes,
        }
    }
}

impl std::ops::AddAssign for LookupStats {
    fn add_assign(&mut self, other: Self) {
        self.row_groups_total += other.row_groups_total;
        self.bloom_pruned += other.bloom_pruned;
        self.row_groups_read += other.row_groups_read;
        self.filter_bytes += other.filter_bytes;
    }
}

/// Resolves `column`'s leaf by path and, for each candidate row group, where
/// its filter is — from footer metadata alone, no I/O. Shared verbatim by
/// the sync and async pruners.
pub fn bloom_targets(
    meta: &ParquetMetaData,
    column: &ColumnPath,
    candidates: &[usize],
) -> BloomTargets {
    let leaf = meta
        .file_metadata()
        .schema_descr()
        .columns()
        .iter()
        .position(|descr| descr.path() == column);
    let Some(leaf) = leaf else {
        return BloomTargets {
            leaf: None,
            with_filter: Vec::new(),
            without_filter: candidates.to_vec(),
        };
    };
    let mut targets = BloomTargets {
        leaf: Some(leaf),
        ..BloomTargets::default()
    };
    for &row_group in candidates {
        let chunk = meta.row_group(row_group).column(leaf);
        match chunk
            .bloom_filter_offset()
            .and_then(|offset| u64::try_from(offset).ok())
        {
            Some(offset) => targets.with_filter.push(BloomTarget {
                row_group,
                offset,
                length: chunk
                    .bloom_filter_length()
                    .and_then(|length| u64::try_from(length).ok()),
            }),
            None => targets.without_filter.push(row_group),
        }
    }
    targets
}

/// A filter keeps its row group when ANY value may be present (IN
/// semantics). Values are probed as their raw UTF-8 bytes, which is what
/// the writer hashed.
pub(crate) fn sbbf_matches_any(sbbf: &Sbbf, values: &[&str]) -> bool {
    values.iter().any(|value| sbbf.check(*value))
}

/// A top-level column's single-part leaf path.
pub(crate) fn top_level_path(name: &str) -> ColumnPath {
    ColumnPath::new(vec![name.to_string()])
}

/// The value `pred` probes a bloom filter with: only string equality. The
/// numeric predicates compare through `f64` and never consult a filter.
pub(crate) fn bloom_probe_value(pred: &AttrPredicate) -> Option<&str> {
    match pred {
        AttrPredicate::Eq(serde_json::Value::String(value)) => Some(value.as_str()),
        _ => None,
    }
}
```

Replace the whole `id_row_filter` function (lines 346-367, doc comment included) with:

```rust
/// The `column == value` [`RowFilter`] both transports install for the
/// identifier lookups (`id`, `feature_id`): the predicate's own projection is
/// `column` alone; the output projection stays untouched (the full row is
/// decoded on a hit). A positive bloom result is never a match — this filter
/// is what decides every row.
pub(crate) fn utf8_eq_row_filter(
    parquet_schema: &SchemaDescriptor,
    column: &str,
    value: &str,
) -> RowFilter {
    let predicate_mask = ProjectionMask::columns(parquet_schema, [column]);
    let owned_column = column.to_string();
    let owned_value = value.to_string();
    let predicate_fn = ArrowPredicateFn::new(predicate_mask, move |batch: RecordBatch| {
        let values = batch
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .ok_or_else(|| {
                arrow_schema::ArrowError::SchemaError(format!(
                    "'{owned_column}' column is not Utf8"
                ))
            })?;
        Ok(BooleanArray::from_iter((0..values.len()).map(|i| {
            Some(!values.is_null(i) && values.value(i) == owned_value)
        })))
    });
    RowFilter::new(vec![Box::new(predicate_fn)])
}
```

In `query_async.rs` `id_lookup_async`, replace `let row_filter = query_core::id_row_filter(builder.parquet_schema(), id);` with `let row_filter = query_core::utf8_eq_row_filter(builder.parquet_schema(), "id", id);` (Task 4 rewrites the function).

- [ ] **Step 4: Add the sync pruner and lookup**

In `query.rs`, replace the imports (lines 22-31) with:

```rust
use std::fs::File;
use std::path::Path;
use std::sync::Arc;

use cityparquet_schema::{CityMetadata, CityParquetError, Result};
use parquet::arrow::ProjectionMask;
use parquet::arrow::arrow_reader::{
    ArrowReaderMetadata, ArrowReaderOptions, ParquetRecordBatchReaderBuilder,
};
use parquet::bloom_filter::Sbbf;
use parquet::file::metadata::ParquetMetaData;
use parquet::file::reader::ChunkReader;
use parquet::schema::types::ColumnPath;

use crate::decode::DecodedObject;
use crate::query_core;
use crate::reader::{CityParquetReaderBuilder, CityParquetRecordBatchReader};

pub use crate::query_core::{
    AttrPredicate, AttrStats, BBoxQueryResult, BloomPrune, BloomTarget, BloomTargets,
    FullReadResult, LookupStats, bloom_targets,
};
```

Append to the module doc comment (after line 20):

```rust
//!
//! **Bloom filters.** [`bloom_keep_row_groups`] probes a column's Parquet
//! bloom filters for a set of values and keeps only the row groups that can
//! hold one; a chunk without a filter is always kept, and a positive result
//! is never a match — the exact `RowFilter` still decides every row. The
//! identifier lookups ([`id_lookup_with_stats`], `feature_lookup*`) and
//! string-equality [`attr_filter`] use it.
```

Directly before `id_lookup`, insert:

```rust
/// Probes `column`'s bloom filter in each of `candidates` (row-group
/// indices) for `values`, with one `pread` per filter on `reader` — a local
/// file, not the builder's own reader, which the arrow builders keep
/// private. Keeps a row group when its chunk has no filter or when any value
/// may be present.
pub fn bloom_keep_row_groups<R: ChunkReader>(
    reader: &R,
    meta: &ParquetMetaData,
    column: &ColumnPath,
    values: &[&str],
    candidates: &[usize],
) -> Result<BloomPrune> {
    let targets = query_core::bloom_targets(meta, column, candidates);
    let mut verdicts = Vec::with_capacity(targets.with_filter.len());
    let mut filter_bytes = 0u64;
    if let Some(leaf) = targets.leaf {
        for target in &targets.with_filter {
            let chunk = meta.row_group(target.row_group).column(leaf);
            match Sbbf::read_from_column_chunk(chunk, reader)
                .map_err(CityParquetError::parquet_from)?
            {
                Some(sbbf) => {
                    filter_bytes += target
                        .length
                        .unwrap_or(32 * sbbf.num_blocks() as u64);
                    verdicts.push(query_core::sbbf_matches_any(&sbbf, values));
                }
                None => verdicts.push(true),
            }
        }
    }
    Ok(BloomPrune::from_verdicts(
        &targets,
        candidates,
        &verdicts,
        filter_bytes,
    ))
}
```

Replace the whole `id_lookup` function (lines 182-222, doc comment included) with:

```rust
/// Finds and fully materialises the one object whose `id` column equals
/// `id`; `None` if no row matches. See [`id_lookup_with_stats`].
pub fn id_lookup(
    table_path: &Path,
    meta: &CityMetadata,
    id: &str,
) -> Result<Option<DecodedObject>> {
    id_lookup_with_stats(table_path, meta, id).map(|(object, _)| object)
}

/// [`id_lookup`] with what it cost. The `id` bloom filters prune the row
/// groups first ([`bloom_keep_row_groups`]); each surviving row group is
/// then read by a builder of its own, all sharing one parsed footer, with
/// `id` as an exact `Eq` [`RowFilter`](parquet::arrow::arrow_reader::RowFilter)
/// projected to `id` alone, and the first matching row is decoded in full
/// via [`crate::decode::decode_batch`]. One builder per row group is what
/// lets [`LookupStats::row_groups_read`] count exactly the row groups
/// opened: a hit stops before the next one.
pub fn id_lookup_with_stats(
    table_path: &Path,
    meta: &CityMetadata,
    id: &str,
) -> Result<(Option<DecodedObject>, LookupStats)> {
    let file = File::open(table_path)?;
    let arrow_meta = ArrowReaderMetadata::load(&file, ArrowReaderOptions::new())
        .map_err(CityParquetError::parquet_from)?;
    let candidates: Vec<usize> = (0..arrow_meta.metadata().num_row_groups()).collect();
    let prune = bloom_keep_row_groups(
        &file,
        arrow_meta.metadata(),
        &query_core::top_level_path("id"),
        &[id],
        &candidates,
    )?;
    let mut stats = LookupStats::from_prune(&prune);
    let schema =
        ParquetRecordBatchReaderBuilder::new_with_metadata(file.try_clone()?, arrow_meta.clone())
            .cityparquet_arrow_schema()?;

    for row_group in prune.keep {
        stats.row_groups_read += 1;
        let builder =
            ParquetRecordBatchReaderBuilder::new_with_metadata(file.try_clone()?, arrow_meta.clone());
        let row_filter = query_core::utf8_eq_row_filter(builder.parquet_schema(), "id", id);
        let parquet_reader = builder
            .with_row_groups(vec![row_group])
            .with_row_filter(row_filter)
            .build()
            .map_err(CityParquetError::parquet_from)?;
        let reader = CityParquetRecordBatchReader::new(parquet_reader, Arc::clone(&schema));
        for batch in reader {
            if let Some(object) = query_core::first_decoded_object(&batch?, meta)? {
                return Ok((Some(object), stats));
            }
        }
    }
    Ok((None, stats))
}
```

- [ ] **Step 5: Prune `attr_filter` and fix its doc comment**

Replace the whole `attr_filter` function (lines 106-139, doc comment included) with:

```rust
/// Opens `table_path`, restricts the scan to `column` alone via a
/// [`ProjectionMask`], and applies `pred` as a Parquet
/// [`RowFilter`](parquet::arrow::arrow_reader::RowFilter)
/// (`ArrowPredicateFn`) so only `column` is ever decoded. Column statistics
/// prune nothing here. For a string equality, `column`'s bloom filters —
/// where the file carries them — first drop every row group that cannot
/// hold the value ([`bloom_keep_row_groups`]); a positive bloom result is
/// never a match, so the `RowFilter` still decides every row. Returns the
/// COUNT of surviving rows (never the rows themselves): the `RowFilter`
/// drops every non-matching row before it reaches the returned batches, so
/// counting rows across them IS the matching count.
pub fn attr_filter(table_path: &Path, column: &str, pred: &AttrPredicate) -> Result<u64> {
    let file = File::open(table_path)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file.try_clone()?)
        .map_err(CityParquetError::parquet_from)?;

    // Checked now (before `builder` is consumed by `with_projection`/
    // `with_row_filter` below) purely to fail fast with a clear "column not
    // found" error; `evaluate_attr_predicate` re-derives the type itself from
    // the projected batch's own schema, so this lookup is not load-bearing
    // for correctness, only for a better error message.
    query_core::require_column(builder.schema(), column)?;

    // Same single-column mask twice: once as the predicate's own required
    // projection (inside the row filter), once as the builder's overall
    // output projection — `column` is the only thing either the predicate or
    // the final count needs.
    let output_mask = ProjectionMask::columns(builder.parquet_schema(), [column]);
    let row_filter = query_core::attr_predicate_row_filter(builder.parquet_schema(), column, pred);

    let mut builder = builder
        .with_projection(output_mask)
        .with_row_filter(row_filter);
    if let Some(value) = query_core::bloom_probe_value(pred) {
        let candidates: Vec<usize> = (0..builder.metadata().num_row_groups()).collect();
        let prune = bloom_keep_row_groups(
            &file,
            builder.metadata(),
            &query_core::top_level_path(column),
            &[value],
            &candidates,
        )?;
        builder = builder.with_row_groups(prune.keep);
    }
    let reader = builder.build().map_err(CityParquetError::parquet_from)?;

    let mut count = 0u64;
    for batch in reader {
        count += batch.map_err(CityParquetError::parquet_from)?.num_rows() as u64;
    }
    Ok(count)
}
```

- [ ] **Step 6: Run the reader tests**

Run: `cd lib/cityparquet-rs && cargo test -p cityparquet --test bloom_real_data && cargo test -p cityparquet --test query_real_data`
Expected: all pass (the existing `query_real_data` id and attribute tests still pass through the new code paths).

- [ ] **Step 7: Run the library gate**

Run: `cd lib/cityparquet-rs && just check`
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add lib/cityparquet-rs/crates/core/src/query_core.rs lib/cityparquet-rs/crates/core/src/query.rs \
  lib/cityparquet-rs/crates/core/src/query_async.rs lib/cityparquet-rs/crates/core/tests/bloom_real_data.rs
git commit -m "feat(query): prune id_lookup and string attr_filter with bloom filters

bloom_targets locates a column's filters from footer metadata;
bloom_keep_row_groups probes them over any ChunkReader with IN
semantics, keeping chunks without a filter. id_lookup_with_stats reads
the surviving row groups one builder each and reports LookupStats;
id_lookup delegates. attr_filter prunes on string equality and its doc
comment no longer claims statistics pruning."
```

---

### Task 4: Async bloom pruning in one ranged request

The async pruner collects every filter range from metadata and fetches them with a single `AsyncFileReader::get_byte_ranges` call (object_store coalesces nearby ranges into few requests); a filter without a declared length falls back to `get_row_group_column_bloom_filter`. The library gate starts running the async tests.

**Files:**

- Modify: `lib/cityparquet-rs/justfile` — `test` and `lint` recipes (lines 27-31).
- Modify: `lib/cityparquet-rs/crates/core/Cargo.toml` — `[dev-dependencies]` (after `tower-http`).
- Modify: `lib/cityparquet-rs/crates/core/src/query_async.rs` — imports (20-39), `attr_filter_async` (140-173), `id_lookup_async` (209-247), new `bloom_keep_row_groups_async` and `id_lookup_async_with_stats`, tests module (285-447).

**Interfaces:**

- Consumes: `BloomTargets`, `BloomPrune::from_verdicts`, `LookupStats::from_prune`, `sbbf_matches_any`, `top_level_path`, `bloom_probe_value`, `utf8_eq_row_filter` (Task 3); `bloom_keep_row_groups` for parity tests (Task 3).
- Produces:
  - `pub async fn bloom_keep_row_groups_async<R>(reader: &mut R, metadata: &ArrowReaderMetadata, column: &ColumnPath, values: &[&str], candidates: &[usize]) -> Result<BloomPrune> where R: AsyncFileReader + Clone + Send + 'static` in `cityparquet::query_async`.
  - `pub async fn id_lookup_async_with_stats(store: Arc<dyn ObjectStore>, path: &ObjectPath, meta: &CityMetadata, id: &str) -> Result<(Option<DecodedObject>, LookupStats)>`.
  - `id_lookup_async` keeps its signature and delegates.

- [ ] **Step 1: Make the gate run the async tests**

In `lib/cityparquet-rs/justfile`, replace:

```just
test:
    cargo test --workspace

lint:
    cargo clippy --workspace --all-targets -- -D warnings
```

with:

```just
# `--all-features`: the object-store transport (`query_async`,
# `counting_store`) is a feature, and its tests only run with it on.
test:
    cargo test --workspace --all-features

lint:
    cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Run: `cd lib/cityparquet-rs && just test 2>&1 | grep -E "query_async|test result" | head`
Expected: the seven existing `query_async::tests` run and pass.

- [ ] **Step 2: Write the failing async tests**

In `crates/core/Cargo.toml` `[dev-dependencies]`, after `tower-http = { workspace = true }` add:

```toml
bytes = { workspace = true }
```

In `query_async.rs` `mod tests`, add below the existing `delft_table` helper:

```rust
    /// `delft_table`, written with `recipe`.
    async fn delft_table_with(
        dir: &Path,
        recipe: crate::recipe::WriterRecipe,
    ) -> (Arc<dyn ObjectStore>, ObjectPath) {
        let fixture = fixture_dir().join("delft.city.jsonl");
        assert!(fixture.exists(), "missing fixture; run `just fixtures`");
        let mut opts = crate::package::ConvertOptions::new(fixture, dir.to_path_buf());
        opts.recipe = recipe;
        crate::package::convert(&opts).unwrap();
        let store = LocalFileSystem::new_with_prefix(dir).unwrap();
        (Arc::new(store), ObjectPath::from("building.parquet"))
    }

    fn small_groups() -> crate::recipe::WriterRecipe {
        crate::recipe::WriterRecipe {
            row_group_size: 64,
            ..crate::recipe::WriterRecipe::default()
        }
    }

    fn sync_meta(table_file: &Path) -> CityMetadata {
        let file = std::fs::File::open(table_file).unwrap();
        let builder =
            parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
        crate::reader::CityParquetReaderBuilder::cityparquet_metadata(&builder).unwrap()
    }

    /// Every id in the table, in row order.
    fn every_id(table_file: &Path) -> Vec<String> {
        let file = std::fs::File::open(table_file).unwrap();
        let builder =
            parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
        let mask = ProjectionMask::columns(builder.parquet_schema(), ["id"]);
        let mut ids = Vec::new();
        for batch in builder.with_projection(mask).build().unwrap() {
            let batch = batch.unwrap();
            let column = batch
                .column(0)
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap();
            ids.extend((0..column.len()).map(|i| column.value(i).to_string()));
        }
        ids
    }

    const MISS: &str = "NL.IMBAG.Pand.readbench-absent";

    /// An `AsyncFileReader` that counts its `get_byte_ranges` calls.
    #[derive(Clone)]
    struct RangeCallCounter<R> {
        inner: R,
        calls: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl<R: AsyncFileReader> AsyncFileReader for RangeCallCounter<R> {
        fn get_bytes(
            &mut self,
            range: std::ops::Range<u64>,
        ) -> futures::future::BoxFuture<'_, parquet::errors::Result<bytes::Bytes>> {
            self.inner.get_bytes(range)
        }

        fn get_byte_ranges(
            &mut self,
            ranges: Vec<std::ops::Range<u64>>,
        ) -> futures::future::BoxFuture<'_, parquet::errors::Result<Vec<bytes::Bytes>>> {
            self.calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.inner.get_byte_ranges(ranges)
        }

        fn get_metadata<'a>(
            &'a mut self,
            options: Option<&'a ArrowReaderOptions>,
        ) -> futures::future::BoxFuture<'a, parquet::errors::Result<Arc<ParquetMetaData>>> {
            self.inner.get_metadata(options)
        }
    }

    /// `meta` with every `bloom_filter_length` removed — the shape of a file
    /// from a writer that sets only the offset.
    fn without_filter_lengths(meta: &ParquetMetaData) -> ParquetMetaData {
        let mut builder = meta.clone().into_builder();
        let row_groups = builder
            .take_row_groups()
            .into_iter()
            .map(|rg| {
                let columns = rg
                    .columns()
                    .iter()
                    .map(|c| {
                        c.clone()
                            .into_builder()
                            .set_bloom_filter_length(None)
                            .build()
                            .unwrap()
                    })
                    .collect();
                rg.into_builder().set_column_metadata(columns).build().unwrap()
            })
            .collect();
        builder.set_row_groups(row_groups).build()
    }

    #[tokio::test]
    async fn id_lookup_async_with_stats_matches_the_sync_stats() {
        let dir = tempfile::tempdir().unwrap();
        let (store, path) = delft_table_with(dir.path(), small_groups()).await;
        let table_file = dir.path().join(path.as_ref());
        let meta = sync_meta(&table_file);
        let mut probes: Vec<String> = every_id(&table_file).into_iter().step_by(311).collect();
        probes.push(MISS.to_string());
        for id in &probes {
            let (sync_object, sync_stats) =
                crate::query::id_lookup_with_stats(&table_file, &meta, id).unwrap();
            let (async_object, async_stats) =
                id_lookup_async_with_stats(Arc::clone(&store), &path, &meta, id)
                    .await
                    .unwrap();
            assert_eq!(async_stats, sync_stats, "{id}");
            assert_eq!(
                async_object.map(|o| o.id),
                sync_object.map(|o| o.id),
                "{id}"
            );
        }
    }

    /// Acceptance 4, on the fixture: every filter the prune needs arrives
    /// through ONE `get_byte_ranges` call, which the object store coalesces
    /// into one request (the filters sit together after the last row group).
    #[tokio::test]
    async fn the_async_prune_fetches_every_filter_with_one_ranged_call() {
        use crate::counting_store::CountingObjectStore;

        let dir = tempfile::tempdir().unwrap();
        let (_, path) = delft_table_with(dir.path(), small_groups()).await;
        let table_file = dir.path().join(path.as_ref());
        let counting = Arc::new(CountingObjectStore::new(
            LocalFileSystem::new_with_prefix(dir.path()).unwrap(),
        ));
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut reader = RangeCallCounter {
            inner: ParquetObjectReader::new(
                Arc::clone(&counting) as Arc<dyn ObjectStore>,
                path.clone(),
            ),
            calls: Arc::clone(&calls),
        };
        let arrow_meta = ArrowReaderMetadata::load_async(&mut reader, ArrowReaderOptions::new())
            .await
            .unwrap();
        let candidates: Vec<usize> = (0..arrow_meta.metadata().num_row_groups()).collect();
        assert_eq!(candidates.len(), 35);
        let before = counting.tally();
        calls.store(0, std::sync::atomic::Ordering::SeqCst);

        let prune = bloom_keep_row_groups_async(
            &mut reader,
            &arrow_meta,
            &query_core::top_level_path("id"),
            &[MISS],
            &candidates,
        )
        .await
        .unwrap();

        let after = counting.tally();
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(after.requests - before.requests, 1);
        assert!(after.bytes - before.bytes >= prune.filter_bytes);
        assert!(prune.pruned >= 1, "{prune:?}");

        let sync = crate::query::bloom_keep_row_groups(
            &std::fs::File::open(&table_file).unwrap(),
            arrow_meta.metadata(),
            &query_core::top_level_path("id"),
            &[MISS],
            &candidates,
        )
        .unwrap();
        assert_eq!(prune, sync);
    }

    /// A filter without `bloom_filter_length` is read on its own through the
    /// builder's `get_row_group_column_bloom_filter`, never through the
    /// ranged call — and prunes exactly as the ranged path does.
    #[tokio::test]
    async fn a_filter_without_a_declared_length_falls_back_to_a_per_row_group_read() {
        let dir = tempfile::tempdir().unwrap();
        let (store, path) = delft_table_with(dir.path(), small_groups()).await;
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut reader = RangeCallCounter {
            inner: ParquetObjectReader::new(store, path.clone()),
            calls: Arc::clone(&calls),
        };
        let declared = ArrowReaderMetadata::load_async(&mut reader, ArrowReaderOptions::new())
            .await
            .unwrap();
        let stripped = ArrowReaderMetadata::try_new(
            Arc::new(without_filter_lengths(declared.metadata())),
            ArrowReaderOptions::new(),
        )
        .unwrap();
        let candidates: Vec<usize> = (0..declared.metadata().num_row_groups()).collect();
        let column = query_core::top_level_path("id");

        calls.store(0, std::sync::atomic::Ordering::SeqCst);
        let fallback =
            bloom_keep_row_groups_async(&mut reader, &stripped, &column, &[MISS], &candidates)
                .await
                .unwrap();
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);

        let ranged =
            bloom_keep_row_groups_async(&mut reader, &declared, &column, &[MISS], &candidates)
                .await
                .unwrap();
        assert_eq!(fallback.keep, ranged.keep);
        assert_eq!(fallback.pruned, ranged.pruned);
    }

    #[tokio::test]
    async fn attr_filter_async_prunes_and_counts_like_the_sync_path() {
        use crate::query::AttrPredicate;

        let dir = tempfile::tempdir().unwrap();
        let (store, path) = delft_table_with(dir.path(), small_groups()).await;
        let table_file = dir.path().join(path.as_ref());
        for value in ["NL.IMBAG.Pand.0503100000012869", MISS] {
            let pred = AttrPredicate::Eq(serde_json::Value::String(value.to_string()));
            let sync_count = crate::query::attr_filter(&table_file, "identificatie", &pred).unwrap();
            let async_count = attr_filter_async(Arc::clone(&store), &path, "identificatie", &pred)
                .await
                .unwrap();
            assert_eq!(async_count, sync_count, "{value}");
        }
    }
```

`NL.IMBAG.Pand.0503100000012869` is the first Building's `identificatie` in `delft.city.jsonl`; the test compares transports, so it holds whatever that count is.

- [ ] **Step 3: Run and watch them fail**

Run: `cd lib/cityparquet-rs && cargo test -p cityparquet --all-features --lib query_async`
Expected: compile error — `cannot find function id_lookup_async_with_stats`, `cannot find function bloom_keep_row_groups_async`.

- [ ] **Step 4: Implement the async pruner and lookup**

In `query_async.rs`, replace the imports (lines 20-39) with:

```rust
use std::ops::Range;
use std::sync::Arc;

use arrow_array::RecordBatch;
use arrow_schema::SchemaRef;
use futures::TryStreamExt;
use object_store::ObjectStore;
use object_store::path::Path as ObjectPath;
use parquet::arrow::ParquetRecordBatchStreamBuilder;
use parquet::arrow::ProjectionMask;
use parquet::arrow::arrow_reader::{ArrowReaderMetadata, ArrowReaderOptions};
use parquet::arrow::async_reader::{AsyncFileReader, ParquetObjectReader};
use parquet::bloom_filter::Sbbf;
use parquet::schema::types::ColumnPath;

use cityparquet_schema::{CityMetadata, CityParquetError, Result};

use crate::decode::DecodedObject;
use crate::query::{
    AttrPredicate, AttrStats, BBoxQueryResult, BloomPrune, FullReadResult, LookupStats,
};
use crate::query_core;
use crate::reader::CityParquetReaderBuilder;

/// Used only by this module's `mod tests` (via its `use super::*`).
#[cfg(test)]
use arrow_array::{Array, StringArray};
#[cfg(test)]
use parquet::file::metadata::ParquetMetaData;
```

Before `id_lookup_async`, insert:

```rust
/// The async mirror of [`crate::query::bloom_keep_row_groups`]: every
/// declared-length filter the candidates need is fetched with ONE
/// [`AsyncFileReader::get_byte_ranges`] call on `reader` — a reader the
/// caller holds beside any builder (a clone of the same
/// [`ParquetObjectReader`] is cheap); `ParquetObjectReader` forwards it to
/// `object_store`'s `get_ranges`, which coalesces nearby ranges into few
/// requests. Under `BloomFilterPosition::End` successive filters of one
/// column are separated only by that row group's other filters, so the
/// ranges coalesce. A filter without a declared length is read on its own
/// through the builder's `get_row_group_column_bloom_filter`.
pub async fn bloom_keep_row_groups_async<R>(
    reader: &mut R,
    metadata: &ArrowReaderMetadata,
    column: &ColumnPath,
    values: &[&str],
    candidates: &[usize],
) -> Result<BloomPrune>
where
    R: AsyncFileReader + Clone + Send + 'static,
{
    let targets = query_core::bloom_targets(metadata.metadata(), column, candidates);
    let mut filters: Vec<Option<Sbbf>> = vec![None; targets.with_filter.len()];
    let mut filter_bytes = 0u64;

    let ranged: Vec<(usize, Range<u64>)> = targets
        .with_filter
        .iter()
        .enumerate()
        .filter_map(|(i, target)| {
            target
                .length
                .map(|length| (i, target.offset..target.offset + length))
        })
        .collect();
    if !ranged.is_empty() {
        let fetched = reader
            .get_byte_ranges(ranged.iter().map(|(_, range)| range.clone()).collect())
            .await
            .map_err(CityParquetError::parquet_from)?;
        for ((i, range), bytes) in ranged.iter().zip(fetched) {
            filter_bytes += range.end - range.start;
            filters[*i] = Some(Sbbf::from_bytes(&bytes).map_err(CityParquetError::parquet_from)?);
        }
    }

    if let Some(leaf) = targets.leaf {
        for (i, target) in targets.with_filter.iter().enumerate() {
            if target.length.is_some() {
                continue;
            }
            let mut builder =
                ParquetRecordBatchStreamBuilder::new_with_metadata(reader.clone(), metadata.clone());
            let sbbf = builder
                .get_row_group_column_bloom_filter(target.row_group, leaf)
                .await
                .map_err(CityParquetError::parquet_from)?;
            if let Some(sbbf) = &sbbf {
                filter_bytes += 32 * sbbf.num_blocks() as u64;
            }
            filters[i] = sbbf;
        }
    }

    let verdicts: Vec<bool> = filters
        .iter()
        .map(|filter| filter.as_ref().is_none_or(|sbbf| query_core::sbbf_matches_any(sbbf, values)))
        .collect();
    Ok(BloomPrune::from_verdicts(
        &targets,
        candidates,
        &verdicts,
        filter_bytes,
    ))
}
```

Replace the whole `id_lookup_async` function (lines 206-247, doc comment included) with:

```rust
/// The async mirror of [`crate::query::id_lookup`]. See
/// [`id_lookup_async_with_stats`].
pub async fn id_lookup_async(
    store: Arc<dyn ObjectStore>,
    path: &ObjectPath,
    meta: &CityMetadata,
    id: &str,
) -> Result<Option<DecodedObject>> {
    id_lookup_async_with_stats(store, path, meta, id)
        .await
        .map(|(object, _)| object)
}

/// The async mirror of [`crate::query::id_lookup_with_stats`]: the footer is
/// fetched once, the `id` filters with one ranged call
/// ([`bloom_keep_row_groups_async`]), and each surviving row group by a
/// stream of its own built from the shared footer, stopping at the first
/// match.
pub async fn id_lookup_async_with_stats(
    store: Arc<dyn ObjectStore>,
    path: &ObjectPath,
    meta: &CityMetadata,
    id: &str,
) -> Result<(Option<DecodedObject>, LookupStats)> {
    let mut reader = ParquetObjectReader::new(store, path.clone());
    let arrow_meta = ArrowReaderMetadata::load_async(&mut reader, ArrowReaderOptions::new())
        .await
        .map_err(CityParquetError::parquet_from)?;
    let candidates: Vec<usize> = (0..arrow_meta.metadata().num_row_groups()).collect();
    let prune = bloom_keep_row_groups_async(
        &mut reader,
        &arrow_meta,
        &query_core::top_level_path("id"),
        &[id],
        &candidates,
    )
    .await?;
    let mut stats = LookupStats::from_prune(&prune);
    let schema = ParquetRecordBatchStreamBuilder::new_with_metadata(reader.clone(), arrow_meta.clone())
        .cityparquet_arrow_schema()?;

    for row_group in prune.keep {
        stats.row_groups_read += 1;
        let builder =
            ParquetRecordBatchStreamBuilder::new_with_metadata(reader.clone(), arrow_meta.clone());
        let row_filter = query_core::utf8_eq_row_filter(builder.parquet_schema(), "id", id);
        let mut stream = builder
            .with_row_groups(vec![row_group])
            .with_row_filter(row_filter)
            .build()
            .map_err(CityParquetError::parquet_from)?;
        while let Some(batch) = stream
            .try_next()
            .await
            .map_err(CityParquetError::parquet_from)?
        {
            if batch.num_rows() == 0 {
                continue;
            }
            let batch = restamp(batch, &schema)?;
            if let Some(object) = query_core::first_decoded_object(&batch, meta)? {
                return Ok((Some(object), stats));
            }
        }
    }
    Ok((None, stats))
}
```

In `attr_filter_async`, replace everything from `let reader = ParquetObjectReader::new(store, path.clone());` to the `.map_err(CityParquetError::parquet_from)?;` that ends the `let mut stream = builder ... .build()` statement with:

```rust
    let mut reader = ParquetObjectReader::new(store, path.clone());
    let arrow_meta = ArrowReaderMetadata::load_async(&mut reader, ArrowReaderOptions::new())
        .await
        .map_err(CityParquetError::parquet_from)?;
    let builder =
        ParquetRecordBatchStreamBuilder::new_with_metadata(reader.clone(), arrow_meta.clone());

    query_core::require_column(builder.schema(), column)?;

    let output_mask = ProjectionMask::columns(builder.parquet_schema(), [column]);
    let row_filter = query_core::attr_predicate_row_filter(builder.parquet_schema(), column, pred);

    let mut builder = builder
        .with_projection(output_mask)
        .with_row_filter(row_filter);
    if let Some(value) = query_core::bloom_probe_value(pred) {
        let candidates: Vec<usize> = (0..arrow_meta.metadata().num_row_groups()).collect();
        let prune = bloom_keep_row_groups_async(
            &mut reader,
            &arrow_meta,
            &query_core::top_level_path(column),
            &[value],
            &candidates,
        )
        .await?;
        builder = builder.with_row_groups(prune.keep);
    }
    let mut stream = builder.build().map_err(CityParquetError::parquet_from)?;
```

and extend its doc comment with: `/// String equality prunes with `column`'s bloom filters first, exactly as the sync path does ([`bloom_keep_row_groups_async`]).`

The existing test `id_lookup_async_matches_sync_id_lookup_on_a_real_fixture` uses `StringArray` through `use super::*`; the new `#[cfg(test)] use arrow_array::{Array, StringArray};` keeps it compiling.

- [ ] **Step 5: Run the async tests**

Run: `cd lib/cityparquet-rs && cargo test -p cityparquet --all-features --lib query_async`
Expected: all eleven `query_async::tests` pass.

- [ ] **Step 6: Run the library gate**

Run: `cd lib/cityparquet-rs && just check`
Expected: PASS, with the async tests now among those run.

- [ ] **Step 7: Commit**

```bash
git add lib/cityparquet-rs/justfile lib/cityparquet-rs/crates/core/Cargo.toml lib/cityparquet-rs/Cargo.lock \
  lib/cityparquet-rs/crates/core/src/query_async.rs
git commit -m "feat(query): async bloom pruning in one ranged request

bloom_keep_row_groups_async fetches every declared-length filter with a
single get_byte_ranges call and falls back to
get_row_group_column_bloom_filter for a filter without a length.
id_lookup_async_with_stats and string-equality attr_filter_async use it.
The library gate runs with --all-features so the object-store transport
is tested and linted."
```

---

### Task 5: `feature_id` lookup — table, async and package level

`feature_lookup` returns every row whose `feature_id` equals the target — a feature and all its parts — so it never stops early: it bloom-prunes on `feature_id` and reads the surviving row groups to the end with an exact equality `RowFilter`.

**Files:**

- Modify: `lib/cityparquet-rs/crates/core/src/query.rs` — new functions after `id_lookup_with_stats`; module doc.
- Modify: `lib/cityparquet-rs/crates/core/src/query_async.rs` — new functions after `id_lookup_async_with_stats`; tests.
- Modify: `lib/cityparquet-rs/crates/core/tests/bloom_real_data.rs` — feature lookup tests.

**Interfaces:**

- Consumes: `bloom_keep_row_groups`, `bloom_keep_row_groups_async`, `LookupStats` (+ `AddAssign`, `from_prune`), `utf8_eq_row_filter`, `top_level_path` (Tasks 3-4).
- Produces:
  - `pub fn feature_lookup(table_path: &Path, meta: &CityMetadata, feature_id: &str) -> Result<Vec<DecodedObject>>`
  - `pub fn feature_lookup_with_stats(table_path: &Path, meta: &CityMetadata, feature_id: &str) -> Result<(Vec<DecodedObject>, LookupStats)>` — `row_groups_read == keep.len()`.
  - `pub fn package_feature_lookup(package_dir: &Path, feature_id: &str) -> Result<Vec<DecodedObject>>`
  - `pub fn package_feature_lookup_with_stats(package_dir: &Path, feature_id: &str) -> Result<(Vec<DecodedObject>, LookupStats)>` — iterates the object tables `metadata.json` names, in manifest order, summing the statistics.
  - `pub async fn feature_lookup_async(store: Arc<dyn ObjectStore>, path: &ObjectPath, meta: &CityMetadata, feature_id: &str) -> Result<Vec<DecodedObject>>`
  - `pub async fn feature_lookup_async_with_stats(store: Arc<dyn ObjectStore>, path: &ObjectPath, meta: &CityMetadata, feature_id: &str) -> Result<(Vec<DecodedObject>, LookupStats)>`
  - Objects are returned in table row order.

- [ ] **Step 1: Write the failing sync tests**

In `bloom_real_data.rs`, extend the `cityparquet::query` import with `feature_lookup, feature_lookup_with_stats, package_feature_lookup, package_feature_lookup_with_stats` and append:

```rust
/// `feature_id` -> the ids of its rows, read straight off the table.
fn rows_by_feature(table: &Path) -> BTreeMap<String, Vec<String>> {
    let ids = values_by_row_group(table, "id");
    let features = values_by_row_group(table, "feature_id");
    assert_eq!(ids.len(), features.len(), "feature_id is non-null on every row");
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for ((_, id), (_, feature)) in ids.into_iter().zip(features) {
        out.entry(feature).or_default().push(id);
    }
    out
}

/// Acceptance 4b on the fixture: every row of every multi-part delft feature
/// (a Building and its BuildingParts), identical with and without filters.
#[test]
fn feature_lookup_returns_every_part_with_and_without_filters() {
    let on = convert_with("delft.city.jsonl", delft_recipe(64));
    let off = convert_with("delft.city.jsonl", nobloom(64));
    let on_table = on.path().join("building.parquet");
    let off_table = off.path().join("building.parquet");
    let expected = rows_by_feature(&on_table);
    let multi_part: Vec<(&String, &Vec<String>)> =
        expected.iter().filter(|(_, ids)| ids.len() >= 2).collect();
    assert!(multi_part.len() >= 1000, "delft features are Building + BuildingPart");
    let largest = expected.values().map(Vec::len).max().unwrap();
    assert_eq!(largest, 3, "delft has one three-object feature");

    let mut probes: Vec<(&String, &Vec<String>)> =
        multi_part.iter().step_by(53).copied().collect();
    probes.extend(expected.iter().filter(|(_, ids)| ids.len() == 3));
    for (feature, ids) in probes {
        for table in [&on_table, &off_table] {
            let (objects, stats) =
                feature_lookup_with_stats(table, &table_meta(table), feature).unwrap();
            let got: Vec<String> = objects.iter().map(|o| o.id.clone()).collect();
            assert_eq!(&got, ids, "{feature} in {}", table.display());
            assert!(
                objects.iter().all(|o| o.feature_id.as_deref() == Some(feature.as_str())),
                "{feature}"
            );
            assert_eq!(stats.row_groups_total, 35);
            assert_eq!(stats.row_groups_read, 35 - stats.bloom_pruned);
        }
        let plain = feature_lookup(&on_table, &table_meta(&on_table), feature).unwrap();
        assert_eq!(plain.len(), ids.len());
    }
}

#[test]
fn a_feature_miss_is_pruned_and_returns_nothing() {
    let on = convert_with("delft.city.jsonl", delft_recipe(64));
    let table = on.path().join("building.parquet");
    let (objects, stats) = feature_lookup_with_stats(&table, &table_meta(&table), MISS).unwrap();
    assert!(objects.is_empty());
    assert!(stats.bloom_pruned >= 1, "{stats:?}");
    assert!(stats.filter_bytes > 0);
}

/// The package form walks every object table the manifest names: on the
/// multi-table railway package, a feature's rows come back from every table
/// that holds them, and the statistics sum over the tables.
#[test]
fn package_feature_lookup_spans_every_object_table() {
    let out = convert_with("lod3_railway.city.json", WriterRecipe::default());
    let tables = PackageTables::open(out.path()).unwrap();
    assert!(tables.tables.len() > 1, "railway is a multi-table package");

    let mut expected: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for table in &tables.tables {
        for (feature, ids) in rows_by_feature(table) {
            expected.entry(feature).or_default().extend(ids);
        }
    }
    let row_groups: usize = tables.tables.iter().map(|t| row_group_count(t)).sum();

    for (feature, ids) in &expected {
        let (objects, stats) = package_feature_lookup_with_stats(out.path(), feature).unwrap();
        let mut got: Vec<String> = objects.iter().map(|o| o.id.clone()).collect();
        let mut want = ids.clone();
        got.sort();
        want.sort();
        assert_eq!(got, want, "{feature}");
        assert_eq!(stats.row_groups_total, row_groups);
        assert_eq!(
            package_feature_lookup(out.path(), feature).unwrap().len(),
            ids.len()
        );
    }
    let (objects, _) = package_feature_lookup_with_stats(out.path(), MISS).unwrap();
    assert!(objects.is_empty());
}
```

- [ ] **Step 2: Run and watch them fail**

Run: `cd lib/cityparquet-rs && cargo test -p cityparquet --test bloom_real_data feature`
Expected: compile error — `unresolved imports cityparquet::query::feature_lookup` (and the three siblings).

- [ ] **Step 3: Implement the sync forms**

In `query.rs`, add `use crate::decode::decode_batch;` beside the `DecodedObject` import, extend the module doc's **Bloom filters** paragraph so its list reads `the identifier lookups ([`id_lookup_with_stats`], [`feature_lookup_with_stats`], [`package_feature_lookup_with_stats`])`, and after `id_lookup_with_stats` insert:

```rust
/// Every object whose `feature_id` equals `feature_id` — a feature and all
/// its parts — in table row order; empty if none. See
/// [`feature_lookup_with_stats`].
pub fn feature_lookup(
    table_path: &Path,
    meta: &CityMetadata,
    feature_id: &str,
) -> Result<Vec<DecodedObject>> {
    feature_lookup_with_stats(table_path, meta, feature_id).map(|(objects, _)| objects)
}

/// [`feature_lookup`] with what it cost. The `feature_id` bloom filters
/// prune the row groups first; the survivors are then read to the end — a
/// feature's rows may span row groups, so there is no early stop — with an
/// exact `feature_id` equality `RowFilter`, and every matching row is
/// decoded in full. [`LookupStats::row_groups_read`] is therefore the number
/// of row groups kept.
pub fn feature_lookup_with_stats(
    table_path: &Path,
    meta: &CityMetadata,
    feature_id: &str,
) -> Result<(Vec<DecodedObject>, LookupStats)> {
    let file = File::open(table_path)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file.try_clone()?)
        .map_err(CityParquetError::parquet_from)?;
    let schema = builder.cityparquet_arrow_schema()?;
    let candidates: Vec<usize> = (0..builder.metadata().num_row_groups()).collect();
    let prune = bloom_keep_row_groups(
        &file,
        builder.metadata(),
        &query_core::top_level_path("feature_id"),
        &[feature_id],
        &candidates,
    )?;
    let mut stats = LookupStats::from_prune(&prune);
    stats.row_groups_read = prune.keep.len();

    let row_filter =
        query_core::utf8_eq_row_filter(builder.parquet_schema(), "feature_id", feature_id);
    let parquet_reader = builder
        .with_row_groups(prune.keep)
        .with_row_filter(row_filter)
        .build()
        .map_err(CityParquetError::parquet_from)?;
    let reader = CityParquetRecordBatchReader::new(parquet_reader, schema);

    let mut objects = Vec::new();
    for batch in reader {
        objects.extend(decode_batch(&batch?, meta)?);
    }
    Ok((objects, stats))
}

/// [`feature_lookup`] over a whole package: every object table the
/// package's `metadata.json` names, in manifest order. See
/// [`package_feature_lookup_with_stats`].
pub fn package_feature_lookup(package_dir: &Path, feature_id: &str) -> Result<Vec<DecodedObject>> {
    package_feature_lookup_with_stats(package_dir, feature_id).map(|(objects, _)| objects)
}

/// [`feature_lookup_with_stats`] per object table, the objects concatenated
/// in manifest order and the statistics summed. Each table's own footer
/// supplies the metadata its rows decode with.
pub fn package_feature_lookup_with_stats(
    package_dir: &Path,
    feature_id: &str,
) -> Result<(Vec<DecodedObject>, LookupStats)> {
    let tables = crate::stac::properties::PackageTables::open(package_dir)?;
    let mut objects = Vec::new();
    let mut stats = LookupStats::default();
    for table in &tables.tables {
        let meta = ParquetRecordBatchReaderBuilder::try_new(File::open(table)?)
            .map_err(CityParquetError::parquet_from)?
            .cityparquet_metadata()?;
        let (found, table_stats) = feature_lookup_with_stats(table, &meta, feature_id)?;
        objects.extend(found);
        stats += table_stats;
    }
    Ok((objects, stats))
}
```

- [ ] **Step 4: Run the sync tests**

Run: `cd lib/cityparquet-rs && cargo test -p cityparquet --test bloom_real_data feature`
Expected: the three feature tests pass.

- [ ] **Step 5: Write the failing async test**

In `query_async.rs` `mod tests`, append:

```rust
    #[tokio::test]
    async fn feature_lookup_async_matches_the_sync_lookup() {
        let dir = tempfile::tempdir().unwrap();
        let (store, path) = delft_table_with(dir.path(), small_groups()).await;
        let table_file = dir.path().join(path.as_ref());
        let meta = sync_meta(&table_file);
        let first = crate::query::id_lookup(&table_file, &meta, &every_id(&table_file)[0])
            .unwrap()
            .unwrap();
        let feature = first.feature_id.clone().expect("feature_id is non-null");
        for target in [feature.as_str(), MISS] {
            let (sync_objects, sync_stats) =
                crate::query::feature_lookup_with_stats(&table_file, &meta, target).unwrap();
            let (async_objects, async_stats) =
                feature_lookup_async_with_stats(Arc::clone(&store), &path, &meta, target)
                    .await
                    .unwrap();
            assert_eq!(async_stats, sync_stats, "{target}");
            let ids = |objects: &[DecodedObject]| -> Vec<String> {
                objects.iter().map(|o| o.id.clone()).collect()
            };
            assert_eq!(ids(&async_objects), ids(&sync_objects), "{target}");
            assert_eq!(
                feature_lookup_async(Arc::clone(&store), &path, &meta, target)
                    .await
                    .unwrap()
                    .len(),
                sync_objects.len()
            );
        }
    }
```

- [ ] **Step 6: Run and watch it fail**

Run: `cd lib/cityparquet-rs && cargo test -p cityparquet --all-features --lib query_async::tests::feature`
Expected: compile error — `cannot find function feature_lookup_async_with_stats`.

- [ ] **Step 7: Implement the async forms**

In `query_async.rs`, after `id_lookup_async_with_stats`, insert:

```rust
/// The async mirror of [`crate::query::feature_lookup`].
pub async fn feature_lookup_async(
    store: Arc<dyn ObjectStore>,
    path: &ObjectPath,
    meta: &CityMetadata,
    feature_id: &str,
) -> Result<Vec<DecodedObject>> {
    feature_lookup_async_with_stats(store, path, meta, feature_id)
        .await
        .map(|(objects, _)| objects)
}

/// The async mirror of [`crate::query::feature_lookup_with_stats`]: the
/// `feature_id` filters arrive in one ranged call
/// ([`bloom_keep_row_groups_async`]) and the surviving row groups are read
/// to the end by one stream.
pub async fn feature_lookup_async_with_stats(
    store: Arc<dyn ObjectStore>,
    path: &ObjectPath,
    meta: &CityMetadata,
    feature_id: &str,
) -> Result<(Vec<DecodedObject>, LookupStats)> {
    let mut reader = ParquetObjectReader::new(store, path.clone());
    let arrow_meta = ArrowReaderMetadata::load_async(&mut reader, ArrowReaderOptions::new())
        .await
        .map_err(CityParquetError::parquet_from)?;
    let candidates: Vec<usize> = (0..arrow_meta.metadata().num_row_groups()).collect();
    let prune = bloom_keep_row_groups_async(
        &mut reader,
        &arrow_meta,
        &query_core::top_level_path("feature_id"),
        &[feature_id],
        &candidates,
    )
    .await?;
    let mut stats = LookupStats::from_prune(&prune);
    stats.row_groups_read = prune.keep.len();

    let builder = ParquetRecordBatchStreamBuilder::new_with_metadata(reader, arrow_meta);
    let schema = builder.cityparquet_arrow_schema()?;
    let row_filter =
        query_core::utf8_eq_row_filter(builder.parquet_schema(), "feature_id", feature_id);
    let mut stream = builder
        .with_row_groups(prune.keep)
        .with_row_filter(row_filter)
        .build()
        .map_err(CityParquetError::parquet_from)?;

    let mut objects = Vec::new();
    while let Some(batch) = stream
        .try_next()
        .await
        .map_err(CityParquetError::parquet_from)?
    {
        if batch.num_rows() == 0 {
            continue;
        }
        let batch = restamp(batch, &schema)?;
        objects.extend(crate::decode::decode_batch(&batch, meta)?);
    }
    Ok((objects, stats))
}
```

Update `restamp`'s doc comment, which names its callers: replace `only [`full_read_async`]/`id_lookup_async` need it.` with `only the full-row readers ([`full_read_async`] and the identifier lookups) need it.`

- [ ] **Step 8: Run the tests and the gate**

Run: `cd lib/cityparquet-rs && cargo test -p cityparquet --all-features --lib query_async && just check`
Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add lib/cityparquet-rs/crates/core/src/query.rs lib/cityparquet-rs/crates/core/src/query_async.rs \
  lib/cityparquet-rs/crates/core/tests/bloom_real_data.rs
git commit -m "feat(query): feature_id lookup pruned by bloom filters

feature_lookup returns every row of a feature — the feature and its
parts — after pruning on the feature_id filters, reading the survivors
to the end. Sync, async and package-level forms, each with a
_with_stats variant."
```

---

### Task 6: duckdb-cityjson writes the same policy

`cityparquet_write` states its row-group size, raises the object tables' dictionary cut-off to it so `id`, `feature_id` and every string column whose dictionary page fits in 8 MiB are dictionary-encoded — the only chunks DuckDB filters — and turns filters on at FPP 0.01. Sidecars carry none. A `bloom` named parameter (default `true`) mirrors `--no-bloom`. Work happens in the submodule; the monorepo bumps the pointer.

**Files:**

- Modify: `lib/duckdb-cityjson/src/cityjson/cityparquet_write.cpp` — `WriteBindData` (64-89), a new helper before the anonymous namespace closes (line 421), `WriteBind` (432-438), the `COPY` in `write_table` (630-633), `RegisterCityParquetWriteFunction` (779-785).
- Create: `lib/duckdb-cityjson/test/sql/cityparquet_bloom.test`.
- Modify: `lib/duckdb-cityjson/docs/FUNCTIONS.md` — "The package round trip" (lines 787-789).
- Modify (monorepo): the `lib/duckdb-cityjson` submodule pointer and `ai/mcp/corpus/corpus.json`.

**Interfaces:**

- Consumes: nothing from the Rust tasks (the extension depends on the specification, never on cityparquet-rs internals).
- Produces: `cityparquet_write(schema, directory, crs => …, source_format => …, bloom => BOOLEAN)`; object-table `COPY` options `ROW_GROUP_SIZE 122880, DICTIONARY_SIZE_LIMIT 122880, STRING_DICTIONARY_PAGE_SIZE_LIMIT 8388608, WRITE_BLOOM_FILTER true, BLOOM_FILTER_FALSE_POSITIVE_RATIO 0.01` (with `bloom => false`: `ROW_GROUP_SIZE 122880, WRITE_BLOOM_FILTER false`); sidecar `COPY` options `ROW_GROUP_SIZE 122880, WRITE_BLOOM_FILTER false`.

- [ ] **Step 1: Put the submodule on `develop`**

The submodule is detached at `origin/develop` (`39c9de4`) with a clean tree.

```bash
git -C lib/duckdb-cityjson status --short          # expect no output
git -C lib/duckdb-cityjson fetch origin
git -C lib/duckdb-cityjson switch -c develop --track origin/develop \
  || git -C lib/duckdb-cityjson switch develop
git -C lib/duckdb-cityjson merge --ff-only origin/develop
git -C lib/duckdb-cityjson branch --show-current   # expect: develop
(cd lib/duckdb-cityjson && just hooks)
```

- [ ] **Step 2: Write the failing sqllogictest**

Create `lib/duckdb-cityjson/test/sql/cityparquet_bloom.test`:

```
# name: test/sql/cityparquet_bloom.test
# description: cityparquet_write writes bloom filters on the identifier columns, after the data, none on sidecars; bloom => false writes none
# group: [sql]

require cityjson

require parquet

statement ok
CREATE SCHEMA bl;

statement ok
CREATE TABLE bl.building AS SELECT * FROM read_cityjsonseq('test/data/delft_subset.city.jsonl');

statement ok
PRAGMA cityparquet_init('bl');

statement ok
SELECT COUNT(*) FROM cityparquet_write('bl', '__TEST_DIR__/bloom_on', crs => 'EPSG:7415');

# The identifier columns carry a filter in every row group. With DuckDB's
# default cut-off (a fifth of the row group's rows) they would be PLAIN, and
# DuckDB filters dictionary-encoded chunks only.
query I
SELECT string_agg(path_in_schema, ',' ORDER BY path_in_schema)
FROM (
  SELECT path_in_schema
  FROM parquet_metadata('__TEST_DIR__/bloom_on/building.parquet')
  WHERE path_in_schema IN ('id', 'feature_id', 'identificatie')
  GROUP BY path_in_schema
  HAVING bool_and(bloom_filter_offset IS NOT NULL)
);
----
feature_id,id,identificatie

# Every filter starts after the last byte of column data and declares its length.
query II
SELECT
  bool_and(m.bloom_filter_offset >= e.data_end),
  bool_and(m.bloom_filter_length IS NOT NULL)
FROM parquet_metadata('__TEST_DIR__/bloom_on/building.parquet') m,
  (SELECT max(coalesce(nullif(dictionary_page_offset, 0), data_page_offset) + total_compressed_size) AS data_end
   FROM parquet_metadata('__TEST_DIR__/bloom_on/building.parquet')) e
WHERE m.bloom_filter_offset IS NOT NULL;
----
true	true

statement ok
SELECT COUNT(*) FROM cityparquet_write('bl', '__TEST_DIR__/bloom_off', crs => 'EPSG:7415', bloom => false);

query I
SELECT COUNT(*) FROM parquet_metadata('__TEST_DIR__/bloom_off/building.parquet')
WHERE bloom_filter_offset IS NOT NULL;
----
0

# Sidecars never carry a filter, whatever DuckDB would dictionary-encode in them.
statement ok
CREATE SCHEMA bl_tpl;

statement ok
CREATE TABLE bl_tpl.vegetation AS
  SELECT * FROM read_cityjson('test/data/geometry_templates.city.json', appearance := 'sidecar');

statement ok
CREATE TABLE bl_tpl.geometry_templates AS
  SELECT * FROM cityjson_geometry_templates('test/data/geometry_templates.city.json');

statement ok
PRAGMA cityparquet_init('bl_tpl');

statement ok
SELECT COUNT(*) FROM cityparquet_write('bl_tpl', '__TEST_DIR__/bloom_tpl', crs => 'EPSG:7415');

query I
SELECT COUNT(*) FROM parquet_metadata('__TEST_DIR__/bloom_tpl/geometry_templates.parquet')
WHERE bloom_filter_offset IS NOT NULL;
----
0

query I
SELECT COUNT(*) > 0 FROM parquet_metadata('__TEST_DIR__/bloom_tpl/vegetation.parquet')
WHERE path_in_schema = 'id' AND bloom_filter_offset IS NOT NULL;
----
true
```

- [ ] **Step 3: Run it and watch it fail**

Run: `cd lib/duckdb-cityjson && just rebuild && ./build/release/test/unittest "test/sql/cityparquet_bloom.test"`
Expected: FAIL on the first `query I` (the result is empty, not `feature_id,id,identificatie`), and — once that is fixed — on `bloom => false` with a binder error for the unknown named parameter.

- [ ] **Step 4: Implement the options**

In `WriteBindData`, after `std::string source_format;` add:

```cpp
	//! Write Parquet bloom filters on the object tables (`bloom => false` writes none).
	bool bloom = true;
```

In its `Copy()`, after `result->source_format = source_format;` add `result->bloom = bloom;`, and change `Equals` to:

```cpp
		return catalog == o.catalog && schema == o.schema && directory == o.directory && bloom == o.bloom;
```

Inside the anonymous namespace, directly before its closing `} // namespace` (line 421), add:

```cpp
//! Row-group size of every package COPY -- DuckDB's own default, stated so the
//! object tables' dictionary cut-off below can equal it.
constexpr idx_t PACKAGE_ROW_GROUP_SIZE = 122880;
//! Cap on one string dictionary page. Identifier dictionaries stay far below
//! it; a WKB or JSON column's page exceeds it and is written PLAIN, unfiltered.
constexpr idx_t STRING_DICTIONARY_PAGE_LIMIT = 8388608;

//! The COPY options that decide bloom filters (spec 02-object-table-schema.mdx,
//! "Bloom filters"). DuckDB writes a filter only for a dictionary-encoded chunk,
//! and its dictionary and bloom options are file-wide, so an object table raises
//! the dictionary cut-off to the row-group size: `id`, `feature_id` and every
//! other string column whose dictionary page fits under the cap is
//! dictionary-encoded and filtered. That is a superset of the reference writer's
//! columns, documented in 06-resources/02-software.mdx. Sidecars carry none.
std::string BloomCopyOptions(bool is_object, bool bloom) {
	const auto row_groups = ", ROW_GROUP_SIZE " + std::to_string(PACKAGE_ROW_GROUP_SIZE);
	if (!is_object || !bloom) {
		return row_groups + ", WRITE_BLOOM_FILTER false";
	}
	return row_groups + ", DICTIONARY_SIZE_LIMIT " + std::to_string(PACKAGE_ROW_GROUP_SIZE) +
	       ", STRING_DICTIONARY_PAGE_SIZE_LIMIT " + std::to_string(STRING_DICTIONARY_PAGE_LIMIT) +
	       ", WRITE_BLOOM_FILTER true, BLOOM_FILTER_FALSE_POSITIVE_RATIO 0.01";
}
```

In `WriteBind`, extend the named-parameter loop:

```cpp
		} else if (entry.first == "source_format") {
			result->source_format = StringValue::Get(entry.second);
		} else if (entry.first == "bloom") {
			result->bloom = BooleanValue::Get(entry.second);
		}
```

Replace the `COPY` statement (lines 630-633):

```cpp
		Run(connection, "COPY (SELECT " +
		                    CopySourceList(context, bind_data.schema, table, legal_geometry, crs_annotation) +
		                    " FROM " + QualifiedName(bind_data.catalog, bind_data.schema, table) + ") TO " +
		                    Literal(path) + " (FORMAT PARQUET, GEOPARQUET_VERSION 'none', KV_METADATA {" + kv + "});");
```

with:

```cpp
		Run(connection, "COPY (SELECT " +
		                    CopySourceList(context, bind_data.schema, table, legal_geometry, crs_annotation) +
		                    " FROM " + QualifiedName(bind_data.catalog, bind_data.schema, table) + ") TO " +
		                    Literal(path) + " (FORMAT PARQUET, GEOPARQUET_VERSION 'none', KV_METADATA {" + kv + "}" +
		                    BloomCopyOptions(is_object, bind_data.bloom) + ");");
```

In `RegisterCityParquetWriteFunction`, after the `source_format` named parameter add:

```cpp
	func.named_parameters["bloom"] = LogicalType(LogicalTypeId::BOOLEAN);
```

- [ ] **Step 5: Run the new test, then the suite**

Run: `cd lib/duckdb-cityjson && just rebuild && ./build/release/test/unittest "test/sql/cityparquet_bloom.test" && make test`
Expected: the new file passes; the whole sqllogictest suite passes.

- [ ] **Step 6: Document the parameter**

In `lib/duckdb-cityjson/docs/FUNCTIONS.md`, replace:

```markdown
It takes two named parameters: `crs` (below) and `source_format`, which records
the format the data originally came from into each file's `city` footer as
`source_format`.
```

with:

```markdown
It takes three named parameters: `crs` (below); `source_format`, which records
the format the data originally came from into each file's `city` footer as
`source_format`; and `bloom` (default `true`), which writes Parquet bloom
filters on the object tables. DuckDB writes a filter only for a
dictionary-encoded column chunk, and its dictionary and bloom options apply to a
whole file, so each object table is written with 122 880-row row groups and a
dictionary cut-off of the same size under an 8 MiB dictionary-page cap: `id`,
`feature_id` and every other string column whose dictionary fits carry a filter
(FPP 0.01), placed after the last row group. WKB geometry and JSON columns
exceed the cap and stay unfiltered, except in a row group small enough for their
dictionary to fit. Sidecars carry no filter; `bloom => false` writes none at
all. DuckDB itself consults the filters for `=` and `IN` predicates, including
those pushed down from a join.
```

- [ ] **Step 7: Commit in the submodule**

```bash
git -C lib/duckdb-cityjson add src/cityjson/cityparquet_write.cpp test/sql/cityparquet_bloom.test docs/FUNCTIONS.md
git -C lib/duckdb-cityjson commit -m "feat: cityparquet_write writes bloom filters on the identifier columns

Object tables are written with 122880-row groups, a dictionary cut-off of
the same size and an 8 MiB dictionary-page cap, and bloom filters at FPP
0.01, so id, feature_id and the other short string columns carry a filter
after the last row group. Sidecars carry none. A bloom named parameter
(default true) turns them off."
```

- [ ] **Step 8: Bump the pointer and regenerate the MCP corpus in the monorepo**

```bash
just mcp-corpus
just mcp-check
git add lib/duckdb-cityjson ai/mcp/corpus/corpus.json
git commit -m "chore(duckdb-cityjson): bloom filters in cityparquet_write

Bumps the extension to the commit that writes bloom filters on the
object tables, and regenerates the MCP corpus from its FUNCTIONS.md."
```

Expected: `just mcp-check` passes. The submodule commit must be pushed to `origin/develop` before this pointer is pushed.

---

### Task 7: Specification and documentation

**Files:**

- Modify: `documents/docs/03-specification/02-object-table-schema.mdx` — a `### Bloom filters` subsection at the end of "## Physical encoding and conformance" (after line 168, before `## Attribute types and promotion`).
- Modify: `documents/docs/03-specification/05-metadata.mdx` — "## Recommended writer defaults (informative)" (line 193-195).
- Create: `documents/docs/04-design-decisions/07-bloom-filters.mdx`.
- Modify: `documents/docs/04-design-decisions/meta.ts` — `pages`.
- Modify: `documents/docs/04-design-decisions/index.mdx` — `CardGroup`.
- Modify: `documents/docs/06-resources/02-software.mdx` — "Current implementation support" table (64-71) and the DuckDB CityJSON section (after line 108).
- Modify: `lib/cityparquet-rs/docs/architecture.md` — recipe table (195-205) and "Testing discipline" (209-214).
- Modify: `docs/superpowers/specs/2026-08-25-readbench-query-design-design.md` — the out-of-scope bullet (lines 64-69).
- Modify: `ai/mcp/corpus/corpus.json` (regenerated).

**Interfaces:**

- Consumes: the behaviour of Tasks 1-6 (column rule, FPP, placement, the duckdb-cityjson divergence).
- Produces: the anchor `/specification/object-table-schema#bloom-filters` and the page `/design-decisions/bloom-filters`, which later docs link to.

- [ ] **Step 1: The normative text**

In `02-object-table-schema.mdx`, directly after the paragraph that ends `plausible values attached to the wrong column.` (line 168), add:

```mdx
### Bloom filters

Writers **MAY** include standard Parquet bloom filters. Writers targeting
selective identifier lookups **SHOULD** include filters on `id` and
`feature_id`. Filters **MUST** conform to the Parquet bloom-filter
specification; writers **SHOULD** set `bloom_filter_length`. Readers **MUST**
accept files without filters, **MUST NOT** depend on filter placement, and
**MUST NOT** treat a positive bloom result as a match.

The recommended column set, false-positive probability and placement are in
[recommended writer defaults](/specification/metadata#recommended-writer-defaults-informative);
the reasoning is in the [bloom-filter decision](/design-decisions/bloom-filters).
```

- [ ] **Step 2: The informative defaults**

In `05-metadata.mdx`, after the paragraph under `## Recommended writer defaults (informative)`, add:

```mdx
Bloom filters, at a false-positive probability of **0.01**: on every object
table's `id` and `feature_id`, and on each scalar `VARCHAR` attribute column
(not JSON) whose distinct values number at least **0.2 ×** its non-null values
across the dataset. The rule is decided once for the whole dataset, before the
first row group is written, so every file of a package — and every partition
of a partitioned dataset — carries the same column set. All filters sit
together after the last row group, before the page indexes and footer, with
`bloom_filter_length` set, so a reader fetches every filter of a column in one
range request. Low-cardinality strings, numeric, list, JSON and geometry
columns, and the sidecars, carry none.
```

- [ ] **Step 3: The decision page**

Create `documents/docs/04-design-decisions/07-bloom-filters.mdx`:

```mdx
---
title: G · Bloom filters
description: Parquet bloom filters on the identifier columns, placed together after the last row group.
sidebar:
  label: G · Bloom filters
---

How CityParquet lets a lookup by `id` or `feature_id` skip the row groups that
cannot hold it. Row-group min/max statistics cannot: 3DBAG's
`NL.IMBAG.Pand.…` identifiers span almost the same range in every row group,
and Hilbert ordering spreads them further. The prior art weighed here is
DuckDB's Parquet writer and the Parquet bloom-filter specification.

## Filters on the identifiers and high-cardinality strings, placed at the end

<Badge variant="success">decided</Badge>

**Decision.** Writers put standard Parquet bloom filters on every object
table's `id` and `feature_id`, and on each scalar string attribute column whose
distinct values reach 0.2 of its non-null values across the dataset, at a
false-positive probability of 0.01. All filters sit together after the last row
group, before the page indexes and footer, with `bloom_filter_length` set.
Sidecars and numeric, JSON, list and geometry columns carry none. See the
[specification](/specification/object-table-schema#bloom-filters).

**Alternatives considered.**

- **DuckDB's column rule** writes a filter only for a dictionary-encoded chunk.
  The reference writer encodes `id` and `feature_id` with `DELTA_BYTE_ARRAY` and
  no dictionary, so that rule would skip exactly the columns lookups need.
  CityParquet follows DuckDB's _placement_, not its column rule.
- **A filter after each row group** (parquet-rs's default placement) keeps
  writer memory flat, but scatters a column's filters across the file, so a
  remote reader needs a request per row group.
- **Filters on numeric attributes**: an integer equality predicate compares
  through `f64`, so an exact-integer probe could reject a row the predicate
  accepts above 2^53. They wait for typed integer equality.

**Why this.** `id` and `feature_id` are what lookups and joins select on —
`feature_id` joins a feature's parts to the feature — and both are unique or
nearly so, which is where a filter prunes and statistics do not. With every
filter in one place a remote reader fetches all of a column's filters in one
coalesced range request, the access pattern object storage rewards. A positive
result is never a match: the reader's exact predicate still decides every row.

**Trade-off.** Filters cost bytes on disk (folded to the values actually
present) and, placed at the end, writer memory: every completed filter stays in
memory until its file is closed. The high-cardinality rule is decided for the
whole dataset before the first row group is written, so a sparse column, or a
vocabulary repeated across row groups, can be judged differently than a
per-row-group rule would. duckdb-cityjson, bound to DuckDB's file-wide options,
writes a superset of these columns (see [software](/resources/software)).

**Status.** Decided. Its effect on lookup latency, bytes and requests is
measured by the `bloom` benchmark family.
```

In `04-design-decisions/meta.ts`, add `"bloom-filters",` after `"extensions",` in `pages`. In `04-design-decisions/index.mdx`, after the `F · Extensions` card, add:

```mdx
  <Card title="G · Bloom filters" href="/design-decisions/bloom-filters" icon="filter">
    Filters on the identifiers, placed after the last row group.
  </Card>
```

- [ ] **Step 4: Implementation status and the divergence**

In `06-resources/02-software.mdx`, replace the `| Reading | … |` row with:

```markdown
| Reading | Arrow record batches, column projection, attribute filtering, bounding-box pruning, bloom-filter pruning for `id`, `feature_id` and string-equality lookups (every filter of a column fetched in one range request over object storage), and optional asynchronous object-store access |
```

and add a row after `| Metadata and CRS | … |`:

```markdown
| Bloom filters | On `id`, `feature_id` and high-cardinality string attributes at FPP 0.01, after the last row group; `--no-bloom` turns them off |
```

After the bullet list of package operations — its last bullet ends `structure and manage unreferenced sidecar rows.` (line 106) — and before `Use the package-writing API when you need the complete package contract.`, add:

```mdx
`cityparquet_write` writes Parquet bloom filters under its `bloom` option (on by
default). DuckDB writes a filter only for a dictionary-encoded column chunk and
applies its dictionary and bloom options to a whole file, so the extension
writes each object table with 122 880-row row groups and a dictionary cut-off of
the same size, under an 8 MiB dictionary-page cap. `id`, `feature_id` and the
high-cardinality string attributes carry filters, as the recommended defaults
prescribe; so do the low-cardinality strings and lists of short strings, which
the Rust writer leaves without. WKB geometry and JSON columns exceed the cap and
stay unfiltered, except in a row group small enough — typically the last — for
their dictionary page to fit. Sidecars carry none. DuckDB consults the filters
itself for `=` and `IN` predicates, including those pushed down from a join.
```

- [ ] **Step 5: The crate's architecture notes and the earlier spec**

In `lib/cityparquet-rs/docs/architecture.md`, replace the `cityparquet` row's description with `the tuned default: delta-encoded ids, dictionary \`object_type\`, BYTE_STREAM_SPLIT bbox leaves, no stats/dictionary on WKB+JSON, bloom filters on ids and high-cardinality strings, zstd 3`, and after the sentence `Row-group size and zstd level remain independent CLI knobs on top.` add:

```markdown
Bloom filters are orthogonal to the preset: `--no-bloom` (variant `+nobloom`)
turns them off under any preset, and `parquet-defaults` writes none. The
attribute columns that get one are chosen by `scan` (a HyperLogLog distinct
count per string attribute, dataset-wide) and passed to
`WriterRecipe::writer_properties`.
```

In "Testing discipline", replace `` `just check` runs clippy (`-D warnings`), the full test suite, `` with `` `just check` runs clippy (`-D warnings`) and the full test suite with every feature on (the object-store transport included), ``.

In `docs/superpowers/specs/2026-08-25-readbench-query-design-design.md`, replace the bullet that begins `- **A bloom filter on the `id` column.**` (lines 64-69) with:

```markdown
- **A bloom filter on the `id` column.** Designed on its own, argued from the
  `id-miss` figure this work produces, in `2026-09-22-bloom-filters-design.md`,
  which covers `id` and `feature_id` and the benchmark family that measures
  them.
```

- [ ] **Step 6: Build and check**

```bash
npx --yes prettier@3.9.6 --write lib/cityparquet-rs/docs/architecture.md docs/superpowers/specs/2026-08-25-readbench-query-design-design.md
just docs-build
just mcp-corpus
just mcp-check
```

Expected: the site builds with no broken-link errors (the new page appears under Design decisions); `mcp-check` passes after the corpus regeneration.

- [ ] **Step 7: Commit**

```bash
git add documents/docs/03-specification/02-object-table-schema.mdx documents/docs/03-specification/05-metadata.mdx \
  documents/docs/04-design-decisions/07-bloom-filters.mdx documents/docs/04-design-decisions/meta.ts \
  documents/docs/04-design-decisions/index.mdx documents/docs/06-resources/02-software.mdx \
  lib/cityparquet-rs/docs/architecture.md docs/superpowers/specs/2026-08-25-readbench-query-design-design.md \
  ai/mcp/corpus/corpus.json
git commit -m "docs(spec): bloom filters

The object-table schema allows standard Parquet bloom filters and asks
for them on id and feature_id; readers must not depend on them or treat
a positive result as a match. The recommended defaults give the FPP,
the high-cardinality rule and the placement; decision G records why; the
software page states the duckdb-cityjson superset."
```

---

### Task 8: Readbench harness — lookup counters and the `feature-lookup` scenario

The CityParquet runner calls the `_with_stats` lookups and reports their counters; the CSV gains four trailing columns; a CityParquet-only `feature-lookup` scenario runs two probes, `feature-50pct` and `feature-miss`. No family wiring yet (Task 9).

**Files:**

- Modify: `benchmark/readbench/src/scenario.rs` — `Scenario` (20-35), `as_str` (51-61), `FromStr` (74-96), `QueryParams` (131-146), tests (148-163).
- Modify: `benchmark/readbench/src/formats/mod.rs` — `RunOutcome` (40-45) and new items beside it.
- Modify: `benchmark/readbench/src/formats/cityparquet.rs` — imports (21-38), `ScenarioPlan` (123-173), `run_http` (231-285), `impl FormatRunner for CityParquetRunner` (294-328).
- Modify: `benchmark/readbench/src/formats/cityjson.rs` (arm after line 275; `RunOutcome` literals at 324, 350), `citygml.rs` (arm after line 361; literals 427, 453), `cityjsonseq.rs` (arm after line 459; literals 516, 536), `flatcitybuf.rs` (arms after lines 736 and 787; literals 744, 793).
- Modify: `benchmark/readbench/src/params.rs` — new constants and `feature_probes` after `id_probes` (after line 447), `ResolvedParams` (540-560), `resolve` (576-630), tests.
- Modify: `benchmark/readbench/src/coordinator.rs` — module doc (33-37), imports (74-77), `RunOptions` (after `id_probes`, line 116), `CSV_HEADER` (142-143), probe filtering (360-391), the params `eprintln!` (393-411), the scenario match (after the `Scenario::AttrStats | Scenario::Project` arm, line 597), `ChildLine` (881-893), `spawn_child` (918-1042), `run_measurement` (1059-1147), `run_write`'s `Row` literal, the cold `Row` literal (650-666), `Row` (1318-1367), tests (1445-1542).
- Modify: `benchmark/readbench/src/main.rs` — child args (after `target_id`, line 102), `RunArgs` (after `id_probes`, line 183), `RunOptions` construction (225), `QueryParams` construction (262-268), stdout/stderr report (323-332).
- Modify: `benchmark/readbench/tests/coordinator.rs` — `CSV_COLUMNS` (116-130), `EXPECTED_HEADER` (131-132), a new test.
- Modify: `benchmark/readbench/tests/variants.rs` — `HEADER` (12-13), a new test.
- Modify: `benchmark/scripts/readbench_duckdb.sh` — header comment (6-7), `CSV_HEADER` (136), `append_row` (349-361).
- Modify: `benchmark/scripts/format_write.py` — `HEADER` (14) and the aggregate row (100).
- Modify: `benchmark/formats/READ_BENCHMARK.md` — the CSV contract (lines 216-258).

**Interfaces:**

- Consumes: `cityparquet::query::{id_lookup_with_stats, feature_lookup_with_stats, LookupStats}`, `cityparquet::query_async::{id_lookup_async_with_stats, feature_lookup_async_with_stats}` (Tasks 3-5).
- Produces:
  - `Scenario::FeatureLookup` (`"feature-lookup"`), not in `Scenario::ALL`.
  - `QueryParams::target_feature_id: Option<String>`; child flag `--target-feature-id`; run flag `--feature-probes <tags>`; `RunOptions::feature_probes: Option<Vec<String>>`.
  - `pub struct LookupCounters { pub row_groups_total: u64, pub bloom_pruned: u64, pub row_groups_read: u64, pub filter_bytes: u64 }` and `RunOutcome::lookup: Option<LookupCounters>` in `formats`; `pub const LOOKUP_STATS_MARKER: &str = "cityparquet-readbench: lookup-stats"`; `pub const FEATURE_LOOKUP_CITYPARQUET_ONLY: &str`.
  - `params::FEATURE_50PCT_TAG = "feature-50pct"`, `params::FEATURE_MISS_TAG = "feature-miss"`, `pub fn feature_probes(id_probes: &[IdProbe]) -> Vec<IdProbe>`, `ResolvedParams::feature_probes: Vec<IdProbe>` (in `<out>.params.json`).
  - CSV header: `dataset,format,scenario,selectivity,result_count,time_s,time_mad_s,peak_heap_bytes,peak_rss_bytes,repeat,notes,bytes_read,http_requests,row_groups_total,bloom_pruned,row_groups_read,filter_bytes` — the last four filled only on CityParquet `id-lookup`/`feature-lookup` rows.

- [ ] **Step 1: Write the failing unit tests**

In `scenario.rs` `mod tests`, append:

```rust
    #[test]
    fn feature_lookup_parses_but_is_not_in_the_format_comparison_set() {
        assert_eq!(
            "feature-lookup".parse::<Scenario>().unwrap(),
            Scenario::FeatureLookup
        );
        assert_eq!(Scenario::FeatureLookup.as_str(), "feature-lookup");
        assert!(!Scenario::ALL.contains(&Scenario::FeatureLookup));
        let err = "nope".parse::<Scenario>().unwrap_err();
        assert!(err.contains("feature-lookup"), "{err}");
    }
```

In `params.rs` `mod tests`, append:

```rust
    #[test]
    fn feature_probes_reuse_the_middle_feature_and_the_verified_miss() {
        let seq = ids(10);
        let verifiable: HashSet<String> = seq.iter().cloned().collect();
        let id_probes = id_probes(&seq, &verifiable);
        let features = feature_probes(&id_probes);
        let tags: Vec<&str> = features.iter().map(|p| p.tag.as_str()).collect();
        assert_eq!(tags, vec![FEATURE_50PCT_TAG, FEATURE_MISS_TAG]);
        let middle = id_probes.iter().find(|p| p.tag == "id-50pct").unwrap();
        assert_eq!(features[0].id, middle.id);
        assert!(features[0].present);
        assert!(!features[1].present);
        assert!(!seq.contains(&features[1].id));
    }
```

In `coordinator.rs` `mod tests`, add `lookup: None,` to the `row` helper's `Row` literal and append:

```rust
    #[test]
    fn lookup_counters_fill_the_four_trailing_columns_and_are_empty_otherwise() {
        let plain = row(Scenario::IdLookup, &["id-miss"]);
        let rendered = plain.render();
        assert!(rendered.ends_with("id-miss,,,,,,"), "{rendered}");
        assert_eq!(rendered.split(',').count(), CSV_HEADER.split(',').count());

        let mut counted = row(Scenario::IdLookup, &["id-miss"]);
        counted.lookup = Some(LookupCounters {
            row_groups_total: 16,
            bloom_pruned: 15,
            row_groups_read: 1,
            filter_bytes: 4096,
        });
        let rendered = counted.render();
        assert!(rendered.ends_with("id-miss,,,16,15,1,4096"), "{rendered}");
        assert_eq!(rendered.split(',').count(), CSV_HEADER.split(',').count());
    }

    #[test]
    fn a_childs_lookup_counters_are_read_from_its_marker_line() {
        let stderr = format!("some log\n{LOOKUP_STATS_MARKER} 16 15 1 4096\n");
        assert_eq!(
            child_lookup_counters(&stderr).unwrap(),
            Some(LookupCounters {
                row_groups_total: 16,
                bloom_pruned: 15,
                row_groups_read: 1,
                filter_bytes: 4096,
            })
        );
        assert_eq!(child_lookup_counters("some log\n").unwrap(), None);
        assert!(child_lookup_counters(&format!("{LOOKUP_STATS_MARKER} 1 2\n")).is_err());
    }
```

- [ ] **Step 2: Run and watch them fail**

Run: `cargo test --manifest-path benchmark/readbench/Cargo.toml --bins --lib`
Expected: compile errors — `no variant FeatureLookup`, `cannot find function feature_probes`, `cannot find type LookupCounters`.

- [ ] **Step 3: The scenario and its parameter**

In `scenario.rs`, add after `Project,` in `enum Scenario`:

```rust
    /// Every object of the feature with a given `feature_id` — the feature
    /// and all its parts. CityParquet only: no other format stores the
    /// column, so it is not in [`Scenario::ALL`] (the format-comparison set)
    /// and a run names it explicitly.
    FeatureLookup,
```

In `as_str`, add `Scenario::FeatureLookup => "feature-lookup",`. In `from_str`, add `"feature-lookup" | "featurelookup" => Ok(Scenario::FeatureLookup),` before `other =>`, and make the error list name it:

```rust
            other => Err(format!(
                "unknown scenario '{other}'; expected one of: {}, {}",
                Scenario::ALL
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
                Scenario::FeatureLookup.as_str()
            )),
```

In `QueryParams`, after `target_id`, add:

```rust
    /// Target `feature_id` for [`Scenario::FeatureLookup`].
    pub target_feature_id: Option<String>,
```

Add `target_feature_id: cli.target_feature_id,` to the `QueryParams { .. }` literal in `main.rs` (262-268), and after the child's `target_id` argument (line 102):

```rust
    /// Target `feature_id` for `feature-lookup`.
    #[arg(long)]
    target_feature_id: Option<String>,
```

- [ ] **Step 4: Counters on the runner outcome**

In `formats/mod.rs`, replace `RunOutcome` (40-45) with:

```rust
/// A [`FormatRunner::run`] call's result: the scenario's natural result
/// cardinality, plus [`IoStats`] when `source` was [`Source::Http`], plus
/// [`LookupCounters`] for a CityParquet identifier lookup.
#[derive(Debug, Clone)]
pub struct RunOutcome {
    pub result_count: u64,
    pub io: Option<IoStats>,
    pub lookup: Option<LookupCounters>,
}

/// What one CityParquet identifier lookup cost —
/// `cityparquet::query::LookupStats` as the child reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LookupCounters {
    pub row_groups_total: u64,
    pub bloom_pruned: u64,
    pub row_groups_read: u64,
    pub filter_bytes: u64,
}

/// A child reports its [`LookupCounters`] on stderr, after its timed stdout
/// line, as `<marker> <row_groups_total> <bloom_pruned> <row_groups_read>
/// <filter_bytes>`, so the timed stdout protocol keeps its shape.
pub const LOOKUP_STATS_MARKER: &str = "cityparquet-readbench: lookup-stats";

/// Every non-CityParquet runner's answer to [`Scenario::FeatureLookup`].
pub const FEATURE_LOOKUP_CITYPARQUET_ONLY: &str =
    "scenario 'feature-lookup' is measured for CityParquet only";
```

In each of the other runners, add `lookup: None,` to every `RunOutcome { .. }` literal (`cityjson.rs` 324 and 350, `citygml.rs` 427 and 453, `cityjsonseq.rs` 516 and 536, `flatcitybuf.rs` 744 and 793), and directly after each `Scenario::IdLookup => { .. }` arm (`cityjson.rs` after 275, `citygml.rs` after 361, `cityjsonseq.rs` after 459, `flatcitybuf.rs` after 736 and after 787) add:

```rust
        Scenario::FeatureLookup => bail!("{}", super::FEATURE_LOOKUP_CITYPARQUET_ONLY),
```

and add `bail` to `cityjsonseq.rs`'s `use anyhow::{Context, Result, anyhow};` (the other three import it already). `grep -n "match scenario" benchmark/readbench/src/formats/*.rs` lists the five matches; the compiler's non-exhaustive errors confirm none is missed.

- [ ] **Step 5: The CityParquet runner calls the `_with_stats` lookups**

In `formats/cityparquet.rs`, extend the imports:

```rust
use cityparquet::query::{self, AttrPredicate, LookupStats};

use super::{FormatRunner, IoStats, LookupCounters, RunOutcome, Source};
```

After `to_query_predicate`, add:

```rust
fn counters(stats: LookupStats) -> LookupCounters {
    LookupCounters {
        row_groups_total: stats.row_groups_total as u64,
        bloom_pruned: stats.bloom_pruned as u64,
        row_groups_read: stats.row_groups_read as u64,
        filter_bytes: stats.filter_bytes,
    }
}
```

In `enum ScenarioPlan`, after `IdLookup { id: &'a str },` add `FeatureLookup { feature_id: &'a str },`, and in `resolve` add:

```rust
            Scenario::FeatureLookup => Self::FeatureLookup {
                feature_id: require(&params.target_feature_id, "target-feature-id", scenario)?
                    .as_str(),
            },
```

Replace the body of `run_http` from `let plan = ScenarioPlan::resolve(scenario, params)?;` to the end with:

```rust
    let plan = ScenarioPlan::resolve(scenario, params)?;
    let (result_count, lookup) = match plan {
        ScenarioPlan::Count => (query_async::count_async(dyn_store(), &table_path).await?, None),
        ScenarioPlan::FullRead => {
            let meta = open_metadata_http(dyn_store(), &table_path).await?;
            let result = query_async::full_read_async(dyn_store(), &table_path, &meta).await?;
            (result.feature_count, None)
        }
        ScenarioPlan::BBoxQuery(bbox) => {
            let result = query_async::bbox_query_async(dyn_store(), &table_path, bbox).await?;
            (result.ids.len() as u64, None)
        }
        ScenarioPlan::AttrFilter { column, pred } => (
            query_async::attr_filter_async(dyn_store(), &table_path, column, &pred).await?,
            None,
        ),
        ScenarioPlan::AttrStats { column } => (
            query_async::attr_stats_async(dyn_store(), &table_path, column)
                .await?
                .count,
            None,
        ),
        // The footer is read twice — here for the decode metadata, and again
        // inside the lookup — equally for every variant (disclosed caveat).
        ScenarioPlan::IdLookup { id } => {
            let meta = open_metadata_http(dyn_store(), &table_path).await?;
            let (object, stats) =
                query_async::id_lookup_async_with_stats(dyn_store(), &table_path, &meta, id)
                    .await?;
            (object.is_some() as u64, Some(counters(stats)))
        }
        ScenarioPlan::FeatureLookup { feature_id } => {
            let meta = open_metadata_http(dyn_store(), &table_path).await?;
            let (objects, stats) = query_async::feature_lookup_async_with_stats(
                dyn_store(),
                &table_path,
                &meta,
                feature_id,
            )
            .await?;
            (objects.len() as u64, Some(counters(stats)))
        }
        ScenarioPlan::Project { column } => (
            query_async::project_column_async(dyn_store(), &table_path, column).await?,
            None,
        ),
    };

    let stats = store.tally();
    Ok(RunOutcome {
        result_count,
        io: Some(IoStats {
            bytes: stats.bytes,
            requests: stats.requests,
        }),
        lookup,
    })
}
```

Replace the `Source::Local(path) => { .. }` arm of `CityParquetRunner::run` with:

```rust
            Source::Local(path) => {
                let table = locate_main_table(path)?;
                let plan = ScenarioPlan::resolve(scenario, params)?;
                let (result_count, lookup) = match plan {
                    ScenarioPlan::Count => (query::count(&table)?, None),
                    ScenarioPlan::FullRead => {
                        let meta = open_metadata(&table)?;
                        (query::full_read(&table, &meta)?.feature_count, None)
                    }
                    ScenarioPlan::BBoxQuery(bbox) => {
                        (query::bbox_query(&table, bbox)?.ids.len() as u64, None)
                    }
                    ScenarioPlan::AttrFilter { column, pred } => {
                        (query::attr_filter(&table, column, &pred)?, None)
                    }
                    ScenarioPlan::AttrStats { column } => {
                        (query::attr_stats(&table, column)?.count, None)
                    }
                    ScenarioPlan::IdLookup { id } => {
                        let meta = open_metadata(&table)?;
                        let (object, stats) = query::id_lookup_with_stats(&table, &meta, id)?;
                        (object.is_some() as u64, Some(counters(stats)))
                    }
                    ScenarioPlan::FeatureLookup { feature_id } => {
                        let meta = open_metadata(&table)?;
                        let (objects, stats) =
                            query::feature_lookup_with_stats(&table, &meta, feature_id)?;
                        (objects.len() as u64, Some(counters(stats)))
                    }
                    ScenarioPlan::Project { column } => (query::project_column(&table, column)?, None),
                };
                return Ok(RunOutcome {
                    result_count,
                    io: None,
                    lookup,
                });
            }
```

In `main.rs`, after the `match outcome.io { .. }` that prints the stdout line, add:

```rust
    if let Some(lookup) = outcome.lookup {
        eprintln!(
            "{} {} {} {} {}",
            formats::LOOKUP_STATS_MARKER,
            lookup.row_groups_total,
            lookup.bloom_pruned,
            lookup.row_groups_read,
            lookup.filter_bytes
        );
    }
```

- [ ] **Step 6: The feature probes**

In `params.rs`, after `id_probes`, add:

```rust
/// The feature-lookup probe tags: the feature at the 50 % position of the
/// canonical order, and a `feature_id` verified absent.
pub const FEATURE_50PCT_TAG: &str = "feature-50pct";
pub const FEATURE_MISS_TAG: &str = "feature-miss";

/// The two feature-lookup probes, taken from the id probes: a CityJSONSeq
/// feature's own id IS the `feature_id` of every row it contributes, so the
/// `id-50pct` probe names the middle feature and the verified `id-miss` is
/// absent from `feature_id` too (it is checked against every feature id).
pub fn feature_probes(id_probes: &[IdProbe]) -> Vec<IdProbe> {
    id_probes
        .iter()
        .filter_map(|probe| {
            let tag = match probe.tag.as_str() {
                "id-50pct" => FEATURE_50PCT_TAG,
                ID_MISS_TAG => FEATURE_MISS_TAG,
                _ => return None,
            };
            Some(IdProbe {
                tag: tag.to_string(),
                ..probe.clone()
            })
        })
        .collect()
}
```

In `ResolvedParams`, after `id_probes`, add:

```rust
    /// `feature-50pct` and `feature-miss`, from [`feature_probes`]; EMPTY
    /// exactly when `id_probes` is.
    pub feature_probes: Vec<IdProbe>,
```

and in `resolve`, before `let meta = open_metadata(cp_table)?;` add `let feature_probes = feature_probes(&id_probes);`, and `feature_probes,` to the returned literal after `id_probes,`.

- [ ] **Step 7: The coordinator**

In `coordinator.rs`:

- Imports: `use crate::formats::{IoStats, LOOKUP_STATS_MARKER, LookupCounters, Source};`
- Module doc, after the sentence ending `(`id-10pct`, `id-50pct`, `id-90pct`, `id-miss`).` add: `//! [`Scenario::FeatureLookup`] (CityParquet only, and only when named) emits two: `feature-50pct` and `feature-miss`. Every CityParquet lookup row carries its [`LookupCounters`] in the four trailing CSV columns.`
- `RunOptions`, after `id_probes`:

```rust
    /// Optional subset of resolved `feature-lookup` probe tags.
    pub feature_probes: Option<Vec<String>>,
```

- `CSV_HEADER`:

```rust
const CSV_HEADER: &str = "dataset,format,scenario,selectivity,result_count,time_s,time_mad_s,\
peak_heap_bytes,peak_rss_bytes,repeat,notes,bytes_read,http_requests,row_groups_total,\
bloom_pruned,row_groups_read,filter_bytes";
```

and append to its doc comment: `/// The last four are a CityParquet lookup's [`LookupCounters`], empty on every other row.`

- Replace the whole `if let Some(requested) = &opts.id_probes { .. }` block (360-391) with:

```rust
    retain_requested_probes("id-probes", &mut resolved.id_probes, opts.id_probes.as_ref())?;
    retain_requested_probes(
        "feature-probes",
        &mut resolved.feature_probes,
        opts.feature_probes.as_ref(),
    )?;
```

and add the helper after `parse_variant_list`:

```rust
/// Keeps only the `requested` tags of `probes`; an empty request, or a tag
/// that did not resolve, is an error naming what is available.
fn retain_requested_probes(
    flag: &str,
    probes: &mut Vec<params::IdProbe>,
    requested: Option<&Vec<String>>,
) -> Result<()> {
    let Some(requested) = requested else {
        return Ok(());
    };
    if requested.is_empty() {
        bail!("--{flag} must name at least one resolved probe tag");
    }
    let available: Vec<&str> = probes.iter().map(|probe| probe.tag.as_str()).collect();
    let unknown: Vec<&str> = requested
        .iter()
        .map(String::as_str)
        .filter(|tag| !available.contains(tag))
        .collect();
    if !unknown.is_empty() {
        bail!(
            "--{flag} requested unavailable tag(s) {}; available: {}",
            unknown.join(", "),
            available.join(", ")
        );
    }
    probes.retain(|probe| requested.iter().any(|tag| tag == &probe.tag));
    Ok(())
}
```

- In the derived-params `eprintln!`, change `id probes={:?}, CityObject` to `id probes={:?}, feature probes={:?}, CityObject` and add, after the id-probe argument:

```rust
        resolved
            .feature_probes
            .iter()
            .map(|p| (p.tag.as_str(), p.id.as_str()))
            .collect::<Vec<_>>(),
```

- In the scenario match, directly before `Scenario::IdLookup if resolved.id_probes.is_empty() => ..`, add:

```rust
                Scenario::FeatureLookup
                    if !matches!(format, Format::CityParquet | Format::CityParquetHilbert) =>
                {
                    eprintln!(
                        "cityparquet-readbench: skipping scenario '{scenario}' for format \
                         '{format}': {}",
                        crate::formats::FEATURE_LOOKUP_CITYPARQUET_ONLY
                    )
                }
                Scenario::FeatureLookup if resolved.feature_probes.is_empty() => eprintln!(
                    "cityparquet-readbench: skipping scenario '{scenario}' for format \
                     '{format}': dataset '{dataset}' has no prepared cityjsonseq artefact to \
                     take the feature probes from (never fabricated)"
                ),
                Scenario::FeatureLookup => {
                    for probe in &resolved.feature_probes {
                        let params = QueryParams {
                            target_feature_id: Some(probe.id.clone()),
                            ..Default::default()
                        };
                        let mut notes = probe.tag.clone();
                        if probe.substituted {
                            notes.push_str(";id-substituted");
                        }
                        run_measurement(
                            &mut rows,
                            &mut samples,
                            &dataset,
                            format,
                            label,
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

- `ChildLine`: add `lookup: Option<LookupCounters>,` (documented `/// The child's [`LookupCounters`], from its [`LOOKUP_STATS_MARKER`] line.`). In `spawn_child`, replace `let notes = child_disclosures(&String::from_utf8_lossy(&output.stderr));` with:

```rust
    let stderr = String::from_utf8_lossy(&output.stderr);
    let notes = child_disclosures(&stderr);
    let lookup = child_lookup_counters(&stderr)?;
```

add, after the `--target-id` argument block:

```rust
    if let Some(feature_id) = &params.target_feature_id {
        cmd.arg("--target-feature-id").arg(feature_id);
    }
```

and `lookup,` to the returned `ChildLine`. After `child_disclosures`, add:

```rust
/// The [`LookupCounters`] a child reported after [`LOOKUP_STATS_MARKER`],
/// if it reported any.
fn child_lookup_counters(stderr: &str) -> Result<Option<LookupCounters>> {
    let Some(line) = stderr
        .lines()
        .find_map(|line| line.strip_prefix(LOOKUP_STATS_MARKER))
    else {
        return Ok(None);
    };
    let fields: Vec<u64> = line
        .split_whitespace()
        .map(|field| {
            field
                .parse::<u64>()
                .with_context(|| format!("parsing lookup counter '{field}'"))
        })
        .collect::<Result<_>>()?;
    let [row_groups_total, bloom_pruned, row_groups_read, filter_bytes] = fields[..] else {
        bail!("expected four lookup counters after '{LOOKUP_STATS_MARKER}', got '{line}'");
    };
    Ok(Some(LookupCounters {
        row_groups_total,
        bloom_pruned,
        row_groups_read,
        filter_bytes,
    }))
}
```

- `run_measurement`: beside `let mut io: Option<IoStats> = None;` add `let mut lookup: Option<LookupCounters> = None;`, set `lookup = line.lookup;` next to `io = line.io;`, and add `lookup,` to its `Row { .. }`.
- The cold `Row { .. }` literal and `run_write`'s `Row { .. }` literal: add `lookup: None,`.
- `Row`: add `lookup: Option<LookupCounters>,` after `io`, and in `render` add

```rust
        let lookup_fields = match self.lookup {
            Some(l) => format!(
                "{},{},{},{}",
                l.row_groups_total, l.bloom_pruned, l.row_groups_read, l.filter_bytes
            ),
            None => ",,,".to_string(),
        };
```

  ending the `format!` with `{bytes_field},{requests_field},{lookup_fields}"`.

In `main.rs` `RunArgs`, after `id_probes`:

```rust
    /// Restrict `feature-lookup` to resolved probe tags (`feature-50pct`,
    /// `feature-miss`). Omit to keep both.
    #[arg(long, value_delimiter = ',')]
    feature_probes: Option<Vec<String>>,
```

and `feature_probes: run_args.feature_probes,` in the `RunOptions` literal.

- [ ] **Step 8: Run the unit tests**

Run: `cargo test --manifest-path benchmark/readbench/Cargo.toml --bins --lib`
Expected: all unit tests pass.

- [ ] **Step 9: Write the failing integration tests**

In `tests/coordinator.rs`, extend `CSV_COLUMNS` to `[&str; 17]` with `"row_groups_total", "bloom_pruned", "row_groups_read", "filter_bytes"` appended, and set:

```rust
const EXPECTED_HEADER: &str = "dataset,format,scenario,selectivity,result_count,time_s,\
time_mad_s,peak_heap_bytes,peak_rss_bytes,repeat,notes,bytes_read,http_requests,\
row_groups_total,bloom_pruned,row_groups_read,filter_bytes";
```

then append:

```rust
/// `feature-lookup` is CityParquet's alone: a named non-CityParquet format is
/// skipped and told so, and the CityParquet rows are the middle feature (a
/// Building and its part) and a verified miss, each with its lookup counters.
#[test]
fn feature_lookup_measures_cityparquet_only_with_lookup_counters() {
    let prepared = tempfile::tempdir().unwrap();
    let input = fixture("delft.city.jsonl");
    convert(&ConvertOptions::new(input.clone(), prepared.path().join("delft.parquet"))).unwrap();
    prepare_seq_artefact(prepared.path(), &input, "delft");

    let out_csv = prepared.path().join("out.csv");
    let output = run_coordinator(&[
        "--input",
        input.to_str().unwrap(),
        "--prepared-dir",
        prepared.path().to_str().unwrap(),
        "--out",
        out_csv.to_str().unwrap(),
        "--repeat",
        "1",
        "--scenarios",
        "feature-lookup",
        "--formats",
        "cityparquet,cityjsonseq",
    ]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("skipping scenario 'feature-lookup' for format 'cityjsonseq'"),
        "{stderr}"
    );

    let csv_text = std::fs::read_to_string(&out_csv).unwrap();
    assert_eq!(csv_text.lines().next().unwrap(), EXPECTED_HEADER);
    let rows: Vec<Row> = csv_text.lines().skip(1).map(Row::parse).collect();
    assert_eq!(rows.len(), 2, "{csv_text}");
    for row in &rows {
        assert_eq!(row.field("format"), "cityparquet");
        assert_eq!(row.field("scenario"), "feature-lookup");
        assert_eq!(row.field("row_groups_total"), "1", "delft is one row group");
        assert!(!row.field("filter_bytes").is_empty());
    }
    let hit = rows.iter().find(|r| r.field("notes").starts_with("feature-50pct")).unwrap();
    assert!(
        ["2", "3"].contains(&hit.field("result_count")),
        "a delft feature is a Building and its parts"
    );
    let miss = rows.iter().find(|r| r.field("notes").starts_with("feature-miss")).unwrap();
    assert_eq!(miss.field("result_count"), "0");
}
```

In `tests/variants.rs`, set `HEADER` to the same 17-column string and append:

```rust
/// The bloom pair: the same package with and without filters. Lookup rows
/// carry counters — no filter bytes and nothing pruned without filters —
/// and write rows carry none.
#[test]
fn a_bloom_pair_records_lookup_counters() {
    let (prepared, input) = prepared_delft();
    let out_csv = prepared.path().join("out.csv");
    let output = run(&[
        "--input",
        input.to_str().unwrap(),
        "--prepared-dir",
        prepared.path().to_str().unwrap(),
        "--out",
        out_csv.to_str().unwrap(),
        "--repeat",
        "1",
        "--write-repeat",
        "1",
        "--scenarios",
        "id-lookup,feature-lookup",
        "--id-probes",
        "id-50pct,id-miss",
        "--variants",
        "cityparquet,cityparquet+nobloom",
    ]);
    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = std::fs::read_to_string(&out_csv).unwrap();
    let mut lines = text.lines();
    assert_eq!(lines.next().unwrap(), HEADER);
    let rows: Vec<&str> = lines.collect();
    // Per variant: write, id-50pct, id-miss, feature-50pct, feature-miss.
    assert_eq!(rows.len(), 10, "{text}");
    for row in &rows {
        let (label, scenario) = (field(row, 1), field(row, 2));
        let counters: Vec<&str> = (13..17).map(|i| field(row, i)).collect();
        if scenario == "write" {
            assert_eq!(counters, vec!["", "", "", ""], "{row}");
            continue;
        }
        assert_eq!(counters[0], "1", "delft is one row group: {row}");
        if label == "cityparquet+nobloom" {
            assert_eq!(counters[1], "0", "{row}");
            assert_eq!(counters[3], "0", "{row}");
        } else {
            assert_ne!(counters[3], "0", "{row}");
        }
    }
}
```

- [ ] **Step 10: Run the integration tests**

Run: `cargo test --manifest-path benchmark/readbench/Cargo.toml`
Expected: the whole suite passes, the two new tests included. (Before Step 9's header constants were updated, `run_produces_the_exact_csv_contract…` and `a_variants_run_writes_reads…` failed on the header — that is the red for the 17-column contract.)

- [ ] **Step 11: The two other CSV writers**

In `benchmark/scripts/readbench_duckdb.sh`, change the header comment (lines 6-7) to end `…,notes,bytes_read,http_requests,row_groups_total,bloom_pruned,row_groups_read,filter_bytes`, set

```bash
CSV_HEADER="dataset,format,scenario,selectivity,result_count,time_s,time_mad_s,peak_heap_bytes,peak_rss_bytes,repeat,notes,bytes_read,http_requests,row_groups_total,bloom_pruned,row_groups_read,filter_bytes"
```

and in `append_row` print six trailing empty fields instead of two:

```bash
  printf '%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s\n' \
    "$dataset" "$format" "$scenario" "$selectivity" "$result_count" \
    "$time_s" "$time_mad_s" "$peak_heap_bytes" "$peak_rss_bytes" "$repeat" "$notes" \
    "" "" "" "" "" "" \
    >> "$OUT_CSV"
```

extending its comment: `The trailing lookup counters are empty too: they belong to the CityParquet runner's own lookups.`

In `benchmark/scripts/format_write.py`, append `"row_groups_total","bloom_pruned","row_groups_read","filter_bytes"` to `HEADER`, and four `""` to the end of the `aggregates.append([...])` row.

In `benchmark/formats/READ_BENCHMARK.md`, extend the header line in the CSV-contract code block with `,row_groups_total,bloom_pruned,row_groups_read,filter_bytes` and add after the `bytes_read` / `http_requests` bullet:

```markdown
- `row_groups_total` / `bloom_pruned` / `row_groups_read` / `filter_bytes` —
  **empty on every row but a CityParquet `id-lookup` or `feature-lookup`**:
  the table's row groups, those its bloom filters ruled out, those actually
  opened (an `id-lookup` hit stops at its first match; a `feature-lookup`
  reads every surviving row group to the end), and the bytes of filter read.
  Deterministic across repeats; the first warm sample's values are recorded.
```

Add `feature-lookup` to the scenario section with one sentence: under `## The seven scenarios`, append the paragraph `` `feature-lookup` (CityParquet only, run by name — the `bloom` family) returns every object of one `feature_id`, probed at `feature-50pct` (the `id-50pct` feature) and `feature-miss`; it is not one of the seven comparison scenarios, because no other format stores the column. ``

- [ ] **Step 12: Run every affected suite**

```bash
cargo clippy --manifest-path benchmark/readbench/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path benchmark/readbench/Cargo.toml
cargo fmt --manifest-path benchmark/readbench/Cargo.toml --check
just scripts-test
npx --yes prettier@3.9.6 --check benchmark/formats/READ_BENCHMARK.md
```

Expected: all pass.

- [ ] **Step 13: Commit**

```bash
git add benchmark/readbench/src benchmark/readbench/tests benchmark/scripts/readbench_duckdb.sh \
  benchmark/scripts/format_write.py benchmark/formats/READ_BENCHMARK.md
git commit -m "feat(readbench)!: lookup counters and a feature-lookup scenario

The CityParquet runner calls the _with_stats lookups and reports their
LookupStats; the CSV gains row_groups_total, bloom_pruned,
row_groups_read and filter_bytes, filled on CityParquet lookup rows. A
CityParquet-only feature-lookup scenario runs the feature-50pct and
feature-miss probes, selectable with --feature-probes."
```

---

### Task 9: The `bloom` benchmark family

One pair, `cityparquet` against `cityparquet+nobloom`, over the 3DBAG scaling slices and the corpus, measuring the identifier lookups the filters exist for. Built on the same machinery as `codec` and `rowgroup`.

**Files:**

- Modify: `benchmark/manifest.toml` — `[suite] families` (line 7).
- Modify: `justfile` (root) — `variant-bench` (503-562), new `bloom-bench` after `rowgroup-bench` (after line 579).
- Modify: `benchmark/scripts/bench_suite.py` — `FAMILIES` (21), `dataset_selection` (42-70), `prepare` (182-212), `result_dir` (232-235), `run_suite` (245-285), the `--families` help is derived from `FAMILIES`.
- Modify: `benchmark/scripts/tests/test_bench_suite.py` — new cases.
- Modify: `benchmark/scripts/tests/bench_recipe_test.sh` — new case (after line 327) and its call (after line 338).
- Modify: `benchmark/plot/benchviz/prep.py` — `Inputs` (after `scaling_rowgroup_dir`, line 65), constants (172-188), `_scenario_key` (391-410), `load_scaling_axis` (696-835), `build` (1060-1129), `main` (1132-1165).
- Modify: `benchmark/plot/benchviz/figures.py` — palette constants (46-48), `_axis_palette` (175-189), `_axis_queries` (501-504), `main` (846).
- Modify: `benchmark/plot/benchviz/__main__.py` — family loops (75-77, 82-106), `--families` help (171), the stale-results hint (183).
- Modify: `benchmark/plot/benchviz/html.py` — `ORDER`, `TITLES` (12-21).
- Create: `benchmark/plot/tests/fixtures/benchviz/scaling_bloom_results/delft.csv`, `…/sizes.csv`.
- Modify: `benchmark/plot/tests/test_benchviz.py` — figure set (30) and a new test.
- Modify: `benchmark/README.md` — layout table (17-25), "Selecting work" (41), experimental matrix (72-78), figures table (96-104).
- Modify: `benchmark/formats/README.md` — the opening caveats (8-29) and a new `## The bloom family` section before `## Reproduce` (line 135).

**Interfaces:**

- Consumes: `+nobloom` (Task 1); `--scenarios feature-lookup`, `--feature-probes`, the 17-column CSV (Task 8).
- Produces: family `bloom`; recipe `just bloom-bench FOLDER [OUT] [PREPARED] [REPEAT] [WRITE_REPEAT]` writing `benchmark/runs/formats/scaling_bloom_results/`; `variant-bench` gains `SCENARIOS` (default `full-read,bbox-query,id-lookup`), `ID_PROBES` (default `id-50pct`) and `FEATURE_PROBES` (default empty) after `WRITE_REPEAT`; `prep.BLOOM_MEASURES = ("write", "id-50pct", "id-miss", "feature-50pct", "feature-miss")`; `prep.load_scaling_axis(directory, baseline=AXIS_BASELINE, measures=AXIS_MEASURES)`; `data["scaling"]["bloom"]`; figures `bloom` and `bloom-scaling`.

- [ ] **Step 1: Write the failing Python and shell tests**

In `benchmark/scripts/tests/test_bench_suite.py`, add to `SelectionTests`:

```python
    def test_bloom_default_is_the_scaling_series_and_the_corpus(self):
        selected = bench_suite.dataset_selection(self.manifest, ["bloom"], "", False)
        self.assertEqual(len(selected), 12)
        self.assertIn("rotterdam", selected)
        self.assertIn("3dbag_n1000000", selected)
    def test_bloom_results_have_their_own_directory(self):
        locations = bench_suite.paths(bench_suite.DEFAULT_DATA_ROOT)
        self.assertEqual(bench_suite.result_dir(locations, "bloom", False).name, "scaling_bloom_results")
    def test_bloom_is_a_family(self):
        self.assertIn("bloom", bench_suite.family_selection("all"))
```

In `benchmark/scripts/tests/bench_recipe_test.sh`, after `case_rowgroup_bench_list() { .. }`, add:

```bash
case_bloom_bench_list() {
  local name="bloom-bench passes the one bloom pair and the lookup scenarios"
  local expected="cityparquet,cityparquet+nobloom"
  local actual line
  actual="$(recipe_variants bloom-bench)"
  if [[ "$actual" != "$expected" ]]; then
    fail "$name" "bloom-bench passes '$actual'"
    return
  fi
  line="$(sed -n '/^bloom-bench /,/^$/p' "$JUSTFILE" | grep 'just variant-bench')"
  if [[ "$line" != *'"id-lookup,feature-lookup" "id-50pct,id-miss" "feature-50pct,feature-miss"'* ]]; then
    fail "$name" "bloom-bench does not pass the lookup scenarios and probes: $line"
    return
  fi
  pass "$name"
}
```

and call `case_bloom_bench_list` after `case_rowgroup_bench_list` at the end of the file.

Create `benchmark/plot/tests/fixtures/benchviz/scaling_bloom_results/delft.csv` (renderer fixture values, not measurements):

```
dataset,format,scenario,selectivity,result_count,time_s,time_mad_s,peak_heap_bytes,peak_rss_bytes,repeat,notes,bytes_read,http_requests,row_groups_total,bloom_pruned,row_groups_read,filter_bytes
delft.city.jsonl,cityparquet,write,,2231,1.170000,0.003000,182000000,225000000,2,,,,,,,
delft.city.jsonl,cityparquet,id-lookup,0.000448,1,0.004100,0.000100,390000,14700000,2,id-50pct,,,1,0,1,8192
delft.city.jsonl,cityparquet,id-lookup,0.000000,0,0.002100,0.000100,380000,14600000,2,id-miss,,,1,1,0,8192
delft.city.jsonl,cityparquet,feature-lookup,0.000896,2,0.004300,0.000100,395000,14700000,2,feature-50pct,,,1,0,1,8192
delft.city.jsonl,cityparquet,feature-lookup,0.000000,0,0.002200,0.000100,381000,14600000,2,feature-miss,,,1,1,0,8192
delft.city.jsonl,cityparquet+nobloom,write,,2231,1.150000,0.003000,181000000,224000000,2,,,,,,,
delft.city.jsonl,cityparquet+nobloom,id-lookup,0.000448,1,0.004000,0.000100,389000,14700000,2,id-50pct,,,1,0,1,0
delft.city.jsonl,cityparquet+nobloom,id-lookup,0.000000,0,0.004900,0.000100,389000,14700000,2,id-miss,,,1,0,1,0
delft.city.jsonl,cityparquet+nobloom,feature-lookup,0.000896,2,0.004200,0.000100,394000,14700000,2,feature-50pct,,,1,0,1,0
delft.city.jsonl,cityparquet+nobloom,feature-lookup,0.000000,0,0.004800,0.000100,394000,14700000,2,feature-miss,,,1,0,1,0
```

and `…/scaling_bloom_results/sizes.csv`:

```
dataset,format,bytes,mb,ratio_vs_cityjsonseq,baseline_format,ratio_vs_baseline
delft,cityparquet,2412000,2.300262,2.738806,cityparquet,1.000000
delft,cityparquet+nobloom,2392201,2.281381,2.761358,cityparquet,1.008276
```

In `benchmark/plot/tests/test_benchviz.py`, change the figure-name tuple (line 30) to `("sizes", "heatmap", "codec", "codec-scaling", "rowgroup", "rowgroup-scaling", "bloom", "bloom-scaling")` and append:

```python
def test_bloom_axis_keys_the_lookup_probes_and_carries_the_counters(tmp_path: Path):
    bench = fixture_bench(tmp_path)
    data, _ = prep.build(prep.Inputs(bench))
    axis = data["scaling"]["bloom"]
    assert axis["variants"] == ["cityparquet", "cityparquet+nobloom"]
    assert {r["measure"] for r in axis["records"]} == set(prep.BLOOM_MEASURES)

    def record(variant: str, measure: str) -> dict:
        return next(
            r for r in axis["records"] if r["variant"] == variant and r["measure"] == measure
        )

    on = record("cityparquet", "id-miss")
    assert (on["row_groups_total"], on["bloom_pruned"], on["row_groups_read"]) == (1, 1, 0)
    assert on["filter_bytes"] == 8192
    off = record("cityparquet+nobloom", "id-miss")
    assert (off["bloom_pruned"], off["filter_bytes"]) == (0, 0)
    assert off["time_ratio"] == 0.0049 / 0.0021
    assert record("cityparquet", "write")["row_groups_total"] is None
```

- [ ] **Step 2: Run and watch them fail**

```bash
python3 -m unittest benchmark/scripts/tests/test_bench_suite.py
./benchmark/scripts/tests/bench_recipe_test.sh
just plot-test
```

Expected: `unknown benchmark family: bloom` / `KeyError: 'bloom'` in the Python suite; `not ok - bloom-bench passes …: bloom-bench passes ''` from the shell suite; `KeyError: 'bloom'` and the missing `bloom.svg` in `plot-test`.

- [ ] **Step 3: Manifest and recipes**

In `benchmark/manifest.toml` set `families = ["sizes", "formats", "codec", "rowgroup", "bloom", "databases"]`.

In the root `justfile`, change the `variant-bench` signature to:

```just
variant-bench FOLDER OUT VARIANTS PREPARED=(BENCH / "runs/data/readbench") REPEAT='7' WRITE_REPEAT='3' SCENARIOS='full-read,bbox-query,id-lookup' ID_PROBES='id-50pct' FEATURE_PROBES='':
```

replace its `cargo run` invocation with:

```bash
        feature_args=()
        if [[ -n "{{FEATURE_PROBES}}" ]]; then
            feature_args=(--feature-probes "{{FEATURE_PROBES}}")
        fi
        cargo run --release {{READBENCH_CARGO}} -- run \
            --input "$f" \
            --prepared-dir "{{PREPARED}}" \
            --out "$out" \
            --repeat {{REPEAT}} \
            --write-repeat {{WRITE_REPEAT}} \
            --scenarios "{{SCENARIOS}}" \
            --id-probes "{{ID_PROBES}}" \
            ${feature_args[@]+"${feature_args[@]}"} \
            --variants "{{VARIANTS}}"
```

and extend its comment block's second sentence to: `…then \`full-read\` and the three bbox windows against it (the default SCENARIOS/ID_PROBES; the bloom axis passes the lookups instead).` After `rowgroup-bench`, add:

```just
# The BLOOM axis: the default package, which carries bloom filters, against
# the same package without them. Identifier lookups only — what the filters
# exist for — by `id` and by `feature_id`, each at the middle position and a
# verified miss. Every variant at the default codec and row-group size.
[private]
[doc("Bloom axis over the scaling slices and corpus: cityparquet vs cityparquet+nobloom")]
bloom-bench FOLDER OUT=(BENCH / "runs/formats/scaling_bloom_results") PREPARED=(BENCH / "runs/data/readbench") REPEAT='7' WRITE_REPEAT='3':
    just variant-bench "{{FOLDER}}" "{{OUT}}" "cityparquet,cityparquet+nobloom" "{{PREPARED}}" "{{REPEAT}}" "{{WRITE_REPEAT}}" "id-lookup,feature-lookup" "id-50pct,id-miss" "feature-50pct,feature-miss"
```

- [ ] **Step 4: The suite driver**

In `bench_suite.py`:

```python
FAMILIES = ("sizes", "formats", "codec", "rowgroup", "bloom", "databases")
```

In `dataset_selection`, after the `codec`/`rowgroup` extension, add:

```python
        if "bloom" in families:
            result.extend(key for key, entry in datasets.items() if entry["role"] == "corpus")
            result.extend(key for key, entry in datasets.items() if entry["role"] in {"scaling", "largest-scaling"})
```

In `prepare`, the configuration families convert from the prepared CityJSONSeq (`variant-bench` refuses to run without it), so replace

```python
        just("readbench-prepare", str(input_path), str(locations["prepared"]), "" if full_formats else "cityparquet")
```

with

```python
        just("readbench-prepare", str(input_path), str(locations["prepared"]), "" if full_formats else "cityparquet,cityjsonseq")
```

In `result_dir`, add `"bloom": "scaling_bloom_results"` to `names`. In `run_suite`, after the `for family, recipe in (("codec", …), ("rowgroup", …)):` loop, add:

```python
    if "bloom" in families:
        bloom_inputs = [source(entry, locations) for entry in selected.values() if entry["role"] in {"corpus", "scaling", "largest-scaling"}]
        if not bloom_inputs:
            raise SystemExit("bloom needs a corpus dataset or a 3DBAG scaling slice")
        output = result_dir(locations, "bloom", smoke)
        just("bloom-bench", str(stage(locations, "bloom", bloom_inputs)), str(output), str(locations["prepared"]), "1" if smoke else "7", "1" if smoke else "3")
        for input_path in bloom_inputs:
            write_run_manifest(input_path, output / f"{dataset_stem(input_path)}.csv", family="bloom", repeat=1 if smoke else 7, write_repeat=1 if smoke else 3, smoke=smoke, fixed_configuration="bloom/zstd-3/default-row-groups")
```

`require_prepared(format_inputs + scaling_inputs, locations)` already covers every bloom input: `format_inputs` holds each selected corpus dataset whatever the family.

- [ ] **Step 5: The renderer**

In `prep.py`:

- After `scaling_rowgroup_dir`, add:

```python
    @property
    def scaling_bloom_dir(self) -> Path:
        return self.bench_dir / "scaling_bloom_results"
```

- After `ID_NOTE_RE`, add `FEATURE_NOTE_RE = re.compile(r"^feature-(?:\d+pct|miss)$")`; after `AXIS_MEASURES`, add:

```python
# The bloom axis measures the identifier lookups only; every other query is
# untouched by the filters.
BLOOM_MEASURES = ("write", "id-50pct", "id-miss", "feature-50pct", "feature-miss")
# The lookup counters a CityParquet lookup row carries, appended to the read
# CSV after `http_requests`.
LOOKUP_COLUMNS = ("row_groups_total", "bloom_pruned", "row_groups_read", "filter_bytes")
```

- In `_scenario_key`, before the final `return scenario`, add:

```python
    if scenario == "feature-lookup":
        tag = _primary_tag(notes)
        if FEATURE_NOTE_RE.match(tag):
            return tag
```

- `load_scaling_axis`: change the signature to `def load_scaling_axis(directory: Path, baseline: str = AXIS_BASELINE, measures: tuple[str, ...] = AXIS_MEASURES) -> dict:`, the doc's first line to `"""One configuration axis (codec, row group or bloom) from a `--variants` run.`, `for key in AXIS_MEASURES:` to `for key in measures:`, and add to the appended record dict, after `"notes": row.get("notes", ""),`:

```python
                        **{column: _int(row.get(column)) for column in LOOKUP_COLUMNS},
```

- In `build`, after `scaling["rowgroup"] = …`, add `scaling["bloom"] = load_scaling_axis(inputs.scaling_bloom_dir, measures=BLOOM_MEASURES)`; add `"bloom": inputs.label(inputs.scaling_bloom_dir),` to `meta.sources` and `"bloom": read_machine(inputs.scaling_bloom_dir),` to `meta.machine`.
- In `main`, add `"bloom": bool(data["scaling"]["bloom"]["records"]),` to `completeness`, and `f"{len(data['scaling']['bloom']['records'])} bloom records, "` after the row-group count in the summary print.

In `figures.py`:

- After `ROWGROUP_HUE`, add `BLOOM_OFF_COLOUR = "#5F6E7A"  # the package without filters, against the accent default`.
- In `_axis_palette`, replace `else:` with `elif key == "rowgroup":` and add at the end, before `return palette`:

```python
    else:
        for v in variants:
            if v != "cityparquet":
                palette[v] = BLOOM_OFF_COLOUR
```

- In `_axis_queries`, set `wanted = ["full-read", "bbox-1pct", "bbox-5pct", "bbox-25pct", "id-50pct", "id-miss", "feature-50pct", "feature-miss", "id-lookup"]`.
- In `main`, `for key in ("codec", "rowgroup", "bloom"):`.

In `__main__.py`, change the family loop to `for name in ("codec", "rowgroup", "bloom"):`, add to `coverage["families"]`:

```python
            "bloom": {
                "present": bool(payload["scaling"]["bloom"]["records"]),
                "metrics": sorted(
                    {r.get("measure") for r in payload["scaling"]["bloom"]["records"]}
                ),
            },
```

set the `--families` help to `"selected families: sizes,formats,codec,rowgroup,bloom,databases"`, and add `/ \`just bloom-bench\`` after `` `just rowgroup-bench` `` in the stale-results hint (line 183).

In `html.py`, `ORDER = ("sizes", "heatmap", "codec", "codec-scaling", "rowgroup", "rowgroup-scaling", "bloom", "bloom-scaling", "databases")` and add to `TITLES`:

```python
    "bloom": "Bloom-filter configuration",
    "bloom-scaling": "Bloom-filter scaling",
```

- [ ] **Step 6: Run the suites**

```bash
python3 -m unittest benchmark/scripts/tests/test_bench_suite.py
just scripts-test
just plot-test
```

Expected: all pass.

- [ ] **Step 7: The methodology and its caveats**

In `benchmark/README.md`:

- Layout table: change `formats/results/`, `formats/scaling_{codec,rowgroup}_results/` to `formats/results/`, `formats/scaling_{codec,rowgroup,bloom}_results/`.
- "Selecting work": `The family names are \`sizes\`, \`formats\`, \`codec\`, \`rowgroup\`, \`bloom\` and \`databases\`. With no selection, the suite includes all six.`
- Experimental matrix, after the `rowgroup` row:

```markdown
| `bloom` | Nested 3DBAG scaling slices and the corpus | Size and write time/memory; lookup time, memory and row-group counters | `id-lookup` at `id-50pct`/`id-miss`; `feature-lookup` at `feature-50pct`/`feature-miss` |
```

- Figures table: `| \`codec\`, \`rowgroup\`, \`bloom\` | Five metric panels for the largest measured scaling dataset |` and `| \`codec-scaling\`, \`rowgroup-scaling\`, \`bloom-scaling\` | Absolute metrics against actual CityObject counts |`.

In `benchmark/formats/README.md`, after the paragraph ending `…so those columns are empty in every package they measured.` (line 29), add:

```markdown
**The committed codec and row-group CSVs predate bloom filters.** Every
package they measured, the `cityparquet` baseline included, carries none, and
their `id-50pct` rows read the `id` column without bloom pruning. The current
writer puts filters on `id`, `feature_id` and high-cardinality string
attributes by default, so its packages are larger and its lookups prune:
re-run both families before comparing them with the `bloom` family, or with
each other across that change.
```

and before `## Reproduce`, add:

```markdown
## The bloom family

`just bloom-bench` (via `just bench-run --families bloom`) writes each input
twice — `cityparquet`, which carries bloom filters, and `cityparquet+nobloom`,
which carries none — and times `id-lookup` (`id-50pct`, `id-miss`) and
`feature-lookup` (`feature-50pct`, `feature-miss`) against both. Package bytes
go to `sizes.csv`; the write rows carry write time and peak RSS, where the
filters' memory shows (every filter is held until its file is closed). Every
lookup row carries `row_groups_total`, `bloom_pruned`, `row_groups_read` and
`filter_bytes`. Caveats that travel with every number:

1. **Only the multi-row-group inputs can prune.** At the default 65 536-row
   groups the small slices are one row group, where a filter can only save
   that one; the slices from `3dbag_n100000` upward are the informative ones.
2. **A filter's positive is not a match.** At FPP 0.01 a miss can still open a
   row group; `row_groups_read` on the `*-miss` rows shows how often.
3. **Lookups open one reader per kept row group.** `id-lookup` reads each
   surviving row group with a builder of its own (all sharing one parsed
   footer), which is what makes `row_groups_read` exact and lets a hit stop
   early; `feature-lookup` reads its survivors with one reader, to the end.
   Both variants use the same code path.
4. **The footer is read twice per lookup** — once for the decode metadata,
   once inside the lookup — equally for both variants.
5. **Single-table packages only.** The runner queries one object table; a
   multi-table corpus package is refused, never partially read.
6. **`feature-50pct` is the `id-50pct` feature**, and `feature-miss` the same
   verified-absent string as `id-miss` (absent from both columns).
7. **Requests are logical.** Over `--transport http`, `CountingObjectStore`
   counts the requests the reader made after object_store coalesced nearby
   ranges — not raw wire traffic, retries or connection reuse.
```

Then `npx --yes prettier@3.9.6 --write benchmark/README.md benchmark/formats/README.md`.

- [ ] **Step 8: Run the full root gate**

Run: `just check`
Expected: PASS (library gate, readbench clippy/test/fmt, plot-test, scripts-test, mcp-check, citylake-check — the last needs the local duckdb-cityjson build from Task 6).

- [ ] **Step 9: Commit**

```bash
git add benchmark/manifest.toml justfile benchmark/scripts/bench_suite.py \
  benchmark/scripts/tests/test_bench_suite.py benchmark/scripts/tests/bench_recipe_test.sh \
  benchmark/plot/benchviz benchmark/plot/tests benchmark/README.md benchmark/formats/README.md
git commit -m "feat(benchmark): the bloom family

cityparquet against cityparquet+nobloom over the scaling slices and the
corpus, timing id-lookup and feature-lookup at their middle and miss
probes with lookup counters. variant-bench takes its scenarios and probes
as parameters; the renderer draws the bloom axis; the READMEs state its
caveats and that the committed codec and row-group CSVs predate bloom
filters."
```

---

### Task 10: The bloom pair over HTTP

A `--variants` run over `--transport http` becomes a read-only run against the packages a local run wrote (`<prepared>/<base>.<id>.parquet`) once they are uploaded beside the prepared artefacts — the same arrangement the format comparison already uses over HTTP. Its rows carry `bytes_read`, `http_requests` and the lookup counters; it writes no `write` row and no `sizes.csv`.

**Files:**

- Modify: `benchmark/readbench/src/coordinator.rs` — module doc (38-50), `RunOptions::variants` doc (100-105), the validation (207-212), `variant_seq` and the variants block (452-488), the `write_sizes` call (714-719).
- Modify: `benchmark/readbench/tests/variants.rs` — the `server-side write` rejection (294-307) and a new test.
- Modify: `justfile` (root) — new `bloom-bench-http` after `bloom-bench`.
- Modify: `benchmark/scripts/tests/bench_recipe_test.sh` — a new case.
- Modify: `benchmark/formats/README.md` — the `## The bloom family` section (Task 9).

**Interfaces:**

- Consumes: `--variants`, the 17-column CSV and `feature-lookup` (Task 8); `bloom-bench`'s variant list and output layout (Task 9).
- Produces: `cityparquet-readbench run --variants … --transport http --base-url URL` (read-only); `just bloom-bench-http FOLDER BASE_URL [OUT] [PREPARED] [REPEAT]` writing `benchmark/runs/formats/scaling_bloom_http_results/`.

- [ ] **Step 1: Write the failing test**

In `tests/variants.rs`, delete the `expect_rejection(.. "server-side write")` call (the one passing `--transport http --base-url http://localhost:1`), and append:

```rust
async fn spawn_server(dir: PathBuf) -> std::net::SocketAddr {
    let app = axum::Router::new().fallback_service(tower_http::services::ServeDir::new(dir));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    addr
}

/// Over HTTP a `--variants` run reads the packages a local run wrote and an
/// operator uploaded — here, the prepared directory served as it is. No
/// write rows, no sizes, and every lookup row carries its transport and
/// lookup counters. `multi_thread`: `run` blocks on a child process while the
/// server task must keep accepting.
#[tokio::test(flavor = "multi_thread")]
async fn a_variants_run_over_http_reads_the_uploaded_packages_without_writing() {
    let (prepared, input) = prepared_delft();
    let common = |out: &PathBuf| -> Vec<String> {
        [
            "--input",
            input.to_str().unwrap(),
            "--prepared-dir",
            prepared.path().to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
            "--repeat",
            "1",
            "--write-repeat",
            "1",
            "--scenarios",
            "id-lookup",
            "--id-probes",
            "id-miss",
            "--variants",
            "cityparquet,cityparquet+nobloom",
        ]
        .map(String::from)
        .to_vec()
    };
    let local_csv = prepared.path().join("local.csv");
    let args = common(&local_csv);
    let local = run(&args.iter().map(String::as_str).collect::<Vec<_>>());
    assert!(local.status.success(), "{}", String::from_utf8_lossy(&local.stderr));
    let sizes = prepared.path().join("sizes.csv");
    let sizes_before = std::fs::read_to_string(&sizes).unwrap();

    let addr = spawn_server(prepared.path().to_path_buf()).await;
    let http_csv = prepared.path().join("http.csv");
    let mut args = common(&http_csv);
    args.extend(["--transport".to_string(), "http".to_string()]);
    args.extend(["--base-url".to_string(), format!("http://{addr}")]);
    let output = run(&args.iter().map(String::as_str).collect::<Vec<_>>());
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));

    let text = std::fs::read_to_string(&http_csv).unwrap();
    let rows: Vec<&str> = text.lines().skip(1).collect();
    assert_eq!(rows.len(), 2, "one id-miss row per variant, no write rows:\n{text}");
    for row in &rows {
        assert_eq!(field(row, 2), "id-lookup", "{row}");
        assert!(!field(row, 11).is_empty(), "bytes_read: {row}");
        assert!(!field(row, 12).is_empty(), "http_requests: {row}");
        assert_eq!(field(row, 13), "1", "row_groups_total: {row}");
    }
    assert_eq!(std::fs::read_to_string(&sizes).unwrap(), sizes_before);
}
```

- [ ] **Step 2: Run and watch it fail**

Run: `cargo test --manifest-path benchmark/readbench/Cargo.toml --test variants over_http`
Expected: FAIL — the HTTP run exits non-zero with `--variants runs on --transport local only: there is no server-side write`.

- [ ] **Step 3: Read-only variants over HTTP**

In `coordinator.rs`, delete:

```rust
    if variants.is_some() && opts.transport == Transport::Http {
        bail!("--variants runs on --transport local only: there is no server-side write");
    }
```

and make the write-repeat check local-only:

```rust
    if variants.is_some() && opts.transport == Transport::Local && opts.write_repeat == 0 {
        bail!("--write-repeat must be >= 1");
    }
```

Replace the `variant_seq` binding and the `if let Some(list) = &variants { .. }` block with:

```rust
    let variant_seq: Option<PathBuf> = match (&variants, opts.transport) {
        (Some(_), Transport::Local) => Some(seq_path.clone().ok_or_else(|| {
            anyhow::anyhow!(
                "--variants needs the prepared CityJSONSeq artefact {}.city.jsonl to convert \
                 from (run `just readbench-prepare {}` first)",
                base,
                opts.input.display()
            )
        })?),
        _ => None,
    };
    if let Some(list) = &variants {
        for (id, _) in list {
            match opts.transport {
                Transport::Local => {
                    let seq = variant_seq
                        .as_deref()
                        .expect("set for every local `variants` run");
                    let package = run_write(
                        &mut rows,
                        &mut samples,
                        &dataset,
                        base,
                        id,
                        &opts.prepared_dir,
                        seq,
                        opts.write_repeat,
                    )?;
                    sizes.push(SizeRow {
                        dataset: base.to_string(),
                        label: id.clone(),
                        bytes: dir_bytes(&package)?,
                    });
                    resolved_formats.push((
                        Format::CityParquet,
                        Source::Local(package),
                        id.clone(),
                    ));
                }
                // Read-only: the package a local run wrote under this same
                // name, uploaded beside the prepared artefacts. The write is a
                // local measurement and is not repeated over the network.
                Transport::Http => resolved_formats.push((
                    Format::CityParquet,
                    Source::Http {
                        base_url: opts
                            .base_url
                            .clone()
                            .expect("run validated --base-url for --transport http"),
                        key: format!("{base}.{id}.parquet"),
                    },
                    id.clone(),
                )),
            }
        }
    }
```

At the end of `run`, replace `if variants.is_some() {` (before `write_sizes`) with `if variants.is_some() && opts.transport == Transport::Local {`, and change its `let seq = variant_seq.as_deref().expect("set together with `variants`");` message to `"set for every local variants run"`.

In the module doc's `**--variants: the configuration run.**` paragraph, append: `//! Over `--transport http` the run is read-only: it reads the `<base>.<id>.parquet` packages a local run wrote, uploaded beside the prepared artefacts, and writes no `write` row and no `sizes.csv`.` In `RunOptions::variants`, replace `Exclusive with `formats`; local transport only.` with `Exclusive with `formats`. Over HTTP the packages are read, never written.`

- [ ] **Step 4: Run the harness suite**

Run: `cargo test --manifest-path benchmark/readbench/Cargo.toml && cargo clippy --manifest-path benchmark/readbench/Cargo.toml --all-targets -- -D warnings`
Expected: PASS.

- [ ] **Step 5: The recipe and its test**

In the root `justfile`, after `bloom-bench`, add:

```just
# The bloom axis over HTTP: reads (never writes) the two packages a local
# `bloom-bench` run left in PREPARED, after PREPARED was uploaded to BASE_URL
# (benchmark/scripts/readbench_upload.md). Not part of `bench-run`: it needs a
# real bucket, and its timings are a snapshot of one network path.
[private]
[doc("Bloom axis over HTTP, against uploaded bloom-bench packages")]
bloom-bench-http FOLDER BASE_URL OUT=(BENCH / "runs/formats/scaling_bloom_http_results") PREPARED=(BENCH / "runs/data/readbench") REPEAT='7':
    #!/usr/bin/env bash
    set -euo pipefail
    mkdir -p "{{OUT}}"
    found=0
    while IFS= read -r -d '' f; do
        name="$(basename "$f")"
        for ext in {{KNOWN_INPUT_EXTENSIONS}}; do
            if [[ "$name" == *"$ext" ]]; then name="${name%"$ext"}"; break; fi
        done
        out="{{OUT}}/${name}.csv"
        echo ">> ${f} -> ${out}"
        rm -f "$out"
        cargo run --release {{READBENCH_CARGO}} -- run \
            --input "$f" \
            --prepared-dir "{{PREPARED}}" \
            --out "$out" \
            --repeat {{REPEAT}} \
            --transport http \
            --base-url "{{BASE_URL}}" \
            --scenarios "id-lookup,feature-lookup" \
            --id-probes "id-50pct,id-miss" \
            --feature-probes "feature-50pct,feature-miss" \
            --variants "cityparquet,cityparquet+nobloom"
        found=$((found + 1))
    done < <(find "{{FOLDER}}" -type f \
        \( {{KNOWN_INPUT_FIND}} \) ! -name 'metadata.json' -print0 \
        | sort -z)
    if [[ "$found" -eq 0 ]]; then
        echo "bloom-bench-http: no city-model inputs found under {{FOLDER}}" >&2
        exit 1
    fi
    ./{{BENCH_SCRIPTS}}/machine_record.sh > "{{OUT}}/MACHINE.md"
    echo "bloom-bench-http: ${found} file(s) benchmarked into {{OUT}}"
```

In `bench_recipe_test.sh`, add after `case_bloom_bench_list`:

```bash
case_bloom_http_matches_the_local_pair() {
  local name="bloom-bench-http reads the same pair, scenarios and probes as bloom-bench"
  local body
  body="$(sed -n '/^bloom-bench-http /,/^$/p' "$JUSTFILE")"
  local want
  for want in '--variants "cityparquet,cityparquet+nobloom"' \
    '--scenarios "id-lookup,feature-lookup"' \
    '--id-probes "id-50pct,id-miss"' \
    '--feature-probes "feature-50pct,feature-miss"' \
    '--transport http'; do
    if [[ "$body" != *"$want"* ]]; then
      fail "$name" "bloom-bench-http lacks: $want"
      return
    fi
  done
  pass "$name"
}
```

and call `case_bloom_http_matches_the_local_pair` after `case_bloom_bench_list`. Note: `sed -n '/^bloom-bench /,…'` in `case_bloom_bench_list` matches `bloom-bench ` (with a space) only, so it does not pick up `bloom-bench-http`.

In `benchmark/formats/README.md`'s `## The bloom family`, before the caveat list, add:

```markdown
Over HTTP, `just bloom-bench-http FOLDER BASE_URL` reads — never writes — the
two packages a local `bloom-bench` run left in the prepared directory, once that
directory is uploaded to `BASE_URL` (`benchmark/scripts/readbench_upload.md`).
Its rows add `bytes_read` and `http_requests`; the results go to
`scaling_bloom_http_results/` and are not part of `bench-run` or the rendered
summary. They are a snapshot of one network path at one time.
```

- [ ] **Step 6: Run the gates**

```bash
just scripts-test
cargo test --manifest-path benchmark/readbench/Cargo.toml
cargo fmt --manifest-path benchmark/readbench/Cargo.toml --check
npx --yes prettier@3.9.6 --check benchmark/formats/README.md
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add benchmark/readbench/src/coordinator.rs benchmark/readbench/tests/variants.rs justfile \
  benchmark/scripts/tests/bench_recipe_test.sh benchmark/formats/README.md
git commit -m "feat(readbench): read the bloom pair over HTTP

A --variants run over --transport http reads the packages a local run
wrote and an operator uploaded, writing no write rows and no sizes.
bloom-bench-http drives it with the bloom family's pair, scenarios and
probes."
```

---

### Task 11: Acceptance at corpus scale, and the final gates

The spec's Acceptance list, item by item. Items 2, 3, 4 and 4b are `#[ignore]` tests over the 3DBAG scaling slices; 1, 4c and 5 are commands; 6 is the gates. All scratch output goes under `benchmark/runs/work/bloom-acceptance/`.

**Files:**

- Create: `lib/cityparquet-rs/crates/core/tests/bloom_corpus.rs`.

**Interfaces:**

- Consumes: everything above. Environment: `CITYPARQUET_SCALING_DIR` (a directory holding `3dbag_n10000.city.jsonl` and `3dbag_n1000000.city.jsonl`; here `benchmark/runs/data/scaling`, verified to hold both) and `CITYPARQUET_BLOOM_SCRATCH` (an existing directory under `benchmark/runs/`).
- Produces: evidence for each acceptance item; no library API.

- [ ] **Step 1: Write the corpus tests**

Create `lib/cityparquet-rs/crates/core/tests/bloom_corpus.rs`:

```rust
//! Bloom-filter acceptance at corpus scale (spec Acceptance 2, 3, 4, 4b).
//! Ignored by default: they need the 3DBAG scaling slices and minutes of
//! conversion. Run from the repository root with
//!
//! ```sh
//! mkdir -p benchmark/runs/work/bloom-acceptance
//! CITYPARQUET_SCALING_DIR=$PWD/benchmark/runs/data/scaling \
//! CITYPARQUET_BLOOM_SCRATCH=$PWD/benchmark/runs/work/bloom-acceptance \
//! cargo test --release --all-features --manifest-path lib/cityparquet-rs/Cargo.toml \
//!   -p cityparquet --test bloom_corpus -- --ignored --nocapture --test-threads 1
//! ```
#![cfg(feature = "object-store")]

use std::collections::BTreeMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use arrow_array::{Array, StringArray};
use cityparquet::counting_store::CountingObjectStore;
use cityparquet::package::{ConvertOptions, convert};
use cityparquet::query::{feature_lookup_with_stats, id_lookup, id_lookup_with_stats};
use cityparquet::query_async::{bloom_keep_row_groups_async, id_lookup_async};
use cityparquet::reader::CityParquetReaderBuilder;
use cityparquet::recipe::{BloomPolicy, WriterRecipe};
use cityparquet::schema::CityMetadata;
use object_store::ObjectStore;
use object_store::local::LocalFileSystem;
use object_store::path::Path as ObjectPath;
use parquet::arrow::ProjectionMask;
use parquet::arrow::arrow_reader::{
    ArrowReaderMetadata, ArrowReaderOptions, ParquetRecordBatchReaderBuilder,
};
use parquet::arrow::async_reader::{AsyncFileReader, ParquetObjectReader};
use parquet::file::metadata::ParquetMetaData;
use parquet::schema::types::ColumnPath;

const MISS: &str = "NL.IMBAG.Pand.readbench-absent";

fn slice(name: &str) -> PathBuf {
    let dir = std::env::var("CITYPARQUET_SCALING_DIR")
        .expect("set CITYPARQUET_SCALING_DIR to the directory holding the 3DBAG slices");
    let path = Path::new(&dir).join(name);
    assert!(path.exists(), "missing {}", path.display());
    path
}

fn scratch() -> tempfile::TempDir {
    let dir = std::env::var("CITYPARQUET_BLOOM_SCRATCH")
        .expect("set CITYPARQUET_BLOOM_SCRATCH to a directory under benchmark/runs/");
    tempfile::tempdir_in(dir).unwrap()
}

/// Converts `input` at `row_group_size` rows per group, with or without
/// filters, into a fresh scratch package; returns it and its main table.
fn convert_slice(input: &Path, row_group_size: usize, bloom: bool) -> (tempfile::TempDir, PathBuf) {
    let out = scratch();
    let mut opts = ConvertOptions::new(input.to_path_buf(), out.path().to_path_buf());
    opts.recipe = WriterRecipe {
        row_group_size,
        bloom: BloomPolicy {
            enabled: bloom,
            ..BloomPolicy::default()
        },
        ..WriterRecipe::default()
    };
    convert(&opts).unwrap();
    let table = out.path().join("building.parquet");
    assert!(table.exists(), "3DBAG slices are single-table (building)");
    (out, table)
}

fn column_values(table: &Path, column: &str) -> Vec<String> {
    let builder = ParquetRecordBatchReaderBuilder::try_new(File::open(table).unwrap()).unwrap();
    let mask = ProjectionMask::columns(builder.parquet_schema(), [column]);
    let mut out = Vec::new();
    for batch in builder.with_projection(mask).build().unwrap() {
        let batch = batch.unwrap();
        let values = batch.column(0).as_any().downcast_ref::<StringArray>().unwrap();
        out.extend((0..values.len()).map(|i| values.value(i).to_string()));
    }
    out
}

fn table_meta(table: &Path) -> CityMetadata {
    ParquetRecordBatchReaderBuilder::try_new(File::open(table).unwrap())
        .unwrap()
        .cityparquet_metadata()
        .unwrap()
}

/// Acceptance 2: no false negatives — every `id` of `3dbag_n10000` written
/// at 512-row groups is found by `id_lookup` and `id_lookup_async`.
#[test]
#[ignore]
fn every_id_of_n10000_at_rg512_is_found_sync_and_async() {
    let (out, table) = convert_slice(&slice("3dbag_n10000.city.jsonl"), 512, true);
    let meta = table_meta(&table);
    let ids = column_values(&table, "id");
    println!("n10000 at rg512: {} ids", ids.len());
    let store: Arc<dyn ObjectStore> = Arc::new(LocalFileSystem::new_with_prefix(out.path()).unwrap());
    let path = ObjectPath::from("building.parquet");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    for id in &ids {
        let found = id_lookup(&table, &meta, id).unwrap();
        assert_eq!(found.map(|o| o.id), Some(id.clone()), "sync lost {id}");
        let found = runtime
            .block_on(id_lookup_async(Arc::clone(&store), &path, &meta, id))
            .unwrap();
        assert_eq!(found.map(|o| o.id), Some(id.clone()), "async lost {id}");
    }
}

/// An `AsyncFileReader` that counts its `get_byte_ranges` calls.
#[derive(Clone)]
struct RangeCallCounter<R> {
    inner: R,
    calls: Arc<AtomicUsize>,
}

impl<R: AsyncFileReader> AsyncFileReader for RangeCallCounter<R> {
    fn get_bytes(
        &mut self,
        range: std::ops::Range<u64>,
    ) -> futures::future::BoxFuture<'_, parquet::errors::Result<bytes::Bytes>> {
        self.inner.get_bytes(range)
    }

    fn get_byte_ranges(
        &mut self,
        ranges: Vec<std::ops::Range<u64>>,
    ) -> futures::future::BoxFuture<'_, parquet::errors::Result<Vec<bytes::Bytes>>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inner.get_byte_ranges(ranges)
    }

    fn get_metadata<'a>(
        &'a mut self,
        options: Option<&'a ArrowReaderOptions>,
    ) -> futures::future::BoxFuture<'a, parquet::errors::Result<Arc<ParquetMetaData>>> {
        self.inner.get_metadata(options)
    }
}

/// Acceptance 3 and 4 on the 1M slice at 512-row groups: a miss is ruled out
/// by the filters in at least 95 % of the row groups, for `id` and for
/// `feature_id`; and the async prune fetches every `id` filter through one
/// `get_byte_ranges` call that the store serves in at most 8 requests.
#[test]
#[ignore]
fn a_miss_on_the_1m_slice_at_rg512_is_pruned_and_fetched_in_few_requests() {
    let (out, table) = convert_slice(&slice("3dbag_n1000000.city.jsonl"), 512, true);
    let meta = table_meta(&table);

    let (found, id_stats) = id_lookup_with_stats(&table, &meta, MISS).unwrap();
    assert!(found.is_none());
    let (objects, feature_stats) = feature_lookup_with_stats(&table, &meta, MISS).unwrap();
    assert!(objects.is_empty());
    println!("id: {id_stats:?}\nfeature_id: {feature_stats:?}");
    for stats in [id_stats, feature_stats] {
        assert!(stats.row_groups_total >= 1950, "{stats:?}");
        assert!(
            stats.bloom_pruned * 100 >= stats.row_groups_total * 95,
            "pruned {} of {}",
            stats.bloom_pruned,
            stats.row_groups_total
        );
    }

    let counting = Arc::new(CountingObjectStore::new(
        LocalFileSystem::new_with_prefix(out.path()).unwrap(),
    ));
    let calls = Arc::new(AtomicUsize::new(0));
    let mut reader = RangeCallCounter {
        inner: ParquetObjectReader::new(
            Arc::clone(&counting) as Arc<dyn ObjectStore>,
            ObjectPath::from("building.parquet"),
        ),
        calls: Arc::clone(&calls),
    };
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let arrow_meta = runtime
        .block_on(ArrowReaderMetadata::load_async(&mut reader, ArrowReaderOptions::new()))
        .unwrap();
    let candidates: Vec<usize> = (0..arrow_meta.metadata().num_row_groups()).collect();
    let before = counting.tally();
    calls.store(0, Ordering::SeqCst);
    let prune = runtime
        .block_on(bloom_keep_row_groups_async(
            &mut reader,
            &arrow_meta,
            &ColumnPath::new(vec!["id".to_string()]),
            &[MISS],
            &candidates,
        ))
        .unwrap();
    let after = counting.tally();
    println!(
        "async prune: {} filters, {} requests, {} bytes, {prune:?}",
        candidates.len(),
        after.requests - before.requests,
        after.bytes - before.bytes
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(after.requests - before.requests <= 8);
}

/// Acceptance 4b at 3DBAG scale: `feature_lookup` returns every row of a
/// multi-part feature (a Building and its BuildingParts), identical with and
/// without filters.
#[test]
#[ignore]
fn feature_lookup_returns_every_part_of_multi_part_3dbag_features() {
    let input = slice("3dbag_n10000.city.jsonl");
    let (_on, on_table) = convert_slice(&input, 512, true);
    let (_off, off_table) = convert_slice(&input, 512, false);
    let mut expected: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (id, feature) in column_values(&on_table, "id")
        .into_iter()
        .zip(column_values(&on_table, "feature_id"))
    {
        expected.entry(feature).or_default().push(id);
    }
    let multi: Vec<(&String, &Vec<String>)> =
        expected.iter().filter(|(_, ids)| ids.len() >= 2).collect();
    assert!(!multi.is_empty(), "3DBAG features are Building + BuildingPart");
    for (feature, ids) in multi.iter().step_by((multi.len() / 200).max(1)) {
        for table in [&on_table, &off_table] {
            let (objects, _) = feature_lookup_with_stats(table, &table_meta(table), feature).unwrap();
            let got: Vec<String> = objects.into_iter().map(|o| o.id).collect();
            assert_eq!(&got, *ids, "{feature} in {}", table.display());
        }
    }
}
```

- [ ] **Step 2: Run them**

```bash
mkdir -p benchmark/runs/work/bloom-acceptance
CITYPARQUET_SCALING_DIR=$PWD/benchmark/runs/data/scaling \
CITYPARQUET_BLOOM_SCRATCH=$PWD/benchmark/runs/work/bloom-acceptance \
cargo test --release --all-features --manifest-path lib/cityparquet-rs/Cargo.toml \
  -p cityparquet --test bloom_corpus -- --ignored --nocapture --test-threads 1
```

Expected: three passes. Record the printed `id`/`feature_id` statistics (the 1M slice at 512-row groups is about 1954 row groups) and the async request count in the review notes. A failure here is a finding, not a threshold to relax: report it.

- [ ] **Step 3: Acceptance 1 — the written 3DBAG package, read by DuckDB**

```bash
W=benchmark/runs/work/bloom-acceptance
DUCKDB=lib/duckdb-cityjson/build/release/duckdb
cargo build --release --manifest-path lib/cityparquet-rs/Cargo.toml -p cityparquet-cli
lib/cityparquet-rs/target/release/cityparquet convert benchmark/runs/data/scaling/3dbag_n10000.city.jsonl -o $W/rs_n10000 --overwrite
$DUCKDB -c "
WITH m AS (SELECT * FROM parquet_metadata('$W/rs_n10000/building.parquet')),
     e AS (SELECT max(coalesce(nullif(dictionary_page_offset, 0), data_page_offset) + total_compressed_size) AS data_end FROM m)
SELECT path_in_schema,
       count(*) FILTER (WHERE bloom_filter_offset IS NOT NULL) AS filtered,
       count(*) AS row_groups,
       bool_and(bloom_filter_offset IS NULL OR bloom_filter_offset >= (SELECT data_end FROM e)) AS after_data,
       bool_and(bloom_filter_offset IS NULL OR bloom_filter_length IS NOT NULL) AS has_length
FROM m GROUP BY path_in_schema HAVING filtered > 0 ORDER BY 1;"
$DUCKDB -c "
SELECT column_name, approx_unique, count, null_percentage,
       approx_unique / (count * (1 - null_percentage / 100)) AS distinct_ratio
FROM (SUMMARIZE SELECT * FROM read_parquet('$W/rs_n10000/building.parquet'))
WHERE column_type = 'VARCHAR' ORDER BY distinct_ratio DESC;"
```

Expected: the first query lists exactly `feature_id`, `id` and the attribute columns whose `distinct_ratio` in the second query is at least 0.2 (among them `identificatie`, never `status`); `filtered = row_groups`, `after_data` and `has_length` true on every row. `object_type`, `other` and JSON columns are VARCHAR too but are never candidates — ignore them in the second list. A column whose ratio lies within ±0.02 of 0.2 may differ between the two estimators; note it rather than treat it as a failure. Sidecars: 3DBAG has none; `sidecars_carry_no_filter_and_every_object_table_filters_its_identifiers` (Task 1) covers them in the gate.

- [ ] **Step 4: Acceptance 4c — a duckdb-cityjson package passes the same check**

```bash
# The extension's own CLI build links the extension in; no LOAD is needed.
$DUCKDB <<SQL
CREATE SCHEMA bag;
CREATE TABLE bag.building AS SELECT * FROM read_cityjsonseq('benchmark/runs/data/scaling/3dbag_n10000.city.jsonl');
PRAGMA cityparquet_init('bag');
SELECT * FROM cityparquet_write('bag', '$W/duckdb_n10000', crs => 'EPSG:7415');
SQL
$DUCKDB -c "
WITH m AS (SELECT * FROM parquet_metadata('$W/duckdb_n10000/building.parquet')),
     e AS (SELECT max(coalesce(nullif(dictionary_page_offset, 0), data_page_offset) + total_compressed_size) AS data_end FROM m)
SELECT path_in_schema,
       count(*) FILTER (WHERE bloom_filter_offset IS NOT NULL) AS filtered,
       count(*) AS row_groups,
       bool_and(bloom_filter_offset IS NULL OR bloom_filter_offset >= (SELECT data_end FROM e)) AS after_data,
       bool_and(bloom_filter_offset IS NULL OR bloom_filter_length IS NOT NULL) AS has_length
FROM m GROUP BY path_in_schema HAVING filtered > 0 ORDER BY 1;"
ls $W/duckdb_n10000
```

Expected: `id`, `feature_id` and `identificatie` are listed with `filtered = row_groups`; the list is a superset of Step 3's (low-cardinality strings too, as documented); `after_data` and `has_length` true; no sidecar file is written for 3DBAG (the sidecar rule is pinned by `test/sql/cityparquet_bloom.test`).

- [ ] **Step 5: Acceptance 5 — `+nobloom` is today's layout, byte for byte**

The base is the commit that added this plan, before any implementation.

```bash
BASE=$(git log --format=%H -1 --grep='^docs(plan): bloom filters')
git worktree add $W/base "$BASE"
CARGO_TARGET_DIR=$PWD/$W/base-target cargo build --release \
  --manifest-path $W/base/lib/cityparquet-rs/Cargo.toml -p cityparquet-cli
OLD=$W/base-target/release/cityparquet
NEW=lib/cityparquet-rs/target/release/cityparquet
for input in lib/cityparquet-rs/tests/fixtures/delft.city.jsonl benchmark/runs/data/scaling/3dbag_n10000.city.jsonl; do
  name=$(basename "$input" .city.jsonl)
  $OLD convert "$input" -o $W/old_a_$name --overwrite
  $OLD convert "$input" -o $W/old_b_$name --overwrite
  $NEW convert "$input" -o $W/new_nobloom_$name --overwrite --no-bloom
  cmp $W/old_a_$name/building.parquet $W/old_b_$name/building.parquet && echo "$name: base is deterministic"
  cmp $W/old_a_$name/building.parquet $W/new_nobloom_$name/building.parquet && echo "$name: +nobloom identical"
done
git worktree remove $W/base
```

Expected: both lines printed for both inputs. If the base itself is not deterministic (the first `cmp` fails), compare layouts instead — `$DUCKDB -c "SELECT * EXCLUDE (file_name) FROM parquet_metadata('$W/old_a_$name/building.parquet') EXCEPT SELECT * EXCLUDE (file_name) FROM parquet_metadata('$W/new_nobloom_$name/building.parquet')"` must return no rows — and report the non-determinism.

- [ ] **Step 6: A smoke run of the family**

```bash
just bench-prep --families bloom --datasets 3dbag --smoke
just bench-run --families bloom --datasets 3dbag --smoke
just bench-summary --families bloom --smoke
```

Expected: `benchmark/runs/formats/smoke/scaling_bloom_results/3dbag_n1000.csv` holds 10 rows (a write and four lookups per variant) with the four counters on every lookup row, and `benchmark/runs/summary/smoke/` holds `bloom.svg` and `bloom-scaling.svg`. A smoke run is not a publication run.

- [ ] **Step 7: Acceptance 6 — the gates**

```bash
(cd lib/cityparquet-rs && just check)
just plot-test
just scripts-test
cargo clippy --manifest-path benchmark/readbench/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path benchmark/readbench/Cargo.toml
cargo fmt --manifest-path benchmark/readbench/Cargo.toml --check
just check
```

Expected: every command passes. `just check` also runs `mcp-check` (needs the submodules) and `citylake-check` (needs the local duckdb-cityjson build from Task 6).

- [ ] **Step 8: Commit**

```bash
git add lib/cityparquet-rs/crates/core/tests/bloom_corpus.rs
git commit -m "test(query): bloom-filter acceptance at corpus scale

Ignored tests over the 3DBAG scaling slices: no false negatives for
every id of n10000 at 512-row groups (sync and async), at least 95 %
of row groups pruned for an id and a feature_id miss on the 1M slice,
one ranged call in at most 8 requests for its id filters, and every
part of multi-part features with and without filters."
```

Then record in the pull-request description (or the review hand-off): the printed statistics from Step 2, the column lists from Steps 3-4, and the outcome of Step 5.

---

## Spec coverage

| Spec section | Task |
|---|---|
| Decisions 1-2 (columns, FPP) | 1, 2 |
| Decision 3 (End placement, lengths) | 1 |
| Decision 4 (async one coalesced request) | 4 |
| Decision 5 (default on, disableable) | 1 |
| Decision 6 (both writers; `feature_id` lookup) | 5, 6 |
| Decision 7 (one `bloom` family, one pair) | 9, 10 |
| Decision 8 (crates, not hand-rolled) | 2 (HLL crate), 3-4 (`Sbbf`) |
| Writer — column policy, numeric exclusion | 1, 2 |
| Writer — high-cardinality rule, `ScanResult`, diversion, `writer_properties` signature, dotted names | 2 |
| Writer — properties, NDV, memory | 1 (properties), 9 (write-row RSS caveat) |
| Writer — `BloomPolicy`, `ParquetDefaults` | 1 |
| Surfaces — CLI flags, `+nobloom` | 1 |
| Reader — `bloom_targets`, sync/async prune, leaf by path, IN semantics, raw bytes, exact `RowFilter` | 3, 4 |
| Reader — fallback without length | 4 |
| Reader — `LookupStats`, `_with_stats`, readbench adapter | 3, 4, 8 |
| `feature_id` lookup (table, async, package; CLI) | 5 (CLI: none exists, resolved note 5) |
| Call sites incl. `attr_filter` and its doc comment | 3, 4 |
| Specification (documents/) | 7 |
| duckdb-cityjson writer | 6 |
| Benchmark — variants, scenarios, probes, datasets, metrics | 8, 9, 10 |
| Benchmark — disclosed caveats, knock-on for codec/rowgroup | 9 |
| Outputs under `benchmark/runs/` only | 9, 10, 11 |
| Acceptance 1-6 | 11 (2, 3, 4, 4b also on fixtures in 3-5; 4c in 6) |
| Out of scope (numeric, sidecars, page index, FPP sweep) | not implemented; sidecars pinned filter-free in 1 and 6 |
| Superseding the 2026-08-25 out-of-scope note | 7 |
