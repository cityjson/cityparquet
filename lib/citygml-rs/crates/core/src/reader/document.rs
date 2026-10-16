//! What one pass over a whole document finds ahead of its objects.

use std::collections::BTreeMap;
use std::sync::Arc;

use super::templates::{TemplateIndex, Templates};
use crate::GmlObject;
use crate::model::{
    AdeIndex, GroupIndex, ModelAppearance, Namespace, Template, XmlContent, XmlNode,
};

/// What an object needs but may come after it in the document: the
/// CityModel-level appearance, which groups reference which members, the
/// relative geometries its `ImplicitGeometry`s place, and the ADEs it uses,
/// with their schemas where the document names them by a relative path
/// ([`AdeIndex::read_schemas`]). A per-version reader fills it in one pass.
#[derive(Debug, Default)]
pub struct Document {
    appearance: ModelAppearance,
    groups_of: GroupIndex,
    templates: Templates,
    ade: Arc<AdeIndex>,
}

impl Document {
    pub fn new(
        appearance: ModelAppearance,
        groups_of: GroupIndex,
        templates: Templates,
        ade: AdeIndex,
    ) -> Self {
        Self {
            appearance,
            groups_of,
            templates,
            ade: Arc::new(ade),
        }
    }

    /// The relative geometries the document's `ImplicitGeometry`s place, in
    /// document order, each with the CityModel-level appearance that targets
    /// it (none when read without it). An [`Instance`]'s `template` is a
    /// position in this list.
    ///
    /// [`Instance`]: crate::model::Instance
    pub fn templates(&self) -> &[Template] {
        &self.templates.templates
    }

    /// Where each of [`Self::templates`] is in the document.
    pub fn template_index(&self) -> &TemplateIndex {
        &self.templates.index
    }

    /// The ADEs the document uses.
    pub fn ade(&self) -> &Arc<AdeIndex> {
        &self.ade
    }

    /// Finish an object the walker read: give it the CityModel-level
    /// appearance that targets it and the groups that reference it, and
    /// count its ADE properties that hold GML geometry in `ade_geometries`.
    pub fn complete(&self, object: &mut GmlObject, ade_geometries: &mut BTreeMap<String, usize>) {
        object.scope_model_appearance(&self.appearance);
        if !self.groups_of.is_empty() {
            object.assign_groups(&self.groups_of);
        }
        tally_ade_geometries(object, &self.ade, ade_geometries);
    }
}

/// Count `object`'s ADE properties, and its children's, that hold GML, by
/// property name (`uro:lod0Point`), the prefix being the ADE's.
fn tally_ade_geometries(object: &GmlObject, ade: &AdeIndex, tally: &mut BTreeMap<String, usize>) {
    fn holds_gml(node: &XmlNode) -> bool {
        match &node.content {
            XmlContent::Children(children) => children
                .iter()
                .any(|c| c.name.ns == Namespace::Gml || holds_gml(c)),
            _ => false,
        }
    }
    for node in object.ade.iter().filter(|n| holds_gml(n)) {
        let name = match &node.name.ns {
            Namespace::Other(uri) => match ade.prefix(uri) {
                Some(prefix) => format!("{prefix}:{}", node.name.local),
                None => node.name.local.clone(),
            },
            _ => node.name.local.clone(),
        };
        *tally.entry(name).or_insert(0) += 1;
    }
    for child in &object.children {
        tally_ade_geometries(child, ade, tally);
    }
}
