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
        .arg("-o")
        .arg(out.path())
        .output()
        .expect("failed to run convert");

    assert!(output.status.success(), "convert command failed");
    // delft is a single 1st-level family, so by-type conversion (the only,
    // mandatory layout) writes exactly one main table: building.parquet.
    assert!(out.path().join("building.parquet").exists());
    assert!(out.path().join("metadata.json").exists());

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("2231"),
        "stdout should contain object count"
    );
}

/// Multiple DISTINCT inputs (delft + a copy of it) merge into ONE package; the
/// object count doubles (2231 * 2 = 4462). Two distinct paths are needed
/// because `resolve_inputs` de-duplicates identical paths.
#[test]
fn convert_two_inputs_merges_into_one_package() {
    let out = tempfile::tempdir().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let copy = tmp.path().join("delft_copy.city.jsonl");
    std::fs::copy(fixture("delft.city.jsonl"), &copy).unwrap();
    let binary = env!("CARGO_BIN_EXE_cityparquet");
    let output = Command::new(binary)
        .arg("convert")
        .arg(fixture("delft.city.jsonl"))
        .arg(&copy)
        .arg("-o")
        .arg(out.path())
        .output()
        .expect("failed to run convert");

    assert!(
        output.status.success(),
        "merge convert failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.split_whitespace().next() == Some("4462"),
        "merged object count should be 2231*2=4462, got: {stdout}"
    );
    // Both merged inputs are delft (same single 1st-level family), so
    // by-type conversion still writes exactly one main table.
    assert!(out.path().join("building.parquet").exists());
}

/// `--partition count --number 3` writes 3 self-contained package subdirs.
#[test]
fn partition_count_writes_n_package_dirs() {
    let out = tempfile::tempdir().unwrap();
    let binary = env!("CARGO_BIN_EXE_cityparquet");
    let o = Command::new(binary)
        .arg("convert")
        .arg(fixture("delft.city.jsonl"))
        .arg("-o")
        .arg(out.path())
        .arg("--partition")
        .arg("count")
        .arg("--number")
        .arg("3")
        .output()
        .expect("run");
    assert!(
        o.status.success(),
        "partition convert failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    // delft is a single 1st-level family, so every partition's by-type
    // conversion writes exactly one main table: building.parquet.
    for i in 0..3 {
        assert!(
            out.path()
                .join(format!("count-{i:05}"))
                .join("building.parquet")
                .exists(),
            "missing partition count-{i:05}"
        );
    }
    let stdout = String::from_utf8_lossy(&o.stdout);
    assert!(
        stdout.contains("partitions=3"),
        "summary should report partitions=3, got: {stdout}"
    );
}

/// `--partition box` without `--cell-size` must fail with a clear error.
#[test]
fn partition_box_requires_cell_size() {
    let out = tempfile::tempdir().unwrap();
    let binary = env!("CARGO_BIN_EXE_cityparquet");
    let o = Command::new(binary)
        .arg("convert")
        .arg(fixture("delft.city.jsonl"))
        .arg("-o")
        .arg(out.path())
        .arg("--partition")
        .arg("box")
        .output()
        .expect("run");
    assert!(!o.status.success(), "box without --cell-size must fail");
    let stderr = String::from_utf8_lossy(&o.stderr);
    assert!(
        stderr.contains("cell-size"),
        "error should mention --cell-size, got: {stderr}"
    );
}

/// A non-finite `--cell-size` (e.g. `inf`) must be rejected, not silently
/// collapse every feature into one `box_x0_y0` partition.
#[test]
fn partition_box_rejects_nonfinite_cell_size() {
    let out = tempfile::tempdir().unwrap();
    let binary = env!("CARGO_BIN_EXE_cityparquet");
    let o = Command::new(binary)
        .arg("convert")
        .arg(fixture("delft.city.jsonl"))
        .arg("-o")
        .arg(out.path())
        .arg("--partition")
        .arg("box")
        .arg("--cell-size")
        .arg("inf")
        .output()
        .expect("run");
    assert!(!o.status.success(), "--cell-size inf must fail");
    let stderr = String::from_utf8_lossy(&o.stderr);
    assert!(
        stderr.contains("finite"),
        "error should mention finite, got: {stderr}"
    );
}

/// A sizing flag without `--partition` is a usage error.
#[test]
fn sizing_flag_without_partition_errors() {
    let out = tempfile::tempdir().unwrap();
    let binary = env!("CARGO_BIN_EXE_cityparquet");
    let o = Command::new(binary)
        .arg("convert")
        .arg(fixture("delft.city.jsonl"))
        .arg("-o")
        .arg(out.path())
        .arg("--number")
        .arg("3")
        .output()
        .expect("run");
    assert!(
        !o.status.success(),
        "--number without --partition must fail"
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
        .arg("-o")
        .arg(out.path())
        .status()
        .expect("failed to run first convert");
    assert!(status.success());

    // Second conversion without --overwrite should fail
    let output = Command::new(binary)
        .arg("convert")
        .arg(fixture("delft.city.jsonl"))
        .arg("-o")
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
        .arg("-o")
        .arg(out.path())
        .status()
        .expect("failed to run first convert");
    assert!(status.success());

    // Second conversion with --overwrite should succeed
    let status = Command::new(binary)
        .arg("convert")
        .arg(fixture("delft.city.jsonl"))
        .arg("-o")
        .arg(out.path())
        .arg("--overwrite")
        .status()
        .expect("failed to run second convert");

    assert!(status.success(), "convert with --overwrite should succeed");
    // delft is a single 1st-level family, so by-type conversion writes
    // exactly one main table: building.parquet.
    assert!(out.path().join("building.parquet").exists());
    assert!(out.path().join("metadata.json").exists());
}

/// M4 task 11 (Step 3): the `convert` report line gains 3 fields
/// (`materials_written textures_written templates_written`), appended after
/// the 6 fields it already printed. Exercised against a Compatibility
/// convert of railway, whose sidecar counts are pinned elsewhere (85/34/3 —
/// `railway_compatibility_convert_writes_materials_and_textures_sidecars` in
/// `crates/cityparquet/tests/convert_real_data.rs`).
#[test]
fn convert_compatibility_reports_sidecar_counts() {
    let out = tempfile::tempdir().unwrap();
    let binary = env!("CARGO_BIN_EXE_cityparquet");
    let output = Command::new(binary)
        .arg("convert")
        .arg(fixture("lod3_railway.city.json"))
        .arg("-o")
        .arg(out.path())
        .arg("--profile")
        .arg("compatibility")
        .output()
        .expect("failed to run convert");

    assert!(
        output.status.success(),
        "convert command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parts: Vec<&str> = stdout.split_whitespace().collect();
    assert_eq!(
        parts.len(),
        9,
        "convert should print 9 space-separated values (6 original + 3 sidecar counts), got: {}",
        stdout
    );
    assert_eq!(
        &parts[6..9],
        &["85", "34", "3"],
        "the 3 new trailing fields must be materials_written textures_written \
         templates_written in that order, got: {}",
        stdout
    );
}

#[test]
fn export_package_to_cityjsonl() {
    let package_dir = tempfile::tempdir().unwrap();
    let binary = env!("CARGO_BIN_EXE_cityparquet");

    // First, convert delft to a package
    let status = Command::new(binary)
        .arg("convert")
        .arg(fixture("delft.city.jsonl"))
        .arg("-o")
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
    // stdout should contain: feature_count, object_count,
    // instance_geometries_dropped, appearance_refs_dropped. (The former
    // 5th field, appearance_lod_misses, was removed with G20's per-LoD
    // appearance columns — the raw-vs-canonical LoD-key miss it counted can
    // no longer occur.)
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

    // Convert delft to package. `--no-lod0`: the CLI synthesises an LoD0
    // footprint by default (§9), but this test asserts a source-faithful round
    // trip, and synthesis is an additive enrichment.
    let status = Command::new(binary)
        .arg("convert")
        .arg("--no-lod0")
        .arg(fixture("delft.city.jsonl"))
        .arg("-o")
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

    // Convert railway to package (`--no-lod0` for a source-faithful round trip;
    // railway has no source LoD0, so synthesis would otherwise add one).
    let status = Command::new(binary)
        .arg("convert")
        .arg("--no-lod0")
        .arg(fixture("lod3_railway.city.json"))
        .arg("-o")
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

/// By-type is the sole, mandatory table layout (the CLI's layout flag was
/// removed 2026-07-21): a plain `convert` with no layout flag at all writes
/// delft's single pinned family table (delft contains Building/BuildingPart
/// objects → `building.parquet`). Per the CityJSON 2.0.1 1st-level/2nd-level
/// distinction, delft's `BuildingPart` rows (2nd-level) share
/// `building.parquet` with the `Building` rows (1st-level) rather than
/// getting their own `buildingpart.parquet`.
#[test]
fn convert_writes_by_type_family_tables_named_without_prefix() {
    let out = tempfile::tempdir().unwrap();
    let binary = env!("CARGO_BIN_EXE_cityparquet");
    let output = Command::new(binary)
        .arg("convert")
        .arg(fixture("delft.city.jsonl"))
        .arg("-o")
        .arg(out.path())
        .output()
        .expect("failed to run convert");

    assert!(
        output.status.success(),
        "convert failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(out.path().join("building.parquet").exists());
    assert!(
        !out.path().join("buildingpart.parquet").exists(),
        "BuildingPart is 2nd-level and must share building.parquet, not get its own file"
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("2231"),
        "stdout should contain object count"
    );
}

/// `--compression` overrides the recipe's default codec: converting the same
/// fixture with `gzip` vs `zstd` both succeed, both round-trip losslessly,
/// and — proving the codec actually took effect — the two runs produce
/// DIFFERENT `building.parquet` sizes (delft is a single 1st-level family,
/// so by-type conversion writes exactly one main table).
#[test]
fn convert_with_compression_override_changes_output_size_and_round_trips() {
    let binary = env!("CARGO_BIN_EXE_cityparquet");

    let convert_and_export = |codec: &str| -> u64 {
        let out = tempfile::tempdir().unwrap();
        let status = Command::new(binary)
            .arg("convert")
            .arg("--no-lod0") // source-faithful round trip (see the compare below)
            .arg(fixture("delft.city.jsonl"))
            .arg("-o")
            .arg(out.path())
            .arg("--compression")
            .arg(codec)
            .status()
            .expect("failed to run convert");
        assert!(status.success(), "convert --compression {codec} failed");

        let building = out.path().join("building.parquet");
        assert!(building.exists());
        let size = std::fs::metadata(&building).unwrap().len();

        let export_dir = tempfile::tempdir().unwrap();
        let export_path = export_dir.path().join("exported.city.jsonl");
        let status = Command::new(binary)
            .arg("export")
            .arg(out.path())
            .arg(&export_path)
            .status()
            .expect("failed to run export");
        assert!(
            status.success(),
            "export after --compression {codec} failed"
        );

        let compare = Command::new(binary)
            .arg("compare")
            .arg(fixture("delft.city.jsonl"))
            .arg(&export_path)
            .output()
            .expect("failed to run compare");
        assert!(
            compare.status.success(),
            "round trip after --compression {codec} was not equal: {}",
            String::from_utf8_lossy(&compare.stdout)
        );

        size
    };

    let gzip_size = convert_and_export("gzip");
    let zstd_size = convert_and_export("zstd");

    assert_ne!(
        gzip_size, zstd_size,
        "gzip and zstd should produce differently-sized building.parquet, both got {gzip_size} bytes"
    );
}

/// An unrecognised `--compression` value must fail with a clear error.
#[test]
fn convert_with_an_invalid_compression_fails() {
    let out = tempfile::tempdir().unwrap();
    let binary = env!("CARGO_BIN_EXE_cityparquet");
    let output = Command::new(binary)
        .arg("convert")
        .arg(fixture("delft.city.jsonl"))
        .arg("-o")
        .arg(out.path())
        .arg("--compression")
        .arg("bogus")
        .output()
        .expect("failed to run convert");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("invalid compression"),
        "expected an invalid-compression error, got: {stderr}"
    );
}

#[test]
fn export_package_to_gml_writes_citygml() {
    let tmp = tempfile::tempdir().unwrap();
    let pkg = tmp.path().join("pkg");
    let gml = tmp.path().join("model.gml");
    let binary = env!("CARGO_BIN_EXE_cityparquet");
    let ingolstadt = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../cityparquet/tests/data/savenow_ingolstadt_lod2.gml");
    assert!(ingolstadt.exists(), "missing Ingolstadt CityGML fixture");

    // .gml -> CityParquet package.
    let c = Command::new(binary)
        .arg("convert")
        .arg(&ingolstadt)
        .arg("-o")
        .arg(&pkg)
        .output()
        .expect("failed to run convert");
    assert!(
        c.status.success(),
        "convert failed: {}",
        String::from_utf8_lossy(&c.stderr)
    );

    // package -> .gml (the new export branch).
    let e = Command::new(binary)
        .arg("export")
        .arg(&pkg)
        .arg(&gml)
        .output()
        .expect("failed to run export");
    assert!(
        e.status.success(),
        "export to gml failed: {}",
        String::from_utf8_lossy(&e.stderr)
    );

    let text = std::fs::read_to_string(&gml).unwrap();
    assert!(text.contains("<CityModel"), "output must be a CityModel");
    assert!(
        text.contains("<bldg:Building"),
        "output must contain a Building"
    );

    let stdout = String::from_utf8_lossy(&e.stdout);
    assert_eq!(
        stdout.split_whitespace().next(),
        Some("3"),
        "report line should start with buildings_written=3, got {stdout:?}"
    );
}

/// The CLI synthesises an LoD0 footprint by default (§9): converting railway
/// (LoD3 solids, no source LoD0) and exporting yields a real `lod:"0.0"`
/// geometry (canonical spelling), and `--no-lod0` suppresses it.
#[test]
fn convert_synthesises_lod0_by_default_and_no_lod0_suppresses_it() {
    let binary = env!("CARGO_BIN_EXE_cityparquet");

    let export_lod0_present = |no_lod0: bool| -> bool {
        let pkg = tempfile::tempdir().unwrap();
        let mut cmd = Command::new(binary);
        cmd.arg("convert");
        if no_lod0 {
            cmd.arg("--no-lod0");
        }
        let status = cmd
            .arg(fixture("lod3_railway.city.json"))
            .arg("-o")
            .arg(pkg.path())
            .status()
            .expect("failed to run convert");
        assert!(status.success());

        let export_dir = tempfile::tempdir().unwrap();
        let export_path = export_dir.path().join("exported.city.jsonl");
        let status = Command::new(binary)
            .arg("export")
            .arg(pkg.path())
            .arg(&export_path)
            .status()
            .expect("failed to run export");
        assert!(status.success());
        std::fs::read_to_string(&export_path)
            .unwrap()
            .contains("\"lod\":\"0.0\"")
    };

    assert!(
        export_lod0_present(false),
        "default convert must synthesise LoD0"
    );
    assert!(
        !export_lod0_present(true),
        "--no-lod0 must suppress synthesis"
    );
}
