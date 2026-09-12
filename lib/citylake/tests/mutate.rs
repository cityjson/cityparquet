mod common;

use citylake::core::interface::types::{CityLakeError, DatasetName, ModuleName, QueryParams};
use serde_json::json;

fn seeded() -> (
    citylake::core::db::service::DuckLakeService,
    tempfile::TempDir,
    DatasetName,
) {
    let (service, dir) = common::test_service();
    let name = DatasetName::new("hier").unwrap();
    // This fixture has a parent/child hierarchy, which is what makes the
    // cascade observable.
    service
        .create_dataset_impl(
            &name,
            common::fixture("hierarchy.city.json").to_str().unwrap(),
        )
        .unwrap();
    (service, dir, name)
}

fn ids(service: &citylake::core::db::service::DuckLakeService, name: &DatasetName) -> Vec<String> {
    let module = ModuleName::new("building").unwrap();
    service
        .query_objects_impl(
            name,
            &module,
            &QueryParams {
                filter: None,
                limit: 1000,
                offset: 0,
            },
        )
        .unwrap()
        .into_iter()
        .filter_map(|row| row.get("id")?.as_str().map(str::to_string))
        .collect()
}

#[test]
fn an_attribute_update_lands_on_the_row() {
    let (service, _dir, name) = seeded();
    // A storey, not the first (Building) id: bldg-1's object_type is already
    // "Building", so writing that same value there would pass even if the
    // UPDATE silently matched zero rows. A storey's object_type is "Storey",
    // so only a genuine write turns it into "Building".
    let id = ids(&service, &name).into_iter().last().unwrap();

    let mut attributes = serde_json::Map::new();
    attributes.insert("object_type".into(), json!("Building"));
    service
        .update_object_impl(&name, &id, &attributes)
        .expect("update the object");

    let module = ModuleName::new("building").unwrap();
    let rows = service
        .query_objects_impl(
            &name,
            &module,
            &QueryParams {
                filter: Some(format!("id = '{id}'")),
                limit: 1,
                offset: 0,
            },
        )
        .unwrap();
    assert_eq!(rows[0].get("object_type").unwrap(), &json!("Building"));
}

#[test]
fn updating_an_absent_dataset_is_an_error() {
    let (service, _dir) = common::test_service();
    let name = DatasetName::new("nonesuch").unwrap();
    let mut attributes = serde_json::Map::new();
    attributes.insert("object_type".into(), json!("Building"));

    let err = service
        .update_object_impl(&name, "whatever", &attributes)
        .expect_err("the dataset was never created");
    assert!(
        matches!(err, CityLakeError::DatasetNotFound(_)),
        "a never-created dataset must report DatasetNotFound, got: {err}"
    );
}

#[test]
fn updating_an_absent_id_is_an_error() {
    let (service, _dir, name) = seeded();
    let mut attributes = serde_json::Map::new();
    attributes.insert("object_type".into(), json!("Building"));

    let err = service
        .update_object_impl(&name, "no-such-object", &attributes)
        .expect_err("an absent id must not silently succeed");
    assert!(
        matches!(
            &err,
            CityLakeError::ObjectNotFound { id, .. } if id == "no-such-object"
        ),
        "an absent id must report ObjectNotFound, got: {err}"
    );
}

#[test]
fn an_empty_attribute_update_against_an_unknown_id_is_still_an_error() {
    let (service, _dir, name) = seeded();
    // An empty body is legitimately a no-op against an id that exists, but
    // the early return for it must not run before the id is checked: the
    // caller sees 404 either way an unknown id is addressed, not 204 for an
    // empty body and 404 for a non-empty one.
    let attributes = serde_json::Map::new();

    let err = service
        .update_object_impl(&name, "no-such-object", &attributes)
        .expect_err("an absent id must not silently succeed, even with no attributes");
    assert!(
        matches!(
            &err,
            CityLakeError::ObjectNotFound { id, .. } if id == "no-such-object"
        ),
        "an absent id must report ObjectNotFound regardless of body, got: {err}"
    );
}

#[test]
fn deleting_a_parent_cascades_to_its_children() {
    let (service, _dir, name) = seeded();
    let before = ids(&service, &name);
    let parent = before.first().expect("a parent object").clone();

    let deleted = service.delete_object_impl(&name, &parent).unwrap();
    // The fixture is one Building with two Storey children, all in the
    // building module table, so deleting the parent must take the whole
    // subtree — not the one row a non-cascading delete would remove.
    assert_eq!(
        deleted,
        before.len(),
        "deleting the parent must cascade to its children"
    );

    let after = ids(&service, &name);
    assert!(!after.contains(&parent));
    assert_eq!(before.len() - after.len(), deleted);
}

#[test]
fn deleting_by_predicate_removes_the_matching_objects() {
    let (service, _dir, name) = seeded();
    let deleted = service
        .delete_where_impl(&name, "object_type = 'Building'")
        .expect("delete by predicate");
    assert!(deleted > 0);

    let module = ModuleName::new("building").unwrap();
    let remaining = service
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
    assert!(remaining.is_empty());
}

#[test]
fn reconciling_an_untouched_dataset_changes_nothing() {
    let (service, _dir, name) = seeded();
    let before = ids(&service, &name);

    // Both the reader and reconcile union a row's geometry across every stored
    // LoD and across its descendants, so a freshly read package is already
    // reconciled for the structural columns.
    service.reconcile_impl(&name).expect("reconcile");

    assert_eq!(ids(&service, &name), before);
}
