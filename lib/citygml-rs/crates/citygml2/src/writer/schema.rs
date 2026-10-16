//! What the CityGML 2.0 schemas say a writer needs, as data: each class's
//! properties in the order its content model puts them, and the CityJSON
//! names each class and boundary surface is written for.
//!
//! A class's content lists only what follows the properties every city
//! object has (`gml:description`, `gml:name`, `core:creationDate`, …,
//! generic attributes and appearance), which come first. The places for the
//! properties an ADE adds, one per class along the class's line of
//! inheritance, are listed beside it. `tests/schema.rs` checks every
//! list against the official schemas.

use citygml_core::convert::{Class, Property, PropertyKind, PropertyNs, Vocabulary};
use citygml_core::model::{Module, Namespace, QName};

/// One property of a class's content.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Slot {
    /// A property holding a value: name, what it holds, whether it repeats.
    Value(&'static str, PropertyKind, bool),
    /// A geometry property, `lod<N><Kind>`, or a building's `lod0FootPrint`.
    Geometry(&'static str),
    /// A property holding the object's boundary surfaces.
    Surfaces(&'static str),
    /// A property nesting sub-features of the given classes.
    Nest(&'static str, &'static [&'static str]),
    /// A postal address (`bldg:address`).
    Address(&'static str),
    /// A group's `groupMember`.
    Member,
    /// A boundary surface's `opening`, holding doors and windows.
    Opening,
    /// A relief component's `lod`, which a TIN's LoD is written to.
    ReliefLod,
    /// A `TINRelief`'s `tin`.
    Tin,
}

/// A class this writer writes.
#[derive(Debug)]
pub(crate) struct ClassDef {
    pub module: Module,
    pub name: &'static str,
    /// The CityJSON City Object type written as this class, if any.
    pub cityjson: Option<&'static str>,
    pub content: &'static [Slot],
    /// Where the content has the hooks an ADE's properties substitute for,
    /// besides `_CityObject`'s, which follows the generic attributes.
    pub hooks: &'static [Hook],
}

/// A place in a class's content for the properties an ADE adds: the
/// position of `<module>:_GenericApplicationPropertyOf<class>`, which
/// follows `at` of the content's slots.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Hook {
    pub at: usize,
    pub module: Module,
    pub class: &'static str,
}

impl Hook {
    /// The hook element's name.
    pub fn element(&self) -> String {
        format!("_GenericApplicationPropertyOf{}", self.class)
    }
}

use PropertyKind::{Boolean, Integer, MeasureList, Number, Text, Year};
use Slot::{Address, Geometry, Member, Nest, Opening, ReliefLod, Surfaces, Tin, Value};

const CODE: PropertyKind = Text;
const LENGTH: PropertyKind = Number { measure: true };
const COUNT: PropertyKind = Integer { non_negative: true };

/// `class`, `function` and `usage`, which most classes begin with.
macro_rules! cfu {
    () => {
        [
            Value("class", CODE, false),
            Value("function", CODE, true),
            Value("usage", CODE, true),
        ]
    };
}

/// `_AbstractBuilding`'s content, `Building`'s and `BuildingPart`'s alike.
const ABSTRACT_BUILDING: &[Slot] = &{
    let [c, f, u] = cfu!();
    [
        c,
        f,
        u,
        Value("yearOfConstruction", Year, false),
        Value("yearOfDemolition", Year, false),
        Value("roofType", CODE, false),
        Value("measuredHeight", LENGTH, false),
        Value("storeysAboveGround", COUNT, false),
        Value("storeysBelowGround", COUNT, false),
        Value("storeyHeightsAboveGround", MeasureList, false),
        Value("storeyHeightsBelowGround", MeasureList, false),
        Geometry("lod0FootPrint"),
        Geometry("lod0RoofEdge"),
        Geometry("lod1Solid"),
        Geometry("lod1MultiSurface"),
        Geometry("lod1TerrainIntersection"),
        Geometry("lod2Solid"),
        Geometry("lod2MultiSurface"),
        Geometry("lod2MultiCurve"),
        Geometry("lod2TerrainIntersection"),
        Nest("outerBuildingInstallation", &["BuildingInstallation"]),
        Nest("interiorBuildingInstallation", &["IntBuildingInstallation"]),
        Surfaces("boundedBy"),
        Geometry("lod3Solid"),
        Geometry("lod3MultiSurface"),
        Geometry("lod3MultiCurve"),
        Geometry("lod3TerrainIntersection"),
        Geometry("lod4Solid"),
        Geometry("lod4MultiSurface"),
        Geometry("lod4MultiCurve"),
        Geometry("lod4TerrainIntersection"),
        Nest("interiorRoom", &["Room"]),
        Nest("consistsOfBuildingPart", &["BuildingPart"]),
        Address("address"),
    ]
};

const ABSTRACT_BRIDGE: &[Slot] = &{
    let [c, f, u] = cfu!();
    [
        c,
        f,
        u,
        Value("yearOfConstruction", Year, false),
        Value("yearOfDemolition", Year, false),
        Value("isMovable", Boolean, false),
        Geometry("lod1Solid"),
        Geometry("lod1MultiSurface"),
        Geometry("lod1TerrainIntersection"),
        Geometry("lod2Solid"),
        Geometry("lod2MultiSurface"),
        Geometry("lod2MultiCurve"),
        Geometry("lod2TerrainIntersection"),
        Nest("outerBridgeConstruction", &["BridgeConstructionElement"]),
        Nest("outerBridgeInstallation", &["BridgeInstallation"]),
        Nest("interiorBridgeInstallation", &["IntBridgeInstallation"]),
        Surfaces("boundedBy"),
        Geometry("lod3Solid"),
        Geometry("lod3MultiSurface"),
        Geometry("lod3MultiCurve"),
        Geometry("lod3TerrainIntersection"),
        Geometry("lod4Solid"),
        Geometry("lod4MultiSurface"),
        Geometry("lod4MultiCurve"),
        Geometry("lod4TerrainIntersection"),
        Nest("interiorBridgeRoom", &["BridgeRoom"]),
        Nest("consistsOfBridgePart", &["BridgePart"]),
        Address("address"),
    ]
};

const ABSTRACT_TUNNEL: &[Slot] = &{
    let [c, f, u] = cfu!();
    [
        c,
        f,
        u,
        Value("yearOfConstruction", Year, false),
        Value("yearOfDemolition", Year, false),
        Geometry("lod1Solid"),
        Geometry("lod1MultiSurface"),
        Geometry("lod1TerrainIntersection"),
        Geometry("lod2Solid"),
        Geometry("lod2MultiSurface"),
        Geometry("lod2MultiCurve"),
        Geometry("lod2TerrainIntersection"),
        Nest("outerTunnelInstallation", &["TunnelInstallation"]),
        Nest("interiorTunnelInstallation", &["IntTunnelInstallation"]),
        Surfaces("boundedBy"),
        Geometry("lod3Solid"),
        Geometry("lod3MultiSurface"),
        Geometry("lod3MultiCurve"),
        Geometry("lod3TerrainIntersection"),
        Geometry("lod4Solid"),
        Geometry("lod4MultiSurface"),
        Geometry("lod4MultiCurve"),
        Geometry("lod4TerrainIntersection"),
        Nest("interiorHollowSpace", &["HollowSpace"]),
        Nest("consistsOfTunnelPart", &["TunnelPart"]),
    ]
};

/// An outer installation's content, in every module that has one.
const INSTALLATION: &[Slot] = &{
    let [c, f, u] = cfu!();
    [
        c,
        f,
        u,
        Geometry("lod2Geometry"),
        Geometry("lod3Geometry"),
        Geometry("lod4Geometry"),
        Geometry("lod2ImplicitRepresentation"),
        Geometry("lod3ImplicitRepresentation"),
        Geometry("lod4ImplicitRepresentation"),
        Surfaces("boundedBy"),
    ]
};

const INTERIOR_INSTALLATION: &[Slot] = &{
    let [c, f, u] = cfu!();
    [
        c,
        f,
        u,
        Geometry("lod4Geometry"),
        Geometry("lod4ImplicitRepresentation"),
        Surfaces("boundedBy"),
    ]
};

const FURNITURE: &[Slot] = &{
    let [c, f, u] = cfu!();
    [
        c,
        f,
        u,
        Geometry("lod4Geometry"),
        Geometry("lod4ImplicitRepresentation"),
    ]
};

const ROOM: &[Slot] = &{
    let [c, f, u] = cfu!();
    [
        c,
        f,
        u,
        Geometry("lod4Solid"),
        Geometry("lod4MultiSurface"),
        Surfaces("boundedBy"),
        Nest("interiorFurniture", &["BuildingFurniture"]),
        Nest("roomInstallation", &["IntBuildingInstallation"]),
    ]
};

const BRIDGE_ROOM: &[Slot] = &{
    let [c, f, u] = cfu!();
    [
        c,
        f,
        u,
        Geometry("lod4Solid"),
        Geometry("lod4MultiSurface"),
        Surfaces("boundedBy"),
        Nest("interiorFurniture", &["BridgeFurniture"]),
        Nest("bridgeRoomInstallation", &["IntBridgeInstallation"]),
    ]
};

const HOLLOW_SPACE: &[Slot] = &{
    let [c, f, u] = cfu!();
    [
        c,
        f,
        u,
        Geometry("lod4Solid"),
        Geometry("lod4MultiSurface"),
        Surfaces("boundedBy"),
        Nest("interiorFurniture", &["TunnelFurniture"]),
        Nest("hollowSpaceInstallation", &["IntTunnelInstallation"]),
    ]
};

const BRIDGE_CONSTRUCTION_ELEMENT: &[Slot] = &{
    let [c, f, u] = cfu!();
    [
        c,
        f,
        u,
        Geometry("lod1Geometry"),
        Geometry("lod2Geometry"),
        Geometry("lod3Geometry"),
        Geometry("lod4Geometry"),
        Geometry("lod1TerrainIntersection"),
        Geometry("lod2TerrainIntersection"),
        Geometry("lod3TerrainIntersection"),
        Geometry("lod4TerrainIntersection"),
        Geometry("lod1ImplicitRepresentation"),
        Geometry("lod2ImplicitRepresentation"),
        Geometry("lod3ImplicitRepresentation"),
        Geometry("lod4ImplicitRepresentation"),
        Surfaces("boundedBy"),
    ]
};

/// `CityFurniture`'s content, and with a LoD 0 `GenericCityObject`'s.
const CITY_FURNITURE: &[Slot] = &{
    let [c, f, u] = cfu!();
    [
        c,
        f,
        u,
        Geometry("lod1Geometry"),
        Geometry("lod2Geometry"),
        Geometry("lod3Geometry"),
        Geometry("lod4Geometry"),
        Geometry("lod1TerrainIntersection"),
        Geometry("lod2TerrainIntersection"),
        Geometry("lod3TerrainIntersection"),
        Geometry("lod4TerrainIntersection"),
        Geometry("lod1ImplicitRepresentation"),
        Geometry("lod2ImplicitRepresentation"),
        Geometry("lod3ImplicitRepresentation"),
        Geometry("lod4ImplicitRepresentation"),
    ]
};

const GENERIC_CITY_OBJECT: &[Slot] = &{
    let [c, f, u] = cfu!();
    [
        c,
        f,
        u,
        Geometry("lod0Geometry"),
        Geometry("lod1Geometry"),
        Geometry("lod2Geometry"),
        Geometry("lod3Geometry"),
        Geometry("lod4Geometry"),
        Geometry("lod0TerrainIntersection"),
        Geometry("lod1TerrainIntersection"),
        Geometry("lod2TerrainIntersection"),
        Geometry("lod3TerrainIntersection"),
        Geometry("lod4TerrainIntersection"),
        Geometry("lod0ImplicitRepresentation"),
        Geometry("lod1ImplicitRepresentation"),
        Geometry("lod2ImplicitRepresentation"),
        Geometry("lod3ImplicitRepresentation"),
        Geometry("lod4ImplicitRepresentation"),
    ]
};

const SOLITARY_VEGETATION_OBJECT: &[Slot] = &{
    let [c, f, u] = cfu!();
    [
        c,
        f,
        u,
        Value("species", CODE, false),
        Value("height", LENGTH, false),
        Value("trunkDiameter", LENGTH, false),
        Value("crownDiameter", LENGTH, false),
        Geometry("lod1Geometry"),
        Geometry("lod2Geometry"),
        Geometry("lod3Geometry"),
        Geometry("lod4Geometry"),
        Geometry("lod1ImplicitRepresentation"),
        Geometry("lod2ImplicitRepresentation"),
        Geometry("lod3ImplicitRepresentation"),
        Geometry("lod4ImplicitRepresentation"),
    ]
};

const PLANT_COVER: &[Slot] = &{
    let [c, f, u] = cfu!();
    [
        c,
        f,
        u,
        Value("averageHeight", LENGTH, false),
        Geometry("lod1MultiSurface"),
        Geometry("lod2MultiSurface"),
        Geometry("lod3MultiSurface"),
        Geometry("lod4MultiSurface"),
        Geometry("lod1MultiSolid"),
        Geometry("lod2MultiSolid"),
        Geometry("lod3MultiSolid"),
        Geometry("lod4MultiSolid"),
    ]
};

const WATER_BODY: &[Slot] = &{
    let [c, f, u] = cfu!();
    [
        c,
        f,
        u,
        Geometry("lod0MultiCurve"),
        Geometry("lod0MultiSurface"),
        Geometry("lod1MultiCurve"),
        Geometry("lod1MultiSurface"),
        Geometry("lod1Solid"),
        Geometry("lod2Solid"),
        Geometry("lod3Solid"),
        Geometry("lod4Solid"),
        Surfaces("boundedBy"),
    ]
};

const LAND_USE: &[Slot] = &{
    let [c, f, u] = cfu!();
    [
        c,
        f,
        u,
        Geometry("lod0MultiSurface"),
        Geometry("lod1MultiSurface"),
        Geometry("lod2MultiSurface"),
        Geometry("lod3MultiSurface"),
        Geometry("lod4MultiSurface"),
    ]
};

const TRANSPORTATION_COMPLEX: &[Slot] = &{
    let [c, f, u] = cfu!();
    [
        c,
        f,
        u,
        Surfaces("trafficArea"),
        Surfaces("auxiliaryTrafficArea"),
        Geometry("lod0Network"),
        Geometry("lod1MultiSurface"),
        Geometry("lod2MultiSurface"),
        Geometry("lod3MultiSurface"),
        Geometry("lod4MultiSurface"),
    ]
};

const CITY_OBJECT_GROUP: &[Slot] = &{
    let [c, f, u] = cfu!();
    [c, f, u, Member, Geometry("geometry")]
};

const TIN_RELIEF: &[Slot] = &[ReliefLod, Tin];

/// The content of a building's, bridge's or tunnel's boundary surface.
const BOUNDARY_SURFACE: &[Slot] = &[
    Geometry("lod2MultiSurface"),
    Geometry("lod3MultiSurface"),
    Geometry("lod4MultiSurface"),
    Opening,
];

/// A door's or window's.
const OPENING: &[Slot] = &[
    Geometry("lod3MultiSurface"),
    Geometry("lod4MultiSurface"),
    Geometry("lod3ImplicitRepresentation"),
    Geometry("lod4ImplicitRepresentation"),
];

/// A door's, which may have an address.
const DOOR: &[Slot] = &[
    Geometry("lod3MultiSurface"),
    Geometry("lod4MultiSurface"),
    Geometry("lod3ImplicitRepresentation"),
    Geometry("lod4ImplicitRepresentation"),
    Address("address"),
];

const WATER_SURFACE: &[Slot] = &[
    Geometry("lod2Surface"),
    Geometry("lod3Surface"),
    Geometry("lod4Surface"),
    Value("waterLevel", CODE, false),
];

const WATER_BOUNDARY_SURFACE: &[Slot] = &[
    Geometry("lod2Surface"),
    Geometry("lod3Surface"),
    Geometry("lod4Surface"),
];

const TRAFFIC_AREA: &[Slot] = &{
    let [c, f, u] = cfu!();
    [
        c,
        f,
        u,
        Value("surfaceMaterial", CODE, false),
        Geometry("lod2MultiSurface"),
        Geometry("lod3MultiSurface"),
        Geometry("lod4MultiSurface"),
    ]
};

macro_rules! class {
    ($module:ident, $name:literal, $cityjson:expr, $content:expr, [$($hook:expr),*]) => {
        ClassDef {
            module: Module::$module,
            name: $name,
            cityjson: $cityjson,
            content: $content,
            hooks: &[$($hook),*],
        }
    };
}

const fn hook(at: usize, module: Module, class: &'static str) -> Hook {
    Hook { at, module, class }
}

/// Every city object class this writer writes. Where several share a
/// CityJSON type, the first is the one written, unless
/// [`Vocabulary::class`] picks another by an attribute.
pub(crate) const CLASSES: &[ClassDef] = &[
    class!(
        Building,
        "Building",
        Some("Building"),
        ABSTRACT_BUILDING,
        [
            hook(0, Module::Core, "Site"),
            hook(
                ABSTRACT_BUILDING.len(),
                Module::Building,
                "AbstractBuilding"
            ),
            hook(ABSTRACT_BUILDING.len(), Module::Building, "Building")
        ]
    ),
    class!(
        Building,
        "BuildingPart",
        Some("BuildingPart"),
        ABSTRACT_BUILDING,
        [
            hook(0, Module::Core, "Site"),
            hook(
                ABSTRACT_BUILDING.len(),
                Module::Building,
                "AbstractBuilding"
            ),
            hook(ABSTRACT_BUILDING.len(), Module::Building, "BuildingPart")
        ]
    ),
    class!(
        Building,
        "BuildingInstallation",
        Some("BuildingInstallation"),
        INSTALLATION,
        [hook(
            INSTALLATION.len(),
            Module::Building,
            "BuildingInstallation"
        )]
    ),
    class!(
        Building,
        "IntBuildingInstallation",
        None,
        INTERIOR_INSTALLATION,
        [hook(
            INTERIOR_INSTALLATION.len(),
            Module::Building,
            "IntBuildingInstallation"
        )]
    ),
    class!(
        Building,
        "Room",
        Some("BuildingRoom"),
        ROOM,
        [hook(ROOM.len(), Module::Building, "Room")]
    ),
    class!(
        Building,
        "BuildingFurniture",
        Some("BuildingFurniture"),
        FURNITURE,
        [hook(FURNITURE.len(), Module::Building, "BuildingFurniture")]
    ),
    class!(
        Bridge,
        "Bridge",
        Some("Bridge"),
        ABSTRACT_BRIDGE,
        [
            hook(0, Module::Core, "Site"),
            hook(ABSTRACT_BRIDGE.len(), Module::Bridge, "AbstractBridge"),
            hook(ABSTRACT_BRIDGE.len(), Module::Bridge, "Bridge")
        ]
    ),
    class!(
        Bridge,
        "BridgePart",
        Some("BridgePart"),
        ABSTRACT_BRIDGE,
        [
            hook(0, Module::Core, "Site"),
            hook(ABSTRACT_BRIDGE.len(), Module::Bridge, "AbstractBridge"),
            hook(ABSTRACT_BRIDGE.len(), Module::Bridge, "BridgePart")
        ]
    ),
    class!(
        Bridge,
        "BridgeInstallation",
        Some("BridgeInstallation"),
        INSTALLATION,
        [hook(
            INSTALLATION.len(),
            Module::Bridge,
            "BridgeInstallation"
        )]
    ),
    class!(
        Bridge,
        "IntBridgeInstallation",
        None,
        INTERIOR_INSTALLATION,
        [hook(
            INTERIOR_INSTALLATION.len(),
            Module::Bridge,
            "IntBridgeInstallation"
        )]
    ),
    class!(
        Bridge,
        "BridgeConstructionElement",
        Some("BridgeConstructiveElement"),
        BRIDGE_CONSTRUCTION_ELEMENT,
        [hook(
            BRIDGE_CONSTRUCTION_ELEMENT.len(),
            Module::Bridge,
            "BridgeConstructionElement"
        )]
    ),
    class!(
        Bridge,
        "BridgeRoom",
        Some("BridgeRoom"),
        BRIDGE_ROOM,
        [hook(BRIDGE_ROOM.len(), Module::Bridge, "BridgeRoom")]
    ),
    class!(
        Bridge,
        "BridgeFurniture",
        Some("BridgeFurniture"),
        FURNITURE,
        [hook(FURNITURE.len(), Module::Bridge, "BridgeFurniture")]
    ),
    class!(
        Tunnel,
        "Tunnel",
        Some("Tunnel"),
        ABSTRACT_TUNNEL,
        [
            hook(0, Module::Core, "Site"),
            hook(ABSTRACT_TUNNEL.len(), Module::Tunnel, "AbstractTunnel"),
            hook(ABSTRACT_TUNNEL.len(), Module::Tunnel, "Tunnel")
        ]
    ),
    class!(
        Tunnel,
        "TunnelPart",
        Some("TunnelPart"),
        ABSTRACT_TUNNEL,
        [
            hook(0, Module::Core, "Site"),
            hook(ABSTRACT_TUNNEL.len(), Module::Tunnel, "AbstractTunnel"),
            hook(ABSTRACT_TUNNEL.len(), Module::Tunnel, "TunnelPart")
        ]
    ),
    class!(
        Tunnel,
        "TunnelInstallation",
        Some("TunnelInstallation"),
        INSTALLATION,
        [hook(
            INSTALLATION.len(),
            Module::Tunnel,
            "TunnelInstallation"
        )]
    ),
    class!(
        Tunnel,
        "IntTunnelInstallation",
        None,
        INTERIOR_INSTALLATION,
        [hook(
            INTERIOR_INSTALLATION.len(),
            Module::Tunnel,
            "IntTunnelInstallation"
        )]
    ),
    class!(
        Tunnel,
        "HollowSpace",
        Some("TunnelHollowSpace"),
        HOLLOW_SPACE,
        [hook(HOLLOW_SPACE.len(), Module::Tunnel, "HollowSpace")]
    ),
    class!(
        Tunnel,
        "TunnelFurniture",
        Some("TunnelFurniture"),
        FURNITURE,
        [hook(FURNITURE.len(), Module::Tunnel, "TunnelFurniture")]
    ),
    class!(
        WaterBody,
        "WaterBody",
        Some("WaterBody"),
        WATER_BODY,
        [
            hook(0, Module::WaterBody, "WaterObject"),
            hook(WATER_BODY.len(), Module::WaterBody, "WaterBody")
        ]
    ),
    class!(
        LandUse,
        "LandUse",
        Some("LandUse"),
        LAND_USE,
        [hook(LAND_USE.len(), Module::LandUse, "LandUse")]
    ),
    class!(
        CityFurniture,
        "CityFurniture",
        Some("CityFurniture"),
        CITY_FURNITURE,
        [hook(
            CITY_FURNITURE.len(),
            Module::CityFurniture,
            "CityFurniture"
        )]
    ),
    class!(
        Vegetation,
        "SolitaryVegetationObject",
        Some("SolitaryVegetationObject"),
        SOLITARY_VEGETATION_OBJECT,
        [
            hook(0, Module::Vegetation, "VegetationObject"),
            hook(
                SOLITARY_VEGETATION_OBJECT.len(),
                Module::Vegetation,
                "SolitaryVegetationObject"
            )
        ]
    ),
    class!(
        Vegetation,
        "PlantCover",
        Some("PlantCover"),
        PLANT_COVER,
        [
            hook(0, Module::Vegetation, "VegetationObject"),
            hook(PLANT_COVER.len(), Module::Vegetation, "PlantCover")
        ]
    ),
    class!(
        Generics,
        "GenericCityObject",
        Some("GenericCityObject"),
        GENERIC_CITY_OBJECT,
        []
    ),
    class!(
        CityObjectGroup,
        "CityObjectGroup",
        Some("CityObjectGroup"),
        CITY_OBJECT_GROUP,
        [hook(
            CITY_OBJECT_GROUP.len(),
            Module::CityObjectGroup,
            "CityObjectGroup"
        )]
    ),
    class!(
        Transportation,
        "Road",
        Some("Road"),
        TRANSPORTATION_COMPLEX,
        [
            hook(0, Module::Transportation, "TransportationObject"),
            hook(
                TRANSPORTATION_COMPLEX.len(),
                Module::Transportation,
                "TransportationComplex"
            ),
            hook(TRANSPORTATION_COMPLEX.len(), Module::Transportation, "Road")
        ]
    ),
    class!(
        Transportation,
        "Railway",
        Some("Railway"),
        TRANSPORTATION_COMPLEX,
        [
            hook(0, Module::Transportation, "TransportationObject"),
            hook(
                TRANSPORTATION_COMPLEX.len(),
                Module::Transportation,
                "TransportationComplex"
            ),
            hook(
                TRANSPORTATION_COMPLEX.len(),
                Module::Transportation,
                "Railway"
            )
        ]
    ),
    class!(
        Transportation,
        "Track",
        None,
        TRANSPORTATION_COMPLEX,
        [
            hook(0, Module::Transportation, "TransportationObject"),
            hook(
                TRANSPORTATION_COMPLEX.len(),
                Module::Transportation,
                "TransportationComplex"
            ),
            hook(
                TRANSPORTATION_COMPLEX.len(),
                Module::Transportation,
                "Track"
            )
        ]
    ),
    class!(
        Transportation,
        "Square",
        Some("TransportSquare"),
        TRANSPORTATION_COMPLEX,
        [
            hook(0, Module::Transportation, "TransportationObject"),
            hook(
                TRANSPORTATION_COMPLEX.len(),
                Module::Transportation,
                "TransportationComplex"
            ),
            hook(
                TRANSPORTATION_COMPLEX.len(),
                Module::Transportation,
                "Square"
            )
        ]
    ),
    class!(
        Transportation,
        "TransportationComplex",
        None,
        TRANSPORTATION_COMPLEX,
        [
            hook(0, Module::Transportation, "TransportationObject"),
            hook(
                TRANSPORTATION_COMPLEX.len(),
                Module::Transportation,
                "TransportationComplex"
            )
        ]
    ),
    class!(
        Relief,
        "TINRelief",
        Some("TINRelief"),
        TIN_RELIEF,
        [
            hook(1, Module::Relief, "ReliefComponent"),
            hook(2, Module::Relief, "TinRelief")
        ]
    ),
];

/// The boundary surface classes, by the modules they belong to, and the
/// property of their owner that holds them.
pub(crate) const SURFACES: &[(Module, &str, &str, &[Slot])] = &[
    (
        Module::Building,
        "WallSurface",
        "boundedBy",
        BOUNDARY_SURFACE,
    ),
    (
        Module::Building,
        "RoofSurface",
        "boundedBy",
        BOUNDARY_SURFACE,
    ),
    (
        Module::Building,
        "GroundSurface",
        "boundedBy",
        BOUNDARY_SURFACE,
    ),
    (
        Module::Building,
        "ClosureSurface",
        "boundedBy",
        BOUNDARY_SURFACE,
    ),
    (
        Module::Building,
        "OuterCeilingSurface",
        "boundedBy",
        BOUNDARY_SURFACE,
    ),
    (
        Module::Building,
        "OuterFloorSurface",
        "boundedBy",
        BOUNDARY_SURFACE,
    ),
    (
        Module::Building,
        "CeilingSurface",
        "boundedBy",
        BOUNDARY_SURFACE,
    ),
    (
        Module::Building,
        "FloorSurface",
        "boundedBy",
        BOUNDARY_SURFACE,
    ),
    (
        Module::Building,
        "InteriorWallSurface",
        "boundedBy",
        BOUNDARY_SURFACE,
    ),
    (Module::Building, "Door", "opening", DOOR),
    (Module::Building, "Window", "opening", OPENING),
    (Module::Bridge, "WallSurface", "boundedBy", BOUNDARY_SURFACE),
    (Module::Bridge, "RoofSurface", "boundedBy", BOUNDARY_SURFACE),
    (
        Module::Bridge,
        "GroundSurface",
        "boundedBy",
        BOUNDARY_SURFACE,
    ),
    (
        Module::Bridge,
        "ClosureSurface",
        "boundedBy",
        BOUNDARY_SURFACE,
    ),
    (
        Module::Bridge,
        "OuterCeilingSurface",
        "boundedBy",
        BOUNDARY_SURFACE,
    ),
    (
        Module::Bridge,
        "OuterFloorSurface",
        "boundedBy",
        BOUNDARY_SURFACE,
    ),
    (
        Module::Bridge,
        "CeilingSurface",
        "boundedBy",
        BOUNDARY_SURFACE,
    ),
    (
        Module::Bridge,
        "FloorSurface",
        "boundedBy",
        BOUNDARY_SURFACE,
    ),
    (
        Module::Bridge,
        "InteriorWallSurface",
        "boundedBy",
        BOUNDARY_SURFACE,
    ),
    (Module::Bridge, "Door", "opening", DOOR),
    (Module::Bridge, "Window", "opening", OPENING),
    (Module::Tunnel, "WallSurface", "boundedBy", BOUNDARY_SURFACE),
    (Module::Tunnel, "RoofSurface", "boundedBy", BOUNDARY_SURFACE),
    (
        Module::Tunnel,
        "GroundSurface",
        "boundedBy",
        BOUNDARY_SURFACE,
    ),
    (
        Module::Tunnel,
        "ClosureSurface",
        "boundedBy",
        BOUNDARY_SURFACE,
    ),
    (
        Module::Tunnel,
        "OuterCeilingSurface",
        "boundedBy",
        BOUNDARY_SURFACE,
    ),
    (
        Module::Tunnel,
        "OuterFloorSurface",
        "boundedBy",
        BOUNDARY_SURFACE,
    ),
    (
        Module::Tunnel,
        "CeilingSurface",
        "boundedBy",
        BOUNDARY_SURFACE,
    ),
    (
        Module::Tunnel,
        "FloorSurface",
        "boundedBy",
        BOUNDARY_SURFACE,
    ),
    (
        Module::Tunnel,
        "InteriorWallSurface",
        "boundedBy",
        BOUNDARY_SURFACE,
    ),
    (Module::Tunnel, "Door", "opening", OPENING),
    (Module::Tunnel, "Window", "opening", OPENING),
    (
        Module::WaterBody,
        "WaterSurface",
        "boundedBy",
        WATER_SURFACE,
    ),
    (
        Module::WaterBody,
        "WaterGroundSurface",
        "boundedBy",
        WATER_BOUNDARY_SURFACE,
    ),
    (
        Module::WaterBody,
        "WaterClosureSurface",
        "boundedBy",
        WATER_BOUNDARY_SURFACE,
    ),
    (
        Module::Transportation,
        "TrafficArea",
        "trafficArea",
        TRAFFIC_AREA,
    ),
    (
        Module::Transportation,
        "AuxiliaryTrafficArea",
        "auxiliaryTrafficArea",
        TRAFFIC_AREA,
    ),
];

/// The class `class` names, if this writer writes it.
pub(crate) fn class_def(class: &QName) -> Option<&'static ClassDef> {
    let Namespace::Module(module) = class.ns else {
        return None;
    };
    CLASSES
        .iter()
        .find(|c| c.module == module && c.name == class.local)
}

/// The boundary surface class `class` names: the property of its owner (or
/// of the surface it is an opening of) that holds it, and its content.
pub(crate) fn surface_def(class: &QName) -> Option<(&'static str, &'static [Slot])> {
    let Namespace::Module(module) = class.ns else {
        return None;
    };
    SURFACES
        .iter()
        .find(|(m, name, _, _)| *m == module && *name == class.local)
        .map(|(_, _, property, content)| (*property, *content))
}

/// The properties every city object has before its class's own, as far as
/// they hold a value: `gml:description`, `gml:name`, then `_CityObject`'s.
const RELATIVE_TO_TERRAIN: &[&str] = &[
    "entirelyAboveTerrain",
    "substantiallyAboveTerrain",
    "substantiallyAboveAndBelowTerrain",
    "substantiallyBelowTerrain",
    "entirelyBelowTerrain",
];
const RELATIVE_TO_WATER: &[&str] = &[
    "entirelyAboveWaterSurface",
    "substantiallyAboveWaterSurface",
    "substantiallyAboveAndBelowWaterSurface",
    "substantiallyBelowWaterSurface",
    "entirelyBelowWaterSurface",
    "temporarilyAboveAndBelowWaterSurface",
];

pub(crate) const COMMON: &[Property] = &[
    Property {
        name: "description",
        ns: PropertyNs::Gml,
        kind: Text,
        repeated: false,
    },
    Property {
        name: "name",
        ns: PropertyNs::Gml,
        kind: Text,
        repeated: true,
    },
    Property {
        name: "creationDate",
        ns: PropertyNs::Module(Module::Core),
        kind: PropertyKind::Date,
        repeated: false,
    },
    Property {
        name: "terminationDate",
        ns: PropertyNs::Module(Module::Core),
        kind: PropertyKind::Date,
        repeated: false,
    },
    Property {
        name: "relativeToTerrain",
        ns: PropertyNs::Module(Module::Core),
        kind: PropertyKind::Enumeration(RELATIVE_TO_TERRAIN),
        repeated: false,
    },
    Property {
        name: "relativeToWater",
        ns: PropertyNs::Module(Module::Core),
        kind: PropertyKind::Enumeration(RELATIVE_TO_WATER),
        repeated: false,
    },
];

/// The installations that are inside or outside their construction by
/// class, which CityJSON says with `relationToConstruction`.
const INSTALLATIONS: &[(&str, &str, &str)] = &[
    (
        "BuildingInstallation",
        "BuildingInstallation",
        "IntBuildingInstallation",
    ),
    (
        "BridgeInstallation",
        "BridgeInstallation",
        "IntBridgeInstallation",
    ),
    (
        "TunnelInstallation",
        "TunnelInstallation",
        "IntTunnelInstallation",
    ),
];

/// CityGML 2.0's names for CityJSON's.
pub struct Vocabulary2;

impl Vocabulary for Vocabulary2 {
    fn class(
        &self,
        cityjson_type: &str,
        attribute: &dyn Fn(&str) -> Option<String>,
    ) -> Option<Class> {
        let def = CLASSES.iter().find(|c| c.cityjson == Some(cityjson_type))?;
        if let Some((_, _, interior)) = INSTALLATIONS.iter().find(|(t, _, _)| *t == cityjson_type) {
            let inside = attribute("relationToConstruction").as_deref() == Some("inside");
            let name: &str = if inside { interior } else { def.name };
            return Some(Class {
                name: QName::module(def.module, name),
                implied: &["relationToConstruction"],
            });
        }
        Some(Class {
            name: QName::module(def.module, def.name),
            implied: &[],
        })
    }

    fn surface(&self, surface: &str, owner: &QName) -> Option<QName> {
        let def = class_def(owner)?;
        let candidate = QName::module(def.module, surface);
        let (property, _) = surface_def(&candidate)?;
        // An opening belongs in a surface of an object that has them.
        let holder = if property == "opening" {
            "boundedBy"
        } else {
            property
        };
        def.content.contains(&Surfaces(holder)).then_some(candidate)
    }

    fn property(&self, class: &QName, name: &str) -> Option<Property> {
        if let Some(p) = COMMON.iter().find(|p| p.name == name) {
            return Some(*p);
        }
        let Namespace::Module(module) = class.ns else {
            return None;
        };
        let content = match class_def(class) {
            Some(def) => def.content,
            None => surface_def(class)?.1,
        };
        content.iter().find_map(|s| match s {
            Value(n, kind, repeated) if *n == name => Some(Property {
                name: n,
                ns: PropertyNs::Module(module),
                kind: *kind,
                repeated: *repeated,
            }),
            _ => None,
        })
    }
}

#[cfg(test)]
mod tests {
    //! Every content list against the official CityGML 2.0 schemas
    //! (`tests/data/schemas/citygml/2.0`): each property a list names is
    //! one the class's content model declares, in the same order, and each
    //! the content model declares in the class's own module is listed; and
    //! so with a class's hooks.

    use std::collections::HashMap;
    use std::path::PathBuf;

    use quick_xml::events::Event;
    use quick_xml::reader::NsReader;

    use super::*;
    use crate::ns;

    type Name = (String, String);

    #[derive(Default)]
    struct Schemas {
        /// Global element -> its type.
        elements: HashMap<Name, Name>,
        /// Complex type -> its base and the elements its sequence declares,
        /// in order.
        types: HashMap<Name, (Option<Name>, Vec<Name>)>,
    }

    fn resolve(qname: &str, prefixes: &HashMap<String, String>, target: &str) -> Name {
        match qname.split_once(':') {
            Some((p, l)) => (prefixes[p].clone(), l.to_string()),
            None => (target.to_string(), qname.to_string()),
        }
    }

    fn read() -> Schemas {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/schemas/citygml/2.0");
        let mut schemas = Schemas::default();
        for entry in std::fs::read_dir(dir).unwrap() {
            let text = std::fs::read_to_string(entry.unwrap().path()).unwrap();
            let mut reader = NsReader::from_str(&text);
            let mut prefixes = HashMap::new();
            let mut target = String::new();
            // The complex type being read, its base and its elements.
            let mut current: Option<(Name, Option<Name>, Vec<Name>)> = None;
            let mut depth_in_type = 0usize;
            loop {
                let event = reader.read_event().unwrap();
                let (e, empty) = match &event {
                    Event::Start(e) => (e.clone(), false),
                    Event::Empty(e) => (e.clone(), true),
                    Event::End(_) => {
                        if current.is_some() {
                            depth_in_type -= 1;
                            if depth_in_type == 0 {
                                let (name, base, seq) = current.take().unwrap();
                                schemas.types.insert(name, (base, seq));
                            }
                        }
                        continue;
                    }
                    Event::Eof => break,
                    _ => continue,
                };
                let local = String::from_utf8(e.local_name().as_ref().to_vec()).unwrap();
                let attr = |n: &str| {
                    e.attributes()
                        .flatten()
                        .find(|a| a.key.as_ref() == n.as_bytes())
                        .map(|a| String::from_utf8(a.value.to_vec()).unwrap())
                };
                if local == "schema" {
                    target = attr("targetNamespace").unwrap();
                    for a in e.attributes().flatten() {
                        let key = String::from_utf8(a.key.as_ref().to_vec()).unwrap();
                        let value = String::from_utf8(a.value.to_vec()).unwrap();
                        if let Some(p) = key.strip_prefix("xmlns:") {
                            prefixes.insert(p.to_string(), value);
                        }
                    }
                    continue;
                }
                if let Some((_, base, seq)) = current.as_mut() {
                    if !empty {
                        depth_in_type += 1;
                    }
                    match local.as_str() {
                        "extension" => {
                            *base = attr("base").map(|b| resolve(&b, &prefixes, &target))
                        }
                        "element" => seq.push(match attr("ref") {
                            Some(r) => resolve(&r, &prefixes, &target),
                            None => (target.clone(), attr("name").unwrap()),
                        }),
                        _ => {}
                    }
                    continue;
                }
                match local.as_str() {
                    "complexType" if !empty => {
                        if let Some(name) = attr("name") {
                            current = Some(((target.clone(), name), None, Vec::new()));
                            depth_in_type = 1;
                        }
                    }
                    "element" => {
                        if let (Some(name), Some(ty)) = (attr("name"), attr("type")) {
                            schemas
                                .elements
                                .insert((target.clone(), name), resolve(&ty, &prefixes, &target));
                        }
                    }
                    _ => {}
                }
            }
        }
        schemas
    }

    impl Schemas {
        /// The elements a type's content declares, its bases' first.
        fn sequence(&self, ty: &Name) -> Vec<Name> {
            let Some((base, seq)) = self.types.get(ty) else {
                return Vec::new();
            };
            let mut out = base.as_ref().map(|b| self.sequence(b)).unwrap_or_default();
            out.extend(seq.iter().cloned());
            out
        }

        fn content(&self, module: Module, class: &str) -> Vec<Name> {
            let ns = ns::uri(module).unwrap().to_string();
            let ty = self
                .elements
                .get(&(ns, class.to_string()))
                .unwrap_or_else(|| panic!("{class} is not an element of the {module:?} schema"));
            self.sequence(ty)
        }
    }

    fn slot_name(slot: &Slot) -> &'static str {
        match slot {
            Value(n, ..) | Geometry(n) | Surfaces(n) | Nest(n, _) | Address(n) => n,
            Member => "groupMember",
            Opening => "opening",
            ReliefLod => "lod",
            Tin => "tin",
        }
    }

    /// The declared elements of the class's module a writer has no use for.
    const UNUSED: &[&str] = &["parent", "extent"];

    /// `content`, with `hooks` where they are, by namespace URI and local
    /// name.
    fn listed(module: Module, content: &[Slot], hooks: &[Hook]) -> Vec<Name> {
        let ns = ns::uri(module).unwrap().to_string();
        let hooks_at = |at: usize| {
            hooks
                .iter()
                .filter(move |h| h.at == at)
                .map(|h| (ns::uri(h.module).unwrap().to_string(), h.element()))
        };
        let mut out = Vec::new();
        for (at, slot) in content.iter().enumerate() {
            out.extend(hooks_at(at));
            out.push((ns.clone(), slot_name(slot).to_string()));
        }
        out.extend(hooks_at(content.len()));
        out
    }

    /// The class's content and hooks against what its content model
    /// declares. `hooks` is `None` for a boundary surface, which holds no
    /// ADE property.
    fn check(
        schemas: &Schemas,
        module: Module,
        class: &str,
        content: &[Slot],
        hooks: Option<&[Hook]>,
    ) {
        let declared = schemas.content(module, class);
        let ns = ns::uri(module).unwrap();
        let listed = listed(module, content, hooks.unwrap_or_default());
        let mut at = 0;
        for name in &listed {
            let found = declared[at..]
                .iter()
                .position(|d| d == name)
                .unwrap_or_else(|| {
                    panic!("{class}: {name:?} is not declared after position {at}: {declared:?}")
                });
            at += found + 1;
        }
        for name in &declared {
            let (n, local) = name;
            let hook = local.starts_with("_GenericApplicationPropertyOf");
            let wanted = if hook {
                hooks.is_some() && local != "_GenericApplicationPropertyOfCityObject"
            } else {
                n == ns && !UNUSED.contains(&local.as_str())
            };
            assert!(
                !wanted || listed.contains(name),
                "{class}: {local} is declared but not listed"
            );
        }
    }

    #[test]
    fn every_class_lists_its_content_in_schema_order() {
        let schemas = read();
        for def in CLASSES {
            check(&schemas, def.module, def.name, def.content, Some(def.hooks));
        }
        for (module, name, _, content) in SURFACES {
            check(&schemas, *module, name, content, None);
        }
    }

    #[test]
    fn the_common_properties_are_cityobject_s() {
        let schemas = read();
        let declared =
            schemas.sequence(&(ns::CORE.to_string(), "AbstractCityObjectType".to_string()));
        let positions: Vec<usize> = COMMON
            .iter()
            .filter(|p| p.ns != PropertyNs::Gml)
            .map(|p| {
                declared
                    .iter()
                    .position(|(n, l)| n == ns::CORE && l == p.name)
                    .unwrap_or_else(|| panic!("core:{} is not declared", p.name))
            })
            .collect();
        assert!(positions.windows(2).all(|w| w[0] < w[1]), "{positions:?}");
    }
}
