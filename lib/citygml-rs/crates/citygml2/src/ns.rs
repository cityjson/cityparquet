//! CityGML 2.0 namespace URIs and the modules they name.

use citygml_core::model::{Module, Namespace};

pub const CORE: &str = "http://www.opengis.net/citygml/2.0";
pub const APPEARANCE: &str = "http://www.opengis.net/citygml/appearance/2.0";
pub const BRIDGE: &str = "http://www.opengis.net/citygml/bridge/2.0";
pub const BUILDING: &str = "http://www.opengis.net/citygml/building/2.0";
pub const CITY_FURNITURE: &str = "http://www.opengis.net/citygml/cityfurniture/2.0";
pub const CITY_OBJECT_GROUP: &str = "http://www.opengis.net/citygml/cityobjectgroup/2.0";
pub const GENERICS: &str = "http://www.opengis.net/citygml/generics/2.0";
pub const LAND_USE: &str = "http://www.opengis.net/citygml/landuse/2.0";
pub const RELIEF: &str = "http://www.opengis.net/citygml/relief/2.0";
pub const TRANSPORTATION: &str = "http://www.opengis.net/citygml/transportation/2.0";
pub const TUNNEL: &str = "http://www.opengis.net/citygml/tunnel/2.0";
pub const VEGETATION: &str = "http://www.opengis.net/citygml/vegetation/2.0";
pub const WATER_BODY: &str = "http://www.opengis.net/citygml/waterbody/2.0";

const MODULES: &[(&str, Module)] = &[
    (CORE, Module::Core),
    (APPEARANCE, Module::Appearance),
    (BRIDGE, Module::Bridge),
    (BUILDING, Module::Building),
    (CITY_FURNITURE, Module::CityFurniture),
    (CITY_OBJECT_GROUP, Module::CityObjectGroup),
    (GENERICS, Module::Generics),
    (LAND_USE, Module::LandUse),
    (RELIEF, Module::Relief),
    (TRANSPORTATION, Module::Transportation),
    (TUNNEL, Module::Tunnel),
    (VEGETATION, Module::Vegetation),
    (WATER_BODY, Module::WaterBody),
];

/// The module a CityGML 2.0 namespace URI names.
pub fn module(uri: &[u8]) -> Option<Module> {
    MODULES
        .iter()
        .find(|(ns, _)| ns.as_bytes() == uri)
        .map(|(_, m)| *m)
}

/// The model namespace of a namespace URI: a CityGML 2.0 module, GML, or
/// any other namespace by its URI.
pub fn namespace(uri: &[u8]) -> Namespace {
    if let Some(m) = module(uri) {
        return Namespace::Module(m);
    }
    if uri.starts_with(citygml_core::xml::NS_GML.as_bytes()) {
        return Namespace::Gml;
    }
    Namespace::Other(String::from_utf8_lossy(uri).into_owned())
}
