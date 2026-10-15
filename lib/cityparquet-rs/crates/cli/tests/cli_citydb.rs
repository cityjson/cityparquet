//! `cityparquet convert postgresql://…`: a 3DCityDB v5 database as the
//! input (the `citydb` feature). The tests that reach a database read
//! `CITYDB_TEST_URL` (`lib/citygml-rs/crates/citydb/tests/setup-db.sh`
//! prints it) and skip without it.
#![cfg(feature = "citydb")]

use std::path::PathBuf;
use std::process::{Command, Output};

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

/// The URL of the database `setup-db.sh` loaded `dataset` into.
fn database(dataset: &str) -> Option<String> {
    let Ok(base) = std::env::var("CITYDB_TEST_URL") else {
        assert!(
            std::env::var_os("CITYDB_TEST_REQUIRED").is_none(),
            "CITYDB_TEST_REQUIRED is set but CITYDB_TEST_URL is not"
        );
        eprintln!("CITYDB_TEST_URL is not set: skipping");
        return None;
    };
    Some(format!("{}/{dataset}", base.trim_end_matches('/')))
}

fn cityparquet(args: &[&std::ffi::OsStr]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cityparquet"))
        .args(args)
        .output()
        .expect("failed to run cityparquet")
}

/// A database URL converts like a file: the package holds its objects.
#[test]
fn a_database_converts_to_a_package() {
    let Some(url) = database("delft") else {
        return;
    };
    let out = tempfile::tempdir().unwrap();
    let output = cityparquet(&[
        "convert".as_ref(),
        url.as_ref(),
        "-o".as_ref(),
        out.path().as_ref(),
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(out.path().join("building.parquet").exists());
    // 100 buildings, each with one building part.
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.starts_with("200 "), "{stdout}");
}

/// A password written into the URL works, with a warning: a command line
/// is visible to other users of the machine.
#[test]
fn a_password_in_the_url_is_warned_about() {
    let Some(url) = database("delft") else {
        return;
    };
    let userinfo = url
        .split_once("://")
        .and_then(|(_, rest)| rest.split_once('@'))
        .map(|(userinfo, _)| userinfo);
    if !userinfo.is_some_and(|u| u.contains(':')) {
        eprintln!("CITYDB_TEST_URL carries no password: skipping");
        return;
    }
    let out = tempfile::tempdir().unwrap();
    let output = cityparquet(&[
        "convert".as_ref(),
        url.as_ref(),
        "-o".as_ref(),
        out.path().as_ref(),
    ]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("PGPASSWORD"), "{stderr}");
}

/// `--citydb-schema` names the schema; one the database lacks is refused
/// by name.
#[test]
fn a_missing_schema_is_refused_by_name() {
    let Some(url) = database("delft") else {
        return;
    };
    let out = tempfile::tempdir().unwrap();
    let output = cityparquet(&[
        "convert".as_ref(),
        url.as_ref(),
        "--citydb-schema".as_ref(),
        "nosuchschema".as_ref(),
        "-o".as_ref(),
        out.path().as_ref(),
    ]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("nosuchschema"), "{stderr}");
}

/// What the database reader leaves out is reported, one warning per name.
#[test]
fn what_the_reader_leaves_out_is_warned_about() {
    let Some(url) = database("plateau_bldg") else {
        return;
    };
    let out = tempfile::tempdir().unwrap();
    let output = cityparquet(&[
        "convert".as_ref(),
        url.as_ref(),
        "-o".as_ref(),
        out.path().as_ref(),
    ]);
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("app:ParameterizedTexture without an image"),
        "{stderr}"
    );
}

/// A database is converted on its own: with another input, nothing is
/// opened and the conversion is refused.
#[test]
fn a_database_with_another_input_is_refused() {
    let out = tempfile::tempdir().unwrap();
    let output = cityparquet(&[
        "convert".as_ref(),
        "postgresql://localhost/nothing".as_ref(),
        fixture("delft.city.jsonl").as_os_str(),
        "-o".as_ref(),
        out.path().as_ref(),
    ]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("on its own"), "{stderr}");
}

/// `--citydb-schema` without a database input is refused.
#[test]
fn a_schema_without_a_database_is_refused() {
    let out = tempfile::tempdir().unwrap();
    let output = cityparquet(&[
        "convert".as_ref(),
        fixture("delft.city.jsonl").as_os_str(),
        "--citydb-schema".as_ref(),
        "citydb".as_ref(),
        "-o".as_ref(),
        out.path().as_ref(),
    ]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--citydb-schema"), "{stderr}");
}
