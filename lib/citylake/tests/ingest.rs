mod common;

use citylake::core::interface::types::DatasetName;

#[test]
fn ingesting_a_second_source_adds_its_objects() {
    let (service, _dir) = common::test_service();
    let name = DatasetName::new("delft").unwrap();
    service
        .create_dataset_impl(&name, common::fixture("delft.city.jsonl").to_str().unwrap())
        .unwrap();

    let before: usize = service
        .describe_dataset_impl(&name)
        .unwrap()
        .modules
        .iter()
        .map(|m| m.rows)
        .sum();

    // The variant declaring EPSG:7415: a package states one CRS for every row,
    // so a source declaring none would be refused here — correctly, but that is
    // the mismatch test's job, not this one's.
    let added = service
        .ingest_impl(
            &name,
            common::fixture("minimal_7415.city.json").to_str().unwrap(),
        )
        .expect("ingest a second source");
    assert!(added > 0);

    let after: usize = service
        .describe_dataset_impl(&name)
        .unwrap()
        .modules
        .iter()
        .map(|m| m.rows)
        .sum();
    assert_eq!(after, before + added);
}

#[test]
fn ingesting_the_same_source_twice_is_refused() {
    let (service, _dir) = common::test_service();
    let name = DatasetName::new("delft").unwrap();
    let source = common::fixture("delft.city.jsonl");
    service
        .create_dataset_impl(&name, source.to_str().unwrap())
        .unwrap();

    // Ids are identity: an incoming id already present refuses the whole
    // insert rather than renaming silently.
    let err = service
        .ingest_impl(&name, source.to_str().unwrap())
        .expect_err("duplicate ids must refuse the insert");
    assert!(format!("{err}").contains("duplicate id"), "got: {err}");
}

#[test]
fn a_refused_ingest_leaves_the_dataset_untouched() {
    let (service, _dir) = common::test_service();
    let name = DatasetName::new("delft").unwrap();
    let source = common::fixture("delft.city.jsonl");
    service
        .create_dataset_impl(&name, source.to_str().unwrap())
        .unwrap();
    let before: usize = service
        .describe_dataset_impl(&name)
        .unwrap()
        .modules
        .iter()
        .map(|m| m.rows)
        .sum();

    let _ = service.ingest_impl(&name, source.to_str().unwrap());

    let after: usize = service
        .describe_dataset_impl(&name)
        .unwrap()
        .modules
        .iter()
        .map(|m| m.rows)
        .sum();
    assert_eq!(after, before, "a refused ingest must not partially apply");
}

#[test]
fn ingesting_a_new_module_creates_its_table() {
    let (service, _dir) = common::test_service();
    let name = DatasetName::new("mixed").unwrap();
    service
        .create_dataset_impl(&name, common::fixture("delft.city.jsonl").to_str().unwrap())
        .unwrap();

    // This fixture carries a Bridge and a CityFurniture — two further modules.
    // Routing them is the extension's job; create_tables = true is ours.
    service
        .ingest_impl(
            &name,
            common::fixture("railway_7415.city.jsonl").to_str().unwrap(),
        )
        .unwrap();

    let modules: Vec<String> = service
        .describe_dataset_impl(&name)
        .unwrap()
        .modules
        .into_iter()
        .map(|m| m.name)
        .collect();
    assert!(modules.contains(&"bridge".to_string()), "got {modules:?}");
    assert!(
        modules.contains(&"city_furniture".to_string()),
        "got {modules:?}"
    );
}
