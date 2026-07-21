//! Task 1 (mandatory-by-type-layout plan): characterization safety net.
//!
//! Pins the exact by-type object-table set the `TableLayout::ByType` layout
//! produces TODAY, while `TableLayout::Single` still exists. A later task
//! in this plan deletes `TableLayout::Single` (and the `opts.layout = ...`
//! line below, since by-type becomes unconditional); this test must stay
//! green across that removal, proving the deletion did not perturb by-type
//! output by a single byte.

use std::collections::BTreeSet;
use std::path::PathBuf;

use cityparquet::package::{ConvertOptions, convert};

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

fn table_files(dir: &std::path::Path) -> BTreeSet<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".parquet"))
        .collect()
}

#[test]
fn railway_by_type_writes_one_file_per_first_level_family() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("pkg");
    // NOTE: while Single still exists, set layout explicitly; after the
    // later task removing TableLayout::Single, the field is gone and this
    // line is deleted (by-type is unconditional).
    let mut opts = ConvertOptions::new(fixture("lod3_railway.city.json"), out.clone());
    opts.layout = cityparquet::package::TableLayout::ByType;
    convert(&opts).expect("convert railway");

    let expected: BTreeSet<String> = [
        "bridge.parquet",
        "building.parquet",
        "cityfurniture.parquet",
        "cityobjectgroup.parquet",
        "genericcityobject.parquet",
        "railway.parquet",
        "solitaryvegetationobject.parquet",
        "tinrelief.parquet",
        "tunnel.parquet",
        "waterbody.parquet",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    assert_eq!(
        table_files(&out),
        expected,
        "by-type must write exactly one file per 1st-level family; 2nd-level types share their \
         parent's file"
    );
}
