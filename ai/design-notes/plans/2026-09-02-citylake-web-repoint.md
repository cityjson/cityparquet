# Web Client Re-point Implementation Plan (piece B)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `lib/citylake/web` work again by re-pointing it from the removed one-table-per-LoD model at the dataset/module API the crate now serves.

**Architecture:** `src/lib/api.ts` is the contract and changes first; everything else follows from it. Four surfaces consume it — the sidebar in `AppShell`, the datasets list, the dataset detail, and a module object browser that replaces `LodTablePage`. Supabase authentication and `ProtectedRoute` are untouched: this re-points the client at a different API, it does not redesign the application.

**Tech Stack:** React 19, Vite 8, TanStack Query 5, React Router 7, Tailwind 3 with shadcn/Radix components, TypeScript 5.6.

**Spec:** `ai/design-notes/specs/2026-09-02-citylake-real-data-and-ui-e2e-design.md` — read §4 before Task 1. §2 fixes the scope: datasets list, dataset detail showing modules, upload and create, module object browse, delete. Nothing else.

## Global Constraints

- **Minimum viable, matched to the end-to-end journey piece C will drive.** No UI for validate, reconcile, vacuum, merge, package write, export or compact — those are a later piece of work.
- **`vp` needs a login shell, and it does not type-check.** The binary is at
  `/home/hbbaba/.local/share/vite-plus/bin/vp` (v0.3.0), reachable only through
  `bash -lc '...'` — a plain non-interactive shell does not source the profile that
  puts it on `PATH`. `node_modules` starts empty, so run `npm install` first.

  **`vp check` alone is not a sufficient gate for this task.** Measured: with a
  deliberate `const x: number = "not a number"` in `src/lib/api.ts`, `vp check`
  reported format and lint both passing and said nothing, while
  `npx tsc --noEmit` caught it immediately. Its own warning explains why — "This
  project does not use vite-plus", so the local `vite-plus` package is absent and
  the type-check step is skipped. Since a broken re-point is exactly a type error,
  the gate is all three:
  ```bash
  bash -lc 'npm install'                                   # once
  bash -lc 'vp check'                                      # format + lint (oxfmt, oxlint)
  bash -lc 'npx tsc --noEmit -p tsconfig.app.json'         # the type check vp skips
  bash -lc 'vp build'                                      # the production build
  ```
- **British English** in prose and comments. **Document the present, never the past** — no changelog voice, no "previously", no comment explaining what the LoD model used to do.
- **Every server response shape below was read from the Rust handlers**, not guessed. Use them exactly.
- Scope git operations to `lib/citylake/web/`; explicit pathspec, never `git add -A`.

## The API, as the server actually serves it

Read from `src/app/handlers/` and `src/core/interface/types.rs`:

| Method | Path | Request | Response |
|---|---|---|---|
| `GET` | `/datasets` | — | `string[]` — a **bare array** of dataset names |
| `POST` | `/datasets/{ds}` | `{ source_path }` | `201` + `DatasetInfo` |
| `POST` | `/datasets/{ds}/upload` | multipart, field `file` | `201` + `DatasetInfo` |
| `GET` | `/datasets/{ds}` | — | `DatasetInfo` |
| `DELETE` | `/datasets/{ds}` | — | `204`, empty body |
| `POST` | `/datasets/{ds}/objects` | `{ source_path }` | `{ ingested: number }` |
| `GET` | `/datasets/{ds}/modules/{module}/objects` | `?filter=&limit=&offset=` | **bare array** of row objects |
| `DELETE` | `/datasets/{ds}/objects/{id}` | — | `{ deleted: number }` |

`DatasetInfo` is `{ name: string; modules: ModuleInfo[]; crs: string | null }` and `ModuleInfo` is `{ name: string; role: string; rows: number }`. `role` is `"object"` or `"sidecar"`.

**Two shape changes worth naming**, because the current client assumes otherwise: `/datasets` returns a bare array rather than `{ count, tables }`, and the object query returns a bare array rather than `{ table, count, objects }`. A client that destructures the old envelopes gets `undefined` and renders nothing, with no error.

## File Structure

| File | Responsibility |
|---|---|
| `src/lib/api.ts` | The contract: types and one function per endpoint. Rewritten. |
| `src/components/AppShell.tsx` | The sidebar listing datasets. Currently groups LoD tables client-side. |
| `src/pages/DatasetsPage.tsx` | The datasets list. Currently groups LoD tables client-side. |
| `src/pages/DatasetDetailPage.tsx` | One dataset: its modules, their roles and row counts, its CRS. |
| `src/pages/ModulePage.tsx` | **New.** One module's objects, paginated and filterable, with delete. Replaces `LodTablePage`. |
| `src/pages/LodTablePage.tsx` | **Deleted.** |
| `src/pages/UploadPage.tsx` | Create a dataset from an uploaded file. |
| `src/App.tsx` | Routes. |

`LoginPage`, `AuthCallbackPage`, `auth/`, and `components/ui/` are untouched.

---

### Task 1: The contract

**Files:**
- Rewrite: `lib/citylake/web/src/lib/api.ts`

**Interfaces:**
- Consumes: nothing.
- Produces, all exported: `ApiError`; `interface ModuleInfo { name: string; role: string; rows: number }`; `interface DatasetInfo { name: string; modules: ModuleInfo[]; crs: string | null }`; `type ObjectRow = Record<string, unknown>`; and the functions `listDatasets(): Promise<string[]>`, `describeDataset(ds: string): Promise<DatasetInfo>`, `createDataset(ds: string, sourcePath: string): Promise<DatasetInfo>`, `uploadDataset(ds: string, file: File): Promise<DatasetInfo>`, `dropDataset(ds: string): Promise<void>`, `ingestSource(ds: string, sourcePath: string): Promise<{ ingested: number }>`, `queryObjects(ds: string, module: string, params?: { filter?: string; limit?: number; offset?: number }): Promise<ObjectRow[]>`, `deleteObject(ds: string, id: string): Promise<{ deleted: number }>`.

- [ ] **Step 1: Install dependencies and confirm the gate is green before you start**

```bash
cd lib/citylake/web
bash -lc 'npm install'
bash -lc 'npx tsc --noEmit -p tsconfig.app.json'
```
Expected: **clean, no errors.** The pages speak the old contract and the old contract
still exists, so the tree is self-consistent right now. The red state arrives in Step 2,
the moment `api.ts` stops exporting `listTables` and `TableInfo` — that is when every
consumer breaks at once, and Tasks 2-4 are what close it. Confirming green here tells you
the gate runs at all, which is the point of the step.

- [ ] **Step 2: Rewrite the contract**

Keep the existing `request`/`authHeader`/`safeParse` helpers and the `ApiError` class exactly as they are — the Supabase bearer header, the error extraction and the JSON parsing are unchanged by this work. Replace everything from the `// ---------- typed endpoints ----------` marker down with:

```ts
// ---------- typed endpoints ----------

/** A module table inside a dataset. `role` is "object" or "sidecar". */
export interface ModuleInfo {
  name: string;
  role: string;
  rows: number;
}

/** A dataset: a CityParquet package, one table per CityGML module. */
export interface DatasetInfo {
  name: string;
  modules: ModuleInfo[];
  crs: string | null;
}

/** One CityObject, as the server returns it: a flat JSON row. */
export type ObjectRow = Record<string, unknown>;

/** The server returns a bare array of names, not an envelope. */
export function listDatasets(): Promise<string[]> {
  return request<string[]>("/datasets");
}

export function describeDataset(ds: string): Promise<DatasetInfo> {
  return request<DatasetInfo>(`/datasets/${encodeURIComponent(ds)}`);
}

export function createDataset(ds: string, sourcePath: string): Promise<DatasetInfo> {
  return request<DatasetInfo>(`/datasets/${encodeURIComponent(ds)}`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ source_path: sourcePath }),
  });
}

export function uploadDataset(ds: string, file: File): Promise<DatasetInfo> {
  const fd = new FormData();
  fd.append("file", file);
  return request<DatasetInfo>(`/datasets/${encodeURIComponent(ds)}/upload`, {
    method: "POST",
    body: fd,
  });
}

/** Drops the dataset and everything in it. The server answers 204. */
export function dropDataset(ds: string): Promise<void> {
  return request<void>(`/datasets/${encodeURIComponent(ds)}`, { method: "DELETE" });
}

export function ingestSource(ds: string, sourcePath: string): Promise<{ ingested: number }> {
  return request<{ ingested: number }>(`/datasets/${encodeURIComponent(ds)}/objects`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ source_path: sourcePath }),
  });
}

/** Also a bare array — there is no envelope and no total count. */
export function queryObjects(
  ds: string,
  module: string,
  params: { filter?: string; limit?: number; offset?: number } = {},
): Promise<ObjectRow[]> {
  const qs = new URLSearchParams();
  if (params.filter) qs.set("filter", params.filter);
  if (params.limit !== undefined) qs.set("limit", String(params.limit));
  if (params.offset !== undefined) qs.set("offset", String(params.offset));
  const query = qs.toString();

  return request<ObjectRow[]>(
    `/datasets/${encodeURIComponent(ds)}/modules/${encodeURIComponent(module)}/objects${
      query ? `?${query}` : ""
    }`,
  );
}

/** Deletes by id, cascading to the object's children; returns how many went. */
export function deleteObject(ds: string, id: string): Promise<{ deleted: number }> {
  return request<{ deleted: number }>(
    `/datasets/${encodeURIComponent(ds)}/objects/${encodeURIComponent(id)}`,
    { method: "DELETE" },
  );
}
```

**One thing to check while you are here.** `request` does `const text = await res.text()` and returns `payload as T`. A 204 with an empty body yields `undefined`, which `dropDataset` types as `void` — correct. Confirm nothing in `request` throws on an empty body before you rely on that.

- [ ] **Step 3: Confirm the contract compiles on its own**

```bash
bash -lc 'npx tsc --noEmit -p tsconfig.app.json' 2>&1 | grep "src/lib/api.ts"
```
Expected: no output — `api.ts` itself is clean. The pages now fail loudly, which is the
red state Tasks 2-4 close. Do not try to silence them here.

- [ ] **Step 4: Commit**

```bash
git add lib/citylake/web/src/lib/api.ts lib/citylake/web/package-lock.json
git commit -m "feat(web): type the dataset and module API

The server answers /datasets and the object query with bare arrays
rather than envelopes, so a client destructuring {count, tables} or
{table, count, objects} reads undefined and renders nothing without
raising."
```

---

### Task 2: The two surfaces that list datasets

Both the sidebar and the datasets page derive datasets today by grouping LoD table names client-side. The server now answers that question directly, so the grouping goes.

**Files:**
- Modify: `lib/citylake/web/src/components/AppShell.tsx`
- Rewrite: `lib/citylake/web/src/pages/DatasetsPage.tsx`

**Interfaces:**
- Consumes: `listDatasets`, `DatasetInfo` from Task 1.
- Produces: the `/datasets/:name` link target Task 3 serves.

- [ ] **Step 1: Re-point the sidebar**

In `AppShell.tsx`, delete the local `groupByBase` helper and the `Dataset` interface it fills, and replace the `listTables` query with:

```tsx
  const { data, isLoading } = useQuery({
    queryKey: ["datasets"],
    queryFn: listDatasets,
  });

  const datasets = data ?? [];
```

The sidebar then maps over plain names. Keep every piece of visual structure — the `Eyebrow`, the empty-state copy, the `NavLink` styling, the upload button — and change only what it is iterating over and what each entry links to (`/datasets/${name}`). Where the eyebrow reads `Tables · {n}`, make it `Datasets · {n}`: it is naming what the list holds.

- [ ] **Step 2: Rewrite the datasets page**

`DatasetsPage.tsx` loses `groupByBase`, the `Dataset` interface, the `METADATA_TABLE` constant and the LoD tags. A card per dataset name, linking to `/datasets/${name}`. Because `GET /datasets` returns only names, a card cannot show module or row counts without a request per dataset — do not add one. Show the name and let the detail page carry the numbers.

Keep the page's existing shape exactly: the `Eyebrow`/`h1`/description header, the loading skeletons, the error card, the empty state with its link to `/upload`, and the card grid. Update the description to say what a dataset now is — a CityParquet package, one table per CityGML module — without narrating that anything changed.

- [ ] **Step 3: Verify**

```bash
cd lib/citylake/web
bash -lc 'npx tsc --noEmit -p tsconfig.app.json' 2>&1 | grep -E "AppShell|DatasetsPage"
```
Expected: no output for these two files.

- [ ] **Step 4: Commit**

```bash
git add lib/citylake/web/src/components/AppShell.tsx lib/citylake/web/src/pages/DatasetsPage.tsx
git commit -m "feat(web): list datasets from the server rather than deriving them

Both surfaces reconstructed the dataset list by grouping table names
client-side. The server answers it directly, so the grouping goes."
```

---

### Task 3: The drill-down

**Files:**
- Rewrite: `lib/citylake/web/src/pages/DatasetDetailPage.tsx`
- Create: `lib/citylake/web/src/pages/ModulePage.tsx`
- Delete: `lib/citylake/web/src/pages/LodTablePage.tsx`

**Interfaces:**
- Consumes: `describeDataset`, `queryObjects`, `deleteObject`, `dropDataset`, `DatasetInfo`, `ModuleInfo`, `ObjectRow` from Task 1.
- Produces: the `/datasets/:ds/modules/:module` route Task 4 registers.

- [ ] **Step 1: Rewrite the dataset detail page**

It reads `describeDataset(ds)` and shows, for that dataset: its name, its declared CRS (`crs`, which is `null` when the package states none — render that as "not stated" rather than blank, because the two mean different things), and a row per module giving the module name, its `role` and its `rows`. Each **object** module links to `/datasets/${ds}/modules/${module.name}`; sidecar modules are listed but not linked, since there is no browser for them.

Include a drop action for the dataset, using the existing `AlertDialog` component for confirmation — it is destructive and takes everything with it. On success, invalidate the `["datasets"]` query and navigate to `/datasets`.

- [ ] **Step 2: Create the module page**

`ModulePage.tsx` replaces `LodTablePage.tsx`. Read `LodTablePage` before writing it: keep its filter form, its page-size constant, its column inference from the returned rows, its cell formatting, its table layout and its delete confirmation. Change four things:

1. It is keyed on `{ ds, module }` from the route, not a single `tableName`.
2. It calls `queryObjects(ds, module, { filter, limit, offset })`, which returns a **bare array** — there is no `count` field, so pagination cannot show a total. Page forward while a full page comes back and stop when a short page arrives; disable "next" on a short page.
3. Delete calls `deleteObject(ds, id)` and the confirmation must say that deleting an object **also deletes its children**, because the server cascades. The response's `deleted` count says how many went; surface it.
4. **Drop the edit/update dialog entirely.** The spec's scope names delete and not update, and the server's update takes a map of attribute columns rather than a CityJSON document, so the old dialog's meaning no longer exists. Editing belongs to the later piece of work that gives the remaining endpoints a UI.

Then delete `LodTablePage.tsx`.

- [ ] **Step 3: Verify**

```bash
cd lib/citylake/web
bash -lc 'npx tsc --noEmit -p tsconfig.app.json' 2>&1 | grep -E "DatasetDetailPage|ModulePage|LodTablePage"
```
Expected: no output. A lingering `LodTablePage` reference means Task 4's route table has not caught up yet — that is expected until Task 4.

- [ ] **Step 4: Commit**

```bash
git add lib/citylake/web/src/pages/DatasetDetailPage.tsx lib/citylake/web/src/pages/ModulePage.tsx
git rm lib/citylake/web/src/pages/LodTablePage.tsx
git commit -m "feat(web): browse a dataset's modules and their objects

A dataset detail lists its module tables with roles and row counts; a
module page pages through that module's objects. Deleting an object
cascades to its children, so the confirmation says so and the count it
reports is the whole subtree."
```

---

### Task 4: Routes, upload, and the gate

**Files:**
- Rewrite: `lib/citylake/web/src/pages/UploadPage.tsx`
- Modify: `lib/citylake/web/src/App.tsx`

**Interfaces:**
- Consumes: everything from Tasks 1-3.
- Produces: a client that builds.

- [ ] **Step 1: Re-point the upload page**

It takes a dataset name and a file, and calls `uploadDataset(name, file)`. The LoD selector goes — there is no LoD to choose, because a dataset holds every level its source carries. Keep the file input, the validation that a name and a file are both present, the error surfacing and the success navigation; send the user to `/datasets/${name}` on success, since the server returns that dataset's `DatasetInfo`.

The dataset name is a schema name server-side and is validated against `[a-zA-Z0-9_]+`, so reject anything else in the form with a message saying which characters are allowed, rather than letting the server answer 400.

- [ ] **Step 2: Update the routes**

In `App.tsx`, replace the `tables/:tableName` route with `datasets/:ds/modules/:module`, pointing at `ModulePage`, and rename the detail route's parameter from `:base` to `:ds` to match. The final shape:

```tsx
        <Route index element={<Navigate to="/datasets" replace />} />
        <Route path="datasets" element={<DatasetsPage />} />
        <Route path="datasets/:ds" element={<DatasetDetailPage />} />
        <Route path="datasets/:ds/modules/:module" element={<ModulePage />} />
        <Route path="upload" element={<UploadPage />} />
        <Route path="*" element={<Navigate to="/datasets" replace />} />
```

- [ ] **Step 3: Run the whole gate**

```bash
cd lib/citylake/web
bash -lc 'vp check'
bash -lc 'npx tsc --noEmit -p tsconfig.app.json'
bash -lc 'vp build'
```
Expected: all three clean. A type error means a surface still speaks the old contract —
and note that `vp check` will pass regardless, since it does not type-check this project,
so the `tsc` line is the one carrying the weight.

- [ ] **Step 4: Confirm nothing references the removed model**

```bash
grep -rnE "listTables|TableInfo|LodTablePage|groupByBase|_lod_|lod" src/ --include=*.tsx --include=*.ts
```
Expected: no hits outside comments about levels of detail as a domain concept. A hit in live code is a surface that was missed.

- [ ] **Step 5: Commit**

```bash
git add lib/citylake/web/src/pages/UploadPage.tsx lib/citylake/web/src/App.tsx
git commit -m "feat(web): route on datasets and modules, and upload without a LoD

A dataset holds every level its source carries, so there is nothing for
an upload to choose. The name is validated in the form against the same
character class the server enforces on a schema name."
```

---

## Self-Review

**Spec coverage.** §4's `api.ts` re-key → Task 1, with the response shapes read from the handlers rather than assumed. §4's four pages → Tasks 2-4; `LodTablePage` deleted in Task 3. §4's "Supabase authentication and `ProtectedRoute` untouched" → no task touches `auth/`, `LoginPage` or `AuthCallbackPage`. §4's gate → Task 4, adapted below. §2's minimum-viable scope → the module page drops the edit dialog, and no task adds UI for validate, reconcile, vacuum, merge, package, export or compact.

**One addition to the spec's gate, forced by measurement.** The spec names `vp check` and
`vp build`. Both are used here — but `vp check` was measured not to type-check this project
(a deliberate type error passed it, while `tsc` caught it), because the local `vite-plus`
package is absent and it skips that step. A broken re-point IS a type error, so the gate
gains `npx tsc --noEmit`. This also means the repository's own `web-check` recipe, which is
`vp check`, does not currently type-check the web client — worth raising separately, and
outside this plan's scope to change.

**A fourth surface the spec's file list missed.** §4 names `api.ts` and four pages. `AppShell.tsx` also calls `listTables` and carries its own copy of `groupByBase`, so it is a fifth file and is in Task 2. Had it been left, the sidebar would have silently rendered nothing.

**Placeholders.** None. `api.ts` is complete; the page tasks specify behaviour, the exact data they read, and what to preserve, with the existing pages as the style reference — transcribing 900 lines of React into a plan would be less reliable than pointing at the file that already sets the idiom.

**Type consistency.** `DatasetInfo`/`ModuleInfo`/`ObjectRow` are defined in Task 1 and used in Tasks 2-4. Route parameters are `:ds` and `:module` throughout, matching what `ModulePage` and `DatasetDetailPage` read from `useParams`.

**No tests in this piece, deliberately.** The project has no test infrastructure for `web/` and the spec gives behaviour verification to piece C, which drives a real browser. Adding a unit-test harness here would be piece C's work done early and worse. The gate is the type check and the build.
