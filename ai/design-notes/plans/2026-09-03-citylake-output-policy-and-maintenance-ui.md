# Output-Path Policy and Maintenance UI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Constrain where the HTTP API may write, then give the web client screens for the seven operations that currently have none.

**Architecture:** A configured output root is enforced in the axum handlers, not in the repository — the finding is about an unauthenticated API, and the library's own tests legitimately write packages to temporary directories. With the contract narrowed, the client gains a maintenance section and three dialogs on the dataset detail page.

**Tech Stack:** Rust with axum, React 19, Vite 8, TanStack Query 5, Playwright.

**Spec:** `ai/design-notes/specs/2026-09-03-citylake-output-policy-and-maintenance-ui-design.md` — read §2 and §3 before Task 1.

## Global Constraints

- **The policy lives at the HTTP boundary.** `export_module_impl` and `write_package_impl` keep their signatures and keep accepting any path. Only `src/app/` enforces the root. Enforcing deeper would break the real-data round trip and the package suite, which pass absolute temporary paths and are right to.
- **An unset root refuses both operations**, with a message naming `CITYLAKE_OUTPUT_ROOT`. Not a fallback to today's behaviour: a control that is off by default is not a control, and there are no users to break.
- **Rejection is by canonicalised, normalised prefix** — never by string inspection. See the verified algorithm below.
- **No CityJSON, geometry, module-routing or CRS logic in Rust.** No authentication is added. The other three trust surfaces stay as they are.
- Login shell for web commands: `bash -lc 'vp check'`, `bash -lc 'npx tsc --noEmit -p tsconfig.app.json'` (the app), `bash -lc 'npx tsc --noEmit -p tsconfig.node.json'` (the e2e suite). **`vp check` type-checks nothing in this project** — measured.
- The Rust suite needs `CITYLAKE_CITYJSON_EXTENSION` pointing at the local build.
- British English; document the present, never the past. Explicit git pathspecs; never `git add -A` (the repository root has an untracked `benchmark/formats/compression_results/`).

## The algorithm, verified before this plan was written

Probed against a real filesystem with a symlink escape in place. Do not simplify it.

1. If the root is unset — refuse.
2. If the requested path is absolute — refuse.
3. Join the requested path to the root.
4. Walk up to the **deepest existing ancestor** and canonicalise that (`std::fs::canonicalize`, which resolves symlinks). A path that does not exist yet cannot be canonicalised, and `package` write names a directory it is about to create.
5. Re-attach the non-existent remainder, then **normalise the result lexically** to resolve any `..` it contains.
6. Require the result to sit under the canonicalised root.

**Step 5 is not optional and is easy to miss.** Measured: with the root at `…/root`, a requested `newdir/../../outside/pkg` passes the check if the remainder is re-attached without normalising — the `..` lives in the part that does not exist, so canonicalisation never sees it — and resolves to `…/outside/pkg`. Normalising catches it. The spec's own prose omitted this; the probe found it.

Measured outcomes to encode as tests: `pkg` → inside; `../outside/pkg` → refused; `escape/pkg` where `escape` is a symlink out of the root → refused; `/etc/pkg` → refused; `newdir/../../outside/pkg` → refused; `a/b/c` (deep, non-existent, no `..`) → inside.

## File Structure

| File | Responsibility |
|---|---|
| `lib/citylake/src/core/interface/types.rs` | `output_root: Option<String>` on the config; `CityLakeError::BadRequest` |
| `lib/citylake/src/app/output_path.rs` | **New.** The resolution and its errors — the whole policy, in one testable place |
| `lib/citylake/src/app/handlers/package.rs` | Resolves before calling the repository |
| `lib/citylake/src/app/mod.rs` | Declares the module; maps `BadRequest` to 400 |
| `lib/citylake/web/src/lib/api.ts` | The seven operations, typed |
| `lib/citylake/web/src/pages/DatasetDetailPage.tsx` | The maintenance section and the three dialogs |
| `lib/citylake/web/e2e/journey.spec.ts` | Coverage for what a run can honestly exercise |

---

### Task 1: The policy, in isolation

Written and tested with no HTTP and no database, because a security control deserves to be readable on its own.

**Files:**
- Create: `lib/citylake/src/app/output_path.rs`
- Modify: `lib/citylake/src/core/interface/types.rs`, `lib/citylake/src/app/mod.rs`

**Interfaces:**
- Produces: `pub fn resolve_output_path(root: Option<&str>, requested: &str) -> Result<std::path::PathBuf, OutputPathError>`; `pub enum OutputPathError { RootNotConfigured, Absolute, Escapes, ParentMissing }` with `Display`; `CityLakeConfig.output_root: Option<String>` read from `CITYLAKE_OUTPUT_ROOT`; `CityLakeError::BadRequest(String)`.

- [ ] **Step 1: Write the failing tests**

In `output_path.rs`'s test module. Each case was measured against a real filesystem:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// A root with a symlink pointing out of it — the case a textual check passes.
    fn fixture() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path().join("root");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(dir.path().join("outside")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(dir.path().join("outside"), root.join("escape")).unwrap();
        (dir, root)
    }

    #[test]
    fn an_unset_root_refuses_every_path() {
        // A control that is off unless configured is not a control.
        assert!(matches!(
            resolve_output_path(None, "pkg"),
            Err(OutputPathError::RootNotConfigured)
        ));
    }

    #[test]
    fn a_relative_path_resolves_inside_the_root() {
        let (_dir, root) = fixture();
        let got = resolve_output_path(Some(root.to_str().unwrap()), "pkg").unwrap();
        assert!(got.starts_with(fs::canonicalize(&root).unwrap()));
    }

    #[test]
    fn a_deep_path_that_does_not_exist_yet_is_allowed() {
        // `package` write names a directory it is about to create, so a
        // non-existent target is the normal case, not an error.
        let (_dir, root) = fixture();
        let got = resolve_output_path(Some(root.to_str().unwrap()), "a/b/c").unwrap();
        assert!(got.starts_with(fs::canonicalize(&root).unwrap()));
    }

    #[test]
    fn an_absolute_path_is_refused() {
        let (_dir, root) = fixture();
        assert!(matches!(
            resolve_output_path(Some(root.to_str().unwrap()), "/etc/pkg"),
            Err(OutputPathError::Absolute)
        ));
    }

    #[test]
    fn a_parent_traversal_is_refused() {
        let (_dir, root) = fixture();
        assert!(matches!(
            resolve_output_path(Some(root.to_str().unwrap()), "../outside/pkg"),
            Err(OutputPathError::Escapes)
        ));
    }

    #[test]
    fn a_symlink_out_of_the_root_is_refused() {
        // The case that distinguishes a real control from a plausible one: no
        // amount of string inspection sees where a symlink points.
        let (_dir, root) = fixture();
        assert!(matches!(
            resolve_output_path(Some(root.to_str().unwrap()), "escape/pkg"),
            Err(OutputPathError::Escapes)
        ));
    }

    #[test]
    fn a_traversal_hidden_in_a_non_existent_remainder_is_refused() {
        // Measured bypass: `newdir` does not exist, so canonicalisation never
        // sees the `..` that follows it. Only normalising the re-attached
        // remainder catches this.
        let (_dir, root) = fixture();
        assert!(matches!(
            resolve_output_path(Some(root.to_str().unwrap()), "newdir/../../outside/pkg"),
            Err(OutputPathError::Escapes)
        ));
    }
}
```

- [ ] **Step 2: Run them and watch them fail**

```bash
cd lib/citylake && cargo test --lib output_path::
```
Expected: FAIL — the module does not exist.

- [ ] **Step 3: Implement**

`output_path.rs`, following the six steps of the verified algorithm above. Normalise lexically by folding the components yourself — `Component::ParentDir` pops, `Component::CurDir` is skipped, `Component::Normal` pushes — because `std::path` has no normalising method and `canonicalize` cannot help on a path that does not exist.

Add `output_root: Option<String>` to `CityLakeConfig`, read in `from_env` as `std::env::var("CITYLAKE_OUTPUT_ROOT").ok()` — an unset root is a legitimate state here, unlike the five fields whose absence means "use the default", so this one does NOT panic when absent. Add `BadRequest(String)` to `CityLakeError` and map it to `StatusCode::BAD_REQUEST` in `app/mod.rs`'s `IntoResponse`. Declare `pub mod output_path;` in `app/mod.rs`.

- [ ] **Step 4: Verify**

```bash
cd lib/citylake && cargo test --lib && cargo clippy --all-targets -- -D warnings && cargo fmt --check
```
Expected: all passing, clean. Seven new tests.

- [ ] **Step 5: Commit**

```bash
git add lib/citylake/src/app/output_path.rs lib/citylake/src/app/mod.rs lib/citylake/src/core/interface/types.rs
git commit -m "feat(citylake): confine API writes to a configured root

The check is by canonicalised, normalised prefix rather than string
inspection: a symlink inside the root points anywhere and no textual
rule sees it, and a `..` in a not-yet-existing remainder escapes
canonicalisation entirely."
```

---

### Task 2: The handlers use it

**Files:**
- Modify: `lib/citylake/src/app/handlers/package.rs`
- Create: `lib/citylake/tests/output_policy.rs`

**Interfaces:**
- Consumes: `resolve_output_path`, `OutputPathError`, `CityLakeError::BadRequest`, `CityLakeConfig.output_root`.
- Produces: `export` and `write_package` refusing an out-of-root path with 400.

The handlers need the config. `AppState` currently carries `Arc<dyn CityLakeRepository>`; extend it to carry the root as well (or the whole config), and thread it through `server::router`. Say in your report which you chose and why.

- [ ] **Step 1: Write the failing tests**

`lib/citylake/tests/output_policy.rs`, in the style of `tests/api.rs`:

```rust
mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

#[tokio::test]
async fn writing_a_package_outside_the_root_is_refused() {
    // The API's whole write surface, in one assertion: a caller naming a path
    // the operator did not sanction gets 400, not a written directory.
    let (app, _dir) = common::app_with_output_root();
    let (status, _) = common::send(
        &app,
        Request::post("/datasets/any/package")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"output_dir":"/tmp/escaped"}"#))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn writing_a_package_without_a_configured_root_is_refused() {
    let (app, _dir) = common::app_without_output_root();
    let (status, body) = common::send(
        &app,
        Request::post("/datasets/any/package")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"output_dir":"pkg"}"#))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    // The operator meets a sentence naming what to set, not a mystery.
    assert!(
        format!("{body}").contains("CITYLAKE_OUTPUT_ROOT"),
        "the refusal must name the variable: {body}"
    );
}

#[tokio::test]
async fn exporting_outside_the_root_is_refused() {
    let (app, _dir) = common::app_with_output_root();
    let (status, _) = common::send(
        &app,
        Request::post("/datasets/any/export")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"module":"building","output_path":"../escaped.city.jsonl","format":"cityjsonseq"}"#,
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
```

**The path check must run BEFORE the dataset is looked up**, so these tests need no dataset — and more importantly, a caller probing paths learns nothing about which datasets exist. Add `app_with_output_root()` and `app_without_output_root()` to `tests/common/mod.rs` beside the existing helpers, and reuse `tests/api.rs`'s `send` helper by moving it into `common` if it is not there already.

- [ ] **Step 2: Run them and watch them fail**

```bash
cd lib/citylake && cargo test --test output_policy
```
Expected: FAIL — the handlers do not check yet, so these return 404 or 500.

- [ ] **Step 3: Implement**

In both handlers, resolve before touching the repository, converting `OutputPathError` into `CityLakeError::BadRequest` with the error's `Display`. Pass the resolved absolute path to the repository.

- [ ] **Step 4: Verify**

```bash
cd lib/citylake && cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check
```
Expected: everything passing. **The existing package and real-data suites must still pass** — they call the repository directly and are unaffected by a handler-layer policy. If either breaks, the policy has been enforced too deep; move it, do not relax the tests.

- [ ] **Step 5: Commit**

```bash
git add lib/citylake/src/app/handlers/package.rs lib/citylake/src/app/server.rs lib/citylake/tests/output_policy.rs lib/citylake/tests/common/mod.rs
git commit -m "feat(citylake): refuse an API write outside the configured root

The check runs before the dataset lookup, so a caller probing paths
learns nothing about which datasets exist."
```

---

### Task 3: The four operations that need no input

**Files:**
- Modify: `lib/citylake/web/src/lib/api.ts`, `lib/citylake/web/src/pages/DatasetDetailPage.tsx`

**Interfaces:**
- Produces: `validateDataset(ds): Promise<ValidationFinding[]>`, `reconcileDataset(ds): Promise<void>`, `vacuumDataset(ds): Promise<{ vacuumed: number }>`, `compactDataset(ds): Promise<{ files_processed: number; files_created: number }>`, and `interface ValidationFinding { check_name: string; severity: string; table_name: string; object_id: string | null; message: string }`.

- [ ] **Step 1: Extend the contract**

Add the four functions and the finding type to `api.ts`, following the file's existing shape. The server's routes are `POST /datasets/{ds}/validate`, `/reconcile`, `/vacuum` and `/compact`; `reconcile` answers 204, `validate` returns a bare array of findings, and the other two return the small objects above.

- [ ] **Step 2: Build the maintenance section**

On `DatasetDetailPage`, below the modules table. Four actions, each reporting its own result and its own error beside itself rather than in a shared banner:

- **Validate** — renders the findings as a table of check, severity, table, object and message. **No findings must read as a stated result** ("No problems found."), never an empty area: blank and clean look identical and mean opposite things.
- **Reconcile** — no confirmation; it loses nothing. Report that it completed.
- **Vacuum** — confirmation via the existing `AlertDialog`, since it deletes. Report the count it returns, including when that count is zero.
- **Compact** — no confirmation. Report files processed and created.

Invalidate `["dataset", ds]` after vacuum and compact, since both change what `describeDataset` reports.

- [ ] **Step 3: Verify**

```bash
cd lib/citylake/web
bash -lc 'npx tsc --noEmit -p tsconfig.app.json'
bash -lc 'vp check'
```
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add lib/citylake/web/src/lib/api.ts lib/citylake/web/src/pages/DatasetDetailPage.tsx
git commit -m "feat(web): validate, reconcile, vacuum and compact from the dataset page

An empty findings list is stated rather than shown as a blank area —
clean and unchecked look the same otherwise."
```

---

### Task 4: Merge, export and package write

**Files:**
- Modify: `lib/citylake/web/src/lib/api.ts`, `lib/citylake/web/src/pages/DatasetDetailPage.tsx`

**Interfaces:**
- Consumes: Task 3's additions; `listDatasets`, `describeDataset`, `ModuleInfo`.
- Produces: `mergeDataset(ds, source): Promise<void>`, `exportModule(ds, body): Promise<void>`, `writePackage(ds, outputDir): Promise<PackageFile[]>`, `interface PackageFile { file: string; action: string; rows: number; bytes: number }`.

- [ ] **Step 1: Extend the contract**

`POST /datasets/{ds}/merge` with `{ source }` answering 204; `POST /datasets/{ds}/export` with `{ module, output_path, format }` answering 204; `POST /datasets/{ds}/package` with `{ output_dir }` returning the written files.

- [ ] **Step 2: The three dialogs**

**Merge** — a source picked from `listDatasets`, with the current dataset excluded from the list. State the preconditions before the user commits: object ids must be unique across the whole destination and the two CRSs must agree, or the extension refuses the entire merge rather than partially applying it. Destructive to the destination, so confirmation is required. A refusal comes back as 422 with the extension's own message — show it, since it says precisely which id or CRS was the problem.

**Export** — module (from the dataset's object modules), format (`cityjson`, `cityjsonseq`, `flatcitybuf`), and a path. **The path is relative to the server's configured output root**; say so in the dialog rather than letting a user discover it through a 400.

**Package write** — a directory, again relative to the root. On success, show the files it wrote with their row counts and sizes; that table is how a user learns the package is what they expected.

- [ ] **Step 3: Verify**

```bash
cd lib/citylake/web
bash -lc 'npx tsc --noEmit -p tsconfig.app.json'
bash -lc 'vp check'
```
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add lib/citylake/web/src/lib/api.ts lib/citylake/web/src/pages/DatasetDetailPage.tsx
git commit -m "feat(web): merge, export and package write from the dataset page

Both write paths are relative to the server's configured output root,
and the dialogs say so rather than letting a 400 explain it."
```

---

### Task 5: End-to-end coverage

**Files:**
- Modify: `lib/citylake/web/e2e/journey.spec.ts`, `lib/citylake/web/playwright.config.ts`

**Interfaces:**
- Consumes: everything above.

- [ ] **Step 1: Give the harness an output root**

`playwright.config.ts` already creates a per-run directory and points the API's catalog into it. Add an `out/` beneath it and pass `CITYLAKE_OUTPUT_ROOT`. The existing `globalTeardown` removes the run directory, so this needs no separate cleanup.

- [ ] **Step 2: Write the tests**

A second spec, or added steps in the journey — your call, but say which and why. Cover what a run can honestly exercise:

- **Validate on a clean dataset** — asserts the "no problems found" result renders. This is a real assertion: a broken validate call renders an error instead.
- **Reconcile** — completes and reports.
- **Compact** — completes and reports its counts.
- **Merge** — create a second dataset in the test from `../tests/data/minimal_7415.city.json` (three objects, EPSG:7415, ids that do not collide with the Delft fixture), merge it in, and assert the destination's row count grew by the source's size. That growth is the assertion that can fail; "the dialog closed" is not.
- **Package write** — write to a path inside the root and assert the returned file list contains `building.parquet` and `metadata.json`.
- **An out-of-root path is refused** — submit `../escaped` and assert the dialog surfaces an error rather than succeeding silently. This is the one that proves Task 1 and 2 reach the user.

**Vacuum's positive path is not covered**, and that is deliberate: no fixture produces an orphaned sidecar row and a user cannot manufacture one through the interface. Assert only that vacuum runs and reports zero. Say so in your report so it reads as a known gap rather than an assumed pass.

- [ ] **Step 3: Prove they can fail**

Break one thing, watch the right test fail, restore it. Choose the merge row-count assertion or the out-of-root refusal — both have a specific failure mode. Put the output in your report.

- [ ] **Step 4: Verify everything**

```bash
cd lib/citylake/web
bash -lc 'npm run e2e'
bash -lc 'npx tsc --noEmit -p tsconfig.app.json'
bash -lc 'npx tsc --noEmit -p tsconfig.node.json'
bash -lc 'vp check'
cd .. && cargo test && cargo clippy --all-targets -- -D warnings
```
Expected: all green, and no `/tmp/citylake-e2e-*` left behind.

- [ ] **Step 5: Commit**

```bash
git add lib/citylake/web/e2e/ lib/citylake/web/playwright.config.ts
git commit -m "test(web): drive the maintenance operations in a browser

Includes the refusal of an out-of-root write, which is what proves the
path policy reaches a user rather than only a unit test."
```

---

## Self-Review

**Spec coverage.** §2's boundary decision → Tasks 1 and 2, with Task 2's verify step asserting the existing suites still pass, since that is what proves the policy did not go too deep. §3's five rules → Task 1's seven tests, each measured. §4's maintenance section → Task 3; its three dialogs → Task 4. §5's testing → Tasks 1, 2 and 5, including the symlink case called out as the one that matters. §6's exclusions → nothing here adds a download endpoint or authentication.

**One thing the spec got wrong, corrected here.** §3 describes canonicalising the deepest existing ancestor and checking the remainder, and stops there. That is insufficient: a `..` inside the non-existent remainder never reaches canonicalisation and escapes. Measured against a real filesystem — `newdir/../../outside/pkg` resolves outside a root it appears to sit under. The plan's algorithm adds the lexical normalisation step and the case is a required test.

**One decision the spec left open.** The policy needs an error type that reaches an HTTP status. Rather than refactor the handlers onto a new app-layer error enum, `CityLakeError` gains a `BadRequest(String)` variant mapped to 400. The repository never constructs it; it is shared currency between the layers. A reviewer may reasonably prefer a separate `ApiError` — the trade is one variant against a refactor of every handler signature.

**Placeholders.** None. Task 1's tests are complete and each encodes a measured outcome. Tasks 3 and 4 specify behaviour and the exact data each screen reads rather than transcribing React, because the components' idiom is established and pointing at it is more reliable than inventing markup.

**Type consistency.** `ValidationFinding` matches the Rust struct's field names (`check_name`, `severity`, `table_name`, `object_id`, `message`); `PackageFile` matches (`file`, `action`, `rows`, `bytes`); `CompactionStats` is `{ files_processed, files_created }`. `resolve_output_path` and `OutputPathError` are defined in Task 1 and consumed in Task 2.
