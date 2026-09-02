//! The operations against the published Delft datasets, at their real size.
//!
//! Every test here is `#[ignore]`d: they reach the network, and a gate that
//! needs the network is one people learn to skip. Run them deliberately with
//! `just test-real-data`. Marking them ignored rather than returning early
//! keeps a skipped run visibly skipped — an early return would report as a
//! pass, which is worse than no test.
//!
//! Nothing is downloaded. The extension auto-loads `httpfs` and resolves an
//! `https://` source as readily as a local path, so the URL goes straight to
//! `create_dataset` and the remote read path is exercised too.

mod common;

use citylake::core::interface::repository::CityLakeRepository;
use citylake::core::interface::types::{DatasetName, ModuleName, QueryParams};

/// 6.6 MB, 2231 CityObjects, EPSG:7415.
const DELFT_SEQ: &str = "https://cityjson.open3d.city/cityjsonseq/delft.city.jsonl";

/// Every object in this feed is a Building or a BuildingPart, and the Building
/// module holds both — so one module table carries all 2231 rows.
const TOTAL_OBJECTS: usize = 2231;
const BUILDINGS: usize = 1115;
const BUILDING_PARTS: usize = 1116;

// Compile-time guard: catches a maintainer updating one published figure
// after a re-publication of the feed without the other two.
const _: () = assert!(BUILDINGS + BUILDING_PARTS == TOTAL_OBJECTS);

#[tokio::test]
#[ignore = "network: reads the published Delft feed; run with `just test-real-data`"]
async fn the_real_feed_ingests_every_object() {
    let (service, _dir) = common::test_service();
    let name = DatasetName::new("delft_real").unwrap();

    let info = service
        .create_dataset(&name, DELFT_SEQ)
        .await
        .expect("ingest the published Delft feed");

    // Filter to object tables: describe_dataset also reports sidecar tables
    // (materials, textures, geometry_templates) alongside object tables, and
    // TOTAL_OBJECTS counts objects only. Summing every module regardless of
    // role would pass today and fail the moment this feed carries appearance
    // data, for a reason that has nothing to do with an ingest bug.
    let rows: usize = info
        .modules
        .iter()
        .filter(|m| m.role == "object")
        .map(|m| m.rows)
        .sum();
    assert_eq!(
        rows, TOTAL_OBJECTS,
        "every object in the feed must arrive; the fixtures cannot show this"
    );
}

#[tokio::test]
#[ignore = "network: reads the published Delft feed; run with `just test-real-data`"]
async fn the_real_feed_routes_everything_into_the_building_module() {
    let (service, _dir) = common::test_service();
    let name = DatasetName::new("delft_real").unwrap();
    let info = service.create_dataset(&name, DELFT_SEQ).await.unwrap();

    // Building and BuildingPart belong to the same CityGML module, so a
    // correct routing produces exactly one object table. A second object
    // table would mean the extension split a module, or that the empty seed
    // survived when it should not have.
    let object_modules: Vec<&str> = info
        .modules
        .iter()
        .filter(|m| m.role == "object")
        .map(|m| m.name.as_str())
        .collect();
    assert_eq!(
        object_modules,
        vec!["building"],
        "the whole feed is one module; got {object_modules:?}"
    );

    let building = info
        .modules
        .iter()
        .find(|m| m.name == "building")
        .expect("a building module");
    assert_eq!(
        building.rows, TOTAL_OBJECTS,
        "the single building module must carry every object in the feed"
    );
}

#[tokio::test]
#[ignore = "network: reads the published Delft feed; run with `just test-real-data`"]
async fn the_real_feed_declares_its_crs() {
    let (service, _dir) = common::test_service();
    let name = DatasetName::new("delft_real").unwrap();
    let info = service.create_dataset(&name, DELFT_SEQ).await.unwrap();

    // The footer is minted by the extension from a probe row. At this size the
    // probe reads one row out of 2231 — that it still lands on the right CRS is
    // the thing worth checking.
    let crs = info.crs.expect("the feed declares EPSG:7415");
    assert!(crs.contains("7415"), "unexpected CRS: {crs}");
}

#[tokio::test]
#[ignore = "network: reads the published Delft feed; run with `just test-real-data`"]
async fn the_object_type_split_matches_the_published_figures() {
    let (service, _dir) = common::test_service();
    let name = DatasetName::new("delft_real").unwrap();
    let module = ModuleName::new("building").unwrap();
    service.create_dataset(&name, DELFT_SEQ).await.unwrap();

    let count_of = |object_type: &'static str| {
        let service = &service;
        let name = &name;
        let module = &module;
        async move {
            service
                .query_objects(
                    name,
                    module,
                    &QueryParams {
                        filter: Some(format!("object_type = '{object_type}'")),
                        limit: 5000,
                        offset: 0,
                    },
                )
                .await
                .unwrap_or_else(|e| panic!("query {object_type}: {e}"))
                .len()
        }
    };

    // Published figures for this dataset. A routing or filter regression moves
    // one of these without moving the total, which the first test would miss.
    assert_eq!(count_of("Building").await, BUILDINGS);
    assert_eq!(count_of("BuildingPart").await, BUILDING_PARTS);
}

#[tokio::test]
#[ignore = "network: reads the published Delft feed; run with `just test-real-data`"]
async fn the_real_dataset_validates_clean_on_arrival() {
    let (service, _dir) = common::test_service();
    let name = DatasetName::new("delft_real").unwrap();
    service.create_dataset(&name, DELFT_SEQ).await.unwrap();

    // 2231 objects with a real parent/child hierarchy: feature_id, the
    // reciprocal arrays and bbox are all derived across the whole set, and a
    // derivation that works on three objects can still break on 2231.
    let findings = service.validate(&name).await.expect("validate");
    let errors: Vec<_> = findings.iter().filter(|f| f.severity == "error").collect();
    assert!(errors.is_empty(), "unexpected errors: {errors:?}");
}

#[tokio::test]
#[ignore = "network: reads the published Delft feed; run with `just test-real-data`"]
async fn a_real_package_round_trips_through_the_lake() {
    let (service, dir) = common::test_service();
    let name = DatasetName::new("delft_real").unwrap();
    // Filter to object tables, as in the ingest test above: describe_dataset
    // also reports sidecar tables, and TOTAL_OBJECTS-scale round-tripping is
    // about object rows, not sidecar bookkeeping.
    let ingested: usize = service
        .create_dataset(&name, DELFT_SEQ)
        .await
        .unwrap()
        .modules
        .iter()
        .filter(|m| m.role == "object")
        .map(|m| m.rows)
        .sum();
    assert_eq!(
        ingested, TOTAL_OBJECTS,
        "the round trip must start from the whole feed, not an empty ingest"
    );

    let out = dir.path().join("delft_pkg");
    let written = service
        .write_package(&name, out.to_str().unwrap())
        .await
        .expect("write the package");
    assert!(written.iter().any(|f| f.file == "building.parquet"));
    assert!(written.iter().any(|f| f.file == "metadata.json"));

    let reloaded = DatasetName::new("delft_reloaded").unwrap();
    let info = service
        .create_dataset(&reloaded, out.to_str().unwrap())
        .await
        .expect("read the written package back");

    assert_eq!(
        info.modules
            .iter()
            .filter(|m| m.role == "object")
            .map(|m| m.rows)
            .sum::<usize>(),
        ingested,
        "every row must survive the round trip at this size"
    );
    assert!(
        info.crs
            .expect("a CRS survives the round trip")
            .contains("7415"),
        "the written package must still declare EPSG:7415"
    );
}

#[tokio::test]
#[ignore = "network: reads the published Delft feed; run with `just test-real-data`"]
async fn deleting_a_real_parent_cascades_and_leaves_the_package_valid() {
    let (service, _dir) = common::test_service();
    let name = DatasetName::new("delft_real").unwrap();
    let module = ModuleName::new("building").unwrap();
    let before: usize = service
        .create_dataset(&name, DELFT_SEQ)
        .await
        .unwrap()
        .modules
        .iter()
        .filter(|m| m.role == "object")
        .map(|m| m.rows)
        .sum();

    // Find a real parent rather than hardcoding an id: this dataset's
    // Building rows carry BuildingPart children, and picking one from the data
    // keeps the test valid if the published feed is ever regenerated.
    let parents = service
        .query_objects(
            &name,
            &module,
            &QueryParams {
                filter: Some("children IS NOT NULL AND len(children) > 0".into()),
                limit: 1,
                offset: 0,
            },
        )
        .await
        .expect("find a parent");
    let parent_row = parents
        .into_iter()
        .next()
        .expect("the feed must contain a Building with children");
    let parent = parent_row
        .get("id")
        .and_then(|id| id.as_str())
        .expect("the parent row must carry an id")
        .to_string();

    let removed = service.delete_object(&name, &parent).await.expect("delete");
    // This feed is two-level (Building -> BuildingPart), so the parent's own
    // `children` array is the whole subtree the cascade must remove: the
    // parent itself plus each listed child. A deeper hierarchy would need the
    // transitive count instead, and this exact assertion would then be wrong.
    let expected = 1 + parent_row
        .get("children")
        .and_then(|c| c.as_array())
        .map(|c| c.len())
        .expect("the parent row must carry its children");
    assert_eq!(
        removed,
        expected,
        "deleting a parent must remove it and all {} of its children",
        expected - 1
    );

    let after: usize = service
        .describe_dataset(&name)
        .await
        .unwrap()
        .modules
        .iter()
        .filter(|m| m.role == "object")
        .map(|m| m.rows)
        .sum();
    assert_eq!(
        after,
        before - removed,
        "the module's row count must drop by exactly what the cascade reported"
    );

    // The cascade must not leave a dangling reference behind in a set this
    // size — survivor cleanup is the part small fixtures cannot stress.
    let findings = service.validate(&name).await.expect("re-validate");
    let errors: Vec<_> = findings.iter().filter(|f| f.severity == "error").collect();
    assert!(
        errors.is_empty(),
        "cascade left the package invalid: {errors:?}"
    );
}
