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

#[tokio::test]
#[ignore = "network: reads the published Delft feed; run with `just test-real-data`"]
async fn the_real_feed_ingests_every_object() {
    let (service, _dir) = common::test_service();
    let name = DatasetName::new("delft_real").unwrap();

    let info = service
        .create_dataset(&name, DELFT_SEQ)
        .await
        .expect("ingest the published Delft feed");

    let rows: usize = info.modules.iter().map(|m| m.rows).sum();
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
    assert_eq!(building.rows, TOTAL_OBJECTS);
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
    assert_eq!(BUILDINGS + BUILDING_PARTS, TOTAL_OBJECTS);
}
