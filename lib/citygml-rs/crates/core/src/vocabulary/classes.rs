use crate::model::{Module, Namespace, QName};

/// CityGML 2.0 and 3.0 classes and the CityJSON City Object type each
/// becomes. A class both versions name alike is listed once.
///
/// Several classes collapse onto one CityJSON type where CityJSON draws no
/// distinction: an outer and an interior installation, a `Track` or
/// `TransportationComplex`, which CityJSON models as a `Road`, and CityGML
/// 3.0's generic spaces, each a `GenericCityObject`. A few are spelled
/// differently: CityJSON follows CityGML 3.0 (`BridgeConstructiveElement`,
/// `BuildingRoom`) where 2.0 differs, prefixes a tunnel's `HollowSpace`, a
/// building's `Storey` and a transportation `Square`
/// (`TunnelHollowSpace`, `BuildingStorey`, `TransportSquare`).
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
    (Module::Building, "BuildingRoom", "BuildingRoom"),
    (Module::Building, "BuildingFurniture", "BuildingFurniture"),
    (
        Module::Building,
        "BuildingConstructiveElement",
        "BuildingConstructiveElement",
    ),
    (Module::Building, "Storey", "BuildingStorey"),
    (Module::Building, "BuildingUnit", "BuildingUnit"),
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
    (
        Module::Bridge,
        "BridgeConstructiveElement",
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
    (
        Module::Tunnel,
        "TunnelConstructiveElement",
        "TunnelConstructiveElement",
    ),
    (Module::Tunnel, "HollowSpace", "TunnelHollowSpace"),
    (Module::Tunnel, "TunnelFurniture", "TunnelFurniture"),
    (
        Module::Construction,
        "OtherConstruction",
        "OtherConstruction",
    ),
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
        Module::Generics,
        "GenericOccupiedSpace",
        "GenericCityObject",
    ),
    (
        Module::Generics,
        "GenericUnoccupiedSpace",
        "GenericCityObject",
    ),
    (Module::Generics, "GenericLogicalSpace", "GenericCityObject"),
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
    (Module::Transportation, "Waterway", "Waterway"),
    (Module::Relief, "TINRelief", "TINRelief"),
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

/// CityGML boundary surface classes whose CityJSON semantic surface type is
/// spelled differently: CityGML 3.0's filling surfaces, and its
/// transportation markings and holes. Every other boundary surface class
/// of either version is the CityJSON type of the same name.
const SURFACES: &[(Module, &str, &str)] = &[
    (Module::Construction, "DoorSurface", "Door"),
    (Module::Construction, "WindowSurface", "Window"),
    (Module::Transportation, "Marking", "TransportationMarking"),
    (Module::Transportation, "HoleSurface", "TransportationHole"),
];

/// The CityJSON semantic surface type of the CityGML boundary surface class
/// `class` (`con:DoorSurface` -> `Door`).
pub fn cityjson_surface(class: &QName) -> &str {
    let Namespace::Module(module) = class.ns else {
        return &class.local;
    };
    SURFACES
        .iter()
        .find(|(m, local, _)| *m == module && *local == class.local)
        .map_or(&class.local, |(_, _, cityjson)| cityjson)
}
