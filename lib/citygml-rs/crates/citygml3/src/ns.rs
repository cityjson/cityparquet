//! CityGML 3.0 namespace URIs and the modules they name.

use citygml_core::model::{Module, Namespace};

pub const CORE: &str = "http://www.opengis.net/citygml/3.0";
pub const APPEARANCE: &str = "http://www.opengis.net/citygml/appearance/3.0";
pub const BRIDGE: &str = "http://www.opengis.net/citygml/bridge/3.0";
pub const BUILDING: &str = "http://www.opengis.net/citygml/building/3.0";
pub const CITY_FURNITURE: &str = "http://www.opengis.net/citygml/cityfurniture/3.0";
pub const CITY_OBJECT_GROUP: &str = "http://www.opengis.net/citygml/cityobjectgroup/3.0";
pub const CONSTRUCTION: &str = "http://www.opengis.net/citygml/construction/3.0";
pub const DYNAMIZER: &str = "http://www.opengis.net/citygml/dynamizer/3.0";
pub const GENERICS: &str = "http://www.opengis.net/citygml/generics/3.0";
pub const LAND_USE: &str = "http://www.opengis.net/citygml/landuse/3.0";
pub const POINT_CLOUD: &str = "http://www.opengis.net/citygml/pointcloud/3.0";
pub const RELIEF: &str = "http://www.opengis.net/citygml/relief/3.0";
pub const TRANSPORTATION: &str = "http://www.opengis.net/citygml/transportation/3.0";
pub const TUNNEL: &str = "http://www.opengis.net/citygml/tunnel/3.0";
pub const VEGETATION: &str = "http://www.opengis.net/citygml/vegetation/3.0";
pub const VERSIONING: &str = "http://www.opengis.net/citygml/versioning/3.0";
pub const WATER_BODY: &str = "http://www.opengis.net/citygml/waterbody/3.0";

const MODULES: &[(&str, Module)] = &[
    (CORE, Module::Core),
    (APPEARANCE, Module::Appearance),
    (BRIDGE, Module::Bridge),
    (BUILDING, Module::Building),
    (CITY_FURNITURE, Module::CityFurniture),
    (CITY_OBJECT_GROUP, Module::CityObjectGroup),
    (CONSTRUCTION, Module::Construction),
    (DYNAMIZER, Module::Dynamizer),
    (GENERICS, Module::Generics),
    (LAND_USE, Module::LandUse),
    (POINT_CLOUD, Module::PointCloud),
    (RELIEF, Module::Relief),
    (TRANSPORTATION, Module::Transportation),
    (TUNNEL, Module::Tunnel),
    (VEGETATION, Module::Vegetation),
    (VERSIONING, Module::Versioning),
    (WATER_BODY, Module::WaterBody),
];

/// The module a CityGML 3.0 namespace URI names.
pub fn module(uri: &[u8]) -> Option<Module> {
    MODULES
        .iter()
        .find(|(ns, _)| ns.as_bytes() == uri)
        .map(|(_, m)| *m)
}

/// The model namespace of a namespace URI: a CityGML 3.0 module, GML, or
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
