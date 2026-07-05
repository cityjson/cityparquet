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
}
