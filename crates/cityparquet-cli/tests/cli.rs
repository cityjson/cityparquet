use std::path::PathBuf;
use std::process::Command;

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

#[test]
fn convert_delft_to_tempdir_succeeds() {
    let out = tempfile::tempdir().unwrap();
    let binary = env!("CARGO_BIN_EXE_cityparquet");
    let output = Command::new(binary)
        .arg("convert")
        .arg(fixture("delft.city.jsonl"))
        .arg(out.path())
        .output()
        .expect("failed to run convert");

    assert!(output.status.success(), "convert command failed");
    assert!(out.path().join("cityobjects.parquet").exists());
    assert!(out.path().join("metadata.json").exists());

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("2231"),
        "stdout should contain object count"
    );
}

#[test]
fn convert_without_overwrite_fails_on_existing_output() {
    let out = tempfile::tempdir().unwrap();
    let binary = env!("CARGO_BIN_EXE_cityparquet");

    // First conversion succeeds
    let status = Command::new(binary)
        .arg("convert")
        .arg(fixture("delft.city.jsonl"))
        .arg(out.path())
        .status()
        .expect("failed to run first convert");
    assert!(status.success());

    // Second conversion without --overwrite should fail
    let output = Command::new(binary)
        .arg("convert")
        .arg(fixture("delft.city.jsonl"))
        .arg(out.path())
        .output()
        .expect("failed to run second convert");

    assert!(
        !output.status.success(),
        "convert should fail on existing output"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.is_empty(), "stderr should contain error message");
}

#[test]
fn convert_with_overwrite_succeeds() {
    let out = tempfile::tempdir().unwrap();
    let binary = env!("CARGO_BIN_EXE_cityparquet");

    // First conversion
    let status = Command::new(binary)
        .arg("convert")
        .arg(fixture("delft.city.jsonl"))
        .arg(out.path())
        .status()
        .expect("failed to run first convert");
    assert!(status.success());

    // Second conversion with --overwrite should succeed
    let status = Command::new(binary)
        .arg("convert")
        .arg(fixture("delft.city.jsonl"))
        .arg(out.path())
        .arg("--overwrite")
        .status()
        .expect("failed to run second convert");

    assert!(status.success(), "convert with --overwrite should succeed");
    assert!(out.path().join("cityobjects.parquet").exists());
    assert!(out.path().join("metadata.json").exists());
}
