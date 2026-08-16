# Format-Comparison Benchmark Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add CityGML and plain CityJSON as measured read-benchmark formats, split the benchmark into two single-axis sets (format comparison, ordering comparison), and thread format selection through the whole harness.

**Architecture:** A `Format` enum replaces stringly-typed dispatch so adding a format is compiler-checked. Two new `FormatRunner` implementations join the existing three. Artefacts derive forward from a CityGML source via citygml-tools → cjseq → (fcb ser | cityparquet convert), never through CityParquet. Format selection threads from the justfile through `readbench_prepare.sh` into the coordinator's existing `--formats` flag.

**Tech Stack:** Rust (`cityparquet-readbench`), bash (prepare/duckdb/fetch scripts), `just`, Python/uv (`bench/plot`), citygml-tools (Java 21), `cjseq`, `fcb`.

**Design spec:** `docs/superpowers/specs/2026-08-16-format-comparison-benchmark-design.md`

## Global Constraints

- **Strict red-green TDD**: failing test first, run it, confirm it fails for the right reason, then the smallest change to pass, then refactor.
- **British English** in prose, comments and docstrings.
- **Rust tests read real fixtures** from `tests/fixtures/` (downloaded by `just fixtures`) — never inline hand-written CityJSON or CityGML.
- **`cityparquet-schema` stays free of `arrow-array`/`parquet`** (`just isolation`).
- **`just check` green** before any Rust task is done; **`just catalog-test` green** if the driver is touched (it should not be).
- Do **not** modify `vendor/` or `tools/catalog2cityparquet/`.
- **Format tags** (canonical CSV/CLI spellings): `citygml`, `cityjson`, `cityjsonseq`, `cityjsonseq-gz`, `flatcitybuf`, `cityparquet`, `cityparquet-hilbert`. Plus the out-of-band baseline `duckdb-parquet`, which is never a `--child` format.
- **Format-comparison set** (the new default): `citygml, cityjson, cityjsonseq, flatcitybuf, cityparquet-hilbert`.
- **Ordering set**: `cityparquet, cityparquet-hilbert`.
- **Four `find` patterns** (`justfile:98-100, 174-176, 210-212, 248-250`) and **four basename-strippers** (`readbench_prepare.sh:50-52`, `coordinator.rs:422-429`, the justfile pairs, `readbench_duckdb.sh:193-195`) must stay in lockstep. Any extension added to one goes in all.

---

## File Structure

| File | Responsibility |
|---|---|
| `scripts/readbench_duckdb.sh` | *(modify)* 13-column CSV contract |
| `bench/plot/readbench_plot/__init__.py` | *(modify)* 13-column contract; shared `FORMAT_ORDER` + `FORMAT_COLORS` |
| `crates/cityparquet-readbench/src/format.rs` | **(new)** the `Format` enum — `ALL`, `as_str`, `FromStr`, artefact naming |
| `crates/cityparquet-readbench/src/formats/cityjson.rs` | **(new)** plain-document CityJSON runner |
| `crates/cityparquet-readbench/src/formats/citygml.rs` | **(new)** CityGML runner |
| `crates/cityparquet-readbench/src/formats/mod.rs` | *(modify)* dispatch on `Format` |
| `crates/cityparquet-readbench/src/coordinator.rs` | *(modify)* `DEFAULT_FORMATS`, artefact resolution, `strip_known_extension` |
| `scripts/readbench_prepare.sh` | *(modify)* `--formats` selection, conditional guards, new conversion steps |
| `justfile` | *(modify)* `FORMATS` param, `ordering-bench`, find/strip patterns |
| `bench/plot/readbench_plot/{plot,sizes}.py` | *(modify)* shared order/colours, GML discovery and sizing |
| `scripts/fetch_benchmark.sh` | *(modify)* catalogue corpus |
| `bench/READ_BENCHMARK.md`, `bench/CORPUS_REPORT.md`, `bench/README.md` | *(modify)* formats table, caveats, reproduce blocks |

---

### Task 1: Repair the three-way CSV header contract

**Files:**
- Modify: `scripts/readbench_duckdb.sh` (`CSV_HEADER` ~line 126; `append_row` ~lines 316-323 and its 11 call sites)
- Modify: `bench/plot/readbench_plot/__init__.py` (`CSV_HEADER`, lines 8-20)
- Modify: `bench/plot/readbench_plot/plot.py` (the strict header gate, line 80)
- Test: `bench/plot/tests/test_csv_contract.py` **(new)**

**Interfaces:**
- Consumes: nothing.
- Produces: one 13-column contract — `dataset,format,scenario,selectivity,result_count,time_s,time_mad_s,peak_heap_bytes,peak_rss_bytes,repeat,notes,bytes_read,http_requests` — shared by the coordinator (already correct), the DuckDB baseline script, and the plotter.

**Why first:** the coordinator writes 13 columns; the DuckDB script and the plotter both expect 11. So `readbench_duckdb.sh` hard-fails against any CSV the coordinator just wrote (which is what `justfile:167/170` does), and `plot.py:80`'s `if header != CSV_HEADER: return None` silently skips **every** coordinator CSV. `just bench` cannot currently complete. Nothing else in this plan can be verified end-to-end until this is fixed.

- [ ] **Step 1: Write the failing test**

`bench/plot` has no test suite today. Create one. Add to `bench/plot/pyproject.toml` a dev extra with `pytest`, then create `bench/plot/tests/test_csv_contract.py`:

```python
"""The plotter's CSV contract must match what the coordinator actually writes.

These drifted apart once already: the coordinator grew `bytes_read` and
`http_requests` for the HTTP transport, and neither the plotter nor the DuckDB
baseline script followed. The plotter's gate is a strict equality check, so the
symptom was silent - every coordinator CSV was skipped as "not a
read-benchmark CSV" and the charts came out empty rather than wrong.
"""

import re
from pathlib import Path

from readbench_plot import CSV_HEADER

REPO = Path(__file__).resolve().parents[3]


def _coordinator_header() -> list[str]:
    """The header literal the Rust coordinator writes, read from its source."""
    src = (REPO / "crates/cityparquet-readbench/src/coordinator.rs").read_text()
    m = re.search(r'const CSV_HEADER: &str = "(.*?)";', src, re.S)
    assert m, "could not find CSV_HEADER in coordinator.rs"
    # The literal is line-continued with a trailing backslash; rejoin it.
    return m.group(1).replace("\\\n", "").strip().split(",")


def test_plotter_header_matches_the_coordinator():
    assert CSV_HEADER == _coordinator_header()


def test_duckdb_script_header_matches_the_coordinator():
    src = (REPO / "scripts/readbench_duckdb.sh").read_text()
    m = re.search(r'^CSV_HEADER=(?:"|\')(.+?)(?:"|\')$', src, re.M)
    assert m, "could not find CSV_HEADER in readbench_duckdb.sh"
    assert m.group(1).split(",") == _coordinator_header()
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cd bench/plot && uv run --extra dev pytest tests/test_csv_contract.py -v`
Expected: both tests FAIL — the plotter and the shell script each report 11 columns against the coordinator's 13.

- [ ] **Step 3: Fix the plotter contract**

In `bench/plot/readbench_plot/__init__.py`, append `"bytes_read"` and `"http_requests"` to `CSV_HEADER`.

In `plot.py:80`, relax the gate so a future column addition degrades rather than blanking every chart:

```python
    # Prefix check, not equality: the coordinator may add trailing columns
    # (it added bytes_read/http_requests for the HTTP transport). A strict
    # `!=` silently skipped every CSV and produced empty charts - a wrong
    # picture is worse than a loud failure here, because the charts go in the
    # paper.
    if header[: len(CSV_HEADER)] != CSV_HEADER:
        return None
```

- [ ] **Step 4: Fix the DuckDB baseline script**

In `scripts/readbench_duckdb.sh`: extend `CSV_HEADER` to the 13 columns, extend `append_row`'s `printf` from 11 to 13 `%s` fields, and give the two new trailing fields empty defaults so the 11 existing call sites need no change. Prefer a default-parameter approach over editing all 11 sites:

```bash
# bytes_read/http_requests are always empty for this baseline: DuckDB runs
# out-of-process over a local file, so there is no HTTP tally to report. The
# columns exist so every row in the CSV has the coordinator's shape.
append_row() {
  printf '%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s\n' \
    "$1" "$2" "$3" "$4" "$5" "$6" "$7" "$8" "$9" "${10}" "${11}" "" ""
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cd bench/plot && uv run --extra dev pytest tests/ -v`
Expected: PASS (2 tests)

- [ ] **Step 6: Prove the pipeline actually runs end to end**

This is the point of the task. Using the smallest fixture available:

```bash
cargo build --release -p cityparquet-cli
# Pick any small CityJSONSeq fixture, e.g. tests/fixtures/delft.city.jsonl
./scripts/readbench_prepare.sh tests/fixtures/delft.city.jsonl /tmp/rb-prep
cargo run --release -p cityparquet-readbench -- run \
  --input tests/fixtures/delft.city.jsonl --prepared-dir /tmp/rb-prep \
  --out /tmp/rb.csv --repeat 1 --formats cityparquet
./scripts/readbench_duckdb.sh /tmp/rb-prep/delft.parquet /tmp/rb.csv --repeat 1
uv run --project bench/plot python -m readbench_plot "$(dirname /tmp/rb.csv)"
```
Expected: the DuckDB script appends without the header error, and the plotter emits PNGs rather than skipping the CSV.

**`fcb` is not installed on this machine, so `readbench_prepare.sh` will hard-fail at its line 43 guard before building anything.** Making that guard conditional is Task 6's job, not this one. For this task, skip the prepare script entirely and build just the one artefact you need by hand:

```bash
mkdir -p /tmp/rb-prep
./target/release/cityparquet convert tests/fixtures/delft.city.jsonl \
    -o /tmp/rb-prep/delft.parquet --overwrite
```
then run the coordinator with `--formats cityparquet` as above. One format is enough to prove the CSV contract end to end, which is all this task claims.

- [ ] **Step 7: Commit**

```bash
git add scripts/readbench_duckdb.sh bench/plot/
git commit -m "fix(bench): one CSV contract, not three

The coordinator writes 13 columns; the DuckDB baseline script and the
plotter both expected 11. The script hard-failed against any CSV the
coordinator wrote, and the plotter's strict equality gate silently skipped
every one - empty charts rather than a loud failure. The plotter's gate is
now a prefix check so the next column addition degrades instead."
```

---

### Task 2: Introduce the `Format` enum

**Files:**
- Create: `crates/cityparquet-readbench/src/format.rs`
- Modify: `crates/cityparquet-readbench/src/lib.rs` (add `pub mod format;`)
- Modify: `crates/cityparquet-readbench/src/formats/mod.rs` (`resolve` takes `Format`)
- Modify: `crates/cityparquet-readbench/src/coordinator.rs` (`DEFAULT_FORMATS`, `resolve_format_artefact`)
- Modify: `crates/cityparquet-readbench/src/main.rs` (`--format`/`--formats` parsing and doc comments)
- Test: `crates/cityparquet-readbench/tests/format_enum.rs`

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `pub enum Format { CityGml, CityJson, CityJsonSeq, CityJsonSeqGz, FlatCityBuf, CityParquet, CityParquetHilbert, DuckDbParquet }`
  - `Format::ALL: [Format; 8]`, `Format::as_str(self) -> &'static str`, `impl FromStr for Format`, `impl Display for Format`
  - `Format::is_child_format(self) -> bool` — false only for `DuckDbParquet`
  - `formats::resolve(format: Format) -> Result<Box<dyn FormatRunner>>`

**This is a pure refactor — no behaviour change.** Do it before adding runners: it turns "add a format" from six manual edit sites into exhaustive-match errors the compiler finds. Note `Scenario` (`src/scenario.rs:21-97`) is the model to mirror, right down to the `FromStr` error listing every variant.

- [ ] **Step 1: Write the failing test**

Create `crates/cityparquet-readbench/tests/format_enum.rs`:

```rust
//! `Format` is the vocabulary the whole harness shares: the `--format`
//! child dispatch, the coordinator's artefact naming, the CSV's `format`
//! column and the plotter's ordering all spell a format the same way.
//!
//! It exists as an enum rather than a `&str` because a format used to be
//! matched in three unrelated places plus two doc-comment lists plus a test
//! list, with no compiler help - so adding one was six edits and a hope.

use std::str::FromStr;

use cityparquet_readbench::format::Format;

#[test]
fn every_variant_round_trips_through_its_canonical_spelling() {
    for f in Format::ALL {
        assert_eq!(
            Format::from_str(f.as_str()).unwrap(),
            f,
            "{} did not round-trip",
            f.as_str()
        );
    }
}

#[test]
fn the_canonical_spellings_are_the_documented_tags() {
    let names: Vec<&str> = Format::ALL.iter().map(|f| f.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "citygml",
            "cityjson",
            "cityjsonseq",
            "cityjsonseq-gz",
            "flatcitybuf",
            "cityparquet",
            "cityparquet-hilbert",
            "duckdb-parquet",
        ]
    );
}

#[test]
fn an_unknown_name_names_every_valid_one() {
    let err = Format::from_str("not-a-format").unwrap_err();
    for f in Format::ALL {
        assert!(err.contains(f.as_str()), "error should list {}", f.as_str());
    }
}

#[test]
fn duckdb_parquet_is_not_a_child_format() {
    // It is a SQL-engine baseline driven by scripts/readbench_duckdb.sh; the
    // --child path must refuse it rather than pretend to run it.
    assert!(!Format::DuckDbParquet.is_child_format());
    for f in Format::ALL {
        if f != Format::DuckDbParquet {
            assert!(f.is_child_format(), "{} should be a child format", f.as_str());
        }
    }
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p cityparquet-readbench --test format_enum`
Expected: FAIL — `unresolved import cityparquet_readbench::format`.

- [ ] **Step 3: Write the enum**

Create `crates/cityparquet-readbench/src/format.rs` mirroring `scenario.rs`'s shape exactly: `ALL` in canonical order, `as_str`, `Display` delegating to `as_str`, and `FromStr` whose error lists `Format::ALL`. Add `is_child_format`. Register the module in `src/lib.rs`.

Canonical order is the one asserted in Step 1: source formats first (citygml → cityjson → cityjsonseq → cityjsonseq-gz), then indexed/columnar (flatcitybuf → cityparquet → cityparquet-hilbert), then the engine baseline. That order is also what the plotter will use, so charts read left-to-right from "what the data ships as" to "what we propose".

- [ ] **Step 4: Move artefact naming onto the enum**

Add to `format.rs`:

```rust
    /// Where this format's artefact lives, relative to `prepared_dir`.
    ///
    /// Three cases, deliberately distinguished — conflating the last two is
    /// how the old string-matching version got confusing:
    /// - `Prepared(name)`: a file or directory the prepare script builds.
    /// - `TheInputItself`: `CityJsonSeq` reads the original `--input`, which
    ///   normally lives OUTSIDE `prepared_dir`.
    /// - `NotCoordinated`: `DuckDbParquet` is an SQL-engine baseline driven
    ///   by `scripts/readbench_duckdb.sh`, never by this coordinator.
    pub fn artefact(self, base: &str) -> Artefact {
        match self {
            Format::CityGml => Artefact::Prepared(format!("{base}.gml")),
            Format::CityJson => Artefact::Prepared(format!("{base}.city.json")),
            Format::CityJsonSeq => Artefact::TheInputItself,
            Format::CityJsonSeqGz => Artefact::Prepared(format!("{base}.jsonl.gz")),
            Format::FlatCityBuf => Artefact::Prepared(format!("{base}.fcb")),
            Format::CityParquet => Artefact::Prepared(format!("{base}.parquet")),
            Format::CityParquetHilbert => {
                Artefact::Prepared(format!("{base}-hilbert.parquet"))
            }
            Format::DuckDbParquet => Artefact::NotCoordinated,
        }
    }
```

with

```rust
pub enum Artefact {
    Prepared(String),
    TheInputItself,
    NotCoordinated,
}
```

Then rewrite `coordinator::resolve_format_artefact` in terms of `Artefact`. Its three arms map one-to-one onto the existing `ArtefactResolution` variants, and the `if format == "cityjsonseq"` string test at `coordinator.rs:474` — which decides the HTTP key — becomes a match on `Artefact::TheInputItself`, so the special case is exhaustive rather than a string comparison.

- [ ] **Step 5: Thread the enum through dispatch and defaults**

- `formats::resolve` takes `Format`; its match becomes exhaustive (no `other =>` arm, no hardcoded name list in an error string — `FromStr` owns that now). `DuckDbParquet` keeps its explicit `bail!`.
- `DEFAULT_FORMATS` becomes `[Format; N]`. **Keep today's five members for now** — changing the default set is Task 5, and mixing a refactor with a behaviour change makes both harder to review.
- `main.rs`: `--format` parses to `Format` via `FromStr` (clap `value_parser`), `--formats` to `Vec<Format>`. Delete the two hand-maintained format lists in the doc comments at `main.rs:40-41` and `135-137` and point them at the enum instead — they cannot then drift.
- Update the in-crate unit test at `formats/mod.rs:96-103`.

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p cityparquet-readbench`
Expected: PASS, including the existing `tests/coordinator.rs` suite unchanged — this task changes no behaviour.

- [ ] **Step 7: Verify and commit**

Run: `just check`
Expected: exit 0.

```bash
git add crates/cityparquet-readbench/
git commit -m "refactor(readbench): make Format an enum, not a string

A format was matched in three unrelated places plus two doc-comment lists
plus a test list, with no compiler help, while Scenario beside it was a
proper enum. Adding a format is now an exhaustive-match error the compiler
finds, and --formats validates instead of silently skipping typos."
```

---

### Task 3: The `cityjson` runner (plain document)

**Files:**
- Create: `crates/cityparquet-readbench/src/formats/cityjson.rs`
- Modify: `crates/cityparquet-readbench/src/formats/mod.rs`
- Test: `crates/cityparquet-readbench/tests/cityjson_runner.rs`

**Interfaces:**
- Consumes: `Format` (Task 2), `FormatRunner`, `Source`, `Scenario`, `QueryParams`, `RunOutcome`.
- Produces: `pub struct CityJsonRunner;` implementing `FormatRunner` for all 7 scenarios × `Source::Local` and `Source::Http`.

**Not an alias.** `CityJsonSeqRunner` is line-oriented (`BufReader::lines`, `src/formats/cityjsonseq.rs:37,154`); a plain `.city.json` is a single JSON object with a `CityObjects` map and a shared `vertices` array. The parse shape differs, so this is its own runner — unlike `cityparquet-hilbert`, which genuinely is an alias.

**Counting semantics to disclose in the module doc:** a CityJSON document's natural unit is a `CityObjects` map entry, which includes second-level objects (`BuildingPart`, `BuildingInstallation`). State this explicitly — `Count`/`FullRead` are not necessarily comparable across formats, and every existing runner discloses its own grain (see `cityjsonseq.rs`'s doc comment for the house pattern).

- [ ] **Step 1: Write the failing test**

Mirror `tests/cityjsonseq_runner.rs`'s structure. The fixture is a real CityJSON document — `tests/fixtures/lod3_railway.city.json` is already fetched by `just fixtures`. Cover: `Count` returns the object-map size; `FullRead` decodes every geometry; `BBoxQuery` returns only objects intersecting the window; `IdLookup` finds a known id and returns 0 for an absent one; `AttrFilter`/`AttrStats`/`Project` over a real attribute. Assert the counting grain explicitly in at least one test so the disclosure is enforced, not just documented.

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p cityparquet-readbench --test cityjson_runner`
Expected: FAIL — module does not exist.

- [ ] **Step 3: Implement the runner**

Parse the document once per `run` call (the `--child` protocol spawns a fresh process per measurement, so no caching is needed or wanted — see the `FormatRunner` doc comment). Resolve geometry against the document-level `vertices` and `transform`. For `Source::Http`, fetch the whole object — a plain CityJSON document has no index, so a range request buys nothing; report `IoStats` accordingly.

- [ ] **Step 4: Register it**

Add `pub mod cityjson;` and the `Format::CityJson => Ok(Box::new(cityjson::CityJsonRunner))` arm. The compiler will point at the arm if you forget (that is Task 2's payoff).

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p cityparquet-readbench --test cityjson_runner`
Expected: PASS

- [ ] **Step 6: Add the HTTP parity test**

Mirror `tests/cityjsonseq_http_runner.rs` against the in-process server the existing HTTP tests use. Assert byte and request counts are reported.

- [ ] **Step 7: Verify and commit**

Run: `just check`

```bash
git add crates/cityparquet-readbench/
git commit -m "feat(readbench): plain CityJSON document runner"
```

---

### Task 4: The `citygml` runner

**Files:**
- Create: `crates/cityparquet-readbench/src/formats/citygml.rs`
- Modify: `crates/cityparquet-readbench/src/formats/mod.rs`
- Test: `crates/cityparquet-readbench/tests/citygml_runner.rs`

No `Cargo.toml` change is needed: `cityparquet-readbench` already depends on `cityparquet` (`Cargo.toml:15`), which carries the CityGML 2.0 reader.

**Interfaces:**
- Consumes: as Task 3, plus `cityparquet::citygml` (this repo's CityGML 2.0 reader).
- Produces: `pub struct CityGmlRunner;` implementing `FormatRunner`.

**What this measures, stated plainly in the module doc:** CityGML has no index, so every scenario is a full parse plus an in-memory filter. That is the honest baseline and the reason for including it — the row answers "what does it cost to answer this query from the format the data ships in, using the same codebase as every other row". It is **not** a claim about the format's theoretical ceiling; a different parser would give different numbers.

Fixtures already present via `just fixtures`: `tests/fixtures/b1_lod2_s.gml`, `b1_lod2_cs_w_sem.gml`, `berlin_citygml1.gml` (CityGML **1.0** — useful for asserting the version refusal), `freiburg_no_preamble_srs.gml`.

- [ ] **Step 1: Write the failing test**

Mirror `tests/citygml_runner.rs` on the two existing runner test files. Cover the 7 scenarios against `b1_lod2_cs_w_sem.gml`, plus: an unsupported-version input (`berlin_citygml1.gml`) must produce a clear error rather than a wrong number, and the counting grain must be asserted.

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p cityparquet-readbench --test citygml_runner`
Expected: FAIL — module does not exist.

- [ ] **Step 3: Implement the runner**

Stream via `cityparquet::citygml`'s reader. For `Source::Http`, fetch the whole file (no index). Do not attempt to reuse `cityparquet convert` — the runner measures reading CityGML, not converting it.

- [ ] **Step 4: Register, test HTTP parity, verify**

As Task 3 Steps 4-6.

- [ ] **Step 5: Commit**

```bash
git add crates/cityparquet-readbench/
git commit -m "feat(readbench): CityGML runner - the format the data ships in

No index exists, so every scenario is a full parse plus an in-memory
filter. That is the point: the row shows what a query costs against the
source format, using the same codebase as every other row."
```

---

### Task 5: The two benchmark sets

**Files:**
- Modify: `crates/cityparquet-readbench/src/coordinator.rs` (`DEFAULT_FORMATS`)
- Modify: `crates/cityparquet-readbench/tests/coordinator.rs` (row-count arithmetic)
- Test: `crates/cityparquet-readbench/tests/format_sets.rs`

**Interfaces:**
- Consumes: `Format` (Task 2), both new runners (Tasks 3-4).
- Produces: `DEFAULT_FORMATS` = the format-comparison set; `Format::ORDERING_SET` = `[CityParquet, CityParquetHilbert]` as a named const the justfile recipe and docs refer to.

**Why the default changes:** measuring format comparison and CityParquet-ordering comparison in one run confounds both. The default set carries one tag per format family, with CityParquet represented by its **best** configuration (`cityparquet-hilbert`) so the format comparison is not handicapped by an ordering choice. `cityjsonseq-gz` (a compression variant) and `duckdb-parquet` (an SQL-engine baseline) become opt-in — neither is a format.

- [ ] **Step 1: Write the failing test**

`crates/cityparquet-readbench/tests/format_sets.rs`:

```rust
//! The benchmark has two axes and measures them separately: which FORMAT,
//! and (for CityParquet) which ORDERING. A run that varies both at once
//! answers neither question cleanly.

use cityparquet_readbench::format::Format;

#[test]
fn the_default_set_is_one_tag_per_format_family() {
    assert_eq!(
        cityparquet_readbench::coordinator::DEFAULT_FORMATS,
        [
            Format::CityGml,
            Format::CityJson,
            Format::CityJsonSeq,
            Format::FlatCityBuf,
            Format::CityParquetHilbert,
        ]
    );
}

#[test]
fn cityparquet_is_represented_by_its_best_configuration() {
    // Hilbert ordering is the configuration we would ship, so it is the one
    // the format comparison should carry - otherwise CityParquet is
    // handicapped by an ordering choice the other formats never face.
    let d = cityparquet_readbench::coordinator::DEFAULT_FORMATS;
    assert!(d.contains(&Format::CityParquetHilbert));
    assert!(!d.contains(&Format::CityParquet));
}

#[test]
fn the_ordering_set_isolates_the_sort_strategy() {
    assert_eq!(
        Format::ORDERING_SET,
        [Format::CityParquet, Format::CityParquetHilbert]
    );
}

#[test]
fn compression_and_engine_baselines_are_opt_in() {
    let d = cityparquet_readbench::coordinator::DEFAULT_FORMATS;
    assert!(!d.contains(&Format::CityJsonSeqGz), "a compression variant is not a format");
    assert!(!d.contains(&Format::DuckDbParquet), "an engine baseline is not a format");
}
```

`DEFAULT_FORMATS` must become `pub` for this test.

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p cityparquet-readbench --test format_sets`
Expected: FAIL on the default-set assertion (it is still today's five).

- [ ] **Step 3: Change the sets, then fix the fallout**

Change `DEFAULT_FORMATS`; add `Format::ORDERING_SET`. Then run the whole suite: `tests/coordinator.rs:134-138` computes expected row counts as `2 formats × (1 count + 3 bbox) = 8` and similar, so several assertions will break. **Update the arithmetic to match the new default — do not pin the tests to the old set to keep them green.**

- [ ] **Step 4: Run the whole suite**

Run: `cargo test -p cityparquet-readbench`
Expected: PASS

- [ ] **Step 5: Verify and commit**

Run: `just check`

```bash
git add crates/cityparquet-readbench/
git commit -m "feat(readbench): two single-axis benchmark sets

Default is now one tag per format family with CityParquet represented by
its Hilbert-ordered configuration; ordering comparison moves to its own
named set. Measuring both axes in one run answered neither cleanly."
```

---

### Task 6: Format selection in `readbench_prepare.sh`

**Files:**
- Modify: `scripts/readbench_prepare.sh`
- Test: `scripts/tests/test_readbench_prepare.bats` **(new)** — or, if `bats` is unavailable, a plain `scripts/tests/readbench_prepare_test.sh` returning non-zero on failure. State which you used in the report.

**Interfaces:**
- Consumes: nothing.
- Produces: `readbench_prepare.sh [--formats a,b,c] INPUT [OUTDIR]` — builds only the requested artefacts; defaults to the format-comparison set.

**Two behaviours matter more than the flag:**
1. **Binary guards become conditional.** Today `command -v fcb` at lines 43-46 hard-fails unconditionally, so a missing `fcb` kills a run that never asked for FlatCityBuf. `fcb` is in fact **not installed on this machine**, which is why the read benchmark cannot run at all today. Guard each tool only when a format needing it was requested.
2. **A requested format that cannot be built must fail loudly**, not be silently skipped. Skipping is the coordinator's job (it warns and continues); the prepare script's job is to build what was asked for or say why it could not.

- [ ] **Step 1: Write the failing test**

Cover: `--formats cityparquet` builds only `<base>.parquet` and does **not** require `fcb`; `--formats flatcitybuf` without `fcb` on PATH fails with a message naming `fcb`; an unknown format name is rejected listing the valid ones; the default with no flag builds the format-comparison set. Stub the external binaries via a `PATH` shim directory so the test needs neither `fcb` nor Java.

- [ ] **Step 2: Run it to verify it fails**

Expected: FAIL — the script takes no `--formats` argument.

- [ ] **Step 3: Implement**

Add argument parsing before the positional `INPUT [OUTDIR]` (usage block at lines 26-31). Make each build block at lines 84-121 conditional on membership. Move each `command -v` guard next to the block that needs it. Update the header comment (lines 4-10) and the final summary `echo` (line 149) to reflect what was actually built.

- [ ] **Step 4: Run the tests, verify, commit**

```bash
git add scripts/
git commit -m "feat(bench): --formats selection in readbench_prepare

Binary guards are now conditional: a missing fcb no longer kills a run that
did not ask for FlatCityBuf. A requested-but-unbuildable format still fails
loudly - silently skipping is the coordinator's job, not this script's."
```

---

### Task 7: The CityGML → CityJSON → CityJSONSeq conversion chain

**Files:**
- Modify: `scripts/readbench_prepare.sh`
- Modify: `scripts/fetch_tools.sh` **(new)** — fetch and pin citygml-tools
- Modify: `justfile` (a `fetch-tools` recipe)

**Interfaces:**
- Consumes: Task 6's format selection.
- Produces: from a `.gml` input, the full artefact set; from a `.city.jsonl` input, everything except `citygml` (documented, not synthesised).

**The chain, and why it runs this way:**

```
CityGML ──citygml-tools to-cityjson──> CityJSON ──cjseq──> CityJSONSeq
                                                    ├──fcb ser -A──> FlatCityBuf
                                                    └──cityparquet convert──> CityParquet
```

Each artefact derives from the one before. FlatCityBuf and CityParquet both derive from the **same** CityJSONSeq — that is what makes their comparison fair. **Nothing derives from CityParquet**: `cityparquet export` could emit the CityJSON artefacts, and it would be tempting, but deriving a competitor's input from the format under test would favour it.

- [ ] **Step 1: Confirm the tools are usable**

The chain is settled — **citygml-tools cannot emit CityJSONSeq, so `cjseq` performs that hop.** Do not spend time re-investigating that; it is a decision, not an open question.

- `citygml-tools` — `to-cityjson`, Java 17+ (Java 21 present). Fetched by Step 2.
- `cjseq` — CityJSON → CityJSONSeq (`cjseq cat`). Install with `cargo install cjseq` if absent, and pin the version in `scripts/fetch_tools.sh` alongside citygml-tools so the corpus is reproducible.

Verify both run (`citygml-tools --version`, `cjseq --help`) and record the exact versions in your report — they are part of the measurement's provenance and belong in `bench/READ_BENCHMARK.md`'s Environment block.

- [ ] **Step 2: Write `scripts/fetch_tools.sh`**

Fetch citygml-tools' release archive to a gitignored `bench/tools/`, pinned by version **and verified by sha256** — follow `scripts/fetch_3dbag.sh`, which already pins sha256 and retries once before hard-failing. Skip if already present and valid. Fail with a clear message if `java` is absent or below 17.

- [ ] **Step 3: Write the failing test**

Extend Task 6's test file: a `.gml` input with the default format set produces all five artefacts; a `.city.jsonl` input produces four and reports plainly that `citygml` was not derivable. Stub the tools on `PATH` so the test stays offline and fast.

- [ ] **Step 4: Implement the chain**

Add the conversion steps to `readbench_prepare.sh`, keeping its existing idempotency (`dir_is_valid`/`file_is_valid` at lines 63-68) and its post-build verification block (lines 128-147) — extend the latter to the new artefacts, including a non-zero object count, mirroring the `fcb info` "Features: N > 0" check.

- [ ] **Step 5: Prove it on real data**

Use a small single-family CityGML fixture (`tests/fixtures/b1_lod2_cs_w_sem.gml`). Confirm every artefact appears and the coordinator runs the full default format set against it.

- [ ] **Step 6: Commit**

```bash
git add scripts/ justfile
git commit -m "feat(bench): forward conversion chain from CityGML

Artefacts derive forward from the source and never through CityParquet:
FlatCityBuf and CityParquet both come from the same CityJSONSeq, which is
what makes their comparison fair."
```

---

### Task 8: Justfile wiring and the extension conventions

**Files:**
- Modify: `justfile` (four `find` patterns, four basename-strippers, `bench` recipe, new `ordering-bench`)
- Modify: `crates/cityparquet-readbench/src/coordinator.rs` (`strip_known_extension`, lines 422-429)
- Modify: `scripts/readbench_prepare.sh` (basename stripping, lines 50-52)
- Modify: `scripts/readbench_duckdb.sh` (basename stripping, lines 193-195)
- Test: `crates/cityparquet-readbench/tests/strip_extension.rs`

**Interfaces:**
- Consumes: Tasks 5-7.
- Produces: `just bench FOLDER OUT FORMATS`, `just ordering-bench FOLDER OUT`; `.gml`/`.xml` recognised everywhere `.json`/`.jsonl` is.

**The lockstep hazard:** basename stripping is duplicated in **four** places (Rust + three shell) and the input-discovery `find` pattern in **four** more. A `.gml` input currently yields `name=foo.gml`, so every downstream artefact path is wrong. `coordinator.rs:422-429`'s own comment claims lockstep with the others — make that true and test it.

- [ ] **Step 1: Write the failing test**

`tests/strip_extension.rs` asserting `strip_known_extension` handles `.city.jsonl`, `.city.json`, `.jsonl`, `.json`, `.gml`, `.xml`, and leaves an unknown extension alone. Then a shell-level check that the justfile and both scripts strip identically — a small table-driven script comparing all four implementations on the same inputs is the honest way to enforce a convention duplicated four times.

- [ ] **Step 2: Run it, implement, re-run**

Add `.gml`/`.xml` to all four strippers and all four `find` patterns. **Append** `FORMATS` to the `bench` recipe's parameters rather than inserting it — `just` params are positional-with-defaults, so inserting before `OUT` silently breaks existing callers. Add `ordering-bench` passing `Format::ORDERING_SET`'s members.

- [ ] **Step 3: Verify and commit**

Run: `just --list` (must parse), `just check`.

```bash
git add justfile crates/ scripts/
git commit -m "feat(bench): recognise CityGML inputs; format selection through just

The find pattern and basename stripping were each duplicated four times and
matched only .json/.jsonl, so a .gml input was invisible to every recipe and
its artefact paths were wrong. Now tested against all four implementations."
```

---

### Task 9: Plotting

**Files:**
- Modify: `bench/plot/readbench_plot/__init__.py` (shared `FORMAT_ORDER`, `FORMAT_COLORS`)
- Modify: `bench/plot/readbench_plot/plot.py` (use the shared maps; pass `color=`)
- Modify: `bench/plot/readbench_plot/sizes.py` (discovery, per-format measurement, GML raw-size baseline)
- Test: `bench/plot/tests/test_formats.py`

**Interfaces:**
- Consumes: Task 2's canonical order.
- Produces: one `FORMAT_ORDER` and one `FORMAT_COLORS` in `__init__.py`, imported by both modules.

**Three real defects here, not just new names:**
1. `FORMAT_ORDER` exists **twice** and disagrees — `plot.py:54-61` has six entries including `duckdb-parquet`, `sizes.py:46-52` has five without it.
2. `plot.py` has **no colour map at all**; bars take matplotlib's default cycle by draw order, so **adding a format silently recolours every published chart**. The "consistent colours across figures" claim at `sizes.py:55-58` is already false.
3. `sizes.py` discovers datasets only from `*.fcb`/`*.jsonl.gz` siblings (`discover_datasets`, 87-99) and computes `ratio_vs_cityjsonseq` from the gzip ISIZE trailer (140-145). A GML-native dataset has neither, so it is invisible and its ratio is uncomputable.

- [ ] **Step 1: Write the failing test**

Assert: both modules import the same `FORMAT_ORDER` object; every `Format::ALL` tag has a colour; a colour map is stable when a format is added (pin the mapping by name, not by index); `discover_datasets` finds a dataset whose only artefacts are `.gml`/`.city.json`; `ratio_vs_cityjsonseq` is computed for a GML-native dataset. Read the canonical tag list from `format.rs` the same way Task 1's test reads the CSV header, so the Python and Rust vocabularies cannot drift.

- [ ] **Step 2: Implement**

Centralise both maps in `__init__.py`, keyed **by name** so adding a format never shifts an existing one's colour. Add per-format measurement blocks in `sizes.py` (directory vs file — use the existing `dir_size` helper at line 82). Replace the gzip-ISIZE baseline with the actual source file's size for GML-native inputs.

- [ ] **Step 3: Run, verify, commit**

Run: `cd bench/plot && uv run --extra dev pytest -v`

```bash
git add bench/plot/
git commit -m "fix(plot): one format vocabulary, stable colours

FORMAT_ORDER existed twice and disagreed; plot.py had no colour map at all,
so adding a format recoloured every published chart. Colours are now keyed
by name, and a GML-native dataset is discoverable and measurable."
```

---

### Task 10: Re-point the corpus

**Files:**
- Modify: `scripts/fetch_benchmark.sh`
- Modify: `justfile` (`fetch-data` comment)
- Test: `scripts/tests/` (extend Task 6's harness)

**Interfaces:**
- Consumes: `bench/catalogue_benchmark_urls.txt` (38 verified URLs, committed at `47aac62`).
- Produces: a curated single-family CityGML/CityJSON corpus under `bench/data/benchmark/`, normalised (gunzipped, unzipped), byte-size verified.

**Preserve the existing script's virtues** — a pinned `local_name | bytes | url` table, size verification with a hard fail on mismatch, and idempotent skip-if-present. A pinned local name is required regardless: **11 of the 38 URLs carry no derivable filename** (CRAIG and Estonia download endpoints with query strings). Swap `gsutil` for `curl`, dropping that dependency.

**Curate to single-family inputs** — the harness rejects multi-table packages (`coordinator.rs:508-544`, `readbench_duckdb.sh:187`). PLATEAU's per-module tiles already qualify (a `bldg` tile is Building only); the whole-city ZIP bundles do not. **Verify family count per dataset rather than assuming** — convert each candidate and check the package has exactly one object table. Record which datasets you dropped and why.

Keep the old `gs://` corpus behind a separate recipe for the ordering benchmark and for continuity with published results.

- [ ] **Step 1: Write the failing test** — size mismatch hard-fails; an already-present valid file is skipped; `.gz`/`.zip` are normalised to plain files.
- [ ] **Step 2: Implement, verify against a small real subset, commit.**

---

### Task 11: Documentation

**Files:**
- Modify: `bench/READ_BENCHMARK.md` (Formats table lines 24-31; fairness caveats 198+; Reproduce block 367-379)
- Modify: `bench/CORPUS_REPORT.md`, `bench/README.md`

- [ ] **Step 1: Formats table** — a row per tag, with whether an index exists.
- [ ] **Step 2: The three fairness caveats** from the spec §6, alongside the existing counting-granularity disclosures.
- [ ] **Step 3: Fix the documented-but-missing recipes.** `READ_BENCHMARK.md:369-379` documents `just readbench-prepare` and `just readbench-all`; **neither exists**. Either add them or correct the docs — say which you did.
- [ ] **Step 4: Mark the invalidated results.** The tables in all three files predate both the corpus change and the new format columns. State plainly that they need a re-run before being quoted; do not silently leave stale numbers next to new prose.
- [ ] **Step 5: Commit.**

---

## Post-implementation

Not a code task: re-run the benchmark on the new corpus and refresh the results tables. `just bench bench/data/benchmark` with the default format set, then `just ordering-bench`. The numbers in `bench/README.md`, `bench/CORPUS_REPORT.md` and `bench/READ_BENCHMARK.md` are only quotable after that.
