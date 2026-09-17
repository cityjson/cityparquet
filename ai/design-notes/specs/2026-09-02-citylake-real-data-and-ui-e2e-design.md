# Real-data server tests, a rebuilt web client, and UI end-to-end coverage

**Date:** 2026-09-02
**Subject:** Three follow-ups to the CityParquet rebuild of `lib/citylake`: server
tests against the real Delft datasets, re-pointing `web/` at the dataset/module
API, and end-to-end coverage of the resulting UI.

## 1. Why

The rebuild left three gaps, each real and each different in kind.

**Nothing has been tested at scale.** The suite's fixtures hold three or four
objects. That proves the *shape* of every operation — routing, cascade,
round trip, the CRS footer — and proves nothing about what happens to 2231. The
real Delft feed is published, the extension reads `https://` sources directly,
and no test uses either.

**The web client does not work.** It is keyed to the model the rebuild removed:
`TableInfo { name, base, lod }`, a `LodTablePage`, `/tables/{name}_lod_X_Y`
routes. It was scoped out deliberately and left knowingly broken.

**There is no end-to-end coverage at all.** Seventy-four tests exercise the
library and the HTTP layer; none drives a browser. The layer where a wrong field
name or a missing await shows up is the one layer with no tests.

## 2. Scope

Three pieces, sequenced **A, then B, then C**. A is independent and ships alone;
C requires B.

**A — real-data server tests.** A network-gated suite against the published Delft
CityJSONSeq and a CityParquet package.

**B — re-point `web/`.** The API client and the pages, at minimum-viable scope:
datasets list, dataset detail, upload and create, module object browse, delete.

**C — UI end-to-end.** Playwright, plus the test-only authentication bypass that
makes an automated run possible.

**Out of scope, explicitly.** No authentication is added to the Rust API. No UI
for validate, reconcile, vacuum, merge, package write, export or compact — those
are a later piece of work, recorded as such. The four input-trust surfaces stay
documented rather than constrained; that decision has not changed.

## 3. Piece A — real data

**Source.** `https://cityjson.open3d.city/cityjsonseq/delft.city.jsonl` — 6.6 MB,
2231 CityObjects, 1115 `Building` and 1116 `BuildingPart`, EPSG:7415. A real
CityParquet package is published at `https://cityparquet.open3d.city/data/delft/`;
the extension's own remote tests read it.

**Nothing is downloaded and nothing is committed.** The extension auto-loads
`httpfs` and resolves an `https://` source as readily as a local path, so the
test passes the URL to `create_dataset` and the remote read path is exercised for
free. 6.6 MB is far past what belongs in git, and the monorepo's convention is
already that anything worth measuring is fetch-scripted.

**Gating.** A new `lib/citylake/tests/real_data.rs`, skipped unless
`CITYLAKE_REAL_DATA=1`, mirroring the FCB remote tests that are skipped when
`FCB_REMOTE_TEST_URL` is unset. `just check` stays fast and works offline; a
dedicated recipe runs this suite deliberately.

**What it asserts.** That the ingest routes all 2231 objects into `building` and
the `Building`/`BuildingPart` split matches the published figures; that the CRS
resolves to 7415 rather than to nothing; that the package writes, re-imports and
validates clean at that size; and that a cascade delete of a real parent leaves
the package consistent. These are the claims the small fixtures cannot make.

**A limitation this piece records rather than fixes.** `create_dataset` decides
between the file bootstrap and `import_package` with
`std::path::Path::is_dir()`, which is **false for a URL**. So the hosted package
cannot be imported by URL at all: a caller passing that directory URL falls into
the file path and fails. This piece round-trips through a package it writes
locally, and URL package import becomes its own small follow-up — teaching the
detection about URLs is a change to the create path, and it deserves its own
test rather than riding along here.

**Runtime.** Minutes, not seconds. That is the reason for the gate.

## 4. Piece B — the web client

`src/lib/api.ts` is re-keyed: `DatasetInfo { name, modules, crs }` and
`ModuleInfo { name, role, rows }` replace `TableInfo`; the endpoints become
`/datasets`, `/datasets/{ds}`, `/datasets/{ds}/objects`,
`/datasets/{ds}/modules/{module}/objects`. Query parameters go through the
boundary DTO the API expects — `filter`, `limit`, `offset`.

Pages, minimum viable and chosen to match the end-to-end journey:

- **DatasetsPage** — the datasets, with create and drop.
- **DatasetDetailPage** — the dataset's modules, their roles and row counts, and
  its declared CRS.
- **ModulePage** — replaces `LodTablePage`; a paginated object browse for one
  module, with delete by id.
- **UploadPage** — create a dataset from an uploaded CityJSON-family file.

`LodTablePage` is deleted. Supabase authentication and `ProtectedRoute` are
untouched: this piece re-points the client at a different API, it does not
redesign the application.

Gate: `vp check` (oxlint, oxfmt, tsgo) and `vp build` clean.

## 5. Piece C — end-to-end, and the bypass

**The bottleneck is not what it appears.** The Rust API has no authentication —
it ignores the bearer token the client sends. The only gate is the Supabase login
on the UI routes. So what an automated run needs is a way past that login screen,
not a server-side token.

**The bypass, and why it is shaped this way.** `AuthContext` synthesises a session
instead of consulting Supabase when **both** hold: `import.meta.env.DEV` is true,
and `VITE_E2E_AUTH_BYPASS` is `1`. The second is the switch; the first is the
safety. Vite replaces `import.meta.env.DEV` with the literal `false` in a
production build, so the branch is constant-folded and eliminated — a `vp build`
artefact does not contain the bypass at all, and no environment variable can
enable it after the fact. A `VITE_`-prefixed flag alone would not have that
property, which is the whole reason for the pairing.

**Harness.** `@playwright/test` as a devDependency, with Playwright's `webServer`
starting the Rust server (with `CITYLAKE_CITYJSON_EXTENSION` set) and `vp dev`,
and tearing both down afterwards.

**The journey.** Past the login, upload a fixture and create a dataset, see it in
the list, open it, see the `building` module and its row count, browse the
objects, delete one and watch the count fall, drop the dataset.

**Small fixtures, deliberately.** The end-to-end run uses the committed
three-object fixtures, not the 6.6 MB feed. Piece A owns scale; piece C owns
whether the interface works. Putting a national dataset behind a browser test
buys nothing and costs minutes on every run.

## 6. Testing, as a whole

Four gates, each with a distinct job:

- `cargo test` — the existing 74, unchanged, offline apart from the extension.
- `cargo test --test real_data` with `CITYLAKE_REAL_DATA=1` — piece A, network.
- `vp check` and `vp build` — piece B.
- `npx playwright test` — piece C, driving both servers.

The root `just check` gains none of them: it stays the fast gate. Piece A and
piece C get their own recipes, because a gate that needs the network or a browser
is a gate people learn to skip.
