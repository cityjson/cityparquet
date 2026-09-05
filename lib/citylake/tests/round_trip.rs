//! The acceptance gate: everything composing, once.

mod common;

use citylake::core::interface::repository::CityLakeRepository;
use citylake::core::interface::types::{DatasetName, ModuleName, QueryParams};

#[tokio::test]
async fn a_cityjson_source_survives_the_full_journey() {
    let (service, dir) = common::test_service();
    let name = DatasetName::new("hierarchy").unwrap();
    let building = ModuleName::new("building").unwrap();

    // 1. Ingest. The fixture is one Building (bldg-1) with two Storey
    // children (storey-1, storey-2), plus an unrelated standalone Building
    // (bldg-2) — all four routed to the building module table, plus a
    // declared CRS. bldg-2 exists so the cascade in step 3 (which removes
    // bldg-1's whole subtree) does not empty the dataset entirely: an object
    // table left with zero rows cannot be written out and read back as a
    // package.
    let created = service
        .create_dataset(
            &name,
            common::fixture("hierarchy_7415.city.json")
                .to_str()
                .unwrap(),
        )
        .await
        .expect("create");
    let ingested: usize = created.modules.iter().map(|m| m.rows).sum();
    assert_eq!(ingested, 4);
    assert!(created.crs.as_ref().expect("a CRS").contains("7415"));

    // 2. It validates clean on arrival.
    assert!(service
        .validate(&name)
        .await
        .unwrap()
        .iter()
        .all(|f| f.severity != "error"));

    // 3. Delete the PARENT — not an arbitrary row, and not the standalone
    // bldg-2 (also a Building, but childless) — so the cascade removes the
    // whole subtree: the parent plus its two Storey children.
    let rows = service
        .query_objects(
            &name,
            &building,
            &QueryParams {
                filter: None,
                limit: 1000,
                offset: 0,
            },
        )
        .await
        .unwrap();
    let parent = rows
        .iter()
        .find(|row| row["object_type"] == "Building" && !row["children"].is_null())
        .expect("the Building row with children")["id"]
        .as_str()
        .unwrap()
        .to_string();
    let removed = service.delete_object(&name, &parent).await.expect("delete");
    assert_eq!(removed, 3, "the parent and both of its children");

    // 4. Still consistent afterwards — a cascade that left a dangling child
    //    or a stale feature_id would show up here.
    assert!(service
        .validate(&name)
        .await
        .unwrap()
        .iter()
        .all(|f| f.severity != "error"));

    // 5. Write it out as a package.
    let package = dir.path().join("out");
    let files = service
        .write_package(&name, package.to_str().unwrap())
        .await
        .expect("write the package");
    assert!(files.iter().any(|f| f.file == "metadata.json"));

    // 6. Read the package back into a second dataset.
    let reloaded = DatasetName::new("reloaded").unwrap();
    let info = service
        .create_dataset(&reloaded, package.to_str().unwrap())
        .await
        .expect("read the package back");

    // 7. What went out is what came back: the same rows, the same CRS.
    assert_eq!(
        info.modules.iter().map(|m| m.rows).sum::<usize>(),
        ingested - removed
    );
    assert!(info
        .crs
        .expect("a CRS survives the round trip")
        .contains("7415"));

    // 8. And it is still a valid package.
    assert!(service
        .validate(&reloaded)
        .await
        .unwrap()
        .iter()
        .all(|f| f.severity != "error"));
}
