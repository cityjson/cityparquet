//! A Road's traffic areas are semantic surfaces of the Road.
//!
//! CityGML 2.0 hangs a `tran:Road`'s surfaces off two dedicated properties,
//! `tran:trafficArea` and `tran:auxiliaryTrafficArea`, each wrapping a
//! `TrafficArea` / `AuxiliaryTrafficArea` feature with its own `lodNMultiSurface`.
//! CityJSON flattens them into the Road's geometry at that LoD, with
//! `TrafficArea` / `AuxiliaryTrafficArea` as semantic surface types — the same
//! shape `boundedBy` takes for a Building, and read by the same machinery.
//!
//! Counts are hand-transcribed from the fixture, not snapshotted from the
//! reader.

use std::collections::BTreeMap;
use std::path::PathBuf;

use cityparquet::citygml::{FeatureReader, parse_header};

fn data_fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(name);
    assert!(p.exists(), "missing committed fixture {name}");
    p
}

#[test]
fn a_roads_traffic_areas_become_its_semantic_surfaces() {
    let path = data_fixture("plateau_yokohama_road_traffic_areas.gml");
    let header = parse_header(&path).unwrap();
    let mut reader = FeatureReader::open(&path, &header.transform).unwrap();
    let feature = reader.next().expect("one member").expect("it must read");

    assert_eq!(
        feature.city_objects.len(),
        1,
        "the areas are not objects of their own"
    );
    let road = feature.city_objects.values().next().unwrap();
    assert_eq!(road.thetype, "Road");
    let geoms = road.geometry.as_ref().expect("geometry");

    // LoD1 is untouched: the one standalone polygon, no semantics.
    let lod1: Vec<_> = geoms
        .iter()
        .filter(|g| g.lod.as_deref() == Some("1"))
        .collect();
    assert_eq!(lod1.len(), 1);
    let lod1_faces: Vec<Vec<Vec<usize>>> =
        serde_json::from_value(lod1[0].boundaries.clone()).unwrap();
    assert_eq!(lod1_faces.len(), 1);

    // LoD3 is the traffic areas: 19 + 4 faces, each typed.
    let lod3: Vec<_> = geoms
        .iter()
        .filter(|g| g.lod.as_deref() == Some("3"))
        .collect();
    assert_eq!(lod3.len(), 1, "one geometry per LoD");
    let semantics = lod3[0].semantics.as_ref().expect("typed faces");
    let types: Vec<&str> = semantics["surfaces"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["type"].as_str().unwrap())
        .collect();
    let mut tally: BTreeMap<&str, usize> = BTreeMap::new();
    for v in semantics["values"].as_array().unwrap() {
        *tally
            .entry(types[v.as_u64().expect("every face tagged") as usize])
            .or_default() += 1;
    }
    assert_eq!(
        tally,
        [("AuxiliaryTrafficArea", 4), ("TrafficArea", 19)]
            .into_iter()
            .collect()
    );
}

#[test]
fn a_standalone_face_no_traffic_area_covers_is_kept_untyped() {
    // One LoD yields one geometry — the encoder keeps only the first per
    // object and LoD — so the standalone surface cannot be emitted beside the
    // semantic one. It cannot be dropped either: face B belongs to no traffic
    // area and would simply vanish. It joins the semantic geometry, untyped.
    let path = data_fixture("road_standalone_face_beside_traffic_area.gml");
    let header = parse_header(&path).unwrap();
    let mut reader = FeatureReader::open(&path, &header.transform).unwrap();
    let feature = reader.next().expect("one member").expect("it must read");
    let road = feature.city_objects.values().next().unwrap();
    let lod3: Vec<_> = road
        .geometry
        .as_ref()
        .unwrap()
        .iter()
        .filter(|g| g.lod.as_deref() == Some("3"))
        .collect();
    assert_eq!(lod3.len(), 1, "one geometry per LoD");

    let faces: Vec<Vec<Vec<usize>>> = serde_json::from_value(lod3[0].boundaries.clone()).unwrap();
    assert_eq!(faces.len(), 2, "A once, and B");
    let semantics = lod3[0].semantics.as_ref().unwrap();
    let values = semantics["values"].as_array().unwrap();
    let typed = values.iter().filter(|v| !v.is_null()).count();
    assert_eq!((typed, values.len() - typed), (1, 1), "A typed, B untyped");
}
