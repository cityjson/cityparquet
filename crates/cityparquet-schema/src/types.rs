use crate::error::{CityParquetError, Result};

/// A CityJSON Level of Detail such as `1`, `2`, or `2.2` (major, optional minor).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Lod {
    major: u8,
    minor: Option<u8>,
}

impl Lod {
    pub fn parse(s: &str) -> Result<Self> {
        let err = || CityParquetError::Lod(s.to_string());
        let mut parts = s.split('.');
        let major = parts.next().filter(|p| !p.is_empty()).ok_or_else(err)?;
        let major: u8 = major.parse().map_err(|_| err())?;
        let minor = match parts.next() {
            Some(m) => Some(m.parse().map_err(|_| err())?),
            None => None,
        };
        if parts.next().is_some() {
            return Err(err());
        }
        Ok(Self { major, minor })
    }

    /// The major LoD component (`2` for both `2` and `2.2`).
    pub fn major(&self) -> u8 {
        self.major
    }

    /// CityParquet geometry column suffix, e.g. `lod2_2` for LoD 2.2.
    pub fn column_suffix(&self) -> String {
        match self.minor {
            Some(minor) => format!("lod{}_{minor}", self.major),
            None => format!("lod{}", self.major),
        }
    }

    pub fn from_column_suffix(suffix: &str) -> Option<Self> {
        let rest = suffix.strip_prefix("lod")?;
        Self::parse(&rest.replace('_', ".")).ok()
    }
}

/// The LoD that occupies the un-suffixed `geometry` column (§9): the **highest**
/// LoD of the `0.*` family present (`0`, `0.0`, `0.1`, `0.2`, `0.3`), or `None`
/// when the dataset has no `0.*` LoD. Derived `Ord` on `(major, minor)` makes
/// `max` pick the finest refinement (bare `0` sorts below `0.0`).
pub fn footprint_lod(lods: &[Lod]) -> Option<Lod> {
    lods.iter().copied().filter(|l| l.major() == 0).max()
}

/// Column name for a reserved geometry/appearance `prefix` at `lod`, given the
/// dataset's `footprint` LoD (see [`footprint_lod`]): the bare `prefix` for the
/// footprint LoD, else `prefix_lod<suffix>` (§9).
pub fn geometry_column_name(prefix: &str, lod: &Lod, footprint: Option<Lod>) -> String {
    if footprint == Some(*lod) {
        prefix.to_string()
    } else {
        format!("{prefix}_{}", lod.column_suffix())
    }
}

impl std::fmt::Display for Lod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.minor {
            Some(minor) => write!(f, "{}.{minor}", self.major),
            None => write!(f, "{}", self.major),
        }
    }
}

/// CityGML 3.0 thematic module that owns a feature class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CityGmlModule {
    Core,
    Building,
    Bridge,
    Tunnel,
    Construction,
    Transportation,
    Vegetation,
    Relief,
    WaterBody,
    LandUse,
    CityFurniture,
    CityObjectGroup,
    Generics,
}

/// CityGML 3.0 conceptual-model alignment for one CityJSON object type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClassInfo {
    /// CityJSON `type` value (also the CityGML class name for these entries).
    pub cityjson_type: &'static str,
    /// Corresponding CityGML 3.0 CM class.
    pub citygml_class: &'static str,
    /// Owning CityGML 3.0 module.
    pub module: CityGmlModule,
    /// Immediate CM superclass (is-a).
    pub citygml_parent: &'static str,
    /// Whether CityJSON allows this as a first-level (top-level) city object.
    pub top_level: bool,
}

macro_rules! class {
    ($cj:literal, $gml:literal, $module:ident, $parent:literal, $top:literal) => {
        ClassInfo {
            cityjson_type: $cj,
            citygml_class: $gml,
            module: CityGmlModule::$module,
            citygml_parent: $parent,
            top_level: $top,
        }
    };
}

/// CityJSON 2.0 type ⇄ CityGML 3.0 CM mapping table.
pub static TAXONOMY: &[ClassInfo] = &[
    class!("Building", "Building", Building, "AbstractBuilding", true),
    class!(
        "BuildingPart",
        "BuildingPart",
        Building,
        "AbstractBuilding",
        false
    ),
    class!(
        "BuildingInstallation",
        "BuildingInstallation",
        Building,
        "AbstractInstallation",
        false
    ),
    class!(
        "BuildingConstructiveElement",
        "BuildingConstructiveElement",
        Building,
        "AbstractConstructiveElement",
        false
    ),
    class!(
        "BuildingFurniture",
        "BuildingFurniture",
        Building,
        "AbstractFurniture",
        false
    ),
    class!(
        "BuildingStorey",
        "Storey",
        Building,
        "AbstractBuildingSubdivision",
        false
    ),
    class!(
        "BuildingRoom",
        "BuildingRoom",
        Building,
        "AbstractUnoccupiedSpace",
        false
    ),
    class!(
        "BuildingUnit",
        "BuildingUnit",
        Building,
        "AbstractBuildingSubdivision",
        false
    ),
    class!("Bridge", "Bridge", Bridge, "AbstractBridge", true),
    class!("BridgePart", "BridgePart", Bridge, "AbstractBridge", false),
    class!(
        "BridgeInstallation",
        "BridgeInstallation",
        Bridge,
        "AbstractInstallation",
        false
    ),
    class!(
        "BridgeConstructiveElement",
        "BridgeConstructiveElement",
        Bridge,
        "AbstractConstructiveElement",
        false
    ),
    class!(
        "BridgeRoom",
        "BridgeRoom",
        Bridge,
        "AbstractUnoccupiedSpace",
        false
    ),
    class!(
        "BridgeFurniture",
        "BridgeFurniture",
        Bridge,
        "AbstractFurniture",
        false
    ),
    class!("Tunnel", "Tunnel", Tunnel, "AbstractTunnel", true),
    class!("TunnelPart", "TunnelPart", Tunnel, "AbstractTunnel", false),
    class!(
        "TunnelInstallation",
        "TunnelInstallation",
        Tunnel,
        "AbstractInstallation",
        false
    ),
    class!(
        "TunnelConstructiveElement",
        "TunnelConstructiveElement",
        Tunnel,
        "AbstractConstructiveElement",
        false
    ),
    class!(
        "TunnelHollowSpace",
        "HollowSpace",
        Tunnel,
        "AbstractUnoccupiedSpace",
        false
    ),
    class!(
        "TunnelFurniture",
        "TunnelFurniture",
        Tunnel,
        "AbstractFurniture",
        false
    ),
    class!(
        "OtherConstruction",
        "OtherConstruction",
        Construction,
        "AbstractConstruction",
        true
    ),
    class!(
        "Road",
        "Road",
        Transportation,
        "AbstractTransportationSpace",
        true
    ),
    class!(
        "Railway",
        "Railway",
        Transportation,
        "AbstractTransportationSpace",
        true
    ),
    class!(
        "Waterway",
        "Waterway",
        Transportation,
        "AbstractTransportationSpace",
        true
    ),
    class!(
        "TransportSquare",
        "Square",
        Transportation,
        "AbstractTransportationSpace",
        true
    ),
    class!(
        "PlantCover",
        "PlantCover",
        Vegetation,
        "AbstractVegetationObject",
        true
    ),
    class!(
        "SolitaryVegetationObject",
        "SolitaryVegetationObject",
        Vegetation,
        "AbstractVegetationObject",
        true
    ),
    class!(
        "TINRelief",
        "TINRelief",
        Relief,
        "AbstractReliefComponent",
        true
    ),
    class!(
        "WaterBody",
        "WaterBody",
        WaterBody,
        "AbstractOccupiedSpace",
        true
    ),
    class!(
        "LandUse",
        "LandUse",
        LandUse,
        "AbstractThematicSurface",
        true
    ),
    class!(
        "CityFurniture",
        "CityFurniture",
        CityFurniture,
        "AbstractOccupiedSpace",
        true
    ),
    class!(
        "CityObjectGroup",
        "CityObjectGroup",
        CityObjectGroup,
        "AbstractLogicalSpace",
        true
    ),
    class!(
        "GenericCityObject",
        "GenericOccupiedSpace",
        Generics,
        "AbstractOccupiedSpace",
        true
    ),
];

/// Look up the CM alignment for a CityJSON object type.
pub fn class_info(cityjson_type: &str) -> Option<&'static ClassInfo> {
    TAXONOMY.iter().find(|c| c.cityjson_type == cityjson_type)
}

/// CityJSON extension object types start with `+` and are defined by an
/// Extension schema rather than the CM taxonomy.
pub fn is_extension_type(cityjson_type: &str) -> bool {
    cityjson_type.starts_with('+')
}

/// The 1st-level (top-level) CityObject type whose CityParquet by-type table
/// a given `object_type` is stored in. CityJSON 2nd-level objects
/// (BuildingPart, BridgeInstallation, TunnelConstructiveElement, …) share
/// their 1st-level parent's table (Building/Bridge/Tunnel); a 1st-level type,
/// or an unknown/extension type, maps to itself.
pub fn first_level_type(object_type: &str) -> &str {
    match class_info(object_type) {
        Some(info) if !info.top_level => TAXONOMY
            .iter()
            .find(|c| c.module == info.module && c.top_level)
            .map(|c| c.cityjson_type)
            .unwrap_or(object_type),
        _ => object_type,
    }
}

#[cfg(test)]
mod lod_tests {
    use super::*;

    #[test]
    fn parses_and_displays() {
        assert_eq!(Lod::parse("2").unwrap().to_string(), "2");
        assert_eq!(Lod::parse("2.2").unwrap().to_string(), "2.2");
        assert!(Lod::parse("").is_err());
        assert!(Lod::parse("2.x").is_err());
        assert!(Lod::parse("2.2.2").is_err());
    }

    #[test]
    fn column_suffix_round_trip() {
        let lod = Lod::parse("2.2").unwrap();
        assert_eq!(lod.column_suffix(), "lod2_2");
        assert_eq!(Lod::from_column_suffix("lod2_2"), Some(lod));
        assert_eq!(
            Lod::from_column_suffix("lod1"),
            Some(Lod::parse("1").unwrap())
        );
        assert_eq!(Lod::from_column_suffix("geometry"), None);
    }

    #[test]
    fn footprint_lod_is_the_highest_zero_family_lod() {
        let p = |s: &str| Lod::parse(s).unwrap();
        // Highest 0.* wins.
        assert_eq!(
            footprint_lod(&[p("0.1"), p("0.3"), p("2.2")]),
            Some(p("0.3"))
        );
        // Bare 0 when it is the only 0.* LoD.
        assert_eq!(footprint_lod(&[p("0"), p("2.2")]), Some(p("0")));
        assert_eq!(footprint_lod(&[p("0.2")]), Some(p("0.2")));
        // No 0.* LoD at all.
        assert_eq!(footprint_lod(&[p("1.2"), p("2.2")]), None);
    }

    #[test]
    fn footprint_lod_maps_to_bare_names_others_keep_suffix() {
        let p = |s: &str| Lod::parse(s).unwrap();
        let lods = [p("0.1"), p("0.3"), p("2.2")];
        let fp = footprint_lod(&lods);
        // The highest 0.* (0.3) is the un-suffixed geometry column.
        assert_eq!(geometry_column_name("geometry", &p("0.3"), fp), "geometry");
        assert_eq!(
            geometry_column_name("geometry_properties", &p("0.3"), fp),
            "geometry_properties"
        );
        // A lower 0.* keeps its suffix.
        assert_eq!(
            geometry_column_name("geometry", &p("0.1"), fp),
            "geometry_lod0_1"
        );
        // A non-zero LoD keeps its suffix.
        assert_eq!(
            geometry_column_name("geometry", &p("2.2"), fp),
            "geometry_lod2_2"
        );
    }

    #[test]
    fn lod_major_extracts_major_component() {
        assert_eq!(Lod::parse("2").unwrap().major(), 2);
        assert_eq!(Lod::parse("2.2").unwrap().major(), 2);
        assert_eq!(Lod::parse("1").unwrap().major(), 1);
    }

    #[test]
    fn orders_numerically() {
        let mut lods = [
            Lod::parse("2.2").unwrap(),
            Lod::parse("1.3").unwrap(),
            Lod::parse("2").unwrap(),
        ];
        lods.sort();
        let s: Vec<String> = lods.iter().map(|l| l.to_string()).collect();
        assert_eq!(s, ["1.3", "2", "2.2"]);
    }
}

#[cfg(test)]
mod taxonomy_tests {
    use super::*;

    #[test]
    fn building_maps_to_cm() {
        let info = class_info("Building").unwrap();
        assert_eq!(info.citygml_class, "Building");
        assert_eq!(info.module, CityGmlModule::Building);
        assert_eq!(info.citygml_parent, "AbstractBuilding");
        assert!(info.top_level);
    }

    #[test]
    fn building_part_is_second_level() {
        let info = class_info("BuildingPart").unwrap();
        assert_eq!(info.citygml_parent, "AbstractBuilding");
        assert!(!info.top_level);
    }

    #[test]
    fn all_cityjson_20_types_are_covered() {
        // The full CityJSON 2.0 city-object type list.
        let types = [
            "Bridge",
            "BridgePart",
            "BridgeInstallation",
            "BridgeConstructiveElement",
            "BridgeRoom",
            "BridgeFurniture",
            "Building",
            "BuildingPart",
            "BuildingInstallation",
            "BuildingConstructiveElement",
            "BuildingFurniture",
            "BuildingStorey",
            "BuildingRoom",
            "BuildingUnit",
            "CityFurniture",
            "CityObjectGroup",
            "GenericCityObject",
            "LandUse",
            "OtherConstruction",
            "PlantCover",
            "SolitaryVegetationObject",
            "TINRelief",
            "TransportSquare",
            "Road",
            "Railway",
            "Waterway",
            "Tunnel",
            "TunnelPart",
            "TunnelInstallation",
            "TunnelConstructiveElement",
            "TunnelHollowSpace",
            "TunnelFurniture",
            "WaterBody",
        ];
        for t in types {
            assert!(class_info(t).is_some(), "missing taxonomy entry for {t}");
        }
        assert_eq!(TAXONOMY.len(), types.len());
    }

    #[test]
    fn extension_types_are_flagged_not_mapped() {
        assert!(is_extension_type("+NoiseBuilding"));
        assert!(!is_extension_type("Building"));
        assert!(class_info("+NoiseBuilding").is_none());
    }

    #[test]
    fn first_level_type_maps_second_level_to_family_root() {
        assert_eq!(first_level_type("BuildingPart"), "Building");
        assert_eq!(first_level_type("BuildingInstallation"), "Building");
        assert_eq!(first_level_type("BridgeConstructiveElement"), "Bridge");
        assert_eq!(first_level_type("TunnelHollowSpace"), "Tunnel");
    }

    #[test]
    fn first_level_type_is_identity_for_top_level_and_unknown_types() {
        assert_eq!(first_level_type("Building"), "Building");
        assert_eq!(first_level_type("CityFurniture"), "CityFurniture");
        assert_eq!(first_level_type("+NoiseBuilding"), "+NoiseBuilding");
    }
}
