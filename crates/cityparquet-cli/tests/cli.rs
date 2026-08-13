use std::path::PathBuf;
use std::process::Command;

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

/// The real `lod3_railway.city.json` fixture carries no `referenceSystem` at
/// all. Since `scan` now hard-fails on coordinate-bearing input with no
/// resolvable CRS (spec "CRS rules"), tests below that convert (or compare
/// against) railway use a small on-disk COPY with a CRS injected via JSON
/// mutation of the real fixture — never hand-written CityJSON. Used both as
/// the conversion INPUT and, where a test also compares against "the
/// source", as that comparison baseline.
fn railway_fixture_with_crs() -> (tempfile::TempDir, PathBuf) {
    let mut doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(fixture("lod3_railway.city.json")).unwrap())
            .unwrap();
    doc["metadata"]["referenceSystem"] =
        serde_json::json!("https://www.opengis.net/def/crs/EPSG/0/7415");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("railway_with_crs.city.json");
    std::fs::write(&path, serde_json::to_string(&doc).unwrap()).unwrap();
    (dir, path)
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
/// the 6 fields it already printed. Exercised against a convert of railway,
/// which carries real materials/textures/templates so they are written
/// unconditionally (spec-alignment gap 19 dropped the `--profile` flag this
/// test used to pass), whose sidecar counts are pinned elsewhere (85/34/3 —
/// `railway_compatibility_convert_writes_materials_and_textures_sidecars` in
/// `crates/cityparquet/tests/convert_real_data.rs`).
#[test]
fn convert_compatibility_reports_sidecar_counts() {
    let out = tempfile::tempdir().unwrap();
    let (_crs_dir, railway_path) = railway_fixture_with_crs();
    let binary = env!("CARGO_BIN_EXE_cityparquet");
    let output = Command::new(binary)
        .arg("convert")
        .arg(railway_path)
        .arg("-o")
        .arg(out.path())
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
    let (_crs_dir, railway_path) = railway_fixture_with_crs();
    let binary = env!("CARGO_BIN_EXE_cityparquet");

    // Convert railway to package (`--no-lod0` for a source-faithful round trip;
    // railway has no source LoD0, so synthesis would otherwise add one).
    let status = Command::new(binary)
        .arg("convert")
        .arg("--no-lod0")
        .arg(&railway_path)
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
        .arg(&railway_path)
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
    let (_crs_dir, railway_path) = railway_fixture_with_crs();

    let export_lod0_present = |no_lod0: bool| -> bool {
        let pkg = tempfile::tempdir().unwrap();
        let mut cmd = Command::new(binary);
        cmd.arg("convert");
        if no_lod0 {
            cmd.arg("--no-lod0");
        }
        let status = cmd
            .arg(&railway_path)
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

/// A real CityJSON fixture copied with its `referenceSystem` removed — the
/// shape of the catalogue collections whose CityJSON carries no CRS at all.
/// Written under `dir` with the given `name` so two DISTINCT such inputs can
/// be merged in one run.
fn crs_less_delft(dir: &std::path::Path, name: &str) -> PathBuf {
    let text = std::fs::read_to_string(fixture("delft.city.jsonl")).unwrap();
    let mut lines = text.lines();
    let mut header: serde_json::Value = serde_json::from_str(lines.next().unwrap()).unwrap();
    header
        .get_mut("metadata")
        .and_then(|m| m.as_object_mut())
        .expect("delft header has metadata")
        .remove("referenceSystem");
    let mut out = serde_json::to_string(&header).unwrap();
    for line in lines {
        out.push('\n');
        out.push_str(line);
    }
    let dest = dir.join(name);
    std::fs::write(&dest, out).unwrap();
    dest
}

/// The footer `city` object of one table in a written package.
fn city_footer(table: &std::path::Path) -> cityparquet_schema::CityMetadata {
    use cityparquet::reader::CityParquetReaderBuilder;
    use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
    let builder =
        ParquetRecordBatchReaderBuilder::try_new(std::fs::File::open(table).unwrap()).unwrap();
    builder.cityparquet_metadata().expect("footer must parse")
}

/// End-to-end at the binary: `--crs` is what unblocks a CRS-less source, and
/// the package it writes says so. The applied-ness → stamp linkage is CLI-side
/// wiring (open, apply, merge), so it is proven here against the real binary,
/// not only at the library level.
#[test]
fn convert_with_crs_flag_supplies_the_crs_and_stamps_its_provenance() {
    let binary = env!("CARGO_BIN_EXE_cityparquet");
    let dir = tempfile::tempdir().unwrap();
    let input = crs_less_delft(dir.path(), "no_crs.city.jsonl");
    let out = dir.path().join("pkg");

    // Without the flag the CRS-less source is still a hard error.
    let output = Command::new(binary)
        .args(["convert".as_ref(), input.as_os_str()])
        .arg("-o")
        .arg(&out)
        .output()
        .expect("failed to run convert");
    assert!(
        !output.status.success(),
        "a CRS-less source must still fail without --crs"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("declares no CRS"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // With it, the conversion succeeds and records where the CRS came from.
    let output = Command::new(binary)
        .args(["convert".as_ref(), input.as_os_str()])
        .arg("-o")
        .arg(&out)
        .args(["--overwrite", "--crs", "EPSG:7415"])
        .output()
        .expect("failed to run convert");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let meta = city_footer(&out.join("building.parquet"));
    assert!(meta.crs.is_some(), "city.crs must be populated");
    let other = meta.other.expect("city.other must exist");
    assert_eq!(
        other.get("crs_source").and_then(|v| v.as_str()),
        Some("operator-supplied"),
        "footer: {other}"
    );
    assert!(
        other
            .get("source_metadata")
            .and_then(|m| m.get("referenceSystem"))
            .is_none(),
        "the operator's CRS must not leak into the verbatim source metadata: {other}"
    );
}

/// `--crs` is ignored for a source that declares its own CRS, so the footer
/// must keep that source's CRS and claim nothing about an operator.
#[test]
fn convert_with_crs_flag_on_a_source_that_declares_one_stamps_nothing() {
    let binary = env!("CARGO_BIN_EXE_cityparquet");
    let out = tempfile::tempdir().unwrap();
    let output = Command::new(binary)
        .arg("convert")
        .arg(fixture("delft.city.jsonl"))
        .arg("-o")
        .arg(out.path())
        .args(["--overwrite", "--crs", "EPSG:28992"])
        .output()
        .expect("failed to run convert");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let meta = city_footer(&out.path().join("building.parquet"));
    assert_eq!(
        meta.crs.as_ref().and_then(|c| c.pointer("/id/code")),
        Some(&serde_json::json!(7415)),
        "delft's own CRS must be the one written"
    );
    assert!(
        meta.other
            .as_ref()
            .and_then(|o| o.get("crs_source"))
            .is_none(),
        "a source-declared CRS must never be stamped operator-supplied: {:?}",
        meta.other
    );
}

/// Several inputs are merged into ONE in-memory source before conversion, so
/// the provenance has to survive that rebuild — otherwise a merged run writes
/// the operator's CRS with no record of where it came from.
#[test]
fn convert_with_crs_flag_keeps_the_provenance_across_a_merge() {
    let binary = env!("CARGO_BIN_EXE_cityparquet");
    let dir = tempfile::tempdir().unwrap();
    let a = crs_less_delft(dir.path(), "a.city.jsonl");
    let b = crs_less_delft(dir.path(), "b.city.jsonl");
    let out = dir.path().join("pkg");
    let output = Command::new(binary)
        .args(["convert".as_ref(), a.as_os_str(), b.as_os_str()])
        .arg("-o")
        .arg(&out)
        .args(["--overwrite", "--crs", "EPSG:7415"])
        .output()
        .expect("failed to run convert");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let other = city_footer(&out.join("building.parquet"))
        .other
        .expect("city.other must exist");
    assert_eq!(
        other.get("crs_source").and_then(|v| v.as_str()),
        Some("operator-supplied"),
        "the merged package lost the CRS provenance: {other}"
    );
}

/// Several inputs, only SOME of which declare their own CRS. `merge_sources`
/// enforces one shared CRS, so if ANY input declared one the merged CRS IS
/// source-declared — which makes `.all()`, not `.any()`, the question to ask.
/// Under `.any()` one CRS-less input in a batch stamped the WHOLE package
/// `crs_source: "operator-supplied"` and stripped the genuine
/// `referenceSystem` out of the "verbatim" `source_metadata`: a footer
/// implying the source did not declare a CRS it did carry, which is the exact
/// inverse of the guarantee the stamp exists to give.
#[test]
fn a_mixed_batch_is_not_stamped_operator_supplied() {
    let binary = env!("CARGO_BIN_EXE_cityparquet");
    let dir = tempfile::tempdir().unwrap();
    // One input declaring EPSG:7415 (the real fixture) beside one declaring
    // nothing, and a `--crs` that supplies the same code to the latter so the
    // merge's shared-CRS check passes.
    let declared = fixture("delft.city.jsonl");
    let silent = crs_less_delft(dir.path(), "no_crs.city.jsonl");
    let out = dir.path().join("pkg");
    let output = Command::new(binary)
        .args(["convert".as_ref(), declared.as_os_str(), silent.as_os_str()])
        .arg("-o")
        .arg(&out)
        .args(["--overwrite", "--crs", "EPSG:7415"])
        .output()
        .expect("failed to run convert");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let other = city_footer(&out.join("building.parquet"))
        .other
        .expect("city.other must exist");
    assert!(
        other.get("crs_source").is_none(),
        "an input declared this CRS itself, so nothing may claim an operator supplied it: {other}"
    );
    assert!(
        other
            .get("source_metadata")
            .and_then(|m| m.get("referenceSystem"))
            .is_some(),
        "the source's own referenceSystem must survive in the verbatim passthrough: {other}"
    );
}

/// `--crs` used to be swallowed whenever it happened to be ignored: the CLI
/// set `crs_override` only when the declaration was actually applied, so on a
/// source that declares its own CRS an unusable value exited 0 with no
/// message at all. The value is validated because it was given, not because it
/// took effect — the provenance stamp reads the SOURCE, never the option, so
/// validating unconditionally cannot produce a false stamp.
#[test]
fn an_unusable_crs_is_reported_even_when_the_source_declares_its_own() {
    let binary = env!("CARGO_BIN_EXE_cityparquet");
    for (spec, needle) in [
        ("banana", "EPSG"),
        ("EPSG:4326", "geographic"),
        ("", "EPSG"),
    ] {
        let out = tempfile::tempdir().unwrap();
        let output = Command::new(binary)
            .arg("convert")
            .arg(fixture("delft.city.jsonl"))
            .arg("-o")
            .arg(out.path())
            .args(["--overwrite", "--crs", spec])
            .output()
            .expect("failed to run convert");
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        assert!(
            !output.status.success(),
            "--crs {spec:?} must not be swallowed; stdout: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert!(stderr.contains(needle), "--crs {spec:?}: stderr: {stderr}");
    }
}
