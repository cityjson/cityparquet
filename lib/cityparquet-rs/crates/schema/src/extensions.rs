//! Extension namespaces (spec "Extensions"): every name an extension adds to
//! a package — an attribute column, a semantic surface `type` inside
//! `geometry_properties.surfaces`, an `object_type` value, an extension module
//! file — carries the extension's short namespace as a `<namespace>_` prefix.
//! Core names are never prefixed.
//!
//! This module is the rule as pure functions: deriving a namespace from a
//! CityJSON Extension's name, reading a source's `extensions` declarations
//! into the `city.extensions` footer object, attributing a CityJSON `+` name
//! to its extension, and restoring the `+` on export.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use crate::error::{CityParquetError, Result};
use crate::metadata::{
    AdeLayout, ExtensionAdds, ExtensionDeclaration, ExtensionDeclarations, XmlAttributes,
};
use crate::types::{ExtensionClassDecl, ExtensionRegistry};

/// Derives an extension's namespace from its source name (spec "Extensions",
/// namespace derivation): the name lower-cased, with every character outside
/// `[a-z0-9]` removed (`"Energy"` → `energy`, `"Energy ADE"` → `energyade`).
/// A result that is empty or does not start with a letter fails the
/// namespace grammar `^[a-z][a-z0-9]*$` and is a hard error.
pub fn derive_namespace(name: &str) -> Result<String> {
    let namespace: String = name
        .chars()
        .flat_map(char::to_lowercase)
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        .collect();
    if !namespace.starts_with(|c: char| c.is_ascii_lowercase()) {
        return Err(CityParquetError::Schema(format!(
            "extension '{name}': derived namespace '{namespace}' is not of the form \
             ^[a-z][a-z0-9]*$"
        )));
    }
    Ok(namespace)
}

/// Reads a CityJSON document's `extensions` member (a map from extension
/// name to `{url, version}`, and for a CityGML ADE `xmlns`, `xmlAttributes`
/// and the [`AdeLayout`] members) into the `city.extensions` footer object, keyed by
/// derived namespace. `None` (no member) stays `None`; an empty object
/// stays an empty map. A declaration without a string `url`, an empty or
/// ill-formed namespace, and two extensions deriving the same namespace are
/// hard errors.
pub fn declarations_from_cityjson(
    extensions: Option<&Value>,
) -> Result<Option<ExtensionDeclarations>> {
    let Some(extensions) = extensions else {
        return Ok(None);
    };
    let members = extensions.as_object().ok_or_else(|| {
        CityParquetError::Schema(format!(
            "the CityJSON 'extensions' member is not an object: {extensions}"
        ))
    })?;
    let mut declarations = ExtensionDeclarations::new();
    for (name, declaration) in members {
        let url = declaration
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                CityParquetError::Schema(format!(
                    "extension '{name}' declares no 'url' for its schema document"
                ))
            })?;
        let version = declaration
            .get("version")
            .and_then(Value::as_str)
            .map(str::to_string);
        let xmlns = declaration
            .get("xmlns")
            .and_then(Value::as_str)
            .map(str::to_string);
        let xml_attributes = declaration
            .get("xmlAttributes")
            .map(|v| {
                serde_json::from_value::<XmlAttributes>(v.clone()).map_err(|e| {
                    CityParquetError::Schema(format!(
                        "extension '{name}': 'xmlAttributes' is not an object of objects of \
                         strings: {e}"
                    ))
                })
            })
            .transpose()?;
        let adds = serde_json::from_value::<ExtensionAdds>(declaration.clone())
            .map_err(|e| CityParquetError::Schema(format!("extension '{name}': {e}")))?;
        let layout = serde_json::from_value::<AdeLayout>(declaration.clone())
            .map_err(|e| CityParquetError::Schema(format!("extension '{name}': {e}")))?;
        let namespace = derive_namespace(name)?;
        if let Some(other) = declarations.get(&namespace) {
            return Err(CityParquetError::Schema(format!(
                "extensions '{}' and '{name}' both derive the namespace '{namespace}'",
                other.name
            )));
        }
        declarations.insert(
            namespace,
            ExtensionDeclaration {
                name: name.clone(),
                url: url.to_string(),
                version,
                xmlns,
                xml_attributes,
                adds,
                layout,
            },
        );
    }
    Ok(Some(declarations))
}

/// Rebuilds a CityJSON `extensions` member from `city.extensions`: each
/// declaration's `name` becomes the key, carrying its `url` and, when
/// present, its `version`, `xmlns`, `xmlAttributes`, the members naming
/// what it adds and those saying where a CityGML writer puts it.
pub fn cityjson_extensions_member(declarations: &ExtensionDeclarations) -> Value {
    let members = declarations
        .values()
        .map(|d| {
            let mut entry = serde_json::Map::new();
            entry.insert("url".to_string(), Value::String(d.url.clone()));
            if let Some(version) = &d.version {
                entry.insert("version".to_string(), Value::String(version.clone()));
            }
            if let Some(xmlns) = &d.xmlns {
                entry.insert("xmlns".to_string(), Value::String(xmlns.clone()));
            }
            if let Some(attributes) = &d.xml_attributes {
                entry.insert(
                    "xmlAttributes".to_string(),
                    serde_json::to_value(attributes).unwrap_or_default(),
                );
            }
            for (member, value) in d.adds.members().into_iter().chain(d.layout.members()) {
                if let Some(value) = value {
                    entry.insert(member.to_string(), value.clone());
                }
            }
            (d.name.clone(), Value::Object(entry))
        })
        .collect();
    Value::Object(members)
}

/// What a name names: the member of a CityJSON Extension document that
/// lists such names, and the words an error message uses for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum NameKind {
    /// A City Object attribute (`extraAttributes`).
    Attribute,
    /// A City Object type (`extraCityObjects`).
    CityObjectType,
    /// A semantic surface type (`extraSemanticSurfaces`).
    SemanticSurfaceType,
    /// A root property (`extraRootProperties`).
    RootProperty,
}

impl NameKind {
    fn member(self) -> &'static str {
        match self {
            NameKind::Attribute => "extraAttributes",
            NameKind::CityObjectType => "extraCityObjects",
            NameKind::SemanticSurfaceType => "extraSemanticSurfaces",
            NameKind::RootProperty => "extraRootProperties",
        }
    }
}

impl std::fmt::Display for NameKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            NameKind::Attribute => "attribute",
            NameKind::CityObjectType => "city-object type",
            NameKind::SemanticSurfaceType => "semantic surface type",
            NameKind::RootProperty => "root property",
        })
    }
}

/// Which declared extensions add each name, keyed by its kind and the name
/// without its `+`, valued by namespace: what [`ExtensionNaming`]
/// attributes a `+` name by when a source declares several extensions.
/// Read from each declaration's [`ExtensionAdds`] members; the keys of
/// `extraAttributes` are City Object types, whose keys are the names.
fn attribution(
    declarations: &ExtensionDeclarations,
) -> BTreeMap<(NameKind, String), BTreeSet<String>> {
    let mut attribution: BTreeMap<(NameKind, String), BTreeSet<String>> = BTreeMap::new();
    for (namespace, declaration) in declarations {
        for kind in [
            NameKind::Attribute,
            NameKind::CityObjectType,
            NameKind::SemanticSurfaceType,
            NameKind::RootProperty,
        ] {
            let Some(names) = declaration
                .adds
                .members()
                .into_iter()
                .find(|(member, _)| *member == kind.member())
                .and_then(|(_, value)| value?.as_object())
            else {
                continue;
            };
            let names: Vec<&String> = if kind == NameKind::Attribute {
                names
                    .values()
                    .filter_map(Value::as_object)
                    .flat_map(|by_type| by_type.keys())
                    .collect()
            } else {
                names.keys().collect()
            };
            for name in names {
                attribution
                    .entry((kind, name.strip_prefix('+').unwrap_or(name).to_string()))
                    .or_default()
                    .insert(namespace.clone());
            }
        }
    }
    attribution
}

/// The naming policy one package's declared extensions impose (spec
/// "Extensions"): which namespace a CityJSON `+` name is attributed to on
/// write, which names are reserved for extensions, and how export restores
/// the `+`. Built once per conversion from `city.extensions`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExtensionNaming {
    /// Declared namespace → the extension's source name, for messages.
    declared: BTreeMap<String, String>,
    /// The names each declared extension adds, by kind and name without
    /// `+`, for a source declaring several.
    attribution: BTreeMap<(NameKind, String), BTreeSet<String>>,
}

impl ExtensionNaming {
    /// The policy for a package declaring `declarations` (`None` and an
    /// empty map both declare no extension).
    pub fn new(declarations: Option<&ExtensionDeclarations>) -> Self {
        let declared = declarations
            .into_iter()
            .flatten()
            .map(|(namespace, d)| (namespace.clone(), d.name.clone()))
            .collect();
        Self {
            declared,
            attribution: declarations.map(attribution).unwrap_or_default(),
        }
    }

    /// The declared namespace whose `<ns>_` prefix `name` carries, if any.
    fn namespace_of<'a>(&self, name: &'a str) -> Option<(&str, &'a str)> {
        self.declared.keys().find_map(|namespace| {
            name.strip_prefix(namespace.as_str())
                .and_then(|rest| rest.strip_prefix('_'))
                .map(|rest| (namespace.as_str(), rest))
        })
    }

    /// The declared namespace whose `<ns>_` prefix the stored `name` carries.
    pub fn namespace(&self, name: &str) -> Option<&str> {
        self.namespace_of(name).map(|(namespace, _)| namespace)
    }

    /// Whether `name` carries a declared namespace prefix (`<ns>_…`).
    pub fn is_extension_name(&self, name: &str) -> bool {
        self.namespace_of(name).is_some()
    }

    /// The stored form of a source name (an attribute, a semantic surface
    /// `type` or a city-object class). A CityJSON `+name` is attributed to
    /// its extension and becomes `<ns>_name`: the sole declared extension,
    /// or among several the one whose declaration lists the name (its
    /// [`ExtensionAdds`]). A source that declares no extension, or
    /// several of which none or more than one lists the name, cannot
    /// attribute it and is a hard error. A name without `+` is a core name
    /// and is returned unchanged, unless it begins with a declared
    /// namespace's prefix, which would make it indistinguishable from an
    /// extension name on export — a hard error. `kind` names what `name` is
    /// in any error message, and among several extensions which of their
    /// members lists it.
    pub fn encode_name(&self, name: &str, kind: NameKind) -> Result<String> {
        let Some(rest) = name.strip_prefix('+') else {
            if let Some((namespace, _)) = self.namespace_of(name) {
                return Err(CityParquetError::Schema(format!(
                    "{kind} '{name}' is not an extension name but begins with the declared \
                     extension namespace prefix '{namespace}_', so it could not be told apart \
                     from an extension name"
                )));
            }
            return Ok(name.to_string());
        };
        let mut declared = self.declared.iter();
        match (declared.next(), declared.next()) {
            (Some((namespace, _)), None) => Ok(format!("{namespace}_{rest}")),
            (None, _) => Err(CityParquetError::Schema(format!(
                "{kind} '{name}' is an extension name, but the source does not declare any \
                 extension: CityJSON requires every extension to be declared in the \
                 'extensions' member"
            ))),
            (Some(_), Some(_)) => {
                let listed: Vec<&str> = self
                    .attribution
                    .get(&(kind, rest.to_string()))
                    .into_iter()
                    .flatten()
                    .filter(|namespace| self.declared.contains_key(*namespace))
                    .map(String::as_str)
                    .collect();
                match listed.as_slice() {
                    [namespace] => Ok(format!("{namespace}_{rest}")),
                    [] => Err(CityParquetError::Schema(format!(
                        "{kind} '{name}' is an extension name, and the source declares several \
                         extensions ({}), none of whose declarations lists it in '{}'; reading \
                         the extensions' schema documents to attribute it is not implemented",
                        self.declared
                            .values()
                            .map(String::as_str)
                            .collect::<Vec<_>>()
                            .join(", "),
                        kind.member()
                    ))),
                    several => Err(CityParquetError::Schema(format!(
                        "{kind} '{name}' is an extension name that several declared extensions \
                         ({}) list, so it belongs to none of them alone",
                        several.join(", ")
                    ))),
                }
            }
        }
    }

    /// The CityJSON form of a stored name: a declared namespace prefix
    /// becomes the `+` marker (`energy_heatCapacity` → `+heatCapacity`);
    /// every other name is returned unchanged.
    pub fn decode_name(&self, name: &str) -> String {
        match self.namespace_of(name) {
            Some((_, rest)) => format!("+{rest}"),
            None => name.to_string(),
        }
    }
}

/// The extension classes `declarations` say specialise another class, for
/// [`crate::types::ModuleKeyResolver`] to route by: each `extraCityObjects`
/// entry whose schema refers, through `allOf`, to a definition of CityJSON's
/// `cityobjects.schema.json`, the way a CityJSON Extension declares the
/// class it builds on (`{"allOf": [{"$ref":
/// "cityobjects.schema.json#/_AbstractBuilding"}, …]}`). A class with no
/// such reference is not declared, and goes to its extension's own file. A
/// class two extensions both list is an error: it could not be told which
/// one's declaration routes it.
pub fn class_registry(declarations: Option<&ExtensionDeclarations>) -> Result<ExtensionRegistry> {
    let mut registry = ExtensionRegistry::new();
    let mut listed: BTreeMap<&str, &str> = BTreeMap::new();
    for (namespace, declaration) in declarations.into_iter().flatten() {
        let Some(Value::Object(classes)) = &declaration.adds.extra_city_objects else {
            continue;
        };
        for (class, schema) in classes {
            if let Some(other) = listed.insert(class.trim_start_matches('+'), namespace) {
                return Err(CityParquetError::Schema(format!(
                    "the extensions '{other}' and '{namespace}' both list the class '{class}'"
                )));
            }
            let parent = schema
                .get("allOf")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|part| part.get("$ref").and_then(Value::as_str))
                .find_map(|reference| reference.split_once("cityobjects.schema.json#/"))
                .and_then(|(_, path)| path.split('/').next())
                .map(str::to_string);
            if let Some(parent) = parent {
                registry.declare(
                    class,
                    ExtensionClassDecl {
                        namespace: namespace.clone(),
                        module: None,
                        parent: Some(parent),
                    },
                );
            }
        }
    }
    Ok(registry)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn energy() -> ExtensionDeclarations {
        declarations_from_cityjson(Some(&json!({
            "Energy": {"url": "https://example.org/energy.ext.json", "version": "3.0"}
        })))
        .unwrap()
        .unwrap()
    }

    #[test]
    fn namespace_is_the_lower_cased_alphanumeric_name() {
        assert_eq!(derive_namespace("Energy").unwrap(), "energy");
        assert_eq!(derive_namespace("Noise").unwrap(), "noise");
        assert_eq!(derive_namespace("Energy ADE").unwrap(), "energyade");
        assert_eq!(derive_namespace("my-ext_2").unwrap(), "myext2");
    }

    #[test]
    fn an_empty_or_digit_led_namespace_is_a_hard_error() {
        for bad in ["", "+-_", "3D"] {
            let e = derive_namespace(bad).unwrap_err();
            assert!(matches!(e, CityParquetError::Schema(_)), "{bad:?}: {e:?}");
        }
    }

    #[test]
    fn declarations_are_keyed_by_namespace() {
        let decls = energy();
        assert_eq!(decls.len(), 1);
        let d = &decls["energy"];
        assert_eq!(d.name, "Energy");
        assert_eq!(d.url, "https://example.org/energy.ext.json");
        assert_eq!(d.version.as_deref(), Some("3.0"));
        assert_eq!(d.xmlns, None);
    }

    #[test]
    fn absent_and_empty_declarations_are_kept_apart() {
        assert_eq!(declarations_from_cityjson(None).unwrap(), None);
        assert_eq!(
            declarations_from_cityjson(Some(&json!({}))).unwrap(),
            Some(ExtensionDeclarations::new())
        );
    }

    #[test]
    fn two_extensions_deriving_one_namespace_is_a_hard_error() {
        let e = declarations_from_cityjson(Some(&json!({
            "Energy": {"url": "https://a.example/e.json"},
            "ENERGY": {"url": "https://b.example/e.json"}
        })))
        .unwrap_err();
        assert!(matches!(e, CityParquetError::Schema(_)));
        assert!(e.to_string().contains("energy"), "{e}");
    }

    #[test]
    fn a_declaration_without_a_url_is_a_hard_error() {
        let e =
            declarations_from_cityjson(Some(&json!({"Energy": {"version": "3.0"}}))).unwrap_err();
        assert!(matches!(e, CityParquetError::Schema(_)));
        assert!(e.to_string().contains("Energy"), "{e}");
    }

    #[test]
    fn the_cityjson_member_is_rebuilt_from_the_declarations() {
        let source = json!({
            "Energy": {"url": "https://example.org/energy.ext.json", "version": "3.0"},
            "Noise": {"url": "https://example.org/noise.ext.json"},
            "uro": {"url": "https://example.org/uro.xsd", "version": "3.2",
                    "xmlns": "https://example.org/uro/3.2",
                    "xmlAttributes": {"width": {"uom": "m"}}}
        });
        let decls = declarations_from_cityjson(Some(&source)).unwrap().unwrap();
        assert_eq!(cityjson_extensions_member(&decls), source);
    }

    #[test]
    fn a_plus_name_takes_the_sole_declared_namespace() {
        let naming = ExtensionNaming::new(Some(&energy()));
        assert_eq!(
            naming
                .encode_name("+heatCapacity", NameKind::Attribute)
                .unwrap(),
            "energy_heatCapacity"
        );
        assert_eq!(
            naming
                .encode_name("+PartyWallSurface", NameKind::SemanticSurfaceType)
                .unwrap(),
            "energy_PartyWallSurface"
        );
        assert_eq!(
            naming
                .encode_name("yearOfConstruction", NameKind::Attribute)
                .unwrap(),
            "yearOfConstruction"
        );
    }

    #[test]
    fn a_plus_name_without_a_declared_extension_is_a_hard_error() {
        for naming in [
            ExtensionNaming::new(None),
            ExtensionNaming::new(Some(&ExtensionDeclarations::new())),
        ] {
            let e = naming
                .encode_name("+heatCapacity", NameKind::Attribute)
                .unwrap_err();
            assert!(matches!(e, CityParquetError::Schema(_)));
            let msg = e.to_string();
            assert!(msg.contains("+heatCapacity"), "{msg}");
            assert!(msg.contains("declare"), "{msg}");
        }
    }

    #[test]
    fn a_plus_name_with_several_declared_extensions_is_a_hard_error() {
        let decls = declarations_from_cityjson(Some(&json!({
            "Energy": {"url": "https://example.org/energy.ext.json"},
            "Noise": {"url": "https://example.org/noise.ext.json"}
        })))
        .unwrap()
        .unwrap();
        let e = ExtensionNaming::new(Some(&decls))
            .encode_name("+heatCapacity", NameKind::Attribute)
            .unwrap_err();
        assert!(matches!(e, CityParquetError::Schema(_)));
        let msg = e.to_string();
        for needle in ["+heatCapacity", "Energy", "Noise", "not implemented"] {
            assert!(msg.contains(needle), "missing {needle:?} in {msg}");
        }
    }

    /// Among several declared extensions, a `+` name belongs to the one
    /// whose declaration lists it as a name of its kind, and a name two of
    /// them list as one kind to neither.
    #[test]
    fn a_plus_name_among_several_extensions_belongs_to_the_one_listing_it() {
        let source = json!({
            "Energy": {"url": "https://example.org/energy.ext.json",
                       "extraCityObjects": {"+ThermalZone": {}},
                       "extraAttributes": {"Building": {"+heatCapacity": {}, "+level": {},
                                                        "+NoiseBarrier": {}}}},
            "Noise": {"url": "https://example.org/noise.ext.json",
                      "extraSemanticSurfaces": {"+NoiseBarrier": {}},
                      "extraAttributes": {"Road": {"+level": {}}}}
        });
        let decls = declarations_from_cityjson(Some(&source)).unwrap().unwrap();
        assert_eq!(cityjson_extensions_member(&decls), source);
        let naming = ExtensionNaming::new(Some(&decls));
        for (name, kind, stored) in [
            (
                "+ThermalZone",
                NameKind::CityObjectType,
                "energy_ThermalZone",
            ),
            ("+heatCapacity", NameKind::Attribute, "energy_heatCapacity"),
            ("+NoiseBarrier", NameKind::Attribute, "energy_NoiseBarrier"),
            (
                "+NoiseBarrier",
                NameKind::SemanticSurfaceType,
                "noise_NoiseBarrier",
            ),
        ] {
            assert_eq!(naming.encode_name(name, kind).unwrap(), stored);
        }
        assert_eq!(naming.namespace("noise_NoiseBarrier"), Some("noise"));
        let e = naming
            .encode_name("+level", NameKind::Attribute)
            .unwrap_err();
        assert!(e.to_string().contains("energy, noise"), "{e}");
    }

    #[test]
    fn a_core_name_carrying_a_declared_prefix_is_a_hard_error() {
        let naming = ExtensionNaming::new(Some(&energy()));
        let e = naming
            .encode_name("energy_label", NameKind::Attribute)
            .unwrap_err();
        assert!(matches!(e, CityParquetError::Schema(_)));
        assert!(e.to_string().contains("energy_label"), "{e}");
        // Without the declaration the same name is an ordinary core name.
        assert_eq!(
            ExtensionNaming::new(None)
                .encode_name("energy_label", NameKind::Attribute)
                .unwrap(),
            "energy_label"
        );
    }

    #[test]
    fn decode_restores_the_plus_marker_for_declared_prefixes_only() {
        let naming = ExtensionNaming::new(Some(&energy()));
        assert!(naming.is_extension_name("energy_heatCapacity"));
        assert!(!naming.is_extension_name("heatCapacity"));
        assert_eq!(naming.decode_name("energy_heatCapacity"), "+heatCapacity");
        assert_eq!(
            naming.decode_name("energy_PartyWallSurface"),
            "+PartyWallSurface"
        );
        assert_eq!(naming.decode_name("energyheat"), "energyheat");
        assert_eq!(naming.decode_name("noise_level"), "noise_level");
        assert_eq!(
            ExtensionNaming::new(None).decode_name("energy_heatCapacity"),
            "energy_heatCapacity"
        );
    }
}
