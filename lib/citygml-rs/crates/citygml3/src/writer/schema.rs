//! What the CityGML 3.0 schemas say a writer needs, as data: each class's
//! properties in the order its content model puts them, and the CityJSON
//! names each class and boundary surface is written for.
//!
//! A CityGML 3.0 class's content is a chain of levels, one per type along
//! its line of inheritance below `core:AbstractCityObject`
//! (`core:AbstractSpace`, `con:AbstractConstruction`,
//! `bldg:AbstractBuilding`, `bldg:Building`), each in its own module and
//! each ending in the `adeOf…` hook an ADE's properties of that type go in.
//! A level lists only what a writer writes. The properties every city
//! object has (`gml:description`, `gml:name`, `core:creationDate`, …,
//! generic attributes and appearance) come before the first level. The
//! tests check every level against the official schemas.

use citygml_core::convert::{Class, Property, PropertyKind, PropertyNs, Vocabulary};
use citygml_core::model::{Module, Namespace, QName};

/// One property of a level's content.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Slot {
    /// A property holding a value: name, what it holds, whether it repeats.
    Value(&'static str, PropertyKind, bool),
    /// A geometry property, `lod<N><Kind>`.
    Geometry(&'static str),
    /// `core:boundary`, holding the object's boundary surfaces.
    Boundary,
    /// A property holding boundary surfaces directly (`tran:marking`).
    Surfaces(&'static str),
    /// A property holding a space of the given class whose boundary
    /// surfaces are the object's (`tran:trafficSpace`); the space states
    /// its `granularity` when the flag says it must.
    Spaces(&'static str, &'static str, bool),
    /// A property nesting sub-features of the given classes.
    Nest(&'static str, &'static [&'static str]),
    /// A postal address.
    Address(&'static str),
    /// A group's `groupMember`.
    Member,
    /// A construction surface's `con:fillingSurface`, holding doors and
    /// windows.
    Filling,
    /// A construction's `con:height`, which a measured height is written
    /// to.
    Height,
    /// A relief component's `lod`, which a TIN's LoD is written to.
    ReliefLod,
    /// A `TINRelief`'s `tin`.
    Tin,
}

impl Slot {
    /// The property holding boundary surfaces this slot is, by the name a
    /// surface's holder has.
    pub fn holder(&self) -> Option<&'static str> {
        match self {
            Slot::Boundary => Some("boundary"),
            Slot::Filling => Some("fillingSurface"),
            Slot::Surfaces(name) | Slot::Spaces(name, ..) => Some(name),
            _ => None,
        }
    }
}

/// One type of a class's line of inheritance: its module, the class whose
/// `adeOf<class>` hook ends it, and what it declares before that hook.
#[derive(Debug)]
pub(crate) struct Level {
    pub module: Module,
    pub class: &'static str,
    pub slots: &'static [Slot],
}

impl Level {
    /// The hook element's name.
    pub fn hook(&self) -> String {
        format!("adeOf{}", self.class)
    }
}

/// A class this writer writes.
#[derive(Debug)]
pub(crate) struct ClassDef {
    pub module: Module,
    pub name: &'static str,
    /// The CityJSON City Object type written as this class, if any.
    pub cityjson: Option<&'static str>,
    pub levels: &'static [&'static Level],
}

impl ClassDef {
    /// Every slot of every level, with its level.
    pub fn slots(&self) -> impl Iterator<Item = (&'static Level, &'static Slot)> {
        self.levels
            .iter()
            .flat_map(|level| level.slots.iter().map(move |s| (*level, s)))
    }
}

/// A boundary surface class this writer writes.
#[derive(Debug)]
pub(crate) struct SurfaceDef {
    pub module: Module,
    pub name: &'static str,
    /// The CityJSON semantic surface types written as this class.
    pub cityjson: &'static [&'static str],
    /// The property of its owner, or of the surface it fills, that holds it.
    pub holder: &'static str,
    pub levels: &'static [&'static Level],
}

use PropertyKind::{Boolean, Date, Integer, MeasureList, Number, Text};
use Slot::{
    Address, Boundary, Filling, Geometry, Height, Member, Nest, ReliefLod, Spaces, Surfaces, Tin,
    Value,
};

const CODE: PropertyKind = Text;
const LENGTH: PropertyKind = Number { measure: true };

const SPACE_TYPE: &[&str] = &["closed", "open", "semiOpen"];
const CONDITION: &[&str] = &[
    "declined",
    "demolished",
    "functional",
    "projected",
    "ruin",
    "underConstruction",
];
const RELATION: &[&str] = &["inside", "outside", "both"];
const TRAFFIC_DIRECTION: &[&str] = &["forwards", "backwards", "both"];

/// `class`, `function` and `usage`, which most classes begin with.
const CFU: [Slot; 3] = [
    Value("class", CODE, false),
    Value("function", CODE, true),
    Value("usage", CODE, true),
];

macro_rules! level {
    ($name:ident, $module:ident, $class:literal, [$($slot:expr),* $(,)?]) => {
        const $name: Level = Level {
            module: Module::$module,
            class: $class,
            slots: &[$($slot),*],
        };
    };
}

/// A level that holds only `class`, `function` and `usage`.
macro_rules! cfu_level {
    ($name:ident, $module:ident, $class:literal) => {
        const $name: Level = Level {
            module: Module::$module,
            class: $class,
            slots: &CFU,
        };
    };
}

// The core module's. What every city object holds before its class's
// own (the `COMMON` properties, appearance and generic attributes) the
// writer writes itself; these three levels give their hooks.
level!(FEATURE, Core, "AbstractFeature", []);
level!(
    FEATURE_WITH_LIFESPAN,
    Core,
    "AbstractFeatureWithLifespan",
    []
);
level!(CITY_OBJECT, Core, "AbstractCityObject", []);
level!(
    SPACE,
    Core,
    "AbstractSpace",
    [
        Value("spaceType", PropertyKind::Enumeration(SPACE_TYPE), false),
        Boundary,
        Geometry("lod0Point"),
        Geometry("lod0MultiSurface"),
        Geometry("lod0MultiCurve"),
        Geometry("lod1Solid"),
        Geometry("lod2Solid"),
        Geometry("lod2MultiSurface"),
        Geometry("lod2MultiCurve"),
        Geometry("lod3Solid"),
        Geometry("lod3MultiSurface"),
        Geometry("lod3MultiCurve"),
    ]
);
level!(
    PHYSICAL_SPACE,
    Core,
    "AbstractPhysicalSpace",
    [
        Geometry("lod1TerrainIntersectionCurve"),
        Geometry("lod2TerrainIntersectionCurve"),
        Geometry("lod3TerrainIntersectionCurve"),
    ]
);
level!(
    OCCUPIED_SPACE,
    Core,
    "AbstractOccupiedSpace",
    [
        Geometry("lod1ImplicitRepresentation"),
        Geometry("lod2ImplicitRepresentation"),
        Geometry("lod3ImplicitRepresentation"),
    ]
);
level!(UNOCCUPIED_SPACE, Core, "AbstractUnoccupiedSpace", []);
level!(LOGICAL_SPACE, Core, "AbstractLogicalSpace", []);
level!(SPACE_BOUNDARY, Core, "AbstractSpaceBoundary", []);
level!(
    THEMATIC_SURFACE,
    Core,
    "AbstractThematicSurface",
    [
        Geometry("lod0MultiCurve"),
        Geometry("lod0MultiSurface"),
        Geometry("lod1MultiSurface"),
        Geometry("lod2MultiSurface"),
        Geometry("lod3MultiSurface"),
    ]
);
level!(CLOSURE_SURFACE, Core, "ClosureSurface", []);

// The construction module's.
level!(
    CONSTRUCTION,
    Construction,
    "AbstractConstruction",
    [
        Value(
            "conditionOfConstruction",
            PropertyKind::Enumeration(CONDITION),
            false
        ),
        Value("dateOfConstruction", Date, false),
        Value("dateOfDemolition", Date, false),
        Height,
    ]
);
level!(
    INSTALLATION,
    Construction,
    "AbstractInstallation",
    [Value(
        "relationToConstruction",
        PropertyKind::Enumeration(RELATION),
        false
    )]
);
level!(FURNITURE, Construction, "AbstractFurniture", []);
level!(
    CONSTRUCTIVE_ELEMENT,
    Construction,
    "AbstractConstructiveElement",
    [Value("isStructuralElement", Boolean, false)]
);
cfu_level!(OTHER_CONSTRUCTION, Construction, "OtherConstruction");
level!(
    CONSTRUCTION_SURFACE,
    Construction,
    "AbstractConstructionSurface",
    [Filling]
);
level!(FILLING_SURFACE, Construction, "AbstractFillingSurface", []);
level!(WALL_SURFACE, Construction, "WallSurface", []);
level!(ROOF_SURFACE, Construction, "RoofSurface", []);
level!(GROUND_SURFACE, Construction, "GroundSurface", []);
level!(
    OUTER_CEILING_SURFACE,
    Construction,
    "OuterCeilingSurface",
    []
);
level!(OUTER_FLOOR_SURFACE, Construction, "OuterFloorSurface", []);
level!(CEILING_SURFACE, Construction, "CeilingSurface", []);
level!(FLOOR_SURFACE, Construction, "FloorSurface", []);
level!(
    INTERIOR_WALL_SURFACE,
    Construction,
    "InteriorWallSurface",
    []
);
// A door's addresses: the model holds none on a boundary surface, so the
// writer writes none, and the slot only states the content model.
level!(
    DOOR_SURFACE,
    Construction,
    "DoorSurface",
    [Address("address")]
);
level!(WINDOW_SURFACE, Construction, "WindowSurface", []);

// The building module's.
level!(
    ABSTRACT_BUILDING,
    Building,
    "AbstractBuilding",
    [
        CFU[0],
        CFU[1],
        CFU[2],
        Value("roofType", CODE, false),
        Value(
            "storeysAboveGround",
            Integer {
                non_negative: false
            },
            false
        ),
        Value(
            "storeysBelowGround",
            Integer {
                non_negative: false
            },
            false
        ),
        Value("storeyHeightsAboveGround", MeasureList, false),
        Value("storeyHeightsBelowGround", MeasureList, false),
        Nest(
            "buildingConstructiveElement",
            &["BuildingConstructiveElement"]
        ),
        Nest("buildingInstallation", &["BuildingInstallation"]),
        Nest("buildingRoom", &["BuildingRoom"]),
        Nest("buildingFurniture", &["BuildingFurniture"]),
        Nest("buildingSubdivision", &["Storey", "BuildingUnit"]),
        Address("address"),
    ]
);
level!(
    BUILDING,
    Building,
    "Building",
    [Nest("buildingPart", &["BuildingPart"])]
);
level!(BUILDING_PART, Building, "BuildingPart", []);
cfu_level!(BUILDING_INSTALLATION, Building, "BuildingInstallation");
level!(
    BUILDING_ROOM,
    Building,
    "BuildingRoom",
    [
        CFU[0],
        CFU[1],
        CFU[2],
        Nest("buildingFurniture", &["BuildingFurniture"]),
        Nest("buildingInstallation", &["BuildingInstallation"]),
    ]
);
cfu_level!(BUILDING_FURNITURE, Building, "BuildingFurniture");
cfu_level!(
    BUILDING_CONSTRUCTIVE_ELEMENT,
    Building,
    "BuildingConstructiveElement"
);
level!(
    BUILDING_SUBDIVISION,
    Building,
    "AbstractBuildingSubdivision",
    [
        CFU[0],
        CFU[1],
        CFU[2],
        Value("sortKey", Number { measure: false }, false),
        Nest(
            "buildingConstructiveElement",
            &["BuildingConstructiveElement"]
        ),
        Nest("buildingFurniture", &["BuildingFurniture"]),
        Nest("buildingInstallation", &["BuildingInstallation"]),
        Nest("buildingRoom", &["BuildingRoom"]),
    ]
);
level!(
    STOREY,
    Building,
    "Storey",
    [Nest("buildingUnit", &["BuildingUnit"])]
);
level!(
    BUILDING_UNIT,
    Building,
    "BuildingUnit",
    [Nest("storey", &["Storey"]), Address("address")]
);

// The bridge module's.
level!(
    ABSTRACT_BRIDGE,
    Bridge,
    "AbstractBridge",
    [
        CFU[0],
        CFU[1],
        CFU[2],
        Value("isMovable", Boolean, false),
        Nest("bridgeConstructiveElement", &["BridgeConstructiveElement"]),
        Nest("bridgeInstallation", &["BridgeInstallation"]),
        Nest("bridgeRoom", &["BridgeRoom"]),
        Nest("bridgeFurniture", &["BridgeFurniture"]),
        Address("address"),
    ]
);
level!(
    BRIDGE,
    Bridge,
    "Bridge",
    [Nest("bridgePart", &["BridgePart"])]
);
level!(BRIDGE_PART, Bridge, "BridgePart", []);
cfu_level!(BRIDGE_INSTALLATION, Bridge, "BridgeInstallation");
level!(
    BRIDGE_ROOM,
    Bridge,
    "BridgeRoom",
    [
        CFU[0],
        CFU[1],
        CFU[2],
        Nest("bridgeFurniture", &["BridgeFurniture"]),
        Nest("bridgeInstallation", &["BridgeInstallation"]),
    ]
);
cfu_level!(BRIDGE_FURNITURE, Bridge, "BridgeFurniture");
cfu_level!(
    BRIDGE_CONSTRUCTIVE_ELEMENT,
    Bridge,
    "BridgeConstructiveElement"
);

// The tunnel module's.
level!(
    ABSTRACT_TUNNEL,
    Tunnel,
    "AbstractTunnel",
    [
        CFU[0],
        CFU[1],
        CFU[2],
        Nest("tunnelConstructiveElement", &["TunnelConstructiveElement"]),
        Nest("tunnelInstallation", &["TunnelInstallation"]),
        Nest("hollowSpace", &["HollowSpace"]),
        Nest("tunnelFurniture", &["TunnelFurniture"]),
    ]
);
level!(
    TUNNEL,
    Tunnel,
    "Tunnel",
    [Nest("tunnelPart", &["TunnelPart"])]
);
level!(TUNNEL_PART, Tunnel, "TunnelPart", []);
cfu_level!(TUNNEL_INSTALLATION, Tunnel, "TunnelInstallation");
level!(
    HOLLOW_SPACE,
    Tunnel,
    "HollowSpace",
    [
        CFU[0],
        CFU[1],
        CFU[2],
        Nest("tunnelFurniture", &["TunnelFurniture"]),
        Nest("tunnelInstallation", &["TunnelInstallation"]),
    ]
);
cfu_level!(TUNNEL_FURNITURE, Tunnel, "TunnelFurniture");
cfu_level!(
    TUNNEL_CONSTRUCTIVE_ELEMENT,
    Tunnel,
    "TunnelConstructiveElement"
);

// The other modules'.
cfu_level!(WATER_BODY, WaterBody, "WaterBody");
level!(
    WATER_BOUNDARY_SURFACE,
    WaterBody,
    "AbstractWaterBoundarySurface",
    []
);
level!(
    WATER_SURFACE,
    WaterBody,
    "WaterSurface",
    [Value("waterLevel", CODE, false)]
);
level!(WATER_GROUND_SURFACE, WaterBody, "WaterGroundSurface", []);
cfu_level!(LAND_USE, LandUse, "LandUse");
cfu_level!(CITY_FURNITURE, CityFurniture, "CityFurniture");
level!(
    VEGETATION_OBJECT,
    Vegetation,
    "AbstractVegetationObject",
    []
);
level!(
    SOLITARY_VEGETATION_OBJECT,
    Vegetation,
    "SolitaryVegetationObject",
    [
        CFU[0],
        CFU[1],
        CFU[2],
        Value("species", CODE, false),
        Value("height", LENGTH, false),
        Value("trunkDiameter", LENGTH, false),
        Value("crownDiameter", LENGTH, false),
        Value("rootBallDiameter", LENGTH, false),
        Value("maxRootBallDepth", LENGTH, false),
    ]
);
level!(
    PLANT_COVER,
    Vegetation,
    "PlantCover",
    [
        CFU[0],
        CFU[1],
        CFU[2],
        Value("averageHeight", LENGTH, false),
        Value("minHeight", LENGTH, false),
        Value("maxHeight", LENGTH, false),
    ]
);
cfu_level!(GENERIC_OCCUPIED_SPACE, Generics, "GenericOccupiedSpace");
cfu_level!(GENERIC_UNOCCUPIED_SPACE, Generics, "GenericUnoccupiedSpace");
cfu_level!(GENERIC_LOGICAL_SPACE, Generics, "GenericLogicalSpace");
level!(
    CITY_OBJECT_GROUP,
    CityObjectGroup,
    "CityObjectGroup",
    [CFU[0], CFU[1], CFU[2], Member]
);
level!(
    TRANSPORTATION_SPACE,
    Transportation,
    "AbstractTransportationSpace",
    [
        Value(
            "trafficDirection",
            PropertyKind::Enumeration(TRAFFIC_DIRECTION),
            false
        ),
        Spaces("trafficSpace", "TrafficSpace", true),
        Spaces("auxiliaryTrafficSpace", "AuxiliaryTrafficSpace", true),
        Spaces("hole", "Hole", false),
        Surfaces("marking"),
    ]
);
cfu_level!(ROAD, Transportation, "Road");
cfu_level!(RAILWAY, Transportation, "Railway");
cfu_level!(TRACK, Transportation, "Track");
cfu_level!(SQUARE, Transportation, "Square");
cfu_level!(WATERWAY, Transportation, "Waterway");
level!(
    TRAFFIC_AREA,
    Transportation,
    "TrafficArea",
    [
        CFU[0],
        CFU[1],
        CFU[2],
        Value("surfaceMaterial", CODE, false)
    ]
);
level!(
    AUXILIARY_TRAFFIC_AREA,
    Transportation,
    "AuxiliaryTrafficArea",
    [
        CFU[0],
        CFU[1],
        CFU[2],
        Value("surfaceMaterial", CODE, false)
    ]
);
level!(
    MARKING,
    Transportation,
    "Marking",
    [Value("class", CODE, false)]
);
level!(HOLE_SURFACE, Transportation, "HoleSurface", []);
level!(
    RELIEF_COMPONENT,
    Relief,
    "AbstractReliefComponent",
    [ReliefLod]
);
level!(TIN_RELIEF, Relief, "TINRelief", [Tin]);

/// The levels every space shares, then those of an occupied, unoccupied
/// or logical one.
const OCCUPIED: [&Level; 3] = [&SPACE, &PHYSICAL_SPACE, &OCCUPIED_SPACE];
const UNOCCUPIED: [&Level; 3] = [&SPACE, &PHYSICAL_SPACE, &UNOCCUPIED_SPACE];
const LOGICAL: [&Level; 2] = [&SPACE, &LOGICAL_SPACE];
const THEMATIC: [&Level; 2] = [&SPACE_BOUNDARY, &THEMATIC_SURFACE];

/// `prefix`'s levels followed by `rest`.
macro_rules! levels {
    ($prefix:expr, [$($rest:expr),* $(,)?]) => {{
        const P: &[&Level] = &$prefix;
        const R: &[&Level] = &[$($rest),*];
        const N: usize = P.len() + R.len();
        const L: [&Level; N] = {
            let mut out: [&Level; N] = [&SPACE; N];
            let mut i = 0;
            while i < P.len() {
                out[i] = P[i];
                i += 1;
            }
            let mut j = 0;
            while j < R.len() {
                out[P.len() + j] = R[j];
                j += 1;
            }
            out
        };
        &L
    }};
}

macro_rules! class {
    ($module:ident, $name:literal, $cityjson:expr, $levels:expr) => {
        ClassDef {
            module: Module::$module,
            name: $name,
            cityjson: $cityjson,
            levels: $levels,
        }
    };
}

/// Every city object class this writer writes. Where several share a
/// CityJSON type, the first is the one written.
pub(crate) const CLASSES: &[ClassDef] = &[
    class!(
        Building,
        "Building",
        Some("Building"),
        levels!(OCCUPIED, [&CONSTRUCTION, &ABSTRACT_BUILDING, &BUILDING])
    ),
    class!(
        Building,
        "BuildingPart",
        Some("BuildingPart"),
        levels!(
            OCCUPIED,
            [&CONSTRUCTION, &ABSTRACT_BUILDING, &BUILDING_PART]
        )
    ),
    class!(
        Building,
        "BuildingInstallation",
        Some("BuildingInstallation"),
        levels!(OCCUPIED, [&INSTALLATION, &BUILDING_INSTALLATION])
    ),
    class!(
        Building,
        "BuildingRoom",
        Some("BuildingRoom"),
        levels!(UNOCCUPIED, [&BUILDING_ROOM])
    ),
    class!(
        Building,
        "BuildingFurniture",
        Some("BuildingFurniture"),
        levels!(OCCUPIED, [&FURNITURE, &BUILDING_FURNITURE])
    ),
    class!(
        Building,
        "BuildingConstructiveElement",
        Some("BuildingConstructiveElement"),
        levels!(
            OCCUPIED,
            [&CONSTRUCTIVE_ELEMENT, &BUILDING_CONSTRUCTIVE_ELEMENT]
        )
    ),
    class!(
        Building,
        "Storey",
        Some("BuildingStorey"),
        levels!(LOGICAL, [&BUILDING_SUBDIVISION, &STOREY])
    ),
    class!(
        Building,
        "BuildingUnit",
        Some("BuildingUnit"),
        levels!(LOGICAL, [&BUILDING_SUBDIVISION, &BUILDING_UNIT])
    ),
    class!(
        Bridge,
        "Bridge",
        Some("Bridge"),
        levels!(OCCUPIED, [&CONSTRUCTION, &ABSTRACT_BRIDGE, &BRIDGE])
    ),
    class!(
        Bridge,
        "BridgePart",
        Some("BridgePart"),
        levels!(OCCUPIED, [&CONSTRUCTION, &ABSTRACT_BRIDGE, &BRIDGE_PART])
    ),
    class!(
        Bridge,
        "BridgeInstallation",
        Some("BridgeInstallation"),
        levels!(OCCUPIED, [&INSTALLATION, &BRIDGE_INSTALLATION])
    ),
    class!(
        Bridge,
        "BridgeRoom",
        Some("BridgeRoom"),
        levels!(UNOCCUPIED, [&BRIDGE_ROOM])
    ),
    class!(
        Bridge,
        "BridgeFurniture",
        Some("BridgeFurniture"),
        levels!(OCCUPIED, [&FURNITURE, &BRIDGE_FURNITURE])
    ),
    class!(
        Bridge,
        "BridgeConstructiveElement",
        Some("BridgeConstructiveElement"),
        levels!(
            OCCUPIED,
            [&CONSTRUCTIVE_ELEMENT, &BRIDGE_CONSTRUCTIVE_ELEMENT]
        )
    ),
    class!(
        Tunnel,
        "Tunnel",
        Some("Tunnel"),
        levels!(OCCUPIED, [&CONSTRUCTION, &ABSTRACT_TUNNEL, &TUNNEL])
    ),
    class!(
        Tunnel,
        "TunnelPart",
        Some("TunnelPart"),
        levels!(OCCUPIED, [&CONSTRUCTION, &ABSTRACT_TUNNEL, &TUNNEL_PART])
    ),
    class!(
        Tunnel,
        "TunnelInstallation",
        Some("TunnelInstallation"),
        levels!(OCCUPIED, [&INSTALLATION, &TUNNEL_INSTALLATION])
    ),
    class!(
        Tunnel,
        "HollowSpace",
        Some("TunnelHollowSpace"),
        levels!(UNOCCUPIED, [&HOLLOW_SPACE])
    ),
    class!(
        Tunnel,
        "TunnelFurniture",
        Some("TunnelFurniture"),
        levels!(OCCUPIED, [&FURNITURE, &TUNNEL_FURNITURE])
    ),
    class!(
        Tunnel,
        "TunnelConstructiveElement",
        Some("TunnelConstructiveElement"),
        levels!(
            OCCUPIED,
            [&CONSTRUCTIVE_ELEMENT, &TUNNEL_CONSTRUCTIVE_ELEMENT]
        )
    ),
    class!(
        Construction,
        "OtherConstruction",
        Some("OtherConstruction"),
        levels!(OCCUPIED, [&CONSTRUCTION, &OTHER_CONSTRUCTION])
    ),
    class!(
        WaterBody,
        "WaterBody",
        Some("WaterBody"),
        levels!(OCCUPIED, [&WATER_BODY])
    ),
    class!(
        LandUse,
        "LandUse",
        Some("LandUse"),
        levels!(THEMATIC, [&LAND_USE])
    ),
    class!(
        CityFurniture,
        "CityFurniture",
        Some("CityFurniture"),
        levels!(OCCUPIED, [&CITY_FURNITURE])
    ),
    class!(
        Vegetation,
        "SolitaryVegetationObject",
        Some("SolitaryVegetationObject"),
        levels!(OCCUPIED, [&VEGETATION_OBJECT, &SOLITARY_VEGETATION_OBJECT])
    ),
    class!(
        Vegetation,
        "PlantCover",
        Some("PlantCover"),
        levels!(OCCUPIED, [&VEGETATION_OBJECT, &PLANT_COVER])
    ),
    class!(
        Generics,
        "GenericOccupiedSpace",
        Some("GenericCityObject"),
        levels!(OCCUPIED, [&GENERIC_OCCUPIED_SPACE])
    ),
    class!(
        Generics,
        "GenericUnoccupiedSpace",
        None,
        levels!(UNOCCUPIED, [&GENERIC_UNOCCUPIED_SPACE])
    ),
    class!(
        Generics,
        "GenericLogicalSpace",
        None,
        levels!(LOGICAL, [&GENERIC_LOGICAL_SPACE])
    ),
    class!(
        CityObjectGroup,
        "CityObjectGroup",
        Some("CityObjectGroup"),
        levels!(LOGICAL, [&CITY_OBJECT_GROUP])
    ),
    class!(
        Transportation,
        "Road",
        Some("Road"),
        levels!(UNOCCUPIED, [&TRANSPORTATION_SPACE, &ROAD])
    ),
    class!(
        Transportation,
        "Railway",
        Some("Railway"),
        levels!(UNOCCUPIED, [&TRANSPORTATION_SPACE, &RAILWAY])
    ),
    class!(
        Transportation,
        "Track",
        None,
        levels!(UNOCCUPIED, [&TRANSPORTATION_SPACE, &TRACK])
    ),
    class!(
        Transportation,
        "Square",
        Some("TransportSquare"),
        levels!(UNOCCUPIED, [&TRANSPORTATION_SPACE, &SQUARE])
    ),
    class!(
        Transportation,
        "Waterway",
        Some("Waterway"),
        levels!(UNOCCUPIED, [&TRANSPORTATION_SPACE, &WATERWAY])
    ),
    class!(
        Relief,
        "TINRelief",
        Some("TINRelief"),
        levels!([&SPACE_BOUNDARY], [&RELIEF_COMPONENT, &TIN_RELIEF])
    ),
];

macro_rules! surface {
    ($module:ident, $name:literal, [$($cityjson:literal),*], $holder:literal, $levels:expr) => {
        SurfaceDef {
            module: Module::$module,
            name: $name,
            cityjson: &[$($cityjson),*],
            holder: $holder,
            levels: $levels,
        }
    };
}

/// A construction's boundary surface of class `$level`.
macro_rules! construction_surface {
    ($name:literal, $level:expr) => {
        surface!(
            Construction,
            $name,
            [$name],
            "boundary",
            levels!(THEMATIC, [&CONSTRUCTION_SURFACE, $level])
        )
    };
}

/// The boundary surface classes, with the CityJSON semantic surface types
/// each is written for and the property of its owner (or of the surface it
/// fills) that holds it.
pub(crate) const SURFACES: &[SurfaceDef] = &[
    construction_surface!("WallSurface", &WALL_SURFACE),
    construction_surface!("RoofSurface", &ROOF_SURFACE),
    construction_surface!("GroundSurface", &GROUND_SURFACE),
    construction_surface!("OuterCeilingSurface", &OUTER_CEILING_SURFACE),
    construction_surface!("OuterFloorSurface", &OUTER_FLOOR_SURFACE),
    construction_surface!("CeilingSurface", &CEILING_SURFACE),
    construction_surface!("FloorSurface", &FLOOR_SURFACE),
    construction_surface!("InteriorWallSurface", &INTERIOR_WALL_SURFACE),
    surface!(
        Construction,
        "DoorSurface",
        ["Door"],
        "fillingSurface",
        levels!(THEMATIC, [&FILLING_SURFACE, &DOOR_SURFACE])
    ),
    surface!(
        Construction,
        "WindowSurface",
        ["Window"],
        "fillingSurface",
        levels!(THEMATIC, [&FILLING_SURFACE, &WINDOW_SURFACE])
    ),
    surface!(
        Core,
        "ClosureSurface",
        ["ClosureSurface", "WaterClosureSurface"],
        "boundary",
        levels!(THEMATIC, [&CLOSURE_SURFACE])
    ),
    surface!(
        WaterBody,
        "WaterSurface",
        ["WaterSurface"],
        "boundary",
        levels!(THEMATIC, [&WATER_BOUNDARY_SURFACE, &WATER_SURFACE])
    ),
    surface!(
        WaterBody,
        "WaterGroundSurface",
        ["WaterGroundSurface"],
        "boundary",
        levels!(THEMATIC, [&WATER_BOUNDARY_SURFACE, &WATER_GROUND_SURFACE])
    ),
    surface!(
        Transportation,
        "TrafficArea",
        ["TrafficArea"],
        "trafficSpace",
        levels!(THEMATIC, [&TRAFFIC_AREA])
    ),
    surface!(
        Transportation,
        "AuxiliaryTrafficArea",
        ["AuxiliaryTrafficArea"],
        "auxiliaryTrafficSpace",
        levels!(THEMATIC, [&AUXILIARY_TRAFFIC_AREA])
    ),
    surface!(
        Transportation,
        "Marking",
        ["TransportationMarking"],
        "marking",
        levels!(THEMATIC, [&MARKING])
    ),
    surface!(
        Transportation,
        "HoleSurface",
        ["TransportationHole"],
        "hole",
        levels!(THEMATIC, [&HOLE_SURFACE])
    ),
];

/// The levels every city object begins with, whose hooks are among the
/// properties every city object has.
pub(crate) const COMMON_LEVELS: [&Level; 3] = [&FEATURE, &FEATURE_WITH_LIFESPAN, &CITY_OBJECT];

/// The levels of an object of an ADE class whose CityGML class is not
/// known: those of an occupied space, which most ADE classes are.
pub(crate) const ADE_CLASS: &[&Level] = &OCCUPIED;

/// The levels of an ADE's boundary surface, which substitutes for a
/// thematic surface: its geometry at LoDs 0 to 3.
pub(crate) const ADE_SURFACE: &[&Level] = &THEMATIC;

/// The class `class` names, if this writer writes it.
pub(crate) fn class_def(class: &QName) -> Option<&'static ClassDef> {
    let Namespace::Module(module) = class.ns else {
        return None;
    };
    CLASSES
        .iter()
        .find(|c| c.module == module && c.name == class.local)
}

/// The boundary surface class `class` names, if this writer writes it.
pub(crate) fn surface_def(class: &QName) -> Option<&'static SurfaceDef> {
    let Namespace::Module(module) = class.ns else {
        return None;
    };
    SURFACES
        .iter()
        .find(|s| s.module == module && s.name == class.local)
}

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

const fn core(name: &'static str, kind: PropertyKind) -> Property {
    Property {
        name,
        ns: PropertyNs::Module(Module::Core),
        kind,
        repeated: false,
    }
}

/// The properties every city object has before its class's own, as far as
/// they hold a value: `gml:description`, `gml:name`, then those of
/// `core:AbstractFeatureWithLifespan` and `core:AbstractCityObject`.
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
    core("creationDate", PropertyKind::DateTime),
    core("terminationDate", PropertyKind::DateTime),
    core("validFrom", PropertyKind::DateTime),
    core("validTo", PropertyKind::DateTime),
    core(
        "relativeToTerrain",
        PropertyKind::Enumeration(RELATIVE_TO_TERRAIN),
    ),
    core(
        "relativeToWater",
        PropertyKind::Enumeration(RELATIVE_TO_WATER),
    ),
];

/// The attribute a measured height is: what CityGML 2.0 calls
/// `measuredHeight`, which CityGML 3.0 writes as a construction's
/// `con:height` whose status is `measured`.
pub(crate) const MEASURED_HEIGHT: Property = Property {
    name: "measuredHeight",
    ns: PropertyNs::Module(Module::Construction),
    kind: LENGTH,
    repeated: false,
};

/// CityGML 3.0's names for CityJSON's.
pub struct Vocabulary3;

impl Vocabulary for Vocabulary3 {
    fn class(&self, cityjson_type: &str, _: &dyn Fn(&str) -> Option<String>) -> Option<Class> {
        let def = CLASSES.iter().find(|c| c.cityjson == Some(cityjson_type))?;
        Some(Class {
            name: QName::module(def.module, def.name),
            implied: &[],
        })
    }

    fn surface(&self, surface: &str, owner: &QName) -> Option<QName> {
        let candidate = SURFACES.iter().find(|s| s.cityjson.contains(&surface))?;
        // A door or window belongs in a surface of an object that has
        // boundary surfaces.
        let holder = match candidate.holder {
            "fillingSurface" => "boundary",
            holder => holder,
        };
        // An ADE class of no known CityGML class has `ADE_CLASS`'s content.
        let holds = match class_def(owner) {
            Some(def) => def.slots().any(|(_, s)| s.holder() == Some(holder)),
            None => ADE_CLASS
                .iter()
                .flat_map(|l| l.slots)
                .any(|s| s.holder() == Some(holder)),
        };
        holds.then(|| QName::module(candidate.module, candidate.name))
    }

    fn property(&self, class: &QName, name: &str) -> Option<Property> {
        if let Some(p) = COMMON.iter().find(|p| p.name == name) {
            return Some(*p);
        }
        let levels = match class_def(class) {
            Some(def) => def.levels,
            None => surface_def(class)?.levels,
        };
        levels.iter().find_map(|level| {
            level.slots.iter().find_map(|s| match s {
                Value(n, kind, repeated) if *n == name => Some(Property {
                    name: n,
                    ns: PropertyNs::Module(level.module),
                    kind: *kind,
                    repeated: *repeated,
                }),
                Height if name == MEASURED_HEIGHT.name => Some(MEASURED_HEIGHT),
                _ => None,
            })
        })
    }
}

#[cfg(test)]
mod tests {
    //! Every level against the official CityGML 3.0 schemas
    //! (`tests/data/schemas/citygml/3.0`): what a class's levels list, and
    //! each level's hook, is what its content model declares, in the same
    //! order; and each property the content model declares outside what
    //! every city object has is listed, unless [`UNUSED`] names it.

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
        /// Complex type -> its base and the elements its own sequence
        /// declares, in order: not those of the anonymous types inside it.
        types: HashMap<Name, (Option<Name>, Vec<Name>)>,
    }

    fn resolve(qname: &str, prefixes: &HashMap<String, String>, target: &str) -> Name {
        match qname.split_once(':') {
            Some((p, l)) => (prefixes[p].clone(), l.to_string()),
            None => (target.to_string(), qname.to_string()),
        }
    }

    fn read() -> Schemas {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/schemas/citygml/3.0");
        let mut schemas = Schemas::default();
        for entry in std::fs::read_dir(dir).unwrap() {
            let text = std::fs::read_to_string(entry.unwrap().path()).unwrap();
            let mut reader = NsReader::from_str(&text);
            let mut prefixes = HashMap::new();
            let mut target = String::new();
            // The named complex type being read, its base and its elements.
            let mut current: Option<(Name, Option<Name>, Vec<Name>)> = None;
            // The elements open inside it, by local name.
            let mut open: Vec<String> = Vec::new();
            loop {
                let event = reader.read_event().unwrap();
                let (e, empty) = match &event {
                    Event::Start(e) => (e.clone(), false),
                    Event::Empty(e) => (e.clone(), true),
                    Event::End(_) => {
                        if current.is_some() {
                            open.pop();
                            if open.is_empty() {
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
                    // Only the type's own content: no element is open
                    // between the type and this one.
                    let own = !open.iter().any(|o| o == "element");
                    match local.as_str() {
                        "extension" if own && base.is_none() => {
                            *base = attr("base").map(|b| resolve(&b, &prefixes, &target))
                        }
                        "element" if own => seq.push(match attr("ref") {
                            Some(r) => resolve(&r, &prefixes, &target),
                            None => (target.clone(), attr("name").unwrap()),
                        }),
                        _ => {}
                    }
                    if !empty {
                        open.push(local);
                    }
                    continue;
                }
                match local.as_str() {
                    "complexType" if !empty => {
                        if let Some(name) = attr("name") {
                            current = Some(((target.clone(), name), None, Vec::new()));
                            open.push(local);
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

        fn ty(&self, module: Module, class: &str) -> &Name {
            let ns = ns::uri(module).unwrap().to_string();
            self.elements
                .get(&(ns, class.to_string()))
                .unwrap_or_else(|| panic!("{class} is not an element of the {module:?} schema"))
        }

        fn content(&self, module: Module, class: &str) -> Vec<Name> {
            self.sequence(self.ty(module, class))
        }
    }

    fn name(module: Module, local: &str) -> Name {
        (ns::uri(module).unwrap().to_string(), local.to_string())
    }

    /// The element a slot of a level of `module` writes.
    fn slot_name(module: Module, slot: &Slot) -> Name {
        match *slot {
            Value(n, ..) | Surfaces(n) | Spaces(n, ..) | Nest(n, _) | Address(n) => name(module, n),
            Geometry(n) => name(Module::Core, n),
            Boundary => name(Module::Core, "boundary"),
            Member => name(module, "groupMember"),
            Filling => name(module, "fillingSurface"),
            Height => name(module, "height"),
            ReliefLod => name(module, "lod"),
            Tin => name(module, "tin"),
        }
    }

    /// The properties a writer has no use for: CityJSON has nothing they
    /// would hold (a constructive element's `filling` holds door and
    /// window features, which CityJSON does not have).
    const UNUSED: &[&str] = &[
        "volume",
        "area",
        "pointCloud",
        "constructionEvent",
        "elevation",
        "occupancy",
        "roomHeight",
        "section",
        "intersection",
        "parent",
        "extent",
        "filling",
    ];

    /// What `levels` list, each level's hook after its slots, the common
    /// levels' first.
    fn listed(levels: &[&Level]) -> Vec<Name> {
        let mut out = Vec::new();
        for level in COMMON_LEVELS.iter().chain(levels) {
            out.extend(level.slots.iter().map(|s| slot_name(level.module, s)));
            out.push(name(level.module, &level.hook()));
        }
        out
    }

    /// `levels` against what the content model of `class` declares.
    fn check(schemas: &Schemas, module: Module, class: &str, levels: &[&Level]) {
        let declared = schemas.content(module, class);
        let listed = listed(levels);
        let mut at = 0;
        for n in &listed {
            let found = declared[at..]
                .iter()
                .position(|d| d == n)
                .unwrap_or_else(|| {
                    panic!("{class}: {n:?} is not declared after position {at}: {declared:?}")
                });
            at += found + 1;
        }
        // What every city object has is the writer's own.
        let common: Vec<Name> = ["AbstractFeatureType", "AbstractFeatureWithLifespanType"]
            .iter()
            .chain(&["AbstractCityObjectType"])
            .flat_map(|t| schemas.sequence(&(ns::CORE.to_string(), t.to_string())))
            .collect();
        let missing: Vec<&str> = declared
            .iter()
            .filter(|d| !common.contains(d) && !UNUSED.contains(&d.1.as_str()))
            .filter(|d| !listed.contains(d))
            .map(|d| d.1.as_str())
            .collect();
        assert!(
            missing.is_empty(),
            "{class}: declared but not listed: {missing:?}"
        );
    }

    #[test]
    fn every_class_lists_its_content_in_schema_order() {
        let schemas = read();
        for def in CLASSES {
            check(&schemas, def.module, def.name, def.levels);
        }
        for def in SURFACES {
            check(&schemas, def.module, def.name, def.levels);
        }
    }

    #[test]
    fn the_common_properties_are_a_city_object_s() {
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

    /// The classes a nesting property names, and a transportation space's
    /// class, are elements of their module; a space states its
    /// granularity where its content model requires it.
    #[test]
    fn what_a_level_names_exists() {
        let schemas = read();
        let levels = CLASSES.iter().flat_map(|c| c.levels.iter());
        for level in levels {
            for slot in level.slots {
                match *slot {
                    Nest(_, classes) => {
                        for class in classes {
                            schemas.ty(level.module, class);
                        }
                    }
                    Spaces(_, class, granularity) => {
                        let declares = schemas
                            .content(level.module, class)
                            .contains(&name(level.module, "granularity"));
                        assert_eq!(declares, granularity, "{class}");
                    }
                    _ => {}
                }
            }
        }
    }
}
