//! What a 3DCityDB instance says about its own vocabulary: the namespaces,
//! the feature classes and the property datatypes, each by the id its rows
//! use.

use std::collections::HashMap;

use citygml_core::Result;
use citygml_core::model::{Module, Namespace, QName};
use citygml_core::vocabulary::module_of_prefix;

use crate::db::Session;

/// Where a name of the database comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Ns {
    /// A CityGML 3.0 module.
    Module(Module),
    /// 3DCityDB's `depr` namespace: CityGML 2.0 properties CityGML 3.0
    /// dropped, such as `lod4Solid` and `lod0RoofEdge`.
    Deprecated,
    /// An ADE's, by its URI.
    Ade(String),
}

impl Ns {
    /// The namespace a name of this namespace has in the model. A
    /// deprecated property is read as the core module's: CityGML 2.0 spelled
    /// its `lodN…` geometry properties in each thematic module, and CityGML
    /// 3.0's are the core module's.
    pub(crate) fn namespace(&self) -> Namespace {
        match self {
            Self::Module(module) => Namespace::Module(*module),
            Self::Deprecated => Namespace::Module(Module::Core),
            Self::Ade(uri) => Namespace::Other(uri.clone()),
        }
    }
}

/// A feature class.
#[derive(Debug, Clone)]
pub(crate) struct Class {
    pub(crate) name: QName,
    /// `bldg:Building`: how a count of skipped objects names the class.
    pub(crate) prefixed: String,
}

#[derive(Debug, Default)]
pub(crate) struct Catalogue {
    namespaces: HashMap<i32, Ns>,
    classes: HashMap<i32, Class>,
    datatypes: HashMap<i32, String>,
}

impl Catalogue {
    pub(crate) fn load(session: &mut Session) -> Result<Self> {
        let mut catalogue = Self::default();
        let mut aliases: HashMap<i32, String> = HashMap::new();
        for row in session.query("SELECT id, alias, namespace FROM {s}.namespace", &[])? {
            let id: i32 = row.get(0);
            let alias: Option<String> = row.get(1);
            let uri: Option<String> = row.get(2);
            let alias = alias.unwrap_or_default();
            let ns = match alias.as_str() {
                "depr" => Ns::Deprecated,
                a if is_citydb_namespace(uri.as_deref()) => match module_of_prefix(a) {
                    Some(module) => Ns::Module(module),
                    None => Ns::Ade(uri.unwrap_or_default()),
                },
                _ => Ns::Ade(uri.unwrap_or_default()),
            };
            aliases.insert(id, alias);
            catalogue.namespaces.insert(id, ns);
        }
        for row in session.query(
            "SELECT id, classname, namespace_id FROM {s}.objectclass",
            &[],
        )? {
            let id: i32 = row.get(0);
            let local: Option<String> = row.get(1);
            let namespace: Option<i32> = row.get(2);
            let local = local.unwrap_or_default();
            let ns = namespace.and_then(|n| catalogue.namespaces.get(&n));
            let name = QName::new(
                ns.map_or(Namespace::Module(Module::Core), Ns::namespace),
                local.clone(),
            );
            let prefix = namespace.and_then(|n| aliases.get(&n)).cloned();
            let prefixed = match prefix {
                Some(p) if !p.is_empty() => format!("{p}:{local}"),
                _ => local,
            };
            catalogue.classes.insert(id, Class { name, prefixed });
        }
        for row in session.query("SELECT id, typename FROM {s}.datatype", &[])? {
            let id: i32 = row.get(0);
            let name: Option<String> = row.get(1);
            catalogue.datatypes.insert(id, name.unwrap_or_default());
        }
        Ok(catalogue)
    }

    pub(crate) fn namespace(&self, id: Option<i32>) -> Option<&Ns> {
        self.namespaces.get(&id?)
    }

    pub(crate) fn class(&self, id: i32) -> Option<&Class> {
        self.classes.get(&id)
    }

    /// A datatype's name (`Code`, `Height`, `GenericAttributeSet`).
    pub(crate) fn datatype(&self, id: i32) -> &str {
        self.datatypes.get(&id).map_or("", String::as_str)
    }
}

/// Whether a namespace URI is one of 3DCityDB's own, which stand for the
/// CityGML 3.0 modules: `http://3dcitydb.org/3dcitydb/building/5.0`.
fn is_citydb_namespace(uri: Option<&str>) -> bool {
    uri.is_some_and(|u| u.starts_with("http://3dcitydb.org/3dcitydb/"))
}
