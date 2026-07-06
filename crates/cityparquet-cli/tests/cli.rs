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

#[test]
fn export_package_to_cityjsonl() {
    let package_dir = tempfile::tempdir().unwrap();
    let binary = env!("CARGO_BIN_EXE_cityparquet");

    // First, convert delft to a package
    let status = Command::new(binary)
        .arg("convert")
        .arg(fixture("delft.city.jsonl"))
        .arg(package_dir.path())
        .status()
        .expect("failed to run convert");
    assert!(status.success());

    // Now export to CityJSONSeq
    let output_file = tempfile::tempdir().unwrap();
    let output_path = output_file.path().join("exported.city.jsonl");

    let output = Command::new(binary)
        .arg("export")
        .arg(package_dir.path())
        .arg(&output_path)
        .output()
        .expect("failed to run export");

    assert!(
        output.status.success(),
        "export command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output_path.exists(), "exported file does not exist");

    let stdout = String::from_utf8_lossy(&output.stdout);
    // stdout should contain: feature_count, object_count, instance_geometries_dropped, appearance_refs_dropped
    let parts: Vec<&str> = stdout.split_whitespace().collect();
    assert_eq!(
        parts.len(),
        4,
        "export should print 4 space-separated values, got: {}",
        stdout
    );
}

#[test]
fn export_and_compare_source_vs_exported_is_equal() {
    let package_dir = tempfile::tempdir().unwrap();
    let binary = env!("CARGO_BIN_EXE_cityparquet");

    // Convert delft to package
    let status = Command::new(binary)
        .arg("convert")
        .arg(fixture("delft.city.jsonl"))
        .arg(package_dir.path())
        .status()
        .expect("failed to run convert");
    assert!(status.success());

    // Export package back to CityJSONSeq
    let export_file = tempfile::tempdir().unwrap();
    let export_path = export_file.path().join("exported.city.jsonl");

    let status = Command::new(binary)
        .arg("export")
        .arg(package_dir.path())
        .arg(&export_path)
        .status()
        .expect("failed to run export");
    assert!(status.success());

    // Compare source vs exported
    let output = Command::new(binary)
        .arg("compare")
        .arg(fixture("delft.city.jsonl"))
        .arg(&export_path)
        .output()
        .expect("failed to run compare");

    assert!(output.status.success(), "compare should exit 0 when equal");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("equal"),
        "stdout should contain 'equal', got: {}",
        stdout
    );
}

#[test]
fn compare_different_datasets_returns_exit_2_with_differences() {
    let binary = env!("CARGO_BIN_EXE_cityparquet");

    // Compare delft vs railway (different datasets)
    let output = Command::new(binary)
        .arg("compare")
        .arg(fixture("delft.city.jsonl"))
        .arg(fixture("lod3_railway.city.json"))
        .output()
        .expect("failed to run compare");

    assert_eq!(
        output.status.code().unwrap(),
        2,
        "compare should exit 2 when datasets differ"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.is_empty(), "stdout should contain differences");
    // The CLI caps output at the first 20 differences plus one final
    // "... (N more)" line; delft vs railway has far more than 20.
    let lines: Vec<&str> = stdout.lines().collect();
    assert!(
        lines.len() <= 21,
        "compare must print at most 20 differences plus one truncation line, got {} lines",
        lines.len()
    );
    assert!(
        lines.last().unwrap().contains("more"),
        "with more than 20 differences the final line must be the '... (N more)' notice, got: {}",
        lines.last().unwrap()
    );
}

#[test]
fn export_and_compare_railway_with_exclusions() {
    let package_dir = tempfile::tempdir().unwrap();
    let binary = env!("CARGO_BIN_EXE_cityparquet");

    // Convert railway to package
    let status = Command::new(binary)
        .arg("convert")
        .arg(fixture("lod3_railway.city.json"))
        .arg(package_dir.path())
        .status()
        .expect("failed to run convert");
    assert!(status.success());

    // Export package
    let export_file = tempfile::tempdir().unwrap();
    let export_path = export_file.path().join("exported.city.json");

    let status = Command::new(binary)
        .arg("export")
        .arg(package_dir.path())
        .arg(&export_path)
        .status()
        .expect("failed to run export");
    assert!(status.success());

    // Compare with both exclusion flags
    let output = Command::new(binary)
        .arg("compare")
        .arg(fixture("lod3_railway.city.json"))
        .arg(&export_path)
        .arg("--exclude-appearance")
        .arg("--exclude-instances")
        .output()
        .expect("failed to run compare with exclusions");

    assert!(
        output.status.success(),
        "compare with exclusions should exit 0 when logically equal"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("equal"),
        "stdout should contain 'equal', got: {}",
        stdout
    );
}
