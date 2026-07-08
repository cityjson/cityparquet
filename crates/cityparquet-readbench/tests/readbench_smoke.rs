//! RED (readbench Task 8): end-to-end smoke test for the `--child`
//! protocol — converts a real fixture to a CityParquet package in a
//! tempdir, then invokes the BUILT `cityparquet-readbench` binary (never
//! calling into its internals directly) exactly as the Task 11 coordinator
//! eventually will, asserting the printed child-protocol line has the
//! right shape and the right `count` scenario result.

use std::path::PathBuf;
use std::process::Command;

use cityparquet::package::{ConvertOptions, convert};

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

#[test]
fn child_count_scenario_prints_a_four_field_line_with_the_known_object_count() {
    let out = tempfile::tempdir().unwrap();
    let opts = ConvertOptions::new(fixture("lod3_railway.city.json"), out.path().to_path_buf());
    let report = convert(&opts).unwrap();
    assert_eq!(report.object_count, 121);

    let output = Command::new(env!("CARGO_BIN_EXE_cityparquet-readbench"))
        .args([
            "--child",
            "--format",
            "cityparquet",
            "--scenario",
            "count",
            "--input",
        ])
        .arg(out.path())
        .output()
        .expect("failed to run the built cityparquet-readbench binary");

    assert!(
        output.status.success(),
        "child process exited non-zero; stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout must be valid UTF-8");
    let line = stdout.trim();
    let fields: Vec<&str> = line.split_whitespace().collect();
    assert_eq!(
        fields.len(),
        4,
        "expected exactly 4 whitespace-separated fields in '{line}'"
    );

    let time_s: f64 = fields[0]
        .parse()
        .unwrap_or_else(|e| panic!("field 1 (time_s) '{}' did not parse as f64: {e}", fields[0]));
    assert!(time_s >= 0.0, "time_s must be non-negative, got {time_s}");

    let _peak_heap_bytes: u64 = fields[1].parse().unwrap_or_else(|e| {
        panic!(
            "field 2 (peak_heap_bytes) '{}' did not parse as u64: {e}",
            fields[1]
        )
    });
    let _ru_maxrss_bytes: u64 = fields[2].parse().unwrap_or_else(|e| {
        panic!(
            "field 3 (ru_maxrss_bytes) '{}' did not parse as u64: {e}",
            fields[2]
        )
    });

    let result_count: u64 = fields[3].parse().unwrap_or_else(|e| {
        panic!(
            "field 4 (result_count) '{}' did not parse as u64: {e}",
            fields[3]
        )
    });
    assert_eq!(
        result_count, 121,
        "CityParquet's `count` scenario counts one row per CityObject (parents \
         and children); lod3_railway.city.json has 121"
    );
}
