//! What is known about ADEs without reading their schemas.

use crate::model::Module;

/// The i-UR (PLATEAU) ADEs whose schemas are published at a stable URL:
/// namespace path, schema file, and the versions published.
const IUR: &[(&str, &str)] = &[("uro", "urbanObject.xsd"), ("urf", "urbanFunction.xsd")];
const IUR_VERSIONS: &[&str] = &["2.0", "3.0", "3.1", "3.2"];

/// The published schema of a known ADE, by its namespace URI: the i-UR
/// `uro` and `urf` namespaces of versions 2.0 to 3.2, whose documents
/// usually name the schema by a path relative to themselves.
pub fn known_ade_schema(namespace: &str) -> Option<String> {
    let rest = namespace.strip_prefix("https://www.geospatial.jp/iur/")?;
    let (name, version) = rest.split_once('/')?;
    let (_, file) = IUR.iter().find(|(n, _)| *n == name)?;
    IUR_VERSIONS
        .contains(&version)
        .then(|| format!("https://www.geospatial.jp/iur/schemas/{name}/{version}/{file}"))
}

/// The prefix CityGML's own documents bind to a module's namespace, for
/// naming a CityGML element found inside an ADE property.
pub fn module_prefix(module: Module) -> &'static str {
    match module {
        Module::Core => "core",
        Module::Appearance => "app",
        Module::Bridge => "brid",
        Module::Building => "bldg",
        Module::CityFurniture => "frn",
        Module::CityObjectGroup => "grp",
        Module::Generics => "gen",
        Module::LandUse => "luse",
        Module::Relief => "dem",
        Module::Transportation => "tran",
        Module::Tunnel => "tun",
        Module::Vegetation => "veg",
        Module::WaterBody => "wtr",
    }
}
