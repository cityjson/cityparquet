# CLAUDE.md

Crate orientation for **CityLake** — a lakehouse runtime for CityParquet
packages.

## What CityLake is

A CityLake **dataset** is a CityParquet package living as a schema in a
DuckLake catalog: one table per CityGML module the package carries
(`building`, `bridge`, `tunnel`, `construction`, `transportation`,
`vegetation`, `relief`, `water_body`, `land_use`, `city_furniture`,
`generics`), optional sidecars (`materials`, `textures`,
`geometry_templates`), and the extension's own bookkeeping table,
`__cityparquet`, which records each table's role (`object` or sidecar). The
service opens one in-memory DuckDB database on startup and `ATTACH`es one
DuckLake catalog to it, named `lake` by default; every dataset is a schema
inside that attached catalog, distinct from the in-memory database's own
default (`memory`) catalog — a distinction the CRS footer section below
depends on.

CityLake itself is a thin Rust layer: a connection, a set of SQL statement
builders, and an axum HTTP API on top. It holds no CityJSON parser, no
geometry code and no CRS logic of its own.

## The rule: everything through duckdb-cityjson

Every CityJSON operation *and* every package operation goes through the
`cityjson` DuckDB extension, called as SQL pragmas and table functions. No
CityJSON parsing, no CityGML-module routing, no CRS resolution and no
derived-state computation happens in Rust. The pragmas in use, and what each
is for:

| Pragma / function | For |
| --- | --- |
| `insert_cityjson` / `insert_cityjsonseq` / `insert_flatcitybuf` | Bootstrap or extend a package from a CityJSON / CityJSONSeq / FlatCityBuf source, routed to the right module table by the extension. `create_tables = true` lets a source bring further module tables the dataset has not seen yet. |
| `cityparquet_init` | Register a freshly seeded schema as a CityParquet package. |
| `cityparquet_read` | Load an existing package directory into a schema, recovering each file's Parquet footer (CRS included). |
| `cityparquet_write` | Write a schema out as a package directory, minting the footer. |
| `cityparquet_delete` | Delete by predicate, cascading transitively through `children`. |
| `cityparquet_reconcile` | Re-derive what a structural edit invalidates: `feature_id`, the reciprocal hierarchy, bbox. |
| `cityparquet_validate` | Run every structural check the extension knows, materialised into `cityparquet_validation`. |
| `cityparquet_orphans` / `cityparquet_vacuum` | Find, then reclaim, unreferenced sidecar rows. |
| `cityparquet_merge` | Fold one package's schema into another's — identity, the one-CRS rule and sidecar renumbering are all the pragma's. |
| `cityparquet_city_field` | Read one field back out of a table's footer (`crs`, in practice). |
| `cityjson_metadata` / `cityjsonseq_metadata` / `flatcitybuf_metadata` | A source's declared metadata, `referenceSystem` included. |
| `ducklake_merge_adjacent_files` | DuckLake's own compaction — not CityParquet's, but the mechanism `compact_impl` uses to merge an object table's small Parquet files without rewriting it behind DuckLake's back. |

`src/core/db/sql.rs` builds every statement above that names a dataset or
module — the pragmas, the package operations, the object reads — which is
where quoting and identifier validation matter: identifiers (schema, table
names) cannot be parameterised, so they are validated and double-quoted
through `sql.rs`'s own `ident`/`qualified` helpers, and values go through a
literal-escaping helper that doubles apostrophes. `sql.rs` touches no
database, which is what makes it unit testable without one. A handful of
fixed introspection and DDL statements — the export `COPY TO`, the attribute
`UPDATE`, `ATTACH`, `DROP SCHEMA`, both footer reads, and the
`information_schema`/`COUNT(*)` introspection queries — are built inline at
their call sites instead, reusing `sql.rs`'s quoting helpers where they touch
an identifier but assembling the surrounding statement themselves.

## File structure

```
src/
├── lib.rs
├── main.rs
├── core/
│   ├── interface/
│   │   ├── repository.rs   # CityLakeRepository trait — every operation, object-safe
│   │   └── types.rs        # DatasetName / ModuleName (validated newtypes), CityLakeError, DTOs
│   └── db/
│       ├── sql.rs              # every statement naming a dataset or module, pure
│       ├── service.rs          # the connection: with_connection, scoped, with_search_path, in_transaction
│       ├── dataset.rs          # create / list / describe / drop, and the CRS-minting probe
│       ├── ingest.rs           # insert a further source into an existing dataset
│       ├── query.rs            # paginated object reads
│       ├── mutate.rs           # attribute update, delete, reconcile
│       ├── package.rs          # import / write / export / merge a package
│       ├── inspect.rs          # validate, vacuum
│       ├── compaction.rs       # ducklake_merge_adjacent_files
│       └── repository_impl.rs  # the trait impl, off the async executor
└── app/
    ├── server.rs            # the axum router, 19 routes
    └── handlers/
        ├── dataset.rs
        ├── objects.rs
        ├── package.rs
        └── maintenance.rs
```

`DatasetName` and `ModuleName` (`src/core/interface/types.rs`) are
constructed only through `new`, which validates against the SQL builder's own
rules — a dataset name against `[a-zA-Z0-9_]+`, a module name against the
closed set of CityGML modules and sidecars. There is no public field and no
`Deserialize` impl, so an unvalidated string cannot reach `sql.rs` through
either type; a handler validates at the HTTP boundary by calling `new` and
nowhere else.

`CityLakeRepository` (`src/core/interface/repository.rs`) is the trait every
handler calls through — `Arc<dyn CityLakeRepository>`, so it stays
object-safe. `repository_impl.rs` hands every method to
`tokio::task::spawn_blocking`: the DuckDB connection sits behind a
`std::sync::Mutex` and its operations are CPU-bound, so running them on the
async executor would let one slow ingest stall every other request.
`DuckLakeService::handle()` shares the connection's `Arc` rather than moving
`self`, so the blocking task sees the same catalog the caller does.

`app/mod.rs` maps `CityLakeError` to an HTTP status exactly once, via
`IntoResponse` — `DatasetNotFound`/`ModuleNotFound`/`ObjectNotFound` to 404,
`DatasetExists` to 409, a malformed `Sql` newtype construction to 400, and a
pragma's own refusal (a duplicate id, a CRS mismatch, an unresolved parent,
a reprojection request) to 422. Everything else is 500. Doing this once, as a
`match` on an enum, is why `CityLakeError` is an enum rather than a boxed
trait object: a handler cannot classify a string.

## The four mechanics

**One pragma per submitted statement.** DuckDB expands every pragma in a
multi-statement script before executing any of it, so batching two pragmas in
one `execute_batch` call has each one see pre-batch state rather than the
other's effect. Every call site in `src/core/db/` submits exactly one pragma
per `execute_batch`.

**`SET search_path`, not `USE`.** The package pragmas take a bare schema
argument and resolve it by search path, so scoping a call to one dataset (or,
for a merge, two at once) means setting `search_path`, never `USE`.
`DuckLakeService::with_search_path` (`service.rs`) is the one place that
happens: it sets the path, runs the closure, and resets it — on success *and*
on failure, because leaving it set would silently resolve the next operation
against this dataset, and a reset failure is logged rather than allowed to
mask the body's own error. `scoped` is the convenience wrapper for the common
case of one dataset on a connection the caller does not already hold;
`with_search_path` itself takes a connection directly so it can nest inside a
transaction, where `scoped` cannot go.

**The seed table.** No pragma builds a package from nothing:
`insert_cityjson` on an empty schema fails with "schema has no CityParquet
object table", and `create_tables = true` creates the *further* module tables
a source needs, not the first one. So `create_dataset_impl` creates one
object table from the source's inferred schema and no rows (`LIMIT 0` — a
seeded row would be a row the insert then duplicates, and an empty object
table yields no Parquet file on write) before calling `cityparquet_init` and
then `insert_cityjson`/`insert_cityjsonseq`/`insert_flatcitybuf` with
`create_tables = true` for everything after the seed.

**Transactions under DuckLake.** `DuckLakeService::in_transaction` wraps a
closure in `BEGIN`/`COMMIT`, rolling back on failure — so a delete's cascade,
survivor cleanup and re-derivation commit or fail as one unit. What holds
under DuckLake and constrains where a transaction can start and end: a
DuckLake table has no Parquet footer of its own, `cityparquet_write` reads
only committed state, and one transaction may write to only one attached
database. Those two facts are why `dataset.rs`'s CRS-minting probe runs
outside the ingest transaction — see below.

## The CRS footer

A DuckLake table is not a Parquet file, so it has no footer, and
`__cityparquet.city` — the row a package's CRS is read from — is `NULL` on a
freshly ingested dataset. The extension's CRS guard reads that field; a
`NULL` guard is silently off, not silently permissive, so a fresh dataset
needs a real footer minted before its guard means anything.

The footer's `crs` is canonical PROJJSON, resolved by the extension's own CRS
resolver — CityLake never assembles or guesses it. So minting one means
asking the extension to do it: write a single row out as a throwaway package
and read the footer `cityparquet_write` put on it back out with
`parquet_kv_metadata` and `json_extract`. `dataset.rs::mint_crs_footer` does
exactly that — one probe schema, one object table with one row copied from
whatever module in the new dataset already has data, one `cityparquet_write`
naming the source's declared CRS (`reference_system.authority || ':' ||
reference_system.code`, read from the source's own `*_metadata()` — a struct
field concatenation, not CRS logic), and one read of `parquet_kv_metadata`'s
`city` key back out. Only `crs` is kept from that footer: the guard reads
that field alone, and a minimal value carries no stale probe inventory. The
probe schema is dropped afterwards regardless of outcome.

This has to happen in three phases, outside the ingest transaction, because
of the two constraints above: `cityparquet_write` sees committed state only,
so the ingest must already be committed before the probe can run against it;
and a transaction that has written to the lake catalog may not also write to
the in-memory default catalog, where the probe schema lives. So
`create_dataset_impl` commits the ingest first, then mints the footer as a
separate step, then describes the finished dataset — and if minting fails
after a successful ingest, it drops the schema it just committed rather than
leaving a dataset whose CRS guard is silently off.

A source that declares no `referenceSystem` leaves the footer `NULL`, which
is the correct "CRS unknown" state, not a failure to fix.

This probe is the least obvious thing in the crate. It looks removable —
"just write the CRS we already have" — and is not: CityLake does not have a
CRS it can write; only the extension's resolver does, and the only channel
back from that resolver is a written-and-read-back footer.

## Version lockstep

`duckdb = "=1.10504.0"` pins DuckDB v1.5.4 — an exact version, not a caret
range, because `duckdb-rs` versions as `1.105XX.0` for DuckDB `1.5.XX` and the
cityjson extension's distribution pipeline builds for exactly one DuckDB
version at a time. A mismatch does not degrade gracefully; it fails `LOAD
cityjson`. Bump this in lockstep with the extension's release matrix, never
ahead of it.

## Testing

Two tiers. `cargo test --lib` runs the pure unit tests — `sql.rs`'s statement
builders, `types.rs`'s validated newtypes and defaults, `mutate.rs`'s value
binding — none of which open a database connection. Everything under
`tests/` is an integration test: each one opens a real `DuckLakeService`,
which loads the `cityjson` extension and attaches a throwaway DuckLake
catalog in a temp directory.

There is no offline mode. Every operation this crate performs is a pragma, so
a service running without the extension loaded would exercise nothing —
there is no code path that does real work without it. The published
community `cityjson` extension predates v0.4.0 and does not carry the
`cityparquet_*` pragmas the package model runs on, so the integration tests
need `CITYLAKE_CITYJSON_EXTENSION` pointing at a locally built
`cityjson.duckdb_extension` (`just -f lib/duckdb-cityjson/justfile build`
produces one). Without that variable set, `DuckLakeService::new` falls back
to installing the community build, and every pragma call fails outright.
`tests/extension_loads.rs` asserts the environment contract directly: the
connected DuckDB is v1.5.4, and every `cityparquet_*` pragma and every
`insert_*` pragma this crate calls is registered.

## The trust model

The API has no authentication, and CORS is permissive. Four surfaces act on
caller-supplied input directly, and the caller is trusted to have the rights
that implies:

- `source_path`, on dataset creation and ingest, names a path the **server**
  reads. The extension resolves `http(s)://` and `s3://` URLs as readily as
  local paths, so a caller chooses both which of the server's files are read
  and which hosts it contacts.
- `output_path` / `output_dir`, on export and package write, name a
  destination the **server** writes, replacing whatever is already there.
- `filter`, on query and predicate delete, is a SQL predicate interpolated as
  written — `cityparquet_delete` takes its predicate as a SQL fragment by
  design, so there is nothing to bind it to.
- the attribute object's **keys**, on object update (`update_object_impl`,
  `src/core/db/mutate.rs`), become column identifiers, quoted through
  `sql::ident` — so there is no injection, but they are the only identifiers
  in the crate not validated through a newtype. A caller can therefore write
  `id`, `parents`, `children`, `feature_id` or a `geometry_lod*` column
  through an endpoint documented as updating "attributes". A structural
  column written this way is re-derived by the reconcile that follows the
  update, but the endpoint does not restrict which columns a caller may name.

This belongs on a trusted network, run by people who already hold the rights
it exercises on their behalf. `src/app/handlers/mod.rs` states this in full;
exposing the API more widely would need authentication, a path policy
confining reads and writes to a configured root, and a restricted predicate
grammar — none of which exist today.

## `geometry_templates` orphans are not vacuumed

`vacuum_impl` runs `cityparquet_orphans` then `cityparquet_vacuum` inside one
transaction, and reclaims unreferenced sidecar rows for `materials` and
`textures`. `geometry_templates` is the exception: the extension's own
`cityparquet_validate.cpp` probes for template references on a connection of
its own, using two-part table names that do not resolve under an attached
catalog's search path, so the probe fails and contributes no term — the
"undeterminable" fallback fires instead. The effect is fail-safe by
construction: a `geometry_templates` orphan is missed, never deleted data. It
is the extension's limitation, and is not worked around here.

## `web/` is stale

`web/` is keyed to a table-per-LoD API this crate does not expose, and does
not build against the dataset/module model above. It awaits a follow-up.

## API

`src/app/server.rs` wires 19 routes to `src/app/handlers/`:

| Method | Path | Handler | Does |
| --- | --- | --- | --- |
| GET | `/health` | — | Liveness. |
| GET | `/datasets` | `dataset::list` | Every dataset's name. |
| POST | `/datasets/{ds}` | `dataset::create` | Bootstrap a dataset from a server-side `source_path` (file or CityParquet package directory). |
| GET | `/datasets/{ds}` | `dataset::describe` | A dataset's modules, roles, row counts and CRS. |
| DELETE | `/datasets/{ds}` | `dataset::drop_dataset` | Drop the schema, cascading. |
| POST | `/datasets/{ds}/upload` | `dataset::create_upload` | Multipart variant of create. |
| POST | `/datasets/{ds}/objects` | `objects::ingest` | Ingest a further source into an existing dataset. |
| DELETE | `/datasets/{ds}/objects` | `objects::delete_where` | Delete by SQL predicate, cascading. |
| POST | `/datasets/{ds}/objects/upload` | `objects::ingest_upload` | Multipart variant of ingest. |
| GET | `/datasets/{ds}/modules/{module}/objects` | `objects::query` | A bounded, filterable, ordered page of one module's objects as JSON. |
| PUT | `/datasets/{ds}/objects/{id}` | `objects::update` | Update one object's attributes, then reconcile. |
| DELETE | `/datasets/{ds}/objects/{id}` | `objects::delete` | Delete one object by id, cascading. |
| POST | `/datasets/{ds}/export` | `package::export` | Export one module to a single CityJSON-family file. |
| POST | `/datasets/{ds}/package` | `package::write_package` | Write the whole dataset out as a CityParquet package directory. |
| POST | `/datasets/{ds}/merge` | `package::merge` | Fold another dataset's schema into this one. |
| POST | `/datasets/{ds}/validate` | `maintenance::validate` | Run the extension's structural checks; report, do not repair. |
| POST | `/datasets/{ds}/reconcile` | `maintenance::reconcile` | Re-derive `feature_id`, hierarchy and bbox. |
| POST | `/datasets/{ds}/vacuum` | `maintenance::vacuum` | Reclaim unreferenced sidecar rows (see the `geometry_templates` caveat above). |
| POST | `/datasets/{ds}/compact` | `maintenance::compact` | Merge each object table's small Parquet files via DuckLake. |

## Dev commands

```bash
cargo build                       # library + binary (default features include `server`)
cargo build --no-default-features # library only, no axum/tower deps
cargo run                         # start the HTTP server
cargo test --lib                  # the pure tier, offline
cargo test                        # everything, needs CITYLAKE_CITYJSON_EXTENSION
cargo clippy --all-targets -- -D warnings
```

A `justfile` wraps the common workflows: `just` (no args) lists them. `just
test` is the pure tier; `just test-integration` is the integration tier and
needs `CITYLAKE_CITYJSON_EXTENSION` set, as above. `just check` is the clippy
gate.
`just api` starts the server with a dotenvx-loaded `.env`; `just web`/`just
dev` are `web/`'s toolchain, which the staleness note above applies to.

The root `justfile`'s `citylake-check` runs this crate's clippy gate and full
test suite as part of `just check` from the repository root, resolving
`CITYLAKE_CITYJSON_EXTENSION` from a local `lib/duckdb-cityjson` build if the
caller has not already exported it.
