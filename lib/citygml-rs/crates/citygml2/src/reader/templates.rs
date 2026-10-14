//! The relative geometries a document's `ImplicitGeometry`s instantiate.
//!
//! A relative geometry is written inline in the first `relativeGMLGeometry`
//! that uses it, inside some object, and referenced by `xlink:href` from the
//! others, possibly in objects read before it. So the up-front pass over the
//! document reads every inline one, and the object pass finds each again:
//! an inline one by where it starts in the file, a referenced one by the
//! `gml:id` of its geometry.

use std::collections::HashMap;
use std::io::BufRead;

use citygml_core::gml::{ObjectBuilder, XlinkBudget};
use citygml_core::model::{ModelAppearance, Module, QName, Template};
use citygml_core::{GmlObject, Result};
use quick_xml::reader::NsReader;

use super::walker;

/// Where each relative geometry of a document is.
#[derive(Debug, Default)]
pub struct TemplateIndex {
    /// The byte position just after an inline `relativeGMLGeometry`'s start
    /// tag -> its template.
    at: HashMap<u64, usize>,
    /// The `gml:id` of an inline relative geometry -> its template.
    by_id: HashMap<String, usize>,
}

impl TemplateIndex {
    /// The template defined inline at byte position `position`.
    pub fn at(&self, position: u64) -> Option<usize> {
        self.at.get(&position).copied()
    }

    /// The template whose geometry has `gml:id` `id`.
    pub fn by_id(&self, id: &str) -> Option<usize> {
        self.by_id.get(id).copied()
    }
}

/// The templates the up-front pass reads, with where each is.
#[derive(Default)]
pub struct Templates {
    pub index: TemplateIndex,
    pub templates: Vec<Template>,
}

impl Templates {
    /// Read the inline relative geometry `reader` is inside (positioned just
    /// after the `relativeGMLGeometry` start tag) as the next template, at
    /// the LoD `lod` of the property holding it.
    pub fn read<R: BufRead>(
        &mut self,
        reader: &mut NsReader<R>,
        buf: &mut Vec<u8>,
        lod: String,
    ) -> Result<()> {
        let position = reader.buffer_position();
        let (id, geom) = walker::read_lod_geometry(reader, buf, b"relativeGMLGeometry")?;
        let mut b = ObjectBuilder::new(id.clone(), QName::module(Module::Core, "ImplicitGeometry"));
        walker::add_lod_geometry(&mut b, lod, geom);
        let Some(geometry) = b.finish(&XlinkBudget::default())?.geometries.pop() else {
            return Ok(());
        };
        let n = self.templates.len();
        self.index.at.insert(position, n);
        if let Some(id) = &id {
            self.index.by_id.entry(id.clone()).or_insert(n);
        }
        self.templates.push(Template {
            id,
            geometry,
            appearance: Default::default(),
        });
        Ok(())
    }

    /// Give each template the CityModel-level materials and textures that
    /// target its faces.
    pub fn scope_model_appearance(&mut self, model: &ModelAppearance) {
        if model.is_empty() {
            return;
        }
        for template in &mut self.templates {
            let mut holder = GmlObject {
                id: template.id.clone(),
                class: QName::module(Module::Core, "ImplicitGeometry"),
                attributes: Vec::new(),
                addresses: Vec::new(),
                geometries: vec![template.geometry.clone()],
                boundary_surfaces: Vec::new(),
                appearance: Default::default(),
                children: Vec::new(),
                members: Vec::new(),
                groups: Vec::new(),
            };
            holder.scope_model_appearance(model);
            template.appearance = holder.appearance;
        }
    }
}
