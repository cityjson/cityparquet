//! A CityGML 3.0 document survives CityGML → package → CityJSONSeq.
//!
//! The files are the `citygml3` crate's: a PLATEAU building tile upgraded to
//! CityGML 3.0 by `citygml-tools upgrade`, and the OGC Test ADE's sample,
//! whose `IndustrialBuilding` specialises a building. Each converts to a
//! conformant package whose export compares equal with its source.

use std::path::{Path, PathBuf};

use cityparquet::compare::{CompareOptions, compare_datasets};
use cityparquet::export::{ExportOptions, export};
use cityparquet::package::{ConvertOptions, convert};
use cityparquet::validate::validate_package;

fn citygml3_data(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../citygml-rs/crates/citygml3/tests/data")
        .join(path)
}

/// Converts `source` into `dir/pkg`, checks the package is conformant,
/// exports it and checks the export equals `source`; returns the package.
fn round_trip(source: &Path, dir: &Path) -> PathBuf {
    let pkg = dir.join("pkg");
    convert(&ConvertOptions::new(source.to_path_buf(), pkg.clone())).unwrap();
    let report = validate_package(&pkg).unwrap();
    assert!(report.is_conformant(), "{report}");
    let out = dir.join("out.city.jsonl");
    export(&ExportOptions::new(pkg.clone(), out.clone())).unwrap();
    let cmp = compare_datasets(source, &out, &CompareOptions::default()).unwrap();
    assert!(cmp.equal, "{:#?}", cmp.differences);
    pkg
}

#[test]
fn a_citygml_3_building_tile_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let pkg = round_trip(
        &citygml3_data("plateau/53394519_bldg_6697_op.gml"),
        dir.path(),
    );
    assert!(pkg.join("building.parquet").exists());
}

/// The ADE's class substitutes for a building, so it goes to the building
/// table, and the export declares the ADE as an extension again.
#[test]
fn a_citygml_3_ade_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let pkg = round_trip(
        &citygml3_data("ogc/test-ade/industrial_building_dataset_v3.gml"),
        dir.path(),
    );
    assert!(pkg.join("building.parquet").exists());
}
