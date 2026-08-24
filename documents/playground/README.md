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
lib/          duckdb (boot + extensions), query (run + Arrow), bytes, share
components/   Editor, PresetList, ResultsTable, SchemaPanel, StatusBar
```

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
    "ExposeHeaders": [
      "Content-Length",
      "Content-Range",
      "Accept-Ranges",
      "ETag",
      "Last-Modified"
    ],
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
