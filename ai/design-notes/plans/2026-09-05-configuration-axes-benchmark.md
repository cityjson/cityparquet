# Configuration-axes benchmark Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Measure the codec axis and the row-group axis of the CityParquet writer with the read benchmark's own harness (child process per measurement, peak RSS, the paper's query primitives), over the 3DBAG scaling slices, and draw one read-figure-style sheet per axis.

**Architecture:** The variant grammar moves into the core crate as `cityparquet::variant::Variant`. The read coordinator (`benchmark/readbench`) gains `--variants`: for every variant it spawns a write child that converts the slice and reports time and peak RSS, keeps the package under `<prepared_dir>/<base>.<variant>.parquet`, then runs the ordinary read children against it. The CSV shape is unchanged (a `write` row per variant, the variant id in the `format` column, package bytes in `sizes.csv`). `benchviz` gains one loader and one figure function used for both axes.

**Tech Stack:** Rust 2024 (two Cargo workspaces: `lib/cityparquet-rs`, `benchmark/readbench`), `just`, bash test suites under `benchmark/scripts/tests`, Python 3 + matplotlib under `benchmark/plot` (run with `uv`).

**Spec:** `ai/design-notes/specs/2026-09-04-configuration-axes-benchmark-design.md`

## Global Constraints

- Branch `bench/configuration-axes` in the worktree `/data2/hideba/cityparquet-paper/cityparquet-bench/` (from `develop` at 75d0ca6). Every path below is relative to that root unless absolute.
- British English in prose and comments. No changelog voice in docs: describe the present.
- Rust gate: `cd lib/cityparquet-rs && just check` (clippy `-D warnings`, tests, fmt). Harness gate: `cargo clippy --manifest-path benchmark/readbench/Cargo.toml --all-targets -- -D warnings`, `cargo test --manifest-path benchmark/readbench/Cargo.toml`, `cargo fmt --manifest-path benchmark/readbench/Cargo.toml --check`.
- **Known-red baseline on this host:** the harness test binaries `flatcitybuf_runner` and `flatcitybuf_http_runner` (6 tests) fail at their `fcb ser` shell-out before any assertion. They are unrelated. Every "run the harness tests" step below means: everything green except those two binaries. Scope with `--test <name>` when iterating.
- Python suite: `just plot-test` (48 tests green at baseline). Shell suites: `just scripts-test` (needs `jq`).
- Variant grammar: `<preset>[+hilbert][+rg<N>][+<codec>[<level>]]`. Only `zstd` takes a level, 1 to 22. The bare id `cityparquet` is the baseline of both axes.
- Codec list, in figure order: `cityparquet,cityparquet+zstd1,cityparquet+zstd9,cityparquet+zstd19,cityparquet+lz4,cityparquet+snappy,cityparquet+gzip,cityparquet+brotli,cityparquet+uncompressed`.
- Row-group list, in figure order: `cityparquet,cityparquet+rg32768,cityparquet+rg8192,cityparquet+rg2048,cityparquet+rg512`.
- Package naming: `<prepared_dir>/<base>.<variant-id>.parquet`.
- Ratio convention in `bench_data.json` for the two new axes: baseline over variant, so above 1× is faster, leaner or smaller. Do **not** copy `formats()`'s `1.0 / ratio` inversion.
- Every commit message ends with the line `Claude-Session: https://claude.ai/code/session_012rPUqbWax2xKR4GuT8Bmtq`.
- `AGENTS.md` mirrors `CLAUDE.md` at each level: edit one, copy to the other.

---

## File structure

| File | Responsibility |
|---|---|
| `lib/cityparquet-rs/crates/core/src/variant.rs` (new) | `Variant`: parse, canonical id, recipe, ordering. The one grammar. |
| `lib/cityparquet-rs/crates/core/src/lib.rs` | `pub mod variant;` |
| `lib/cityparquet-rs/crates/cli/src/bench.rs` | Uses `Variant`; its own parser is deleted. |
| `lib/cityparquet-rs/crates/cli/src/main.rs` | `--variants` help text names the new grammar. |
| `lib/cityparquet-rs/crates/cli/tests/bench_smoke.rs` | One new test: a zstd level suffix is accepted. |
| `benchmark/readbench/src/main.rs` | `--child --write` path; `RunArgs` gains `--variants`, `--write-repeat`. |
| `benchmark/readbench/src/coordinator.rs` | `Measure`, `Row.label`, `run_write`, `sizes.csv`, `--variants` validation and loop. |
| `benchmark/readbench/tests/write_child.rs` (new) | The write child's protocol and package. |
| `benchmark/readbench/tests/variants.rs` (new) | The `--variants` run end to end on the delft fixture, plus rejections. |
| `benchmark/scripts/machine_record.sh` (new) | Prints `MACHINE.md`. |
| `justfile` | `variant-bench`, `codec-bench`, `rowgroup-bench`; `compression-bench`, `compression-plot` removed. |
| `benchmark/scripts/tests/bench_recipe_test.sh` | Pins the two variant lists. |
| `benchmark/readbench/tests/strip_extension.rs` | Comment naming the four per-dataset recipes. |
| `benchmark/plot/benchviz/prep.py` | `load_scaling_axis`, machine record, meta; compression loader removed. |
| `benchmark/plot/benchviz/figures.py` | `axis_sheet`, `_axis_headline`; `compression` removed. |
| `benchmark/plot/benchviz/html.py` | Compression view removed; section 3b repointed. |
| `benchmark/plot/benchviz/DESIGN.md` | Data contract: `scaling.codec`, `scaling.rowgroup`. |
| `benchmark/plot/tests/fixtures/benchviz/scaling_codec_results/`, `.../scaling_rowgroup_results/` (new) | Measured fixture CSVs from a real delft run. |
| `benchmark/plot/tests/test_benchviz.py` | Axis loader and figure tests; compression tests removed. |
| `benchmark/plot/readbench_plot/compression.py` | Deleted. |
| `benchmark/formats/compression_results/`, `benchmark/formats/scaling_compression_results/` | Deleted. |
| `benchmark/formats/scaling_codec_results/`, `benchmark/formats/scaling_rowgroup_results/` (new) | The measured runs, committed. |
| Docs: `CLAUDE.md`, `AGENTS.md`, `lib/cityparquet-rs/{CLAUDE,AGENTS,README}.md`, `benchmark/README.md`, `benchmark/formats/README.md`, `benchmark/formats/READ_BENCHMARK.md`, `test/TESTING.md` | Every mention of `compression-bench`. |

---

### Task 1: `Variant` in the core crate

**Files:**
- Create: `lib/cityparquet-rs/crates/core/src/variant.rs`
- Modify: `lib/cityparquet-rs/crates/core/src/lib.rs` (module list, alphabetical after `stac`)

**Interfaces:**
- Consumes: `crate::recipe::{Codec, RecipePreset, WriterRecipe}`, `crate::package::RowOrder`, `cityparquet_schema::{CityParquetError, Result}`, `parquet::basic::ZstdLevel`.
- Produces:
  ```rust
  pub const GRAMMAR: &str = "<preset>[+hilbert][+rg<N>][+<codec>[<level>]]";
  #[derive(Debug, Clone, Copy, PartialEq, Eq)]
  pub struct Variant { pub preset: RecipePreset, pub ordering: RowOrder,
                       pub row_group_size: Option<usize>, pub compression: Option<Codec>,
                       pub zstd_level: Option<i32> }
  impl Variant {
      pub fn parse(id: &str) -> Result<Variant>;
      pub fn id(&self) -> String;          // canonical spelling; parse(id()) == self
      pub fn recipe(&self) -> WriterRecipe;
      pub fn ordering(&self) -> RowOrder;
  }
  ```

- [ ] **Step 1: Write the failing tests** (at the bottom of the new file, so the file must exist with the type stubbed; write the tests first, then the implementation in step 3)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const CODEC_LIST: [&str; 9] = [
        "cityparquet", "cityparquet+zstd1", "cityparquet+zstd9", "cityparquet+zstd19",
        "cityparquet+lz4", "cityparquet+snappy", "cityparquet+gzip", "cityparquet+brotli",
        "cityparquet+uncompressed",
    ];
    const ROWGROUP_LIST: [&str; 5] = [
        "cityparquet", "cityparquet+rg32768", "cityparquet+rg8192", "cityparquet+rg2048",
        "cityparquet+rg512",
    ];

    #[test]
    fn the_two_benchmark_lists_round_trip_through_their_canonical_ids() {
        for id in CODEC_LIST.iter().chain(ROWGROUP_LIST.iter()) {
            let v = Variant::parse(id).unwrap();
            assert_eq!(v.id(), *id, "canonical spelling must be the list's spelling");
            assert_eq!(Variant::parse(&v.id()).unwrap(), v);
        }
    }

    #[test]
    fn a_zstd_level_reaches_the_recipe_and_a_bare_zstd_keeps_the_default() {
        let nine = Variant::parse("cityparquet+zstd9").unwrap();
        assert_eq!(nine.compression, Some(Codec::Zstd));
        assert_eq!(nine.zstd_level, Some(9));
        assert_eq!(nine.recipe().zstd_level, 9);
        assert_eq!(nine.recipe().compression, Some(Codec::Zstd));

        let bare = Variant::parse("cityparquet+zstd").unwrap();
        assert_eq!(bare.zstd_level, None);
        assert_eq!(bare.recipe().zstd_level, 3);
        assert_eq!(bare.id(), "cityparquet+zstd");

        let default = Variant::parse("cityparquet").unwrap();
        assert_eq!(default.recipe(), WriterRecipe::default());
        assert_eq!(default.ordering(), RowOrder::Source);
    }

    #[test]
    fn only_zstd_takes_a_level() {
        for id in ["cityparquet+gzip6", "cityparquet+brotli11", "cityparquet+snappy1"] {
            let err = Variant::parse(id).unwrap_err().to_string();
            assert!(err.contains(id), "{err}");
            assert!(err.contains("only zstd takes a level"), "{err}");
        }
    }

    #[test]
    fn a_zstd_level_outside_the_codec_range_is_rejected() {
        for id in ["cityparquet+zstd0", "cityparquet+zstd23"] {
            let err = Variant::parse(id).unwrap_err().to_string();
            assert!(err.contains(id), "{err}");
        }
    }

    #[test]
    fn duplicates_malformed_suffixes_and_unknown_presets_are_rejected_with_the_grammar() {
        for id in [
            "cityparquet+hilbert+hilbert", "cityparquet+rg4096+rg8192", "cityparquet+gzip+zstd",
            "cityparquet+rg0", "cityparquet+rg-1", "cityparquet+rgabc", "cityparquet+rg",
            "not-a-real-preset", "cityparquet+bogus", "",
        ] {
            let err = Variant::parse(id).unwrap_err().to_string();
            assert!(err.contains(id), "error must name the offending id: {err}");
            assert!(err.contains(GRAMMAR), "error must show the grammar: {err}");
        }
    }

    #[test]
    fn suffix_order_on_input_does_not_matter_but_the_id_is_canonical() {
        let a = Variant::parse("cityparquet+rg512+hilbert+gzip").unwrap();
        let b = Variant::parse("cityparquet+gzip+hilbert+rg512").unwrap();
        assert_eq!(a, b);
        assert_eq!(a.id(), "cityparquet+hilbert+rg512+gzip");
        assert_eq!(a.ordering(), RowOrder::Hilbert);
        assert_eq!(a.recipe().row_group_size, 512);
        assert_eq!(a.recipe().compression, Some(Codec::Gzip));
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd lib/cityparquet-rs && cargo test -p cityparquet variant::`
Expected: compile error (`Variant` not defined) or, with a stub, assertion failures.

- [ ] **Step 3: Write the implementation** (above the tests in the same file)

```rust
//! Benchmark variant identifiers — the one grammar behind `cityparquet bench
//! --variants` and `cityparquet-readbench run --variants`.
//!
//! An id is `<preset>[+hilbert][+rg<N>][+<codec>[<level>]]`: a
//! [`RecipePreset`] name, then any of three suffixes, each at most once, in
//! any order on input. [`Variant::id`] spells the same variant back in the
//! fixed order above, and that spelling is what the result CSVs carry.
//!
//! Only `zstd` takes a level (`zstd9`), because zstd is the codec CityParquet
//! ships with and the benchmark sweeps its effort. Every other codec runs at
//! the parquet-rs default the recipe carries (gzip 6, brotli 1), and a level
//! on one of them is a grammar error rather than a second sweep nobody
//! matched across codecs.

use parquet::basic::ZstdLevel;

use cityparquet_schema::{CityParquetError, Result};

use crate::package::RowOrder;
use crate::recipe::{Codec, RecipePreset, WriterRecipe};

/// The grammar, as printed in every rejection.
pub const GRAMMAR: &str = "<preset>[+hilbert][+rg<N>][+<codec>[<level>]]";

/// One parsed variant id. See the module doc for the grammar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Variant {
    pub preset: RecipePreset,
    pub ordering: RowOrder,
    pub row_group_size: Option<usize>,
    pub compression: Option<Codec>,
    /// Only ever `Some` together with `compression == Some(Codec::Zstd)`.
    pub zstd_level: Option<i32>,
}

impl Variant {
    /// Parses an id; the error names the id and prints [`GRAMMAR`].
    pub fn parse(id: &str) -> Result<Variant> {
        let mut parts = id.split('+');
        let preset_name = parts.next().unwrap_or("");
        let preset = RecipePreset::parse(preset_name).ok_or_else(|| grammar_err(id, None))?;

        let mut ordering = RowOrder::Source;
        let mut row_group_size: Option<usize> = None;
        let mut compression: Option<Codec> = None;
        let mut zstd_level: Option<i32> = None;
        let mut seen_hilbert = false;
        for part in parts {
            if let Some(digits) = part.strip_prefix("rg") {
                if row_group_size.is_some() {
                    return Err(grammar_err(id, None));
                }
                let n: usize = digits.parse().map_err(|_| grammar_err(id, None))?;
                if n == 0 {
                    return Err(grammar_err(id, None));
                }
                row_group_size = Some(n);
                continue;
            }
            if let Some((codec, level)) = split_codec_token(part) {
                if compression.is_some() {
                    return Err(grammar_err(id, None));
                }
                match (codec, level) {
                    (_, None) => {}
                    (Codec::Zstd, Some(level)) => {
                        ZstdLevel::try_new(level).map_err(|e| {
                            grammar_err(id, Some(&format!("zstd level {level} is out of range: {e}")))
                        })?;
                        zstd_level = Some(level);
                    }
                    (other, Some(_)) => {
                        return Err(grammar_err(
                            id,
                            Some(&format!("only zstd takes a level, not {}", other.name())),
                        ));
                    }
                }
                compression = Some(codec);
                continue;
            }
            match part {
                "hilbert" if !seen_hilbert => {
                    seen_hilbert = true;
                    ordering = RowOrder::Hilbert;
                }
                _ => return Err(grammar_err(id, None)),
            }
        }

        Ok(Variant {
            preset,
            ordering,
            row_group_size,
            compression,
            zstd_level,
        })
    }

    /// The canonical spelling: preset, then `+hilbert`, `+rg<N>`, `+<codec>[<level>]`.
    pub fn id(&self) -> String {
        let mut id = self.preset.name().to_string();
        if self.ordering == RowOrder::Hilbert {
            id.push_str("+hilbert");
        }
        if let Some(n) = self.row_group_size {
            id.push_str(&format!("+rg{n}"));
        }
        if let Some(codec) = self.compression {
            id.push('+');
            id.push_str(codec.name());
            if let Some(level) = self.zstd_level {
                id.push_str(&level.to_string());
            }
        }
        id
    }

    /// The preset's recipe with this variant's row-group size, codec and
    /// zstd level applied on top.
    pub fn recipe(&self) -> WriterRecipe {
        let mut recipe = self.preset.recipe();
        if let Some(row_group_size) = self.row_group_size {
            recipe.row_group_size = row_group_size;
        }
        if let Some(compression) = self.compression {
            recipe.compression = Some(compression);
        }
        if let Some(level) = self.zstd_level {
            recipe.zstd_level = level;
        }
        recipe
    }

    pub fn ordering(&self) -> RowOrder {
        self.ordering
    }
}

/// `"zstd9"` -> `(Zstd, Some(9))`; `"gzip"` -> `(Gzip, None)`; `"gzip6"` ->
/// `(Gzip, Some(6))` (the caller rejects it); anything else -> `None`.
fn split_codec_token(part: &str) -> Option<(Codec, Option<i32>)> {
    if let Some(codec) = Codec::parse(part) {
        return Some((codec, None));
    }
    let split = part.find(|c: char| c.is_ascii_digit())?;
    let (name, digits) = part.split_at(split);
    let codec = Codec::parse(name)?;
    let level: i32 = digits.parse().ok()?;
    Some((codec, Some(level)))
}

fn grammar_err(id: &str, detail: Option<&str>) -> CityParquetError {
    let presets: Vec<&str> = RecipePreset::ALL.iter().map(|p| p.name()).collect();
    let codecs: Vec<&str> = Codec::ALL.iter().map(|c| c.name()).collect();
    let detail = detail.map(|d| format!(" ({d})")).unwrap_or_default();
    CityParquetError::Schema(format!(
        "invalid variant '{id}'{detail}: expected `{GRAMMAR}` (each suffix at most once, <N> a \
         positive integer, <codec> one of: {}, <level> only after zstd, 1-22) where preset is \
         one of: {}",
        codecs.join(", "),
        presets.join(", ")
    ))
}
```

Add `pub mod variant;` to `lib/cityparquet-rs/crates/core/src/lib.rs` between `pub mod stac;` and `pub mod wkb_read;`.

Note for the `"cityparquet+zstd"` case: `Codec::parse("zstd")` matches first in `split_codec_token`, so the level stays `None`. Note for `"cityparquet+gzip+zstd"`: the second codec token hits `compression.is_some()` and is rejected before the level check.

- [ ] **Step 4: Run the tests and the library gate**

Run: `cd lib/cityparquet-rs && cargo test -p cityparquet variant:: && just check`
Expected: 6 variant tests pass; `just check` green.

- [ ] **Step 5: Commit**

```bash
git add lib/cityparquet-rs/crates/core/src/variant.rs lib/cityparquet-rs/crates/core/src/lib.rs
git commit -m "feat(core): move the benchmark variant grammar into the library, with a zstd level suffix

Claude-Session: https://claude.ai/code/session_012rPUqbWax2xKR4GuT8Bmtq"
```

---

### Task 2: `cityparquet bench` uses `Variant`

**Files:**
- Modify: `lib/cityparquet-rs/crates/cli/src/bench.rs` (delete `ParsedVariant`, `parse_variant`, `variant_grammar_err`; `run_variant` takes a `Variant`)
- Modify: `lib/cityparquet-rs/crates/cli/src/main.rs:174-179` (the `--variants` doc comment)
- Test: `lib/cityparquet-rs/crates/cli/tests/bench_smoke.rs`

**Interfaces:**
- Consumes: `cityparquet::variant::{Variant, GRAMMAR}` from Task 1.
- Produces: nothing new. `BenchOptions` and `run` keep their signatures.

- [ ] **Step 1: Write the failing test** (append to `bench_smoke.rs`; copy the shape of `bench_run_accepts_a_codec_suffix_combined_with_rg` at line 321 for the CSV read-back)

```rust
/// The zstd level suffix is part of the shared grammar now, and the bench
/// must pass it through to the writer: zstd 1 and zstd 19 are different
/// codecs' worth of bytes on the same input.
#[test]
fn bench_run_accepts_a_zstd_level_suffix_and_the_levels_differ_in_bytes() {
    let out_dir = tempfile::tempdir().unwrap();
    let opts = BenchOptions {
        input: fixture("delft.city.jsonl"),
        out_csv: out_dir.path().join("bench.csv"),
        repeat: 1,
        variants: vec![
            "cityparquet+zstd1".to_string(),
            "cityparquet+zstd19".to_string(),
        ],
        window_frac: 0.05,
        skip_roundtrip: true,
    };
    run(&opts).expect("zstd level suffixes must be accepted");
    let text = std::fs::read_to_string(out_dir.path().join("bench.csv")).unwrap();
    let rows: Vec<&str> = text.lines().skip(1).collect();
    assert_eq!(rows.len(), 2);
    let bytes = |row: &str| -> u64 { row.split(',').nth(4).unwrap().parse().unwrap() };
    assert!(rows[0].starts_with("delft.city.jsonl,cityparquet+zstd1,"));
    assert!(rows[1].starts_with("delft.city.jsonl,cityparquet+zstd19,"));
    assert_ne!(bytes(rows[0]), bytes(rows[1]), "two zstd levels wrote the same bytes");
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cd lib/cityparquet-rs && cargo test -p cityparquet-cli --test bench_smoke zstd_level`
Expected: FAIL, the error message says `invalid variant 'cityparquet+zstd1'`.

- [ ] **Step 3: Swap the bench onto `Variant`**

In `bench.rs`:
- Replace `use cityparquet::recipe::{Codec, RecipePreset, WriterRecipe};` with `use cityparquet::recipe::RecipePreset;` and add `use cityparquet::variant::Variant;`. Drop the now-unused `RowOrder` from the `package` import if nothing else uses it (`convert_opts.ordering = variant.ordering();` needs no import of the type).
- Delete `struct ParsedVariant`, its `impl`, `fn parse_variant`, `fn variant_grammar_err`, and the module doc lines that describe the grammar (point them at `cityparquet::variant` instead).
- In `run`: `.map(|id| Variant::parse(id).map(|parsed| (id.clone(), parsed)))`.
- In `run_variant(opts, dataset, variant_id, variant: Variant)`: `convert_opts.recipe = variant.recipe(); convert_opts.ordering = variant.ordering();` in both places.

In `main.rs` lines 174-179, the `--variants` doc comment becomes:

```rust
        /// Comma-separated variant identifiers
        /// (`<preset>[+hilbert][+rg<N>][+<codec>[<level>]]`, e.g.
        /// `cityparquet+hilbert`, `cityparquet+rg512`, `cityparquet+zstd9`;
        /// see `cityparquet::variant`); omit for the default 9-variant set
```

- [ ] **Step 4: Run the CLI tests and the library gate**

Run: `cd lib/cityparquet-rs && cargo test -p cityparquet-cli --test bench_smoke && just check`
Expected: all bench_smoke tests pass, including `bench_run_rejects_an_unknown_variant_with_the_grammar_in_the_message` (its `<preset>[+hilbert][+rg<N>]` substring is a prefix of the new `GRAMMAR`). `just check` green.

- [ ] **Step 5: Commit**

```bash
git add lib/cityparquet-rs/crates/cli
git commit -m "refactor(cli): parse bench variants through cityparquet::variant

Claude-Session: https://claude.ai/code/session_012rPUqbWax2xKR4GuT8Bmtq"
```

---

### Task 3: the write child in `cityparquet-readbench`

**Files:**
- Modify: `benchmark/readbench/src/main.rs` (`Cli` gains `--write`, `--variant`, `--out`; `run` dispatches; new `run_write_child`)
- Create: `benchmark/readbench/tests/write_child.rs`

**Interfaces:**
- Consumes: `cityparquet::variant::{Variant, GRAMMAR}`, `cityparquet::package::{convert, ConvertOptions}`, existing `alloc::{reset, peak_heap_bytes}`, `max_rss_bytes`.
- Produces: the child protocol line for a write, printed to stdout: `"{time_s:.6} {peak_heap_bytes} {ru_maxrss_bytes} {object_count}"`, four fields, the same shape a read child prints when it has no I/O stats. Invocation: `cityparquet-readbench --child --write --variant <id> --input <seq> --out <dir>`. `<dir>` must not exist beforehand.

- [ ] **Step 1: Write the failing test**

```rust
//! The write child: `--child --write --variant <id> --input <seq> --out <dir>`
//! converts one input with one variant's recipe, prints the four-field child
//! protocol line, and leaves the package at `--out`. The coordinator's
//! `--variants` path is built on exactly this.

use std::path::PathBuf;
use std::process::Command;

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../lib/cityparquet-rs/tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

fn row_groups_in(package: &std::path::Path) -> usize {
    use parquet::file::reader::{FileReader, SerializedFileReader};
    let table = std::fs::read_dir(package)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|x| x == "parquet"))
        .expect("the package holds a .parquet table");
    let reader = SerializedFileReader::new(std::fs::File::open(table).unwrap()).unwrap();
    reader.metadata().num_row_groups()
}

#[test]
fn a_write_child_prints_the_protocol_line_and_applies_the_variants_recipe() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("pkg");
    let output = Command::new(env!("CARGO_BIN_EXE_cityparquet-readbench"))
        .args([
            "--child", "--write",
            "--variant", "cityparquet+rg512",
            "--input", fixture("delft.city.jsonl").to_str().unwrap(),
            "--out", out.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "stderr:\n{}", String::from_utf8_lossy(&output.stderr));

    let stdout = String::from_utf8(output.stdout).unwrap();
    let fields: Vec<&str> = stdout.split_whitespace().collect();
    assert_eq!(fields.len(), 4, "four fields, got: {stdout:?}");
    let time_s: f64 = fields[0].parse().unwrap();
    let peak_heap: u64 = fields[1].parse().unwrap();
    let peak_rss: u64 = fields[2].parse().unwrap();
    let objects: u64 = fields[3].parse().unwrap();
    assert!(time_s > 0.0);
    assert!(peak_heap > 0);
    assert!(peak_rss > 0);
    assert_eq!(objects, 2231, "delft has 2231 CityObjects");

    assert!(out.join("metadata.json").is_file(), "a package was left at --out");
    // 2231 rows / 512 per group = 5 groups: the rg512 recipe reached the writer.
    assert_eq!(row_groups_in(&out), 5);
}

#[test]
fn a_write_child_rejects_a_bad_variant_with_the_grammar_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("pkg");
    let output = Command::new(env!("CARGO_BIN_EXE_cityparquet-readbench"))
        .args([
            "--child", "--write",
            "--variant", "cityparquet+gzip6",
            "--input", fixture("delft.city.jsonl").to_str().unwrap(),
            "--out", out.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("cityparquet+gzip6"), "{stderr}");
    assert!(stderr.contains("<preset>[+hilbert][+rg<N>][+<codec>[<level>]]"), "{stderr}");
    assert!(!out.exists(), "a rejected write must leave nothing behind");
}

#[test]
fn a_write_child_needs_all_three_of_variant_input_and_out() {
    for missing in ["--variant", "--input", "--out"] {
        let dir = tempfile::tempdir().unwrap();
        let mut args = vec!["--child", "--write"];
        let input = fixture("delft.city.jsonl");
        let out = dir.path().join("pkg");
        for (flag, value) in [
            ("--variant", "cityparquet"),
            ("--input", input.to_str().unwrap()),
            ("--out", out.to_str().unwrap()),
        ] {
            if flag != missing {
                args.push(flag);
                args.push(value);
            }
        }
        let output = Command::new(env!("CARGO_BIN_EXE_cityparquet-readbench"))
            .args(&args)
            .output()
            .unwrap();
        assert!(!output.status.success(), "{missing} omitted must fail");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(missing), "the message names the missing flag: {stderr}");
    }
}
```

Add `parquet` to `[dev-dependencies]` only if it is not already a regular dependency (it is: `parquet = { version = "58", ... }` in `[dependencies]`, so nothing to add).

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test --manifest-path benchmark/readbench/Cargo.toml --test write_child`
Expected: FAIL, clap rejects `--write` as an unknown argument.

- [ ] **Step 3: Implement the write child**

In `main.rs`, add to `struct Cli` after `child: bool`:

```rust
    /// With `--child`: measure one CONVERSION instead of one read. Needs
    /// `--variant`, `--input` (a CityJSONSeq artefact) and `--out` (a
    /// directory that does not exist yet). Prints the same four-field line
    /// a read child prints, with the conversion's object count last.
    #[arg(long)]
    write: bool,

    /// With `--child --write`: the variant id whose recipe to convert with
    /// (`cityparquet::variant`'s grammar).
    #[arg(long)]
    variant: Option<String>,

    /// With `--child --write`: where the package is written.
    #[arg(long)]
    out: Option<PathBuf>,
```

In `run`, directly after the `if !cli.child { bail!(...) }` block:

```rust
    if cli.write {
        return run_write_child(cli);
    }
```

And the function, placed after `run`:

```rust
/// One timed conversion, in a process of its own so its peak RSS is its own.
///
/// `ConvertOptions` is filled the way the CLI's `convert` fills it
/// (`generate_lod0: true`, the default batch size), so a variant package has
/// the same content as the prepare script's `<base>.parquet` and differs from
/// it only in the recipe under test. A library-default `ConvertOptions::new`
/// would leave LoD0 generation OFF and the row counts would not line up.
fn run_write_child(cli: Cli) -> Result<()> {
    let id = cli.variant.context("--write requires --variant")?;
    let input = cli.input.context("--write requires --input")?;
    let out = cli.out.context("--write requires --out")?;
    let variant = cityparquet::variant::Variant::parse(&id).map_err(|e| anyhow::anyhow!("{e}"))?;
    if out.exists() {
        bail!("--out {} exists; the write child needs a fresh directory", out.display());
    }

    let mut opts = cityparquet::package::ConvertOptions::new(input, out);
    opts.recipe = variant.recipe();
    opts.ordering = variant.ordering();
    opts.generate_lod0 = true;

    alloc::reset();
    let start = Instant::now();
    let report = cityparquet::package::convert(&opts)
        .with_context(|| format!("converting with variant '{id}'"))?;
    let time_s = start.elapsed().as_secs_f64();
    let peak_heap_bytes = alloc::peak_heap_bytes();
    let ru_maxrss_bytes = max_rss_bytes()?;
    println!("{time_s:.6} {peak_heap_bytes} {ru_maxrss_bytes} {}", report.object_count);
    Ok(())
}
```

Check `alloc::reset` exists (it is called on the read path at `alloc::reset();`). If `convert` leaves a partial directory on error, the rejection test above still holds because parsing fails before `convert` runs.

- [ ] **Step 4: Run the test and the harness gate**

Run:
```bash
cargo test --manifest-path benchmark/readbench/Cargo.toml --test write_child
cargo clippy --manifest-path benchmark/readbench/Cargo.toml --all-targets -- -D warnings
cargo fmt --manifest-path benchmark/readbench/Cargo.toml --check
```
Expected: 3 tests pass, clippy and fmt clean.

- [ ] **Step 5: Commit**

```bash
git add benchmark/readbench/src/main.rs benchmark/readbench/tests/write_child.rs
git commit -m "feat(readbench): a --child --write path that times one conversion in its own process

Claude-Session: https://claude.ai/code/session_012rPUqbWax2xKR4GuT8Bmtq"
```

---

### Task 4: the coordinator's `--variants` run

**Files:**
- Modify: `benchmark/readbench/src/main.rs` (`RunArgs` and the `RunOptions` construction)
- Modify: `benchmark/readbench/src/coordinator.rs`
- Create: `benchmark/readbench/tests/variants.rs`

**Interfaces:**
- Consumes: the write child from Task 3; `cityparquet::variant::Variant`.
- Produces:
  - `RunOptions { variants: Option<Vec<String>>, write_repeat: usize, .. }`
  - `pub enum Measure { Write, Read(Scenario) }` with `Display` (`write` / the scenario's `as_str`).
  - `Row { label: String, measure: Measure, .. }` replacing `Row.scenario`; `format: Format` stays.
  - `run_measurement(.., label: &str, ..)`.
  - `sizes.csv` beside `--out`, columns `dataset,format,bytes,mb,ratio_vs_cityjsonseq,baseline_format,ratio_vs_baseline`, `dataset` = the base name (the stem), `format` = the variant id.
  - Packages at `<prepared_dir>/<base>.<id>.parquet`.

- [ ] **Step 1: Write the failing tests**

```rust
//! `run --variants`: the configuration-axis run. Per variant, one timed write
//! in a child of its own, the package kept under
//! `<prepared_dir>/<base>.<variant>.parquet`, then the ordinary read
//! children against it. The CSV shape is the read run's, with a `write` row
//! per variant and the variant id in the `format` column.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use cityparquet::package::{ConvertOptions, convert};

const HEADER: &str = "dataset,format,scenario,selectivity,result_count,time_s,time_mad_s,\
peak_heap_bytes,peak_rss_bytes,repeat,notes,bytes_read,http_requests";

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../lib/cityparquet-rs/tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

/// A prepared dir the way `readbench_prepare.sh` leaves it for a
/// CityJSONSeq input: `<base>.parquet` (the package the query parameters
/// derive from) and `<base>.city.jsonl`.
fn prepared_delft() -> (tempfile::TempDir, PathBuf) {
    let prepared = tempfile::tempdir().unwrap();
    let input = fixture("delft.city.jsonl");
    let mut opts = ConvertOptions::new(input.clone(), prepared.path().join("delft.parquet"));
    opts.generate_lod0 = true;
    convert(&opts).unwrap();
    std::fs::copy(&input, prepared.path().join("delft.city.jsonl")).unwrap();
    (prepared, input)
}

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cityparquet-readbench"))
        .arg("run")
        .args(args)
        .output()
        .expect("failed to run the built cityparquet-readbench binary")
}

fn field<'a>(row: &'a str, i: usize) -> &'a str {
    row.split(',').nth(i).unwrap()
}

#[test]
fn a_variants_run_writes_reads_keeps_the_packages_and_records_sizes() {
    let (prepared, input) = prepared_delft();
    let out_csv = prepared.path().join("out.csv");
    let output = run(&[
        "--input", input.to_str().unwrap(),
        "--prepared-dir", prepared.path().to_str().unwrap(),
        "--out", out_csv.to_str().unwrap(),
        "--repeat", "1",
        "--write-repeat", "1",
        "--scenarios", "full-read,bbox",
        "--variants", "cityparquet,cityparquet+rg512,cityparquet+zstd1",
    ]);
    assert!(output.status.success(), "stderr:\n{}", String::from_utf8_lossy(&output.stderr));

    let text = std::fs::read_to_string(&out_csv).unwrap();
    let mut lines = text.lines();
    assert_eq!(lines.next().unwrap(), HEADER);
    let rows: Vec<&str> = lines.collect();

    // Grouped per variant, write row first, then the reads in scenario order.
    let expected_order = [
        ("cityparquet", "write"), ("cityparquet", "full-read"),
        ("cityparquet", "bbox-query"), ("cityparquet", "bbox-query"), ("cityparquet", "bbox-query"),
        ("cityparquet+rg512", "write"), ("cityparquet+rg512", "full-read"),
        ("cityparquet+rg512", "bbox-query"), ("cityparquet+rg512", "bbox-query"), ("cityparquet+rg512", "bbox-query"),
        ("cityparquet+zstd1", "write"), ("cityparquet+zstd1", "full-read"),
        ("cityparquet+zstd1", "bbox-query"), ("cityparquet+zstd1", "bbox-query"), ("cityparquet+zstd1", "bbox-query"),
    ];
    assert_eq!(rows.len(), expected_order.len(), "rows:\n{text}");
    for (row, (label, scenario)) in rows.iter().zip(expected_order) {
        assert_eq!(field(row, 0), "delft.city.jsonl");
        assert_eq!(field(row, 1), label, "row: {row}");
        assert_eq!(field(row, 2), scenario, "row: {row}");
    }

    for row in rows.iter().filter(|r| field(r, 2) == "write") {
        assert_eq!(field(row, 3), "", "a write row has no selectivity: {row}");
        assert_eq!(field(row, 4), "2231", "result_count is the object count: {row}");
        assert!(field(row, 5).parse::<f64>().unwrap() > 0.0);
        assert!(field(row, 8).parse::<u64>().unwrap() > 0, "peak_rss_bytes: {row}");
        assert_eq!(field(row, 9), "1", "repeat is --write-repeat: {row}");
        assert_eq!(field(row, 10), "");
        assert_eq!(field(row, 11), "");
        assert_eq!(field(row, 12), "");
    }
    let full_reads: Vec<&&str> = rows.iter().filter(|r| field(r, 2) == "full-read").collect();
    assert!(full_reads.iter().all(|r| field(r, 4) == "2231"));

    for id in ["cityparquet", "cityparquet+rg512", "cityparquet+zstd1"] {
        let pkg = prepared.path().join(format!("delft.{id}.parquet"));
        assert!(pkg.join("metadata.json").is_file(), "package kept at {}", pkg.display());
    }
    assert!(prepared.path().join("delft.parquet").is_dir(), "the prepare script's package is untouched");
    let leftovers: Vec<String> = std::fs::read_dir(prepared.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with('.'))
        .collect();
    assert!(leftovers.is_empty(), "repeat directories were not cleaned up: {leftovers:?}");

    let sizes = std::fs::read_to_string(prepared.path().join("sizes.csv")).unwrap();
    let mut sizes = sizes.lines();
    assert_eq!(
        sizes.next().unwrap(),
        "dataset,format,bytes,mb,ratio_vs_cityjsonseq,baseline_format,ratio_vs_baseline"
    );
    let size_rows: Vec<&str> = sizes.collect();
    assert_eq!(size_rows.len(), 3);
    for (row, id) in size_rows.iter().zip(["cityparquet", "cityparquet+rg512", "cityparquet+zstd1"]) {
        assert_eq!(field(row, 0), "delft");
        assert_eq!(field(row, 1), id);
        assert!(field(row, 2).parse::<u64>().unwrap() > 0);
        assert!(field(row, 4).parse::<f64>().unwrap() > 0.0, "ratio_vs_cityjsonseq: {row}");
        assert_eq!(field(row, 5), "cityparquet");
    }
    assert_eq!(field(size_rows[0], 6), "1.000000", "the baseline is 1x against itself");
    assert!(prepared.path().join("out.csv.params.json").is_file());
}

#[test]
fn a_rerun_replaces_its_own_sizes_rows_instead_of_appending() {
    let (prepared, input) = prepared_delft();
    let out_csv = prepared.path().join("out.csv");
    let args = [
        "--input", input.to_str().unwrap(),
        "--prepared-dir", prepared.path().to_str().unwrap(),
        "--out", out_csv.to_str().unwrap(),
        "--repeat", "1", "--write-repeat", "1",
        "--scenarios", "full-read",
        "--variants", "cityparquet,cityparquet+rg512",
    ];
    assert!(run(&args).status.success());
    assert!(run(&args).status.success());
    let sizes = std::fs::read_to_string(prepared.path().join("sizes.csv")).unwrap();
    assert_eq!(sizes.lines().count(), 3, "header + two rows, not four:\n{sizes}");
}

fn expect_rejection(args: &[&str], needle: &str) {
    let output = run(args);
    assert!(!output.status.success(), "must be rejected: {args:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(needle), "expected {needle:?} in:\n{stderr}");
}

#[test]
fn variants_and_formats_are_exclusive_and_the_list_is_validated() {
    let (prepared, input) = prepared_delft();
    let out = prepared.path().join("out.csv");
    let base: Vec<&str> = vec![
        "--input", input.to_str().unwrap(),
        "--prepared-dir", prepared.path().to_str().unwrap(),
        "--out", out.to_str().unwrap(),
        "--repeat", "1", "--write-repeat", "1", "--scenarios", "count",
    ];
    let with = |extra: &[&str]| -> Vec<&str> { base.iter().copied().chain(extra.iter().copied()).collect() };

    expect_rejection(&with(&["--variants", "cityparquet", "--formats", "cityjsonseq"]), "exclusive");
    expect_rejection(&with(&["--variants", "cityparquet+rg512"]), "baseline");
    expect_rejection(&with(&["--variants", "cityparquet,cityparquet+rg512,cityparquet+rg512"]), "duplicate");
    expect_rejection(&with(&["--variants", "cityparquet,cityparquet+gzip6"]), "only zstd takes a level");
    expect_rejection(&with(&["--variants", "cityparquet", "--write-repeat", "0"]), "--write-repeat");
    expect_rejection(&with(&["--variants", "cityparquet", "--scenarios", "write"]), "unknown scenario 'write'");
    expect_rejection(
        &with(&["--variants", "cityparquet", "--transport", "http", "--base-url", "http://localhost:1"]),
        "server-side write",
    );
    assert!(!out.exists(), "a rejected run writes no CSV");
    let _ = Path::new("unused");
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test --manifest-path benchmark/readbench/Cargo.toml --test variants`
Expected: FAIL, clap rejects `--variants` / `--write-repeat`.

- [ ] **Step 3: Implement**

**`main.rs`, `RunArgs`:** add after `formats`:

```rust
    /// Comma-separated variant ids (`cityparquet::variant`'s grammar). A
    /// CONFIGURATION run: every id is written with its recipe by a write
    /// child, kept as `<prepared-dir>/<base>.<id>.parquet`, then read by the
    /// CityParquet runner. Exclusive with `--formats`; the list must contain
    /// the bare `cityparquet` baseline; local transport only.
    #[arg(long, value_delimiter = ',')]
    variants: Option<Vec<String>>,

    /// Warm write repeats per variant (a discarded warmup precedes them).
    /// Only read by `--variants`. Must be >= 1.
    #[arg(long, default_value_t = 3)]
    write_repeat: usize,
```

and thread both into `coordinator::RunOptions { variants: run_args.variants, write_repeat: run_args.write_repeat, .. }`.

**`coordinator.rs`:**

1. `RunOptions` gains `pub variants: Option<Vec<String>>` and `pub write_repeat: usize`.

2. New types, next to `Row`:

```rust
/// What one CSV row measured: a conversion or one read scenario. `write`
/// never enters `Scenario::ALL`, so `--scenarios write` is rejected by the
/// scenario parser and a write row can only come from the `--variants` path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Measure {
    Write,
    Read(Scenario),
}

impl std::fmt::Display for Measure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Measure::Write => f.write_str("write"),
            Measure::Read(s) => f.write_str(s.as_str()),
        }
    }
}
```

`Row` becomes:

```rust
struct Row {
    dataset: String,
    /// What the `format` column prints: `format.as_str()` on a format run,
    /// the variant id on a `--variants` run.
    label: String,
    format: Format,
    measure: Measure,
    selectivity: Option<f64>,
    result_count: u64,
    time_s: f64,
    time_mad_s: f64,
    peak_heap_bytes: u64,
    peak_rss_bytes: u64,
    repeat: usize,
    notes: Vec<String>,
    io: Option<IoStats>,
}
```

`render` prints `self.label` where it printed `format` and `self.measure` where it printed `scenario`. `tag_attr_filter_mismatch` matches `Measure::Read(Scenario::AttrFilter)`. The `cold` row and the unit tests at the bottom set `label: format.as_str().to_string()` and `measure: Measure::Read(..)`.

3. `run_measurement` gains `label: &str` right after `format: Format`; sets `label: label.to_string()`, `measure: Measure::Read(scenario)`. Every existing call passes `format.as_str()`.

4. Validation, at the top of `run` after the `--transport http` check:

```rust
    let variants = match (&opts.formats, &opts.variants) {
        (Some(f), Some(v)) if !f.is_empty() && !v.is_empty() => bail!(
            "--formats and --variants are exclusive: a CSV is either a format comparison or a \
             configuration run, never both"
        ),
        (_, Some(v)) if !v.is_empty() => Some(parse_variant_list(v)?),
        _ => None,
    };
    if variants.is_some() && opts.transport == Transport::Http {
        bail!("--variants runs on --transport local only: there is no server-side write");
    }
    if variants.is_some() && opts.write_repeat == 0 {
        bail!("--write-repeat must be >= 1");
    }
```

```rust
/// The variant list, parsed, de-duplicated by canonical id, and required to
/// carry the bare `cityparquet` baseline every ratio is taken against.
fn parse_variant_list(ids: &[String]) -> Result<Vec<(String, Variant)>> {
    let mut seen: Vec<String> = Vec::new();
    let mut out = Vec::with_capacity(ids.len());
    for raw in ids {
        let variant = Variant::parse(raw).map_err(|e| anyhow::anyhow!("--variants: {e}"))?;
        let id = variant.id();
        if seen.contains(&id) {
            bail!("--variants: duplicate variant '{id}'");
        }
        seen.push(id.clone());
        out.push((id, variant));
    }
    if !seen.iter().any(|id| id == VARIANT_BASELINE) {
        bail!(
            "--variants must include the bare '{VARIANT_BASELINE}' baseline: every ratio on a \
             configuration axis is taken against it"
        );
    }
    Ok(out)
}

const VARIANT_BASELINE: &str = "cityparquet";
```

with `use cityparquet::variant::Variant;` at the top.

5. Resolving what to measure. Replace `resolved_formats: Vec<(Format, Source)>` with `Vec<(Format, Source, String)>` (the label). In the existing branch, push `(format, source, format.as_str().to_string())` and keep the skip/warning logic as is. In the variants branch, skip that block entirely and instead, after the params sidecar and the CSV header are written (so that a failing write still leaves the header and sidecar):

```rust
    let mut rows: Vec<Row> = Vec::new();
    let mut sizes: Vec<SizeRow> = Vec::new();
    let resolved_formats: Vec<(Format, Source, String)> = match &variants {
        None => resolved_formats,
        Some(list) => {
            let seq = seq_path.clone().ok_or_else(|| anyhow::anyhow!(
                "--variants needs the prepared CityJSONSeq artefact {}.city.jsonl to convert \
                 from (run `just readbench-prepare {}` first)",
                base, opts.input.display()
            ))?;
            let mut out = Vec::with_capacity(list.len());
            for (id, _) in list {
                let package = run_write(
                    &mut rows, &dataset, base, id, &opts.prepared_dir, &seq, opts.write_repeat,
                )?;
                sizes.push(SizeRow {
                    dataset: base.to_string(),
                    label: id.clone(),
                    bytes: dir_bytes(&package)?,
                });
                out.push((Format::CityParquet, Source::Local(package), id.clone()));
            }
            out
        }
    };
```

(`rows` is currently declared just before the format loop; move that declaration up so `run_write` can push into it.) The format loop then reads `for (format, source, label) in &resolved_formats` and passes `label` to every `run_measurement`; `attr_filter_counts` keeps keying by `Format` (on a variants run every entry is `Format::CityParquet`, so the consistency check compares the variants against each other, which is the right question).

6. The write measurement:

```rust
/// One variant's write: a discarded warmup and `write_repeat` warm repeats,
/// each a child process converting into a fresh directory INSIDE
/// `prepared_dir` (a rename across filesystems would fail, and a 1M-object
/// package does not belong in /tmp). The last repeat's package is kept as
/// `<prepared_dir>/<base>.<id>.parquet`; returns that path.
fn run_write(
    rows: &mut Vec<Row>,
    dataset: &str,
    base: &str,
    id: &str,
    prepared_dir: &Path,
    seq: &Path,
    write_repeat: usize,
) -> Result<PathBuf> {
    let self_exe = std::env::current_exe().context("cannot determine own executable path")?;
    let mut times = Vec::with_capacity(write_repeat);
    let mut peak_heap_max = 0u64;
    let mut peak_rss_max = 0u64;
    let mut object_count: Option<u64> = None;
    let mut kept: Option<tempfile::TempDir> = None;

    for i in 0..=write_repeat {
        let scratch = tempfile::Builder::new()
            .prefix(&format!(".{base}.{id}.repeat."))
            .tempdir_in(prepared_dir)
            .with_context(|| format!("creating a scratch directory in {}", prepared_dir.display()))?;
        let out = scratch.path().join("pkg");
        let output = Command::new(&self_exe)
            .arg("--child").arg("--write")
            .arg("--variant").arg(id)
            .arg("--input").arg(seq)
            .arg("--out").arg(&out)
            .output()
            .with_context(|| format!("spawning the write child (variant={id})"))?;
        if !output.status.success() {
            bail!(
                "write child failed (variant={id}); stderr:\n{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let stdout = String::from_utf8(output.stdout).context("write child stdout was not valid UTF-8")?;
        let fields: Vec<&str> = stdout.split_whitespace().collect();
        if fields.len() != 4 {
            bail!("expected 4 fields from the write child, got {} in '{}'", fields.len(), stdout.trim());
        }
        let time_s: f64 = fields[0].parse().with_context(|| format!("parsing time_s from '{}'", fields[0]))?;
        let heap: u64 = fields[1].parse().with_context(|| format!("parsing peak_heap_bytes from '{}'", fields[1]))?;
        let rss: u64 = fields[2].parse().with_context(|| format!("parsing ru_maxrss_bytes from '{}'", fields[2]))?;
        let count: u64 = fields[3].parse().with_context(|| format!("parsing object_count from '{}'", fields[3]))?;
        if i == 0 {
            continue; // warmup: the directory drops here and is deleted
        }
        times.push(time_s);
        peak_heap_max = peak_heap_max.max(heap);
        peak_rss_max = peak_rss_max.max(rss);
        object_count = Some(count);
        kept = Some(scratch); // an earlier warm repeat's TempDir drops and is deleted
    }

    let kept = kept.expect("write_repeat >= 1 guarantees a kept repeat");
    let target = prepared_dir.join(format!("{base}.{id}.parquet"));
    if target.exists() {
        fs::remove_dir_all(&target).with_context(|| format!("removing the previous {}", target.display()))?;
    }
    let scratch = kept.keep();
    fs::rename(scratch.join("pkg"), &target)
        .with_context(|| format!("moving the kept package to {}", target.display()))?;
    fs::remove_dir(&scratch).with_context(|| format!("removing {}", scratch.display()))?;

    let time_s = median(&times);
    rows.push(Row {
        dataset: dataset.to_string(),
        label: id.to_string(),
        format: Format::CityParquet,
        measure: Measure::Write,
        selectivity: None,
        result_count: object_count.expect("at least one warm repeat"),
        time_s,
        time_mad_s: mad(&times, time_s),
        peak_heap_bytes: peak_heap_max,
        peak_rss_bytes: peak_rss_max,
        repeat: write_repeat,
        notes: Vec::new(),
        io: None,
    });
    Ok(target)
}
```

`tempfile` 3.27 has `TempDir::keep(self) -> PathBuf`. `Command` needs `use std::process::Command;` (already imported for `spawn_child`).

7. Sizes:

```rust
struct SizeRow {
    dataset: String,
    label: String,
    bytes: u64,
}

/// Bytes of every regular file directly inside a package directory — the
/// rule `cityparquet bench` uses for `total_bytes`.
fn dir_bytes(dir: &Path) -> Result<u64> {
    let mut total = 0u64;
    for entry in fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let meta = entry?.metadata()?;
        if meta.is_file() {
            total += meta.len();
        }
    }
    Ok(total)
}

const SIZES_HEADER: &str =
    "dataset,format,bytes,mb,ratio_vs_cityjsonseq,baseline_format,ratio_vs_baseline";

/// `sizes.csv` beside `out`, in the read run's columns. Rows for THIS dataset
/// are replaced; other datasets' rows are kept, so one recipe walking many
/// slices builds up one file and a re-run of one slice never duplicates.
fn write_sizes(out: &Path, base: &str, seq: &Path, sizes: &[SizeRow]) -> Result<()> {
    let path = out.parent().unwrap_or_else(|| Path::new(".")).join("sizes.csv");
    let mut kept: Vec<String> = Vec::new();
    if path.exists() {
        let existing = fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        let mut lines = existing.lines();
        match lines.next() {
            Some(header) if header == SIZES_HEADER => {}
            Some(header) => bail!(
                "{} has a foreign header; expected `{SIZES_HEADER}`, found `{header}`",
                path.display()
            ),
            None => {}
        }
        kept.extend(
            lines
                .filter(|l| l.split(',').next() != Some(base))
                .map(str::to_string),
        );
    }
    let seq_bytes = fs::metadata(seq).with_context(|| format!("stat {}", seq.display()))?.len() as f64;
    let baseline_bytes = sizes
        .iter()
        .find(|s| s.label == VARIANT_BASELINE)
        .map(|s| s.bytes as f64)
        .expect("parse_variant_list guarantees the baseline");
    for s in sizes {
        let bytes = s.bytes as f64;
        kept.push(format!(
            "{},{},{},{:.6},{:.6},{VARIANT_BASELINE},{:.6}",
            s.dataset,
            s.label,
            s.bytes,
            bytes / (1024.0 * 1024.0),
            seq_bytes / bytes,
            baseline_bytes / bytes,
        ));
    }
    let mut text = String::from(SIZES_HEADER);
    text.push('\n');
    for line in kept {
        text.push_str(&line);
        text.push('\n');
    }
    fs::write(&path, text).with_context(|| format!("writing {}", path.display()))
}
```

Call `write_sizes(&opts.out, base, &seq, &sizes)?` after the read loop, inside `if variants.is_some()` (the `seq` binding from step 5 must be in scope; hoist it as `let seq_for_sizes = ...` or restructure so the variants block returns `(resolved, seq)`).

8. Row order in the CSV. Before the final `for row in &rows { writeln!(...) }`, add:

```rust
    // A configuration run groups its rows per variant, write first, so a CSV
    // reads top to bottom the way the recipe listed the variants.
    if let Some(list) = &variants {
        let position = |label: &str| list.iter().position(|(id, _)| id == label).unwrap_or(usize::MAX);
        rows.sort_by_key(|r| (position(&r.label), r.measure != Measure::Write));
    }
```

`sort_by_key` is stable, so within a variant the read rows keep scenario order.

- [ ] **Step 4: Run the tests and the harness gate**

Run:
```bash
cargo test --manifest-path benchmark/readbench/Cargo.toml --test variants --test coordinator --test write_child --test transport_cli
cargo clippy --manifest-path benchmark/readbench/Cargo.toml --all-targets -- -D warnings
cargo fmt --manifest-path benchmark/readbench/Cargo.toml --check
cargo test --manifest-path benchmark/readbench/Cargo.toml --no-fail-fast 2>&1 | grep -E "^test result|FAILED"
```
Expected: the four named test binaries green; full run green except the two `flatcitybuf_*` binaries.

- [ ] **Step 5: Commit**

```bash
git add benchmark/readbench
git commit -m "feat(readbench): run --variants measures a writer configuration axis with the read harness

Claude-Session: https://claude.ai/code/session_012rPUqbWax2xKR4GuT8Bmtq"
```

---

### Task 5: measured plot fixtures from the delft run

**Files:**
- Create: `benchmark/plot/tests/fixtures/benchviz/scaling_codec_results/delft.csv`, `.../sizes.csv`, `.../delft.csv.params.json`
- Create: `benchmark/plot/tests/fixtures/benchviz/scaling_rowgroup_results/delft.csv`, `.../sizes.csv`, `.../delft.csv.params.json`
- Delete: `benchmark/plot/tests/fixtures/benchviz/compression_results/`
- Modify: `benchmark/plot/tests/fixtures/benchviz/README.md`

**Interfaces:**
- Consumes: the coordinator from Task 4, the delft fixture.
- Produces: fixture directories in exactly the layout `prep.load_scaling_axis` (Task 7) reads. Nothing hand-written.

- [ ] **Step 1: Produce the two runs**

```bash
cd /data2/hideba/cityparquet-paper/cityparquet-bench
P=$(mktemp -d)
cargo run --release --manifest-path lib/cityparquet-rs/Cargo.toml -p cityparquet-cli --bin cityparquet -- \
  convert lib/cityparquet-rs/tests/fixtures/delft.city.jsonl -o "$P/delft.parquet"
cp lib/cityparquet-rs/tests/fixtures/delft.city.jsonl "$P/delft.city.jsonl"
F=benchmark/plot/tests/fixtures/benchviz
mkdir -p $F/scaling_codec_results $F/scaling_rowgroup_results
cargo run --release --manifest-path benchmark/readbench/Cargo.toml -- run \
  --input "$P/delft.city.jsonl" --prepared-dir "$P" --out $F/scaling_codec_results/delft.csv \
  --repeat 2 --write-repeat 2 --scenarios full-read,bbox-query \
  --variants cityparquet,cityparquet+zstd1,cityparquet+lz4
cargo run --release --manifest-path benchmark/readbench/Cargo.toml -- run \
  --input "$P/delft.city.jsonl" --prepared-dir "$P" --out $F/scaling_rowgroup_results/delft.csv \
  --repeat 2 --write-repeat 2 --scenarios full-read,bbox-query \
  --variants cityparquet,cityparquet+rg512,cityparquet+rg2048
git rm -r $F/compression_results
```

Confirm each directory holds `delft.csv`, `delft.csv.params.json`, `sizes.csv`, and that `delft.csv` has 15 data rows (3 variants × (write + full-read + 3 windows)).

- [ ] **Step 2: Record the provenance in the fixture README**

Replace the `- **Railway** (compression only) — a header-only CSV.` bullet and the Ingolstadt bullet's compression clause, and add after the `ordering_results/` paragraph:

```markdown
`scaling_codec_results/` and `scaling_rowgroup_results/` each hold one
`--variants` run of the coordinator over the `delft.city.jsonl` fixture
(`--repeat 2 --write-repeat 2 --scenarios full-read,bbox-query`), produced
by the command in `ai/design-notes/plans/2026-09-05-configuration-axes-benchmark.md`,
Task 5. Three variants each (`cityparquet` plus `+zstd1`/`+lz4`, and
`cityparquet` plus `+rg512`/`+rg2048`), so the loader's baseline, ratio
direction, variant order and `sizes.csv` join are all exercised on measured
rows. One slice only: the trend strip is drawn from one point, which is a
valid degenerate case, and nothing here is edited by hand.
```

- [ ] **Step 3: Commit**

```bash
git add benchmark/plot/tests/fixtures
git commit -m "test(plot): measured codec and row-group fixtures from a delft --variants run

Claude-Session: https://claude.ai/code/session_012rPUqbWax2xKR4GuT8Bmtq"
```

(The Python suite is red between this commit and Task 7's; that is expected and Task 7 makes it green.)

---

### Task 6: recipes, machine record, recipe tests, old benchmark removed

**Files:**
- Modify: `justfile` (header comment line 19, comment at 189-191, the `compression-bench` and `compression-plot` recipes at 493-556, the `plot-pretty` comment)
- Create: `benchmark/scripts/machine_record.sh`
- Modify: `benchmark/scripts/tests/bench_recipe_test.sh`
- Modify: `benchmark/readbench/tests/strip_extension.rs:246`
- Delete: `benchmark/plot/readbench_plot/compression.py`, `benchmark/formats/compression_results/`, `benchmark/formats/scaling_compression_results/`

**Interfaces:**
- Consumes: `cityparquet-readbench run --variants` from Task 4.
- Produces: `just variant-bench FOLDER OUT VARIANTS [PREPARED]`, `just codec-bench FOLDER [OUT] [PREPARED]`, `just rowgroup-bench FOLDER [OUT] [PREPARED]`; `OUT/MACHINE.md`.

- [ ] **Step 1: Write the failing shell test cases** (append to `bench_recipe_test.sh` before its case runner at the bottom; follow the file's `pass`/`fail` helpers)

```bash
# --------------------------------------------------------------------------
# Cases 6-7: the two configuration-axis recipes pass exactly their lists.
#
# `codec-bench` and `rowgroup-bench` are one-line delegations to
# `variant-bench`, and the variant list each passes IS the benchmark: read
# them out of the recipe rather than trusting a comment.
# --------------------------------------------------------------------------
recipe_variants() {
  # The quoted variant list a delegating recipe passes to `variant-bench`.
  sed -n "/^$1 /,/^$/p" "$JUSTFILE" \
    | sed -n 's/.*just variant-bench "{{FOLDER}}" "{{OUT}}" "\([^"]*\)".*/\1/p'
}

case_codec_bench_list() {
  local name="codec-bench passes the codec axis, all at the default row-group size"
  local expected="cityparquet,cityparquet+zstd1,cityparquet+zstd9,cityparquet+zstd19,cityparquet+lz4,cityparquet+snappy,cityparquet+gzip,cityparquet+brotli,cityparquet+uncompressed"
  local actual
  actual="$(recipe_variants codec-bench)"
  if [[ "$actual" != "$expected" ]]; then
    fail "$name" "codec-bench passes '$actual'"
    return
  fi
  if [[ "$actual" == *"+rg"* ]]; then
    fail "$name" "a codec variant carries a row-group suffix"
    return
  fi
  pass "$name"
}

case_rowgroup_bench_list() {
  local name="rowgroup-bench passes the row-group axis, all at the default codec"
  local expected="cityparquet,cityparquet+rg32768,cityparquet+rg8192,cityparquet+rg2048,cityparquet+rg512"
  local actual
  actual="$(recipe_variants rowgroup-bench)"
  if [[ "$actual" != "$expected" ]]; then
    fail "$name" "rowgroup-bench passes '$actual'"
    return
  fi
  local v
  for v in uncompressed snappy gzip lz4 brotli zstd; do
    if [[ "$actual" == *"+$v"* ]]; then
      fail "$name" "a row-group variant carries a codec suffix ($v)"
      return
    fi
  done
  pass "$name"
}
```

and register `case_codec_bench_list` and `case_rowgroup_bench_list` wherever the file invokes its cases (look for the block that calls `case_bare_run_omits_the_baseline` and add the two after the last call).

- [ ] **Step 2: Run the shell suite to verify the new cases fail**

Run: `./benchmark/scripts/tests/bench_recipe_test.sh`
Expected: the two new cases fail (`codec-bench passes ''`).

- [ ] **Step 3: Write the recipes**

Replace the whole `compression-bench` block (its comment from `# Compression-codec + row-group WRITE-bench recipe` through the recipe body) and the `compression-plot` block with:

```just
# The configuration-axis runner behind `codec-bench` and `rowgroup-bench`:
# for every CityJSON/CityJSONSeq file under FOLDER (recursive), build the
# `cityparquet` artefact the query parameters derive from (and the
# CityJSONSeq the writes convert from), then run the coordinator's
# `--variants` path: per variant a timed write in a child process (peak RSS,
# median of 3 warm repeats after a warmup), the package kept as
# `PREPARED/<name>.<variant>.parquet`, then `full-read` and the three bbox
# windows against it. One OUT/<name>.csv per input in the read run's exact
# CSV shape (a `write` row per variant, the variant id in the `format`
# column), package bytes in OUT/sizes.csv, and the host in OUT/MACHINE.md.
# Each OUT/<name>.csv is removed first; OUT/sizes.csv is removed once at the
# start, and each input's run then appends its own rows. Local transport
# only. Network-independent given already-fetched inputs; multi-hour at the
# 1M-object slice; kept OUT of `just check`/CI.
#
# VARIANTS is the whole benchmark: the two public recipes below pass their
# lists here and nowhere else, and benchmark/scripts/tests/bench_recipe_test.sh
# reads those lists back out of this file.
[doc("Configuration-axis run: timed writes + two reads per variant, over every input under FOLDER")]
variant-bench FOLDER OUT VARIANTS PREPARED=(BENCH / "data/readbench"):
    #!/usr/bin/env bash
    set -euo pipefail
    mkdir -p "{{OUT}}" "{{PREPARED}}"
    rm -f "{{OUT}}/sizes.csv"
    found=0
    while IFS= read -r -d '' f; do
        name="$(basename "$f")"
        for ext in {{KNOWN_INPUT_EXTENSIONS}}; do
            if [[ "$name" == *"$ext" ]]; then name="${name%"$ext"}"; break; fi
        done
        out="{{OUT}}/${name}.csv"
        echo ">> ${f} -> ${out}"
        rm -f "$out"

        ./{{BENCH_SCRIPTS}}/readbench_prepare.sh --formats cityparquet "$f" "{{PREPARED}}"

        cargo run --release {{READBENCH_CARGO}} -- run \
            --input "$f" \
            --prepared-dir "{{PREPARED}}" \
            --out "$out" \
            --repeat 7 \
            --write-repeat 3 \
            --scenarios full-read,bbox-query \
            --variants "{{VARIANTS}}"

        found=$((found + 1))
    done < <(find "{{FOLDER}}" -type f \
        \( {{KNOWN_INPUT_FIND}} \) ! -name 'metadata.json' -print0 \
        | sort -z)
    if [[ "$found" -eq 0 ]]; then
        echo "variant-bench: no city-model inputs found under {{FOLDER}}" >&2
        exit 1
    fi
    ./{{BENCH_SCRIPTS}}/machine_record.sh > "{{OUT}}/MACHINE.md"
    echo "variant-bench: ${found} file(s) benchmarked into {{OUT}}"

# The CODEC axis: which compression codec, and when. zstd — the codec
# CityParquet ships with — is swept at levels 1, 3 (the default and the 1x
# baseline), 9 and 19; the other codecs run at the parquet-rs defaults the
# writer recipe carries (gzip 6, brotli 1) and are reference points, not a
# ranking against each other. Every variant at the default 65536-row groups.
[doc("Codec axis over the scaling slices: zstd 1/3/9/19, lz4, snappy, gzip, brotli, none")]
codec-bench FOLDER OUT=(BENCH / "scaling_codec_results") PREPARED=(BENCH / "data/readbench"):
    just variant-bench "{{FOLDER}}" "{{OUT}}" "cityparquet,cityparquet+zstd1,cityparquet+zstd9,cityparquet+zstd19,cityparquet+lz4,cityparquet+snappy,cityparquet+gzip,cityparquet+brotli,cityparquet+uncompressed" "{{PREPARED}}"

# The ROW-GROUP axis: which group size, and when. The 65536-row default is
# the 1x baseline; every variant at the default codec (zstd 3).
[doc("Row-group axis over the scaling slices: 65536 (default), 32768, 8192, 2048, 512")]
rowgroup-bench FOLDER OUT=(BENCH / "scaling_rowgroup_results") PREPARED=(BENCH / "data/readbench"):
    just variant-bench "{{FOLDER}}" "{{OUT}}" "cityparquet,cityparquet+rg32768,cityparquet+rg8192,cityparquet+rg2048,cityparquet+rg512" "{{PREPARED}}"
```

The `for ext in ... done` block and the `name="$(basename "$f")"` line must be byte-identical to the other three recipes' (`strip_extension.rs` compares them and counts exactly four).

Edit the header comment at line 18-19 to name `variant-bench` in place of `compression-bench`; the comment at 189-191 to say `codec-bench`, `rowgroup-bench`, `ordering-bench`; the `plot-pretty` comment's "what `bench`, `compression-bench` and `sizes` left behind" to "what `bench`, `codec-bench`, `rowgroup-bench` and `sizes` left behind".

Create `benchmark/scripts/machine_record.sh` (`chmod +x`):

```bash
#!/usr/bin/env bash
# The measurement host, for a results directory's MACHINE.md: what
# benchmark/formats/READ_BENCHMARK.md's "Machine" section asks every run to
# capture. Prints Markdown on stdout; the caller redirects it.
set -euo pipefail
echo "# Measurement host"
echo
echo "Captured by benchmark/scripts/machine_record.sh at $(date -u +%Y-%m-%dT%H:%M:%SZ)."
echo
echo '```'
uname -a
if command -v lscpu >/dev/null 2>&1; then lscpu | sed -n '1,15p'; fi
if command -v free >/dev/null 2>&1; then free -b | head -2; fi
if command -v sysctl >/dev/null 2>&1 && [[ "$(uname -s)" == "Darwin" ]]; then
  sysctl -n machdep.cpu.brand_string hw.memsize
fi
rustc --version
cargo --version
echo "git $(git rev-parse HEAD)"
echo '```'
```

Update `strip_extension.rs` line 246's comment to `// convert-all, bench, write-bench, variant-bench.`.

Delete the old artefacts:

```bash
git rm -r benchmark/formats/compression_results benchmark/formats/scaling_compression_results
git rm benchmark/plot/readbench_plot/compression.py
```

Check `benchmark/plot/readbench_plot/__main__.py` and `__init__.py` for an import of `compression`; remove it if present.

- [ ] **Step 4: Run the suites**

Run:
```bash
./benchmark/scripts/tests/bench_recipe_test.sh
cargo test --manifest-path benchmark/readbench/Cargo.toml --test strip_extension
just --list | grep -E "codec-bench|rowgroup-bench|variant-bench"
just plot-test 2>&1 | tail -3
```
Expected: shell cases all pass; `strip_extension` green (still four blocks); the three recipes listed. `plot-test` is red on the compression tests until Task 7; that is expected here.

- [ ] **Step 5: Commit**

```bash
git add justfile benchmark/scripts benchmark/readbench/tests/strip_extension.rs benchmark/plot/readbench_plot
git commit -m "bench: codec-bench and rowgroup-bench on the read harness, replacing compression-bench

Claude-Session: https://claude.ai/code/session_012rPUqbWax2xKR4GuT8Bmtq"
```

---

### Task 7: `prep.py` — the axis loader, the machine record, the meta

**Files:**
- Modify: `benchmark/plot/benchviz/prep.py`
- Modify: `benchmark/plot/tests/test_benchviz.py`
- Modify: `benchmark/plot/tests/test_csv_contract.py` (only if it references `COMPRESSION_COLUMNS`; check with grep)

**Interfaces:**
- Consumes: the fixture directories from Task 5.
- Produces, in `bench_data.json`:
  - `scaling.codec` and `scaling.rowgroup`, each `{"records": [...], "sizes": [...], "gaps": [...], "variants": [...]}`; `variants` is the variant ids in first-seen CSV order, baseline included.
  - record: `{dataset, objects, variant, kind ("default"|"variant"), measure, time_s, rss_b, base_time_s, base_rss_b, time_ratio, rss_ratio, below_floor}`; `measure` in `write | full-read | bbox-1pct | bbox-5pct | bbox-25pct`.
  - size entry: `{dataset, objects, variant, bytes, mb, ratio_vs_cityjsonseq, size_ratio}`.
  - `meta.sources.codec`, `meta.sources.rowgroup`; `meta.machine = {"codec": str|None, "rowgroup": str|None}`; `meta.codec_level_note` (new text); `meta.axis_baseline = "cityparquet"`.
  - Removed: top-level `compression`, `compression_gaps`; `meta.caveats_compression`; `meta.sources.compression`; `scaling.compression`.
  - Module constants `AXIS_BASELINE = "cityparquet"`, `AXIS_MEASURES = ("write", "full-read", "bbox-1pct", "bbox-5pct", "bbox-25pct")`.

- [ ] **Step 1: Write the failing tests** (in `test_benchviz.py`; delete `test_prep_records_the_compression_gaps_rather_than_dropping_them` and `test_a_corpus_with_no_compression_run_is_stated_not_crashed`; in `test_prep_builds_the_design_contract_from_result_csvs` replace `"compression", "compression_gaps",` in the expected key list with nothing and add an assertion `assert set(data["scaling"]) == {"read", "sizes", "ordering", "codec", "rowgroup"}`)

```python
def test_axis_records_are_baselined_against_the_default_variant(tmp_path):
    """Both configuration axes load through one loader, against `cityparquet`.

    Ratios are baseline over variant (above 1x is faster, leaner, smaller), the
    variant order is the CSV's own, and the write row is a measure like any
    other. The fixture is a measured delft run, so every number here is real.
    """
    data, _ = prep.build(prep.Inputs(_bench_dir(tmp_path)))
    for key, expected_variants in (
        ("codec", ["cityparquet", "cityparquet+zstd1", "cityparquet+lz4"]),
        ("rowgroup", ["cityparquet", "cityparquet+rg512", "cityparquet+rg2048"]),
    ):
        axis = data["scaling"][key]
        assert axis["variants"] == expected_variants
        assert axis["gaps"] == []
        by = {(r["variant"], r["measure"]): r for r in axis["records"]}
        assert set(m for _, m in by) == set(prep.AXIS_MEASURES)
        base_write = by[("cityparquet", "write")]
        assert base_write["kind"] == "default"
        assert base_write["objects"] == 2231
        assert base_write["time_ratio"] == 1.0
        assert base_write["rss_ratio"] == 1.0
        variant_write = by[(expected_variants[1], "write")]
        assert variant_write["kind"] == "variant"
        assert variant_write["base_time_s"] == base_write["time_s"]
        assert variant_write["time_ratio"] == pytest.approx(
            base_write["time_s"] / variant_write["time_s"]
        )
        assert isinstance(variant_write["below_floor"], bool)
        assert by[(expected_variants[1], "bbox-5pct")]["dataset"] == "delft"

        sizes = {s["variant"]: s for s in axis["sizes"]}
        assert set(sizes) == set(expected_variants)
        assert sizes["cityparquet"]["size_ratio"] == 1.0
        assert sizes["cityparquet"]["objects"] == 2231
        assert sizes[expected_variants[1]]["size_ratio"] == pytest.approx(
            sizes["cityparquet"]["bytes"] / sizes[expected_variants[1]]["bytes"]
        )
    assert data["meta"]["axis_baseline"] == "cityparquet"
    assert data["meta"]["sources"]["codec"].endswith("scaling_codec_results")
    assert "zstd" in data["meta"]["codec_level_note"]


def test_a_corpus_with_no_axis_run_is_stated_not_crashed(tmp_path):
    """Absence is normal, as for compression before it and ordering beside it."""
    from benchviz import figures

    bench = _bench_dir(tmp_path)
    shutil.rmtree(bench / "scaling_codec_results")
    data, _ = prep.build(prep.Inputs(bench))
    assert data["scaling"]["codec"] == {"records": [], "sizes": [], "gaps": [], "variants": []}
    assert data["scaling"]["rowgroup"]["records"]
    assert data["meta"]["machine"]["codec"] is None

    data_path = tmp_path / "no_codec.json"
    data_path.write_text(prep.json.dumps(data), encoding="utf-8")
    written = sorted(
        p.name for p in figures.main(data_path=data_path, out_dir=tmp_path / "f").glob("*")
    )
    assert "codec.svg" not in written
    assert "rowgroup.svg" in written
    assert "compression.svg" not in written


def test_axis_gaps_are_named_not_dropped(tmp_path):
    bench = _bench_dir(tmp_path)
    axis_dir = bench / "scaling_codec_results"
    header = (axis_dir / "delft.csv").read_text(encoding="utf-8").splitlines()[0]
    (axis_dir / "empty.csv").write_text(header + "\n", encoding="utf-8")
    rows = (axis_dir / "delft.csv").read_text(encoding="utf-8").splitlines()
    kept = [r for r in rows if not r.startswith("delft.city.jsonl,cityparquet,")]
    (axis_dir / "nobase.csv").write_text("\n".join(kept) + "\n", encoding="utf-8")
    (axis_dir / "MACHINE.md").write_text("# Measurement host\n\nfake\n", encoding="utf-8")

    data, _ = prep.build(prep.Inputs(bench))
    gaps = {(g["dataset"], g["issue"]) for g in data["scaling"]["codec"]["gaps"]}
    assert ("empty", "CSV present but header-only") in gaps
    assert ("nobase", "no 'cityparquet' baseline rows") in gaps
    assert data["meta"]["machine"]["codec"].startswith("# Measurement host")
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `just plot-test 2>&1 | tail -15`
Expected: the three new tests fail (`KeyError: 'codec'` or missing `AXIS_MEASURES`), the old compression tests are gone.

- [ ] **Step 3: Implement**

In `prep.py`:

1. `Inputs`: delete `compression_dir` and `scaling_compression_dir`; add

```python
    @property
    def scaling_codec_dir(self) -> Path:
        return self.bench_dir / "scaling_codec_results"

    @property
    def scaling_rowgroup_dir(self) -> Path:
        return self.bench_dir / "scaling_rowgroup_results"
```

2. Constants: delete `COMPRESSION_COLUMNS`; replace `CODEC_LEVEL_NOTE` with

```python
CODEC_LEVEL_NOTE = (
    "The codec axis sweeps zstd, the codec CityParquet ships with, at levels "
    "1, 3 (the default and the 1x baseline), 9 and 19. The other codecs run "
    "at the parquet-rs defaults the writer recipe carries (gzip 6, brotli 1; "
    "crates/core/src/recipe.rs) and are drawn as reference points, not ranked "
    "against each other: no level was matched across codecs."
)
AXIS_BASELINE = "cityparquet"
AXIS_MEASURES = ("write", "full-read", "bbox-1pct", "bbox-5pct", "bbox-25pct")
MACHINE_MD_NAME = "MACHINE.md"
```

3. Delete `compression_caveats`, `_compression_kind`, `load_compression`, and in `load_scaling` the whole `compression_records` block plus the `"compression": compression_records` key (and the `"compression": []` in its early return).

4. Add, after `load_scaling`:

```python
def _measure_key(row: dict[str, str]) -> str:
    return "write" if row["scenario"] == "write" else _scenario_key(row)


def load_scaling_axis(directory: Path, baseline: str = AXIS_BASELINE) -> dict:
    """One configuration axis (codec or row group) from a `--variants` run.

    Every ratio is baseline over variant, so above 1x is faster, leaner or
    smaller — the ordering records' convention, not the read records'. The
    variant order is the CSVs' own first-seen order, because the recipe's
    list is the figure's order and sorting would lose it. Absolute seconds
    and bytes stay: the trend strip plots them.
    """
    records: list[dict] = []
    sizes: list[dict] = []
    gaps: list[dict] = []
    variants: list[str] = []
    objects_by: dict[str, int | None] = {}

    for path in _dataset_csvs(directory):
        name = path.stem
        rows = _read_rows(path, READ_COLUMNS)
        if not rows:
            gaps.append({"dataset": name, "issue": "CSV present but header-only"})
            continue
        by_measure: dict[str, dict[str, dict[str, str]]] = {}
        for row in rows:
            if COLD_RE.search(row["notes"]):
                continue
            by_measure.setdefault(_measure_key(row), {})[row["format"]] = row
            if row["format"] not in variants:
                variants.append(row["format"])
        if not any(baseline in bucket for bucket in by_measure.values()):
            gaps.append({"dataset": name, "issue": f"no '{baseline}' baseline rows"})
            continue
        write_base = by_measure.get("write", {}).get(baseline)
        objects_by[name] = _int(write_base["result_count"]) if write_base else None
        present = {v for bucket in by_measure.values() for v in bucket}
        for key in AXIS_MEASURES:
            bucket = by_measure.get(key)
            if not bucket:
                continue
            base = bucket.get(baseline)
            base_t = _float(base["time_s"]) if base else None
            base_rss = _int(base["peak_rss_bytes"]) if base else None
            for variant in variants:
                row = bucket.get(variant)
                if row is None:
                    if variant in present:
                        gaps.append({"dataset": name, "issue": f"{variant} has no {key} row"})
                    continue
                t = _float(row["time_s"])
                rss = _int(row["peak_rss_bytes"])
                records.append(
                    {
                        "dataset": name,
                        "objects": objects_by[name],
                        "variant": variant,
                        "kind": "default" if variant == baseline else "variant",
                        "measure": key,
                        "time_s": t,
                        "rss_b": rss,
                        "base_time_s": base_t,
                        "base_rss_b": base_rss,
                        "time_ratio": _ratio(base_t, t),
                        "rss_ratio": _ratio(
                            float(base_rss) if base_rss is not None else None,
                            float(rss) if rss is not None else None,
                        ),
                        "below_floor": (
                            None
                            if t is None or base_t is None
                            else abs(base_t - t) < CITATION_FLOOR_S
                        ),
                    }
                )

    sizes_csv = directory / SIZES_CSV_NAME
    if sizes_csv.exists():
        by_ds: dict[str, dict[str, dict[str, str]]] = {}
        for row in _read_rows(sizes_csv, SIZES_COLUMNS):
            by_ds.setdefault(row["dataset"], {})[row["format"]] = row
        measured = {(r["dataset"], r["variant"]) for r in records}
        for ds, fmts in by_ds.items():
            base_b = _int(fmts[baseline]["bytes"]) if baseline in fmts else None
            for fmt in variants:
                row = fmts.get(fmt)
                if row is None:
                    continue
                b = _int(row["bytes"])
                if (ds, fmt) not in measured:
                    gaps.append(
                        {"dataset": ds, "issue": f"sizes.csv row for {fmt} without a measurement"}
                    )
                sizes.append(
                    {
                        "dataset": ds,
                        "objects": objects_by.get(ds),
                        "variant": fmt,
                        "bytes": b,
                        "mb": _float(row["mb"]),
                        "ratio_vs_cityjsonseq": _float(row["ratio_vs_cityjsonseq"]),
                        "size_ratio": _ratio(
                            float(base_b) if base_b is not None else None,
                            float(b) if b is not None else None,
                        ),
                    }
                )

    gaps.sort(key=lambda g: (g["dataset"], g["issue"]))
    return {"records": records, "sizes": sizes, "gaps": gaps, "variants": variants}


def read_machine(directory: Path) -> str | None:
    """The run's MACHINE.md, verbatim, or None when the directory has none."""
    path = directory / MACHINE_MD_NAME
    if not path.is_file():
        return None
    return path.read_text(encoding="utf-8")
```

5. In `build`: delete `compression_records, compression_gaps = load_compression(inputs)` and the `compression_records.sort(...)`; after `scaling = load_scaling(inputs)` add

```python
    scaling["codec"] = load_scaling_axis(inputs.scaling_codec_dir)
    scaling["rowgroup"] = load_scaling_axis(inputs.scaling_rowgroup_dir)
```

In `meta`: replace `"compression": inputs.label(inputs.compression_dir),` with `"codec": inputs.label(inputs.scaling_codec_dir), "rowgroup": inputs.label(inputs.scaling_rowgroup_dir),`; delete `"caveats_compression"`; add `"axis_baseline": AXIS_BASELINE,` and

```python
            "machine": {
                "codec": read_machine(inputs.scaling_codec_dir),
                "rowgroup": read_machine(inputs.scaling_rowgroup_dir),
            },
```

Delete the `"compression"` and `"compression_gaps"` keys from `data`.

6. In `main`'s summary print, replace the compression counts with `f"{len(data['scaling']['codec']['records'])} codec records, {len(data['scaling']['rowgroup']['records'])} row-group records, "`.

7. Module docstring line 4: `just bench / just codec-bench / just rowgroup-bench / just sizes`.

8. `grep -n compression benchmark/plot/benchviz/prep.py` must return nothing except the docstring sentence about the axis, if any.

- [ ] **Step 4: Run the Python suite**

Run: `just plot-test`
Expected: everything green except tests that call `figures.main` and expect `rowgroup.svg` (Task 9) and any html key check (Task 8). If `test_a_corpus_with_no_axis_run_is_stated_not_crashed` is the only red, proceed; it turns green in Task 9.

- [ ] **Step 5: Commit**

```bash
git add benchmark/plot/benchviz/prep.py benchmark/plot/tests
git commit -m "feat(benchviz): load the codec and row-group axes from the read-shaped CSVs

Claude-Session: https://claude.ai/code/session_012rPUqbWax2xKR4GuT8Bmtq"
```

---

### Task 8: `html.py` — remove the compression view, repoint section 3b

**Files:**
- Modify: `benchmark/plot/benchviz/html.py`

**Interfaces:**
- Consumes: `DATA.scaling.codec`, `DATA.scaling.rowgroup`, `META.machine`, `META.codec_level_note` from Task 7.
- Produces: a page that renders from the Task 7 contract with no missing key.

- [ ] **Step 1: Delete the compression view**

- Remove the `<section id="view-comp" ...>...</section>` block (lines ~426-435) and the `<hr class="rule">` that follows it.
- Remove `var COMP = {};`, the `COMP_DATASETS` computation and the `DATA.compression.forEach` that fills `COMP` (~557-562).
- Remove `function renderCompression()` whole (~1752-1915) and its call `renderCompression();` (~2499).
- In the caveats section HTML (~444-447), remove the `<h3>Baseline geometry coverage (compression corpus), verbatim</h3><div id="caveats-comp"></div>` pair; keep `<h3>Codec levels</h3><div class="verbatim" id="codec-note-verbatim"></div>`.
- In `renderCaveats`, remove the `caveats-comp` assignment; keep `codec-note-verbatim`.
- In `renderCoverage`, replace the `DATA.compression_gaps.forEach(...)` loop with

```javascript
    [["codec", DATA.scaling.codec], ["row group", DATA.scaling.rowgroup]].forEach(function (pair) {
      (pair[1].gaps || []).forEach(function (g) {
        items.push("<b>" + esc(g.dataset) + "</b> — " + pair[0] + " axis: " + esc(g.issue) + ".");
      });
    });
```

- In the required-keys list in `main` (~2560-2575), remove `"compression"` and `"compression_gaps"`.

- [ ] **Step 2: Repoint section 3b**

Replace `function renderConfigVariants()` with:

```javascript
  function renderConfigVariants() {
    var axes = [
      ["Codec", DATA.scaling.codec, "just codec-bench"],
      ["Row-group size", DATA.scaling.rowgroup, "just rowgroup-bench"],
    ];
    var html = "";
    axes.forEach(function (axis) {
      var title = axis[0], data = axis[1], recipe = axis[2];
      var recs = data.records || [];
      if (!recs.length) {
        html += "<p class='small'><b>No " + title.toLowerCase() + " run in this corpus.</b> `" +
          recipe + "` over the scaling slices produces it.</p>";
        return;
      }
      var slices = [];
      recs.forEach(function (r) {
        if (!slices.some(function (s) { return s.id === r.dataset; })) {
          slices.push({ id: r.dataset, objects: r.objects });
        }
      });
      slices.sort(function (a, b) { return a.objects - b.objects; });
      var sizes = {};
      (data.sizes || []).forEach(function (s) { sizes[s.dataset + "|" + s.variant] = s; });
      var writes = {};
      recs.forEach(function (r) { if (r.measure === "write") { writes[r.dataset + "|" + r.variant] = r; } });

      var head = '<tr><th scope="col">Variant</th>';
      slices.forEach(function (s) {
        head += '<th scope="col" colspan="2">' + num(s.objects) + " obj</th>";
      });
      head += "</tr><tr><th></th>";
      slices.forEach(function () { head += "<th>bytes</th><th>write</th>"; });
      head += "</tr>";
      var body = "";
      data.variants.forEach(function (v) {
        var row = '<tr><th scope="row">' + esc(v.replace("cityparquet+", "").replace(/^cityparquet$/, "default")) + "</th>";
        slices.forEach(function (s) {
          var sz = sizes[s.id + "|" + v], w = writes[s.id + "|" + v];
          row += "<td>" + (sz ? esc(bytes(sz.bytes)) : "&mdash;") + "</td>";
          row += "<td>" + (w ? esc(secs(w.time_s)) : "&mdash;") + "</td>";
        });
        body += row + "</tr>";
      });
      html += '<div class="tablewrap"><table class="corpus"><caption>' +
        esc(title + ": bytes written and median write time per variant, at each cardinality of " +
            "the same city model. Read time and peak memory per variant are in the print " +
            "figures (codec.svg, rowgroup.svg).") +
        "</caption><thead>" + head + "</thead><tbody>" + body + "</tbody></table></div>";
      if (META.machine && META.machine[title === "Codec" ? "codec" : "rowgroup"]) {
        html += '<details><summary>Measurement host</summary><pre class="verbatim">' +
          esc(META.machine[title === "Codec" ? "codec" : "rowgroup"]) + "</pre></details>";
      }
    });
    el("config-var-table").innerHTML = html;
    el("config-var-lede").textContent =
      "Two write-side axes, each measured by the read harness against its own default at 1x: " +
      "a timed write in a child process (peak RSS), then a full read and the bbox windows on " +
      "the package it left. " + META.codec_level_note;
  }
```

Confirm a `secs` helper exists in the page's script (grep `function secs`); if the page's helper is named differently (`fmtSecs`, `seconds`), use that name.

- [ ] **Step 3: Render the page from the fixture and from the live tree**

Run:
```bash
cd benchmark/plot && T=$(mktemp -d) && mkdir -p $T/benchmark && cp -r tests/fixtures/benchviz $T/benchmark/formats \
  && cp ../formats/READ_BENCHMARK.md ../formats/README.md $T/benchmark/formats/ \
  && uv run python -m benchviz prep --bench-dir $T/benchmark/formats --out $T/out \
  && uv run python -m benchviz html --bench-dir $T/benchmark/formats --out $T/out && grep -c "config-var-table" $T/out/bench-summary.html
grep -n "compression" benchmark/plot/benchviz/html.py
```
Expected: `prep` and `html` exit 0; the grep for `compression` in `html.py` returns nothing (or only a sentence in prose that describes the axis, if any).

- [ ] **Step 4: Run the Python suite**

Run: `just plot-test`
Expected: same state as after Task 7 (only the figure-dependent test red).

- [ ] **Step 5: Commit**

```bash
git add benchmark/plot/benchviz/html.py
git commit -m "feat(benchviz): summary page reads the codec and row-group axes, compression view removed

Claude-Session: https://claude.ai/code/session_012rPUqbWax2xKR4GuT8Bmtq"
```

---

### Task 9: `figures.py` — the axis sheet

**Files:**
- Modify: `benchmark/plot/benchviz/figures.py`
- Modify: `benchmark/plot/tests/test_benchviz.py` (one render test)

**Interfaces:**
- Consumes: `data["scaling"]["codec"]`, `data["scaling"]["rowgroup"]`, `data["meta"]["codec_level_note"]`, `data["meta"]["machine"]`, `data["meta"]["citation_floor_s"]`; helpers `_headline`, `_footer`, `_footer_reserve`, `_panel_heading`, `_ratio_bar`, `_bar_value_label`, `_format_panel_axis`, `_axis_label`, `_secs`, `_speedup_label`, `_times`, `_save`, `_wrap`, `_sans`, `_range_frame`.
- Produces: `axis_sheet(data, key, headline, out_dir) -> list[Path]`, `_axis_headline(data, key) -> tuple[str, str]`; files `codec.svg/png`, `rowgroup.svg/png`.

- [ ] **Step 1: Write the failing test** (in `test_benchviz.py`)

```python
def test_the_axis_sheets_render_from_the_measured_fixture(tmp_path):
    """Both configuration axes draw from one function, one slice or many."""
    from benchviz import figures

    data, _ = prep.build(prep.Inputs(_bench_dir(tmp_path)))
    data_path = tmp_path / "axes.json"
    data_path.write_text(prep.json.dumps(data), encoding="utf-8")
    written = sorted(
        p.name for p in figures.main(data_path=data_path, out_dir=tmp_path / "f").glob("*")
    )
    for name in ("codec.svg", "codec.png", "rowgroup.svg", "rowgroup.png"):
        assert name in written
    assert "compression.svg" not in written

    title, subtitle = figures._axis_headline(data, "rowgroup")
    assert "512" in title or "2048" in title or "floor" in title
    assert "2231" in subtitle.replace(",", "") or "1 slice" in subtitle
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cd benchmark/plot && uv run --extra dev pytest -q tests/test_benchviz.py -k axis_sheets`
Expected: FAIL (`AttributeError: _axis_headline` or the compression key error in `_load`).

- [ ] **Step 3: Implement**

1. Remove `compression()` (lines ~1507-1727), `_compression_headline`, `MAX_COMPRESSION_PANELS`. In `_load`, the key tuple becomes `("meta", "datasets", "read", "sizes", "ordering", "scaling")`. `_check_capacity` becomes:

```python
def _check_capacity(data: dict[str, Any]) -> None:
    datasets = len(data["datasets"])
    if datasets > MAX_PANELS:
        raise SystemExit(
            "benchviz figures: this run does not fit the figures' panel grid "
            f"({datasets} datasets against room for {MAX_PANELS}).\n"
            "  The HTML summary page has no such limit and covers all of them; "
            "only the static print figures are pinned.\n"
            "  Re-fitting them means deciding a layout for that many panels "
            "and revising the finding sentences each figure asserts, which are "
            "written for the corpus they were drawn from."
        )
```

Update the module docstring: six figures, `codec` and `rowgroup` in place of `compression`. Update the `configuration` figure's footer sentence (line ~2432-2435) to: `"Row ordering is one of three configuration axes; the codec and row-group axes are measured the same way by `just codec-bench` and `just rowgroup-bench` and drawn on their own sheets."`

2. Constants, next to `FORMAT_STYLE`:

```python
# The configuration axes' bars. Codec: the zstd sweep is one family in the
# accent hue at four lightness steps, the other codecs five muted hues; row
# group: one sequential hue from small groups (light) to large (dark). The
# baseline is never a bar, so it needs no colour.
AXIS_BASELINE = "cityparquet"
AXIS_ROWS = [
    ("write", "write"),
    ("full-read", "full read"),
    ("bbox-5pct", "spatial 5%"),
    ("size", "bytes on disk"),
]
AXIS_PANELS = 4
TREND_PANELS = [
    ("size", "bytes on disk", "MB"),
    ("write", "write time", "s"),
    ("write-rss", "write peak RSS", "MB"),
    ("bbox-5pct", "spatial 5% time", "s"),
]
CODEC_OTHER_COLOURS = ["#4e79a7", "#59a14f", "#9c755f", "#b07aa1", "#76b7b2"]
ROWGROUP_HUE = "#3b6ea5"
```

3. Helpers and the sheet:

```python
def _variant_label(variant: str) -> str:
    if variant == AXIS_BASELINE:
        return "default"
    suffix = variant.removeprefix("cityparquet+")
    if suffix.startswith("rg"):
        return f"{int(suffix[2:]):,} rows"
    if suffix == "uncompressed":
        return "none"
    if suffix.startswith("zstd") and suffix != "zstd":
        return f"zstd {suffix[4:]}"
    return suffix


def _mix(colour: str, white: float) -> str:
    r, g, b = mcolors.to_rgb(colour)
    return mcolors.to_hex((r + (1 - r) * white, g + (1 - g) * white, b + (1 - b) * white))


def _axis_palette(key: str, variants: Sequence[str]) -> dict[str, str]:
    palette: dict[str, str] = {}
    if key == "codec":
        zstd = [v for v in variants if v.startswith("cityparquet+zstd")]
        others = [v for v in variants if v not in zstd and v != AXIS_BASELINE]
        for i, v in enumerate(zstd):
            palette[v] = _mix(ACCENT, 0.55 * (1 - i / max(len(zstd) - 1, 1)))
        for i, v in enumerate(others):
            palette[v] = CODEC_OTHER_COLOURS[i % len(CODEC_OTHER_COLOURS)]
    else:
        ordered = [v for v in variants if v != AXIS_BASELINE]
        for i, v in enumerate(ordered):
            palette[v] = _mix(ROWGROUP_HUE, 0.6 * (1 - i / max(len(ordered) - 1, 1)))
    return palette


def _axis_slices(axis: dict[str, Any]) -> list[dict[str, Any]]:
    """Slices of the run, largest first, in the shape `_panel_pick` reads."""
    seen: dict[str, int | None] = {}
    for r in axis["records"]:
        seen.setdefault(r["dataset"], r.get("objects"))
    slices = [
        {"id": ds, "objects": n or 0, "subtitle": f"{n:,} CityObjects" if n else ""}
        for ds, n in seen.items()
    ]
    return sorted(slices, key=lambda s: -s["objects"])


def _axis_cell(
    axis: dict[str, Any], dataset: str, measure: str, variant: str
) -> dict[str, Any] | None:
    """The ratio cell for one (slice, row, variant): a record, or a size entry
    dressed as one for the `size` row."""
    if measure == "size":
        for s in axis["sizes"]:
            if s["dataset"] == dataset and s["variant"] == variant:
                return {"time_ratio": s.get("size_ratio"), "rss_ratio": None, "below_floor": False,
                        "base": None}
        return None
    for r in axis["records"]:
        if r["dataset"] == dataset and r["measure"] == measure and r["variant"] == variant:
            return {"time_ratio": r.get("time_ratio"), "rss_ratio": r.get("rss_ratio"),
                    "below_floor": bool(r.get("below_floor")), "base": r.get("base_time_s")}
    return None


def _axis_base_size(axis: dict[str, Any], dataset: str) -> float | None:
    for s in axis["sizes"]:
        if s["dataset"] == dataset and s["variant"] == AXIS_BASELINE:
            return s.get("mb")
    return None


def axis_sheet(
    data: dict[str, Any], key: str, headline: tuple[str, str], out_dir: Path
) -> list[Path]:
    """One configuration axis: the read-figure sheet on top, the trend strip below.

    Top: four slices spread across the run by object count, rows write / full
    read / spatial 5 % / bytes on disk, bars per variant against the default at
    the 1x rule; time (and size) left, peak memory right. Bottom: every slice on
    log-log axes, absolute values, one line per variant — the "when" half.
    """
    axis = data["scaling"][key]
    variants = [v for v in axis["variants"] if v != AXIS_BASELINE]
    if not variants or not axis["records"]:
        raise DataContractError(f"bench_data.json carries no {key} records.")
    slices = _axis_slices(axis)
    picked = _panel_pick(slices, AXIS_PANELS)
    palette = _axis_palette(key, axis["variants"])
    floor_ms = data["meta"]["citation_floor_s"] * 1000

    ratios, mems = [], []
    for ds in picked:
        for measure, _ in AXIS_ROWS:
            for v in variants:
                cell = _axis_cell(axis, ds["id"], measure, v)
                if cell and cell["time_ratio"]:
                    ratios.append(cell["time_ratio"])
                if cell and cell["rss_ratio"]:
                    mems.append(cell["rss_ratio"])
    if not ratios:
        raise DataContractError(f"bench_data.json carries no usable {key} ratios.")
    tlo, thi = min(min(ratios) / 2, 0.5), max(max(ratios) * 2, 2.0)
    mlo, mhi = (min(min(mems) / 2, 0.5), max(max(mems) * 2, 2.0)) if mems else (0.5, 2.0)

    cols_n = 2 if len(picked) > 1 else 1
    rows_n = math.ceil(len(picked) / cols_n)
    fig = plt.figure(figsize=(7.1, min(MAX_SHEET_HEIGHT, 2.6 * rows_n + 4.4)))
    head_bottom = _headline(fig, *headline)
    machine = (data["meta"].get("machine") or {}).get(key)
    machine_line = (
        "Measured on: " + machine.strip().splitlines()[4].strip() + "."
        if machine and len(machine.strip().splitlines()) > 4
        else "No machine record for this run."
    )
    footer = [
        "Bars grow out of the 1x rule -- the default CityParquet write of the same "
        "slice, same measure -- on a logarithmic axis. Right is faster, leaner, or "
        "for the bytes row smaller, than the default; the default itself is not a bar.",
        "Every bar prints its ratio. A faded bar is a time difference under the "
        f"{floor_ms:.0f} ms citation floor and its ratio carries a ≈. The grey "
        "figure beside each row is the default's own absolute value. The bytes row "
        "has no memory column.",
        f"{len(picked)} of {len(slices)} slices are drawn above, spread by CityObject "
        "count; the strip below carries every slice on log-log axes with absolute "
        "values, one line per variant, the default drawn heavier.",
        data["meta"]["codec_level_note"] if key == "codec" else
        "Every row-group variant is written with the default codec (zstd 3); only the "
        "rows per group change.",
        machine_line,
    ]
    if axis["gaps"]:
        footer.append(
            "Flagged in this run: "
            + "; ".join(f"{g['dataset']} — {g['issue']}" for g in axis["gaps"]) + "."
        )
    bottom = _footer_reserve(fig, footer, 0.10) + 0.02
    top = min(0.88, head_bottom - 0.05)

    # Vertical budget: the ratio sheet takes the upper share, the trend strip a
    # fixed band above the key strip.
    strip_h = 0.16
    key_h = 0.035
    sheet_top, sheet_bottom = top, bottom + strip_h + key_h + 0.06
    left, right, gutter, inner = 0.012, 0.992, 0.145, 0.028
    w = (right - left - cols_n * (gutter + inner)) / (2 * cols_n)
    cell_w = gutter + inner + 2 * w
    h = (sheet_top - sheet_bottom) / (rows_n + (rows_n - 1) * 0.34)
    gap = 0.34 * h
    panel_in = h * fig.get_figheight()
    pw_in = w * fig.get_figwidth()
    title_y, subtitle_y = 1.0 + 0.30 / panel_in, 1.0 + 0.13 / panel_in
    x_label, x_secs = -0.42 / pw_in, -0.045 / pw_in

    step = len(variants) + 1.4
    rows_total = len(AXIS_ROWS) * step
    for i, ds in enumerate(picked):
        row, col = divmod(i, cols_n)
        x0 = left + col * cell_w + gutter
        y0 = sheet_top - (row + 1) * h - row * gap
        ax_t = fig.add_axes([x0, y0, w, h])
        ax_m = fig.add_axes([x0 + w + inner, y0, w, h])
        _panel_heading(ax_t, ds["id"], ds["subtitle"], title_y, subtitle_y)
        for si, (measure, label) in enumerate(AXIS_ROWS):
            base_y = si * step
            mid = base_y + (len(variants) - 1) / 2
            ax_t.text(
                x_label, mid, label, transform=ax_t.get_yaxis_transform(),
                fontsize=FS_LABEL, family="serif", color=INK, ha="right", va="center",
            )
            if si:
                for ax in (ax_t, ax_m):
                    ax.axhline(
                        base_y - (step - (len(variants) - 1)) / 2,
                        color=AXIS, linewidth=0.4, zorder=0,
                    )
            cells = [(v, _axis_cell(axis, ds["id"], measure, v)) for v in variants]
            if not any(c for _, c in cells):
                ax_t.text(
                    0.02, mid, "not measured in this run",
                    transform=ax_t.get_yaxis_transform(), fontsize=FS_MARK,
                    family="serif", color=INK_3, ha="left", va="center", style="italic",
                )
                continue
            if measure == "size":
                base_mb = _axis_base_size(axis, ds["id"])
                base_text = f"{base_mb:,.1f} MB" if base_mb else ""
            else:
                base_s = next((c["base"] for _, c in cells if c and c["base"]), None)
                base_text = _secs(base_s) if base_s else ""
            if base_text:
                ax_t.text(
                    x_secs, mid, base_text, transform=ax_t.get_yaxis_transform(),
                    fontsize=FS_MARK, family="serif", color=INK_3, ha="right", va="center",
                )
            for vi, (v, cell) in enumerate(cells):
                y = base_y + vi
                if not cell:
                    continue
                colour = palette[v]
                if cell["time_ratio"]:
                    ratio = cell["time_ratio"]
                    faded = cell["below_floor"] and measure != "size"
                    _ratio_bar(
                        ax_t, y, ratio, tlo, thi, height=BAR_H, color=colour,
                        alpha=0.34 if faded else 1.0, linewidth=0,
                    )
                    _bar_value_label(
                        ax_t, y, ratio, tlo, thi, _speedup_label(ratio, faded),
                        pw_in / (math.log10(thi) - math.log10(tlo)),
                    )
                if cell["rss_ratio"]:
                    lean = cell["rss_ratio"]
                    _ratio_bar(
                        ax_m, y, lean, mlo, mhi, height=BAR_H, color=colour,
                        alpha=1.0, linewidth=0,
                    )
                    _bar_value_label(
                        ax_m, y, lean, mlo, mhi, _speedup_label(lean, False),
                        pw_in / (math.log10(mhi) - math.log10(mlo)),
                    )
        _format_panel_axis(ax_t, tlo, thi, rows_total, max_ticks=3)
        _format_panel_axis(ax_m, mlo, mhi, rows_total, max_ticks=3)
        _axis_label(ax_t, "time (x faster than default); bytes row: x smaller")
        _axis_label(ax_m, "peak memory (x leaner)")

    _axis_key(fig, variants, palette, sheet_bottom - 0.03)
    _trend_strip(fig, axis, variants, palette, bottom + 0.035, strip_h)
    _footer(fig, footer)
    return _save(fig, key, out_dir)


def _axis_key(
    fig: Figure, variants: Sequence[str], palette: dict[str, str], y: float
) -> None:
    x = 0.012
    for v in variants:
        fig.add_artist(
            Rectangle(
                (x, y), 0.016, 0.006, transform=fig.transFigure,
                facecolor=palette[v], edgecolor="none", zorder=5,
            )
        )
        label = _variant_label(v)
        fig.text(x + 0.020, y, label, fontsize=FS_LABEL, family="serif", color=INK, va="bottom")
        x += 0.024 + 0.0088 * len(label)
    fig.text(
        x + 0.006, y, "-- same order in every group; the default is the 1x rule",
        fontsize=FS_MARK, family="serif", color=INK_2, va="bottom", style="italic",
    )


def _trend_value(axis: dict[str, Any], dataset: str, panel: str, variant: str) -> float | None:
    if panel == "size":
        for s in axis["sizes"]:
            if s["dataset"] == dataset and s["variant"] == variant:
                return s.get("mb")
        return None
    measure = "write" if panel in ("write", "write-rss") else panel
    for r in axis["records"]:
        if r["dataset"] == dataset and r["measure"] == measure and r["variant"] == variant:
            if panel == "write-rss":
                return r["rss_b"] / (1024 * 1024) if r.get("rss_b") else None
            return r.get("time_s")
    return None


def _trend_strip(
    fig: Figure, axis: dict[str, Any], variants: Sequence[str],
    palette: dict[str, str], y0: float, height: float,
) -> None:
    """Every slice, absolute values, log-log: the slope is the finding."""
    slices = sorted(_axis_slices(axis), key=lambda s: s["objects"])
    xs = [s["objects"] for s in slices]
    n = len(TREND_PANELS)
    left, right, inner = 0.075, 0.992, 0.05
    w = (right - left - (n - 1) * inner) / n
    for pi, (panel, title, unit) in enumerate(TREND_PANELS):
        ax = fig.add_axes([left + pi * (w + inner), y0, w, height])
        drawn_y: list[float] = []
        for v in [AXIS_BASELINE, *variants]:
            ys = [_trend_value(axis, s["id"], panel, v) for s in slices]
            pts = [(x, y) for x, y in zip(xs, ys) if y]
            if not pts:
                continue
            drawn_y.extend(y for _, y in pts)
            ax.plot(
                [p[0] for p in pts], [p[1] for p in pts],
                color=GRAY if v == AXIS_BASELINE else palette[v],
                linewidth=1.6 if v == AXIS_BASELINE else 0.9,
                marker="o", markersize=2.2, zorder=3 if v == AXIS_BASELINE else 2,
            )
        ax.set_xscale("log")
        if drawn_y and min(drawn_y) > 0:
            ax.set_yscale("log")
        ax.set_title(f"{title} ({unit})", fontsize=FS_PANEL_SUB, family="serif", color=INK_2, loc="left")
        ax.tick_params(labelsize=FS_VALUE)
        if len(xs) > 1:
            _range_frame(ax, xs, drawn_y or [1.0])
        _sans(ax)
        if pi == 0:
            ax.set_ylabel("absolute", fontsize=FS_MARK, family="serif", color=INK_3)
    fig.text(
        0.012, y0 + height + 0.008, "trend across every slice",
        fontsize=FS_LABEL, family="serif", color=INK, va="bottom",
    )
    fig.text(
        (left + right) / 2, y0 - 0.028, "CityObjects per slice (log)",
        fontsize=FS_MARK, family="serif", color=INK_2, ha="center", va="top",
    )
```

Check `_range_frame(ax, xs, ys)`'s signature at line ~440 before calling it; if it expects sequences of floats, the call above is correct.

4. Headlines:

```python
def _axis_headline(data: dict[str, Any], key: str) -> tuple[str, str]:
    axis = data["scaling"][key]
    recipe = "codec-bench" if key == "codec" else "rowgroup-bench"
    if not axis["records"]:
        return (f"No {key} run in this corpus.", f"`just {recipe}` produces it.")
    slices = _axis_slices(axis)
    largest = slices[0]
    variants = [v for v in axis["variants"] if v != AXIS_BASELINE]
    objects = f"{largest['objects']:,}"
    subtitle = (
        f"{len(slices)} slice(s) of one 3DBAG model, 3 measures and {len(variants)} "
        f"variants, each against the default CityParquet write of the same slice at 1×. "
        "Time left, peak memory right; both logarithmic."
    )
    if key == "codec":
        zstd = [v for v in variants if v.startswith("cityparquet+zstd")]
        sizes = [
            c["time_ratio"] for v in zstd
            if (c := _axis_cell(axis, largest["id"], "size", v)) and c["time_ratio"]
        ]
        writes = [
            c["time_ratio"] for v in zstd
            if (c := _axis_cell(axis, largest["id"], "write", v)) and c["time_ratio"]
        ]
        reads = [
            (c["time_ratio"], v) for v in variants
            if (c := _axis_cell(axis, largest["id"], "full-read", v)) and c["time_ratio"]
        ]
        title = f"On {objects} objects"
        if sizes and writes:
            title += (
                f" the zstd sweep spans {_times(min(sizes))}–{_times(max(sizes))} of the "
                f"default's bytes for {_times(min(writes))}–{_times(max(writes))} of its write time"
            )
        if reads:
            best, v = max(reads)
            title += f"; {_variant_label(v)} reads fastest, at {_times(best)} the default"
        return title + ".", subtitle
    cleared = []
    for v in variants:
        c = _axis_cell(axis, largest["id"], "bbox-5pct", v)
        if c and c["time_ratio"] and c["time_ratio"] > 1 and not c["below_floor"]:
            w = _axis_cell(axis, largest["id"], "write", v)
            cleared.append((int(v.removeprefix("cityparquet+rg")), c["time_ratio"],
                            w["time_ratio"] if w and w["time_ratio"] else None))
    if not cleared:
        title = (
            f"On {objects} objects no row-group size clears the "
            f"{data['meta']['citation_floor_s'] * 1000:.0f} ms floor on the spatial window"
        )
    else:
        rows, gain, write = max(cleared)  # the largest group that still pays
        title = f"Row groups of {rows:,} rows answer the 5 % window {_times(gain)} faster than the default on {objects} objects"
        if write:
            title += f", for {_times(1 / write)} the write time"
    return title + ".", subtitle
```

5. `main`: replace the `compression` branch with

```python
    for key, recipe in (("codec", "codec-bench"), ("rowgroup", "rowgroup-bench")):
        if data["scaling"].get(key, {}).get("records"):
            written += axis_sheet(data, key, _axis_headline(data, key), out_dir)
        else:
            print(
                f"  {key} figure skipped: this corpus has no {key} run "
                f"(benchmark/formats/scaling_{key}_results is empty) — `just {recipe}` "
                "produces it"
            )
```

- [ ] **Step 4: Render and look**

Run:
```bash
cd benchmark/plot && uv run --extra dev pytest -q tests/test_benchviz.py
T=$(mktemp -d) && mkdir -p $T/benchmark && cp -r tests/fixtures/benchviz $T/benchmark/formats \
  && cp ../formats/READ_BENCHMARK.md ../formats/README.md $T/benchmark/formats/ \
  && uv run python -m benchviz all --bench-dir $T/benchmark/formats --out $T/out && ls $T/out/figures
```
Open `$T/out/figures/codec.png` and `rowgroup.png` (the Read tool renders PNGs) and check: headline not overlapping panels; four rows per panel with labels legible; key strip under the panels; trend strip with one point per line (one fixture slice); footer inside the page. Adjust `strip_h`, `key_h`, the `+ 4.4` figure height and the `0.34` gap until nothing overlaps. Ratio direction sanity: on the row-group fixture, `rg512`'s `bytes on disk` bar must point LEFT (bigger file, ratio below 1) and its `write` bar most likely left too.

Expected: full Python suite green, including `test_a_corpus_with_no_axis_run_is_stated_not_crashed` from Task 7.

- [ ] **Step 5: Commit**

```bash
git add benchmark/plot/benchviz/figures.py benchmark/plot/tests/test_benchviz.py
git commit -m "feat(benchviz): codec and rowgroup sheets in the read figure's shape, with a trend strip

Claude-Session: https://claude.ai/code/session_012rPUqbWax2xKR4GuT8Bmtq"
```

---

### Task 10: documentation

**Files:**
- Modify: `CLAUDE.md:74`, `AGENTS.md:74` (the recipe list: `variant-bench` in place of `compression-bench`; the sentence "The four per-dataset recipes" stays true)
- Modify: `lib/cityparquet-rs/CLAUDE.md:21`, `lib/cityparquet-rs/AGENTS.md:21` (same list)
- Modify: `lib/cityparquet-rs/README.md:209` (table row)
- Modify: `benchmark/README.md:115` (running section)
- Modify: `benchmark/formats/README.md` (title, the "No write-side CSVs are committed" paragraph, the recipe table, the codec-levels section)
- Modify: `benchmark/formats/READ_BENCHMARK.md` (only if it mentions `compression-bench`; check with grep)
- Modify: `benchmark/plot/benchviz/DESIGN.md` (data contract: `scaling.codec`/`scaling.rowgroup`, the figure list, the honesty rule 5 about codec levels)
- Modify: `benchmark/plot/benchviz/__main__.py:87` (description string)
- Modify: `benchmark/readbench/src/scaling.rs:4` (doc comment names `just codec-bench`, `just rowgroup-bench`)
- Modify: `test/TESTING.md:41,1353,1427`

- [ ] **Step 1: Make every edit**

`lib/cityparquet-rs/README.md` table: replace the `compression-bench` row with two rows:

```markdown
| `just codec-bench FOLDER [OUT] [PREPARED]`    | codec axis over every input under `FOLDER` on the read harness: a timed write per variant (peak RSS), then a full read and the bbox windows; `OUT` default `benchmark/formats/scaling_codec_results` |
| `just rowgroup-bench FOLDER [OUT] [PREPARED]` | the same for the row-group axis, `OUT` default `benchmark/formats/scaling_rowgroup_results`                                                                                                  |
```

`benchmark/README.md` running section: replace `just compression-bench benchmark/formats/data/scaling` with

```sh
just codec-bench    benchmark/formats/data/scaling   # codec axis: zstd 1/3/9/19, lz4, snappy, gzip, brotli, none
just rowgroup-bench benchmark/formats/data/scaling   # row-group axis: 65536, 32768, 8192, 2048, 512
```

`benchmark/formats/README.md`:
- Title: `# CityParquet write and configuration benchmark methodology`.
- Replace the "**No write-side CSVs are committed.**" paragraph with: "The write-side CSVs under `results/` and `scaling_write_results/` are committed from the 2026-08-27 run; the configuration axes under `scaling_codec_results/` and `scaling_rowgroup_results/` carry a `MACHINE.md` naming the host they were measured on."
- The recipe table: replace the `compression-bench` row with rows for `codec-bench` and `rowgroup-bench` (same wording as the README table above) and change the sentence after the table to: "`just codec-bench`'s and `just rowgroup-bench`'s CSVs are in the READ run's shape (`READ_BENCHMARK.md`), with a `write` row per variant and the variant id in the `format` column; they feed the summary page's section 3b and the `codec`/`rowgroup` print figures (`benchmark/plot/benchviz`)."
- Section "The codec levels are NOT matched": keep the heading, rewrite the body: zstd is swept at 1, 3, 9, 19 by `codec-bench`; gzip and brotli stay at parquet-rs defaults 6 and 1 and are reference points; "the smallest codec" remains a non-citable claim across codecs, while "zstd level N versus level M" is a measured one.

`DESIGN.md`: in the contract JSON, delete the `"compression"` and `"compression_gaps"` keys and the `scaling.compression` line; add under `"scaling"`:

```jsonc
    "codec": {
      "variants": ["cityparquet", "cityparquet+zstd1", "..."], // recipe order = figure order
      "records": [
        { "dataset": "<slice>", "objects": 50001, "variant": "cityparquet+zstd1",
          "kind": "variant", "measure": "write",       // write | full-read | bbox-1pct | bbox-5pct | bbox-25pct
          "time_s": 1.9, "rss_b": 0, "base_time_s": 1.8, "base_rss_b": 0,
          "time_ratio": 0.95, "rss_ratio": 1.0,        // baseline / variant: > 1 is faster, leaner
          "below_floor": false }
      ],
      "sizes": [ { "dataset": "<slice>", "objects": 50001, "variant": "...", "bytes": 0, "mb": 0.0,
                   "ratio_vs_cityjsonseq": 0.0, "size_ratio": 1.1 } ],   // baseline / variant: > 1 is smaller
      "gaps": [ { "dataset": "<slice>", "issue": "..." } ]
    },
    "rowgroup": { /* the same shape */ },
```

and add `meta.machine`, `meta.axis_baseline`. In the figure list replace item 4 "Compression codec grid" with "`codec` and `rowgroup` — one sheet each in the `formats` shape plus a trend strip; baseline is the default write, never a bar". Honesty rule 5 becomes the new codec-level text.

`test/TESTING.md`: line 41 list; line 1353 `ls` line: `scaling_codec_results scaling_rowgroup_results` in place of `compression_results`; line 1427: `just codec-bench benchmark/formats/data/scaling` and a `just rowgroup-bench` line.

- [ ] **Step 2: Verify no stale mention remains and the mirrors match**

Run:
```bash
grep -rn "compression-bench\|compression_results\|compression-plot\|readbench_plot.compression\|compression\.svg\|compression\.png" --include=*.md --include=justfile --include=*.py --include=*.rs --include=*.sh . | grep -v "ai/design-notes/"
diff CLAUDE.md AGENTS.md && diff lib/cityparquet-rs/CLAUDE.md lib/cityparquet-rs/AGENTS.md && echo mirrors-ok
just plot-test 2>&1 | tail -2
```
Expected: the grep prints nothing; `mirrors-ok`; plot tests green (a caveat-extraction test reads the live `README.md` headings, so a heading change there must keep the `## ` shape the extractor expects; if `read_caveats`/`_extract_section` fails, restore the heading prefix it looks for).

- [ ] **Step 3: Commit**

```bash
git add -A CLAUDE.md AGENTS.md lib/cityparquet-rs/CLAUDE.md lib/cityparquet-rs/AGENTS.md lib/cityparquet-rs/README.md benchmark test/TESTING.md
git commit -m "docs: the codec and row-group benchmarks replace compression-bench everywhere they are named

Claude-Session: https://claude.ai/code/session_012rPUqbWax2xKR4GuT8Bmtq"
```

---

### Task 11: the runs

**Files:**
- Create: `benchmark/formats/data` (symlink, ignored)
- Create: `benchmark/formats/scaling_codec_results/*`, `benchmark/formats/scaling_rowgroup_results/*` (committed)

**Interfaces:**
- Consumes: the recipes from Task 6, the slices at `/data2/hideba/cityparquet/benchmark/formats/data/`.
- Produces: seven `3dbag_n*.csv` + `.params.json` per directory, `sizes.csv`, `MACHINE.md`.

- [ ] **Step 1: Point the worktree at the existing corpus and gather the slices**

```bash
cd /data2/hideba/cityparquet-paper/cityparquet-bench
ln -s /data2/hideba/cityparquet/benchmark/formats/data benchmark/formats/data
git status --short | grep -c "formats/data" ; # must print 0: the symlink is ignored
S=/data2/hideba/cityparquet-paper/cityparquet-bench/benchmark/formats/data/scaling-all
mkdir -p "$S"
for f in benchmark/formats/data/scaling/3dbag_n*.city.jsonl benchmark/formats/data/scaling-large/3dbag_n*.city.jsonl; do ln -sf "$(readlink -f "$f")" "$S/"; done
ls "$S"   # seven slices, 1k .. 1M
```

If `scaling-large/` does not hold `3dbag_n500000` and `3dbag_n1000000`, cut them: `just fetch-scaling-data benchmark/formats/data/scaling-all 500000,1000000` (the source `.fcb` is present, so no download).

- [ ] **Step 2: Smoke the recipe on the smallest slice**

```bash
mkdir -p /tmp/claude-1020/smoke && ln -sf "$S/3dbag_n1000.city.jsonl" /tmp/claude-1020/smoke/
just rowgroup-bench /tmp/claude-1020/smoke /tmp/claude-1020/smoke-out
head -3 /tmp/claude-1020/smoke-out/3dbag_n1000.csv; cat /tmp/claude-1020/smoke-out/sizes.csv; head -8 /tmp/claude-1020/smoke-out/MACHINE.md
```
Expected: 5 variants × 5 rows = 25 data rows; `sizes.csv` with 5 rows; `MACHINE.md` naming this host.

- [ ] **Step 3: Run both axes, one after the other, in the background**

```bash
cd /data2/hideba/cityparquet-paper/cityparquet-bench
nohup bash -c "just rowgroup-bench '$S' && just codec-bench '$S'" \
  > /data2/hideba/cityparquet-paper/cityparquet-bench/benchmark/formats/variant-bench.log 2>&1 &
echo $!
```

Row group first (under an hour at 1M), then codec (2 to 3 hours at 1M). Poll with `tail -3 benchmark/formats/variant-bench.log` and `ls benchmark/formats/scaling_*_results/`. Never start the second while the first runs. Add `benchmark/formats/variant-bench.log` to nothing: delete it when done.

- [ ] **Step 4: Check and commit the results**

```bash
for d in scaling_rowgroup_results scaling_codec_results; do
  echo "== $d"; ls benchmark/formats/$d; wc -l benchmark/formats/$d/sizes.csv
  for f in benchmark/formats/$d/3dbag_n*.csv; do echo "$(basename $f) $(grep -c ',write,' $f) writes $(grep -c ',full-read,' $f) full-reads"; done
done
rm benchmark/formats/variant-bench.log
cd benchmark/plot && uv run python -m benchviz all && ls ../summary/figures/ | grep -E "codec|rowgroup"
```
Expected: 7 CSVs per directory; row group 5 writes and 5 full-reads per CSV; codec 9 and 9; figures rendered. Open `benchmark/summary/figures/rowgroup.png` and `codec.png` and confirm the trend strip now shows seven points per line and the headline reads sensibly. If a headline sentence reads wrongly for the real data (e.g. the row-group "largest group that pays" logic picks a surprising size), fix `_axis_headline` in a follow-up commit rather than editing the figure by hand.

```bash
git add benchmark/formats/scaling_rowgroup_results benchmark/formats/scaling_codec_results
git commit -m "bench(codec,rowgroup): measure both configuration axes over the 3DBAG slices, 1k to 1M

Claude-Session: https://claude.ai/code/session_012rPUqbWax2xKR4GuT8Bmtq"
```

---

### Task 12: the paper repository

**Files (in `/data2/hideba/cityparquet-paper/`):**
- Delete: `paper/assets/bench/compression.png`, `paper/assets/bench/compression.svg` (untracked renders; `rm`, not `git rm`)
- Modify: `cityparquet` submodule pointer (after the branch is merged or pushed), `paper/assets/bench/codec.*`, `paper/assets/bench/rowgroup.*`, `paper/assets/bench/configuration.*` (re-rendered)
- Modify: `CLAUDE.md` and `AGENTS.md` of the paper repo only if they mention the compression figure (they do not today; check with grep)

- [ ] **Step 1: Render the paper's figures from the bench branch**

The paper's `just bench-summary` reads the submodule at `cityparquet/`, which is on the rebuild branch. Until the bench branch is merged, render from the worktree instead:

```bash
cd /data2/hideba/cityparquet-paper
rm -f paper/assets/bench/compression.png paper/assets/bench/compression.svg
uv run --project cityparquet-bench/benchmark/plot python -m benchviz prep \
  --bench-dir cityparquet-bench/benchmark/formats --data cityparquet-bench/benchmark/summary/bench_data.json
uv run --project cityparquet-bench/benchmark/plot python -m benchviz figures \
  --data cityparquet-bench/benchmark/summary/bench_data.json --figures paper/assets/bench
ls paper/assets/bench
just build
```
Expected: `codec.*`, `rowgroup.*` present, `compression.*` gone, the PDF builds (no chapter references the figures yet, so nothing else changes).

- [ ] **Step 2: Report to the author**

Do not commit in the paper repository. Report: the two new figures under `paper/assets/bench/`, the bench branch name, that the submodule pointer will move once `bench/configuration-axes` is merged into `develop`, and that no chapter references the figures yet (that is the author's writing, not this plan's).

---

## Self-review

**Spec coverage.** §3 grammar and lists: Tasks 1, 6. §3.4 naming: Task 4. §4.1 option and rejections: Task 4 (`exclusive`, `baseline`, `duplicate`, http, write-repeat). §4.2 write child, warmup, `tempdir_in`, kept package, `generate_lod0`: Tasks 3, 4. §4.3 CSV shape, `Measure`, `Row.label`: Task 4. §4.4 sizes with replace-on-rerun: Task 4. §4.5 params sidecar: unchanged, asserted in Task 4's test. §5.1 corpus: Task 11. §5.2 recipes, stripper count, `bench_recipe_test.sh`: Task 6. §5.3 MACHINE.md: Task 6, parsed in Task 7, shown in Tasks 8 and 9. §5.4 run order: Task 11. §5.5 removals: Tasks 5, 6. §5.6 committed results: Task 11. §6 contract: Task 7; HTML: Task 8; tests: Tasks 7, 9. §7 figures and paper-side removal: Tasks 9, 12. §8 out of scope: nothing here touches ordering, `write-bench`, HTTP or other codec levels.

**Placeholders.** None; every code step carries its code. Two steps say "check the name of an existing helper before calling it" (`secs` in Task 8, `_range_frame` in Task 9) because the plan was written from partial reads of those files; that is a verification instruction, not a gap.

**Type consistency.** `Variant::{parse, id, recipe, ordering}` used identically in Tasks 1 to 4. `Measure::{Write, Read}` and `Row.label` in Task 4 only. `RunOptions.variants: Option<Vec<String>>`, `write_repeat: usize` in Task 4 and the `RunArgs` block. `load_scaling_axis` returns `{"records","sizes","gaps","variants"}` in Task 7; Tasks 8 and 9 read exactly those keys plus `meta.machine`, `meta.codec_level_note`, `meta.axis_baseline`. `AXIS_BASELINE`, `AXIS_MEASURES` defined in `prep.py` (Task 7) and `AXIS_BASELINE` redefined in `figures.py` (Task 9) with the same value, as `FORMAT_AXIS` already is in both modules. Ratio direction: baseline over variant in Task 4's `sizes.csv` (`baseline_bytes / bytes`), Task 7's `_ratio(base_t, t)` and `_ratio(base_b, b)`, and Task 9 draws them without inversion.
