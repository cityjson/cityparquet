//! The acceptance gate: everything composing, once.

mod common;

use citylake::core::interface::repository::CityLakeRepository;
use citylake::core::interface::types::{DatasetName, ModuleName, QueryParams};

#[tokio::test]
async fn a_cityjson_source_survives_the_full_journey() {
    let (service, dir) = common::test_service();
    let name = DatasetName::new("delft").unwrap();
    let building = ModuleName::new("building").unwrap();

    // 1. Ingest.
    let created = service
        .create_dataset(&name, common::fixture("delft.city.jsonl").to_str().unwrap())
        .await
        .expect("create");
    let ingested: usize = created.modules.iter().map(|m| m.rows).sum();
    assert!(ingested > 0);
    assert!(created.crs.as_ref().expect("a CRS").contains("7415"));

    // 2. It validates clean on arrival.
    assert!(service
        .validate(&name)
        .await
        .unwrap()
        .iter()
        .all(|f| f.severity != "error"));

    // 3. Delete one object; the cascade is the extension's.
    let first = service
        .query_objects(
            &name,
            &building,
            &QueryParams {
                filter: None,
                limit: 1,
                offset: 0,
            },
        )
        .await
        .unwrap()[0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let removed = service.delete_object(&name, &first).await.expect("delete");
    assert!(removed >= 1);

    // 4. Still consistent afterwards — a cascade that left a dangling parent
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
