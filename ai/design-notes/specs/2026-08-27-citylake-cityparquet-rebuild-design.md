# CityLake on CityParquet packages — a rebuild

**Date:** 2026-08-27
**Subject:** Rebuilding `lib/citylake` on the CityParquet package model and the
`cityparquet_*` pragmas that duckdb-cityjson v0.4.0 added.

## 1. Why

`lib/citylake` was written before the specification settled and before the
CityJSON extension grew a package API. It encodes a model the specification no
longer has, and hand-rolls operations the extension now performs.

**The storage model is wrong.** CityLake stores **one table per LoD** —
`buildings_lod_2_2`, `buildings_lod_1_3` — discovered by scanning a source's
columns for a `geom_lodX_Y` pattern and validated through a `LodKey` newtype.
The specification says a dataset is one table **per CityGML module**, with every
LoD of an object present as a column (`geometry_lod2_2`) on that object's single
row. The two models disagree about what a row is.

That disagreement has a visible cost. `tasks.md` records multi-LoD round-trip
export as deferred, because re-stitching per-LoD tables into one CityJSON
document means joining on `id`/`feature_id` across tables and projecting the
result back through the extension. Under the module model the problem does not
exist: an object's LoDs are already columns of one row, so exporting it is a
plain `SELECT`.

**The operations are hand-rolled.** `update_object` deletes and re-inserts
through a temporary file. `delete_object` is a flat `DELETE … WHERE id = ?` that
cannot cascade to children. `compact_table` is CTAS + `DROP` + `RENAME`, which is
not how a DuckLake table is compacted. Nothing derives `feature_id`, the
reciprocal `parents` / `children` / `children_roles` arrays, or `bbox` after an
edit; nothing checks CRS agreement on ingest; nothing knows that GeoParquet
legality flips as solids are inserted and deleted.

**The extension already does all of it.** duckdb-cityjson v0.4.0 — published to
community-extensions, so `INSTALL cityjson FROM community` reaches it — ships
`cityparquet_read`, `_init`, `_write`, `_merge`, `_delete`, `_reconcile`,
`_validate`, `_orphans`, `_vacuum`, and `insert_cityjson` / `insert_cityjsonseq`
/ `insert_flatcitybuf`. Between them they cover module routing, sidecar id
renumbering with reference rewriting, transitive cascade delete, derived-state
re-derivation, CRS preconditions, GeoParquet-legal footers and STAC Item
generation.

CityLake's own rule already says CityJSON I/O goes through the extension and
CityLake implements none of it. This rebuild extends that rule from *I/O* to
*the package*: CityLake orchestrates, the extension knows the encoding.

## 2. Scope

**In scope.** The `citylake` crate: its repository trait, its DuckDB/DuckLake
service, and its HTTP API. A fix to `cityparquet_write` in the `duckdb-cityjson`
submodule (§8). Documentation: a rewritten `CLAUDE.md` and byte-identical
`AGENTS.md`; removal of the stale `tasks.md` and `milestones.md`. Wiring
citylake into the root `just check`.

**Out of scope.** The React application under `web/`. It is keyed to the LoD
table model — `LodTablePage`, `/tables/:name_lod_X_Y` — and will not work
against the rebuilt API. It is left knowingly broken and re-pointed in a
follow-up.

**No backward compatibility.** There are no users. Names, endpoints and types
change wherever a better one exists; no shims, no deprecation path.

## 3. Approaches considered

**A dataset lives in DuckLake; CityParquet directories are the boundary.**
*(chosen)* A dataset is a DuckLake schema of module tables. Packages on disk are
what CityLake imports from and exports to. Keeps ACID, snapshots and DuckLake's
own compaction — the reason the monorepo describes CityLake as a lakehouse
runtime — and makes every mutation a single pragma inside a transaction.

**A dataset is a package directory; DuckDB is a query cache.** Spec-conformant
files on disk at every moment. Rejected: every edit becomes load, mutate and
rewrite of an entire package, with no transactions and no snapshots. It
discards the lakehouse and keeps only the file format.

**Write a package at ingest, then load it into DuckLake.** Conformant artifacts
immediately. Rejected: it doubles every write and leaves two copies of the same
dataset whose divergence becomes CityLake's problem to manage.

## 4. The model

A **dataset** is a CityParquet package, materialised as a DuckLake schema
`lake.<dataset>` holding:

- **object tables** named for their CityGML module — `building`, `bridge`,
  `tunnel`, `construction`, `transportation`, `vegetation`, `relief`,
  `water_body`, `land_use`, `city_furniture`, `generics`;
- **sidecars**, present only when the source has something to put in them —
  `materials`, `textures`, `geometry_templates`;
- **`__cityparquet`**, the extension's bookkeeping: one row per package file
  (`table_name`, `file_name`, `role`, `city`).

Addressing is **dataset + module**. The `_lod_X_Y` suffix disappears; LoDs are
columns. `src/core/db/lod.rs`, `LodKey` as a table-addressing type, and
`metadata_table.rs` are all deleted.

Identifier validation changes shape with it. A dataset name is still checked
against `[a-zA-Z0-9_]`, because it becomes a schema name and cannot be
parameterised. A module name is checked against the **closed set** above — a
stronger check than a character-class regex, and one the specification defines.

## 5. Crate structure

The three layers stay: `core/interface` (the trait and its types) →  `core/db`
(the DuckLake implementation) → `app` (axum). Handlers reach the database only
through `CityLakeRepository`.

```
src/core/db/
  service.rs      connection, extension loading, ATTACH, search_path scoping
  sql.rs          SQL construction, identifier and literal quoting — pure
  dataset.rs      create, list, describe, drop
  ingest.rs       insert_cityjson[seq|flatcitybuf]; the fresh-dataset bootstrap
  query.rs        SELECT with filter, limit, offset
  mutate.rs       attribute UPDATE, cityparquet_delete, cityparquet_reconcile
  package.rs      cityparquet_read (import), cityparquet_write (export), _merge
  inspect.rs      cityparquet_validate / _orphans / _vacuum, footers, metadata
  compaction.rs   DuckLake maintenance
```

`sql.rs` is new and load-bearing: it is the only place that builds SQL text, so
it is the only place quoting can go wrong, and it is pure — every function maps
arguments to a string with no database in sight. That is what makes the fast
half of the test strategy (§9) possible.

## 6. Mechanics

These four are not implementation detail; each one is a constraint that changes
what the code may do. All four were verified against the real extension binary
before this spec was written.

**One pragma per submitted statement.** DuckDB expands every pragma in a script
*before* executing any of it, so two pragmas submitted together each see the
catalog as it was before the batch. Two inserts in one submission whose sources
share an object id will not catch each other. CityLake therefore never places
two pragmas in one `execute_batch`.

**Scope with `SET search_path`, not `USE`.** The pragmas take a bare schema
name and resolve it through the caller's search path. `SET
search_path='lake.<dataset>'` scopes them to a DuckLake schema and is restored
afterwards, under the connection mutex. `USE` works too but leaves sticky
session state; `search_path` composes.

**Fresh datasets need a seed table.** There is no pragma that creates a package
from nothing: `insert_cityjson` on an empty schema fails with *"schema has no
CityParquet object table"*, and `create_tables = true` creates the *additional*
module tables a source needs, not the first one. The bootstrap is therefore:

```sql
CREATE SCHEMA lake.<ds>;
CREATE TABLE lake.<ds>.building AS
  SELECT * FROM read_cityjsonseq('<src>') LIMIT 0;   -- schema only, no rows
SET search_path='lake.<ds>';
PRAGMA cityparquet_init('<ds>');
PRAGMA insert_cityjsonseq('<ds>', '<src>', create_tables = true);
```

The reader and the insert pragma are chosen to match the source format —
`read_cityjson` / `insert_cityjson` for `.city.json`, the `*seq` pair for
`.city.jsonl`, the `*flatcitybuf` pair for `.fcb`. The insert then routes every
object to its module table and creates the sidecars the source needs. An empty object table produces no Parquet file on write, so a
seed that stays empty costs nothing on disk — but it remains registered in
`__cityparquet`. **Open point for the plan:** confirm whether dropping an empty
seed and re-running `cityparquet_init` removes its bookkeeping row, and do so if
it does.

**Multi-statement operations run in an explicit transaction.** `BEGIN` /
`COMMIT` around a pragma sequence is honoured under DuckLake: a
`cityparquet_delete` was observed to drop a row to zero and return it on
`ROLLBACK`, cascade and re-derivation included.

## 7. CRS

A DuckLake table has no Parquet footer, so `__cityparquet.city` is NULL for
every table CityLake creates. The extension's rule is that a destination
declaring nothing has nothing to check — which silently disables the CRS guard
on every ingest, and leaves `cityparquet_write` with no CRS to write.

The obvious repair — have CityLake write a `city` footer into
`__cityparquet` at dataset creation — is circular, and the spec records that so
the plan does not rediscover it. A footer's `crs` is **canonical PROJJSON**,
produced by the extension's own `ProjjsonForReferenceSystem` and re-dumped so
that key ordering matches (`cityparquet_insert.cpp:249`). That resolution is the
whole point of the check: comparing a source's `EPSG:7415` against a footer
directly "made EVERY insert into a package with a known CRS a bogus mismatch".
So producing a footer means resolving a CRS, which is exactly what §7 promises
CityLake will not do. `cityparquet_city_field` does not help — it reads a field
out of a footer, it does not build one — and the resolver has no SQL surface.

The real question for the plan is therefore narrower: **can CityLake obtain a
canonical footer without resolving anything itself?** Three candidates, in
order.

**Let the extension mint the footer** *(preferred)*. After the first ingest,
`cityparquet_write` the dataset to a temporary directory with `crs => <the
source's referenceSystem>`; the extension resolves it, writes it into the
Parquet footer, and `parquet_kv_metadata` reads that canonical text straight
back out for `UPDATE lake.<ds>.__cityparquet SET city = …`. One throwaway write
per dataset creation, and every byte of CRS handling stays inside the extension.
It depends on the §8 fix landing first.

**Expose the resolver upstream** *(alternative)*. A scalar
`cityparquet_projjson(reference_system)` would reduce the above to one call.
Natural company for the §8 fix, but it widens the upstream change from a bug fix
to an API addition, so it is an option rather than the plan.

**CityLake records the CRS itself** *(fallback)*. A `lake.__citylake_datasets`
table holds the source's `referenceSystem` verbatim and passes it to
`cityparquet_write` as `crs =>`. Exports stay correct. Ingest-time agreement,
though, then has to be checked in Rust against unresolved spellings, so
`EPSG:7415` and `urn:ogc:def:crs:EPSG::7415` compare unequal — a false mismatch
that refuses a legitimate insert. If it comes to this, the limitation is
documented, not papered over.

Under all three, CityLake never resolves or compares CRS representations
itself.

## 8. The `cityparquet_write` fix

`cityparquet_write` is a table function, not a pragma, because its output
depends on committed data — so it runs its queries on a **second connection**
opened against the same database (`src/cityjson/cityparquet_write.cpp:446`).
That connection has never seen the caller's `USE` or `search_path`, so its
default catalog is `memory`, and the two-part names it emits — `SELECT COUNT(*)
FROM pkg.building` — resolve to a schema that catalog does not have.

This is a plain qualification bug, not a DuckLake one: it reproduces against a
bare `ATTACH '/tmp/plain.db' AS plain` with no DuckLake loaded. It is invisible
until the schema lives in an attached catalog. Every pragma is unaffected,
because a pragma's generated SQL runs on the caller's own connection — which is
also why DuckLake needs no special handling anywhere else in this design: it
intercepts ordinary DML and rewrites it into its own layout, and the extension
never learns it is there.

The fix is to emit three-part `catalog.schema.table` names, resolving the
catalog from the caller's context — which the function's *bind* phase already
does correctly (`Catalog::GetEntry(context, …)`, lines 147 and 180), which is
why schema inspection succeeds and only the runtime queries fail. It lands in
the submodule's own repository, with a regression test that writes a package
from a schema in an attached catalog.

Reimplementing the pragmas in Rust was considered and rejected. They carry
module routing, sidecar renumbering, cascade semantics, derived-state
re-derivation, GeoParquet footer legality and STAC generation; a second
implementation of the encoding is the coupling this project's rules exist to
prevent.

## 9. Testing

`DuckLakeService::new_for_testing()` skips extension loading to keep the suite
offline and fast. Under this design every operation is a pragma, so that mode
would exercise nothing. It is removed, and replaced by two tiers:

- **Unit tests** over `sql.rs` — construction, quoting, identifier and module
  validation. Pure functions, no database, sub-millisecond.
- **Integration tests** against a real DuckDB with the real extension, loaded
  either from `INSTALL cityjson FROM community` or from a local build via
  `CITYLAKE_CITYJSON_EXTENSION`, over committed fixtures.

The suite's present "57 offline sub-second tests" property does not survive
this. That is a deliberate trade: those tests passed without the component that
now does the work.

## 10. HTTP API

Same shape, re-keyed from LoD tables to dataset and module, with the package
operations the extension makes available:

```
GET    /datasets                                  list datasets
POST   /datasets/:ds            (+ /upload)       create from a source
GET    /datasets/:ds                              modules, row counts, CRS, metadata
DELETE /datasets/:ds                              drop
POST   /datasets/:ds/objects    (+ /upload)       ingest a further source
GET    /datasets/:ds/modules/:module/objects      query: filter, limit, offset
PUT    /datasets/:ds/objects/:id                  update attributes, then reconcile
DELETE /datasets/:ds/objects/:id                  delete by id, cascading
DELETE /datasets/:ds/objects?filter=              delete by predicate, cascading
POST   /datasets/:ds/export                       COPY TO cityjson / cityjsonseq / fcb
POST   /datasets/:ds/package                      cityparquet_write to a directory
POST   /datasets/:ds/merge                        cityparquet_merge from another dataset
POST   /datasets/:ds/validate                     cityparquet_validate findings
POST   /datasets/:ds/reconcile                    re-derive feature_id, hierarchy, bbox
POST   /datasets/:ds/vacuum                       orphans, then vacuum
POST   /datasets/:ds/compact                      DuckLake maintenance
GET    /health
```

Creating a dataset takes either a CityJSON/CityJSONSeq/FlatCityBuf file — the
bootstrap of §6 — or an existing **CityParquet package directory**, which loads
through `cityparquet_read` and brings its Parquet footers, and so its CRS, with
it. One endpoint covers both; the source's shape decides the path. **Open point
for the plan:** `cityparquet_read` creates the schema it loads into, and whether
it honours the caller's search path and creates that schema inside the attached
DuckLake catalog is unverified. If it does not, the import loads into the
default catalog and CityLake copies the tables across, which costs a copy but
keeps the footers.

Object ids are unique across a whole package — `parents`, `children` and
`feature_id` all resolve by bare id across files — so `:id` needs no module. An
optional `?module=` skips the cross-table scan.

`filter` on query and predicate-delete is a **SQL predicate supplied by the
caller**, interpolated into the generated statement — `cityparquet_delete` takes
a predicate string by design, and the query filter matches it. Table and schema
identifiers are validated (§4) because they cannot be parameterised, but a
predicate is not a value and cannot be bound. The API has no authentication.
Both together mean callers are trusted: this is a research runtime on a trusted
network, not an internet-facing service, and that is a recorded decision rather
than an oversight. Putting it behind anything public needs authentication and a
restricted predicate grammar first.

Two behaviours are new and worth stating plainly. **Delete cascades**
transitively through `children`, where the old endpoint deleted one row; it
walks the hierarchy, never `feature_id` equality, so deleting a `BuildingPart`
does not take out the `Building` that shares its `feature_id`. **Export is
whole-dataset**, and genuinely multi-LoD, because an object's LoDs are columns
of one row. Exporting a dataset that spans several modules requires a
`UNION ALL BY NAME` across module tables whose schemas differ; per-module export
is the simple case and is what the plan builds first.

## 11. Version pinning

The extension's distribution pipeline builds for **DuckDB v1.5.4**
(`MainDistributionPipeline.yml`), which is also what the rest of the stack pins.
duckdb-rs versions as `1.105XX.0` for DuckDB `1.5.XX`, so the crate pin moves
from `=1.10501.0` to **`=1.10504.0`**. It stays exact and stays in lockstep with
the extension's release matrix: a mismatch does not degrade, it fails `LOAD
cityjson`.

## 12. Documentation

`lib/citylake/CLAUDE.md` documents the LoD table model in detail — the naming
convention, the `geom_lodX_Y` discovery scan, the key SQL patterns, the
endpoint list. All of it describes something that will not exist. It is
rewritten to describe the package model, and copied byte-identically to
`AGENTS.md` per the monorepo convention.

`tasks.md` and `milestones.md` are removed. They record a Postgres-and-Supabase
era, a completed milestone list, and a deferred multi-LoD export that this
design makes moot. History belongs in git.

`lib/citylake` is wired into the root `just check`, which today contains no
citylake reference despite the monorepo's `CLAUDE.md` describing it as the third
Cargo workspace.
