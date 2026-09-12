mod common;

use citylake::core::interface::types::{DatasetName, ExportFormat, ModuleName};

#[test]
fn a_dataset_writes_out_as_a_package_directory() {
    let (service, dir) = common::test_service();
    let name = DatasetName::new("delft").unwrap();
    service
        .create_dataset_impl(&name, common::fixture("delft.city.jsonl").to_str().unwrap())
        .unwrap();

    let out = dir.path().join("pkg");
    let written = service
        .write_package_impl(&name, out.to_str().unwrap())
        .expect("write the package");

    // One data file per non-empty object table, plus the STAC Item.
    assert!(written.iter().any(|f| f.file == "building.parquet"));
    assert!(written.iter().any(|f| f.file == "metadata.json"));
    assert!(out.join("building.parquet").exists());
    assert!(out.join("metadata.json").exists());
}

#[test]
fn a_written_package_carries_the_datasets_crs() {
    let (service, dir) = common::test_service();
    let name = DatasetName::new("delft").unwrap();
    service
        .create_dataset_impl(&name, common::fixture("delft.city.jsonl").to_str().unwrap())
        .unwrap();
    let out = dir.path().join("pkg");
    service
        .write_package_impl(&name, out.to_str().unwrap())
        .unwrap();

    // The footer minted at creation is what lets the writer state a CRS
    // instead of an explicit null.
    let reimported = DatasetName::new("reimported").unwrap();
    let info = service
        .create_dataset_impl(&reimported, out.to_str().unwrap())
        .expect("load the written package back");
    assert!(info.crs.expect("a CRS").contains("7415"));
}

#[test]
fn a_package_round_trips_through_the_lake() {
    let (service, dir) = common::test_service();
    let name = DatasetName::new("delft").unwrap();
    service
        .create_dataset_impl(&name, common::fixture("delft.city.jsonl").to_str().unwrap())
        .unwrap();
    let original: usize = service
        .describe_dataset_impl(&name)
        .unwrap()
        .modules
        .iter()
        .map(|m| m.rows)
        .sum();

    let out = dir.path().join("roundtrip");
    service
        .write_package_impl(&name, out.to_str().unwrap())
        .unwrap();

    let loaded = DatasetName::new("loaded").unwrap();
    let info = service
        .create_dataset_impl(&loaded, out.to_str().unwrap())
        .unwrap();
    assert_eq!(info.modules.iter().map(|m| m.rows).sum::<usize>(), original);
}

#[test]
fn a_module_exports_to_a_cityjsonseq_file() {
    let (service, dir) = common::test_service();
    let name = DatasetName::new("delft").unwrap();
    service
        .create_dataset_impl(&name, common::fixture("delft.city.jsonl").to_str().unwrap())
        .unwrap();

    let out = dir.path().join("delft_out.city.jsonl");
    service
        .export_module_impl(
            &name,
            &ModuleName::new("building").unwrap(),
            out.to_str().unwrap(),
            ExportFormat::CityJsonSeq,
        )
        .expect("export the module");
    assert!(out.exists());
    assert!(std::fs::metadata(&out).unwrap().len() > 0);

    // A CityJSONSeq file leads with the metadata object.
    let written = std::fs::read_to_string(&out).unwrap();
    let header: serde_json::Value =
        serde_json::from_str(written.lines().next().expect("a first line")).unwrap();

    // CityJSON defines referenceSystem as a URI. The COPY option is written
    // through verbatim, so an export that hands it the footer's PROJJSON
    // produces a file declaring a JSON document where a URI belongs —
    // well-formed, non-empty, and wrong. Assert the URI itself: a length
    // check cannot tell the two apart.
    assert_eq!(
        header["metadata"]["referenceSystem"],
        serde_json::json!("https://www.opengis.net/def/crs/EPSG/0/7415")
    );
}

#[test]
fn merging_folds_one_dataset_into_another() {
    let (service, _dir) = common::test_service();
    let destination = DatasetName::new("dst").unwrap();
    let source = DatasetName::new("src").unwrap();
    service
        .create_dataset_impl(
            &destination,
            common::fixture("delft.city.jsonl").to_str().unwrap(),
        )
        .unwrap();
    // Both sides are footers once created, and the merge applies the same CRS
    // rule as an insert — so the source must declare the destination's CRS.
    service
        .create_dataset_impl(
            &source,
            common::fixture("minimal_7415.city.json").to_str().unwrap(),
        )
        .unwrap();

    let before: usize = service
        .describe_dataset_impl(&destination)
        .unwrap()
        .modules
        .iter()
        .map(|m| m.rows)
        .sum();

    service.merge_impl(&destination, &source).expect("merge");

    let after: usize = service
        .describe_dataset_impl(&destination)
        .unwrap()
        .modules
        .iter()
        .map(|m| m.rows)
        .sum();
    assert!(after > before);
}
