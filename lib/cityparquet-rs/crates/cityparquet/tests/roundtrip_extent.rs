//! `geographicalExtent` is reconstructed from `bbox` on export.

use std::path::PathBuf;

use cityparquet::export::{ExportOptions, export};
use cityparquet::package::{ConvertOptions, convert};

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

#[test]
fn export_emits_geographical_extent_from_bbox() {
    let package_dir = tempfile::tempdir().unwrap();
    convert(&ConvertOptions::new(
        fixture("delft.city.jsonl"),
        package_dir.path().to_path_buf(),
    ))
    .unwrap();

    let export_dir = tempfile::tempdir().unwrap();
    let output = export_dir.path().join("export.city.jsonl");
    export(&ExportOptions {
        package_dir: package_dir.path().to_path_buf(),
        output: output.clone(),
    })
    .unwrap();

    let text = std::fs::read_to_string(&output).unwrap();
    let mut checked = 0;
    for line in text.lines() {
        let feature: serde_json::Value = serde_json::from_str(line).unwrap();
        for (id, obj) in feature["CityObjects"].as_object().unwrap() {
            let extent = &obj["geographicalExtent"];
            assert!(
                extent.is_array() && extent.as_array().unwrap().len() == 6,
                "object {id} must carry a six-number geographicalExtent, got {extent}"
            );
            checked += 1;
        }
        if checked > 50 {
            break;
        }
    }
    assert!(checked > 0, "exported at least one object");
}
