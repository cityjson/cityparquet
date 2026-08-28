mod common;

use citylake::core::interface::types::DatasetName;

#[test]
fn creating_a_dataset_routes_objects_to_module_tables() {
    let (service, _dir) = common::test_service();
    let name = DatasetName::new("delft").unwrap();

    let info = service
        .create_dataset_impl(&name, common::fixture("delft.city.jsonl").to_str().unwrap())
        .expect("create the dataset");

    // The fixture holds Buildings, so the building module table exists and
    // carries them. Routing is the extension's, not ours.
    let building = info
        .modules
        .iter()
        .find(|m| m.name == "building")
        .expect("a building module table");
    assert!(building.rows > 0, "buildings were not ingested");
    assert_eq!(building.role, "object");
}

#[test]
fn a_created_dataset_declares_its_crs() {
    let (service, _dir) = common::test_service();
    let name = DatasetName::new("delft").unwrap();

    let info = service
        .create_dataset_impl(&name, common::fixture("delft.city.jsonl").to_str().unwrap())
        .unwrap();

    // The fixture declares EPSG:7415. The footer is minted by the extension,
    // so what comes back is canonical PROJJSON, not the source's spelling.
    let crs = info.crs.expect("the dataset should declare a CRS");
    assert!(crs.contains("7415"), "unexpected CRS: {crs}");
}

#[test]
fn the_declared_crs_arms_the_guard_against_a_mismatched_source() {
    let (service, _dir) = common::test_service();
    let name = DatasetName::new("delft").unwrap();
    service
        .create_dataset_impl(&name, common::fixture("delft.city.jsonl").to_str().unwrap())
        .unwrap();

    // This is the point of minting the footer at all: without it the package
    // states nothing, and a differently-projected source would be accepted.
    let source = common::fixture("bench_28992.city.json");
    let err = service
        .ingest_impl(&name, source.to_str().unwrap())
        .expect_err("a 28992 source must not enter a 7415 package");
    assert!(
        format!("{err}").contains("CRS mismatch"),
        "expected a CRS mismatch, got: {err}"
    );
}

#[test]
fn a_source_without_a_crs_still_creates_a_dataset() {
    let (service, _dir) = common::test_service();
    let name = DatasetName::new("nocrs").unwrap();

    // Nothing to mint is not a failure: a package that states no CRS is the
    // correct "unknown" state, and the extension treats it as one.
    let info = service
        .create_dataset_impl(
            &name,
            common::fixture("minimal_nocrs.city.json").to_str().unwrap(),
        )
        .expect("a source without a referenceSystem is still ingestable");
    assert!(info.modules.iter().any(|m| m.rows > 0));
}

#[test]
fn minting_the_footer_does_not_leave_the_ingest_uncommitted() {
    let (service, _dir) = common::test_service();
    let name = DatasetName::new("delft").unwrap();

    // The regression this pins: cityparquet_write sees committed state only,
    // and a transaction that has written to `lake` may not also write to
    // `memory`. Minting inside the ingest transaction fails on both counts —
    // so if this passes, the phases are correctly separated.
    let info = service
        .create_dataset_impl(&name, common::fixture("delft.city.jsonl").to_str().unwrap())
        .expect("create must survive the probe");
    assert!(info.crs.is_some(), "the footer was not minted");
    assert!(
        info.modules.iter().map(|m| m.rows).sum::<usize>() > 0,
        "the ingest was lost"
    );
}

#[test]
fn creating_a_dataset_twice_is_refused() {
    let (service, _dir) = common::test_service();
    let name = DatasetName::new("delft").unwrap();
    let source = common::fixture("delft.city.jsonl");
    service
        .create_dataset_impl(&name, source.to_str().unwrap())
        .unwrap();

    let err = service
        .create_dataset_impl(&name, source.to_str().unwrap())
        .expect_err("the second create must be refused");
    assert!(format!("{err}").contains("delft"));
}

#[test]
fn a_failed_create_leaves_no_half_built_schema() {
    let (service, _dir) = common::test_service();
    let name = DatasetName::new("broken").unwrap();

    let failed = service.create_dataset_impl(&name, "/nonexistent/source.city.jsonl");
    assert!(failed.is_err());

    // A dataset that failed to ingest must not be left addressable — the next
    // create would then fail as a duplicate.
    assert!(!service
        .list_datasets_impl()
        .unwrap()
        .contains(&"broken".to_string()));
}

#[test]
fn datasets_can_be_listed_described_and_dropped() {
    let (service, _dir) = common::test_service();
    let name = DatasetName::new("delft").unwrap();
    service
        .create_dataset_impl(&name, common::fixture("delft.city.jsonl").to_str().unwrap())
        .unwrap();

    assert!(service
        .list_datasets_impl()
        .unwrap()
        .contains(&"delft".to_string()));
    assert_eq!(service.describe_dataset_impl(&name).unwrap().name, "delft");

    service.drop_dataset_impl(&name).unwrap();
    assert!(!service
        .list_datasets_impl()
        .unwrap()
        .contains(&"delft".to_string()));
}
