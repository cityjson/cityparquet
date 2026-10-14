---
name: testing-cityparquet-toolchain
description: Use when asked for extensive, end-to-end, interoperability or lossless round-trip testing of the CityParquet tool chain (cityparquet-rs, duckdb-cityjson, duckdb-3d) on real datasets, before a release, or after a change to any reader, writer or the encoding.
---

# Testing the CityParquet tool chain on real data

`test/toolchain/` pushes every corpus dataset through every writer and reader
and compares what comes back with the source. The comparator
(`cjcompare.py`) shares no code with any tool under test, so a tool cannot
pass by agreeing with itself. Your job is to run it, find whose bug each
failure is, and report. Never make a run greener by changing what is checked.

## 1. Get the tools

| Variable             | Default (in this tree)                                                           | How to get it                                                       |
| -------------------- | -------------------------------------------------------------------------------- | ------------------------------------------------------------------- |
| `CITYPARQUET`        | `lib/cityparquet-rs/target/release/cityparquet`                                  | `cd lib/cityparquet-rs && cargo build --release -p cityparquet-cli` |
| `CITYJSON_EXTENSION` | `lib/duckdb-cityjson/build/release/extension/cityjson/cityjson.duckdb_extension` | `just -f lib/duckdb-cityjson/justfile build`                        |
| `THREE_D_EXTENSION`  | `lib/duckdb-3d/build/release/extension/three_d/three_d.duckdb_extension`         | `make -C lib/duckdb-3d`                                             |
| `CITYGML_TOOLS`      | `benchmark/formats/tools/citygml-tools/citygml-tools`                            | `just fetch-tools` (needs Java 17+)                                 |
| `TOOLCHAIN_WORK`     | `test/runs/toolchain` (ignored)                                                  | any directory with a few GB free                                    |

Test what the user asked about. The submodules pin a commit that may be
older than their `develop`; build the commit you mean to test and say which
in the report. A missing tool skips its steps, and the report lists them.

## 2. Run

```sh
uv run --python 3.12 --with duckdb==1.5.5 --with flatcitybuf==0.5.2 --with pytest \
  pytest -q test/toolchain/test_cjcompare.py          # the comparator itself first
alias tc='uv run --python 3.12 --with duckdb==1.5.5 --with flatcitybuf==0.5.2 test/toolchain/run.py'
tc fetch --profile quick        # pinned by sha256; a mismatch stops, it never refetches
tc run --profile quick          # writes run-<time>/results.jsonl and report.md
tc run --only rotterdam,delft-fcb --keep   # rerun a few, keep their outputs
tc report RUN_DIR               # rewrite report.md from results.jsonl
```

`quick` takes about ten minutes on four cores; `full` adds the large city
models and `xl` the whole Helsinki CityJSONSeq. DuckDB is pinned to the
version the extensions target.

## 3. Read the statuses

| Status     | Meaning                                                                                                                                                                                                                                                                         |
| ---------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `pass`     | Equal as a model.                                                                                                                                                                                                                                                               |
| `admitted` | Equal up to a loss the specification allows: a row of the round-trip table in `01-dataset-package.mdx`, a ring of fewer than three distinct positions (`03-geometry-semantics`), or a date-time respelled as the same UTC instant (`02-object-table-schema`, Temporal columns). |
| `noted`    | A difference no rule covers but which changes no content (two writers' column choices, a rotated ring start). Listed for a reader to judge.                                                                                                                                     |
| `known`    | Every failure line in the step matches `known.toml`.                                                                                                                                                                                                                            |
| `fail`     | Anything else.                                                                                                                                                                                                                                                                  |

## 4. Find whose bug it is

The step name says which tool produced the output:

| Failing step                                                | Look at                                                                      |
| ----------------------------------------------------------- | ---------------------------------------------------------------------------- |
| `rs convert`, `rs export`, `round trip rs`                  | cityparquet-rs reader or writer                                              |
| `dk write`, `dk export`, `round trip dk`                    | duckdb-cityjson (`duck.py` holds the exact SQL)                              |
| `cross dk package -> rs export` while both round trips pass | how cityparquet-rs reads the extension's package, or the reverse             |
| `packages agree (...)`                                      | the two writers disagree; the round trips say which is right                 |
| `three_d ...`                                               | duckdb-3d, or a package it cannot parse                                      |
| `round trip rs` on CityGML only                             | the GML writer: `gml census` counts elements, citygml-tools reads both sides |

Before calling it a tool bug, rule out the harness: reproduce with the
tool alone on the source file (a `cityparquet` command or one SQL
statement) and check the input is valid for its format. Read the
specification and the tool's own docs (`lib/duckdb-cityjson/docs/FUNCTIONS.md`)
before deciding a behaviour is wrong. A fixture that is not valid CityJSON
gets a repair step in `corpus.toml` (`derive = "fix:..."`), not a known
issue.

## 5. Rules that keep the result honest

- **Never loosen a check to pass a run.** A new admitted loss needs a rule in
  the specification first; cite it where the comparator admits it.
- **A comparator change needs a mutation test** in `test_cjcompare.py` that
  fails without the change, on real corpus data.
- **`known.toml` is for decided or tracked issues**, each with a reference.
  A failure nobody has triaged stays a failure.
- **Datasets are pinned.** Add one with its size and sha256, its profile,
  and what it exercises.

## 6. Report

`report.md` in the run directory has the verdict, versions, corpus, a
category matrix per round trip, every step per dataset, the failures and
the known issues. Deliver it with a findings list on top: one line per
distinct defect, the tool that owns it, the datasets that show it, and a
minimal reproduction. Group the many failures one defect causes under that
defect.
