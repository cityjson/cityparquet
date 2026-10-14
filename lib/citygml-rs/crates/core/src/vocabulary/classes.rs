use crate::model::{Module, Namespace, QName};

/// CityGML classes and the CityJSON City Object type each becomes.
///
/// Two CityGML 2.0 classes collapse onto one CityJSON type where CityJSON
/// draws no distinction (an outer and an interior installation), and a few
/// are spelled differently: CityJSON follows CityGML 3.0 in
/// `BridgeConstructiveElement`, and calls a transportation `Square` a
/// `TransportSquare`.
const CLASSES: &[(Module, &str, &str)] = &[
    (Module::Building, "Building", "Building"),
    (Module::Building, "BuildingPart", "BuildingPart"),
    (
        Module::Building,
        "BuildingInstallation",
        "BuildingInstallation",
    ),
    (
        Module::Building,
        "IntBuildingInstallation",
        "BuildingInstallation",
    ),
    (Module::Bridge, "Bridge", "Bridge"),
    (Module::Bridge, "BridgePart", "BridgePart"),
    (Module::Bridge, "BridgeInstallation", "BridgeInstallation"),
    (
        Module::Bridge,
        "IntBridgeInstallation",
        "BridgeInstallation",
    ),
    (
        Module::Bridge,
        "BridgeConstructionElement",
        "BridgeConstructiveElement",
    ),
    (Module::Tunnel, "Tunnel", "Tunnel"),
    (Module::WaterBody, "WaterBody", "WaterBody"),
    (Module::LandUse, "LandUse", "LandUse"),
    (Module::CityFurniture, "CityFurniture", "CityFurniture"),
    (
        Module::Vegetation,
        "SolitaryVegetationObject",
        "SolitaryVegetationObject",
    ),
    (Module::Vegetation, "PlantCover", "PlantCover"),
    (Module::Generics, "GenericCityObject", "GenericCityObject"),
    (
        Module::CityObjectGroup,
        "CityObjectGroup",
        "CityObjectGroup",
    ),
    (Module::Transportation, "Road", "Road"),
    (Module::Transportation, "Railway", "Railway"),
    (Module::Transportation, "Square", "TransportSquare"),
];

/// The CityJSON City Object type of a CityGML class, or `None` for a class
/// CityJSON has no type for.
pub fn cityjson_type(class: &QName) -> Option<&'static str> {
    let Namespace::Module(module) = class.ns else {
        return None;
    };
    CLASSES
        .iter()
        .find(|(m, local, _)| *m == module && *local == class.local)
        .map(|(_, _, cityjson)| *cityjson)
}
