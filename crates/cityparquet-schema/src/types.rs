use crate::error::{CityParquetError, Result};

/// A CityJSON Level of Detail such as `1`, `2`, or `2.2` (major, minor —
/// defaulting to `0` when the source string carried none, e.g. `"1"` and
/// `"1.0"` both parse to the same value). This is a canonicalisation of the
/// LoD *string*, not a value distinction (spec "Levels of detail").
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Lod {
    major: u8,
    minor: u8,
}

impl Lod {
    pub fn parse(s: &str) -> Result<Self> {
        let err = || CityParquetError::Lod(s.to_string());
        let mut parts = s.split('.');
        let major = parts.next().filter(|p| !p.is_empty()).ok_or_else(err)?;
        let major: u8 = major.parse().map_err(|_| err())?;
        let minor: u8 = match parts.next() {
            Some(m) => m.parse().map_err(|_| err())?,
            None => 0,
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

    /// CityParquet geometry column suffix, e.g. `lod2_2` for LoD 2.2, always
    /// with a minor: LoD `1` yields `lod1_0`, never `lod1` (spec "Levels of
    /// detail" — "a suffix always carries a minor").
    pub fn column_suffix(&self) -> String {
        format!("lod{}_{}", self.major, self.minor)
    }

    /// Parse a `lod<major>_<minor>` column suffix. `None` for any shape that
    /// is not exactly `major_minor` — in particular a bare `lod<major>` (no
    /// minor) no longer parses, since every column name now carries one.
    pub fn from_column_suffix(suffix: &str) -> Option<Self> {
        let rest = suffix.strip_prefix("lod")?;
        let (major, minor) = rest.split_once('_')?;
        Self::parse(&format!("{major}.{minor}")).ok()
    }
}

/// Column name for a reserved geometry/appearance `prefix` at `lod`:
/// `prefix_lod<suffix>` unconditionally — every geometry column is suffixed,
/// including LoD0 (spec "Levels of detail": "there is no un-suffixed
/// `geometry`... column").
pub fn geometry_column_name(prefix: &str, lod: &Lod) -> String {
    format!("{prefix}_{}", lod.column_suffix())
}

impl std::fmt::Display for Lod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
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
        // Display always carries the minor (canonical export spelling).
        assert_eq!(Lod::parse("2").unwrap().to_string(), "2.0");
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
        assert_eq!(Lod::from_column_suffix("geometry"), None);
    }

    /// spec §"Levels of detail": "A suffix always carries a minor. LoD `1`
    /// yields `geometry_lod1_0`, never `geometry_lod1`."
    #[test]
    fn column_suffix_always_carries_a_minor() {
        assert_eq!(Lod::parse("1").unwrap().column_suffix(), "lod1_0");
        assert_eq!(Lod::parse("0").unwrap().column_suffix(), "lod0_0");
        assert_eq!(
            Lod::from_column_suffix("lod1"),
            None,
            "a bare-major suffix with no minor is no longer legal column-name shape"
        );
    }

    /// spec: "a source `\"1\"` and a source `\"1.0\"` both map to the same
    /// column `geometry_lod1_0`" — a canonicalisation of the LoD string, not
    /// its value, so the two must be the identical `Lod`.
    #[test]
    fn bare_and_dot_zero_minor_collapse_to_the_same_lod() {
        assert_eq!(Lod::parse("1").unwrap(), Lod::parse("1.0").unwrap());
        assert_eq!(Lod::parse("0").unwrap(), Lod::parse("0.0").unwrap());
        assert_eq!(
            Lod::parse("1").unwrap().column_suffix(),
            Lod::parse("1.0").unwrap().column_suffix()
        );
    }

    /// spec: `Display` always shows `"{major}.{minor}"`, e.g. `"1.0"`, never
    /// bare `"1"` — the canonical export spelling.
    #[test]
    fn display_always_shows_the_minor() {
        assert_eq!(Lod::parse("1").unwrap().to_string(), "1.0");
        assert_eq!(Lod::parse("0").unwrap().to_string(), "0.0");
        assert_eq!(Lod::parse("2.2").unwrap().to_string(), "2.2");
    }

    /// spec "Levels of detail": every LoD's geometry column is suffixed,
    /// including the `0.*` family — there is no picked-out "footprint" LoD
    /// that goes unsuffixed.
    #[test]
    fn geometry_column_name_always_suffixes_every_lod() {
        let p = |s: &str| Lod::parse(s).unwrap();
        assert_eq!(
            geometry_column_name("geometry", &p("0.3")),
            "geometry_lod0_3"
        );
        assert_eq!(
            geometry_column_name("geometry_properties", &p("0")),
            "geometry_properties_lod0_0"
        );
        assert_eq!(
            geometry_column_name("geometry", &p("0.1")),
            "geometry_lod0_1"
        );
        assert_eq!(
            geometry_column_name("geometry", &p("2.2")),
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
        assert_eq!(s, ["1.3", "2.0", "2.2"]);
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
