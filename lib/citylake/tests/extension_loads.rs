//! The environment contract: the pinned DuckDB must be able to load the
//! CityJSON extension and expose the CityParquet package pragmas.
//!
//! `CITYLAKE_CITYJSON_EXTENSION` points at a locally built
//! `cityjson.duckdb_extension`; without it the community build is installed.

use duckdb::{Config, Connection};

/// Open an in-memory connection with the cityjson and ducklake extensions loaded.
///
/// `allow_unsigned_extensions` is a startup-only DuckDB option — it must be
/// set on the `Config` handed to the connection, not via `SET` afterwards —
/// so the local-build path opens the connection with that config rather than
/// opening plain and configuring it after the fact.
fn connect() -> Connection {
    let conn = match std::env::var("CITYLAKE_CITYJSON_EXTENSION") {
        Ok(path) => {
            let config = Config::default()
                .allow_unsigned_extensions()
                .expect("allow unsigned extensions");
            let conn =
                Connection::open_in_memory_with_flags(config).expect("open in-memory duckdb");
            conn.execute_batch(&format!("LOAD '{path}';"))
                .expect("load the local cityjson build");
            conn
        }
        Err(_) => {
            let conn = Connection::open_in_memory().expect("open in-memory duckdb");
            conn.execute_batch("INSTALL cityjson FROM community; LOAD cityjson;")
                .expect("install and load cityjson from community");
            conn
        }
    };
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
    // `cityparquet_orphans` belongs here too: `vacuum_impl` (src/core/db/inspect.rs)
    // calls it directly, alongside `cityparquet_vacuum`.
    for name in [
        "cityparquet_read",
        "cityparquet_init",
        "cityparquet_write",
        "cityparquet_merge",
        "cityparquet_delete",
        "cityparquet_reconcile",
        "cityparquet_validate",
        "cityparquet_orphans",
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
    for name in [
        "insert_cityjson",
        "insert_cityjsonseq",
        "insert_flatcitybuf",
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
fn ducklake_maintenance_function_is_registered() {
    let conn = connect();
    // compaction.rs::compact calls this directly. It is DuckLake's own
    // function, not the cityjson extension's, so it is not gated by the
    // version this contract otherwise pins — but the crate still depends on
    // it being there, and `LOAD ducklake` succeeding proves only that the
    // extension loaded, not that this particular function survived to the
    // version in use. Overloaded (multiple signatures), so `>= 1` rather
    // than `== 1`.
    let found: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM duckdb_functions() WHERE function_name = ?",
            ["ducklake_merge_adjacent_files"],
            |row| row.get(0),
        )
        .expect("look up ducklake_merge_adjacent_files");
    assert!(
        found >= 1,
        "ducklake_merge_adjacent_files is not registered"
    );
}

#[test]
fn metadata_table_functions_are_registered() {
    let conn = connect();
    // dataset.rs::mint_crs_footer calls one of these, by source format, to
    // read the CRS a source declares. Missing any means create_dataset_impl
    // cannot mint a footer for that format.
    for name in [
        "cityjson_metadata",
        "cityjsonseq_metadata",
        "flatcitybuf_metadata",
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
