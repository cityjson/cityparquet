# A SQL playground for CityParquet

**Date:** 2026-08-24
**Status:** design, awaiting review

A browser-hosted SQL console at `/playground` on the specification site, running
DuckDB-WASM with the `cityjson` and `three_d` extensions, querying CityParquet
on object storage. Its purpose is to make the argument the specification makes
in prose **executable**: that a 3D city model can be stored as static files and
queried at national scale from a browser tab, with no server and no download.

Every number in §10 was measured while writing this note, not assumed.

## 1. Where it lives

```
documents/
  playground/               React + TypeScript source
    Playground.tsx          island entry (client:only)
    config.ts               data + extension URLs — the file that changes per environment
    presets.ts              the preset registry
    lib/
      duckdb.ts             bundle selection, instantiation, extension loading
      query.ts              run, Arrow → rows, timing
      bytes.ts              the worker XHR shim and its byte counter
      share.ts              URL-hash encode/decode
    components/
      Editor.tsx  PresetList.tsx  ResultsTable.tsx  SchemaPanel.tsx  StatusBar.tsx
    README.md               how to build the dev extensions; the bucket CORS policy
  pages/
    playground.astro        mounts the island at /playground
```

The playground is a **Blume island**, not a separate Vite application. Blume
generates an Astro project and enables `@astrojs/react` the moment the project
contains any `.tsx` — its `detectNeedsReact` globs `**/*.{tsx,jsx}` from the
project root — so a component under `documents/playground/` turns React on with
no `islands/` folder and no `components.ts` entry. A custom page under
`documents/pages/` mounts at its own route and may import components relatively.

The consequences that made this the choice over a standalone app: one build, one
deploy, `.github/workflows/docs.yml` unchanged (it already triggers on
`documents/**`), and the site's theme, header, fonts and dark mode inherited
rather than reimplemented. Blume is Astro + Vite + React + TypeScript
underneath, so nothing about the stack is given up.

The island hydrates with `client:only="react"`. DuckDB-WASM touches `Worker` and
`window`, and must never be server-rendered.

Custom pages sit outside Blume's Markdown link rewriter, so root-absolute URLs
would 404 under the GitHub Pages project subpath. `playground.astro` prefixes
`import.meta.env.BASE_URL` with the same `url()` helper `pages/index.astro`
already uses.

## 2. Runtime

`@duckdb/duckdb-wasm` is pinned **exactly** to `1.33.1-dev57.0`, which carries
DuckDB `v1.5.4`. The pin is exact rather than a caret range for the reason
`benchmark/readbench` pins `fcb_core`: the extensions are built against a
specific DuckDB version, and a later release would silently change what loads.
`lib/duckdb-cityjson/test/wasm/smoke.mjs` already pins the same pairing.

The **`eh` bundle only**. The `coi` bundle needs `SharedArrayBuffer`, which needs
COOP/COEP response headers, which GitHub Pages cannot send. There is therefore
no threading.

`mainModule` and `mainWorker` must be **fully qualified URLs**, and this bit
twice before it was understood.

DuckDB's worker resolves `mainModule` against its own location rather than the
page's. A relative `./duckdb-dist/duckdb-eh.wasm` therefore produced a request
for `/duckdb-dist/duckdb-dist/duckdb-eh.wasm` and stalled with no error at all.

A **root-absolute** path fails too, which is the trap, because that is exactly
what Vite's `?url` import returns (`/_astro/duckdb-eh.<hash>.wasm`). It is
correct on the page and useless in the worker: the worker is created from a
`blob:` URL — required, so the byte counter can patch `XMLHttpRequest` before
DuckDB's script runs — and inside a blob worker `self.location` is the blob URL,
which gives a root-absolute path nothing sensible to resolve against. The
symptom is the same silence: the worker script is fetched, the WebAssembly
module never is, and the promise never settles.

Both URLs are therefore resolved with `new URL(url, location.href).href` on the
main thread, where the base is the real page. Instantiation is additionally
raced against the worker's `error` event and a deadline, so this class of
failure reports itself instead of hanging.

The WASM binaries are bundled from `node_modules` through Vite rather than
fetched from a CDN, so the site carries no third-party runtime dependency and
the version cannot drift underneath it.

## 3. Extensions

Two sources, chosen per environment through `config.ts`:

- **Production: the published community builds.** `INSTALL <name> FROM
  community; LOAD <name>;` resolves against
  `community-extensions.duckdb.org/v1.5.4/wasm_eh/`, which serves HTTPS with
  `Access-Control-Allow-Origin: *`.
- **Development: locally built `wasm_eh` artefacts** served from
  `documents/public/ext/`. That directory is **gitignored** — the binaries are
  not committed; `documents/playground/README.md` carries the build
  instructions instead.

The development path exists because the published builds are materially behind
the source in this repository:

| Extension | Published | Local source |
|---|---|---|
| `cityjson` | 4 functions, commit `d511bdb`, 194 commits behind | 14 functions, including `cityjson_geoparquet_geo`, `cityjson_materials`, `cityjson_textures`, `cityjson_wkb_extent`, `cityjson_geometry_templates` |
| `three_d` | 25 `ST_3D*` functions, commit `a08f240` | 40, adding `ST_3DAsText`, `ST_3DAsGeoJSON`, `ST_3DConvexHull`, `ST_3DFootprintArea`, `ST_3DTransform`, `ST_3DRotate*`, `ST_3DScale` |

Presets that depend on functions absent from the published builds are marked in
the registry with the extension version they need, and the playground reports a
clear message rather than a raw catalogue error when one is missing. Production
presets stay inside the published surface until the registry is refreshed.

Locally built artefacts are not signed by DuckDB Labs, so the development
configuration opens the database with `allowUnsignedExtensions: true`. The
production configuration does not need it.

Three further constraints govern the local builds:

- **`wasm_eh`, never `wasm_mvp`.** The `mvp` bundle references `_setThrew` and
  `__cxa_can_catch` without defining them, so the first C++ exception surfaces
  as `ReferenceError: _setThrew is not defined` rather than the real message —
  documented in `lib/duckdb-cityjson/docs/TRAPS.md`. In a SQL playground that
  makes every SQL error unreadable, which is disqualifying. `wasm_eh` is a
  target in `extension-ci-tools/makefiles/duckdb_extension.Makefile`; only
  `wasm_mvp` is currently wired into `just wasm`.
- **Both extensions need a `wasm_eh` recipe.** `lib/duckdb-cityjson` has
  `wasm-setup` and `wasm` (mvp only); `lib/duckdb-3d` has no WASM path at all.
  Both gain one, committed **in their own repositories**, per the submodule
  rule as amended in `ed5d02c`.
- **`ST_3DTransform` needs PROJ's EPSG database at runtime**
  (`proj_create_crs_to_crs` in `src/kernel/crs_transform.cpp`), which requires
  `proj.db` inside the Emscripten filesystem. The other 39 `ST_3D*` functions do
  not touch it. Presets avoid `ST_3DTransform`; making it work under WASM is out
  of scope here.

Extension loads are **serialised, with one retry**. Loading two community
extensions back to back, the second failed on its first attempt under headless
Chromium — observed in both orders, so it is positional rather than specific to
either extension. It may well not reproduce for real users; the retry is cheap
insurance either way.

## 4. Data

One constant in `config.ts`:

```ts
export const DATA_BASE_URL = "https://cityparquet.open3d.city/data";
```

The example dataset is `3dbag/building.parquet` — the whole of 3DBAG as a single
16.4 GB CityParquet file.

**Globs cannot work.** A `*` wildcard needs a directory listing, which requires
the storage API; a plain `https://` URL cannot list. Multi-file queries must use
an explicit `read_parquet([…])` list. Any preset that would have swept a
directory says so in its blurb — a demonstration that quietly drops its own
caveat is a defect.

**The bucket must expose range headers.** DuckDB-WASM's worker reads
`Accept-Ranges`, `Content-Length`, `Content-Encoding`, `Etag` and `Last-Modified`
through `getResponseHeader`. Cross-origin, a response header is invisible to
JavaScript unless named in `Access-Control-Expose-Headers` — except the
CORS-safelisted ones, which include `Content-Length` and `Last-Modified` but
**not** `Accept-Ranges`, `Content-Range` or `ETag`. Measured from inside a
browser page:

| Header | `cityjson.open3d.city` | `cityparquet.open3d.city` |
|---|---|---|
| `content-length` | `100` | `100` (safelisted) |
| `content-range` | `bytes 0-99/6605724` | `null` |
| `accept-ranges` | `bytes` | `null` |
| `etag` | readable | `null` |

Both hosts answer `206` to a range request; only one lets the browser see that
it did. With `Accept-Ranges` invisible, DuckDB cannot confirm range support, and
the likely fallback is to read the entire 16.4 GB object — consistent with the
observed behaviour, where a `DESCRIBE` against that file never returned.

The required policy, matching what `cityjson.open3d.city` already serves:

```json
[{
  "AllowedOrigins": ["*"],
  "AllowedMethods": ["GET", "HEAD"],
  "AllowedHeaders": ["range", "if-match"],
  "ExposeHeaders": ["Content-Length", "Content-Range", "Accept-Ranges", "ETag", "Last-Modified"],
  "MaxAgeSeconds": 3600
}]
```

The file's Parquet footer is **4,709,461 bytes (4.71 MB)**, read from its last
eight bytes. A first `DESCRIBE` therefore transfers about 4.7 MB before any
schema appears, which the interface must cover with a loading state rather than
a frozen panel.

## 5. Measuring bytes and time

Wall time is measured around the query. Bytes are harder and worth stating
plainly: DuckDB-WASM performs its HTTP inside the worker via `XMLHttpRequest`,
which main-thread instrumentation cannot observe.

The worker is therefore started from a small wrapper that patches
`XMLHttpRequest.prototype.send` to accumulate response sizes, then
`importScripts` DuckDB's own worker. This was verified during design: DuckDB
instantiates cleanly inside the patched blob worker and reports `wasm_eh`.

If the shim ever fails to bind, the readout **hides itself** rather than
displaying a number that might be wrong. Benchmark caveats are part of the
artefact, and a plausible-but-wrong figure is worse than no figure.

## 6. Interface

A preset list; a CodeMirror 6 editor with `@codemirror/lang-sql` and
Cmd/Ctrl+Enter to run; a results table with a display cap that reports "N of M";
a collapsible schema panel driven by `DESCRIBE`; and a status bar carrying rows,
wall time, bytes read and the loaded extension versions.

Styling uses the site's existing custom properties, so the `#7253ed` accent,
`sm` radius, Roboto and light/dark all come free.

Only patterns are taken from duck-ui (MIT) — its DuckDB connection lifecycle and
its Arrow-to-table handling. Its dependency tree (Monaco, web-llm, exceljs,
radix, framer-motion) is not.

Given a 16.4 GB single file, the row cap and `LIMIT`-first presets are not
polish: an unguarded `SELECT *` is an expensive mistake, and the bytes counter is
what makes that visible.

## 7. Presets

```ts
export type ExtName = "cityjson" | "three_d" | "httpfs" | "parquet";

export type Preset = {
  id: string;          // stable; appears in the share URL
  title: string;
  blurb: string;       // one line: what it demonstrates, and any caveat
  extensions: ExtName[];
  sql: string;
};
```

Seeded from the existing tutorials (`documents/docs/07-tutorials/`) and then
replaced by the maintainer's own queries. `id` is stable because documentation
deep-links to it.

Share links carry `#preset=<id>` while a preset is unmodified and
`#sql=<base64url>` once edited, so a tutorial can link "run this yourself" and
the common case stays short and readable.

## 8. Errors

DuckDB errors render verbatim in the results panel, not in a toast. Three cases
are distinguished, because they need different actions from the reader:
extension load failure, network or CORS failure, and SQL error.

Every query carries a **client-side deadline**. This is demonstrated necessary
rather than precautionary: a host that does not expose range headers produces
silence, not an error. A timeout surfaces as a pointed message naming the likely
cause; it must never present as an endless spinner.

## 9. Tests and gates

`pnpm run lint` and `pnpm run build` already gate `documents/` in CI and will now
typecheck and build the playground too. No workflow change is required.

New unit tests cover the pure logic: the share-link round-trip, and registry
invariants — unique ids, declared extensions within the known set, non-empty SQL,
and every URL under `DATA_BASE_URL`.

The browser path is verified manually. Nothing about it is claimed as working
until something has actually run it.

## 10. What was measured

| Claim | Evidence |
|---|---|
| Both extensions load in a real browser | `cityjson` 3.5 s, `three_d` 3.4 s, headless Chromium, `wasm_eh` |
| duckdb-wasm `1.33.1-dev57.0` carries DuckDB `v1.5.4` | reported by `db.getVersion()` |
| Community repo serves WASM over HTTPS with CORS | `200`, `Access-Control-Allow-Origin: *` |
| The byte-counting shim survives DuckDB's worker bootstrap | instantiated, reported `wasm_eh` |
| `mainModule` must be absolute | relative path produced a doubled path and a silent stall |
| The second back-to-back extension load fails first time | reproduced in both orders |
| The data bucket hides its range headers | `accept-ranges`, `content-range`, `etag` all `null` cross-origin |
| The 16.4 GB file's footer is 4.71 MB | decoded from the last 8 bytes |
| Node's duckdb-wasm HTTP runtime cannot fetch URLs | a canonical DuckDB-hosted URL also `404`s there; browser runtime is a separate implementation |
| A locally built `wasm_eh` cityjson loads in a browser | 178 ms, version `94ab4cb`, 15 functions, against 4 in the published build |
| `cityjson_geoparquet_geo` works there | returned PROJJSON CRS metadata for a remote CityJSONSeq; absent from the published build |
| The assembled page boots and runs queries | headless Chromium: engine + both local extensions loaded, then `2 rows · 3.36 s · 6.6 MB read` |
| The byte counter works in the built application | the 6.6 MB above came from the XHR shim, not an estimate |
| 3DBAG attribute/geometry split | `Building` 10,771,547 rows carry every `b3_*` and no geometry; `BuildingPart` 10,783,975 rows carry geometry and no attributes |
| The full package counts | 21,555,522 rows, in 4.7 s natively over the network |

## 11. Phasing

1. **Shell** — boot, editor, run, results, errors. Exercisable immediately
   against `cityjson.open3d.city`, which already serves correct CORS.
2. **Presets, share URLs, schema panel, bytes and timing.**
3. **Point at the real data** once the bucket exposes range headers; add links
   from the landing page and tutorials 01–04.

A map or 3D preview of returned geometry is deliberately out of scope. It is the
most compelling demonstration available and the largest piece of work, and it
belongs in its own note.

## 12. Open questions

- Whether the second-extension load failure reproduces outside headless
  Chromium. The retry makes it moot in practice.
- Whether `ST_3DTransform` can be made to work under WASM by shipping `proj.db`
  in the Emscripten filesystem, and whether that is worth its size.
- When the community registry catches up, whether the development extension
  override is retired or kept for testing unreleased functions.
