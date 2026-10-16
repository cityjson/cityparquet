//! What the database tests share: the URL of a dataset's database, and the
//! files the datasets were imported from.
#![allow(dead_code)]

use std::path::PathBuf;

use citygml_citydb::{CityDb, Config};

/// The database `tests/setup-db.sh` loaded `dataset` into, or `None` when
/// `CITYDB_TEST_URL` is not set. With `CITYDB_TEST_REQUIRED` set, a missing
/// URL is a failure rather than a skip, so a CI job that meant to run these
/// tests cannot pass without them.
pub fn database(dataset: &str) -> Option<CityDb> {
    let Ok(base) = std::env::var("CITYDB_TEST_URL") else {
        assert!(
            std::env::var_os("CITYDB_TEST_REQUIRED").is_none(),
            "CITYDB_TEST_REQUIRED is set but CITYDB_TEST_URL is not: run tests/setup-db.sh"
        );
        eprintln!("CITYDB_TEST_URL is not set: skipping (run tests/setup-db.sh)");
        return None;
    };
    let config = Config::new(format!("{}/{dataset}", base.trim_end_matches('/')));
    Some(CityDb::connect(&config).unwrap_or_else(|e| panic!("cannot open {dataset}: {e}")))
}

/// A file under `crates/`.
pub fn data(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(path)
}
