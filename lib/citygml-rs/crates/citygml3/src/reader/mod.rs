//! Streaming reader: one [`GmlObject`] per top-level city object of a class
//! the walker reads, with its parts and sub-features.
//!
//! It holds one object subtree at a time, so memory is bounded by the largest
//! single object; the `CityModel` and its `cityObjectMember`s are walked past
//! without buffering.

mod ade;
mod attributes;
mod walker;

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::sync::Arc;

use citygml_core::gml::XlinkBudget;
use citygml_core::model::{Appearance, GroupIndex, ModelAppearance};
use citygml_core::reader::appearance::read_appearance;
use citygml_core::reader::geometry;
use citygml_core::reader::scan::{may_contain, uses_ade};
use citygml_core::reader::templates::Templates;
use citygml_core::reader::{Document, ObjectStream};
use citygml_core::xml::{
    NS_GML, get_attr_local, gml_id, member_id, ns_is, ns_of, skip_element, xml_err,
};
use citygml_core::{Error, GmlObject, Result};
use quick_xml::events::Event;
use quick_xml::reader::NsReader;

use crate::ns;

pub struct FeatureReader {
    reader: NsReader<Box<dyn BufRead + Send>>,
    buf: Vec<u8>,
    index: usize,
    done: bool,
    /// What the up-front pass read: the CityModel-level appearance, the
    /// groups' referenced members, where the relative geometries are and the
    /// ADEs. Empty for a reader opened on a stream.
    document: Arc<Document>,
    /// The document's allowance for expanding xlinks to surface aggregates,
    /// shared by every object it holds.
    xlink_budget: XlinkBudget,
    /// Whether the next `Start` is a `cityObjectMember`'s own object. Cleared
    /// as soon as that object has been seen, so [`ObjectStream::skipped_members`]
    /// counts members, never every unmapped element inside one.
    inside_member: bool,
    /// Members of a class this reader does not read, by the element name the
    /// document spells (`dem:RasterRelief`, …).
    skipped_members: BTreeMap<String, usize>,
    /// Members of an ADE's class, read, by the element name the document
    /// spells (`urf:UseDistrict`, …).
    ade_members: BTreeMap<String, usize>,
    /// ADE properties holding GML geometry, by property name.
    ade_geometries: BTreeMap<String, usize>,
}

impl FeatureReader {
    /// Open `path` for streaming.
    ///
    /// Reads the document once up front ([`read_document`]), then streams
    /// it again for the objects: two passes, unless the document has nothing
    /// the up-front pass reads.
    pub fn open(path: &Path) -> Result<Self> {
        Self::with_document(path, Arc::new(read_document(path)?))
    }

    /// Open `path` for streaming without the CityModel-level appearance and
    /// the ADE index.
    ///
    /// Objects stream identically apart from appearance defined at the
    /// CityModel level, which is left out (an object's own `appearance`
    /// is still read), and an ADE property's XML attributes, which a
    /// conversion then states where they occur rather than once for the
    /// document. Leaving both out spares holding every CityModel-level
    /// definition in memory, which a caller that only counts or measures
    /// geometry never consults, and the up-front pass altogether for a
    /// document without groups or implicit geometries.
    pub fn open_without_appearance(path: &Path) -> Result<Self> {
        Self::with_document(path, Arc::new(read_document_without_appearance(path)?))
    }

    /// Open `path` for streaming with what an earlier [`read_document`] of
    /// the same file found, so that several passes over one document share
    /// one up-front pass.
    pub fn with_document(path: &Path, document: Arc<Document>) -> Result<Self> {
        let file = File::open(path)
            .map_err(|e| Error::io(format!("cannot reopen {}", path.display()), e))?;
        Ok(Self::from_parts(Box::new(BufReader::new(file)), document))
    }

    /// Stream objects from any `BufRead` positioned at the document's first
    /// byte, in one pass, without what the up-front pass reads: the
    /// CityModel-level appearance, an object's referencing groups in
    /// [`GmlObject::groups`], its `ImplicitGeometry`s, and the ADE index (so
    /// a conversion states every XML attribute of an ADE property where it
    /// occurs). Nothing is read beyond what the objects pulled so far need.
    pub fn from_reader(input: Box<dyn BufRead + Send>) -> Self {
        Self::from_parts(input, Arc::new(Document::default()))
    }

    fn from_parts(input: Box<dyn BufRead + Send>, document: Arc<Document>) -> Self {
        let mut reader = NsReader::from_reader(input);
        reader.config_mut().expand_empty_elements = true;
        Self {
            reader,
            buf: Vec::new(),
            index: 0,
            done: false,
            document,
            xlink_budget: XlinkBudget::default(),
            inside_member: false,
            skipped_members: BTreeMap::new(),
            ade_members: BTreeMap::new(),
            ade_geometries: BTreeMap::new(),
        }
    }

    fn next_object(&mut self) -> Result<Option<GmlObject>> {
        loop {
            self.buf.clear();
            let (rr, ev) = self
                .reader
                .read_resolved_event_into(&mut self.buf)
                .map_err(xml_err)?;
            match ev {
                Event::Start(e) => {
                    let local = e.local_name();
                    let uri = ns_of(&rr).unwrap_or_default();
                    if local.as_ref() == b"cityObjectMember"
                        || walker::is_container_member(&uri, local.as_ref())
                    {
                        self.inside_member = true;
                        continue;
                    }
                    if walker::is_container(&uri, local.as_ref()) {
                        // Passed through: its members are what is read.
                        self.inside_member = false;
                        continue;
                    }
                    let ade_class = self
                        .inside_member
                        .then(|| walker::ade_class(&uri, local.as_ref()))
                        .flatten();
                    if ade_class.is_some() {
                        let name = String::from_utf8_lossy(e.name().as_ref()).into_owned();
                        *self.ade_members.entry(name).or_insert(0) += 1;
                    }
                    let Some(class) = walker::top_level_class(&uri, local.as_ref()).or(ade_class)
                    else {
                        if self.inside_member {
                            // A member of a class this reader does not read.
                            // Recorded, and descended into rather than
                            // skipped, so a mapped object nested inside it is
                            // still read.
                            let name = String::from_utf8_lossy(e.name().as_ref()).into_owned();
                            *self.skipped_members.entry(name).or_insert(0) += 1;
                            self.inside_member = false;
                        }
                        continue;
                    };
                    let id = gml_id(&e);
                    let end = local.as_ref().to_vec();
                    let cx = walker::Cx {
                        depth: 0,
                        templates: self.document.template_index(),
                    };
                    let builder =
                        walker::read_object(&mut self.reader, &mut self.buf, class, id, &end, cx)?;
                    self.index += 1;
                    self.inside_member = false;
                    let mut object = builder.finish(&self.xlink_budget)?;
                    self.document
                        .complete(&mut object, &mut self.ade_geometries);
                    return Ok(Some(object));
                }
                Event::End(e) => {
                    // An empty `<cityObjectMember/>` never reaches an object,
                    // and a CityModel child after the last member is not a
                    // member.
                    if matches!(
                        e.local_name().as_ref(),
                        b"cityObjectMember" | b"reliefComponent"
                    ) {
                        self.inside_member = false;
                    }
                }
                Event::Eof => {
                    self.done = true;
                    return Ok(None);
                }
                _ => {}
            }
        }
    }
}

impl Iterator for FeatureReader {
    type Item = Result<GmlObject>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        self.next_object().transpose()
    }
}

impl ObjectStream for FeatureReader {
    fn document(&self) -> &Arc<Document> {
        &self.document
    }

    fn emitted_members(&self) -> usize {
        self.index
    }

    fn skipped_members(&self) -> &BTreeMap<String, usize> {
        &self.skipped_members
    }

    fn ade_members(&self) -> &BTreeMap<String, usize> {
        &self.ade_members
    }

    fn ade_geometries(&self) -> &BTreeMap<String, usize> {
        &self.ade_geometries
    }
}

#[cfg(feature = "cjseq")]
impl FeatureReader {
    /// The objects as `cjseq::CityJSONFeature`s, each with its own vertex
    /// pool quantised against `transform`.
    pub fn features(self, transform: cjseq::Transform) -> Features {
        Features::new(self, transform)
    }
}

/// See [`FeatureReader::features`].
#[cfg(feature = "cjseq")]
pub type Features = citygml_core::reader::Features<FeatureReader>;

/// Read `path` once for what its objects need from elsewhere in it
/// ([`Document`]). A document that spells none of `appearanceMember`,
/// `groupMember` and `ImplicitGeometry`, and binds no ADE namespace, is not
/// read past its root.
pub fn read_document(path: &Path) -> Result<Document> {
    read_index(path, true)
}

/// [`read_document`] without the CityModel-level appearance and the ADE
/// index (see [`FeatureReader::open_without_appearance`]).
pub fn read_document_without_appearance(path: &Path) -> Result<Document> {
    read_index(path, false)
}

/// Read every CityModel-level `appearanceMember` (when `full`)
/// into one indexed [`ModelAppearance`], every group's `xlink:href` member
/// into member id -> group ids, every inline relative geometry, and the
/// ADEs. Building subtrees are skipped unless the document has implicit
/// geometries or ADEs, GML elements are skipped unless they are a relative
/// geometry, and an object's own `appearance` (the other property name)
/// is not promoted to model scope.
fn read_index(path: &Path, full: bool) -> Result<Document> {
    let [has_appearance, with_groups, with_templates] = may_contain(
        path,
        [b"appearanceMember", b"groupMember", b"ImplicitGeometry"],
    )?;
    let with_appearance = full && has_appearance;
    // The ADE index only changes where an ADE property's XML attributes are
    // stated, so a read without appearance, for a caller that only counts
    // or measures, goes without it too.
    let with_ade = full && uses_ade(path)?;
    if !with_appearance && !with_groups && !with_templates && !with_ade {
        return Ok(Document::default());
    }
    let file =
        File::open(path).map_err(|e| Error::io(format!("cannot reopen {}", path.display()), e))?;
    let mut reader = NsReader::from_reader(BufReader::new(file));
    reader.config_mut().expand_empty_elements = true;
    let mut buf = Vec::new();
    let mut appearance = Appearance::default();
    let mut groups_of = GroupIndex::default();
    let mut templates = Templates::default();
    let mut ade = ade::AdeScan::default();
    // The LoD of the `lodNImplicitRepresentation` the reader is inside.
    let mut implicit_lod: Option<String> = None;
    // The groups the reader is inside, innermost last: a group may be
    // another group's nested member.
    let mut open_groups: Vec<Option<String>> = Vec::new();
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(&mut buf).map_err(xml_err)?;
        match ev {
            Event::Start(e) => {
                let local = e.local_name();
                if !with_templates
                    && !with_ade
                    && ns_is(&rr, ns::BUILDING)
                    && local.as_ref() == b"Building"
                {
                    skip_element(&mut reader, &mut buf)?;
                } else if ns_is(&rr, NS_GML) {
                    // Geometry, an envelope, a name: nothing this pass reads.
                    skip_element(&mut reader, &mut buf)?;
                } else if let Some(lod) = implicit_lod.clone().filter(|_| {
                    ns_is(&rr, ns::CORE)
                        && local.as_ref() == b"relativeGeometry"
                        && get_attr_local(&e, b"href").is_none()
                }) {
                    // Only inside an `lodNImplicitRepresentation`, the one
                    // place the object pass reads a relative geometry.
                    templates.read(&mut reader, &mut buf, b"relativeGeometry", lod)?;
                } else if with_appearance
                    && ns_is(&rr, ns::CORE)
                    && local.as_ref() == b"appearanceMember"
                {
                    let app = read_appearance(
                        &mut reader,
                        &mut buf,
                        b"appearanceMember",
                        ns::APPEARANCE,
                    )?;
                    appearance.materials.extend(app.materials);
                    appearance.textures.extend(app.textures);
                } else {
                    if with_templates && let Some(lod) = geometry::implicit_lod(local.as_ref()) {
                        implicit_lod = Some(lod);
                    }
                    if ns_is(&rr, ns::CITY_OBJECT_GROUP) {
                        match local.as_ref() {
                            b"CityObjectGroup" => open_groups.push(gml_id(&e)),
                            b"groupMember" => {
                                let member =
                                    get_attr_local(&e, b"href").and_then(|href| member_id(&href));
                                if let (Some(member), Some(Some(group))) =
                                    (member, open_groups.last())
                                {
                                    groups_of.add(group, &member);
                                }
                            }
                            _ => {}
                        }
                    }
                    if with_ade {
                        ade.start(&ns_of(&rr).unwrap_or_default(), &e)?;
                    }
                }
            }
            Event::End(e) => {
                if implicit_lod.is_some()
                    && geometry::implicit_lod(e.local_name().as_ref()).is_some()
                {
                    implicit_lod = None;
                }
                if ns_is(&rr, ns::CITY_OBJECT_GROUP)
                    && e.local_name().as_ref() == b"CityObjectGroup"
                {
                    open_groups.pop();
                }
                if with_ade {
                    ade.end();
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    let appearance = ModelAppearance::new(appearance);
    if with_appearance {
        templates.scope_model_appearance(&appearance);
    }
    let mut ade = ade.finish();
    if with_ade {
        ade.read_schemas(path, |uri| ns::module(uri.as_bytes()))?;
    }
    Ok(Document::new(appearance, groups_of, templates, ade))
}
