mod common;

use citylake::core::interface::types::{CityLakeError, DatasetName, ModuleName, QueryParams};

fn seeded() -> (
    citylake::core::db::service::DuckLakeService,
    tempfile::TempDir,
    DatasetName,
) {
    let (service, dir) = common::test_service();
    let name = DatasetName::new("delft").unwrap();
    service
        .create_dataset_impl(&name, common::fixture("delft.city.jsonl").to_str().unwrap())
        .unwrap();
    (service, dir, name)
}

#[test]
fn objects_come_back_as_json_rows() {
    let (service, _dir, name) = seeded();
    let module = ModuleName::new("building").unwrap();

    let rows = service
        .query_objects_impl(&name, &module, &QueryParams::default())
        .expect("query the building module");
    assert!(!rows.is_empty());
    assert!(rows[0].get("id").is_some(), "a row should carry its id");
}

#[test]
fn a_page_is_bounded_and_offsettable() {
    let (service, _dir, name) = seeded();
    let module = ModuleName::new("building").unwrap();

    let first = service
        .query_objects_impl(
            &name,
            &module,
            &QueryParams {
                filter: None,
                limit: 1,
                offset: 0,
            },
        )
        .unwrap();
    assert_eq!(first.len(), 1);

    let second = service
        .query_objects_impl(
            &name,
            &module,
            &QueryParams {
                filter: None,
                limit: 1,
                offset: 1,
            },
        )
        .unwrap();
    assert_eq!(second.len(), 1);
    assert_ne!(first[0].get("id"), second[0].get("id"));
}

#[test]
fn a_filter_narrows_the_result() {
    // hierarchy_7415.city.json is a mixed set: one Building (bldg-1) with two
    // BuildingStorey children, plus a further standalone Building (bldg-2) —
    // four rows in the building module table, two "Building" and two
    // "BuildingStorey". Unlike delft.city.jsonl (all Building), a filter here
    // can actually narrow the result, so this test can fail for its stated
    // reason: drop the filter from the query and it must fail.
    let (service, _dir) = common::test_service();
    let name = DatasetName::new("hierarchy").unwrap();
    service
        .create_dataset_impl(
            &name,
            common::fixture("hierarchy_7415.city.json")
                .to_str()
                .unwrap(),
        )
        .unwrap();
    let module = ModuleName::new("building").unwrap();

    let unfiltered = service
        .query_objects_impl(&name, &module, &QueryParams::default())
        .unwrap();
    assert_eq!(unfiltered.len(), 4, "the fixture carries four objects");

    let filtered = service
        .query_objects_impl(
            &name,
            &module,
            &QueryParams {
                filter: Some("object_type = 'Building'".into()),
                limit: 100,
                offset: 0,
            },
        )
        .unwrap();
    assert!(!filtered.is_empty(), "two rows are Buildings");
    assert!(filtered
        .iter()
        .all(|row| row.get("object_type").and_then(|v| v.as_str()) == Some("Building")));
    assert!(
        filtered.len() < unfiltered.len(),
        "the filter must actually narrow the result: {} filtered vs {} unfiltered",
        filtered.len(),
        unfiltered.len()
    );
}

#[test]
fn querying_a_module_the_dataset_lacks_is_an_error_not_a_panic() {
    let (service, _dir, name) = seeded();
    let module = ModuleName::new("tunnel").unwrap();

    let err = service
        .query_objects_impl(&name, &module, &QueryParams::default())
        .expect_err("the fixture has no tunnels");
    assert!(format!("{err}").contains("tunnel"));
}

#[test]
fn querying_a_dataset_that_was_never_created_names_the_dataset() {
    let (service, _dir) = common::test_service();
    let name = DatasetName::new("nonesuch").unwrap();
    let module = ModuleName::new("building").unwrap();

    let err = service
        .query_objects_impl(&name, &module, &QueryParams::default())
        .expect_err("the dataset was never created");
    assert!(
        matches!(err, CityLakeError::DatasetNotFound(_)),
        "a never-created dataset must report DatasetNotFound, got: {err}"
    );
}
