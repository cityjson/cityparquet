# UI End-to-End Coverage Implementation Plan (piece C)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Drive the CityLake web client end to end in a real browser, so the layer that has never had a single test gets one.

**Architecture:** Playwright starts two servers — the Rust API and the Vite dev server — and drives Chromium through one journey: past the login gate, upload a file, see the dataset, open a module, delete an object, drop the dataset. Getting past the login gate needs a test-only bypass, and running two servers without colliding with a developer's own needs the API to take its configuration from the environment. Those are the first two tasks; the harness and the journey follow.

**Tech Stack:** Playwright (Chromium), Vite 8, React 19, React Router 7, TanStack Query 5, Supabase JS, Rust with axum.

**Spec:** `ai/design-notes/specs/2026-09-02-citylake-real-data-and-ui-e2e-design.md` — read §5 before Task 1.

## Global Constraints

- **The bypass must be impossible to enable in production.** It requires BOTH `import.meta.env.DEV` and `VITE_E2E_AUTH_BYPASS === "1"`. Vite replaces `import.meta.env.DEV` with the literal `false` in a production build, so the branch is constant-folded and eliminated — a `vp build` artefact does not contain it, and no environment variable can switch it on afterwards. A `VITE_`-prefixed flag alone would NOT have that property. Task 2 proves the elimination rather than asserting it.
- **Login shell for every command.** The tool's shells are non-interactive and do not source the profile that puts `vp` on `PATH`: use `bash -lc '...'`. `npm install` is already done in `lib/citylake/web`.
- **`vp check` does NOT type-check this project** — measured: a deliberate type error passed it, because the local `vite-plus` package is absent and that step is skipped. `npx tsc --noEmit -p tsconfig.app.json` is the gate that matters for the client.
- **The Rust API needs the local extension**, since the published community build lacks the `cityparquet_*` pragmas:
  ```
  CITYLAKE_CITYJSON_EXTENSION=/data2/hideba/cityparquet-paper/cityparquet/lib/duckdb-cityjson/build/release/extension/cityjson/cityjson.duckdb_extension
  ```
- **Playwright's browsers are already cached** at `~/.cache/ms-playwright` (Chromium present). Only the `@playwright/test` package needs installing — do not download browsers.
- British English; **document the present, never the past**. Scope git operations with explicit pathspecs; never `git add -A` — the repository root has an untracked `benchmark/` directory that is not this work's.

## Verified Ground Truth

Measured before this plan was written. Do not re-derive.

1. `src/main.rs` builds `CityLakeConfig::default()` and ignores the environment entirely, so the API always listens on port 3000 and always writes `./metadata.ducklake` and `./data/` relative to its working directory. Two consequences: an end-to-end run collides with any development server, and its state persists between runs.
2. `vite.config.ts` proxies `/api` to a hardcoded `http://127.0.0.1:3000`, rewriting away the `/api` prefix. If the API's port becomes configurable, the proxy target must follow it.
3. `createClient()` (`src/lib/supabase/client.ts`) falls back to placeholder URL and key when the environment is unset, so the bundle stays importable and nothing throws on import. `supabase.auth.getSession()` reads localStorage rather than the network, so with no Supabase configured it resolves to a null session rather than hanging — the app renders the login gate rather than "Checking session…" forever.
4. `ProtectedRoute` gates purely on `session` from `useAuth()`, so a synthetic session in `AuthContext` is sufficient to pass it. `/login` is registered in `main.tsx` as a sibling of the `/*` splat.
5. `lib/citylake/tests/data/delft.city.jsonl` is a three-object CityJSONSeq fixture declaring EPSG:7415 — small and fast, which is what an interface test wants. Piece A owns scale.

## File Structure

| File | Responsibility |
|---|---|
| `lib/citylake/src/core/interface/types.rs` | `CityLakeConfig::from_env()` — the config, read from the environment |
| `lib/citylake/src/main.rs` | Uses it |
| `lib/citylake/web/vite.config.ts` | Proxy target follows the API's port |
| `lib/citylake/web/src/auth/AuthContext.tsx` | The test-only session |
| `lib/citylake/web/playwright.config.ts` | The two servers and the browser |
| `lib/citylake/web/e2e/journey.spec.ts` | The journey |
| `lib/citylake/web/.gitignore` | Playwright's artefacts |

---

### Task 1: The API takes its configuration from the environment

Without this, an end-to-end run writes into whatever directory it starts in and fights any development server for port 3000.

**Files:**
- Modify: `lib/citylake/src/core/interface/types.rs`
- Rewrite: `lib/citylake/src/main.rs`

**Interfaces:**
- Produces: `CityLakeConfig::from_env() -> Self` — `Default::default()` with each field overridden when its variable is set: `CITYLAKE_HOST`, `CITYLAKE_PORT`, `CITYLAKE_CATALOG_NAME`, `CITYLAKE_CATALOG_PATH`, `CITYLAKE_STORAGE_PATH`.

- [ ] **Step 1: Write the failing test**

In `types.rs`'s test module:

```rust
    #[test]
    fn the_config_falls_back_to_its_defaults() {
        // Nothing set: from_env must agree with Default in every field, so an
        // unconfigured run behaves exactly as it did before it could be configured.
        let from_env = CityLakeConfig::from_env();
        let default = CityLakeConfig::default();
        assert_eq!(from_env.host, default.host);
        assert_eq!(from_env.port, default.port);
        assert_eq!(from_env.catalog_name, default.catalog_name);
        assert_eq!(from_env.catalog_path, default.catalog_path);
        assert_eq!(from_env.storage_path, default.storage_path);
    }

    #[test]
    fn an_unparseable_port_is_an_error_not_a_silent_default() {
        // A typo in CITYLAKE_PORT must not quietly serve on 3000 — the operator
        // asked for something specific and deserves to be told it was not honoured.
        assert!(CityLakeConfig::port_from("not-a-number").is_err());
        assert_eq!(CityLakeConfig::port_from("3100").unwrap(), 3100);
    }
```

**On test isolation:** Rust runs tests in threads of one process, so a test that sets a process-wide environment variable can be observed by another running concurrently. Do NOT write a test that sets `CITYLAKE_PORT` and asserts `from_env` reads it — that is the shape that produces intermittent failures. The two tests above avoid it: the first asserts the no-variables case, and the second tests the parsing helper directly. If you want coverage of the override path, expose the parsing as `port_from(&str)` and test that, which is what the second test does.

- [ ] **Step 2: Run them and watch them fail**

```bash
cd lib/citylake && cargo test --lib types::
```
Expected: FAIL — `from_env` and `port_from` do not exist.

- [ ] **Step 3: Implement**

Add to `impl CityLakeConfig`:

```rust
    /// The configuration, with each field taken from its environment variable
    /// when set and left at its default when not.
    ///
    /// An unset variable means "use the default"; a variable set to something
    /// unusable is an error, because an operator who set it meant it.
    pub fn from_env() -> Self {
        let default = Self::default();
        Self {
            host: std::env::var("CITYLAKE_HOST").unwrap_or(default.host),
            port: std::env::var("CITYLAKE_PORT")
                .ok()
                .map(|raw| Self::port_from(&raw).expect("CITYLAKE_PORT must be a port number"))
                .unwrap_or(default.port),
            catalog_name: std::env::var("CITYLAKE_CATALOG_NAME").unwrap_or(default.catalog_name),
            catalog_path: std::env::var("CITYLAKE_CATALOG_PATH").unwrap_or(default.catalog_path),
            storage_path: std::env::var("CITYLAKE_STORAGE_PATH").unwrap_or(default.storage_path),
        }
    }

    /// Parse a port, so the parsing is testable without touching the process
    /// environment — a test that sets a variable can be seen by every other
    /// test in the binary.
    pub fn port_from(raw: &str) -> Result<u16, std::num::ParseIntError> {
        raw.parse()
    }
```

Then `main.rs` uses it — `CityLakeConfig::from_env()` in place of `::default()` — and logs the address it is about to serve on, so a run whose port came from the environment says so.

- [ ] **Step 4: Verify**

```bash
cd lib/citylake && cargo test --lib types:: && cargo clippy --all-targets -- -D warnings && cargo fmt --check
```
Expected: passing, clean. Then prove the override works end to end, by hand:

```bash
export CITYLAKE_CITYJSON_EXTENSION=/data2/hideba/cityparquet-paper/cityparquet/lib/duckdb-cityjson/build/release/extension/cityjson/cityjson.duckdb_extension
cd $(mktemp -d) && CITYLAKE_PORT=3100 cargo run --manifest-path /data2/hideba/cityparquet-paper/cityparquet/lib/citylake/Cargo.toml &
sleep 20 && curl -s localhost:3100/health && curl -s localhost:3100/datasets
```
Expected: `/health` answers and `/datasets` returns `[]`. Confirm the catalog was written into the temporary directory, not into `lib/citylake`. Kill the server afterwards. Put the output in your report.

- [ ] **Step 5: Commit**

```bash
git add lib/citylake/src/core/interface/types.rs lib/citylake/src/main.rs
git commit -m "feat(citylake): take the server's configuration from the environment

The binary built a default config and ignored it, so the port and the
catalog's location were fixed at compile time. An end-to-end run needs
its own port and its own catalog; an unparseable port is an error
rather than a silent fall back to 3000."
```

---

### Task 2: The test-only session, and proof it cannot ship

**Files:**
- Modify: `lib/citylake/web/src/auth/AuthContext.tsx`

**Interfaces:**
- Consumes: nothing.
- Produces: a session for `ProtectedRoute` when both `import.meta.env.DEV` and `VITE_E2E_AUTH_BYPASS === "1"`.

- [ ] **Step 1: Add the bypass**

In `AuthProvider`, before the effect that consults Supabase:

```tsx
/**
 * A stand-in session for automated interface tests.
 *
 * Both conditions are required, and the first is the one that matters:
 * Vite replaces `import.meta.env.DEV` with the literal `false` in a
 * production build, so this branch is constant-folded away and the built
 * artefact does not contain it. The environment variable alone would be a
 * switch anybody could flip in production; paired with DEV it can only ever
 * be flipped in a development server.
 */
const E2E_SESSION_ACTIVE =
  import.meta.env.DEV && import.meta.env.VITE_E2E_AUTH_BYPASS === "1";
```

When it is true, `AuthProvider` supplies a synthetic session and `loading: false` **without calling Supabase at all** — no `getSession`, no `onAuthStateChange` subscription. The session needs only enough shape to satisfy `ProtectedRoute` and anything that reads `session.access_token`; construct a minimal `Session` and cast it, with a comment saying it is deliberately minimal. `signOut` becomes a no-op in this mode, since there is nothing to sign out of.

Keep the real path exactly as it is when the flag is off.

- [ ] **Step 2: Prove it is eliminated from a production build**

This is the step that matters. A comment claiming the branch is stripped is worth nothing; the bundle either contains the marker or it does not.

```bash
cd lib/citylake/web
bash -lc 'VITE_E2E_AUTH_BYPASS=1 vp build'
grep -rc "VITE_E2E_AUTH_BYPASS" dist/ || echo "marker absent from dist — correct"
grep -rn "e2e-bypass" dist/assets/*.js | head -3 || echo "no bypass identifier in the bundle"
```
Expected: the marker does not appear in `dist/`, **even though the variable was set during the build**. That is the whole safety property. If it does appear, the bypass is not gated on `import.meta.env.DEV` correctly — fix it before continuing, and put both greps in your report.

- [ ] **Step 3: Confirm the development path works**

```bash
bash -lc 'npx tsc --noEmit -p tsconfig.app.json'
bash -lc 'vp check'
```
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add lib/citylake/web/src/auth/AuthContext.tsx
git commit -m "feat(web): a development-only session for interface tests

Gated on import.meta.env.DEV as well as the flag: Vite folds the former
to false in a production build, so the branch is eliminated and the
artefact cannot be made to carry it. The flag alone would be a switch
anybody could set in production."
```

---

### Task 3: The harness

**Files:**
- Create: `lib/citylake/web/playwright.config.ts`
- Create: `lib/citylake/web/e2e/smoke.spec.ts`
- Modify: `lib/citylake/web/package.json`, `lib/citylake/web/.gitignore`, `lib/citylake/web/vite.config.ts`

**Interfaces:**
- Consumes: Task 1's environment variables, Task 2's bypass.
- Produces: `npm run e2e`, and a running pair of servers for Task 4.

- [ ] **Step 1: Let the proxy follow the API's port**

`vite.config.ts` hardcodes `http://127.0.0.1:3000` as the proxy target. Read it from `process.env.CITYLAKE_API_TARGET` with that value as the default, in the same style the file already uses for the Supabase variables. Without this, pointing the API at another port silently breaks every request the client makes — the proxy would still be forwarding to 3000.

- [ ] **Step 2: Install Playwright and configure it**

```bash
cd lib/citylake/web && bash -lc 'npm i -D @playwright/test'
```
Browsers are already cached; do not run `playwright install`.

`playwright.config.ts` runs Chromium only, against `http://127.0.0.1:5173`, with two `webServer` entries:

- **The API.** `cargo run --manifest-path <abs>/lib/citylake/Cargo.toml`, with `cwd` set to a directory created for the run so its catalog and data land there rather than in the repository, and env carrying `CITYLAKE_PORT=3100`, `CITYLAKE_CATALOG_PATH`, `CITYLAKE_STORAGE_PATH` and `CITYLAKE_CITYJSON_EXTENSION`. `url: 'http://127.0.0.1:3100/health'`. Give it a generous `timeout` — a cold `cargo run` compiles first.
- **The client.** `vp dev`, env carrying `VITE_E2E_AUTH_BYPASS=1` and `CITYLAKE_API_TARGET=http://127.0.0.1:3100`. `url: 'http://127.0.0.1:5173'`.

Set `reuseExistingServer: false` so a run never silently tests a developer's own server with different state.

Add `"e2e": "playwright test"` to `package.json`'s scripts, and `test-results/`, `playwright-report/` and the run directory to `.gitignore`.

- [ ] **Step 3: Write the smoke test**

`e2e/smoke.spec.ts`:

```ts
import { expect, test } from "@playwright/test";

test("the datasets page renders past the login gate", async ({ page }) => {
  await page.goto("/datasets");

  // Reaching this heading proves three things at once: the bypass satisfied
  // ProtectedRoute, the client booted, and the API answered — an empty list
  // still requires a successful request.
  await expect(page.getByRole("heading", { name: "Datasets" })).toBeVisible();
  await expect(page).toHaveURL(/\/datasets$/);
});
```

- [ ] **Step 4: Run it**

```bash
cd lib/citylake/web && bash -lc 'npm run e2e'
```
Expected: one test passing, both servers started and torn down. If the page redirects to `/login`, the bypass is not reaching the client — check that `vp dev` received `VITE_E2E_AUTH_BYPASS=1`, since Vite only exposes `VITE_`-prefixed variables to the browser. If the heading never appears but the URL is right, open the report (`playwright-report/`) and read the browser console: a failed `/api/datasets` means the proxy target and the API's port disagree.

- [ ] **Step 5: Commit**

```bash
git add lib/citylake/web/playwright.config.ts lib/citylake/web/e2e/smoke.spec.ts \
        lib/citylake/web/package.json lib/citylake/web/package-lock.json \
        lib/citylake/web/.gitignore lib/citylake/web/vite.config.ts
git commit -m "test(web): drive the client in a browser

Playwright starts the API on its own port with its own catalog and the
client with the development-only session, so a run shares nothing with
a developer's servers."
```

---

### Task 4: The journey

**Files:**
- Create: `lib/citylake/web/e2e/journey.spec.ts`

**Interfaces:**
- Consumes: the harness from Task 3.
- Produces: nothing.

- [ ] **Step 1: Write it**

One test, walking the path a person walks. Upload `lib/citylake/tests/data/delft.city.jsonl` — three objects, EPSG:7415 — with `setInputFiles`, giving the dataset a name unique to the run so a re-run against a surviving catalog cannot collide.

The journey, and what each step proves:

1. **Upload and create.** Fill the name, attach the file, submit. Proves the multipart path and that the API ingests.
2. **It appears in the list.** Navigate to `/datasets`; the new name is there. Proves the create invalidated the list — the cache-key defect piece B fixed lives exactly here.
3. **Open it.** The detail page shows the `building` module, a row count of 3, and a CRS mentioning 7415. Proves `describeDataset` and that the CRS the extension minted survived to the interface.
4. **Browse the module.** Click through to the module page; three object rows are visible. Proves the query path and the bare-array shape.
5. **Delete an object.** Confirm the dialog; the row goes and the count falls. Proves the delete path and its cascade reporting.
6. **Drop the dataset.** Confirm; it is gone from `/datasets`. Proves the drop and its invalidation.

Assert on **user-visible text and roles**, not on CSS classes or test ids that do not exist yet — if a step needs a hook, add a `data-testid` to the component in the same commit and say so in your report. Prefer `getByRole`, `getByLabel` and `getByText`.

- [ ] **Step 2: Run the whole suite**

```bash
cd lib/citylake/web && bash -lc 'npm run e2e'
```
Expected: both tests passing.

- [ ] **Step 3: Prove the journey can fail**

A green end-to-end test that cannot fail is worth less than none, and this plan's siblings found sixteen assertions that could not. Break one thing, watch the right test fail, and restore it — for example, change the module page's query to request a module that does not exist, and confirm step 4 fails rather than the suite passing anyway. Put the failure output in your report.

- [ ] **Step 4: Commit**

```bash
git add lib/citylake/web/e2e/journey.spec.ts
git commit -m "test(web): walk the client's whole path in a browser

Upload, list, open, browse, delete, drop — the layer that had no test
until now."
```

---

## Self-Review

**Spec coverage.** §5's bypass, its DEV gating and the reason for the pairing → Task 2, with the elimination proved against a built bundle rather than asserted. §5's harness starting both servers → Task 3. §5's journey → Task 4. §5's "small fixtures, deliberately" → Task 4 uses the three-object fixture; piece A owns scale.

**One addition the spec did not call for, and why.** Task 1 makes the API configurable. The spec assumed Playwright could just start it, but `main.rs` ignores its own config struct, so every run would take port 3000 and write its catalog into whatever directory it started in — colliding with a developer's server and carrying state between runs. An end-to-end suite that is not isolated is one that fails for reasons unrelated to the code. This also closes a defect the rebuild's final review recorded.

**Placeholders.** None. The one place I have specified behaviour rather than code is Task 4's journey, deliberately: the selectors depend on markup I have not read line by line, and inventing `getByRole` calls that do not match would be worse than naming what each step must prove and letting the implementer read the components.

**Type consistency.** `CityLakeConfig::from_env` and `port_from` are defined in Task 1 and used in Task 1's `main.rs`. `CITYLAKE_API_TARGET` is introduced in Task 3's `vite.config.ts` and set by Task 3's `playwright.config.ts`. `VITE_E2E_AUTH_BYPASS` is read in Task 2 and set in Task 3.

**A trap named for the implementer.** Task 1's tests deliberately avoid setting environment variables, because Rust runs a binary's tests in threads of one process and a variable set by one is visible to all — the classic source of intermittent failures. The parsing is exposed as `port_from` so the override path is testable without touching the environment at all.
