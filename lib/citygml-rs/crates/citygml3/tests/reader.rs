//! The CityGML 3.0 reader on real documents, for what the oracle does not
//! show.

#![cfg(feature = "cjseq")]

use std::collections::BTreeMap;
use std::path::PathBuf;

use citygml3::FeatureReader;
use serde_json::{Value, json};

fn data(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(name)
}

fn features(name: &str) -> Vec<Value> {
    let transform = cjseq::Transform {
        scale: [0.001; 3],
        translate: [0.0; 3],
    };
    FeatureReader::open(&data(name))
        .unwrap()
        .features(transform)
        .map(|f| serde_json::to_value(f.unwrap()).unwrap())
        .collect()
}

/// A Melbourne road whose surfaces are in its section's traffic spaces:
/// each traffic and auxiliary traffic area is a semantic surface of the
/// road's own geometry, as CityGML 2.0's `trafficArea` is.
#[test]
fn a_road_section_s_traffic_areas_are_the_road_s_surfaces() {
    let features = features("ogc/citygml3_individual_section_1carriageway.gml");
    assert_eq!(features.len(), 1);
    let road = &features[0]["CityObjects"][features[0]["id"].as_str().unwrap()];
    assert_eq!(road["type"], "Road");
    let geometry = road["geometry"].as_array().unwrap();
    assert_eq!(geometry.len(), 1);
    assert_eq!(geometry[0]["type"], "MultiSurface");
    assert_eq!(geometry[0]["lod"], "2");
    let mut surfaces: BTreeMap<&str, usize> = BTreeMap::new();
    for s in geometry[0]["semantics"]["surfaces"].as_array().unwrap() {
        *surfaces.entry(s["type"].as_str().unwrap()).or_default() += 1;
    }
    assert_eq!(
        surfaces,
        BTreeMap::from([("AuxiliaryTrafficArea", 19), ("TrafficArea", 19)])
    );
}

/// An xAL 3 address names its parts by where a `NameElement`, `Number` or
/// `Identifier` sits; each becomes the CityJSON member an xAL 2 address's
/// part does.
#[test]
fn an_xal3_address_has_the_members_of_an_xal2_one() {
    let features = features("lod4_building_v2.gml");
    let building = features
        .iter()
        .flat_map(|f| f["CityObjects"].as_object().unwrap().values())
        .find(|co| co["type"] == "Building")
        .unwrap();
    let mut address = building["address"][0].clone();
    assert_eq!(address["location"]["type"], "MultiPoint");
    address.as_object_mut().unwrap().remove("location");
    assert_eq!(
        address,
        json!({
            "country": "Germany",
            "locality": "Eggenstein-Leopoldshafen",
            "thoroughfareNumber": "1",
            "thoroughfareName": "Hermann-von-Helmholtz-Platz",
            "postcode": "76344"
        })
    );
}
