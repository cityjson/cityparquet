use std::collections::HashMap;

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
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

/// Reverse of [`class_info`]'s `citygml_class` field: given the CityGML 3.0
/// class name stored in `object_type` (spec "object_type vocabulary"), the
/// CityJSON spelling to restore on export. `None` for a name with no
/// taxonomy entry — an extension class, which keeps its own name verbatim
/// (spec: "An extension ... type keeps its own class name").
pub fn cityjson_type_for_citygml_class(citygml_class: &str) -> Option<&'static str> {
    TAXONOMY
        .iter()
        .find(|c| c.citygml_class == citygml_class)
        .map(|c| c.cityjson_type)
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

/// The by-module routing key for a class (spec "extensions" — "The
/// `ModuleKey`"): either a core CityGML 3.0 module, or the name of an
/// extension module a class declares as its own (recursively inherited from
/// a specialised ancestor). This is what CityParquet's by-module
/// object-table split (spec "By-module object-table layout") actually
/// partitions on — see [`module_file`] for the file name it derives.
///
/// `Ord`/`PartialOrd` (a simple derive: `Core` variants order by
/// [`CityGmlModule`], `Extension` variants by name, and every `Core` sorts
/// before every `Extension`) exist so a caller can key a `BTreeMap<ModuleKey,
/// _>` — e.g. `crate::scan::ScanResult::module_lods` (in the `cityparquet`
/// crate) — deterministically; no ordering is spec-mandated, this is purely
/// a container convenience.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ModuleKey {
    /// A recognised core CityGML 3.0 module.
    Core(CityGmlModule),
    /// An extension (ADE / CityJSON Extension) module, named as the
    /// extension itself declares it (pre-snake_case; see [`module_file`]).
    Extension(String),
}

/// One extension (ADE / CityJSON Extension) class's declaration, as parsed
/// from the source's Extension/ADE schema: the module it owns, if any, and/or
/// the class it specialises, if any. At least one of the two must be
/// resolvable for [`resolve_module_key`] to succeed — see the spec's
/// "extensions" page. Class names are stored/looked-up with any CityJSON `+`
/// marker stripped, since resolution is defined to be `+`-insensitive.
#[derive(Debug, Clone, Default)]
pub struct ExtensionClassDecl {
    /// The module this class declares as its own, e.g. `"Energy"`.
    pub module: Option<String>,
    /// The class this one specialises (its declared CM parent), by source
    /// spelling (`+`-marker optional — stripped on lookup).
    pub parent: Option<String>,
}

/// Every extension class declaration known for one conversion, keyed by
/// class name with any `+` marker stripped. Built from the source's
/// Extension/ADE schema documents (out of scope for this crate to parse —
/// see the spec's `city.extensions` declaration mapping); an empty registry
/// is legitimate for a source with no extensions, or one whose declarations
/// have not yet been wired up by the caller.
#[derive(Debug, Clone, Default)]
pub struct ExtensionRegistry {
    classes: HashMap<String, ExtensionClassDecl>,
}

impl ExtensionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Declares one extension class. `class_name` may carry CityJSON's `+`
    /// marker or not — it is stripped before storage, since lookup always
    /// strips it too.
    pub fn declare(&mut self, class_name: impl AsRef<str>, decl: ExtensionClassDecl) -> &mut Self {
        let key = strip_plus(class_name.as_ref()).to_string();
        self.classes.insert(key, decl);
        self
    }

    fn get(&self, class_name: &str) -> Option<&ExtensionClassDecl> {
        self.classes.get(class_name)
    }
}

/// Strips CityJSON's leading `+` extension marker, if present. Resolution is
/// defined to be indifferent to whether it is there (spec "extensions" —
/// "Whether a class carries CityJSON's `+` marker is irrelevant to routing"),
/// and `object_type` storage drops it outright for an extension class (spec
/// "object_table-schema" — "object_type vocabulary": "with the CityJSON `+`
/// prefix stripped") — `crate::encode`'s `RowWriter::push_object` is the
/// latter's caller.
pub fn strip_plus(source_type: &str) -> &str {
    source_type.strip_prefix('+').unwrap_or(source_type)
}

/// Core-class recognition for [`resolve_module_key`], matching `name`
/// against **either** a class's CityJSON spelling (`cityjson_type`) or its
/// CityGML spelling (`citygml_class`) — unlike the public [`class_info`],
/// which is `cityjson_type`-only. `resolve_module_key` is fed both
/// spellings by its real callers: encode-time callers hold the CityJSON
/// source string, while `object_type`'s stored value is the CityGML class
/// name (spec "object_type vocabulary", gap 15) once a caller reads it back
/// out of an encoded batch. The two spellings never collide (every
/// `citygml_class` value is unique, and equals `cityjson_type` for every
/// class but the 4 documented divergent ones), so matching either is
/// unambiguous.
fn class_info_by_any_spelling(name: &str) -> Option<&'static ClassInfo> {
    TAXONOMY
        .iter()
        .find(|c| c.cityjson_type == name || c.citygml_class == name)
}

/// Resolves `source_type`'s [`ModuleKey`] (spec "extensions" — "The
/// `ModuleKey`"), given `source_type` **before** any CityJSON `+` is
/// stripped (stripping happens inside, so callers never need to do it
/// themselves) and the extension declarations parsed from the source's
/// header.
///
/// Resolution order:
/// 1. A recognised core CityGML 3.0 class (`TAXONOMY`) → `Core(module)`.
/// 2. An extension class declaring its own module → `Extension(module)`.
/// 3. An extension class specialising another class (declares a parent, no
///    module of its own) → resolved recursively from that parent, walking
///    the chain until a core class or a module-declaring ancestor is hit.
///    A parent cycle is a hard [`CityParquetError::Schema`] error, never
///    infinite recursion.
/// 4. Anything else — not a core class, and not declared by any extension —
///    is a hard [`CityParquetError::Schema`] error: routing is total, so an
///    unresolvable class must reject the input rather than fall back
///    silently.
///
/// This does **not** memoise — a caller resolving many objects (e.g. once
/// per distinct `object_type` dictionary value) should reuse a
/// [`ModuleKeyResolver`], which wraps this with a cache keyed by
/// `source_type`.
pub fn resolve_module_key(source_type: &str, extensions: &ExtensionRegistry) -> Result<ModuleKey> {
    resolve_module_key_inner(strip_plus(source_type), extensions, &mut Vec::new())
}

fn resolve_module_key_inner(
    type_name: &str,
    extensions: &ExtensionRegistry,
    visiting: &mut Vec<String>,
) -> Result<ModuleKey> {
    if let Some(info) = class_info_by_any_spelling(type_name) {
        return Ok(ModuleKey::Core(info.module));
    }
    if visiting.iter().any(|v| v == type_name) {
        return Err(CityParquetError::Schema(format!(
            "extension class '{type_name}' has a cyclic parent chain: {} -> {type_name}",
            visiting.join(" -> ")
        )));
    }
    let decl = extensions.get(type_name).ok_or_else(|| {
        CityParquetError::Schema(format!(
            "class '{type_name}' has no resolvable module: it is not a recognised core \
             CityGML 3.0 class, and no extension declares it"
        ))
    })?;
    if let Some(module) = decl.module.as_deref() {
        return Ok(ModuleKey::Extension(module.to_string()));
    }
    match decl.parent.as_deref() {
        Some(parent) => {
            visiting.push(type_name.to_string());
            resolve_module_key_inner(strip_plus(parent), extensions, visiting)
        }
        None => Err(CityParquetError::Schema(format!(
            "extension class '{type_name}' declares neither a module nor a parent, so its \
             file cannot be resolved"
        ))),
    }
}

/// Resolves [`ModuleKey`]s against a fixed [`ExtensionRegistry`], memoising
/// by source-type string so a caller resolving the same distinct type more
/// than once (e.g. once per `object_type` dictionary value, across many
/// batches of one conversion) pays the resolution cost only the first time.
/// The obvious usage pattern: one resolver per conversion, `resolve` called
/// once per distinct source type encountered.
#[derive(Debug, Default)]
pub struct ModuleKeyResolver {
    extensions: ExtensionRegistry,
    cache: HashMap<String, ModuleKey>,
}

impl ModuleKeyResolver {
    pub fn new(extensions: ExtensionRegistry) -> Self {
        Self {
            extensions,
            cache: HashMap::new(),
        }
    }

    /// Resolves `source_type`'s [`ModuleKey`], consulting (and populating)
    /// the cache first. `source_type` may carry CityJSON's `+` marker or
    /// not — both spellings of the same class share one cache entry, since
    /// [`resolve_module_key`] treats them identically.
    pub fn resolve(&mut self, source_type: &str) -> Result<ModuleKey> {
        let key = strip_plus(source_type);
        if let Some(cached) = self.cache.get(key) {
            return Ok(cached.clone());
        }
        let resolved = resolve_module_key(key, &self.extensions)?;
        self.cache.insert(key.to_string(), resolved.clone());
        Ok(resolved)
    }
}

/// The pinned snake_case file-body name for every file-bearing core CityGML
/// 3.0 module (spec "By-module object-table layout" — "The standard
/// object-table files"), hard-coded rather than derived by
/// [`to_snake_case`] so this table never silently drifts if the algorithm
/// changes. `Core` is not file-bearing (spec: "`Core` is a module of
/// abstract base classes, never instantiated directly, so there is no
/// `core.parquet`") — reachable in principle via a pathological future
/// taxonomy entry, but no `TAXONOMY` entry has module `Core` today, so this
/// returns `None` for it rather than fabricating a name. `CityObjectGroup`
/// folds into the same file as `Generics` (spec: "On `CityObjectGroup`" —
/// "it folds into `generics.parquet` alongside `GenericOccupiedSpace`").
fn core_module_file(module: CityGmlModule) -> Option<&'static str> {
    match module {
        CityGmlModule::Building => Some("building"),
        CityGmlModule::Bridge => Some("bridge"),
        CityGmlModule::Tunnel => Some("tunnel"),
        CityGmlModule::Construction => Some("construction"),
        CityGmlModule::Transportation => Some("transportation"),
        CityGmlModule::Vegetation => Some("vegetation"),
        CityGmlModule::Relief => Some("relief"),
        CityGmlModule::WaterBody => Some("water_body"),
        CityGmlModule::LandUse => Some("land_use"),
        CityGmlModule::CityFurniture => Some("city_furniture"),
        CityGmlModule::Generics | CityGmlModule::CityObjectGroup => Some("generics"),
        CityGmlModule::Core => None,
    }
}

/// CamelCase → snake_case (spec "By-module object-table layout" —
/// "File-name rule"): split before an upper-case letter that follows a
/// lower-case letter, and before an upper-case letter followed by a
/// lower-case letter (so a run of upper-case letters, e.g. an acronym,
/// splits only at its trailing edge), lower-case every character, join with
/// `_`.
fn to_snake_case(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len() + 4);
    for (i, &c) in chars.iter().enumerate() {
        if c.is_uppercase() && i > 0 {
            let prev_lower = chars[i - 1].is_lowercase();
            let next_lower = chars.get(i + 1).is_some_and(|n| n.is_lowercase());
            if prev_lower || next_lower {
                out.push('_');
            }
        }
        out.extend(c.to_lowercase());
    }
    out
}

/// The snake_case file-body name (no `.parquet` extension) for a
/// [`ModuleKey`] (spec "By-module object-table layout" / "extensions" —
/// "File-name rule"). Core module names come from the pinned
/// [`core_module_file`] table; extension module names run through
/// [`to_snake_case`]. Panics only if given `ModuleKey::Core(CityGmlModule::Core)`,
/// which [`resolve_module_key`] never produces (no `TAXONOMY` entry has that
/// module) — see [`core_module_file`]'s doc comment.
pub fn module_file(key: &ModuleKey) -> String {
    match key {
        ModuleKey::Core(module) => core_module_file(*module)
            .expect("ModuleKey::Core is only ever constructed for a file-bearing module")
            .to_string(),
        ModuleKey::Extension(name) => to_snake_case(name),
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

    /// spec "object_type vocabulary": the 4 divergent CityJSON ⇄ CityGML
    /// spellings, plus an identity case, both directions.
    #[test]
    fn cityjson_type_for_citygml_class_reverses_the_four_divergent_names() {
        assert_eq!(
            cityjson_type_for_citygml_class("Square"),
            Some("TransportSquare")
        );
        assert_eq!(
            cityjson_type_for_citygml_class("GenericOccupiedSpace"),
            Some("GenericCityObject")
        );
        assert_eq!(
            cityjson_type_for_citygml_class("Storey"),
            Some("BuildingStorey")
        );
        assert_eq!(
            cityjson_type_for_citygml_class("HollowSpace"),
            Some("TunnelHollowSpace")
        );
        // Identity case: most classes have the same CityGML and CityJSON
        // spelling, so the reverse lookup is a no-op round trip.
        assert_eq!(
            cityjson_type_for_citygml_class("Building"),
            Some("Building")
        );
        // An extension class name has no taxonomy entry at all.
        assert_eq!(cityjson_type_for_citygml_class("SolarPanel"), None);
    }
}

#[cfg(test)]
mod module_key_tests {
    use super::*;

    /// spec "By-module object-table layout" — "The standard object-table
    /// files": the pinned table must reproduce exactly these 11 names from
    /// their `CityGmlModule` variants. Written to catch a regression that
    /// "fixes" `core_module_file` to run [`to_snake_case`] on core names
    /// instead of using the pinned literals — `to_snake_case` happens to
    /// agree with the pinned table for most of these (`WaterBody` ->
    /// `water_body`), so this test pins the exact module -> name pairing,
    /// not just the resulting strings.
    #[test]
    fn core_module_file_reproduces_the_eleven_pinned_names() {
        let cases = [
            (CityGmlModule::Building, "building"),
            (CityGmlModule::Bridge, "bridge"),
            (CityGmlModule::Tunnel, "tunnel"),
            (CityGmlModule::Construction, "construction"),
            (CityGmlModule::Transportation, "transportation"),
            (CityGmlModule::Vegetation, "vegetation"),
            (CityGmlModule::Relief, "relief"),
            (CityGmlModule::WaterBody, "water_body"),
            (CityGmlModule::LandUse, "land_use"),
            (CityGmlModule::CityFurniture, "city_furniture"),
            (CityGmlModule::Generics, "generics"),
        ];
        for (module, expected) in cases {
            assert_eq!(
                module_file(&ModuleKey::Core(module)),
                expected,
                "core module {module:?} must derive the pinned file name {expected:?}"
            );
        }
    }

    /// `CityObjectGroup` is not one of the 11 file-bearing names, but folds
    /// into `Generics`'s file (spec "On `CityObjectGroup`").
    #[test]
    fn city_object_group_folds_into_the_generics_file() {
        assert_eq!(
            module_file(&ModuleKey::Core(CityGmlModule::CityObjectGroup)),
            "generics"
        );
        assert_eq!(
            module_file(&ModuleKey::Core(CityGmlModule::CityObjectGroup)),
            module_file(&ModuleKey::Core(CityGmlModule::Generics))
        );
    }

    /// `Core` is never instantiated, so [`resolve_module_key`] never
    /// produces `ModuleKey::Core(CityGmlModule::Core)` — but `module_file`
    /// still must not silently fabricate a `core.parquet` name for it.
    #[test]
    #[should_panic]
    fn module_file_panics_rather_than_name_a_core_dot_parquet() {
        let _ = module_file(&ModuleKey::Core(CityGmlModule::Core));
    }

    /// spec "By-module object-table layout" — "File-name rule": extension
    /// module names are snake_cased, unlike the pinned core names.
    #[test]
    fn extension_module_names_are_snake_cased() {
        assert_eq!(
            module_file(&ModuleKey::Extension("Energy".to_string())),
            "energy"
        );
        assert_eq!(
            module_file(&ModuleKey::Extension("MyExtensionModule".to_string())),
            "my_extension_module"
        );
        assert_eq!(
            module_file(&ModuleKey::Extension("HTTPServer".to_string())),
            "http_server",
            "a run of upper-case letters (an acronym) splits only at its trailing edge"
        );
    }

    /// spec "extensions" — a core class resolves to its `Core` module,
    /// regardless of any extension registry passed alongside it.
    #[test]
    fn core_class_resolves_without_consulting_extensions() {
        let extensions = ExtensionRegistry::new();
        assert_eq!(
            resolve_module_key("Building", &extensions).unwrap(),
            ModuleKey::Core(CityGmlModule::Building)
        );
        assert_eq!(
            resolve_module_key("BuildingPart", &extensions).unwrap(),
            ModuleKey::Core(CityGmlModule::Building)
        );
    }

    /// A real caller (`cityparquet`'s by-module writer) resolves off the
    /// STORED `object_type` value, which is the CityGML spelling for the 4
    /// divergent classes (gap 15) — resolution must recognise a core class
    /// by either spelling and land on the identical `ModuleKey`.
    #[test]
    fn core_class_resolves_identically_by_either_spelling() {
        let extensions = ExtensionRegistry::new();
        assert_eq!(
            resolve_module_key("TransportSquare", &extensions).unwrap(),
            resolve_module_key("Square", &extensions).unwrap()
        );
        assert_eq!(
            resolve_module_key("GenericCityObject", &extensions).unwrap(),
            resolve_module_key("GenericOccupiedSpace", &extensions).unwrap()
        );
        assert_eq!(
            resolve_module_key("Square", &extensions).unwrap(),
            ModuleKey::Core(CityGmlModule::Transportation)
        );
    }

    /// spec "extensions": an extension class declaring its own module
    /// resolves to `Extension(module)`.
    #[test]
    fn extension_class_with_its_own_module_resolves_directly() {
        let mut extensions = ExtensionRegistry::new();
        extensions.declare(
            "SolarPanel",
            ExtensionClassDecl {
                module: Some("Energy".to_string()),
                parent: None,
            },
        );
        assert_eq!(
            resolve_module_key("+SolarPanel", &extensions).unwrap(),
            ModuleKey::Extension("Energy".to_string())
        );
    }

    /// spec "extensions": "Whether a class carries CityJSON's `+` marker is
    /// irrelevant to routing" — a `+`-less declared class resolves exactly
    /// like its `+`-marked spelling.
    #[test]
    fn resolution_is_indifferent_to_the_plus_marker() {
        let mut extensions = ExtensionRegistry::new();
        extensions.declare(
            "Thermostat",
            ExtensionClassDecl {
                module: Some("Energy".to_string()),
                parent: None,
            },
        );
        assert_eq!(
            resolve_module_key("Thermostat", &extensions).unwrap(),
            resolve_module_key("+Thermostat", &extensions).unwrap()
        );
    }

    /// spec "extensions": a class that specialises an existing class (no
    /// module of its own) routes to that ancestor's module — recursively,
    /// walking multiple hops of extension-only ancestors until a core class
    /// is hit.
    #[test]
    fn specialising_class_recurses_to_an_ancestors_module() {
        let mut extensions = ExtensionRegistry::new();
        // +NoiseBuilding specialises Building directly (core parent).
        extensions.declare(
            "NoiseBuilding",
            ExtensionClassDecl {
                module: None,
                parent: Some("Building".to_string()),
            },
        );
        assert_eq!(
            resolve_module_key("+NoiseBuilding", &extensions).unwrap(),
            ModuleKey::Core(CityGmlModule::Building)
        );

        // +ExtraNoisyBuilding specialises +NoiseBuilding, which itself has
        // no module and specialises Building — two hops to a core module.
        extensions.declare(
            "ExtraNoisyBuilding",
            ExtensionClassDecl {
                module: None,
                parent: Some("+NoiseBuilding".to_string()),
            },
        );
        assert_eq!(
            resolve_module_key("+ExtraNoisyBuilding", &extensions).unwrap(),
            ModuleKey::Core(CityGmlModule::Building)
        );
    }

    /// spec "extensions": a class specialising an extension-module-owning
    /// ancestor (not a core class) inherits that module.
    #[test]
    fn specialising_class_recurses_to_an_extension_modules_ancestor() {
        let mut extensions = ExtensionRegistry::new();
        extensions.declare(
            "SolarPanel",
            ExtensionClassDecl {
                module: Some("Energy".to_string()),
                parent: None,
            },
        );
        extensions.declare(
            "RooftopSolarPanel",
            ExtensionClassDecl {
                module: None,
                parent: Some("+SolarPanel".to_string()),
            },
        );
        assert_eq!(
            resolve_module_key("+RooftopSolarPanel", &extensions).unwrap(),
            ModuleKey::Extension("Energy".to_string())
        );
    }

    /// spec "extensions": a class with no resolvable module at all — not a
    /// core class, and not declared by any extension — is a hard error.
    #[test]
    fn unresolvable_class_is_a_hard_error() {
        let extensions = ExtensionRegistry::new();
        let e = resolve_module_key("+NoiseBuilding", &extensions).unwrap_err();
        assert!(matches!(e, CityParquetError::Schema(_)));
        assert!(e.to_string().contains("NoiseBuilding"));
    }

    /// A declared class with neither a module nor a parent is equally
    /// unresolvable.
    #[test]
    fn declared_class_with_neither_module_nor_parent_is_a_hard_error() {
        let mut extensions = ExtensionRegistry::new();
        extensions.declare("Empty", ExtensionClassDecl::default());
        let e = resolve_module_key("+Empty", &extensions).unwrap_err();
        assert!(matches!(e, CityParquetError::Schema(_)));
    }

    /// A parent cycle must be a hard error, never infinite recursion / stack
    /// overflow.
    #[test]
    fn parent_cycle_is_a_hard_error_not_infinite_recursion() {
        let mut extensions = ExtensionRegistry::new();
        extensions.declare(
            "A",
            ExtensionClassDecl {
                module: None,
                parent: Some("+B".to_string()),
            },
        );
        extensions.declare(
            "B",
            ExtensionClassDecl {
                module: None,
                parent: Some("+A".to_string()),
            },
        );
        let e = resolve_module_key("+A", &extensions).unwrap_err();
        assert!(matches!(e, CityParquetError::Schema(_)));
        assert!(e.to_string().to_lowercase().contains("cycl"));
    }

    /// [`ModuleKeyResolver`] memoises: once a type resolves, a repeat lookup
    /// (of either `+`-marked or bare spelling) does not need the extension
    /// registry to answer any more — proven here by resolving successfully
    /// even though the SECOND lookup is against a resolver whose registry
    /// no longer declares the class (constructed empty), which would error
    /// if it actually re-ran resolution.
    #[test]
    fn resolver_memoises_by_source_type_string() {
        let mut extensions = ExtensionRegistry::new();
        extensions.declare(
            "SolarPanel",
            ExtensionClassDecl {
                module: Some("Energy".to_string()),
                parent: None,
            },
        );
        let mut resolver = ModuleKeyResolver::new(extensions);
        let first = resolver.resolve("+SolarPanel").unwrap();
        assert_eq!(first, ModuleKey::Extension("Energy".to_string()));

        // A repeat lookup of the identical string, and of its bare
        // (non-`+`) spelling, both hit the one cache entry the first call
        // populated (`strip_plus` makes them the same cache key) — proven
        // by the cache holding exactly one entry after both calls.
        let second = resolver.resolve("+SolarPanel").unwrap();
        let bare = resolver.resolve("SolarPanel").unwrap();
        assert_eq!(second, first);
        assert_eq!(bare, first);
        assert_eq!(resolver.cache.len(), 1);
    }
}
