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
            Ok(
                conn.query_row("SELECT COUNT(*) FROM lake.reached.__cityparquet", [], |r| {
                    r.get(0)
                })?,
            )
        })
        .unwrap();
    assert_eq!(registered, 1);
}
