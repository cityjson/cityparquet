//! CityGML 3.0 ADEs on the OGC encoding's examples: an `adeOf…` hook's
//! properties on a CityGML object, and an ADE class specialising one.

#![cfg(feature = "cjseq")]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use citygml_core::model::{Module, QName};
use citygml3::{FeatureReader, ObjectStream, read_document};
use serde_json::{Value, json};

fn data(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/ogc")
        .join(name)
}

fn features(path: &Path) -> Vec<Value> {
    let transform = cjseq::Transform {
        scale: [1e-9, 1e-9, 1e-3],
        translate: [0.0; 3],
    };
    FeatureReader::open(path)
        .unwrap()
        .features(transform)
        .map(|f| serde_json::to_value(f.unwrap()).unwrap())
        .collect()
}

/// The Urban Planning ADE's properties sit in a `uro:BuildingProperties`
/// inside `bldg:adeOfAbstractBuilding`; each is a `+` attribute of the
/// building, typed as the ADE's schema declares, and the ADE an extension
/// adding them to `Building`.
#[test]
fn an_ade_hook_s_properties_are_plus_attributes() {
    let path = data("urban-planning-ade/53394517_bldg_6697_2_op_CityGML3.gml");
    let features = features(&path);
    assert_eq!(features.len(), 1);
    let building = &features[0]["CityObjects"]["BLD_77ca1a15-3b35-4386-8f86-152ed71c4c64"];
    let attributes = building["attributes"].as_object().unwrap();
    assert_eq!(
        attributes["+buildingDetails"],
        json!({"BuildingDetails": {
            "buildingRoofEdgeArea": 2513.8552,
            "districtsAndZonesType": "4",
            "prefecture": "13",
            "city": "13104",
            "surveyYear": "2016"
        }})
    );
    assert_eq!(
        attributes["+extendedAttribute"],
        json!([
            {"KeyValuePair": {"key": "2", "codeValue": {
                "$": "2", "@codeSpace": "../codelists/extendedAttribute_key2.xml"}}},
            {"KeyValuePair": {"key": "105", "codeValue": {
                "$": "2", "@codeSpace": "../codelists/extendedAttribute_key105.xml"}}},
            {"KeyValuePair": {"key": "106", "codeValue": {
                "$": "20", "@codeSpace": "../codelists/extendedAttribute_key106.xml"}}}
        ])
    );
    assert_eq!(attributes["measuredHeight"], json!(31.7));

    let document = read_document(&path).unwrap();
    let extensions = citygml_core::convert::cjseq::extensions(document.ade());
    assert_eq!(extensions.len(), 1);
    let (prefix, extension) = &extensions[0];
    assert_eq!(prefix, "uro");
    let extension = serde_json::to_value(extension).unwrap();
    assert_eq!(
        extension["extraAttributes"],
        json!({"Building": {"+buildingDetails": {}, "+extendedAttribute": {}}})
    );
}

/// The Test ADE's `test:IndustrialBuilding` substitutes for
/// `bldg:Building`: a `+IndustrialBuilding` City Object with the building's
/// CityGML attributes, typed, and its own ADE properties, from the
/// `bldg:adeOfAbstractBuilding` hook and beside it, without a `+`. The GML
/// one of them holds is counted and left out.
#[test]
fn an_ade_class_specialising_a_building_keeps_both_kinds_of_property() {
    let path = data("test-ade/industrial_building_dataset_v3.gml");
    let document = read_document(&path).unwrap();
    let ade = document
        .ade()
        .ade("http://www.citygml.org/ade/TestADE/2.0")
        .unwrap();
    assert_eq!(
        ade.parents,
        BTreeMap::from([(
            "IndustrialBuilding".to_string(),
            QName::module(Module::Building, "Building")
        )])
    );
    let extensions = citygml_core::convert::cjseq::extensions(document.ade());
    let extension = serde_json::to_value(&extensions[0].1).unwrap();
    assert_eq!(
        extension["extraCityObjects"],
        json!({"+IndustrialBuilding": {"allOf": [{"$ref": "cityobjects.schema.json#/Building"}]}})
    );

    let transform = cjseq::Transform {
        scale: [0.001; 3],
        translate: [0.0; 3],
    };
    let mut reader = FeatureReader::open(&path).unwrap().features(transform);
    let features: Vec<Value> = reader
        .by_ref()
        .map(|f| serde_json::to_value(f.unwrap()).unwrap())
        .collect();
    assert_eq!(features.len(), 1);
    let objects = features[0]["CityObjects"].as_object().unwrap();
    assert_eq!(objects.len(), 1);
    let building = objects.values().next().unwrap();
    assert_eq!(building["type"], "+IndustrialBuilding");
    assert_eq!(
        building["attributes"],
        json!({
            "description": "FZK/IAI test cases only",
            "name": "Testcase-0-0_LoD2",
            "class": "1100",
            "roofType": "1030",
            "storeysAboveGround": 5,
            "storeysBelowGround": 0,
            "storeyHeightsAboveGround": [2.0],
            "storeyHeightsBelowGround": [0.0],
            "ownerName": "Smith",
            "energyPerformanceCertification": {"EnergyPerformanceCertification": {
                "certificationName": "certName",
                "certificationId": "certId"
            }},
            "buildingUnderground": {"BuildingUnderground": {
                "equippedWith": {"LightingFacility": {"electricalPower": 4000.0}}
            }},
            "remark": "remark"
        })
    );
    assert_eq!(
        reader.reader().ade_geometries(),
        &BTreeMap::from([("test:buildingUnderground".to_string(), 1)])
    );
}
