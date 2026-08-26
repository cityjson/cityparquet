# The SQL playground

A DuckDB-Wasm console at `/playground`, querying CityParquet on object storage
from the reader's own browser. The design note behind it is
[`ai/design-notes/specs/2026-08-24-sql-playground-design.md`](../../ai/design-notes/specs/2026-08-24-sql-playground-design.md).

It is a React island mounted by [`../pages/playground.astro`](../pages/playground.astro).
Blume enables `@astrojs/react` as soon as the project contains a `.tsx`, so there
is nothing to configure; `client:only` keeps DuckDB away from the server render,
which it would not survive.

```
config.ts     the data host, the extension source, the caps and the deadline
presets.ts    the example queries
lib/          duckdb (boot + extensions), serialise, query (run + Arrow),
              completion, saved, files, bytes, share
components/   Editor, EditorToolbar, PresetList, SavedList, ImportedFiles,
              ResultsTable, SchemaPanel, StatusBar
```

## One statement at a time

Everything goes through `Session.query`, and nothing calls `connection.query`
directly. There is one connection into one WebAssembly instance, and two
statements in flight on it do not queue — they interleave and corrupt the
engine's heap. It surfaces as `RuntimeError: memory access out of bounds` or
`null function` from _both_ statements, which reads like a broken query rather
than a broken instance.

This became load-bearing when completion arrived: the page now asks questions of
its own while the reader is running theirs.

The queue has one consequence worth knowing. `QUERY_TIMEOUT_MS` gives up on
_waiting_ for a statement, not on the statement, so a read that never completes
— the CORS case above — leaves the queue held behind it, and the schema panel
and completion go quiet for the rest of the session. The reader still gets the
message that says what went wrong; the tab does not recover on its own.
`cancelSent()` is not the fix: the worker is inside the statement and will not
read the cancellation until it ends.

## Running it

```sh
pnpm dev        # then open /playground
pnpm run build  # what CI runs
npx vitest run playground/
```

## The data host must expose its range headers

This is the one piece of configuration that lives outside the repository, and
the failure it causes is silent, so it is worth stating plainly.

DuckDB-Wasm's worker reads `Accept-Ranges`, `Content-Length`, `Content-Encoding`,
`Etag` and `Last-Modified` with `getResponseHeader`. Cross-origin, a response
header is invisible to JavaScript unless it is named in
`Access-Control-Expose-Headers` — with the exception of the CORS-safelisted ones,
which include `Content-Length` and `Last-Modified` but **not** `Accept-Ranges`,
`Content-Range` or `ETag`.

A host can therefore answer `206` to every range request and still be unusable
from a browser, because the reader cannot see that ranges are supported. The
symptom is not an error: the query simply never finishes, or drags the whole
object down. The playground's per-query deadline exists to turn that into a
message rather than an indefinite spinner.

The bucket needs:

```json
[
  {
    "AllowedOrigins": ["*"],
    "AllowedMethods": ["GET", "HEAD"],
    "AllowedHeaders": ["range", "if-match"],
    "ExposeHeaders": ["Content-Length", "Content-Range", "Accept-Ranges", "ETag", "Last-Modified"],
    "MaxAgeSeconds": 3600
  }
]
```

Check any host with:

```sh
curl -sI -H 'Origin: https://cityjson.github.io' <url> | grep -i access-control
```

`access-control-expose-headers` must be present, and must name `Accept-Ranges`.

## Testing against unpublished extension code

By default the playground loads `cityjson` and `three_d` from the DuckDB
community repository. Those builds lag this project's sources considerably — at
the time of writing the published `cityjson` exposed 4 functions against 15 in
the source tree — so anything recently added needs a local build.

Build `wasm_eh` in each extension repository. **`wasm_eh`, not `wasm_mvp`**: a
browser instance is `eh`, an `eh` instance loads only `eh` extensions, and the
`mvp` bundle cannot report an error message at all.

```sh
cd ../../lib/duckdb-cityjson
just wasm-setup            # once: emsdk + vcpkg into .vendor/ (~2 GB)
just wasm wasm_eh

cd ../duckdb-3d
just wasm-setup            # once
just wasm                  # eh is this repo's default
```

Collect them into a DuckDB extension repository laid out as
`<root>/<duckdb-version>/<platform>/<name>.duckdb_extension.wasm`. `public/ext/`
is gitignored, so nothing built this way is ever committed:

```sh
cd documents
mkdir -p public/ext/v1.5.4/wasm_eh
cp ../lib/duckdb-cityjson/build/wasm_eh/extension/cityjson/cityjson.duckdb_extension.wasm \
   ../lib/duckdb-3d/build/wasm_eh/extension/three_d/three_d.duckdb_extension.wasm \
   public/ext/v1.5.4/wasm_eh/
```

Then point the playground at it:

```sh
PUBLIC_EXT_REPOSITORY=/ext pnpm dev
```

That switch also turns on `allowUnsignedExtensions`, which locally built
artefacts need — they are not signed by DuckDB Labs.

The version directory must match the DuckDB version the `duckdb-wasm` pin
carries: `1.33.1-dev57.0` carries DuckDB `v1.5.4`. The pin is exact for that
reason, and should not be widened to a range.

Production deploys use the published community builds. Once the registry catches
up, the override is only needed for testing functions that have not shipped yet.

## Completion

Typing offers three things: SQL keywords, from `@codemirror/lang-sql`; the
columns of the files the statement reads; and every function the engine knows,
the `cityjson` and `three_d` ones included. After a dot, a `STRUCT` column
offers its fields instead — `bbox.` gives `xmin`, `address[1].` gives `street`.

The columns cannot be declared up front the way `lang-sql`'s own schema
completion expects, because there are no table names: a source is
`read_parquet('https://…/building.parquet')`, and it changes as the reader
types. `lib/completion.ts` therefore reads the `FROM` and `JOIN` clauses out of
the document and asks DuckDB to `DESCRIBE` each one, caching the answer against
the exact expression that produced it — failures included, or a half-typed URL
would be a request per keystroke.

That `DESCRIBE` reads the Parquet footer, which for the national file is 4.7 MB.
It is fetched once per distinct source per session, and nearly every preset
reads the same file, so a session pays for it about twice. It is warmed on the
same 700 ms debounce the schema panel uses, and the completion source waits
1.5 s for it before offering what it already has — a popup that hangs is worse
than one missing its columns for a keystroke.

## Saved queries

`Save in browser` keeps the current query in `localStorage` under
`cityparquet:playground:saved`, and the sidebar's **Saved** tab lists them. The
prefix is not decoration: GitHub project pages share one origin, so
`cityjson.github.io` is every other cityjson project's storage as well.

`lib/saved.ts` is handed its `Storage` rather than reaching for `localStorage`,
which keeps it testable outside a browser and makes the failure modes explicit.
They are real ones — a page in private mode throws from the _accessor_, not just
the write — and all of them degrade to a list that is correct for the session
and simply does not outlive it. The stored shape carries a `version`, and
anything written by another version is dropped rather than trusted.

## Local files

`Import file` registers a file from the reader's machine with DuckDB. Nothing is
uploaded; the formats and their readers are in `lib/files.ts`.

**How a file is registered depends on its reader, and that is not a detail.**
Parquet gets `registerFileHandle`, so DuckDB pulls ranges out of the browser's
file handle on demand — the same access pattern a remote package gets, which is
what makes opening a local 16 GB package unremarkable. The text readers get
`registerFileBuffer` instead: given a lazy handle, `read_cityjsonseq` does not
fail, it **never returns**, and since every statement shares one queue that
takes the page with it. The import `DESCRIBE` is bounded for the same reason.

Two limits of the published extension build, both observed in the browser rather
than assumed:

- **`read_flatcitybuf` is not in it.** Importing a `.fcb` registers the file and
  reports the catalog error. It needs a local build — see above.
- **The city writers report rows and produce nothing.** `COPY … (FORMAT
cityjsonseq)` answers `Count: 20` and leaves DuckDB-Wasm's virtual filesystem
  empty; the same statement writes 1 MB as CSV. So `exportQuery` compares the
  row count `COPY` claims against the bytes it can read back, and refuses rather
  than handing over an empty file. Parquet, CSV and JSON are unaffected.

An export runs the query **again** — `COPY` takes a statement, not a result set
— so it writes every row the query matches, not the `ROW_DISPLAY_CAP` the grid
is holding. The menu says so, and it takes the same deadline as any query.

## Adding a preset

Add an entry to `presets.ts`. `id` is stable — it appears in share links, and
documentation points at it, so renaming one breaks existing links.

Run the query before listing it, and put any caveat in the blurb rather than
leaving it for the reader to discover. Two that matter for the 3DBAG package:

- Attributes are on `Building` rows and geometry is on `BuildingPart` rows, with
  nothing on both. Anything needing both joins through `parents` / `children`.
- Wildcards cannot work over plain HTTPS, because listing a directory needs the
  storage API. Multi-file reads need an explicit `read_parquet([…])` list.

The tests in `playground.test.ts` check the invariants that are easy to get
wrong: unique URL-safe ids, declared extensions matching the functions actually
called, and no unbounded `SELECT *` against the 16.4 GB file.
