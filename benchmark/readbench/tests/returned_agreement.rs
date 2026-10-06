//! The text formats and CityParquet return the same thing under the return
//! rule (READ_BENCHMARK.md, "The six scenarios"): the same identifier set
//! for the spatial windows and the attribute filter, the same comparable
//! totals for read all and the identifier lookups, and the same
//! highest-LoD geometry for the windows.
//!
//! Runs the real coordinator over `cityjson`, `cityjsonseq` and
//! `cityparquet` on real data: `delft.city.jsonl` and the 40-feature Tokyo
//! cut (latitude-first, so the extents cross the axis swap). The
//! coordinator's consistency check exits non-zero on any disagreement, and
//! every part must be reported by all three formats, never skipped.
//! CityGML is not covered here: its artefact is synthesised by
//! citygml-tools, which the test suite does not run.

use std::path::{Path, PathBuf};
use std::process::Command;

use cityparquet::cjseq::{CityJSON, CityJSONFeature, cjseq_to_cj};
use cityparquet::package::{ConvertOptions, convert};

fn lib_fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../lib/cityparquet-rs/tests/fixtures")
        .join(name)
}

fn local_fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// Prepares the three artefacts the way `readbench_prepare.sh` does for a
/// `.city.jsonl` input: the Seq copied, the document collected by `cjseq`,
/// the package converted without LoD 0 synthesis.
fn prepare(input: &Path, base: &str, prepared: &Path) {
    std::fs::copy(input, prepared.join(format!("{base}.city.jsonl"))).unwrap();
    let text = std::fs::read_to_string(input).unwrap();
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let header = CityJSON::from_str(lines.next().unwrap()).unwrap();
    let features: Vec<CityJSONFeature> = lines
        .map(|line| CityJSONFeature::from_str(line).unwrap())
        .collect();
    let doc = cjseq_to_cj(header, features);
    std::fs::write(
        prepared.join(format!("{base}.city.json")),
        serde_json::to_string(&doc).unwrap(),
    )
    .unwrap();
    let opts = ConvertOptions::new(
        input.to_path_buf(),
        prepared.join(format!("{base}.parquet")),
    );
    assert!(!opts.generate_lod0);
    convert(&opts).unwrap();
}

fn assert_agreement(input: &Path, base: &str) {
    let prepared = tempfile::tempdir().unwrap();
    prepare(input, base, prepared.path());
    let out_csv = prepared.path().join("out.csv");
    let output = Command::new(env!("CARGO_BIN_EXE_cityparquet-readbench"))
        .arg("run")
        .args([
            "--input",
            input.to_str().unwrap(),
            "--prepared-dir",
            prepared.path().to_str().unwrap(),
            "--out",
            out_csv.to_str().unwrap(),
            "--repeat",
            "1",
            "--scenarios",
            "full-read,bbox,attr-filter,id-lookup",
            "--formats",
            "cityjson,cityjsonseq,cityparquet",
        ])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "{base}: the coordinator rejected the returned results:\n{stderr}"
    );
    assert!(
        !stderr.contains("not compared for"),
        "{base}: a format did not report a returned part:\n{stderr}"
    );
}

#[test]
fn delft_returns_the_same_ids_totals_and_geometry_in_every_text_format_and_cityparquet() {
    assert_agreement(&lib_fixture("delft.city.jsonl"), "delft");
}

#[test]
fn tokyo_returns_the_same_ids_totals_and_geometry_in_every_text_format_and_cityparquet() {
    assert_agreement(
        &local_fixture("tokyo_chiyoda_40.city.jsonl"),
        "tokyo_chiyoda_40",
    );
}
