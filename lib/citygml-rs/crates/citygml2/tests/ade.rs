//! ADEs on real PLATEAU documents: i-UR `uro:` properties of CityGML
//! objects, and `urf:` classes read as objects of their own.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use citygml_core::model::{Module, Namespace, QName, Shape, XmlAttribute, XmlContent, XmlNode};
use citygml2::{FeatureReader, ObjectStream, read_document};

const URO: &str = "https://www.geospatial.jp/iur/uro/3.2";
const URF: &str = "https://www.geospatial.jp/iur/urf/3.2";

/// A PLATEAU file, where the PLATEAU archive puts it: `udx/<type>/`.
fn data(name: &str) -> PathBuf {
    let ty = name.split('_').nth(1).unwrap();
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/plateau/udx")
        .join(ty)
        .join(name)
}

fn uro(local: &str) -> QName {
    QName::new(Namespace::Other(URO.into()), local)
}

fn leaf(local: &str, code_space: &str, text: &str) -> XmlNode {
    XmlNode {
        name: uro(local),
        attributes: vec![XmlAttribute {
            name: "codeSpace".into(),
            value: format!("../../codelists/{code_space}.xml"),
        }],
        content: XmlContent::Text(text.into()),
    }
}

fn element(local: &str, children: Vec<XmlNode>) -> XmlNode {
    XmlNode {
        name: uro(local),
        attributes: Vec::new(),
        content: XmlContent::Children(children),
    }
}

/// The road the plan's example is drawn from keeps its two `uro:` properties
/// verbatim, nested elements and `codeSpace`s included, and none of their
/// leaves becomes an attribute of the road.
#[test]
fn an_ade_property_is_kept_verbatim() {
    let road = FeatureReader::open(&data("53394529_tran_6697_op.gml"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(
        road.id.as_deref(),
        Some("tran_a031c7ad-64bc-4332-8949-17c3f00b53c0")
    );
    let names: Vec<&str> = road.ade.iter().map(|n| n.name.local.as_str()).collect();
    assert_eq!(
        names,
        ["tranDataQualityAttribute", "roadStructureAttribute"]
    );
    assert_eq!(
        road.ade[1],
        element(
            "roadStructureAttribute",
            vec![element(
                "RoadStructureAttribute",
                vec![leaf(
                    "sectionType",
                    "RoadStructureAttribute_sectionType",
                    "4"
                )]
            )]
        )
    );
    let XmlContent::Children(quality) = &road.ade[0].content else {
        panic!("{:?}", road.ade[0]);
    };
    let XmlContent::Children(fields) = &quality[0].content else {
        panic!("{:?}", quality[0]);
    };
    let thematic: Vec<&XmlNode> = fields
        .iter()
        .filter(|n| n.name.local == "thematicSrcDesc")
        .collect();
    assert_eq!(
        thematic,
        [
            &leaf(
                "thematicSrcDesc",
                "DataQualityAttribute_thematicSrcDesc",
                "023"
            ),
            &leaf(
                "thematicSrcDesc",
                "DataQualityAttribute_thematicSrcDesc",
                "400"
            ),
        ]
    );
    assert!(
        road.attributes.iter().all(
            |a| !matches!(&a.name, citygml_core::model::AttributeName::Property(q)
                if q.ns == Namespace::Other(URO.into()))
        ),
        "{:?}",
        road.attributes
    );
}

/// Every member of the urban-function file is an object of an `urf:` class,
/// read like a CityGML object: its `urf:lod1MultiSurface` is its geometry,
/// and its other `urf:` properties are kept as ADE properties.
#[test]
fn an_ade_class_is_read_as_an_object() {
    let mut reader = FeatureReader::open(&data("533946_urf_6697_08_op.gml")).unwrap();
    let objects: Vec<_> = reader.by_ref().collect::<Result<_, _>>().unwrap();
    assert!(reader.skipped_members().is_empty());
    let mut per_class: BTreeMap<&str, usize> = BTreeMap::new();
    for object in &objects {
        assert_eq!(object.class.ns, Namespace::Other(URF.into()));
        *per_class.entry(object.class.local.as_str()).or_default() += 1;
        assert_eq!(object.geometries.len(), 1, "{:?}", object.id);
        assert_eq!(object.geometries[0].lod, "1");
        assert!(matches!(object.geometries[0].shape, Shape::MultiSurface(_)));
        assert!(
            object
                .ade
                .iter()
                .all(|n| n.name.local != "lod1MultiSurface")
        );
    }
    assert_eq!(
        per_class,
        BTreeMap::from([("FirePreventionDistrict", 641), ("UseDistrict", 1593)])
    );
    assert_eq!(
        reader
            .ade_members()
            .iter()
            .map(|(name, n)| (name.as_str(), *n))
            .collect::<Vec<_>>(),
        [
            ("urf:FirePreventionDistrict", 641),
            ("urf:UseDistrict", 1593)
        ]
    );

    let district = objects
        .iter()
        .find(|o| o.id.as_deref() == Some("urf_ac32effe-8cb0-4ef9-a01f-9ff8925d8a58"))
        .unwrap();
    let names: Vec<&str> = district.ade.iter().map(|n| n.name.local.as_str()).collect();
    assert_eq!(
        names,
        [
            "function",
            "validFrom",
            "validFromType",
            "custodian",
            "notificationNumber",
            "finalNotificationDate",
            "finalNotificationNumber",
            "prefecture",
            "city",
            "dataQualityAttribute",
            "location",
            "floorAreaRate",
            "minimumSiteArea",
            "buildingCoverageRate",
            "buildingHeightLimits",
        ]
    );
    // `core:creationDate` is a CityGML property, read as an attribute.
    assert_eq!(district.attributes[0].name.local(), "creationDate");
}

/// What the up-front pass learns about a document's ADEs: which it uses,
/// under which prefix and schema, which of their names CityJSON will see
/// with a `+`, and which XML attributes are the same on every occurrence.
#[test]
fn the_document_index_declares_each_ade_it_uses() {
    let tran = read_document(&data("53394529_tran_6697_op.gml")).unwrap();
    let ades = tran.ade().ades();
    assert_eq!(ades.len(), 1);
    let ade = &ades[0];
    assert_eq!(ade.prefix, "uro");
    assert_eq!(ade.namespace, URO);
    assert_eq!(
        ade.schema_location.as_deref(),
        Some("../../schemas/iur/uro/3.2/urbanObject.xsd")
    );
    assert!(ade.classes.is_empty() && ade.surfaces.is_empty());
    assert_eq!(
        ade.properties,
        BTreeMap::from([(
            "Road".to_string(),
            BTreeSet::from([
                "roadStructureAttribute".to_string(),
                "tranDataQualityAttribute".to_string()
            ])
        )])
    );
    assert_eq!(
        ade.constant_attributes["sectionType"],
        BTreeMap::from([(
            "codeSpace".to_string(),
            "../../codelists/RoadStructureAttribute_sectionType.xml".to_string()
        )])
    );

    let urf = read_document(&data("533946_urf_6697_08_op.gml")).unwrap();
    let ades = urf.ade().ades();
    let prefixes: Vec<&str> = ades.iter().map(|a| a.prefix.as_str()).collect();
    assert_eq!(prefixes, ["urf", "uro"]);
    assert_eq!(
        ades[0].classes,
        BTreeSet::from(["FirePreventionDistrict".into(), "UseDistrict".into()])
    );
    // Properties of an ADE object are its own, never CityJSON `+` names.
    assert!(ades[0].properties.is_empty() && ades[1].properties.is_empty());
    assert_eq!(
        ades[0].constant_attributes["minimumSiteArea"],
        BTreeMap::from([("uom".to_string(), "m2".to_string())])
    );
}

/// An ADE class whose schema substitutes it for a CityGML class records
/// that class: `uro:UndergroundBuilding` substitutes for
/// `bldg:_AbstractBuilding`. Its CityGML properties, read by name as any
/// object's are, give it rooms of its own.
#[test]
fn an_ade_class_records_the_citygml_class_it_specialises() {
    let path = data("53394611_ubld_6697_op.gml");
    let document = read_document(&path).unwrap();
    let ade = document.ade().ade(URO).unwrap();
    assert_eq!(
        ade.parents,
        BTreeMap::from([(
            "UndergroundBuilding".to_string(),
            QName::module(Module::Building, "_AbstractBuilding")
        )])
    );
    let building = FeatureReader::open(&path)
        .unwrap()
        .find(|o| o.as_ref().unwrap().class == uro("UndergroundBuilding"))
        .unwrap()
        .unwrap();
    let rooms = building
        .children
        .iter()
        .filter(|c| c.class == QName::module(Module::Building, "Room"))
        .count();
    assert_eq!(rooms, 25);
}

/// A schema whose imports are missing still gives what it declares itself:
/// the `urf` tile beside the `urf` schema alone, without the `uro` schema
/// the `urf` one imports.
#[test]
fn a_schema_is_read_without_the_schemas_it_imports() {
    let dir = std::env::temp_dir().join(format!("citygml2-ade-{}", std::process::id()));
    let gml = dir.join("udx/urf/533946_urf_6697_08_op.gml");
    let xsd = dir.join("schemas/iur/urf/3.2/urbanFunction.xsd");
    for (from, to) in [
        (data("533946_urf_6697_08_op.gml"), &gml),
        (
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/data/plateau/schemas/iur/urf/3.2/urbanFunction.xsd"),
            &xsd,
        ),
    ] {
        std::fs::create_dir_all(to.parent().unwrap()).unwrap();
        std::fs::copy(from, to).unwrap();
    }
    let document = read_document(&gml);
    std::fs::remove_dir_all(&dir).unwrap();
    let document = document.unwrap();
    let parents = &document.ade().ade(URF).unwrap().parents;
    assert_eq!(
        parents["UseDistrict"],
        QName::module(Module::Core, "_CityObject")
    );
}

#[cfg(feature = "cjseq")]
mod cityjson {
    use serde_json::{Value, json};

    use super::*;

    fn features(name: &str) -> Vec<Value> {
        let transform = cjseq::Transform {
            scale: [1e-9, 1e-9, 1e-3],
            translate: [0.0; 3],
        };
        FeatureReader::open(&data(name))
            .unwrap()
            .features(transform)
            .map(|f| serde_json::to_value(f.unwrap()).unwrap())
            .collect()
    }

    /// An ADE property of a CityGML object is a `+` attribute: an element
    /// is a key, a repeated one an array, and a `codeSpace` every
    /// occurrence of its element shares is left to the extension's
    /// declaration.
    #[test]
    fn an_ade_property_is_a_plus_attribute() {
        let road = &features("53394529_tran_6697_op.gml")[0]["CityObjects"]["tran_a031c7ad-64bc-4332-8949-17c3f00b53c0"];
        assert_eq!(
            road["attributes"]["+roadStructureAttribute"],
            json!({"RoadStructureAttribute": {"sectionType": "4"}})
        );
        assert_eq!(
            road["attributes"]["+tranDataQualityAttribute"],
            json!({"DataQualityAttribute": {
                "geometrySrcDescLod1": "000",
                "geometrySrcDescLod2": "999",
                "geometrySrcDescLod3": "999",
                "thematicSrcDesc": ["023", "400"],
                "appearanceSrcDescLod3": "99",
                "publicSurveyDataQualityAttribute": {"PublicSurveyDataQualityAttribute": {
                    "srcScaleLod1": "1",
                    "publicSurveySrcDescLod1": "023"
                }}
            }})
        );
    }

    /// A `codeSpace` that differs between occurrences of its element stays
    /// with the value, as `@codeSpace` beside the text in `$`.
    #[test]
    fn a_varying_xml_attribute_stays_with_its_value() {
        let feature = &features("53394519_bldg_6697_op.gml")[0];
        let building = &feature["CityObjects"][feature["id"].as_str().unwrap()];
        assert_eq!(
            building["attributes"]["+bldgKeyValuePairAttribute"][0],
            json!({"KeyValuePairAttribute": {
                "key": "100",
                "codeValue": {
                    "$": "11",
                    "@codeSpace": "../../codelists/KeyValuePairAttribute_key100.xml"
                }
            }})
        );
    }

    /// An object of an ADE class is a `+` City Object whose own properties
    /// are attributes without `+`, typed as the ADE's schema declares them
    /// (`floorAreaRate` an `xs:integer`, `minimumSiteArea` a
    /// `gml:MeasureType`, `prefecture` a `gml:CodeType`); an element of
    /// another ADE keeps its prefix.
    #[test]
    fn an_ade_object_is_a_plus_city_object() {
        let features = features("533946_urf_6697_08_op.gml");
        let id = "urf_ac32effe-8cb0-4ef9-a01f-9ff8925d8a58";
        let feature = features.iter().find(|f| f["id"] == id).unwrap();
        let district = &feature["CityObjects"][id];
        assert_eq!(district["type"], "+UseDistrict");
        assert_eq!(district["geometry"][0]["type"], "MultiSurface");
        assert_eq!(district["geometry"][0]["lod"], "1");
        assert_eq!(
            district["attributes"],
            json!({
                "creationDate": "2026-03-13",
                "function": "1",
                "validFrom": "2004-06-24",
                "validFromType": "3",
                "custodian": "東京都",
                "notificationNumber": "東京都告示第1073号",
                "finalNotificationDate": "2023-04-28",
                "finalNotificationNumber": "東京都告示第625号",
                "prefecture": "13",
                "city": "13100",
                "dataQualityAttribute": {"uro:DataQualityAttribute": {
                    "geometrySrcDescLod1": "400",
                    "thematicSrcDesc": "400"
                }},
                "location": "江戸川区北葛西三丁目",
                "floorAreaRate": 60,
                "minimumSiteArea": 70.0,
                "buildingCoverageRate": 30,
                "buildingHeightLimits": 10.0
            })
        );
    }

    /// Each ADE a document uses is a CityJSON `extensions` member under its
    /// prefix: its schema, its version, its namespace, the XML attributes
    /// stated once for the document, the `+` names it adds, and where in
    /// their objects its elements go.
    #[test]
    fn each_ade_is_declared_as_an_extension() {
        let document = read_document(&data("533946_urf_6697_08_op.gml")).unwrap();
        let extensions = citygml_core::convert::cjseq::extensions(document.ade());
        let keys: Vec<&str> = extensions.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, ["urf", "uro"]);
        let urf = serde_json::to_value(&extensions[0].1).unwrap();
        assert_eq!(
            urf["url"],
            "https://www.geospatial.jp/iur/schemas/urf/3.2/urbanFunction.xsd"
        );
        assert_eq!(urf["version"], "3.2");
        assert_eq!(urf["xmlns"], URF);
        // Both classes specialise `core:_CityObject`, through urf's own.
        let city_object =
            json!({"allOf": [{"$ref": "cityobjects.schema.json#/_AbstractCityObject"}]});
        assert_eq!(
            urf["extraCityObjects"],
            json!({"+FirePreventionDistrict": city_object, "+UseDistrict": city_object})
        );
        assert_eq!(
            urf["xmlAttributes"]["minimumSiteArea"],
            json!({"uom": "m2"})
        );
        assert_eq!(
            urf["xmlAttributes"]["function"],
            json!({"codeSpace": "../../codelists/Common_districtsAndZonesType.xml"})
        );

        let document = read_document(&data("53394529_tran_6697_op.gml")).unwrap();
        let extensions = citygml_core::convert::cjseq::extensions(document.ade());
        assert_eq!(extensions.len(), 1);
        let uro = serde_json::to_value(&extensions[0].1).unwrap();
        assert_eq!(
            uro["extraAttributes"],
            json!({"Road": {"+roadStructureAttribute": {}, "+tranDataQualityAttribute": {}}})
        );
        // Where a writer puts each property: the hook it substitutes for.
        assert_eq!(
            uro["substitutionGroups"],
            json!({
                "roadStructureAttribute": "tran:_GenericApplicationPropertyOfRoad",
                "tranDataQualityAttribute": "tran:_GenericApplicationPropertyOfTransportationComplex"
            })
        );
        // And for an ADE class, its content model's order, its base's first.
        let order: Vec<&str> = urf["propertyOrder"]["UseDistrict"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        let at = |name: &str| order.iter().position(|n| *n == name).unwrap();
        assert_eq!(order[..3], ["class", "function", "usage"]);
        assert!(at("function") < at("lod1MultiCurve"));
        assert!(at("lod1MultiCurve") < at("areaInTotal"));
        assert!(at("areaInTotal") < at("floorAreaRate"));

        // A class specialising a CityGML class is declared the way a
        // CityJSON Extension declares one: by reference to CityJSON's schema.
        let document = read_document(&data("53394611_ubld_6697_op.gml")).unwrap();
        let extensions = citygml_core::convert::cjseq::extensions(document.ade());
        let uro = serde_json::to_value(&extensions[0].1).unwrap();
        assert_eq!(
            uro["extraCityObjects"],
            json!({"+UndergroundBuilding": {"allOf": [{"$ref": "cityobjects.schema.json#/_AbstractBuilding"}]}})
        );
    }
}
