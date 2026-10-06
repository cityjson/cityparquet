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

use std::collections::BTreeMap;

use serde_json::Value;

use crate::error::{CityParquetError, Result};
use crate::metadata::{ExtensionDeclaration, ExtensionDeclarations};

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
/// name to `{url, version}`) into the `city.extensions` footer object, keyed
/// by derived namespace. `None` (no member) stays `None`; an empty object
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
                xmlns: None,
            },
        );
    }
    Ok(Some(declarations))
}

/// Rebuilds a CityJSON `extensions` member from `city.extensions`: each
/// declaration's `name` becomes the key, carrying its `url` and, when
/// present, its `version`.
pub fn cityjson_extensions_member(declarations: &ExtensionDeclarations) -> Value {
    let members = declarations
        .values()
        .map(|d| {
            let mut entry = serde_json::Map::new();
            entry.insert("url".to_string(), Value::String(d.url.clone()));
            if let Some(version) = &d.version {
                entry.insert("version".to_string(), Value::String(version.clone()));
            }
            (d.name.clone(), Value::Object(entry))
        })
        .collect();
    Value::Object(members)
}

/// The naming policy one package's declared extensions impose (spec
/// "Extensions"): which namespace a CityJSON `+` name is attributed to on
/// write, which names are reserved for extensions, and how export restores
/// the `+`. Built once per conversion from `city.extensions`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExtensionNaming {
    /// Declared namespace → the extension's source name, for messages.
    declared: BTreeMap<String, String>,
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
        Self { declared }
    }

    /// The declared namespace whose `<ns>_` prefix `name` carries, if any.
    fn namespace_of<'a>(&self, name: &'a str) -> Option<(&str, &'a str)> {
        self.declared.keys().find_map(|namespace| {
            name.strip_prefix(namespace.as_str())
                .and_then(|rest| rest.strip_prefix('_'))
                .map(|rest| (namespace.as_str(), rest))
        })
    }

    /// Whether `name` carries a declared namespace prefix (`<ns>_…`).
    pub fn is_extension_name(&self, name: &str) -> bool {
        self.namespace_of(name).is_some()
    }

    /// The stored form of a source name (an attribute, a semantic surface
    /// `type` or a city-object class). A CityJSON `+name` is attributed to
    /// its extension and becomes `<ns>_name`; a source that declares no
    /// extension, or several, cannot attribute it and is a hard error (the
    /// several-extensions case needs the extensions' schema documents, which
    /// this implementation does not read). A name without `+` is a core name
    /// and is returned unchanged, unless it begins with a declared
    /// namespace's prefix, which would make it indistinguishable from an
    /// extension name on export — a hard error. `kind` names what `name` is
    /// in any error message.
    pub fn encode_name(&self, name: &str, kind: &str) -> Result<String> {
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
            (Some(_), Some(_)) => Err(CityParquetError::Schema(format!(
                "{kind} '{name}' is an extension name, and the source declares several \
                 extensions ({}): attributing it to one of them needs the extensions' schema \
                 documents, which is not implemented",
                self.declared
                    .values()
                    .map(String::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            ))),
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
            "Noise": {"url": "https://example.org/noise.ext.json"}
        });
        let decls = declarations_from_cityjson(Some(&source)).unwrap().unwrap();
        assert_eq!(cityjson_extensions_member(&decls), source);
    }

    #[test]
    fn a_plus_name_takes_the_sole_declared_namespace() {
        let naming = ExtensionNaming::new(Some(&energy()));
        assert_eq!(
            naming.encode_name("+heatCapacity", "attribute").unwrap(),
            "energy_heatCapacity"
        );
        assert_eq!(
            naming
                .encode_name("+PartyWallSurface", "semantic surface type")
                .unwrap(),
            "energy_PartyWallSurface"
        );
        assert_eq!(
            naming
                .encode_name("yearOfConstruction", "attribute")
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
                .encode_name("+heatCapacity", "attribute")
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
            .encode_name("+heatCapacity", "attribute")
            .unwrap_err();
        assert!(matches!(e, CityParquetError::Schema(_)));
        let msg = e.to_string();
        for needle in ["+heatCapacity", "Energy", "Noise", "not implemented"] {
            assert!(msg.contains(needle), "missing {needle:?} in {msg}");
        }
    }

    #[test]
    fn a_core_name_carrying_a_declared_prefix_is_a_hard_error() {
        let naming = ExtensionNaming::new(Some(&energy()));
        let e = naming.encode_name("energy_label", "attribute").unwrap_err();
        assert!(matches!(e, CityParquetError::Schema(_)));
        assert!(e.to_string().contains("energy_label"), "{e}");
        // Without the declaration the same name is an ordinary core name.
        assert_eq!(
            ExtensionNaming::new(None)
                .encode_name("energy_label", "attribute")
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
