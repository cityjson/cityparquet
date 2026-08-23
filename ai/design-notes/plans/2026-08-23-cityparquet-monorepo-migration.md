# Migration plan: `cityparquet-paper` → public CityParquet monorepo

**Date:** 2026-08-23
**Status:** approved design, not yet executed
**Scope:** repository topology only. No behaviour of any library, extension or
benchmark changes; every code edit in this plan is a path fix or a file move.

---

## 1. Why

`cityparquet-paper` was created to hold one journal manuscript, and grew
submodules so that an assistant could see the whole stack while writing. It is
now the de facto home of the CityParquet project: the normative specification,
the reference implementation, three benchmark families, four submodules and the
paper all live in one private tree.

Two things need to be true that are not true today:

1. **CityParquet needs a public front door.** A specification with no citable,
   cloneable, contributable home is not an open format.
2. **The manuscript must stay private until submission** (~end of August 2026).

These conflict inside a single repository, and the conflict is in the *history*,
not just the working tree. `paper/`, `notes/` (zettel + memos), `references/`
and `.obsidian/` are tracked across all 197 commits. Publishing this repo
publishes every draft revision, and moving those directories out today would not
change that.

---

## 2. Standing constraint: break freely

**CityParquet has no users yet.** Nothing downstream depends on the current
repository layout, module paths, crate names, CLI flags, spec file organisation
or branch names. Every one of them is free to change, and this migration is the
cheapest moment there will ever be to change them.

The plan is therefore written with **no backward compatibility of any kind**:

- **No shims, aliases or forwarding stubs.** No re-export crates at old paths,
  no symlinks from moved directories, no redirect pages for moved docs, no
  deprecated CLI flags kept alive alongside their replacements.
- **No deprecation periods.** When something is replaced, the old thing is
  deleted in the same commit.
- **Nothing is left stale.** Every path reference, README, doc comment,
  `justfile` recipe, tutorial and CI job is updated to the new state as part of
  the move. A file that still describes the old layout is a defect, not
  a leftover — this is why §6 enumerates the reference sites and §7 gates on
  `git grep` returning nothing.
- **"It would churn a lot of links" is not an argument** against a change in
  this plan. Churn is the expected cost and it is being paid once.

The two exceptions, both outside the codebase: the archived GitHub repos keep a
pointer README (courtesy to anyone holding an old link — it costs nothing and
maintains nothing), and the paper repo keeps its committed figure snapshots so
the PDF renders without a submodule checkout.

---

## 3. Decisions taken

| # | Decision | Rationale |
|---|---|---|
| D1 | **Build a fresh public repo**; do not rewrite this one's history | `git filter-repo` over 197 commits with five submodule gitlinks is fiddly, and a mistake publishes draft manuscript irreversibly. Building fresh has no such failure mode. |
| D2 | `cityparquet-paper` **stays** and becomes the private paper repo | It already owns the paper's history. Zero extraction work. |
| D3 | Public repo lives at **`github.com/cityjson/cityparquet`** | Signals community ownership and sits beside `duckdb-cityjson` and `city3d-stac-tool`. |
| D4 | `cityparquet-rs` and `citylake` enter via **`git subtree add`** | Preserves 539 + 26 commits of history. `documents/` and `benchmarking/` are copied in as new commits; their old history remains readable in the private paper repo. |
| D5 | `lib/` children are named **after the artefact**, never after a language | Avoids slicing `lib/` on two axes at once. A future Python port is `lib/cityparquet-py/`, not `lib/*/python/`. |
| D6 | **Two independent Cargo workspaces** (`lib/cityparquet-rs`, `lib/citylake`) | citylake has no dependency on the `cityparquet` crate and pins its own DuckDB (`=1.10501.0`). Merging them buys nothing and costs a lockfile fight. |
| D7 | `cityparquet-prototype` is **dropped** | Archived rather than deleted purely as link courtesy — nothing in the stack references it. |
| D8 | Paper repo adds the monorepo back as **one submodule** | `just bench-summary` needs the benchmark CSVs; one submodule keeps the paper reproducible from a single clone. |
| D9 | **Code MIT OR Apache-2.0; specification and docs CC-BY-4.0** | Matches `cityparquet-rs` today and standard Rust practice; a content licence is what reviewers of a format specification expect. |
| D10 | **No backward compatibility anywhere** | No users yet (§2). Shims and deprecation periods would be maintenance debt taken on for nobody. |
| D11 | Monorepo's default branch is **`main`**; `develop` is retired | The stack currently mixes `develop` (rs, both extensions) and `main` (citylake). A public repo wants one obvious default, and D10 says fix it now rather than inherit the split. The two extension submodules keep their own branch conventions — they are separate repos. |

---

## 4. End state

Three live repositories and two archives.

| Repository | Visibility | Role |
|---|---|---|
| `cityjson/cityparquet` | **public**, new | the monorepo — spec, implementations, benchmarks |
| `HideBa/cityparquet-paper` | private, existing | manuscript only; monorepo as its single submodule |
| `cityjson/duckdb-cityjson` | public, existing | unchanged; consumed as a submodule |
| `HideBa/duckdb-3d` | public, existing | unchanged; consumed as a submodule |
| `HideBa/cityparquet-rs` | **archived** | history preserved inside the monorepo via subtree |
| `HideBa/cityparquet-prototype` | **archived** | dropped from the stack (D7) |

### 4.1 Monorepo layout

```
cityparquet/
├─ README.md                  project front door: what CityParquet is, quickstart, map of the repo
├─ LICENSE-MIT
├─ LICENSE-APACHE
├─ CITATION.cff               so the format is citable before the paper lands
├─ CONTRIBUTING.md
├─ CODE_OF_CONDUCT.md
├─ CLAUDE.md  ·  AGENTS.md    root orientation, kept in sync
├─ justfile                   top-level task runner (delegates into lib/ and benchmark/)
│
├─ documents/                 Blume docs site — the normative specification
│  ├─ LICENSE                 CC-BY-4.0 (D9)
│  └─ docs/{03-specification, 04-design-decisions, 05-open-questions,
│            06-resources, 07-tutorials, 08-benchmark}
│
├─ lib/
│  ├─ cityparquet-rs/         ← git subtree of HideBa/cityparquet-rs (539 commits)
│  │  ├─ crates/{cityparquet-schema, cityparquet, cityparquet-cli, cityparquet-readbench}
│  │  ├─ docs/                implementation-internal technical docs (NOT the spec)
│  │  └─ vendor/
│  │     ├─ city3d-stac-tool/ submodule → cityjson/city3d-stac-tool
│  │     └─ cjseq/            vendored copy (patched; not a submodule)
│  ├─ citylake/               ← git subtree of HideBa/citylake (26 commits); own workspace
│  ├─ duckdb-cityjson/        submodule → cityjson/duckdb-cityjson
│  └─ duckdb-3d/              submodule → HideBa/duckdb-3d
│
├─ benchmark/
│  ├─ README.md               index of the three families + the fairness caveats, in one place
│  ├─ formats/                ← cityparquet-rs/bench/  (cross-format read, compression, ordering, write)
│  │  ├─ READ_BENCHMARK.md
│  │  ├─ read_results/  scaling_{read,write,compression,ordering}_results/
│  │  ├─ archive/
│  │  └─ data/                gitignored; fetched by script (~7.9 GB)
│  ├─ databases/              ← top-level benchmarking/  (CityParquet vs cjdb vs 3DCityDB v5)
│  │  ├─ src/citybench/  tests/  docker/  params/  scripts/  vendor/  docs/
│  │  └─ README.md
│  ├─ plot/                   ← cityparquet-rs/bench/plot  (benchviz + readbench_plot)
│  └─ summary/                rendered bench-summary.html + bench_data.json
│
├─ ai/
│  ├─ skills/                 agent skills (to be written; currently gitignored .claude/skills)
│  ├─ design-notes/           ← docs/superpowers/{plans,specs} after a sensitivity read
│  └─ apm.yml  apm.lock.yaml
│
├─ test/
│  ├─ TESTING.md              ← root TESTING.md; the cross-module manual walkthrough
│  └─ run-all.sh              invokes rust + duckdb-cityjson + duckdb-3d suites in one go
│
└─ example/
   └─ data/                   small fixtures only; large corpora stay fetch-scripted
```

### 4.2 Paper repo after the split

`cityparquet-paper` keeps `paper/`, `notes/`, `references/`, `interacttfvlatex/`,
`.obsidian/`, `sampledata/` and its full 197-commit history. All five current
submodules are removed; one is added back:

```
cityparquet-paper/
├─ paper/                     manuscript (Typst)
├─ notes/  references/  interacttfvlatex/
├─ cityparquet/               submodule → cityjson/cityparquet   (the only one)
├─ justfile                   build / final / watch / bench-summary
└─ CLAUDE.md  ·  AGENTS.md    rewritten: writing assistant only, no software dev
```

`just bench-summary` moves here and renders from `cityparquet/benchmark/` into
`paper/assets/bench/`.

---

## 5. Migration

Nine phases. Phases 1–7 build the public repo and leave the current repo
untouched, so everything up to phase 8 is abandonable at zero cost.

### Phase 0 — Freeze point

Nothing moves until the working state is committed and pushed everywhere.
Current state, as of 2026-08-23:

| Repo | State |
|---|---|
| `cityparquet-rs` | clean, 0 unpushed |
| `citylake` | clean, 0 unpushed |
| `duckdb-3d` | clean, 0 unpushed |
| `duckdb-cityjson` | clean tracked tree, **untracked `src/external/`** — commit or delete |
| parent | gitlink bumps for `cityparquet-rs`, `duckdb-3d`, `duckdb-cityjson` not recorded |

Tasks:

1. Resolve `duckdb-cityjson/src/external/` — commit it or remove it.
2. Commit the three gitlink bumps in the parent: `git add cityparquet-rs duckdb-3d duckdb-cityjson && git commit`.
3. Push `develop` in the parent and every submodule.
4. Tag the freeze point: `git tag pre-monorepo-split && git push --tags`.

### Phase 1 — Create the public repo

Create `cityjson/cityparquet` empty (no auto-generated README — it would
conflict with the subtree import).

> **If org admin rights are unavailable:** create it under `HideBa/cityparquet`
> and transfer later. GitHub keeps redirects, so nothing downstream breaks.

```sh
mkdir cityparquet && cd cityparquet
git init -b main
git commit --allow-empty -m "chore: initialise CityParquet monorepo"
git remote add origin git@github.com:cityjson/cityparquet.git
```

### Phase 2 — Subtree-import `cityparquet-rs`

```sh
git remote add rs git@github.com:HideBa/cityparquet-rs.git
git fetch rs develop
git subtree add --prefix=lib/cityparquet-rs rs develop
```

**Then re-declare the nested submodule.** `subtree add` carries
`vendor/city3d-stac-tool` in as a bare gitlink, and the `.gitmodules` it
imported records the *old* path (`vendor/city3d-stac-tool`), not the new one:

```sh
git rm --cached lib/cityparquet-rs/vendor/city3d-stac-tool
rm -f lib/cityparquet-rs/.gitmodules
git submodule add https://github.com/cityjson/city3d-stac-tool.git \
    lib/cityparquet-rs/vendor/city3d-stac-tool
```

`vendor/cjseq` is a vendored working copy, not a submodule, and comes across
intact — including the texture-offset patches (`4ef5153`). Verify it is present
and that `lib/cityparquet-rs/vendor/cjseq/` has no `.git`.

### Phase 3 — Subtree-import `citylake`

```sh
git remote add citylake git@github.com:HideBa/citylake.git
git fetch citylake main
git subtree add --prefix=lib/citylake citylake main
```

No nested submodules. Its `Cargo.toml` stays a standalone workspace root (D6).

### Phase 4 — Add the extension submodules

```sh
git submodule add git@github.com:cityjson/duckdb-cityjson.git lib/duckdb-cityjson
git submodule add git@github.com:HideBa/duckdb-3d.git        lib/duckdb-3d
git -C lib/duckdb-cityjson checkout develop
git -C lib/duckdb-3d       checkout develop
```

Both carry their own nested submodules (`duckdb`, `extension-ci-tools`,
`vcpkg`) — this is where the ~1.1 GB of `.git/modules` comes from. Document
`--recurse-submodules` prominently in the README, and add a
`just setup-shallow` recipe using `--depth 1` for contributors who only want
the spec.

### Phase 5 — Copy the non-subtree content

Plain file copies from the current repo, committed fresh:

| From | To | Notes |
|---|---|---|
| `documents/` | `documents/` | as-is; exclude `node_modules/`, `dist/` |
| `benchmarking/` | `benchmark/databases/` | as-is; it is already self-contained (`pyproject.toml`, `uv.lock`, own justfile) |
| `TESTING.md` | `test/TESTING.md` | update every path in it (see §6) |
| `docs/superpowers/` | `ai/design-notes/` | **read for paper-sensitive material first** |
| `apm.yml`, `apm.lock.yaml` | `ai/` | |
| `data/small*`, small fixtures | `example/data/` | small only; large corpora stay fetch-scripted |

**Not copied** (stay in the paper repo, or are dropped):
`paper/`, `notes/`, `references/`, `interacttfvlatex/`, `.obsidian/`,
`sampledata/`, `data/` (bulk), `cityparquet-prototype/`, `head/head`
(a stray shell-redirect artefact — delete it in the paper repo too),
`.claudian/`, `docs/bench-summary.html` (regenerated into `benchmark/summary/`).

### Phase 6 — The bench move and path surgery

**This is the single largest piece of work in the migration.** Budget a focused
day, not an afternoon.

```sh
git mv lib/cityparquet-rs/bench/plot benchmark/plot
git mv lib/cityparquet-rs/bench      benchmark/formats
```

What breaks, measured in the current tree:

| Surface | Extent | Fix |
|---|---|---|
| `lib/cityparquet-rs/justfile` | **57** `bench/` references across ~30 recipes | Hoist the bench recipes into the **root** justfile so `just bench-*` runs from the repo root; leave build/test/lint recipes in place |
| Shell scripts | `readbench_prepare.sh`, `readbench_duckdb.sh`, `bench_duckdb.sh`, `fetch_benchmark.sh`, `fetch_tools.sh`, `package_tables.py` | Introduce a `BENCH_ROOT` variable (default `benchmark/formats`) instead of re-hardcoding a new relative path |
| Shell script tests | `scripts/tests/{fetch_benchmark,readbench_prepare,bench_recipe}_test.sh` | Follow `BENCH_ROOT`; verified by `just scripts-test` |
| `benchviz` | `benchviz/paths.py` and defaults in `prep.py`, `html.py`, `figures.py`; `readbench_plot/{plot,sizes,compression}.py` | Re-root; verified by `just plot-test` |
| Rust | one real default (`cityparquet-readbench/src/main.rs:124`, `default_value = "bench/data/readbench"`); the rest are doc comments in `coordinator.rs`, `naming.rs`, `compare.rs` | Update the default and the prose |
| Prose | `lib/cityparquet-rs/{README,CLAUDE,AGENTS}.md`, `benchmark/formats/{README,READ_BENCHMARK}.md` | Path updates |

Two structural notes:

- **`cityparquet-readbench` stays a workspace crate** in
  `lib/cityparquet-rs/crates/`. Only the harness *data*, *results*, *docs* and
  *plotting* move. After the move the binary and the CSVs it writes live in
  different trees — this is intentional (the crate is code, the results are
  evidence) but it must be stated in `benchmark/README.md` or the next reader
  will assume the split is a mistake.
- **`benchmark/formats/data/` (~7.9 GB) is gitignored** and always was. It is
  reconstructed by `just fetch-data` / `just fetch-scaling-data`. Carry the
  `.gitignore` entries across or the first benchmark run will try to commit 7.9 GB.

Write `benchmark/README.md` last, once the three families sit side by side. It
is the piece the current layout has no home for: one page that says what the
three families measure, what each will and will not support as a claim, and
which caveats are load-bearing — including the codec-level mismatch
(zstd@3 / gzip@6 / brotli@1 defaults) that makes "smallest codec" non-citable,
and the databases harness's "ingest is deliberately not compared" scope.

### Phase 7 — Root files, licensing, CI

1. **`README.md`** — what CityParquet is, a 5-line quickstart, a map of the
   repo, and links to the spec site and the paper (once published).
2. **Licences** — `LICENSE-MIT` + `LICENSE-APACHE` at root (copy from
   `lib/cityparquet-rs/`); `documents/LICENSE` as CC-BY-4.0; state the split in
   the README's Licence section.
3. **`CITATION.cff`** — so the format is citable before the paper lands.
4. **`CONTRIBUTING.md`** — submodule setup, how to run the test suites, where
   spec changes go (`documents/docs/03-specification/` with the reasoning in
   `04-design-decisions/`).
5. **`CLAUDE.md` / `AGENTS.md`** — rewrite the root pair for the new layout.
   Drop the "do not edit submodules from this repository" rule for
   `cityparquet-rs` and `citylake`, which are no longer submodules; keep it for
   the two DuckDB extensions.
6. **CI** — only `.github/workflows/docs.yml` exists today. Add at minimum:
   Rust `cargo test` + `clippy` for `lib/cityparquet-rs`, and the docs build.
   Do **not** put the benchmarks in CI; they are multi-hour and corpus-dependent.
7. **`test/run-all.sh`** — the script `test/TESTING.md` currently describes by
   hand: invoke the Rust suite, both extension suites, and the interop check.

Push. The public repo is now complete and self-contained.

### Phase 8 — Convert `cityparquet-paper` into the paper repo

Only now is the current repo touched.

```sh
git submodule deinit -f cityparquet-rs citylake cityparquet-prototype \
                        duckdb-cityjson duckdb-3d
git rm -r cityparquet-rs citylake cityparquet-prototype duckdb-cityjson duckdb-3d
rm -rf .git/modules/{cityparquet-rs,citylake,cityparquet-prototype,duckdb-cityjson,duckdb-3d}
git submodule add git@github.com:cityjson/cityparquet.git cityparquet
git rm -r --cached benchmarking documents docs/superpowers TESTING.md \
              docs/bench-summary.html
rm -rf benchmarking documents head docs/superpowers
```

Then:

- Rewrite the root `CLAUDE.md`/`AGENTS.md` as **writing assistant only** — the
  software-development half of the current file now belongs to the monorepo.
- Move the `bench-summary` recipe's inputs to `cityparquet/benchmark/`:
  ```
  uv run --project cityparquet/benchmark/plot python -m benchviz figures \
      --data cityparquet/benchmark/summary/bench_data.json \
      --figures paper/assets/bench
  ```
- Keep `paper/assets/bench/*.png|svg` committed. The figures are the paper's
  evidence and must not depend on a submodule checkout to render the PDF.
- Rename `develop` to `main` here too (D11), so both repos share one convention:
  `git branch -m develop main && git push -u origin main` then retarget the
  default branch on GitHub and delete the old remote branch.
- Keep the name `cityparquet-paper` — it is finally literally true — and update
  the repo description to say so, since it currently advertises itself as the
  CityParquet workspace.
- Delete `docs/bench-summary.html`. The summary page is regenerated into the
  monorepo's `benchmark/summary/`; only the paper's own figures stay here.

### Phase 9 — Archive

1. Archive `HideBa/cityparquet-rs` on GitHub. First push a final commit whose
   README says: *"Development moved to
   [cityjson/cityparquet](https://github.com/cityjson/cityparquet) under
   `lib/cityparquet-rs/`. Full history preserved there."*
2. Archive `HideBa/cityparquet-prototype` with a one-line pointer to the docs
   site's Resources page.
3. Add both to `documents/docs/06-resources/02-software.mdx` so the
   implementation-status page reflects the new homes.

---

## 6. Path references to fix outside the bench move

Beyond the table in Phase 6, these files name paths that the migration invalidates:

| File | What changes |
|---|---|
| `test/TESTING.md` | Every command; it currently opens with `cd ~/tudelft/papers/citypaquet-paper` and references all five submodule paths. Also update the commit table and the "run everything from the repo root" line. |
| `documents/docs/06-resources/02-software.mdx` | Implementation status: new repo homes, two archives |
| `documents/docs/07-tutorials/*.mdx` | Any path or clone instruction |
| `documents/docs/08-benchmark/index.mdx` | Points at the new `benchmark/` tree |
| root `justfile` (paper repo) | `bench-summary` inputs (Phase 8) |
| `benchmark/databases/README.md` | Its cross-reference to `cityparquet-rs/bench/READ_BENCHMARK.md` → `benchmark/formats/READ_BENCHMARK.md` |
| `benchmark/databases/justfile`, `scripts/*` | Any reference to a `cityparquet` binary built from the sibling tree |

---

## 7. Verification

Run in the public repo before announcing it:

- [ ] `git clone --recurse-submodules` into a clean directory succeeds
- [ ] `cd lib/cityparquet-rs && just check` passes (lint, test, isolation, vendor-check)
- [ ] `just plot-test` and `just scripts-test` pass — these are the gates on the bench path surgery
- [ ] `cd lib/citylake && cargo build` succeeds
- [ ] `cd documents && pnpm build` succeeds
- [ ] `cd benchmark/databases && uv run pytest` passes
- [ ] `test/run-all.sh` passes end to end
- [ ] `git rev-list --count HEAD` ≥ 570 — history survived the subtrees
      (539 + 26 + the migration's own commits)

      > Do **not** check this with `git log -- lib/cityparquet-rs`. `git subtree
      > add` without `--squash` merges the *original* commits, whose trees have
      > paths at the old root (`crates/…`), not under `lib/cityparquet-rs/`. A
      > path-filtered log shows only the merge and everything after it, which
      > looks like the history was lost when it was not. To inspect one
      > imported history directly, use the second parent of its merge:
      > `git log --oneline <subtree-merge-sha>^2 | wc -l`.
- [ ] `git grep -rn "cityparquet-rs/bench"` returns nothing
- [ ] `git grep -rniE "paper|manuscript|zettel"` surfaces nothing draft-sensitive
- [ ] Repo size after a fresh clone is stated in the README (it is large — set expectations)

And in the paper repo:

- [ ] `just build` produces `paper/main.pdf` with figures intact
- [ ] `just bench-summary` regenerates `paper/assets/bench/` from the submodule

---

## 8. Risks

| Risk | Severity | Mitigation |
|---|---|---|
| Draft manuscript reaches the public repo | **high** | D1 avoids it structurally — nothing is copied from `paper/`, `notes/` or `references/`. The `git grep` check in §7 is the backstop. Read `ai/design-notes/` before committing it. |
| Bench path surgery silently breaks a recipe | medium | `just plot-test` + `just scripts-test` are real suites and cover most of it. Re-run one short benchmark end to end before declaring Phase 6 done. |
| `subtree add` leaves the nested submodule broken | medium | Explicit re-declaration step in Phase 2, verified by the clean-clone check. |
| `cityjson` org admin rights unavailable | low | Stage under `HideBa/`, transfer later; GitHub redirects. |
| Clone size (~1.2 GB with all submodules) deters contributors | low | Document `--depth 1` / spec-only setup in the README and a `just setup-shallow` recipe. |
| Benchmark results become unreproducible mid-move | low | Results CSVs are small and committed; `bench/data/` was always gitignored and fetch-driven. |

---

## 9. Open items — flagged, not decided

- **crates.io.** The `cityparquet` name is unclaimed. Worth publishing
  `cityparquet-schema` + `cityparquet` before announcing, or someone else takes
  the name. Requires deciding whether the crate's `repository` metadata points
  at the monorepo path.
- **DOI / Zenodo.** A citable archive of the specification, separate from the
  paper's DOI.
- **Spec versioning.** The monorepo makes the spec publicly versionable for the
  first time; it currently has no version number or changelog. Decide whether
  `v1.0` is tagged at publication or after the paper is accepted.
- **`documents/` naming.** Kept as-is per the brief. Note that the churn
  argument for keeping it is void under D10 — the only reason it survives is
  that you asked for it. If `docs/` or `spec/` reads better on a public repo,
  rename it during the move rather than after; the cost is a `blume.config.ts`
  edit and a `git grep` sweep, and it never gets cheaper than this.
- **Issue and PR templates**, and whether spec changes get a lightweight RFC
  process. Worth having before the first outside contributor, not before the
  move.

---

## 10. Out of scope

- Any change to the CityParquet encoding, the specification's content, or any
  library's behaviour.
- Rewriting `cityparquet-paper`'s history (D1). This is the one place §2 does
  **not** apply: "break freely" is licence to discard compatibility, not to
  discard the privacy constraint that motivates the whole split.
- Merging the two Cargo workspaces (D6) — revisit only if `citylake` gains a
  path dependency on `cityparquet`.
- Putting benchmarks in CI.
- Writing the `ai/skills/` content, which the brief defers.
