# Format-comparison benchmark: CityGML, CityJSON, CityJSONSeq, FlatCityBuf, CityParquet

**Date:** 2026-08-16
**Status:** design, approved
**Scope:** add CityGML and plain CityJSON as measured formats; split the read
benchmark into two single-axis sets (format comparison, ordering comparison);
thread format selection through the whole harness; and re-point the corpus at
the catalogue-derived URL list.

---

## 1. Purpose

The read benchmark exists to answer one question for the paper: **how do 3D
city-model storage formats compare?** Today it measures five *tags* —
`cityparquet`, `cityparquet-hilbert`, `flatcitybuf`, `cityjsonseq`,
`cityjsonseq-gz` — which is really two questions tangled together: a format
comparison and a CityParquet-configuration comparison. Neither is answered
cleanly, and two formats a reader would expect (CityGML, the format the data
actually ships in; and plain CityJSON) are absent entirely.

This work separates the axes and completes the format list.

## 2. Findings from the exploration

Established by reading the code on `develop`, 2026-08-16.

### 2.1 The pipeline is broken today

Three CSV header contracts are meant to be one; two have drifted:

| Component | Columns | Location |
|---|---:|---|
| readbench coordinator (**writes**) | **13** — …`notes,bytes_read,http_requests` | `crates/cityparquet-readbench/src/coordinator.rs:134` |
| `readbench_duckdb.sh` (**appends**) | 11 — stops at `notes` | `scripts/readbench_duckdb.sh:126` |
| `bench/plot` (**reads**, strict `==`) | 11 — stops at `notes` | `bench/plot/readbench_plot/__init__.py:8-20` |

Consequences on `develop`: `readbench_duckdb.sh:198-209` hard-fails against any
CSV the coordinator just wrote — which is exactly what `justfile:167/170` does —
and `plot.py:80` silently skips **every** coordinator CSV as "not a
read-benchmark CSV". **`just bench` cannot currently complete.** This is a
prerequisite fix, not part of the format work.

### 2.2 `--formats` already exists — at one layer only

`RunArgs::formats` (`src/main.rs:138-139`, comma-delimited) → `RunOptions::formats`
(`coordinator.rs:84`) → resolved against `DEFAULT_FORMATS` (`coordinator.rs:111-117`).
Unknown or missing formats are skipped with a warning (`coordinator.rs:182-201`);
only an empty resolved set is fatal. Tests already cover it.

What does **not** exist: any way to pass a format list through
`readbench_prepare.sh` (which unconditionally builds all four artefacts and
hard-fails on a missing `fcb` at lines 43-46) or through the `justfile`'s
`bench` recipe.

### 2.3 Format is stringly-typed; Scenario is not

`Scenario` is a proper enum with `ALL`/`as_str`/`FromStr` (`src/scenario.rs:21-97`).
A format is a bare `&str` matched in three unrelated places —
`formats::resolve` (`src/formats/mod.rs:74-89`), `resolve_format_artefact`
(`coordinator.rs:453-493`) and `DEFAULT_FORMATS` — plus two doc-comment lists
(`main.rs:40-41`, `135-137`) and an explicit test list (`formats/mod.rs:96-103`).
Adding a format means six manual edits with no compiler help.

`cityparquet-hilbert` already demonstrates one runner serving two format tags
(`formats/mod.rs:76`), differing only in which artefact path resolves.

### 2.4 Two structural constraints

- **The CityParquet package is mandatory regardless of `--formats`.**
  `locate_cityparquet_table` (`coordinator.rs:162`) runs before format
  resolution, and every `QueryParams` — bbox windows, the attribute predicate,
  the sample id, the shared selectivity denominator (`coordinator.rs:239`) — is
  derived from it. `--formats citygml` alone still needs a converted package.
- **Single-family only.** `locate_cityparquet_table` (`coordinator.rs:508-544`)
  and `readbench_duckdb.sh:187` both hard-reject multi-table packages.

### 2.5 Duplicated conventions

The input-discovery pattern is copied verbatim four times (`justfile:98-100`,
`174-176`, `210-212`, `248-250`) and matches only `*.json`/`*.jsonl` — a CityGML
corpus is invisible to all four recipes. Basename stripping is duplicated in
four more places (`readbench_prepare.sh:50-52`, `coordinator.rs:422-429`,
`justfile` ×4 pairs, `readbench_duckdb.sh:193-195`) and strips no CityGML
extension, so a `foo.gml` input yields `name=foo.gml` and every downstream path
is wrong.

### 2.6 The plotter will silently mis-colour

`FORMAT_ORDER` exists twice and disagrees (`plot.py:54-61` has six names
including `duckdb-parquet`; `sizes.py:46-52` has five and omits it).
`plot.py` has **no** colour map at all — bars take matplotlib's default cycle by
draw order, so **adding a format shifts every existing format's colour**. The
"consistent colours across figures" claim at `sizes.py:55-58` is already false.
`sizes.py` also discovers datasets only from `*.fcb`/`*.jsonl.gz` siblings
(`discover_datasets`, 87-99) and computes `ratio_vs_cityjsonseq` from the gzip
ISIZE trailer (140-145) — both break for a GML-native input.

## 3. Decisions

| Decision | Choice | Rationale |
|---|---|---|
| Benchmark axes | **Two single-axis sets** | Format comparison and CityParquet-ordering comparison are different questions; measuring them together confounds both. |
| Format-set default | `citygml, cityjson, cityjsonseq, flatcitybuf, cityparquet-hilbert` | One tag per format family, CityParquet represented by its best configuration so the format comparison is not handicapped by an ordering choice. |
| Ordering set | `cityparquet, cityparquet-hilbert` | Isolates the sort strategy against an otherwise identical writer. |
| `cityjsonseq-gz`, `duckdb-parquet` | Opt-in | The first is a compression variant, the second an SQL-engine baseline. Neither is a format. |
| Artefact provenance | **Forward chain from the source, never through CityParquet** | `cityparquet export` could emit the CityJSON artefacts, but deriving a competitor's input from the format under test would favour it. |
| Format-comparison corpus | **CityGML-source, single-family** | Every artefact derives forward and honestly. The existing CityJSONSeq corpus would need `from-cityjson` to synthesise CityGML — a caveat, not a measurement. |
| Multi-family packages | **Curate the corpus, change no code** | PLATEAU's per-module tiles are already single-family (a `bldg` tile is Building only), so most of the new corpus qualifies. |
| Format representation | **Introduce a `Format` enum** | Turns six manual edit sites into exhaustive-match errors, and gives `--formats` real validation instead of silent-skip-on-unknown. |

## 4. Architecture

### 4.1 The conversion chain

```
CityGML ──citygml-tools to-cityjson──> CityJSON ──cjseq──> CityJSONSeq
                                                                │
                                                    ┌───────────┴───────────┐
                                                    │                       │
                                              fcb ser -A            cityparquet convert
                                                    │                    (+ --ordering hilbert)
                                              FlatCityBuf              CityParquet
```

Each artefact derives from the one before it. FlatCityBuf and CityParquet both
derive from the *same* CityJSONSeq, which is what makes their comparison fair.
Nothing derives from CityParquet.

`citygml-tools` (Java 17+; Java 21 present) provides `to-cityjson` and supports
CityJSON 2.0/1.1/1.0. Whether its `to-cityjson` can emit CityJSONSeq directly is
unverified — the plan verifies it and falls back to `cjseq` for that hop.

### 4.2 Benchmark sets

Implemented as **format lists**, not a new CLI concept:

- `DEFAULT_FORMATS` becomes the format-comparison set (§3).
- A new `just ordering-bench` recipe passes the ordering set explicitly via the
  `--formats` flag that already exists.

This keeps one selection mechanism. `--formats` remains an explicit list;
`--suite` is deliberately not introduced.

### 4.3 The `Format` enum

Mirrors `Scenario` (`scenario.rs:21-97`): `ALL`, `as_str`, `FromStr`, and the
artefact-path mapping moving onto the enum so `resolve_format_artefact`'s match
becomes exhaustive. `formats::resolve` keeps returning `Box<dyn FormatRunner>`;
only the dispatch key changes from `&str` to `Format`.

### 4.4 New runners

- **`citygml`** — reads via this repo's own CityGML reader
  (`crates/cityparquet/src/citygml`). No index exists, so every scenario is a
  full parse plus an in-memory filter. That is the honest baseline and the point
  of including it.
- **`cityjson`** — a plain `.city.json` document (single JSON object with a
  `CityObjects` map and shared `vertices`), distinct from the line-delimited
  `cityjsonseq`. Whether it can reuse `CityJsonSeqRunner` behind a second format
  tag (the `cityparquet-hilbert` trick) or needs its own runner is settled in
  the plan; the parse shape differs, so a separate runner is expected.

Both need `Source::Local` **and** `Source::Http` arms for parity with every
existing runner.

## 5. Corpus

`scripts/fetch_benchmark.sh` is re-pointed at the catalogue-derived list
(`bench/catalogue_benchmark_urls.txt`, 38 verified URLs), replacing the
`gs://cityjson/benchmark_dataset/` corpus and removing the `gsutil` dependency
in favour of `curl`.

- **Normalise on fetch**: gunzip `.gz`, extract `.zip`, so what lands on disk is
  plain `.gml` / `.city.json`.
- **Curate to single-family** inputs (§3), because the harness rejects
  multi-table packages.
- **Pinned `local_name | bytes | url` table**, preserving the existing script's
  virtues: byte-size verification with a hard fail on mismatch, and idempotent
  skip-if-present. 11 of the 38 URLs carry no derivable filename (CRAIG and
  Estonia download endpoints with query strings), so a pinned local name is
  required regardless.

The old CityJSONSeq corpus is retained under a separate recipe for the ordering
benchmark and for continuity with published read results.

## 6. Fairness caveats to document

These belong in `bench/READ_BENCHMARK.md` alongside the existing
counting-granularity disclosures, not in code comments:

1. **CityGML measures this repo's reader**, not the format's ceiling. A
   different parser would give different numbers; what the row shows is "what it
   costs to answer this query from the format the data ships in, using the same
   codebase as every other row".
2. **Conversion is lossy in one direction.** Every artefact below CityJSON in
   the chain inherits whatever `citygml-tools` chose; a CityGML row and a
   CityParquet row are not reading identical object sets unless the conversion
   was lossless, which is asserted per dataset rather than assumed.
3. **Selectivity denominators are grain-sensitive.** `total_count_for`
   (`coordinator.rs:911-914`) gives a per-format denominator for `BBoxQuery`,
   but the four object-level scenarios share the CityParquet denominator.
   CityGML's nested `cityObjectMember` hierarchy can produce a different natural
   grain; the self-consistency check (`coordinator.rs:398-412`) warns but never
   fails.

## 7. Change surface

Ordered by dependency. **P** = prerequisite.

| # | Area | Change |
|---|---|---|
| P1 | `scripts/readbench_duckdb.sh` | `CSV_HEADER` 11 → 13; `append_row` printf and its 11 call sites |
| P2 | `bench/plot/readbench_plot/__init__.py` | `CSV_HEADER` 11 → 13; relax `plot.py:80` from `==` to a prefix check so the next column addition does not silently blank every chart |
| A | `crates/cityparquet-readbench` | `Format` enum; `formats::resolve`; `resolve_format_artefact`; `DEFAULT_FORMATS`; two doc-comment lists; the enumerating unit test |
| B | same | `citygml` and `cityjson` runners, 7 scenarios × local/HTTP, with tests mirroring `cityjsonseq_runner.rs` / `flatcitybuf_runner.rs` and the `*_http_runner.rs` trio |
| C | `scripts/readbench_prepare.sh` | `--formats` selection; conditional artefact builds; **conditional** binary guards (a missing `fcb` must no longer kill a run that did not ask for it); citygml-tools and cjseq steps |
| D | `justfile` | `FORMATS` parameter on `bench`, appended so existing positional callers keep working; new `ordering-bench`; `.gml`/`.xml` in four `find` patterns; `.gml`/`.xml` in four basename-strippers |
| E | `coordinator.rs:422-429` | `strip_known_extension` — same extensions, kept in lockstep with D |
| F | `bench/plot` | Centralise `FORMAT_ORDER` + a shared `FORMAT_COLORS` in `__init__.py`; pass `color=` at `plot.py:132,193`; `sizes.py` discovery and per-format measure blocks; a raw-size source for GML-native inputs to replace the gzip-ISIZE baseline |
| G | `scripts/fetch_benchmark.sh` | §5 |
| H | Docs | `READ_BENCHMARK.md` formats table, fairness caveats, reproduce block; `CORPUS_REPORT.md`; `bench/README.md` |

## 8. Non-goals

- **The write/encoding benchmark (`cityparquet bench`) is untouched.** Its axis
  is writer configuration, not comparison formats; CityGML and CityJSON are not
  things `cityparquet convert` writes.
- **No new CLI concept.** Benchmark sets are format lists, not a `--suite` flag.
- **The multi-family restriction stays.** Lifting it would redefine scenario
  semantics and the selectivity denominator; the corpus is curated instead.
- **Re-running the published measurements** is downstream of this work. The
  results tables in `bench/README.md`, `bench/CORPUS_REPORT.md` and
  `bench/READ_BENCHMARK.md` are invalidated by both the corpus change and the
  new format columns, and need a re-run before they are quoted again.
