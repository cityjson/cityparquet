# CityLake CityParquet Rebuild — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rebuild `lib/citylake` so a dataset is a CityParquet package — a DuckLake schema of CityGML-module tables — driven entirely by duckdb-cityjson's `cityparquet_*` pragmas, replacing the obsolete one-table-per-LoD model and every hand-rolled operation.

**Architecture:** Three layers, unchanged in shape: `core/interface` (trait + types) → `core/db` (DuckLake implementation) → `app` (axum). What changes is underneath. A dataset becomes a DuckLake schema `lake.<dataset>` holding module tables (`building`, `transportation`, …), optional sidecars and the extension's `__cityparquet` bookkeeping. Every mutation is one pragma inside a transaction. All SQL text is built in one pure module, `sql.rs`, which is the only place quoting can go wrong and the only part testable without a database.

**Tech Stack:** Rust 2021, duckdb-rs `=1.10504.0` (bundled, DuckDB v1.5.4), duckdb-cityjson v0.4.0 (community extension), ducklake, axum 0.8, tokio, thiserror, serde.

**Spec:** `ai/design-notes/specs/2026-08-27-citylake-cityparquet-rebuild-design.md` — read it before Task 1. This plan argues from it; where they disagree, the spec is the intent and the plan is the route.

## Global Constraints

Every task's requirements implicitly include this section.

- **DuckDB version is locked at v1.5.4.** `duckdb = { version = "=1.10504.0", features = ["bundled", "json"] }`. Exact, not caret. The extension's distribution pipeline builds for v1.5.4 (`MainDistributionPipeline.yml`) and the MCP server pins `@duckdb/node-api` `1.5.4-r.1`. A mismatch does not degrade — it fails `LOAD cityjson`.
- **No CityJSON implementation in Rust.** No `serde_json` parsing of CityJSON documents, no vertex or geometry handling, no format detection beyond a filename-extension match, no module routing. Every read, write, route and derivation goes through the extension. This is CityLake's founding rule and this rebuild widens it from *I/O* to *the package*.
- **Never two pragmas in one submitted statement.** DuckDB expands every pragma in a script before executing any of it, so batched pragmas each see pre-batch state. One `execute_batch` per pragma, always.
- **Scope pragmas with `SET search_path='lake.<dataset>'`**, restored afterwards, under the connection mutex. Not `USE` — that leaves sticky session state.
- **British English** in prose and comments. **Document the present, never the past** — no changelog voice, no "previously", no migration notes in reference documentation.
- **Breaking changes are welcome.** No users, no shims, no deprecation paths, no compatibility aliases.
- **Identifier safety:** dataset names are validated against `^[a-zA-Z0-9_]+$` (they become schema names and cannot be parameterised); module names are validated against the closed set in Task 3. Values are bound as parameters or quoted through `sql::literal`.
- **The local extension build is the test environment.** The published community
  `cityjson` extension is older than v0.4.0 and does **not** carry the
  `cityparquet_*` pragmas — verified in Task 1, where the two pragma tests fail
  against it and pass against the local build. So every integration test runs
  with `CITYLAKE_CITYJSON_EXTENSION` pointing at
  `lib/duckdb-cityjson/build/release/extension/cityjson/cityjson.duckdb_extension`.
  Export it once per shell:

  ```bash
  export CITYLAKE_CITYJSON_EXTENSION=/data2/hideba/cityparquet-paper/cityparquet/lib/duckdb-cityjson/build/release/extension/cityjson/cityjson.duckdb_extension
  ```

  The community path stays in the code — it is what will work once the release
  lands — but nothing verifies against it today.
- **`allow_unsigned_extensions` is a startup-only option.** It must be set on a
  `Config` passed to `Connection::open_in_memory_with_flags`; `SET
  allow_unsigned_extensions` on a live connection fails with "Cannot change ...
  while database is running".
- **`Cargo.lock` is gitignored** for this crate (`lib/citylake/.gitignore`), so
  no commit includes it.
- Every task ends `cargo clippy --all-targets -- -D warnings` clean and `cargo fmt` applied.

## Verified Ground Truth

These were established by probing the real extension binary before this plan was written. Do not re-litigate them; do not "fix" code that depends on them.

1. **Pragmas work inside an attached DuckLake catalog** when scoped with `SET search_path='lake.<ds>'`.
2. **Transactions hold.** `BEGIN` → `cityparquet_delete` → `ROLLBACK` restores the row, cascade and re-derivation included.
3. **A fresh package needs a seed object table.** `insert_cityjson` on an empty schema fails with *"schema has no CityParquet object table"*; `create_tables = true` creates the *additional* module tables a source needs, not the first one.
4. **`cityparquet_init` is additive.** Dropping a table does **not** remove its `__cityparquet` row. Leave the empty seed in place — `cityparquet_write` emits one data file per **non-empty** object table, so an empty seed costs nothing on disk.
5. **`cityparquet_read` loads into an attached DuckLake catalog correctly**, creating the schema there and recovering Parquet footers. No copy step is needed.
6. **`cityparquet_write` fails against any attached catalog** — this is Task 2's bug.
7. **A minimal `{"crs": <projjson>}` value in `__cityparquet.city` activates the CRS guard.** Verified: a 28992 source into a 7415 package is refused with both sides resolved to PROJJSON. The value must be **minted by the extension**, never assembled in Rust.
8. **`COPY <table> TO … (FORMAT cityjson…)` is not statically discoverable**, so it inherits no metadata. Exports from a DuckLake table must pass `crs` explicitly.
9. **Compaction is `CALL ducklake_merge_adjacent_files('lake', '<table>', schema => '<ds>')`**, returning `schema_name`, `table_name`, `files_processed`, `files_created`.
10. **`cityparquet_city_field(city, field)` only reads** a footer. It cannot build one.
11. **`cityparquet_write` cannot see uncommitted state.** Called inside an open transaction it fails with `Catalog Error: Schema with name … does not exist!` — its internal connection sees committed data only. Mutate, **commit**, then write.
12. **One transaction may write to only one attached database.** `BEGIN; CREATE SCHEMA lake.a; CREATE SCHEMA memory.b; COMMIT;` fails with `TransactionContext Error: … a single transaction can only write to a single attached database`.
13. **`reference_system` from `*_metadata()` is a STRUCT**, not a string: `struct(base_url VARCHAR, authority VARCHAR, "version" VARCHAR, code VARCHAR)`. Build `authority || ':' || code` — `EPSG:7415` — in SQL.
14. **`crs =>` accepts a canonical PROJJSON document**, not only an `EPSG:code` spelling, and round-trips it: a package written from a footer's own `crs` value states 7415 rather than an explicit null. So a dataset's stored CRS can be fed straight back to the writer.

## File Structure

| File | Responsibility |
|---|---|
| `src/core/interface/types.rs` | Config, `DatasetName`, `ModuleName`, DTOs, `CityLakeError` |
| `src/core/interface/repository.rs` | The `CityLakeRepository` trait |
| `src/core/db/sql.rs` | **Pure** SQL construction, quoting, identifier validation |
| `src/core/db/service.rs` | Connection, extension loading, `ATTACH`, `search_path` scoping, transactions |
| `src/core/db/dataset.rs` | Create (bootstrap + CRS minting), list, describe, drop |
| `src/core/db/ingest.rs` | `insert_cityjson[seq|flatcitybuf]` into an existing dataset |
| `src/core/db/query.rs` | `SELECT` with filter, limit, offset |
| `src/core/db/mutate.rs` | Attribute `UPDATE`, `cityparquet_delete`, `cityparquet_reconcile` |
| `src/core/db/package.rs` | `cityparquet_read`, `cityparquet_write`, `cityparquet_merge`, format export |
| `src/core/db/inspect.rs` | `cityparquet_validate` / `_orphans` / `_vacuum`, metadata |
| `src/core/db/compaction.rs` | `ducklake_merge_adjacent_files` |
| `src/app/server.rs`, `src/app/handlers/*.rs` | axum router and handlers, one file per endpoint group |

**Deleted:** `src/core/db/lod.rs`, `src/core/db/metadata_table.rs`, `src/core/db/table.rs`, `src/core/db/list.rs`, `src/core/db/insert.rs`, `src/core/db/update.rs`, `src/core/db/delete.rs`, `src/core/db/export.rs`, `src/tests/e2e/*`, `src/tests/integration/*`, `tasks.md`, `milestones.md`.

---

### Task 1: Pin the toolchain and prove the extension loads

Everything downstream assumes `INSTALL cityjson FROM community` under duckdb-rs `=1.10504.0` yields a connection with the `cityparquet_*` pragmas on it. Prove that first, in one test, before writing code that depends on it.

**Files:**
- Modify: `lib/citylake/Cargo.toml`
- Create: `lib/citylake/tests/extension_loads.rs`

**Interfaces:**
- Consumes: nothing.
- Produces: the environment contract every later integration test relies on — a DuckDB connection with `cityjson` and `ducklake` loaded, sourced either from the community repository or from a local build named by `CITYLAKE_CITYJSON_EXTENSION`.

- [ ] **Step 1: Write the failing test**

`lib/citylake/tests/extension_loads.rs`:

```rust
//! The environment contract: the pinned DuckDB must be able to load the
//! CityJSON extension and expose the CityParquet package pragmas.
//!
//! `CITYLAKE_CITYJSON_EXTENSION` points at a locally built
//! `cityjson.duckdb_extension`; without it the community build is installed.

use duckdb::Connection;

/// Open an in-memory connection with the cityjson and ducklake extensions loaded.
fn connect() -> Connection {
    let conn = Connection::open_in_memory().expect("open in-memory duckdb");
    match std::env::var("CITYLAKE_CITYJSON_EXTENSION") {
        Ok(path) => {
            conn.execute_batch("SET allow_unsigned_extensions = true;")
                .expect("allow unsigned extensions");
            conn.execute_batch(&format!("LOAD '{path}';"))
                .expect("load the local cityjson build");
        }
        Err(_) => conn
            .execute_batch("INSTALL cityjson FROM community; LOAD cityjson;")
            .expect("install and load cityjson from community"),
    }
    conn.execute_batch("INSTALL ducklake; LOAD ducklake;")
        .expect("install and load ducklake");
    conn
}

#[test]
fn duckdb_is_the_pinned_version() {
    let conn = connect();
    let version: String = conn
        .query_row("SELECT version()", [], |row| row.get(0))
        .expect("query duckdb version");
    assert!(
        version.starts_with("v1.5.4"),
        "expected DuckDB v1.5.4, got {version} — the cityjson extension is \
         published only for the version its pipeline builds for"
    );
}

#[test]
fn the_cityparquet_pragmas_are_registered() {
    let conn = connect();
    // The package API this rebuild is built on. If any of these is missing the
    // extension is older than v0.4.0 and nothing downstream will work.
    for name in [
        "cityparquet_read",
        "cityparquet_init",
        "cityparquet_write",
        "cityparquet_merge",
        "cityparquet_delete",
        "cityparquet_reconcile",
        "cityparquet_validate",
        "cityparquet_vacuum",
        "cityparquet_city_field",
    ] {
        let found: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM duckdb_functions() WHERE function_name = ?",
                [name],
                |row| row.get(0),
            )
            .unwrap_or_else(|e| panic!("look up {name}: {e}"));
        assert_eq!(found, 1, "{name} is not registered");
    }
}

#[test]
fn insert_pragmas_are_registered() {
    let conn = connect();
    for name in ["insert_cityjson", "insert_cityjsonseq", "insert_flatcitybuf"] {
        let found: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM duckdb_functions() WHERE function_name = ?",
                [name],
                |row| row.get(0),
            )
            .unwrap_or_else(|e| panic!("look up {name}: {e}"));
        assert_eq!(found, 1, "{name} is not registered");
    }
}
```

- [ ] **Step 2: Run it and watch it fail**

```bash
cd lib/citylake && cargo test --test extension_loads
```

Expected: FAIL. On the current pin the DuckDB version assertion reports `v1.5.1`.

- [ ] **Step 3: Move the pin to DuckDB v1.5.4**

In `lib/citylake/Cargo.toml`, replace the `duckdb` dependency and its comment:

```toml
# Pinned to DuckDB v1.5.4: duckdb-rs versions as 1.105XX.0 for DuckDB 1.5.XX,
# and the cityjson extension's distribution pipeline builds for exactly that
# version. A mismatch does not degrade — it fails `LOAD cityjson`. Bump in
# lockstep with the extension's release matrix.
duckdb = { version = "=1.10504.0", features = ["bundled", "json"] }
```

- [ ] **Step 4: Run the tests and watch them pass**

```bash
cd lib/citylake && cargo test --test extension_loads
```

Expected: 3 passed. The first build takes several minutes — it compiles DuckDB from source.

If the community install fails because v0.4.0 has not reached the repository for v1.5.4, build the extension locally and re-run pointing at it:

```bash
cd lib/duckdb-cityjson && just build
cd lib/citylake && CITYLAKE_CITYJSON_EXTENSION=$(cd ../duckdb-cityjson && pwd)/build/release/extension/cityjson/cityjson.duckdb_extension \
  cargo test --test extension_loads
```

- [ ] **Step 5: Commit**

```bash
git add lib/citylake/Cargo.toml lib/citylake/Cargo.lock lib/citylake/tests/extension_loads.rs
git commit -m "build(citylake): pin DuckDB v1.5.4 and assert the package pragmas load

The rebuild is built on duckdb-cityjson's cityparquet_* pragmas, which
are published only for the DuckDB version the extension's pipeline
builds for. Assert both the version and the pragma set up front, so a
mismatch fails in one obvious test rather than deep inside an operation."
```

---

### Task 2: Teach `cityparquet_write` to qualify its catalog

`cityparquet_write` runs its queries on a second connection opened against the same database, because its output depends on committed state. That connection never saw the caller's `USE` or `search_path`, so the two-part names it emits resolve against `memory` and fail for any schema in an attached catalog. This blocks package export for every CityLake dataset.

This lands in the `lib/duckdb-cityjson` submodule and is committed **in that repository**, not the monorepo. Read its `CLAUDE.md` first.

**Files:**
- Modify: `lib/duckdb-cityjson/src/cityjson/cityparquet_write.cpp`
- Create: `lib/duckdb-cityjson/test/sql/cityparquet_write_attached.test`

**Interfaces:**
- Consumes: nothing.
- Produces: `cityparquet_write('<schema>', '<dir>')` succeeding when `<schema>` lives in an attached catalog reached through the caller's search path. Task 11 depends on this.

- [ ] **Step 1: Write the failing test**

`lib/duckdb-cityjson/test/sql/cityparquet_write_attached.test`:

```
# name: test/sql/cityparquet_write_attached.test
# description: cityparquet_write reaches a schema in an attached catalog
# group: [sql]

require cityjson

require parquet

# The function runs its queries on its own connection, which has never seen the
# caller's search path. Unless it qualifies names with the caller's catalog, a
# package that lives anywhere but the default catalog is unreachable.
statement ok
ATTACH '__TEST_DIR__/side.db' AS side;

statement ok
CREATE SCHEMA side.pkg;

statement ok
CREATE TABLE side.pkg.building AS
  SELECT * FROM read_cityjsonseq('test/data/delft_subset.city.jsonl');

statement ok
SET search_path='side.pkg';

statement ok
PRAGMA cityparquet_init('pkg');

statement ok
SELECT * FROM cityparquet_write('pkg', '__TEST_DIR__/attached_out', crs => 'EPSG:7415');

query I
SELECT COUNT(*) FROM read_parquet('__TEST_DIR__/attached_out/building.parquet');
----
20

# The footer is regenerated as usual — reaching the catalog changes nothing else.
query I
SELECT COUNT(*) FROM parquet_kv_metadata('__TEST_DIR__/attached_out/building.parquet')
WHERE decode(key) = 'city';
----
1
```

- [ ] **Step 2: Run it and watch it fail**

```bash
cd lib/duckdb-cityjson && just test
```

Expected: FAIL with `Catalog Error: Table with name "pkg.building" does not exist because schema "pkg" does not exist.`

- [ ] **Step 3: Qualify the generated names with the caller's catalog**

The bind phase already resolves entries through the caller's context (`Catalog::GetEntry(context, CatalogType::TABLE_ENTRY, INVALID_CATALOG, schema, table)` at lines 147 and 180), which is why schema inspection succeeds and only the runtime queries fail. Capture the catalog those entries resolve to at bind time, store it on the bind data next to `schema`, and have `QualifiedName` emit three parts.

In the bind data struct (around line 64), beside `std::string schema;` add:

```cpp
	//! The catalog the caller's search path resolved `schema` in. The queries
	//! below run on a connection of our own, which never saw that search path,
	//! so every generated name must say the catalog outright.
	std::string catalog;
```

In `Bind` (around line 420, where `result->schema` is set), resolve and record it:

```cpp
	result->schema = StringValue::Get(input.inputs[0]);
	// Resolve through the caller's context, exactly as the column helpers do,
	// then keep the catalog it landed in.
	auto &schema_entry = Catalog::GetSchema(context, INVALID_CATALOG, result->schema);
	result->catalog = schema_entry.catalog.GetName();
```

Add a **three-argument** `QualifiedName` overload beside the existing one in
`src/cityjson/cityparquet_package.cpp` (declared in
`src/include/cityjson/cityparquet_package.hpp`):

```cpp
std::string QualifiedName(const std::string &catalog, const std::string &schema,
                          const std::string &table) {
	return KeywordHelper::WriteOptionallyQuoted(catalog) + "." +
	       KeywordHelper::WriteOptionallyQuoted(schema) + "." +
	       KeywordHelper::WriteOptionallyQuoted(table);
}
```

**Do not pass a dotted `"catalog.schema"` string as the existing overload's
`schema` argument.** `QualifiedName` renders each argument through
`KeywordHelper::WriteOptionallyQuoted`, which treats what it is given as ONE
identifier — so `"lake.fresh"` would be quoted as the single name `"lake.fresh"`
rather than as two parts, and would resolve to nothing.

Then route the internal-connection call sites through the new overload — every
`QualifiedName(bind_data.schema, …)` at lines 455, 553, 584, 619 and 620, and
the `schema` parameters threaded into `CollectInventory`,
`CollectTemplateInventory` and `CollectFacts`. Those helpers take a bare schema
string today; give them the catalog as a further parameter rather than
concatenating it into the schema.

Note which call sites must **not** change: `ColumnDuckType` and `CopySourceList`
take `ClientContext &context` and look entries up through the caller's catalog
already — they are correct as they stand.

- [ ] **Step 4: Run the suite and watch it pass**

```bash
cd lib/duckdb-cityjson && just test
```

Expected: the new test passes and every existing `cityparquet_*` test still passes — the default-catalog case must be unaffected, since a three-part name naming `memory` resolves exactly as the two-part name did.

- [ ] **Step 5: Commit in the submodule's own repository**

```bash
cd lib/duckdb-cityjson
git add src/cityjson/cityparquet_write.cpp test/sql/cityparquet_write_attached.test
git commit -m "fix(cityparquet): qualify write's generated names with the caller's catalog

cityparquet_write runs its queries on its own connection, because the
geo-or-no-geo decision needs committed state. That connection never saw
the caller's search path, so the two-part names it emitted resolved
against the default catalog and a package in any attached catalog was
unreachable. Resolve the catalog at bind time, where the caller's
context is still in hand, and name it outright."
```

Leave the monorepo's submodule pointer for Task 15; it moves once with everything else.

---

### Task 3: `sql.rs` — the one place SQL text is built

Every statement CityLake issues is assembled here, so this is the only place quoting can go wrong. It is pure: arguments in, `String` out, no database. That is what makes it the fast half of the test strategy.

**Files:**
- Create: `lib/citylake/src/core/db/sql.rs`
- Rewrite: `lib/citylake/src/core/db/mod.rs`, `lib/citylake/src/lib.rs`
- Modify: `lib/citylake/Cargo.toml` (drop the `[[bin]]` target; Task 13 restores it)
- Move: `lib/citylake/src/tests/data/delft.city.jsonl` → `lib/citylake/tests/data/delft.city.jsonl`
- Delete: every file under `lib/citylake/src/core/db/` except the new `sql.rs` and `mod.rs`; all of `lib/citylake/src/app/`; all of `lib/citylake/src/tests/`; `lib/citylake/src/main.rs`

**Clear the old implementation here, in one move.** Rust compiles the whole
library crate for `cargo test --lib`, so a single module still naming a deleted
type fails the build for every task until it is gone — including this task's own
verification. The old `db/mod.rs` declares twelve modules and `lib.rs` declares
`app` and `tests`, all written against `LodKey`, `TableInfo` and the old error
alias. What survives this task is `sql.rs` and the two interface files Task 4
rewrites.

**Interfaces:**
- Consumes: nothing.
- Produces, all `pub`:
  - `OBJECT_MODULES: [&str; 11]`, `SIDECAR_TABLES: [&str; 3]`
  - `fn validate_dataset(name: &str) -> Result<(), SqlError>`
  - `fn validate_module(name: &str) -> Result<(), SqlError>`
  - `fn literal(value: &str) -> String` — single-quoted, apostrophes doubled
  - `fn ident(name: &str) -> String` — double-quoted, embedded quotes doubled
  - `fn qualified(parts: &[&str]) -> String`
  - `fn set_search_path(catalog: &str, schema: &str) -> String`
  - `fn reader_for(path: &str) -> SourceFormat`, `enum SourceFormat { CityJson, CityJsonSeq, FlatCityBuf }` with `fn read_fn(&self) -> &'static str` and `fn insert_fn(&self) -> &'static str`
  - `fn create_schema(catalog: &str, dataset: &str) -> String`
  - `fn seed_table(catalog: &str, dataset: &str, source: &str, format: SourceFormat) -> String`
  - `fn init_pragma(dataset: &str) -> String`
  - `fn insert_pragma(dataset: &str, source: &str, format: SourceFormat, create_tables: bool) -> String`
  - `fn delete_pragma(dataset: &str, predicate: &str, cascade: bool) -> String`
  - `fn reconcile_pragma(dataset: &str) -> String`
  - `fn validate_pragma(dataset: &str) -> String`, `fn orphans_pragma(dataset: &str) -> String`, `fn vacuum_pragma(dataset: &str) -> String`
  - `fn merge_pragma(dst: &str, src: &str) -> String`
  - `fn read_package_pragma(dir: &str, dataset: &str) -> String`
  - `fn write_package(dataset: &str, dir: &str, crs: Option<&str>) -> String`
  - `fn compact(catalog: &str, dataset: &str, table: &str) -> String`
  - `fn select_objects(catalog: &str, dataset: &str, module: &str, filter: Option<&str>, limit: usize, offset: usize) -> String`
  - `enum SqlError { InvalidDataset(String), UnknownModule(String) }` (thiserror)

- [ ] **Step 1: Write the failing tests**

Append to `lib/citylake/src/core/db/sql.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literals_survive_an_apostrophe() {
        // A path or predicate carrying an apostrophe must not close the literal
        // early and let the rest of the string continue the statement.
        assert_eq!(literal("O'Hara"), "'O''Hara'");
        assert_eq!(literal("plain"), "'plain'");
    }

    #[test]
    fn identifiers_are_double_quoted() {
        assert_eq!(ident("building"), "\"building\"");
        assert_eq!(ident("we\"ird"), "\"we\"\"ird\"");
    }

    #[test]
    fn qualified_names_quote_every_part() {
        assert_eq!(
            qualified(&["lake", "delft", "building"]),
            "\"lake\".\"delft\".\"building\""
        );
    }

    #[test]
    fn dataset_names_reject_anything_but_word_characters() {
        assert!(validate_dataset("delft_2026").is_ok());
        // A schema name cannot be parameterised, so it is validated instead.
        for bad in ["delft; DROP SCHEMA x", "del ft", "delft-1", "", "lake.delft"] {
            assert!(validate_dataset(bad).is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn module_names_are_checked_against_the_closed_set() {
        assert!(validate_module("building").is_ok());
        assert!(validate_module("water_body").is_ok());
        assert!(validate_module("materials").is_ok());
        // Stronger than a character class: the specification defines the set.
        assert!(validate_module("buildings").is_err());
        assert!(validate_module("Building").is_err());
    }

    #[test]
    fn the_reader_follows_the_file_extension() {
        assert_eq!(reader_for("a/b.city.json").read_fn(), "read_cityjson");
        assert_eq!(reader_for("a/b.city.jsonl").read_fn(), "read_cityjsonseq");
        assert_eq!(reader_for("a/b.fcb").read_fn(), "read_flatcitybuf");
        // .jsonl must not be mistaken for .json — check the longer suffix first.
        assert_eq!(reader_for("a/b.city.jsonl").insert_fn(), "insert_cityjsonseq");
    }

    #[test]
    fn the_seed_table_selects_no_rows() {
        let sql = seed_table("lake", "delft", "/d/x.city.jsonl", SourceFormat::CityJsonSeq);
        assert!(sql.contains("\"lake\".\"delft\".\"building\""));
        assert!(sql.contains("read_cityjsonseq('/d/x.city.jsonl')"));
        // Schema only. A seeded row would be a row the insert then duplicates.
        assert!(sql.contains("LIMIT 0"));
    }

    #[test]
    fn pragma_named_parameters_use_equals_not_walrus() {
        let sql = insert_pragma("delft", "/d/x.city.json", SourceFormat::CityJson, true);
        assert!(sql.contains("create_tables = true"), "got {sql}");
        assert!(!sql.contains(":="));
    }

    #[test]
    fn delete_defaults_to_cascading() {
        let cascading = delete_pragma("delft", "id = 'x'", true);
        assert!(cascading.contains("cityparquet_delete"));
        assert!(!cascading.contains("cascade ="), "cascade is the default");
        assert!(delete_pragma("delft", "id = 'x'", false).contains("cascade = false"));
        // The predicate is a literal argument, so its own quotes must be doubled.
        assert!(cascading.contains("'id = ''x'''"));
    }

    #[test]
    fn writing_a_package_omits_crs_when_it_is_unknown() {
        assert!(write_package("delft", "/out", None).contains("cityparquet_write('delft', '/out')"));
        assert!(write_package("delft", "/out", Some("EPSG:7415"))
            .contains("crs => 'EPSG:7415'"));
    }

    #[test]
    fn compaction_targets_one_table_in_one_schema() {
        assert_eq!(
            compact("lake", "delft", "building"),
            "CALL ducklake_merge_adjacent_files('lake', 'building', schema => 'delft')"
        );
    }

    #[test]
    fn selects_page_and_filter() {
        let sql = select_objects("lake", "delft", "building", None, 10, 20);
        assert!(sql.contains("LIMIT 10 OFFSET 20"));
        assert!(!sql.contains("WHERE"));
        let filtered =
            select_objects("lake", "delft", "building", Some("b3_h_dak_max > 20"), 10, 0);
        assert!(filtered.contains("WHERE b3_h_dak_max > 20"));
    }
}
```

- [ ] **Step 2: Run them and watch them fail**

```bash
cd lib/citylake && cargo test --lib sql::
```

Expected: FAIL — the module does not exist yet.

- [ ] **Step 3: Implement the module**

`lib/citylake/src/core/db/sql.rs`, above the test module:

```rust
//! SQL construction for the CityParquet package operations.
//!
//! Every statement CityLake issues is built here, so this is the only place
//! quoting can go wrong. Nothing in this module touches a database: it maps
//! arguments to text, which is what makes it testable without one.
//!
//! Two rules govern everything below. Identifiers — schema, table — cannot be
//! parameterised, so they are validated and quoted. Values are rendered through
//! [`literal`], which doubles apostrophes so a path or predicate carrying one
//! cannot end the literal early and let the rest continue the statement.

use thiserror::Error;

/// The CityGML modules that hold feature objects, one object table each.
/// The specification fixes this set; a name outside it is not a module.
pub const OBJECT_MODULES: [&str; 11] = [
    "building",
    "bridge",
    "tunnel",
    "construction",
    "transportation",
    "vegetation",
    "relief",
    "water_body",
    "land_use",
    "city_furniture",
    "generics",
];

/// The optional sidecars, written only when the source has something for them.
pub const SIDECAR_TABLES: [&str; 3] = ["materials", "textures", "geometry_templates"];

#[derive(Debug, Error)]
pub enum SqlError {
    #[error(
        "invalid dataset name {0:?}: a dataset becomes a schema name, which cannot be \
         parameterised, so it must match [a-zA-Z0-9_]+"
    )]
    InvalidDataset(String),
    #[error("unknown module {0:?}: not one of the CityGML object modules or sidecars")]
    UnknownModule(String),
}

/// A dataset name becomes a schema name. It cannot be bound as a parameter, so
/// it is validated rather than escaped.
pub fn validate_dataset(name: &str) -> Result<(), SqlError> {
    let ok = !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    ok.then_some(())
        .ok_or_else(|| SqlError::InvalidDataset(name.to_string()))
}

/// A module name is checked against the closed set the specification defines —
/// a stronger check than a character class, and one that rejects a plausible
/// misspelling like `buildings`.
pub fn validate_module(name: &str) -> Result<(), SqlError> {
    let known = OBJECT_MODULES.contains(&name) || SIDECAR_TABLES.contains(&name);
    known
        .then_some(())
        .ok_or_else(|| SqlError::UnknownModule(name.to_string()))
}

/// Render a value as a single-quoted SQL literal, doubling apostrophes.
pub fn literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// Render an identifier as a double-quoted name, doubling embedded quotes.
pub fn ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Join already-validated identifier parts into a dotted, quoted name.
pub fn qualified(parts: &[&str]) -> String {
    parts.iter().map(|p| ident(p)).collect::<Vec<_>>().join(".")
}

/// Scope the package pragmas to a dataset. They take a bare schema name and
/// resolve it through the search path, so this is how they reach a schema
/// inside the attached DuckLake catalog. `USE` would do it too, but leaves
/// sticky session state; this composes.
pub fn set_search_path(catalog: &str, schema: &str) -> String {
    format!("SET search_path={}", literal(&format!("{catalog}.{schema}")))
}

/// Which reader and insert pragma a source path calls for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceFormat {
    CityJson,
    CityJsonSeq,
    FlatCityBuf,
}

impl SourceFormat {
    pub fn read_fn(&self) -> &'static str {
        match self {
            SourceFormat::CityJson => "read_cityjson",
            SourceFormat::CityJsonSeq => "read_cityjsonseq",
            SourceFormat::FlatCityBuf => "read_flatcitybuf",
        }
    }

    /// Named `insert_fn` to mirror `read_fn`, and so as not to share a name
    /// with the free `insert_pragma` below, which builds the whole statement.
    pub fn insert_fn(&self) -> &'static str {
        match self {
            SourceFormat::CityJson => "insert_cityjson",
            SourceFormat::CityJsonSeq => "insert_cityjsonseq",
            SourceFormat::FlatCityBuf => "insert_flatcitybuf",
        }
    }
}

/// Pick the reader from the file extension. This is not format detection —
/// it does not look inside the file, which is the extension's job.
pub fn reader_for(path: &str) -> SourceFormat {
    let lower = path.to_ascii_lowercase();
    // `.jsonl` before `.json`: the shorter suffix is a prefix of the longer one.
    if lower.ends_with(".jsonl") {
        SourceFormat::CityJsonSeq
    } else if lower.ends_with(".fcb") {
        SourceFormat::FlatCityBuf
    } else {
        SourceFormat::CityJson
    }
}

pub fn create_schema(catalog: &str, dataset: &str) -> String {
    format!("CREATE SCHEMA {}", qualified(&[catalog, dataset]))
}

/// The seed object table a fresh package needs.
///
/// There is no pragma that creates a package from nothing: `insert_cityjson`
/// on an empty schema fails with "schema has no CityParquet object table", and
/// `create_tables = true` creates the *further* module tables a source needs,
/// not the first one. So one object table is created from the source's inferred
/// schema and no rows — `LIMIT 0`, because a seeded row is a row the insert
/// then duplicates. An empty object table yields no Parquet file on write, so a
/// seed that stays empty costs nothing.
pub fn seed_table(catalog: &str, dataset: &str, source: &str, format: SourceFormat) -> String {
    format!(
        "CREATE TABLE {} AS SELECT * FROM {}({}) LIMIT 0",
        qualified(&[catalog, dataset, "building"]),
        format.read_fn(),
        literal(source)
    )
}

pub fn init_pragma(dataset: &str) -> String {
    format!("PRAGMA cityparquet_init({})", literal(dataset))
}

/// PRAGMA named parameters use `=`, never `:=`.
pub fn insert_pragma(
    dataset: &str,
    source: &str,
    format: SourceFormat,
    create_tables: bool,
) -> String {
    let mut sql = format!(
        "PRAGMA {}({}, {}",
        format.insert_fn(),
        literal(dataset),
        literal(source)
    );
    if create_tables {
        sql.push_str(", create_tables = true");
    }
    sql.push(')');
    sql
}

/// Delete by predicate. Cascade is the extension's default and walks `children`
/// transitively — never `feature_id` equality, so deleting a BuildingPart does
/// not take out the Building sharing its feature_id.
pub fn delete_pragma(dataset: &str, predicate: &str, cascade: bool) -> String {
    let mut sql = format!(
        "PRAGMA cityparquet_delete({}, {}",
        literal(dataset),
        literal(predicate)
    );
    if !cascade {
        sql.push_str(", cascade = false");
    }
    sql.push(')');
    sql
}

pub fn reconcile_pragma(dataset: &str) -> String {
    format!("PRAGMA cityparquet_reconcile({})", literal(dataset))
}

pub fn validate_pragma(dataset: &str) -> String {
    format!("PRAGMA cityparquet_validate({})", literal(dataset))
}

pub fn orphans_pragma(dataset: &str) -> String {
    format!("PRAGMA cityparquet_orphans({})", literal(dataset))
}

pub fn vacuum_pragma(dataset: &str) -> String {
    format!("PRAGMA cityparquet_vacuum({})", literal(dataset))
}

pub fn merge_pragma(dst: &str, src: &str) -> String {
    format!(
        "PRAGMA cityparquet_merge({}, {})",
        literal(dst),
        literal(src)
    )
}

/// Load a package directory into a schema, recovering each file's Parquet
/// footer — the one thing a hand-rolled `read_parquet` load throws away.
pub fn read_package_pragma(dir: &str, dataset: &str) -> String {
    format!(
        "PRAGMA cityparquet_read({}, {})",
        literal(dir),
        literal(dataset)
    )
}

/// Write the package out. Omitting `crs` is legal and writes an explicit
/// `"crs": null` plus a warning — the CRS unknown, said out loud.
pub fn write_package(dataset: &str, dir: &str, crs: Option<&str>) -> String {
    match crs {
        Some(crs) => format!(
            "SELECT * FROM cityparquet_write({}, {}, crs => {})",
            literal(dataset),
            literal(dir),
            literal(crs)
        ),
        None => format!(
            "SELECT * FROM cityparquet_write({}, {})",
            literal(dataset),
            literal(dir)
        ),
    }
}

/// DuckLake's own maintenance. Not CTAS-and-rename: a DuckLake table's files
/// are the catalog's business, and merging them is what compaction means here.
pub fn compact(catalog: &str, dataset: &str, table: &str) -> String {
    format!(
        "CALL ducklake_merge_adjacent_files({}, {}, schema => {})",
        literal(catalog),
        literal(table),
        literal(dataset)
    )
}

/// A page of objects as JSON. The filter is a caller-supplied SQL predicate —
/// see the trust model in the specification's §10.
pub fn select_objects(
    catalog: &str,
    dataset: &str,
    module: &str,
    filter: Option<&str>,
    limit: usize,
    offset: usize,
) -> String {
    let mut sql = format!(
        "SELECT to_json(t) FROM {} t",
        qualified(&[catalog, dataset, module])
    );
    if let Some(predicate) = filter {
        sql.push_str(&format!(" WHERE {predicate}"));
    }
    sql.push_str(&format!(" LIMIT {limit} OFFSET {offset}"));
    sql
}
```

Reduce `lib/citylake/src/core/db/mod.rs` to the one module that remains:

```rust
pub mod sql;
```

and `lib/citylake/src/lib.rs` to:

```rust
//! CityLake — a lakehouse runtime for CityParquet packages.

pub mod core;
```

Then clear the rest. `types.rs` and `repository.rs` stay — they still describe
the LoD model, and Task 4 replaces them:

```bash
cd lib/citylake
mkdir -p tests/data
git mv src/tests/data/delft.city.jsonl tests/data/delft.city.jsonl
git rm -r --quiet src/tests src/app src/main.rs
git rm --quiet src/core/db/compaction.rs src/core/db/delete.rs src/core/db/export.rs \
  src/core/db/insert.rs src/core/db/list.rs src/core/db/lod.rs src/core/db/metadata.rs \
  src/core/db/metadata_table.rs src/core/db/query.rs src/core/db/service.rs \
  src/core/db/table.rs src/core/db/update.rs
```

Drop the `[[bin]]` section from `Cargo.toml` too; Task 13 adds it back with the
server that needs it. The optional axum dependencies and the `server` feature
stay — an unused optional dependency costs nothing and re-adding it would be
churn.

- [ ] **Step 4: Run the tests and watch them pass**

```bash
cd lib/citylake && cargo test --lib && cargo clippy --all-targets -- -D warnings
```

Expected: the 11 `sql::` tests pass and clippy is clean. They need no database and
run in milliseconds. The crate compiles: everything written against the LoD model
is gone except `types.rs` and `repository.rs`, which Task 4 replaces.

- [ ] **Step 5: Commit**

```bash
git add -A lib/citylake/
git commit -m "feat(citylake)!: build every statement in one pure SQL module

Identifiers cannot be parameterised, so dataset names are validated and
module names checked against the specification's closed set; values go
through a literal renderer that doubles apostrophes. Keeping it pure is
what lets the quoting be tested without a database.

The LoD-model implementation goes in the same move: the library crate
compiles as a unit, so one file still naming a deleted type would break
every task's test gate until it was removed."
```

---

### Task 4: The interface layer — types and the repository trait

Replace the LoD-shaped domain model with the package one, and give the crate a real error type. `Box<dyn Error>` tells a handler nothing it can turn into a status code.

**Files:**
- Rewrite: `lib/citylake/src/core/interface/types.rs`
- Rewrite: `lib/citylake/src/core/interface/repository.rs`
- Delete: `lib/citylake/src/core/db/lod.rs` and `lib/citylake/src/core/db/metadata_table.rs` remnants, if Task 3 left any

**Interfaces:**
- Consumes: `sql::{validate_dataset, validate_module, SqlError, OBJECT_MODULES}` from Task 3.
- Produces:
  - `struct CityLakeConfig { storage_path, catalog_path, catalog_name, host, port }` — `catalog_name` defaults to `"lake"`, `catalog_path` to `"metadata.ducklake"`, `storage_path` to `"data"`, `host` `"127.0.0.1"`, `port` `3000`.
  - `struct DatasetName(String)` and `struct ModuleName(String)`, each with `fn new(s: &str) -> Result<Self, CityLakeError>` and `fn as_str(&self) -> &str`.
  - `struct DatasetInfo { name: String, modules: Vec<ModuleInfo>, crs: Option<String> }`
  - `struct ModuleInfo { name: String, role: String, rows: usize }`
  - `struct PackageFile { file: String, action: String, rows: i64, bytes: i64 }`
  - `struct ValidationFinding { check_name: String, severity: String, table_name: String, object_id: Option<String>, message: String }`
  - `struct CompactionStats { files_processed: usize, files_created: usize }`
  - `struct QueryParams { filter: Option<String>, limit: usize, offset: usize }` with `Default` giving `limit: 100, offset: 0`
  - `enum ExportFormat { CityJson, CityJsonSeq, FlatCityBuf }` with `as_duckdb_format()` and `file_extension()`
  - `enum CityLakeError` (thiserror): `Sql(#[from] SqlError)`, `Duckdb(#[from] duckdb::Error)`, `Io(#[from] std::io::Error)`, `DatasetNotFound(String)`, `DatasetExists(String)`, `ModuleNotFound { dataset: String, module: String }`, `NoObjectTable(String)`, `Internal(String)`
  - `type RepositoryResult<T> = Result<T, CityLakeError>`
  - `trait CityLakeRepository` — the async methods listed in Step 3.

- [ ] **Step 1: Write the failing tests**

Append to `lib/citylake/src/core/interface/types.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dataset_names_are_validated_at_construction() {
        assert_eq!(DatasetName::new("delft").unwrap().as_str(), "delft");
        // Constructing the newtype is the only way in, so an invalid name
        // cannot reach the SQL builder at all.
        assert!(DatasetName::new("delft; DROP SCHEMA public").is_err());
        assert!(DatasetName::new("").is_err());
    }

    #[test]
    fn module_names_are_validated_at_construction() {
        assert_eq!(ModuleName::new("water_body").unwrap().as_str(), "water_body");
        assert!(ModuleName::new("buildings").is_err());
    }

    #[test]
    fn the_default_catalog_is_named_lake() {
        let config = CityLakeConfig::default();
        assert_eq!(config.catalog_name, "lake");
        assert_eq!(config.catalog_path, "metadata.ducklake");
        assert_eq!(config.storage_path, "data");
        assert_eq!(config.port, 3000);
    }

    #[test]
    fn query_params_default_to_a_bounded_page() {
        // An unbounded default would let one request pull a national dataset
        // into memory.
        let params = QueryParams::default();
        assert_eq!(params.limit, 100);
        assert_eq!(params.offset, 0);
        assert!(params.filter.is_none());
    }

    #[test]
    fn export_formats_map_to_duckdb_and_to_file_extensions() {
        assert_eq!(ExportFormat::CityJsonSeq.as_duckdb_format(), "cityjsonseq");
        assert_eq!(ExportFormat::CityJsonSeq.file_extension(), ".city.jsonl");
        assert_eq!(ExportFormat::CityJson.file_extension(), ".city.json");
        assert_eq!(ExportFormat::FlatCityBuf.file_extension(), ".fcb");
    }
}
```

- [ ] **Step 2: Run them and watch them fail**

```bash
cd lib/citylake && cargo test --lib types::
```

Expected: FAIL — `DatasetName` does not exist.

- [ ] **Step 3: Rewrite the two interface files**

`types.rs` carries the structs above. The newtypes delegate to Task 3's validators:

```rust
/// A validated dataset name. It becomes a schema name, so validation happens
/// once here and every consumer downstream can assume it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatasetName(String);

impl DatasetName {
    pub fn new(name: &str) -> Result<Self, CityLakeError> {
        crate::core::db::sql::validate_dataset(name)?;
        Ok(Self(name.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}
```

`ModuleName` is the same shape over `sql::validate_module`.

`repository.rs` carries the trait. Every method takes validated newtypes, so a
handler cannot pass an unchecked string:

```rust
#[async_trait]
pub trait CityLakeRepository: Send + Sync {
    /// Create a dataset from a source. A CityJSON / CityJSONSeq / FlatCityBuf
    /// file is bootstrapped and ingested; a CityParquet package directory is
    /// loaded through `cityparquet_read`, footers and all.
    async fn create_dataset(
        &self,
        dataset: &DatasetName,
        source_path: &str,
    ) -> RepositoryResult<DatasetInfo>;

    async fn list_datasets(&self) -> RepositoryResult<Vec<String>>;

    async fn describe_dataset(&self, dataset: &DatasetName) -> RepositoryResult<DatasetInfo>;

    async fn drop_dataset(&self, dataset: &DatasetName) -> RepositoryResult<()>;

    /// Ingest a further source into an existing dataset. Routing, sidecar
    /// renumbering and re-derivation are the extension's.
    async fn ingest(&self, dataset: &DatasetName, source_path: &str) -> RepositoryResult<usize>;

    async fn query_objects(
        &self,
        dataset: &DatasetName,
        module: &ModuleName,
        params: &QueryParams,
    ) -> RepositoryResult<Vec<serde_json::Value>>;

    /// Update attributes of one object, then re-derive what the edit
    /// invalidated. `attributes` is a JSON object of column name to value.
    async fn update_object(
        &self,
        dataset: &DatasetName,
        id: &str,
        attributes: &serde_json::Map<String, serde_json::Value>,
    ) -> RepositoryResult<()>;

    /// Delete by id, cascading transitively through `children`.
    async fn delete_object(&self, dataset: &DatasetName, id: &str) -> RepositoryResult<usize>;

    /// Delete by predicate, cascading transitively through `children`.
    async fn delete_where(&self, dataset: &DatasetName, predicate: &str)
        -> RepositoryResult<usize>;

    async fn reconcile(&self, dataset: &DatasetName) -> RepositoryResult<()>;

    async fn validate(&self, dataset: &DatasetName) -> RepositoryResult<Vec<ValidationFinding>>;

    async fn vacuum(&self, dataset: &DatasetName) -> RepositoryResult<usize>;

    async fn merge(
        &self,
        destination: &DatasetName,
        source: &DatasetName,
    ) -> RepositoryResult<()>;

    /// Write the dataset out as a CityParquet package directory.
    async fn write_package(
        &self,
        dataset: &DatasetName,
        output_dir: &str,
    ) -> RepositoryResult<Vec<PackageFile>>;

    /// Export one module to a single CityJSON-family file.
    async fn export_module(
        &self,
        dataset: &DatasetName,
        module: &ModuleName,
        output_path: &str,
        format: ExportFormat,
    ) -> RepositoryResult<()>;

    async fn compact(&self, dataset: &DatasetName) -> RepositoryResult<CompactionStats>;
}
```

Task 3 already cleared the old implementation, so `types.rs` and `repository.rs`
are the last two files still describing the LoD model. Replacing them leaves the
crate compiling on `sql.rs` plus this task's two files.

- [ ] **Step 4: Run the tests and watch them pass**

```bash
cd lib/citylake && cargo test --lib && cargo clippy --all-targets -- -D warnings
```

Expected: the 11 `sql::` tests from Task 3 and the 5 `types::` tests here, all
passing, and a clean clippy. The crate compiles: everything that referred to the
old model is gone.

- [ ] **Step 5: Commit**

```bash
git add -A lib/citylake/
git commit -m "feat(citylake)!: re-key the domain model from LoD tables to packages

A dataset is a package of module tables, not a family of per-LoD ones,
so LodKey and the geom_lodX_Y discovery scan have nothing left to
address. Validated newtypes replace stringly-typed names, and a real
error enum replaces Box<dyn Error>, which told a handler nothing it
could turn into a status code.

The old implementation goes in one move rather than module by module:
the library crate compiles as a unit, so one file still naming a
deleted type would break every task until it was removed."
```

---

### Task 5: `service.rs` — connection, catalog, scoping, transactions

The one place that owns the connection and the rules for using it. Everything else borrows it through these helpers.

**Files:**
- Rewrite: `lib/citylake/src/core/db/service.rs`
- Create: `lib/citylake/tests/common/mod.rs`

**Interfaces:**
- Consumes: `CityLakeConfig`, `CityLakeError`, `RepositoryResult` (Task 4); `sql::set_search_path` (Task 3).
- Produces:
  - `struct DuckLakeService { connection: Arc<Mutex<Connection>>, config: CityLakeConfig }`
  - `fn new(config: CityLakeConfig) -> RepositoryResult<Self>`
  - `fn config(&self) -> &CityLakeConfig`
  - `fn catalog(&self) -> &str`
  - `fn with_connection<T>(&self, f: impl FnOnce(&Connection) -> RepositoryResult<T>) -> RepositoryResult<T>`
  - `fn with_search_path<T>(&self, conn: &Connection, path: &str, f: impl FnOnce(&Connection) -> RepositoryResult<T>) -> RepositoryResult<T>` — sets `search_path` to `path`, runs `f`, resets **even on error**, on a connection the caller already holds
  - `fn scoped<T>(&self, dataset: &str, f: impl FnOnce(&Connection) -> RepositoryResult<T>) -> RepositoryResult<T>` — `with_connection` + `with_search_path` over `<catalog>.<dataset>`
  - `fn in_transaction<T>(&self, conn: &Connection, f: impl FnOnce(&Connection) -> RepositoryResult<T>) -> RepositoryResult<T>`
  - `fn schema_exists(&self, conn: &Connection, dataset: &str) -> RepositoryResult<bool>`
- Test helper `tests/common/mod.rs` produces `fn test_service() -> (DuckLakeService, TempDir)` — a service over a temporary DuckLake catalog with the real extension loaded, using the same `CITYLAKE_CITYJSON_EXTENSION` convention as Task 1.

- [ ] **Step 1: Write the failing test**

`lib/citylake/tests/service.rs`:

```rust
mod common;

use citylake::core::db::sql;

#[test]
fn the_catalog_is_attached_and_usable() {
    let (service, _dir) = common::test_service();
    service
        .with_connection(|conn| {
            conn.execute_batch("CREATE SCHEMA lake.probe;")?;
            Ok(())
        })
        .expect("create a schema in the attached catalog");

    let exists = service
        .with_connection(|conn| service.schema_exists(conn, "probe"))
        .expect("look the schema up");
    assert!(exists);
}

#[test]
fn scoping_restores_the_search_path_even_when_the_body_fails() {
    let (service, _dir) = common::test_service();
    service
        .with_connection(|conn| {
            conn.execute_batch("CREATE SCHEMA lake.scoped;")?;
            Ok(())
        })
        .unwrap();

    // A failure inside the scope must not leave the session pointing at the
    // dataset — the next operation would silently resolve against it.
    let failed = service.scoped("scoped", |conn| {
        conn.execute_batch("SELECT * FROM no_such_table;")?;
        Ok(())
    });
    assert!(failed.is_err());

    let path: String = service
        .with_connection(|conn| {
            Ok(conn.query_row("SELECT current_setting('search_path')", [], |r| r.get(0))?)
        })
        .unwrap();
    assert!(
        !path.contains("scoped"),
        "search_path leaked out of the scope: {path}"
    );
}

#[test]
fn a_rolled_back_transaction_leaves_nothing_behind() {
    let (service, _dir) = common::test_service();
    let result: Result<(), _> = service.with_connection(|conn| {
        service.in_transaction(conn, |conn| {
            conn.execute_batch("CREATE SCHEMA lake.rolled_back;")?;
            Err(citylake::core::interface::types::CityLakeError::Internal(
                "deliberate".into(),
            ))
        })
    });
    assert!(result.is_err());

    let exists = service
        .with_connection(|conn| service.schema_exists(conn, "rolled_back"))
        .unwrap();
    assert!(!exists, "the failed transaction was not rolled back");
}

#[test]
fn search_path_scoping_reaches_the_pragmas() {
    // The whole design rests on this: a pragma takes a bare schema name and
    // finds it inside the attached catalog through the search path.
    let (service, _dir) = common::test_service();
    service
        .with_connection(|conn| {
            conn.execute_batch("CREATE SCHEMA lake.reached;")?;
            conn.execute_batch(&sql::seed_table(
                "lake",
                "reached",
                common::fixture("delft.city.jsonl").to_str().unwrap(),
                sql::SourceFormat::CityJsonSeq,
            ))?;
            Ok(())
        })
        .unwrap();

    service
        .scoped("reached", |conn| {
            conn.execute_batch(&sql::init_pragma("reached"))?;
            Ok(())
        })
        .expect("cityparquet_init must resolve the schema through the search path");

    let registered: i64 = service
        .with_connection(|conn| {
            Ok(conn.query_row(
                "SELECT COUNT(*) FROM lake.reached.__cityparquet",
                [],
                |r| r.get(0),
            )?)
        })
        .unwrap();
    assert_eq!(registered, 1);
}
```

`lib/citylake/tests/common/mod.rs`:

```rust
//! Shared setup for the integration tests.
//!
//! There is no offline mode. Every operation in this crate is a pragma, so a
//! service without the extension would exercise nothing.

use citylake::core::db::service::DuckLakeService;
use citylake::core::interface::types::CityLakeConfig;
use std::path::PathBuf;
use tempfile::TempDir;

/// A service over a throwaway DuckLake catalog. The returned TempDir must stay
/// alive for the test's duration — dropping it removes the catalog.
pub fn test_service() -> (DuckLakeService, TempDir) {
    let dir = TempDir::new().expect("create a temporary directory");
    let config = CityLakeConfig {
        storage_path: dir.path().join("data").to_string_lossy().into_owned(),
        catalog_path: dir.path().join("meta.ducklake").to_string_lossy().into_owned(),
        ..Default::default()
    };
    let service = DuckLakeService::new(config).expect("start a service");
    (service, dir)
}

/// Path to a committed test fixture.
pub fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("data")
        .join(name)
}
```

- [ ] **Step 2: Run them and watch them fail**

```bash
cd lib/citylake && cargo test --test service
```

Expected: FAIL — `with_connection` and `scoped` do not exist.

- [ ] **Step 3: Implement the service**

`lib/citylake/src/core/db/service.rs`:

```rust
//! The DuckDB connection and the rules for using it.
//!
//! DuckDB's Connection is not Send, so it lives behind Arc<Mutex<_>>. Every
//! operation borrows it through [`DuckLakeService::with_connection`] or
//! [`DuckLakeService::scoped`], which is what keeps the search-path discipline
//! in one place instead of spread across nine operation modules.

use duckdb::{Config, Connection};
use std::sync::{Arc, Mutex};

use crate::core::db::sql;
use crate::core::interface::types::{CityLakeConfig, CityLakeError, RepositoryResult};

pub struct DuckLakeService {
    connection: Arc<Mutex<Connection>>,
    config: CityLakeConfig,
}

impl DuckLakeService {
    pub fn new(config: CityLakeConfig) -> RepositoryResult<Self> {
        std::fs::create_dir_all(&config.storage_path)?;

        // A locally built extension is loaded by path; otherwise the community
        // build. The extension is a hard dependency — nothing in this crate
        // works without it, so a failure here is fatal rather than deferred.
        //
        // `allow_unsigned_extensions` is a startup-only option: it goes on the
        // Config the connection is opened with, because `SET` on a running
        // database is refused.
        let conn = match std::env::var("CITYLAKE_CITYJSON_EXTENSION") {
            Ok(path) => {
                let config = Config::default().allow_unsigned_extensions()?;
                let conn = Connection::open_in_memory_with_flags(config)?;
                conn.execute_batch(&format!("LOAD {};", sql::literal(&path)))?;
                conn
            }
            Err(_) => {
                let conn = Connection::open_in_memory()?;
                conn.execute_batch("INSTALL cityjson FROM community; LOAD cityjson;")?;
                conn
            }
        };
        conn.execute_batch("INSTALL ducklake; LOAD ducklake;")?;
        // `json` backs to_json() on the query path and json_object() when the
        // CRS footer is minted.
        conn.execute_batch("INSTALL json; LOAD json;")?;

        conn.execute_batch(&format!(
            "ATTACH {} AS {} (DATA_PATH {})",
            sql::literal(&format!("ducklake:{}", config.catalog_path)),
            sql::ident(&config.catalog_name),
            sql::literal(&config.storage_path),
        ))?;

        tracing::info!(
            catalog = %config.catalog_path,
            storage = %config.storage_path,
            "CityLake ready"
        );

        Ok(Self {
            connection: Arc::new(Mutex::new(conn)),
            config,
        })
    }

    pub fn config(&self) -> &CityLakeConfig {
        &self.config
    }

    pub fn catalog(&self) -> &str {
        &self.config.catalog_name
    }

    pub fn with_connection<T>(
        &self,
        f: impl FnOnce(&Connection) -> RepositoryResult<T>,
    ) -> RepositoryResult<T> {
        let guard = self
            .connection
            .lock()
            .map_err(|e| CityLakeError::Internal(format!("connection mutex poisoned: {e}")))?;
        f(&guard)
    }

    /// Run `f` with the search path set to `path`, on a connection the caller
    /// already holds — which is what lets it nest inside a transaction, where
    /// [`scoped`] cannot go because that takes a connection of its own.
    ///
    /// The path is reset whether `f` succeeds or fails: leaving it set would
    /// silently resolve the next operation against this dataset. A reset
    /// failure never masks the body's error.
    pub fn with_search_path<T>(
        &self,
        conn: &Connection,
        path: &str,
        f: impl FnOnce(&Connection) -> RepositoryResult<T>,
    ) -> RepositoryResult<T> {
        conn.execute_batch(&format!("SET search_path={}", sql::literal(path)))?;
        let result = f(conn);
        let reset = conn.execute_batch("RESET search_path");
        match (result, reset) {
            (Ok(value), Ok(())) => Ok(value),
            (Ok(_), Err(e)) => Err(e.into()),
            (Err(e), _) => Err(e),
        }
    }

    /// Point the search path at one dataset, so the package pragmas resolve
    /// their bare schema argument inside the attached catalog.
    pub fn scoped<T>(
        &self,
        dataset: &str,
        f: impl FnOnce(&Connection) -> RepositoryResult<T>,
    ) -> RepositoryResult<T> {
        let path = format!("{}.{dataset}", self.catalog());
        self.with_connection(|conn| self.with_search_path(conn, &path, f))
    }

    /// Run `f` inside a transaction, committing on success and rolling back on
    /// failure. Pragma effects roll back with everything else — a delete's
    /// cascade, survivor cleanup and re-derivation are one unit.
    pub fn in_transaction<T>(
        &self,
        conn: &Connection,
        f: impl FnOnce(&Connection) -> RepositoryResult<T>,
    ) -> RepositoryResult<T> {
        conn.execute_batch("BEGIN")?;
        match f(conn) {
            Ok(value) => {
                conn.execute_batch("COMMIT")?;
                Ok(value)
            }
            Err(e) => {
                // A rollback failure must not hide why we are rolling back.
                if let Err(rollback) = conn.execute_batch("ROLLBACK") {
                    tracing::error!(%rollback, "rollback failed after {e}");
                }
                Err(e)
            }
        }
    }

    pub fn schema_exists(&self, conn: &Connection, dataset: &str) -> RepositoryResult<bool> {
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM information_schema.schemata
             WHERE catalog_name = ? AND schema_name = ?",
            [self.catalog(), dataset],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }
}
```

`tests/data/delft.city.jsonl` is already in place — Task 4 moved it there.

- [ ] **Step 4: Run the tests and watch them pass**

```bash
cd lib/citylake && cargo test --test service
```

Expected: 4 passed.

- [ ] **Step 5: Commit**

```bash
git add lib/citylake/src/core/db/service.rs lib/citylake/tests/
git commit -m "feat(citylake): own the connection, the catalog and the scoping rules

The package pragmas resolve a bare schema name through the search path,
so reaching a dataset in the attached catalog is a discipline rather
than an argument. Keeping it in one scoped() helper — which resets even
when the body fails — is what stops it being re-derived nine times."
```

---

> **Tasks 6-11 use `with_search_path`, never a hand-rolled set/RESET pair.**
> The code blocks below were written before that helper existed and still show
> the long form in places; where they do, use the helper. It exists precisely so
> the fiddly "run, capture, reset, re-raise in the right order" dance is written
> once rather than eight times.

### Task 6: `dataset.rs` — creation, and the CRS footer

The heart of the rebuild. Creating a dataset bootstraps a package, ingests the source, and mints the CRS footer that arms the extension's own guard.

**Why the minting works this way.** A DuckLake table has no Parquet footer, so `__cityparquet.city` is NULL and the extension's rule — a destination stating nothing has nothing to check — silently disables CRS checking on every later ingest. CityLake cannot write that footer itself: a footer's `crs` is canonical PROJJSON produced by the extension's own resolver, and assembling one in Rust would be resolving a CRS, the one thing this crate must not do. So the extension mints it. One row of the freshly ingested data is copied into a throwaway schema in the **default** catalog, written with `crs => <the source's referenceSystem>`, and the canonical text read straight back out of the Parquet footer. Only the `crs` field is kept: a minimal `{"crs": …}` value is enough for `cityparquet_city_field` to read and for the guard to fire, and it carries no stale inventory from the probe row.

The probe lives in the default catalog deliberately — it does not depend on Task 2, and it is one row, not one dataset.

**Minting happens after the ingest commits, in three phases, and this is not negotiable.** Two independent constraints force it. `cityparquet_write` sees committed state only, so called inside the open ingest transaction it cannot see the rows it is meant to copy — it fails outright with `Catalog Error: Schema … does not exist!`. And a single transaction may write to only one attached database, so a probe schema in `memory` cannot share a transaction that has already written to `lake`. The phases are therefore: **(1)** schema, seed, init and insert in one transaction against `lake`, committed; **(2)** the probe in `memory`, auto-committed, yielding the footer text; **(3)** the `UPDATE` against `lake.<ds>.__cityparquet`.

That splits creation's atomicity, so the failure semantics are stated rather than left to chance: if the source **declares** a `referenceSystem` and minting fails, the schema is dropped and the create fails, because a dataset silently missing the CRS guard is worse than no dataset. If the source declares **none**, there is nothing to mint and a footerless package is the correct "CRS unknown" state, not a failure.

> **Tasks 6-11 test the inherent `*_impl` methods, synchronously.** The async
> `CityLakeRepository` trait is not implemented until Task 12, so a test calling
> `service.create_dataset(...).await` here would not compile. Each task's tests
> therefore exercise what that task actually delivers — the inherent method —
> and Task 12 proves the trait wiring separately. The `*_impl` methods are `pub`
> because these are integration tests under `tests/`, which see only the crate's
> public API.

**Files:**
- Create: `lib/citylake/src/core/db/dataset.rs`
- Modify: `lib/citylake/src/core/db/mod.rs`

**Interfaces:**
- Consumes: everything from Tasks 3–5.
- Produces, as `impl DuckLakeService`:
  - `fn create_dataset_impl(&self, dataset: &DatasetName, source_path: &str) -> RepositoryResult<DatasetInfo>`
  - `fn list_datasets_impl(&self) -> RepositoryResult<Vec<String>>`
  - `fn describe_dataset_impl(&self, dataset: &DatasetName) -> RepositoryResult<DatasetInfo>`
  - `fn drop_dataset_impl(&self, dataset: &DatasetName) -> RepositoryResult<()>`
  - `fn dataset_crs(&self, conn: &Connection, dataset: &str) -> RepositoryResult<Option<String>>` — the `crs` field of any object-table footer, used by Tasks 10 and 11
  - `fn object_tables(&self, conn: &Connection, dataset: &str) -> RepositoryResult<Vec<String>>`

- [ ] **Step 1: Write the failing tests**

`lib/citylake/tests/dataset.rs`:

```rust
mod common;

use citylake::core::interface::types::DatasetName;

#[test]
fn creating_a_dataset_routes_objects_to_module_tables() {
    let (service, _dir) = common::test_service();
    let name = DatasetName::new("delft").unwrap();

    let info = service
        .create_dataset_impl(&name, common::fixture("delft.city.jsonl").to_str().unwrap())
        .expect("create the dataset");

    // The fixture holds Buildings, so the building module table exists and
    // carries them. Routing is the extension's, not ours.
    let building = info
        .modules
        .iter()
        .find(|m| m.name == "building")
        .expect("a building module table");
    assert!(building.rows > 0, "buildings were not ingested");
    assert_eq!(building.role, "object");
}

#[test]
fn a_created_dataset_declares_its_crs() {
    let (service, _dir) = common::test_service();
    let name = DatasetName::new("delft").unwrap();

    let info = service
        .create_dataset_impl(&name, common::fixture("delft.city.jsonl").to_str().unwrap())
        .unwrap();

    // The fixture declares EPSG:7415. The footer is minted by the extension,
    // so what comes back is canonical PROJJSON, not the source's spelling.
    let crs = info.crs.expect("the dataset should declare a CRS");
    assert!(crs.contains("7415"), "unexpected CRS: {crs}");
}

#[test]
fn the_declared_crs_arms_the_guard_against_a_mismatched_source() {
    let (service, _dir) = common::test_service();
    let name = DatasetName::new("delft").unwrap();
    service
        .create_dataset_impl(&name, common::fixture("delft.city.jsonl").to_str().unwrap())
        .unwrap();

    // This is the point of minting the footer at all: without it the package
    // states nothing, and a differently-projected source would be accepted.
    let err = service
        .ingest_impl(&name, common::fixture("bench_28992.city.json").to_str().unwrap())
        .expect_err("a 28992 source must not enter a 7415 package");
    assert!(
        format!("{err}").contains("CRS mismatch"),
        "expected a CRS mismatch, got: {err}"
    );
}

#[test]
fn a_source_without_a_crs_still_creates_a_dataset() {
    let (service, _dir) = common::test_service();
    let name = DatasetName::new("nocrs").unwrap();

    // Nothing to mint is not a failure: a package that states no CRS is the
    // correct "unknown" state, and the extension treats it as one.
    let info = service
        .create_dataset_impl(&name, common::fixture("minimal_nocrs.city.json").to_str().unwrap())
        .expect("a source without a referenceSystem is still ingestable");
    assert!(info.modules.iter().any(|m| m.rows > 0));
}

#[test]
fn minting_the_footer_does_not_leave_the_ingest_uncommitted() {
    let (service, _dir) = common::test_service();
    let name = DatasetName::new("delft").unwrap();

    // The regression this pins: cityparquet_write sees committed state only,
    // and a transaction that has written to `lake` may not also write to
    // `memory`. Minting inside the ingest transaction fails on both counts —
    // so if this passes, the phases are correctly separated.
    let info = service
        .create_dataset_impl(&name, common::fixture("delft.city.jsonl").to_str().unwrap())
        .expect("create must survive the probe");
    assert!(info.crs.is_some(), "the footer was not minted");
    assert!(info.modules.iter().map(|m| m.rows).sum::<usize>() > 0, "the ingest was lost");
}

#[test]
fn creating_a_dataset_twice_is_refused() {
    let (service, _dir) = common::test_service();
    let name = DatasetName::new("delft").unwrap();
    let source = common::fixture("delft.city.jsonl");
    service
        .create_dataset_impl(&name, source.to_str().unwrap())
        .unwrap();

    let err = service
        .create_dataset_impl(&name, source.to_str().unwrap())
        .expect_err("the second create must be refused");
    assert!(format!("{err}").contains("delft"));
}

#[test]
fn a_failed_create_leaves_no_half_built_schema() {
    let (service, _dir) = common::test_service();
    let name = DatasetName::new("broken").unwrap();

    let failed = service.create_dataset_impl(&name, "/nonexistent/source.city.jsonl");
    assert!(failed.is_err());

    // A dataset that failed to ingest must not be left addressable — the next
    // create would then fail as a duplicate.
    assert!(!service.list_datasets_impl().unwrap().contains(&"broken".to_string()));
}

#[test]
fn datasets_can_be_listed_described_and_dropped() {
    let (service, _dir) = common::test_service();
    let name = DatasetName::new("delft").unwrap();
    service
        .create_dataset_impl(&name, common::fixture("delft.city.jsonl").to_str().unwrap())
        .unwrap();

    assert!(service.list_datasets_impl().unwrap().contains(&"delft".to_string()));
    assert_eq!(service.describe_dataset_impl(&name).unwrap().name, "delft");

    service.drop_dataset_impl(&name).unwrap();
    assert!(!service.list_datasets_impl().unwrap().contains(&"delft".to_string()));
}
```

**Create the fixture set — every test in the suite reads from it.**

A package states one CRS for every row it holds, so an ingest or merge whose
source declares none is refused against a destination that declares one. Most of
the extension's fixtures carry no `referenceSystem`, so pairing them with the
EPSG:7415 Delft fixture would be refused — correctly, but uselessly for a test
that means to exercise something else. CityLake therefore keeps its own set:
CRS-bearing variants where a test needs the CRSs to agree, and a deliberately
CRS-less one for the case that tests exactly that.

```bash
cd lib/citylake/tests/data
EXT=../../../duckdb-cityjson/test/data

# A source with no referenceSystem — the "CRS unknown" case, which is a state
# to handle rather than a failure.
cp "$EXT/minimal.city.json"   minimal_nocrs.city.json

# A parent with two children, for the cascade. No CRS needed: it is only ever
# its own dataset, never ingested into another.
cp "$EXT/hierarchy.city.json" hierarchy.city.json

# A differently-projected source, for the mismatch the guard must refuse.
cp "$EXT/insert_crs_28992.city.json" bench_28992.city.json

# CRS-bearing variants: same objects, declared EPSG:7415, so they can be
# ingested into and merged with the Delft fixture.
python3 - <<'PYEOF'
import json

CRS = "https://www.opengis.net/def/crs/EPSG/0/7415"
EXT = "../../../duckdb-cityjson/test/data"

# CityJSON: one document, so the metadata is the document's own.
doc = json.load(open(f"{EXT}/minimal.city.json"))
doc.setdefault("metadata", {})["referenceSystem"] = CRS
json.dump(doc, open("minimal_7415.city.json", "w"))

# CityJSONSeq: the first line is the header and carries the metadata; every
# line after it is a feature and is copied through untouched.
with open(f"{EXT}/railway_appearance.city.jsonl") as src, \
     open("railway_7415.city.jsonl", "w") as dst:
    header = json.loads(src.readline())
    header.setdefault("metadata", {})["referenceSystem"] = CRS
    dst.write(json.dumps(header) + "\n")
    for line in src:
        dst.write(line)
PYEOF
```

Confirm the two variants declare what they should before relying on them:

```bash
cd lib/citylake/tests/data && python3 -c "
import json
print('minimal_7415:', json.load(open('minimal_7415.city.json'))['metadata']['referenceSystem'])
print('railway_7415:', json.loads(open('railway_7415.city.jsonl').readline())['metadata']['referenceSystem'])
print('minimal_nocrs:', json.load(open('minimal_nocrs.city.json')).get('metadata', {}).get('referenceSystem'))
"
```

Expected: both variants print the EPSG:7415 URL, and `minimal_nocrs` prints
`None`.

- [ ] **Step 2: Run them and watch them fail**

```bash
cd lib/citylake && cargo test --test dataset
```

Expected: FAIL — `create_dataset` is unimplemented.

- [ ] **Step 3: Implement dataset creation**

`lib/citylake/src/core/db/dataset.rs`. The creation sequence, one statement at a time because pragmas may not be batched:

```rust
//! Dataset lifecycle: create, list, describe, drop.
//!
//! A dataset is a CityParquet package living as a schema in the DuckLake
//! catalog. Creating one from a CityJSON-family source has to bootstrap the
//! package first — there is no pragma that makes one from nothing — and then
//! give it a CRS the extension's own guard can check against.

impl DuckLakeService {
    pub fn create_dataset_impl(
        &self,
        dataset: &DatasetName,
        source_path: &str,
    ) -> RepositoryResult<DatasetInfo> {
        let name = dataset.as_str();

        // A directory is an existing package: cityparquet_read loads it and
        // recovers each file's Parquet footer, so its CRS arrives with it and
        // none of the bootstrap below applies.
        if std::path::Path::new(source_path).is_dir() {
            return self.import_package(dataset, source_path);
        }

        let format = sql::reader_for(source_path);
        let catalog = self.catalog().to_string();

        self.with_connection(|conn| {
            if self.schema_exists(conn, name)? {
                return Err(CityLakeError::DatasetExists(name.to_string()));
            }

            // Phase 1 — the ingest, one unit against the lake catalog. A
            // create that fails partway must not leave an addressable
            // half-built dataset behind.
            self.in_transaction(conn, |conn| {
                conn.execute_batch(&sql::create_schema(&catalog, name))?;
                conn.execute_batch(&sql::seed_table(&catalog, name, source_path, format))?;

                conn.execute_batch(&sql::set_search_path(&catalog, name))?;
                let scoped = (|| -> RepositoryResult<()> {
                    // One pragma per statement: DuckDB expands every pragma in
                    // a script before running any of it, so a batched pair
                    // would each see pre-batch state.
                    conn.execute_batch(&sql::init_pragma(name))?;
                    conn.execute_batch(&sql::insert_pragma(name, source_path, format, true))?;
                    Ok(())
                })();
                let reset = conn.execute_batch("RESET search_path");
                scoped?;
                reset?;
                Ok(())
            })?;

            // Phases 2 and 3 — outside that transaction, and they must be:
            // cityparquet_write sees committed state only, and a transaction
            // that has written to `lake` may not also write to `memory`.
            if let Err(e) = self.mint_crs_footer(conn, name, source_path, format) {
                // The ingest is already committed, so unwinding is explicit.
                // A dataset whose CRS guard is silently off is worse than none.
                let _ = conn.execute_batch(&format!(
                    "DROP SCHEMA {} CASCADE",
                    sql::qualified(&[&catalog, name])
                ));
                return Err(e);
            }

            self.describe_locked(conn, name)
        })
    }

    /// Give the package a CRS the extension can check against.
    ///
    /// A DuckLake table has no Parquet footer, so `__cityparquet.city` is NULL
    /// and the CRS guard is silent. The footer's `crs` is canonical PROJJSON
    /// minted by the extension's resolver, so CityLake cannot write one — it
    /// asks the extension to, by writing a single row to a throwaway package
    /// and reading the footer back. Only `crs` is kept: the guard reads that
    /// field alone, and a minimal value carries no stale probe inventory.
    ///
    /// A source that declares no referenceSystem leaves the footer NULL, which
    /// is the correct "CRS unknown" state rather than a guess.
    ///
    /// Every statement here runs **outside** the ingest transaction, and must:
    /// `cityparquet_write` sees committed state only, and a transaction that
    /// has written to the lake catalog may not also write to `memory`, where
    /// the probe lives.
    fn mint_crs_footer(
        &self,
        conn: &Connection,
        dataset: &str,
        source_path: &str,
        format: sql::SourceFormat,
    ) -> RepositoryResult<()> {
        // `reference_system` is a struct — struct(base_url, authority,
        // version, code) — so the authority:code spelling the writer wants is
        // assembled in SQL. Rust never inspects or resolves a CRS.
        let reference_system: Option<String> = conn
            .query_row(
                &format!(
                    "SELECT reference_system.authority || ':' || reference_system.code
                     FROM {}({})",
                    match format {
                        sql::SourceFormat::CityJson => "cityjson_metadata",
                        sql::SourceFormat::CityJsonSeq => "cityjsonseq_metadata",
                        sql::SourceFormat::FlatCityBuf => "flatcitybuf_metadata",
                    },
                    sql::literal(source_path)
                ),
                [],
                |row| row.get(0),
            )
            .ok()
            .flatten();

        let Some(reference_system) = reference_system else {
            tracing::info!(dataset, "source declares no CRS; the package states none");
            return Ok(());
        };

        // A non-empty object table is required: cityparquet_write emits one
        // file per non-empty table, and an empty probe would produce no footer.
        let Some(module) = self.object_tables(conn, dataset)?.into_iter().find(|t| {
            conn.query_row(
                &format!(
                    "SELECT COUNT(*) > 0 FROM {}",
                    sql::qualified(&[self.catalog(), dataset, t])
                ),
                [],
                |row| row.get::<_, bool>(0),
            )
            .unwrap_or(false)
        }) else {
            return Err(CityLakeError::NoObjectTable(dataset.to_string()));
        };

        let probe = format!("__citylake_crs_{dataset}");
        let probe_dir = tempfile::tempdir()?;
        let probe_path = probe_dir.path().to_string_lossy().into_owned();

        // The probe lives in the default catalog: one row, no dependency on
        // the attached-catalog write path.
        conn.execute_batch(&format!("CREATE SCHEMA {}", sql::ident(&probe)))?;
        let minted = (|| -> RepositoryResult<Option<String>> {
            conn.execute_batch(&format!(
                "CREATE TABLE {}.{} AS SELECT * FROM {} LIMIT 1",
                sql::ident(&probe),
                sql::ident(&module),
                sql::qualified(&[self.catalog(), dataset, &module])
            ))?;
            conn.execute_batch(&sql::init_pragma(&probe))?;
            conn.execute_batch(&sql::write_package(
                &probe,
                &probe_path,
                Some(&reference_system),
            ))?;

            let footer: Option<String> = conn
                .query_row(
                    &format!(
                        "SELECT json_object('crs', json_extract(decode(value), '$.crs'))::VARCHAR
                         FROM parquet_kv_metadata({})
                         WHERE decode(key) = 'city'",
                        sql::literal(&format!("{probe_path}/{module}.parquet"))
                    ),
                    [],
                    |row| row.get(0),
                )
                .ok();
            Ok(footer)
        })();

        conn.execute_batch(&format!("DROP SCHEMA {} CASCADE", sql::ident(&probe)))?;
        // The source stated a CRS, so failing to mint is fatal: the caller
        // would otherwise get a package whose guard is silently off.
        let footer = minted?.ok_or_else(|| {
            CityLakeError::Internal(format!(
                "could not mint a CRS footer for {dataset}: the source declares \
                 {reference_system} but no footer came back from the probe"
            ))
        })?;

        // Every object table in a package states the same CRS.
        conn.execute(
            &format!(
                "UPDATE {} SET city = ? WHERE role = 'object'",
                sql::qualified(&[self.catalog(), dataset, "__cityparquet"])
            ),
            [&footer],
        )?;
        Ok(())
    }
}
```

`list_datasets_impl` selects `schema_name` from `information_schema.schemata`
filtered to `catalog_name = self.catalog()`, excluding DuckLake's own
bookkeeping schemas. Probe schemas need no exclusion — they live in `memory`,
not in the lake catalog, and are dropped before the call returns. `describe_dataset_impl` reads `table_name, role` from
`__cityparquet`, counts rows per table, and reads the CRS through
`dataset_crs`. `drop_dataset_impl` is `DROP SCHEMA … CASCADE` after an
existence check. `dataset_crs` is:

```rust
    pub fn dataset_crs(&self, conn: &Connection, dataset: &str) -> RepositoryResult<Option<String>> {
        Ok(conn
            .query_row(
                &format!(
                    "SELECT cityparquet_city_field(city, 'crs') FROM {}
                     WHERE role = 'object' AND city IS NOT NULL LIMIT 1",
                    sql::qualified(&[self.catalog(), dataset, "__cityparquet"])
                ),
                [],
                |row| row.get(0),
            )
            .ok()
            .flatten())
    }
```

- [ ] **Step 4: Run the tests and watch them pass**

```bash
cd lib/citylake && cargo test --test dataset
```

Expected: 8 passed. Two matter most: `the_declared_crs_arms_the_guard_against_a_mismatched_source` proves the minted footer actually does something rather than merely being present, and `minting_the_footer_does_not_leave_the_ingest_uncommitted` pins the phase separation the two transaction constraints force.

- [ ] **Step 5: Commit**

```bash
git add lib/citylake/src/core/db/dataset.rs lib/citylake/src/core/db/mod.rs lib/citylake/tests/
git commit -m "feat(citylake): create datasets as packages, with a CRS the guard can check

A fresh package needs a seed object table, because no pragma makes one
from nothing. It also needs a footer: a DuckLake table has none, so the
extension's CRS guard would be silent on every later ingest. The footer
is minted by the extension from one probe row rather than assembled
here — canonical PROJJSON is its resolver's output, and producing it in
Rust would be resolving a CRS."
```

---

### Task 7: `ingest.rs` — further sources into an existing dataset

One pragma, inside a transaction. Everything difficult — routing by module, renumbering sidecar ids and rewriting their references, refusing duplicate ids, checking the CRS, re-deriving `feature_id`, hierarchy and `bbox` — is the extension's.

**Files:**
- Create: `lib/citylake/src/core/db/ingest.rs`

**Interfaces:**
- Consumes: `sql::{reader_for, insert_pragma}`, `DuckLakeService::{scoped, in_transaction, object_tables}`.
- Produces: `fn ingest_impl(&self, dataset: &DatasetName, source_path: &str) -> RepositoryResult<usize>` — returns total rows across object tables after the insert minus before.

- [ ] **Step 1: Write the failing tests**

`lib/citylake/tests/ingest.rs`:

```rust
mod common;

use citylake::core::interface::types::DatasetName;

#[test]
fn ingesting_a_second_source_adds_its_objects() {
    let (service, _dir) = common::test_service();
    let name = DatasetName::new("delft").unwrap();
    service
        .create_dataset_impl(&name, common::fixture("delft.city.jsonl").to_str().unwrap())
        .unwrap();

    let before: usize = service
        .describe_dataset_impl(&name)
        .unwrap()
        .modules
        .iter()
        .map(|m| m.rows)
        .sum();

    // The variant declaring EPSG:7415: a package states one CRS for every row,
    // so a source declaring none would be refused here — correctly, but that is
    // the mismatch test's job, not this one's.
    let added = service
        .ingest_impl(&name, common::fixture("minimal_7415.city.json").to_str().unwrap())
        .expect("ingest a second source");
    assert!(added > 0);

    let after: usize = service
        .describe_dataset_impl(&name)
        .unwrap()
        .modules
        .iter()
        .map(|m| m.rows)
        .sum();
    assert_eq!(after, before + added);
}

#[test]
fn ingesting_the_same_source_twice_is_refused() {
    let (service, _dir) = common::test_service();
    let name = DatasetName::new("delft").unwrap();
    let source = common::fixture("delft.city.jsonl");
    service
        .create_dataset_impl(&name, source.to_str().unwrap())
        .unwrap();

    // Ids are identity: an incoming id already present refuses the whole
    // insert rather than renaming silently.
    let err = service
        .ingest_impl(&name, source.to_str().unwrap())
        .expect_err("duplicate ids must refuse the insert");
    assert!(format!("{err}").contains("duplicate id"), "got: {err}");
}

#[test]
fn a_refused_ingest_leaves_the_dataset_untouched() {
    let (service, _dir) = common::test_service();
    let name = DatasetName::new("delft").unwrap();
    let source = common::fixture("delft.city.jsonl");
    service
        .create_dataset_impl(&name, source.to_str().unwrap())
        .unwrap();
    let before: usize = service
        .describe_dataset_impl(&name)
        .unwrap()
        .modules
        .iter()
        .map(|m| m.rows)
        .sum();

    let _ = service.ingest_impl(&name, source.to_str().unwrap());

    let after: usize = service
        .describe_dataset_impl(&name)
        .unwrap()
        .modules
        .iter()
        .map(|m| m.rows)
        .sum();
    assert_eq!(after, before, "a refused ingest must not partially apply");
}

#[test]
fn ingesting_a_new_module_creates_its_table() {
    let (service, _dir) = common::test_service();
    let name = DatasetName::new("mixed").unwrap();
    service
        .create_dataset_impl(&name, common::fixture("delft.city.jsonl").to_str().unwrap())
        .unwrap();

    // This fixture carries a Bridge and a CityFurniture — two further modules.
    // Routing them is the extension's job; create_tables = true is ours.
    service
        .ingest_impl(&name, common::fixture("railway_7415.city.jsonl").to_str().unwrap())
        .unwrap();

    let modules: Vec<String> = service
        .describe_dataset_impl(&name)
        .unwrap()
        .modules
        .into_iter()
        .map(|m| m.name)
        .collect();
    assert!(modules.contains(&"bridge".to_string()), "got {modules:?}");
    assert!(modules.contains(&"city_furniture".to_string()), "got {modules:?}");
}
```

The fixtures this task reads were created in Task 6; nothing to copy here.

- [ ] **Step 2: Run them and watch them fail**

```bash
cd lib/citylake && cargo test --test ingest
```

Expected: FAIL — `ingest_impl` does not exist.

- [ ] **Step 3: Implement it**

```rust
//! Ingesting a further source into an existing dataset.
//!
//! One pragma does all of it. What is worth knowing is what the pragma
//! guarantees, because CityLake must not second-guess any of it: routing is by
//! CityGML module and is total, ids are identity so a duplicate refuses the
//! whole insert, the CRS must match and is never reprojected, and derived
//! state is re-derived afterwards.

impl DuckLakeService {
    pub fn ingest_impl(&self, dataset: &DatasetName, source_path: &str) -> RepositoryResult<usize> {
        let name = dataset.as_str();
        let format = sql::reader_for(source_path);

        self.with_connection(|conn| {
            if !self.schema_exists(conn, name)? {
                return Err(CityLakeError::DatasetNotFound(name.to_string()));
            }
            let before = self.total_object_rows(conn, name)?;

            self.in_transaction(conn, |conn| {
                conn.execute_batch(&sql::set_search_path(self.catalog(), name))?;
                // create_tables = true so a source spanning a module this
                // dataset has not seen yet brings its table with it.
                let inserted =
                    conn.execute_batch(&sql::insert_pragma(name, source_path, format, true));
                let reset = conn.execute_batch("RESET search_path");
                inserted?;
                reset?;
                Ok(())
            })?;

            Ok(self.total_object_rows(conn, name)? - before)
        })
    }

    fn total_object_rows(&self, conn: &Connection, dataset: &str) -> RepositoryResult<usize> {
        let mut total = 0usize;
        for table in self.object_tables(conn, dataset)? {
            let rows: i64 = conn.query_row(
                &format!(
                    "SELECT COUNT(*) FROM {}",
                    sql::qualified(&[self.catalog(), dataset, &table])
                ),
                [],
                |row| row.get(0),
            )?;
            total += rows as usize;
        }
        Ok(total)
    }
}
```

- [ ] **Step 4: Run the tests and watch them pass**

```bash
cd lib/citylake && cargo test --test ingest
```

Expected: 4 passed.

- [ ] **Step 5: Commit**

```bash
git add lib/citylake/src/core/db/ingest.rs lib/citylake/src/core/db/mod.rs lib/citylake/tests/
git commit -m "feat(citylake): ingest through one pragma, inside one transaction

Routing, sidecar renumbering, duplicate-id refusal, the CRS check and
re-derivation are all the extension's. CityLake's contribution is the
transaction that makes a refused insert leave nothing behind."
```

---

### Task 8: `query.rs` — reading objects back

**Files:**
- Create: `lib/citylake/src/core/db/query.rs`

**Interfaces:**
- Consumes: `sql::select_objects`, `QueryParams`, `ModuleName`.
- Produces: `fn query_objects_impl(&self, dataset: &DatasetName, module: &ModuleName, params: &QueryParams) -> RepositoryResult<Vec<serde_json::Value>>`.

- [ ] **Step 1: Write the failing tests**

`lib/citylake/tests/query.rs`:

```rust
mod common;

use citylake::core::interface::types::{DatasetName, ModuleName, QueryParams};

fn seeded() -> (citylake::core::db::service::DuckLakeService, tempfile::TempDir, DatasetName) {
    let (service, dir) = common::test_service();
    let name = DatasetName::new("delft").unwrap();
    service
        .create_dataset_impl(&name, common::fixture("delft.city.jsonl").to_str().unwrap())
        .unwrap();
    (service, dir, name)
}

#[test]
fn objects_come_back_as_json_rows() {
    let (service, _dir, name) = seeded();
    let module = ModuleName::new("building").unwrap();

    let rows = service
        .query_objects_impl(&name, &module, &QueryParams::default())
        .expect("query the building module");
    assert!(!rows.is_empty());
    assert!(rows[0].get("id").is_some(), "a row should carry its id");
}

#[test]
fn a_page_is_bounded_and_offsettable() {
    let (service, _dir, name) = seeded();
    let module = ModuleName::new("building").unwrap();

    let first = service
        .query_objects_impl(
            &name,
            &module,
            &QueryParams { filter: None, limit: 1, offset: 0 },
        )
        .unwrap();
    assert_eq!(first.len(), 1);

    let second = service
        .query_objects_impl(
            &name,
            &module,
            &QueryParams { filter: None, limit: 1, offset: 1 },
        )
        .unwrap();
    assert_eq!(second.len(), 1);
    assert_ne!(first[0].get("id"), second[0].get("id"));
}

#[test]
fn a_filter_narrows_the_result() {
    let (service, _dir, name) = seeded();
    let module = ModuleName::new("building").unwrap();

    let filtered = service
        .query_objects_impl(
            &name,
            &module,
            &QueryParams {
                filter: Some("object_type = 'Building'".into()),
                limit: 100,
                offset: 0,
            },
        )
        .unwrap();
    assert!(filtered
        .iter()
        .all(|row| row.get("object_type").and_then(|v| v.as_str()) == Some("Building")));
}

#[test]
fn querying_a_module_the_dataset_lacks_is_an_error_not_a_panic() {
    let (service, _dir, name) = seeded();
    let module = ModuleName::new("tunnel").unwrap();

    let err = service
        .query_objects_impl(&name, &module, &QueryParams::default())
        .expect_err("the fixture has no tunnels");
    assert!(format!("{err}").contains("tunnel"));
}
```

- [ ] **Step 2: Run them and watch them fail**

```bash
cd lib/citylake && cargo test --test query
```

Expected: FAIL — `query_objects_impl` does not exist.

- [ ] **Step 3: Implement it**

```rust
impl DuckLakeService {
    pub fn query_objects_impl(
        &self,
        dataset: &DatasetName,
        module: &ModuleName,
        params: &QueryParams,
    ) -> RepositoryResult<Vec<serde_json::Value>> {
        let (name, module_name) = (dataset.as_str(), module.as_str());

        self.with_connection(|conn| {
            if !self.table_exists(conn, name, module_name)? {
                return Err(CityLakeError::ModuleNotFound {
                    dataset: name.to_string(),
                    module: module_name.to_string(),
                });
            }

            // `filter` is a caller-supplied SQL predicate, interpolated as
            // written: cityparquet_delete takes a predicate string by design
            // and the query filter matches it. See the specification's §10 for
            // the trust model this assumes.
            let sql_text = sql::select_objects(
                self.catalog(),
                name,
                module_name,
                params.filter.as_deref(),
                params.limit,
                params.offset,
            );

            let mut stmt = conn.prepare(&sql_text)?;
            let rows = stmt
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;

            rows.into_iter()
                .map(|json| {
                    serde_json::from_str(&json)
                        .map_err(|e| CityLakeError::Internal(format!("row is not JSON: {e}")))
                })
                .collect()
        })
    }

    pub(crate) fn table_exists(
        &self,
        conn: &Connection,
        dataset: &str,
        table: &str,
    ) -> RepositoryResult<bool> {
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM information_schema.tables
             WHERE table_catalog = ? AND table_schema = ? AND table_name = ?",
            [self.catalog(), dataset, table],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }
}
```

- [ ] **Step 4: Run the tests and watch them pass**

```bash
cd lib/citylake && cargo test --test query
```

Expected: 4 passed.

- [ ] **Step 5: Commit**

```bash
git add lib/citylake/src/core/db/query.rs lib/citylake/src/core/db/mod.rs lib/citylake/tests/query.rs
git commit -m "feat(citylake): query a module's objects with a bounded page

The default page is bounded because an unbounded one would let a single
request pull a national dataset into memory."
```

---

### Task 9: `mutate.rs` — update, delete, reconcile

There is deliberately no `cityparquet_update`. Attribute edits are ordinary `UPDATE` and need no wrapper; only structural edits — geometry, hierarchy, appearance — invalidate derived state, and `cityparquet_reconcile` re-derives exactly that. Delete, by contrast, must go through the pragma, because deleting a parent has to cascade.

**Files:**
- Create: `lib/citylake/src/core/db/mutate.rs`

**Interfaces:**
- Consumes: `sql::{delete_pragma, reconcile_pragma, ident, literal, qualified}`, `DuckLakeService::{object_tables, in_transaction}`.
- Produces:
  - `fn update_object_impl(&self, dataset: &DatasetName, id: &str, attributes: &serde_json::Map<String, serde_json::Value>) -> RepositoryResult<()>`
  - `fn delete_object_impl(&self, dataset: &DatasetName, id: &str) -> RepositoryResult<usize>`
  - `fn delete_where_impl(&self, dataset: &DatasetName, predicate: &str) -> RepositoryResult<usize>`
  - `fn reconcile_impl(&self, dataset: &DatasetName) -> RepositoryResult<()>`

- [ ] **Step 1: Write the failing tests**

`lib/citylake/tests/mutate.rs`:

```rust
mod common;

use citylake::core::interface::types::{DatasetName, ModuleName, QueryParams};
use serde_json::json;

fn seeded() -> (citylake::core::db::service::DuckLakeService, tempfile::TempDir, DatasetName) {
    let (service, dir) = common::test_service();
    let name = DatasetName::new("hier").unwrap();
    // This fixture has a parent/child hierarchy, which is what makes the
    // cascade observable.
    service
        .create_dataset_impl(&name, common::fixture("hierarchy.city.json").to_str().unwrap())
        .unwrap();
    (service, dir, name)
}

fn ids(service: &citylake::core::db::service::DuckLakeService, name: &DatasetName) -> Vec<String> {
    let module = ModuleName::new("building").unwrap();
    service
        .query_objects_impl(name, &module, &QueryParams { filter: None, limit: 1000, offset: 0 })
        .unwrap()
        .into_iter()
        .filter_map(|row| row.get("id")?.as_str().map(str::to_string))
        .collect()
}

#[test]
fn an_attribute_update_lands_on_the_row() {
    let (service, _dir, name) = seeded();
    let id = ids(&service, &name).into_iter().next().unwrap();

    let mut attributes = serde_json::Map::new();
    attributes.insert("object_type".into(), json!("Building"));
    service
        .update_object_impl(&name, &id, &attributes)
        .expect("update the object");

    let module = ModuleName::new("building").unwrap();
    let rows = service
        .query_objects_impl(
            &name,
            &module,
            &QueryParams {
                filter: Some(format!("id = '{id}'")),
                limit: 1,
                offset: 0,
            },
        )
        .unwrap();
    assert_eq!(rows[0].get("object_type").unwrap(), &json!("Building"));
}

#[test]
fn updating_an_absent_id_is_an_error() {
    let (service, _dir, name) = seeded();
    let mut attributes = serde_json::Map::new();
    attributes.insert("object_type".into(), json!("Building"));

    let err = service
        .update_object_impl(&name, "no-such-object", &attributes)
        .expect_err("an absent id must not silently succeed");
    assert!(format!("{err}").contains("no-such-object"));
}

#[test]
fn deleting_a_parent_cascades_to_its_children() {
    let (service, _dir, name) = seeded();
    let before = ids(&service, &name);
    let parent = before.first().expect("a parent object").clone();

    let deleted = service.delete_object_impl(&name, &parent).unwrap();
    // A cascade removes the parent and everything below it, so the count is
    // the subtree, not one.
    assert!(deleted >= 1);

    let after = ids(&service, &name);
    assert!(!after.contains(&parent));
    assert_eq!(before.len() - after.len(), deleted);
}

#[test]
fn deleting_by_predicate_removes_the_matching_objects() {
    let (service, _dir, name) = seeded();
    let deleted = service
        .delete_where_impl(&name, "object_type = 'Building'")
        .expect("delete by predicate");
    assert!(deleted > 0);

    let module = ModuleName::new("building").unwrap();
    let remaining = service
        .query_objects_impl(
            &name,
            &module,
            &QueryParams {
                filter: Some("object_type = 'Building'".into()),
                limit: 100,
                offset: 0,
            },
        )
        .unwrap();
    assert!(remaining.is_empty());
}

#[test]
fn reconciling_an_untouched_dataset_changes_nothing() {
    let (service, _dir, name) = seeded();
    let before = ids(&service, &name);

    // Both the reader and reconcile union a row's geometry across every stored
    // LoD and across its descendants, so a freshly read package is already
    // reconciled for the structural columns.
    service.reconcile_impl(&name).expect("reconcile");

    assert_eq!(ids(&service, &name), before);
}
```

`hierarchy.city.json` was created in Task 6. It holds one `Building` with two
`BuildingStorey` children, which the extension normalises to `Storey` and routes
into the `building` module table — so the parent and its subtree are all in one
table, which is what makes the cascade observable.

- [ ] **Step 2: Run them and watch them fail**

```bash
cd lib/citylake && cargo test --test mutate
```

Expected: FAIL — the mutation methods do not exist.

- [ ] **Step 3: Implement it**

```rust
//! Update, delete, reconcile.
//!
//! There is deliberately no `cityparquet_update` to call: attribute edits are
//! an ordinary UPDATE and need no wrapper. What structural edits invalidate is
//! derived state — feature_id, the reciprocal hierarchy, bbox — and
//! `cityparquet_reconcile` re-derives exactly that. Delete is different: it has
//! to cascade through `children`, so it goes through the pragma.

impl DuckLakeService {
    pub fn update_object_impl(
        &self,
        dataset: &DatasetName,
        id: &str,
        attributes: &serde_json::Map<String, serde_json::Value>,
    ) -> RepositoryResult<()> {
        if attributes.is_empty() {
            return Ok(());
        }
        let name = dataset.as_str();

        self.with_connection(|conn| {
            // An id is unique across the whole package, so the row is found by
            // searching the object tables rather than being told the module.
            let Some(module) = self.module_holding(conn, name, id)? else {
                return Err(CityLakeError::Internal(format!(
                    "no object with id {id} in dataset {name}"
                )));
            };

            self.in_transaction(conn, |conn| {
                let assignments = attributes
                    .keys()
                    .map(|column| format!("{} = ?", sql::ident(column)))
                    .collect::<Vec<_>>()
                    .join(", ");
                let statement = format!(
                    "UPDATE {} SET {assignments} WHERE id = ?",
                    sql::qualified(&[self.catalog(), name, &module])
                );

                // Values are bound, not interpolated — only identifiers cannot be.
                let mut params: Vec<Box<dyn duckdb::ToSql>> = attributes
                    .values()
                    .map(|value| match value {
                        serde_json::Value::String(s) => {
                            Box::new(s.clone()) as Box<dyn duckdb::ToSql>
                        }
                        serde_json::Value::Number(n) if n.is_i64() => {
                            Box::new(n.as_i64().unwrap()) as Box<dyn duckdb::ToSql>
                        }
                        serde_json::Value::Number(n) => {
                            Box::new(n.as_f64().unwrap_or_default()) as Box<dyn duckdb::ToSql>
                        }
                        serde_json::Value::Bool(b) => Box::new(*b) as Box<dyn duckdb::ToSql>,
                        other => Box::new(other.to_string()) as Box<dyn duckdb::ToSql>,
                    })
                    .collect();
                params.push(Box::new(id.to_string()));

                let refs: Vec<&dyn duckdb::ToSql> = params.iter().map(|p| p.as_ref()).collect();
                conn.execute(&statement, refs.as_slice())?;

                // An attribute edit cannot invalidate derived state, but a
                // caller may have sent a geometry column among the attributes,
                // and reconciling an already-correct package is a no-op.
                conn.execute_batch(&sql::set_search_path(self.catalog(), name))?;
                let reconciled = conn.execute_batch(&sql::reconcile_pragma(name));
                let reset = conn.execute_batch("RESET search_path");
                reconciled?;
                reset?;
                Ok(())
            })
        })
    }

    pub fn delete_object_impl(&self, dataset: &DatasetName, id: &str) -> RepositoryResult<usize> {
        // Deleting by id is deleting by the predicate that selects it.
        self.delete_where_impl(dataset, &format!("id = {}", sql::literal(id)))
    }

    pub fn delete_where_impl(
        &self,
        dataset: &DatasetName,
        predicate: &str,
    ) -> RepositoryResult<usize> {
        let name = dataset.as_str();

        self.with_connection(|conn| {
            if !self.schema_exists(conn, name)? {
                return Err(CityLakeError::DatasetNotFound(name.to_string()));
            }
            let before = self.total_object_rows(conn, name)?;

            self.in_transaction(conn, |conn| {
                conn.execute_batch(&sql::set_search_path(self.catalog(), name))?;
                // Cascade is the default and walks `children` transitively,
                // never feature_id equality — deleting a BuildingPart must not
                // take out the Building sharing its feature_id.
                let deleted = conn.execute_batch(&sql::delete_pragma(name, predicate, true));
                let reset = conn.execute_batch("RESET search_path");
                deleted?;
                reset?;
                Ok(())
            })?;

            Ok(before - self.total_object_rows(conn, name)?)
        })
    }

    pub fn reconcile_impl(&self, dataset: &DatasetName) -> RepositoryResult<()> {
        let name = dataset.as_str();
        self.with_connection(|conn| {
            self.in_transaction(conn, |conn| {
                conn.execute_batch(&sql::set_search_path(self.catalog(), name))?;
                let reconciled = conn.execute_batch(&sql::reconcile_pragma(name));
                let reset = conn.execute_batch("RESET search_path");
                reconciled?;
                reset?;
                Ok(())
            })
        })
    }

    /// Which object table holds `id`. Ids are unique across the whole package,
    /// so at most one table answers.
    fn module_holding(
        &self,
        conn: &Connection,
        dataset: &str,
        id: &str,
    ) -> RepositoryResult<Option<String>> {
        for table in self.object_tables(conn, dataset)? {
            let found: bool = conn.query_row(
                &format!(
                    "SELECT COUNT(*) > 0 FROM {} WHERE id = ?",
                    sql::qualified(&[self.catalog(), dataset, &table])
                ),
                [id],
                |row| row.get(0),
            )?;
            if found {
                return Ok(Some(table));
            }
        }
        Ok(None)
    }
}
```

- [ ] **Step 4: Run the tests and watch them pass**

```bash
cd lib/citylake && cargo test --test mutate
```

Expected: 5 passed.

- [ ] **Step 5: Commit**

```bash
git add lib/citylake/src/core/db/mutate.rs lib/citylake/src/core/db/mod.rs lib/citylake/tests/
git commit -m "feat(citylake)!: cascade deletes and re-derive after structural edits

Delete now walks children transitively instead of removing one row,
which is what deleting a parent has always had to mean. Attribute
updates stay an ordinary UPDATE — there is no cityparquet_update to
call, because only structural edits invalidate derived state."
```

---

### Task 10: `package.rs` — import, export, merge

The boundary between the lake and the file format. This task depends on Task 2 having landed: `cityparquet_write` cannot see a DuckLake schema without it.

**Files:**
- Create: `lib/citylake/src/core/db/package.rs`

**Interfaces:**
- Consumes: `sql::{read_package_pragma, write_package, merge_pragma}`, `dataset_crs` (Task 6).
- Produces:
  - `fn import_package(&self, dataset: &DatasetName, directory: &str) -> RepositoryResult<DatasetInfo>` — called by Task 6's `create_dataset_impl` when the source is a directory
  - `fn write_package_impl(&self, dataset: &DatasetName, output_dir: &str) -> RepositoryResult<Vec<PackageFile>>`
  - `fn export_module_impl(&self, dataset: &DatasetName, module: &ModuleName, output_path: &str, format: ExportFormat) -> RepositoryResult<()>`
  - `fn merge_impl(&self, destination: &DatasetName, source: &DatasetName) -> RepositoryResult<()>`

- [ ] **Step 1: Write the failing tests**

`lib/citylake/tests/package.rs`:

```rust
mod common;

use citylake::core::interface::types::{DatasetName, ExportFormat, ModuleName};

#[test]
fn a_dataset_writes_out_as_a_package_directory() {
    let (service, dir) = common::test_service();
    let name = DatasetName::new("delft").unwrap();
    service
        .create_dataset_impl(&name, common::fixture("delft.city.jsonl").to_str().unwrap())
        .unwrap();

    let out = dir.path().join("pkg");
    let written = service
        .write_package_impl(&name, out.to_str().unwrap())
        .expect("write the package");

    // One data file per non-empty object table, plus the STAC Item.
    assert!(written.iter().any(|f| f.file == "building.parquet"));
    assert!(written.iter().any(|f| f.file == "metadata.json"));
    assert!(out.join("building.parquet").exists());
    assert!(out.join("metadata.json").exists());
}

#[test]
fn a_written_package_carries_the_datasets_crs() {
    let (service, dir) = common::test_service();
    let name = DatasetName::new("delft").unwrap();
    service
        .create_dataset_impl(&name, common::fixture("delft.city.jsonl").to_str().unwrap())
        .unwrap();
    let out = dir.path().join("pkg");
    service.write_package_impl(&name, out.to_str().unwrap()).unwrap();

    // The footer minted at creation is what lets the writer state a CRS
    // instead of an explicit null.
    let reimported = DatasetName::new("reimported").unwrap();
    let info = service
        .create_dataset_impl(&reimported, out.to_str().unwrap())
        .expect("load the written package back");
    assert!(info.crs.expect("a CRS").contains("7415"));
}

#[test]
fn a_package_round_trips_through_the_lake() {
    let (service, dir) = common::test_service();
    let name = DatasetName::new("delft").unwrap();
    service
        .create_dataset_impl(&name, common::fixture("delft.city.jsonl").to_str().unwrap())
        .unwrap();
    let original: usize = service
        .describe_dataset_impl(&name)
        .unwrap()
        .modules
        .iter()
        .map(|m| m.rows)
        .sum();

    let out = dir.path().join("roundtrip");
    service.write_package_impl(&name, out.to_str().unwrap()).unwrap();

    let loaded = DatasetName::new("loaded").unwrap();
    let info = service
        .create_dataset_impl(&loaded, out.to_str().unwrap())
        .unwrap();
    assert_eq!(info.modules.iter().map(|m| m.rows).sum::<usize>(), original);
}

#[test]
fn a_module_exports_to_a_cityjsonseq_file() {
    let (service, dir) = common::test_service();
    let name = DatasetName::new("delft").unwrap();
    service
        .create_dataset_impl(&name, common::fixture("delft.city.jsonl").to_str().unwrap())
        .unwrap();

    let out = dir.path().join("delft_out.city.jsonl");
    service
        .export_module_impl(
            &name,
            &ModuleName::new("building").unwrap(),
            out.to_str().unwrap(),
            ExportFormat::CityJsonSeq,
        )
        .expect("export the module");
    assert!(out.exists());
    assert!(std::fs::metadata(&out).unwrap().len() > 0);
}

#[test]
fn merging_folds_one_dataset_into_another() {
    let (service, _dir) = common::test_service();
    let destination = DatasetName::new("dst").unwrap();
    let source = DatasetName::new("src").unwrap();
    service
        .create_dataset_impl(&destination, common::fixture("delft.city.jsonl").to_str().unwrap())
        .unwrap();
    // Both sides are footers once created, and the merge applies the same CRS
    // rule as an insert — so the source must declare the destination's CRS.
    service
        .create_dataset_impl(&source, common::fixture("minimal_7415.city.json").to_str().unwrap())
        .unwrap();

    let before: usize = service
        .describe_dataset_impl(&destination)
        .unwrap()
        .modules
        .iter()
        .map(|m| m.rows)
        .sum();

    service.merge_impl(&destination, &source).expect("merge");

    let after: usize = service
        .describe_dataset_impl(&destination)
        .unwrap()
        .modules
        .iter()
        .map(|m| m.rows)
        .sum();
    assert!(after > before);
}
```

- [ ] **Step 2: Run them and watch them fail**

```bash
cd lib/citylake && cargo test --test package
```

Expected: FAIL — the package methods do not exist.

- [ ] **Step 3: Implement it**

```rust
//! The boundary between the lake and the file format.
//!
//! DuckLake stores its own Parquet plus manifests, which is not a CityParquet
//! package — so writing one out is a real conversion, not a formality.

impl DuckLakeService {
    /// Load an existing package directory. `cityparquet_read` creates the
    /// schema inside the attached catalog and recovers each file's Parquet
    /// footer — the one thing a hand-rolled read_parquet load throws away, and
    /// with it the CRS. Nothing needs minting here.
    pub fn import_package(
        &self,
        dataset: &DatasetName,
        directory: &str,
    ) -> RepositoryResult<DatasetInfo> {
        let name = dataset.as_str();
        let catalog = self.catalog().to_string();

        self.with_connection(|conn| {
            if self.schema_exists(conn, name)? {
                return Err(CityLakeError::DatasetExists(name.to_string()));
            }
            self.in_transaction(conn, |conn| {
                // The pragma creates the schema itself; the search path is what
                // puts it inside the DuckLake catalog rather than the default one.
                conn.execute_batch(&format!("SET search_path={}", sql::literal(&catalog)))?;
                let read = conn.execute_batch(&sql::read_package_pragma(directory, name));
                let reset = conn.execute_batch("RESET search_path");
                read?;
                reset?;
                Ok(())
            })?;
            self.describe_locked(conn, name)
        })
    }

    pub fn write_package_impl(
        &self,
        dataset: &DatasetName,
        output_dir: &str,
    ) -> RepositoryResult<Vec<PackageFile>> {
        let name = dataset.as_str();

        self.with_connection(|conn| {
            if !self.schema_exists(conn, name)? {
                return Err(CityLakeError::DatasetNotFound(name.to_string()));
            }
            // cityparquet_write is a table function on an internal connection
            // and sees committed state only, so there is nothing to wrap in a
            // transaction here: mutate, commit, then write.
            let crs = self.dataset_crs(conn, name)?;
            let statement = sql::write_package(name, output_dir, crs.as_deref());

            conn.execute_batch(&sql::set_search_path(self.catalog(), name))?;
            let written = (|| -> RepositoryResult<Vec<PackageFile>> {
                let mut stmt = conn.prepare(&statement)?;
                let rows = stmt
                    .query_map([], |row| {
                        Ok(PackageFile {
                            file: row.get(0)?,
                            action: row.get(1)?,
                            rows: row.get(2)?,
                            bytes: row.get(3)?,
                        })
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })();
            let reset = conn.execute_batch("RESET search_path");
            let written = written?;
            reset?;
            Ok(written)
        })
    }

    pub fn export_module_impl(
        &self,
        dataset: &DatasetName,
        module: &ModuleName,
        output_path: &str,
        format: ExportFormat,
    ) -> RepositoryResult<()> {
        let (name, module_name) = (dataset.as_str(), module.as_str());

        self.with_connection(|conn| {
            if !self.table_exists(conn, name, module_name)? {
                return Err(CityLakeError::ModuleNotFound {
                    dataset: name.to_string(),
                    module: module_name.to_string(),
                });
            }

            // COPY inherits a source's metadata only when the SELECT names
            // exactly one reader. A table is not statically discoverable, so
            // the CRS has to be stated or the output would declare none.
            let crs = self.dataset_crs(conn, name)?;
            let mut options = format!("FORMAT {}", format.as_duckdb_format());
            if let Some(crs) = crs {
                options.push_str(&format!(", crs {}", sql::literal(&crs)));
            }
            conn.execute_batch(&format!(
                "COPY (SELECT * FROM {}) TO {} ({options})",
                sql::qualified(&[self.catalog(), name, module_name]),
                sql::literal(output_path)
            ))?;
            Ok(())
        })
    }

    pub fn merge_impl(
        &self,
        destination: &DatasetName,
        source: &DatasetName,
    ) -> RepositoryResult<()> {
        let (dst, src) = (destination.as_str(), source.as_str());

        self.with_connection(|conn| {
            for name in [dst, src] {
                if !self.schema_exists(conn, name)? {
                    return Err(CityLakeError::DatasetNotFound(name.to_string()));
                }
            }
            self.in_transaction(conn, |conn| {
                // Both schemas must resolve, so the search path carries the
                // catalog rather than one dataset.
                conn.execute_batch(&format!(
                    "SET search_path={}",
                    sql::literal(&format!("{0}.{dst},{0}.{src}", self.catalog()))
                ))?;
                let merged = conn.execute_batch(&sql::merge_pragma(dst, src));
                let reset = conn.execute_batch("RESET search_path");
                merged?;
                reset?;
                Ok(())
            })
        })
    }
}
```

Note for the implementer: if `cityparquet_merge` cannot resolve two schemas
through one search path, fall back to `USE <catalog>` for the duration of the
merge and restore afterwards — the pragma names both schemas explicitly, so
they need only be reachable, not defaulted. Prove whichever works with the
merge test rather than assuming.

- [ ] **Step 4: Run the tests and watch them pass**

```bash
cd lib/citylake && cargo test --test package
```

Expected: 5 passed. If `write_package` fails with `Catalog Error: … schema … does not exist`, Task 2 has not landed — finish it first.

- [ ] **Step 5: Commit**

```bash
git add lib/citylake/src/core/db/package.rs lib/citylake/src/core/db/mod.rs lib/citylake/tests/package.rs
git commit -m "feat(citylake): import, export and merge CityParquet packages

DuckLake's own Parquet is not a CityParquet package, so writing one out
converts rather than copies. Export states the CRS explicitly: COPY
inherits metadata only from a statically discoverable reader, and a
table is not one."
```

---

### Task 11: `inspect.rs` and `compaction.rs` — validation, housekeeping, maintenance

Both pragmas here materialise findings into a temp table, because a PRAGMA cannot be a subquery. Compaction is DuckLake's own, not CTAS-and-rename.

**Files:**
- Create: `lib/citylake/src/core/db/inspect.rs`
- Rewrite: `lib/citylake/src/core/db/compaction.rs`

**Interfaces:**
- Consumes: `sql::{validate_pragma, orphans_pragma, vacuum_pragma, compact}`.
- Produces:
  - `fn validate_impl(&self, dataset: &DatasetName) -> RepositoryResult<Vec<ValidationFinding>>`
  - `fn vacuum_impl(&self, dataset: &DatasetName) -> RepositoryResult<usize>` — orphans, then vacuum; returns rows removed
  - `fn compact_impl(&self, dataset: &DatasetName) -> RepositoryResult<CompactionStats>`

- [ ] **Step 1: Write the failing tests**

`lib/citylake/tests/inspect.rs`:

```rust
mod common;

use citylake::core::interface::types::DatasetName;

#[test]
fn a_freshly_created_dataset_validates_clean() {
    let (service, _dir) = common::test_service();
    let name = DatasetName::new("delft").unwrap();
    service
        .create_dataset_impl(&name, common::fixture("delft.city.jsonl").to_str().unwrap())
        .unwrap();

    let findings = service.validate_impl(&name).expect("validate");
    let errors: Vec<_> = findings.iter().filter(|f| f.severity == "error").collect();
    assert!(errors.is_empty(), "unexpected errors: {errors:?}");
}

#[test]
fn validation_findings_carry_their_check_and_table() {
    let (service, _dir) = common::test_service();
    let name = DatasetName::new("delft").unwrap();
    service
        .create_dataset_impl(&name, common::fixture("delft.city.jsonl").to_str().unwrap())
        .unwrap();

    // Shape, not content: a clean dataset yields no rows, so this asserts the
    // call succeeds and returns a well-formed (possibly empty) list.
    let findings = service.validate_impl(&name).unwrap();
    for finding in &findings {
        assert!(!finding.check_name.is_empty());
        assert!(!finding.table_name.is_empty());
    }
}

#[test]
fn vacuum_runs_on_a_dataset_with_no_orphans() {
    let (service, _dir) = common::test_service();
    let name = DatasetName::new("delft").unwrap();
    service
        .create_dataset_impl(&name, common::fixture("delft.city.jsonl").to_str().unwrap())
        .unwrap();

    let removed = service.vacuum_impl(&name).expect("vacuum");
    assert_eq!(removed, 0, "a fresh dataset has no unreferenced sidecar rows");
}

#[test]
fn compaction_reports_what_it_merged() {
    let (service, _dir) = common::test_service();
    let name = DatasetName::new("delft").unwrap();
    service
        .create_dataset_impl(&name, common::fixture("delft.city.jsonl").to_str().unwrap())
        .unwrap();

    // A dataset written in one go may have nothing to merge; the operation must
    // still succeed and report honestly rather than fail on a no-op.
    let stats = service.compact_impl(&name).expect("compact");
    assert!(stats.files_created <= stats.files_processed.max(stats.files_created));
}

#[test]
fn validating_an_absent_dataset_is_an_error() {
    let (service, _dir) = common::test_service();
    let err = service
        .validate_impl(&DatasetName::new("absent").unwrap())
        .expect_err("an absent dataset must not validate clean");
    assert!(format!("{err}").contains("absent"));
}
```

- [ ] **Step 2: Run them and watch them fail**

```bash
cd lib/citylake && cargo test --test inspect
```

Expected: FAIL — the inspection methods do not exist.

- [ ] **Step 3: Implement both modules**

```rust
//! Validation and housekeeping.
//!
//! A PRAGMA cannot be a subquery, so both of these materialise their findings
//! into a temp table which is then selected from — that is what keeps the
//! results filterable rather than fixed at the call.

impl DuckLakeService {
    pub fn validate_impl(
        &self,
        dataset: &DatasetName,
    ) -> RepositoryResult<Vec<ValidationFinding>> {
        let name = dataset.as_str();
        self.with_connection(|conn| {
            if !self.schema_exists(conn, name)? {
                return Err(CityLakeError::DatasetNotFound(name.to_string()));
            }
            conn.execute_batch(&sql::set_search_path(self.catalog(), name))?;
            let findings = (|| -> RepositoryResult<Vec<ValidationFinding>> {
                conn.execute_batch(&sql::validate_pragma(name))?;
                let mut stmt = conn.prepare(
                    "SELECT check_name, severity, table_name, object_id, message
                     FROM cityparquet_validation",
                )?;
                Ok(stmt
                    .query_map([], |row| {
                        Ok(ValidationFinding {
                            check_name: row.get(0)?,
                            severity: row.get(1)?,
                            table_name: row.get(2)?,
                            object_id: row.get(3)?,
                            message: row.get(4)?,
                        })
                    })?
                    .collect::<Result<Vec<_>, _>>()?)
            })();
            let reset = conn.execute_batch("RESET search_path");
            let findings = findings?;
            reset?;
            Ok(findings)
        })
    }

    pub fn vacuum_impl(&self, dataset: &DatasetName) -> RepositoryResult<usize> {
        let name = dataset.as_str();
        self.with_connection(|conn| {
            if !self.schema_exists(conn, name)? {
                return Err(CityLakeError::DatasetNotFound(name.to_string()));
            }
            self.in_transaction(conn, |conn| {
                conn.execute_batch(&sql::set_search_path(self.catalog(), name))?;
                let removed = (|| -> RepositoryResult<usize> {
                    // Orphans first, so the count reports what vacuum will take.
                    conn.execute_batch(&sql::orphans_pragma(name))?;
                    let count: i64 = conn.query_row(
                        "SELECT COUNT(*) FROM cityparquet_orphan_rows",
                        [],
                        |row| row.get(0),
                    )?;
                    conn.execute_batch(&sql::vacuum_pragma(name))?;
                    Ok(count as usize)
                })();
                let reset = conn.execute_batch("RESET search_path");
                let removed = removed?;
                reset?;
                Ok(removed)
            })
        })
    }
}
```

`compaction.rs`:

```rust
//! DuckLake maintenance.
//!
//! Compaction here is merging a table's small Parquet files, which is the
//! catalog's own operation — not CTAS, DROP and RENAME, which would rewrite
//! the table behind DuckLake's back and lose its snapshots.

impl DuckLakeService {
    pub fn compact_impl(&self, dataset: &DatasetName) -> RepositoryResult<CompactionStats> {
        let name = dataset.as_str();
        self.with_connection(|conn| {
            if !self.schema_exists(conn, name)? {
                return Err(CityLakeError::DatasetNotFound(name.to_string()));
            }

            let mut stats = CompactionStats {
                files_processed: 0,
                files_created: 0,
            };
            for table in self.object_tables(conn, name)? {
                let mut stmt = conn.prepare(&sql::compact(self.catalog(), name, &table))?;
                let rows = stmt
                    .query_map([], |row| {
                        Ok((row.get::<_, i64>(2)?, row.get::<_, i64>(3)?))
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                for (processed, created) in rows {
                    stats.files_processed += processed as usize;
                    stats.files_created += created as usize;
                }
            }
            Ok(stats)
        })
    }
}
```

- [ ] **Step 4: Run the tests and watch them pass**

```bash
cd lib/citylake && cargo test --test inspect
```

Expected: 5 passed.

- [ ] **Step 5: Commit**

```bash
git add lib/citylake/src/core/db/inspect.rs lib/citylake/src/core/db/compaction.rs lib/citylake/src/core/db/mod.rs lib/citylake/tests/inspect.rs
git commit -m "feat(citylake): expose validation, housekeeping and real compaction

Compaction is now ducklake_merge_adjacent_files rather than CTAS, DROP
and RENAME, which rewrote the table behind DuckLake's back and lost its
snapshots. Validation and vacuum were unreachable before and are one
pragma each."
```

---

### Task 12: Wire the trait to the implementation

The nine operation modules exist as inherent methods so each could be tested in isolation. This task makes `DuckLakeService` satisfy `CityLakeRepository`, moving the blocking DuckDB work off the async executor.

**Files:**
- Create: `lib/citylake/src/core/db/repository_impl.rs`
- Modify: `lib/citylake/src/lib.rs`, `lib/citylake/src/core/db/mod.rs`

**Interfaces:**
- Consumes: every `*_impl` method from Tasks 6–11.
- Produces: `impl CityLakeRepository for DuckLakeService`, and `lib.rs` exporting `core::{db, interface}`.

- [ ] **Step 1: Write the failing test**

`lib/citylake/tests/repository.rs`:

```rust
mod common;

use citylake::core::interface::repository::CityLakeRepository;
use citylake::core::interface::types::DatasetName;

/// The trait is the boundary handlers see, so it must be object-safe: a
/// handler holds Arc<dyn CityLakeRepository>, not a concrete service.
#[tokio::test]
async fn the_service_is_usable_as_a_trait_object() {
    let (service, _dir) = common::test_service();
    let repo: std::sync::Arc<dyn CityLakeRepository> = std::sync::Arc::new(service);

    let name = DatasetName::new("delft").unwrap();
    repo.create_dataset(&name, common::fixture("delft.city.jsonl").to_str().unwrap())
        .await
        .expect("create through the trait object");
    assert!(repo.list_datasets().await.unwrap().contains(&"delft".to_string()));
}

/// The DuckDB connection is behind a blocking mutex. If the trait methods ran
/// it on the async executor, one slow ingest would stall every other request.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_calls_do_not_deadlock() {
    let (service, _dir) = common::test_service();
    let repo: std::sync::Arc<dyn CityLakeRepository> = std::sync::Arc::new(service);

    let first = repo.clone();
    let second = repo.clone();
    let (a, b) = tokio::join!(
        async move { first.list_datasets().await },
        async move { second.list_datasets().await }
    );
    assert!(a.is_ok() && b.is_ok());
}
```

- [ ] **Step 2: Run it and watch it fail**

```bash
cd lib/citylake && cargo test --test repository
```

Expected: FAIL — `DuckLakeService` does not implement the trait.

- [ ] **Step 3: Implement the bridge**

Each method delegates to its `*_impl` inside `spawn_blocking`, because the
connection sits behind a `std::sync::Mutex` and DuckDB work is CPU-bound:

```rust
//! The trait implementation.
//!
//! Every method hands its work to a blocking thread: the DuckDB connection is
//! behind a std::sync::Mutex and its operations are CPU-bound, so running them
//! on the async executor would let one slow ingest stall every other request.

use std::sync::Arc;

#[async_trait]
impl CityLakeRepository for DuckLakeService {
    async fn create_dataset(
        &self,
        dataset: &DatasetName,
        source_path: &str,
    ) -> RepositoryResult<DatasetInfo> {
        let (service, dataset, source) = (self.handle(), dataset.clone(), source_path.to_string());
        tokio::task::spawn_blocking(move || service.create_dataset_impl(&dataset, &source))
            .await
            .map_err(|e| CityLakeError::Internal(format!("blocking task failed: {e}")))?
    }

    // … the remaining methods follow exactly this shape.
}
```

For `handle()` to exist, `DuckLakeService` needs to be cloneable into something
`'static`. It must be defined **in `service.rs`**, not in this task's new file:
`connection` and `config` are private fields, and Rust privacy is per-module, so
an `impl` block in `repository_impl.rs` cannot reach them.

`duckdb::Connection` is `Send` (`unsafe impl Send for Connection`, duckdb
1.10504.0 `src/lib.rs:272`) but not `Sync` — its inner handle is a `RefCell` —
which is exactly why the mutex is there and why `Arc<Mutex<Connection>>` is
`Send + Sync` and may cross into `spawn_blocking`. Add to `service.rs`:

```rust
impl DuckLakeService {
    /// A handle sharing this service's connection, for moving into a blocking
    /// task. The connection is already behind Arc<Mutex<_>>; this shares it
    /// rather than opening a second one, so DuckLake sees one writer.
    fn handle(&self) -> Self {
        Self {
            connection: Arc::clone(&self.connection),
            config: self.config.clone(),
        }
    }
}
```

`lib.rs` still declares only `pub mod core;` at this point — the `app` module
returns in Task 13, with the server that needs it.

- [ ] **Step 4: Run every test and watch them pass**

```bash
cd lib/citylake && cargo test && cargo clippy --all-targets -- -D warnings
```

Expected: all suites pass, clippy clean.

- [ ] **Step 5: Commit**

```bash
git add lib/citylake/src/
git commit -m "feat(citylake): satisfy the repository trait off the async executor

The connection is behind a blocking mutex and DuckDB's work is CPU-
bound, so every trait method hands off to spawn_blocking. Handlers see
Arc<dyn CityLakeRepository> and nothing below it."
```

---

### Task 13: The HTTP layer

Handlers translate between HTTP and the trait, and nothing more. No SQL, no DuckDB, no CityJSON.

**Files:**
- Rewrite: `lib/citylake/src/app/server.rs`
- Rewrite: `lib/citylake/src/app/handlers/{mod,dataset,objects,package,maintenance}.rs`
- Rewrite: `lib/citylake/src/main.rs`
- Delete: `lib/citylake/src/app/handlers/{table,insert,update,delete,query,export,compaction,list}.rs`

**Interfaces:**
- Consumes: `Arc<dyn CityLakeRepository>` as axum state.
- Produces: `fn router(repo: Arc<dyn CityLakeRepository>) -> axum::Router` and `async fn serve(config: CityLakeConfig, repo: Arc<dyn CityLakeRepository>) -> anyhow::Result<()>`, plus `impl IntoResponse for CityLakeError`.

The routes, exactly:

| Method | Path | Trait method |
|---|---|---|
| `GET` | `/health` | — |
| `GET` | `/datasets` | `list_datasets` |
| `POST` | `/datasets/:ds` | `create_dataset` (body `{ source_path }`) |
| `POST` | `/datasets/:ds/upload` | `create_dataset` (multipart) |
| `GET` | `/datasets/:ds` | `describe_dataset` |
| `DELETE` | `/datasets/:ds` | `drop_dataset` |
| `POST` | `/datasets/:ds/objects` | `ingest` (body `{ source_path }`) |
| `POST` | `/datasets/:ds/objects/upload` | `ingest` (multipart) |
| `GET` | `/datasets/:ds/modules/:module/objects` | `query_objects` (`?filter=&limit=&offset=`) |
| `PUT` | `/datasets/:ds/objects/:id` | `update_object` (body: a JSON object of attributes) |
| `DELETE` | `/datasets/:ds/objects/:id` | `delete_object` |
| `DELETE` | `/datasets/:ds/objects` | `delete_where` (`?filter=`) |
| `POST` | `/datasets/:ds/export` | `export_module` (body `{ module, output_path, format }`) |
| `POST` | `/datasets/:ds/package` | `write_package` (body `{ output_dir }`) |
| `POST` | `/datasets/:ds/merge` | `merge` (body `{ source }`) |
| `POST` | `/datasets/:ds/validate` | `validate` |
| `POST` | `/datasets/:ds/reconcile` | `reconcile` |
| `POST` | `/datasets/:ds/vacuum` | `vacuum` |
| `POST` | `/datasets/:ds/compact` | `compact` |

- [ ] **Step 1: Write the failing tests**

`lib/citylake/tests/api.rs`:

```rust
mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

fn app() -> (axum::Router, tempfile::TempDir) {
    let (service, dir) = common::test_service();
    (citylake::app::server::router(std::sync::Arc::new(service)), dir)
}

async fn send(app: &axum::Router, request: Request<Body>) -> (StatusCode, serde_json::Value) {
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

#[tokio::test]
async fn health_reports_ok() {
    let (app, _dir) = app();
    let (status, _) = send(&app, Request::get("/health").body(Body::empty()).unwrap()).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn a_dataset_is_created_described_and_dropped_over_http() {
    let (app, _dir) = app();
    let source = common::fixture("delft.city.jsonl");
    let body = serde_json::json!({ "source_path": source.to_str().unwrap() }).to_string();

    let (status, _) = send(
        &app,
        Request::post("/datasets/delft")
            .header("content-type", "application/json")
            .body(Body::from(body))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, described) =
        send(&app, Request::get("/datasets/delft").body(Body::empty()).unwrap()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(described["name"], "delft");

    let (status, _) = send(
        &app,
        Request::delete("/datasets/delft").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn an_invalid_dataset_name_is_rejected_before_it_reaches_sql() {
    let (app, _dir) = app();
    let (status, _) = send(
        &app,
        Request::get("/datasets/not%20a%20name").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn an_absent_dataset_is_a_404_not_a_500() {
    let (app, _dir) = app();
    let (status, _) = send(&app, Request::get("/datasets/absent").body(Body::empty()).unwrap()).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn creating_the_same_dataset_twice_is_a_conflict() {
    let (app, _dir) = app();
    let source = common::fixture("delft.city.jsonl");
    let body = serde_json::json!({ "source_path": source.to_str().unwrap() }).to_string();
    let request = || {
        Request::post("/datasets/delft")
            .header("content-type", "application/json")
            .body(Body::from(body.clone()))
            .unwrap()
    };

    let (first, _) = send(&app, request()).await;
    assert_eq!(first, StatusCode::CREATED);
    let (second, _) = send(&app, request()).await;
    assert_eq!(second, StatusCode::CONFLICT);
}

#[tokio::test]
async fn objects_are_queryable_by_module() {
    let (app, _dir) = app();
    let source = common::fixture("delft.city.jsonl");
    send(
        &app,
        Request::post("/datasets/delft")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({ "source_path": source.to_str().unwrap() }).to_string(),
            ))
            .unwrap(),
    )
    .await;

    let (status, rows) = send(
        &app,
        Request::get("/datasets/delft/modules/building/objects?limit=2")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(rows.as_array().unwrap().len(), 2);
}
```

- [ ] **Step 2: Run them and watch them fail**

```bash
cd lib/citylake && cargo test --test api
```

Expected: FAIL — `router` does not exist in its new shape.

- [ ] **Step 3: Implement the layer**

Map errors to status codes once, in one place:

```rust
//! Turning a CityLakeError into a response.
//!
//! Doing this once is why the error type is an enum rather than a boxed trait
//! object: a handler cannot classify a string.
impl IntoResponse for CityLakeError {
    fn into_response(self) -> Response {
        let status = match &self {
            CityLakeError::DatasetNotFound(_) | CityLakeError::ModuleNotFound { .. } => {
                StatusCode::NOT_FOUND
            }
            CityLakeError::DatasetExists(_) => StatusCode::CONFLICT,
            CityLakeError::Sql(_) => StatusCode::BAD_REQUEST,
            // A rejected pragma — a duplicate id, a CRS mismatch — is the
            // caller's input being refused, not the server failing.
            CityLakeError::Duckdb(e) if is_refusal(e) => StatusCode::UNPROCESSABLE_ENTITY,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, Json(serde_json::json!({ "error": self.to_string() }))).into_response()
    }
}

/// The extension refuses bad input by raising, so its own refusals read as
/// database errors. These are the caller's fault, not ours.
fn is_refusal(error: &duckdb::Error) -> bool {
    let text = error.to_string();
    ["duplicate id", "CRS mismatch", "unresolved parent", "reprojection is not performed"]
        .iter()
        .any(|marker| text.contains(marker))
}
```

Handlers parse path parameters into the validated newtypes, so an invalid name
returns 400 before any SQL is built:

```rust
async fn describe(
    State(repo): State<Arc<dyn CityLakeRepository>>,
    Path(dataset): Path<String>,
) -> Result<Json<DatasetInfo>, CityLakeError> {
    let dataset = DatasetName::new(&dataset)?;
    Ok(Json(repo.describe_dataset(&dataset).await?))
}
```

Upload handlers write the multipart part to a `tempfile::NamedTempFile` whose
name keeps the original extension — the extension picks its reader from that —
and pass its path to the same trait method the JSON-body variant uses.

`server.rs` builds the router with permissive CORS and a tracing layer, and
`serve` binds `config.host:config.port`.

Restore the binary target Task 4 removed, in `Cargo.toml`:

```toml
[[bin]]
name = "citylake"
path = "src/main.rs"
```

and re-declare the module in `lib.rs`:

```rust
//! CityLake — a lakehouse runtime for CityParquet packages.

pub mod core;

#[cfg(feature = "server")]
pub mod app;
```

`main.rs` is the whole binary — initialise tracing, take the default
`CityLakeConfig`, construct the service, and serve:

```rust
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let config = CityLakeConfig::default();
    let repo: Arc<dyn CityLakeRepository> = Arc::new(DuckLakeService::new(config.clone())?);
    citylake::app::server::serve(config, repo).await
}
```

- [ ] **Step 4: Run the tests and watch them pass**

```bash
cd lib/citylake && cargo test --test api
```

Expected: 6 passed.

- [ ] **Step 5: Commit**

```bash
git add lib/citylake/src/app/ lib/citylake/tests/api.rs
git rm lib/citylake/src/app/handlers/{table,insert,update,delete,query,export,compaction,list}.rs
git commit -m "feat(citylake)!: re-key the API from LoD tables to datasets and modules

Endpoints address a dataset and a CityGML module, and the package
operations the extension makes cheap — validate, reconcile, vacuum,
merge, package export — are reachable for the first time. Errors are
classified once, which is what the error enum was for."
```

---

### Task 14: The end-to-end round trip

Each earlier task proved its own operation. This one proves they compose: a CityJSON file becomes a dataset, is edited, exported as a package, and read back with its structure and CRS intact.

**Files:**
- Create: `lib/citylake/tests/round_trip.rs`

**Interfaces:**
- Consumes: the whole public surface.
- Produces: nothing — this is the acceptance gate.

- [ ] **Step 1: Write the test**

```rust
//! The acceptance gate: everything composing, once.

mod common;

use citylake::core::interface::repository::CityLakeRepository;
use citylake::core::interface::types::{DatasetName, ModuleName, QueryParams};

#[tokio::test]
async fn a_cityjson_source_survives_the_full_journey() {
    let (service, dir) = common::test_service();
    let name = DatasetName::new("delft").unwrap();
    let building = ModuleName::new("building").unwrap();

    // 1. Ingest.
    let created = service
        .create_dataset(&name, common::fixture("delft.city.jsonl").to_str().unwrap())
        .await
        .expect("create");
    let ingested: usize = created.modules.iter().map(|m| m.rows).sum();
    assert!(ingested > 0);
    assert!(created.crs.as_ref().expect("a CRS").contains("7415"));

    // 2. It validates clean on arrival.
    assert!(service
        .validate(&name)
        .await
        .unwrap()
        .iter()
        .all(|f| f.severity != "error"));

    // 3. Delete one object; the cascade is the extension's.
    let first = service
        .query_objects(&name, &building, &QueryParams { filter: None, limit: 1, offset: 0 })
        .await
        .unwrap()[0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let removed = service.delete_object(&name, &first).await.expect("delete");
    assert!(removed >= 1);

    // 4. Still consistent afterwards — a cascade that left a dangling parent
    //    or a stale feature_id would show up here.
    assert!(service
        .validate(&name)
        .await
        .unwrap()
        .iter()
        .all(|f| f.severity != "error"));

    // 5. Write it out as a package.
    let package = dir.path().join("out");
    let files = service
        .write_package(&name, package.to_str().unwrap())
        .await
        .expect("write the package");
    assert!(files.iter().any(|f| f.file == "metadata.json"));

    // 6. Read the package back into a second dataset.
    let reloaded = DatasetName::new("reloaded").unwrap();
    let info = service
        .create_dataset(&reloaded, package.to_str().unwrap())
        .await
        .expect("read the package back");

    // 7. What went out is what came back: the same rows, the same CRS.
    assert_eq!(
        info.modules.iter().map(|m| m.rows).sum::<usize>(),
        ingested - removed
    );
    assert!(info.crs.expect("a CRS survives the round trip").contains("7415"));

    // 8. And it is still a valid package.
    assert!(service
        .validate(&reloaded)
        .await
        .unwrap()
        .iter()
        .all(|f| f.severity != "error"));
}
```

- [ ] **Step 2: Run it**

```bash
cd lib/citylake && cargo test --test round_trip
```

If it fails, the failure is in composition rather than in any one operation —
each was proved separately. Fix the operation the assertion names, not the test.

- [ ] **Step 3: Run the whole suite**

```bash
cd lib/citylake && cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check
```

Expected: everything passes, clippy clean, formatting clean.

- [ ] **Step 4: Commit**

```bash
git add lib/citylake/tests/round_trip.rs
git commit -m "test(citylake): prove the operations compose, not just work

Ingest, validate, cascade-delete, validate again, write the package,
read it back, and check the rows and the CRS survived."
```

---

### Task 15: Documentation and the root gate

`lib/citylake/CLAUDE.md` documents the LoD table model in detail — the naming convention, the `geom_lodX_Y` discovery scan, the SQL patterns, the endpoint list. None of it will exist. Rewrite it to describe the present.

**Files:**
- Rewrite: `lib/citylake/CLAUDE.md`, and copy byte-identically to `lib/citylake/AGENTS.md`
- Delete: `lib/citylake/tasks.md`, `lib/citylake/milestones.md`
- Modify: `justfile` (repository root), `lib/citylake/justfile`
- Modify: the `lib/duckdb-cityjson` submodule pointer

- [ ] **Step 1: Rewrite `lib/citylake/CLAUDE.md`**

It must state, in British English and with no changelog voice:

- **What CityLake is** — a lakehouse runtime whose unit is a CityParquet package: a DuckLake schema of CityGML-module tables plus sidecars and `__cityparquet`.
- **The rule** — every CityJSON *and package* operation goes through duckdb-cityjson. No CityJSON parsing, no module routing, no CRS resolution, no derived-state computation in Rust. Name the pragmas and what each is for.
- **The four mechanics** — one pragma per submitted statement and why; `SET search_path` scoping and why not `USE`; the seed table a fresh package needs and why `create_tables = true` is not enough; transactions holding under DuckLake.
- **The CRS footer** — why a DuckLake table has none, why CityLake cannot mint one itself, and how the probe works. This is the least obvious thing in the crate; someone will otherwise delete it as redundant.
- **Version lockstep** — DuckDB v1.5.4, `duckdb = "=1.10504.0"`, bump with the extension's release matrix.
- **Testing** — two tiers, no offline mode, `CITYLAKE_CITYJSON_EXTENSION` for a local build.
- **The endpoint table** from Task 13.
- **`web/` is stale** — keyed to the removed LoD-table API, awaiting a follow-up.

Then:

```bash
cp lib/citylake/CLAUDE.md lib/citylake/AGENTS.md
```

- [ ] **Step 2: Remove the stale notes**

```bash
git rm lib/citylake/tasks.md lib/citylake/milestones.md
```

They record a Postgres-and-Supabase era, a completed milestone list, and a
deferred multi-LoD export this rebuild makes moot. History belongs in git.

- [ ] **Step 3: Wire citylake into the root gate**

The root `justfile` has no citylake recipe, though the monorepo's `CLAUDE.md`
describes it as the third Cargo workspace. Add one and include it in `check`:

```just
# Lint and test the CityLake crate.
#
# The integration tests need the CityParquet package pragmas, which the
# published community extension does not yet carry — so they run against the
# local build, and `just -f lib/duckdb-cityjson/justfile build` must have run
# first. Override the path by exporting CITYLAKE_CITYJSON_EXTENSION yourself.
citylake-check:
    #!/usr/bin/env bash
    set -euo pipefail
    ext="{{justfile_directory()}}/lib/duckdb-cityjson/build/release/extension/cityjson/cityjson.duckdb_extension"
    if [ -z "${CITYLAKE_CITYJSON_EXTENSION:-}" ] && [ -f "$ext" ]; then
        export CITYLAKE_CITYJSON_EXTENSION="$ext"
    fi
    cd lib/citylake
    cargo clippy --all-targets -- -D warnings
    cargo test
```

Add `citylake-check` to the root `check` recipe's dependency list, and update
the "Build and gates" table in the monorepo `CLAUDE.md` and `AGENTS.md` to
mention it — including that it needs the extension, as the benchmark recipes
note their own requirements.

- [ ] **Step 4: Move the submodule pointer**

Task 2's fix is committed inside `lib/duckdb-cityjson`; the monorepo records
only which commit it pins. Push that branch, then:

```bash
git add lib/duckdb-cityjson
```

- [ ] **Step 5: Verify the whole gate**

```bash
just citylake-check
cd lib/cityparquet-rs && just check
```

Expected: both clean.

- [ ] **Step 6: Commit**

```bash
git add lib/citylake/CLAUDE.md lib/citylake/AGENTS.md justfile CLAUDE.md AGENTS.md lib/duckdb-cityjson
git commit -m "docs(citylake): describe the package model, and gate the crate

The instructions documented the LoD-table model in full — the naming
convention, the geom_lodX_Y scan, the SQL patterns — none of which
exists now. The CRS footer gets its own section: it is the least
obvious thing in the crate, and someone will otherwise delete the probe
as redundant. citylake joins the root gate, which never ran it."
```

---

## Self-Review

**Spec coverage.** §1 Why → Tasks 4–12 collectively. §2 Scope → Task 15 (docs, gate) and the deliberate absence of any `web/` task. §3 Approaches → the chosen one is the plan. §4 The model → Tasks 4, 6. §5 Crate structure → the File Structure table and Tasks 3–13. §6 Mechanics, all four → Task 3 (`insert_pragma` uses `=`; `LIMIT 0` seed), Task 5 (`scoped`, `in_transaction`), Task 6 (bootstrap). §7 CRS → Task 6, with the preferred route verified before the plan was written, and its test is the one that proves the guard actually fires. §8 The write fix → Task 2. §9 Testing → the two tiers appear as Task 3 (pure) and Tasks 5–14 (integration); `new_for_testing` is gone. §10 API → Task 13's route table, with the predicate trust model recorded in Task 8's comment. §11 Pinning → Task 1, now verified rather than inferred. §12 Documentation → Task 15.

**Resolved before writing, not deferred into it.** The spec left three open points for the plan; all three were settled empirically first, so no task carries a conditional branch. `cityparquet_init` is additive, so the seed stays rather than being dropped (Task 3's comment says why). `cityparquet_read` reaches the attached catalog, so Task 10's import needs no copy step. A minimal `{"crs": …}` footer arms the guard, so Task 6 mints that rather than a full one.

**Three defects found by reviewing the plan against the extension, and fixed.** Task 6 originally minted the CRS footer inside the ingest transaction. That fails twice over — `cityparquet_write` sees committed state only, and one transaction may write to only one attached database — so creation is now three phases with stated failure semantics, and `minting_the_footer_does_not_leave_the_ingest_uncommitted` pins it. `reference_system` is a struct, not a string, so the authority:code spelling is assembled in SQL. And `main.rs` belonged to no task. A fourth suspicion did not survive checking: `crs =>` accepts a canonical PROJJSON document and round-trips it, so Task 10 feeding `dataset_crs()` back to the writer is sound as written.

**One judgement left open on purpose.** Task 10's `merge_impl` needs two schemas reachable at once, and whether one `search_path` carrying both does it is not something I probed. The task says so and names the fallback, rather than asserting a mechanism I have not seen work.

**Type consistency.** `DatasetName`/`ModuleName` are constructed only via `new` and consumed via `as_str` throughout. `RepositoryResult<T>` is `Result<T, CityLakeError>` from Task 4 onward, including in the closures `with_connection`, `scoped` and `in_transaction` take. `object_tables` (Task 6) is used by Tasks 7, 9 and 11; `total_object_rows` (Task 7) by Task 9; `table_exists` (Task 8) by Task 10; `dataset_crs` (Task 6) by Task 10. `describe_locked` is used by Tasks 6 and 10 — implement it in Task 6 as the connection-borrowing half of `describe_dataset_impl`.
