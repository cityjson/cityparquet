//! The write child: `--child --write --variant <id> --input <seq> --out <dir>`
//! converts one input with one variant's recipe, prints the four-field child
//! protocol line, and leaves the package at `--out`. The coordinator's
//! `--variants` path is built on exactly this.

use std::path::PathBuf;
use std::process::Command;

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../lib/cityparquet-rs/tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

fn row_groups_in(package: &std::path::Path) -> usize {
    use parquet::file::reader::{FileReader, SerializedFileReader};
    let table = std::fs::read_dir(package)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|x| x == "parquet"))
        .expect("the package holds a .parquet table");
    let reader = SerializedFileReader::new(std::fs::File::open(table).unwrap()).unwrap();
    reader.metadata().num_row_groups()
}

#[test]
fn a_write_child_prints_the_protocol_line_and_applies_the_variants_recipe() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("pkg");
    let output = Command::new(env!("CARGO_BIN_EXE_cityparquet-readbench"))
        .args([
            "--child",
            "--write",
            "--variant",
            "cityparquet+rg512",
            "--input",
            fixture("delft.city.jsonl").to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).unwrap();
    let fields: Vec<&str> = stdout.split_whitespace().collect();
    assert_eq!(fields.len(), 4, "four fields, got: {stdout:?}");
    let time_s: f64 = fields[0].parse().unwrap();
    let peak_heap: u64 = fields[1].parse().unwrap();
    let peak_rss: u64 = fields[2].parse().unwrap();
    let objects: u64 = fields[3].parse().unwrap();
    assert!(time_s > 0.0);
    assert!(peak_heap > 0);
    assert!(peak_rss > 0);
    assert_eq!(objects, 2231, "delft has 2231 CityObjects");

    assert!(
        out.join("metadata.json").is_file(),
        "a package was left at --out"
    );
    // 2231 rows / 512 per group = 5 groups: the rg512 recipe reached the writer.
    assert_eq!(row_groups_in(&out), 5);
}

#[test]
fn a_write_child_rejects_a_bad_variant_with_the_grammar_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("pkg");
    let output = Command::new(env!("CARGO_BIN_EXE_cityparquet-readbench"))
        .args([
            "--child",
            "--write",
            "--variant",
            "cityparquet+gzip6",
            "--input",
            fixture("delft.city.jsonl").to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("cityparquet+gzip6"), "{stderr}");
    assert!(
        stderr.contains("<preset>[+hilbert][+rg<N>][+<codec>[<level>]]"),
        "{stderr}"
    );
    assert!(!out.exists(), "a rejected write must leave nothing behind");
}

#[test]
fn a_write_child_needs_all_three_of_variant_input_and_out() {
    for missing in ["--variant", "--input", "--out"] {
        let dir = tempfile::tempdir().unwrap();
        let mut args = vec!["--child", "--write"];
        let input = fixture("delft.city.jsonl");
        let out = dir.path().join("pkg");
        for (flag, value) in [
            ("--variant", "cityparquet"),
            ("--input", input.to_str().unwrap()),
            ("--out", out.to_str().unwrap()),
        ] {
            if flag != missing {
                args.push(flag);
                args.push(value);
            }
        }
        let output = Command::new(env!("CARGO_BIN_EXE_cityparquet-readbench"))
            .args(&args)
            .output()
            .unwrap();
        assert!(!output.status.success(), "{missing} omitted must fail");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains(missing),
            "the message names the missing flag: {stderr}"
        );
    }
}
