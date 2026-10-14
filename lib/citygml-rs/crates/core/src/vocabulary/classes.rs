use crate::model::{Module, Namespace, QName};

/// CityGML classes and the CityJSON City Object type each becomes.
///
/// Several CityGML 2.0 classes collapse onto one CityJSON type where CityJSON
/// draws no distinction: an outer and an interior installation, and a
/// `Track` or `TransportationComplex`, which CityJSON models as a `Road`. A
/// few are spelled differently, following CityGML 3.0
/// (`BridgeConstructiveElement`, `BuildingRoom`, `TunnelHollowSpace`), and a
/// transportation `Square` is a `TransportSquare`.
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
    (Module::Building, "Room", "BuildingRoom"),
    (Module::Building, "BuildingFurniture", "BuildingFurniture"),
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
    (Module::Bridge, "BridgeRoom", "BridgeRoom"),
    (Module::Bridge, "BridgeFurniture", "BridgeFurniture"),
    (Module::Tunnel, "Tunnel", "Tunnel"),
    (Module::Tunnel, "TunnelPart", "TunnelPart"),
    (Module::Tunnel, "TunnelInstallation", "TunnelInstallation"),
    (
        Module::Tunnel,
        "IntTunnelInstallation",
        "TunnelInstallation",
    ),
    (Module::Tunnel, "HollowSpace", "TunnelHollowSpace"),
    (Module::Tunnel, "TunnelFurniture", "TunnelFurniture"),
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
    (Module::Transportation, "Track", "Road"),
    (Module::Transportation, "TransportationComplex", "Road"),
    (Module::Transportation, "Square", "TransportSquare"),
    (Module::Relief, "TINRelief", "TINRelief"),
];

/// The installations whose CityGML 2.0 class says whether they are inside
/// or outside the construction, which CityJSON's single installation type
/// does not: CityGML 3.0 says it with `relationToConstruction`.
const RELATION_TO_CONSTRUCTION: &[(Module, &str, &str)] = &[
    (Module::Building, "BuildingInstallation", "outside"),
    (Module::Building, "IntBuildingInstallation", "inside"),
    (Module::Bridge, "BridgeInstallation", "outside"),
    (Module::Bridge, "IntBridgeInstallation", "inside"),
    (Module::Tunnel, "TunnelInstallation", "outside"),
    (Module::Tunnel, "IntTunnelInstallation", "inside"),
];

/// CityGML 3.0's `relationToConstruction` of an installation class whose
/// name implies it, or `None`.
pub fn relation_to_construction(class: &QName) -> Option<&'static str> {
    let Namespace::Module(module) = class.ns else {
        return None;
    };
    RELATION_TO_CONSTRUCTION
        .iter()
        .find(|(m, local, _)| *m == module && *local == class.local)
        .map(|(_, _, relation)| *relation)
}

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

/// CityGML's abstract classes an ADE class substitutes for, and the
/// definition of CityJSON's `cityobjects.schema.json` each corresponds to.
/// A name is matched without its leading `_` (CityGML 2.0 writes
/// `_AbstractBuilding`, CityGML 3.0 `AbstractBuilding`). CityJSON has no
/// abstract bridge, tunnel or water object, so those take the class
/// itself.
const ABSTRACT: &[(Module, &str, &str)] = &[
    (Module::Building, "AbstractBuilding", "_AbstractBuilding"),
    (Module::Bridge, "AbstractBridge", "Bridge"),
    (Module::Tunnel, "AbstractTunnel", "Tunnel"),
    (
        Module::Transportation,
        "TransportationObject",
        "_AbstractTransportationComplex",
    ),
    (
        Module::Transportation,
        "TransportationComplex",
        "_AbstractTransportationComplex",
    ),
    (Module::WaterBody, "WaterObject", "WaterBody"),
];

/// The definition of CityJSON's `cityobjects.schema.json` an extension
/// class specialising the CityGML class `class` refers to:
/// `_AbstractBuilding` for `bldg:_AbstractBuilding`, the City Object type
/// of a class CityJSON has one for, else `_AbstractCityObject`.
pub fn cityjson_definition(class: &QName) -> &'static str {
    let Namespace::Module(module) = class.ns else {
        return "_AbstractCityObject";
    };
    let local = class.local.trim_start_matches('_');
    ABSTRACT
        .iter()
        .find(|(m, name, _)| *m == module && *name == local)
        .map(|(_, _, definition)| *definition)
        .or_else(|| cityjson_type(class))
        .unwrap_or("_AbstractCityObject")
}
