//! Streaming reader: one [`GmlObject`] per top-level city object of a class
//! the walker reads, with its parts and sub-features.
//!
//! It holds one object subtree at a time, so memory is bounded by the largest
//! single object; the `CityModel` and its `cityObjectMember`s are walked past
//! without buffering.

mod appearance;
mod attributes;
pub mod document;
mod templates;
mod walker;

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

use citygml_core::gml::XlinkBudget;
use citygml_core::model::{Appearance, GroupIndex, ModelAppearance, Template};
use citygml_core::xml::{get_attr_local, gml_id, ns_is, ns_of, skip_element, xml_err};
use citygml_core::{Error, GmlObject, Result};
use quick_xml::events::Event;
use quick_xml::reader::NsReader;

use crate::ns;
use templates::{TemplateIndex, Templates};

pub struct FeatureReader {
    reader: NsReader<Box<dyn BufRead + Send>>,
    buf: Vec<u8>,
    index: usize,
    done: bool,
    /// The CityModel-level appearance (`app:appearanceMember`), read up front
    /// in a separate pass so it applies to every object by `gml:id` wherever
    /// in the document it is declared. Empty when the reader was opened
    /// without it.
    model_appearance: ModelAppearance,
    /// Member `gml:id` -> the groups that reference it by `xlink:href`, read
    /// up front with the appearance, so a member read before its group
    /// still names it. Empty for a reader opened on a stream.
    groups_of: GroupIndex,
    /// Where the document's relative geometries are, read up front, so an
    /// `ImplicitGeometry` that references one defined later still finds it.
    /// Empty for a reader opened on a stream, whose instances are then
    /// left out.
    templates: TemplateIndex,
    /// The document's allowance for expanding xlinks to surface aggregates,
    /// shared by every object it holds.
    xlink_budget: XlinkBudget,
    /// Whether the next `Start` is a `cityObjectMember`'s own object. Cleared
    /// as soon as that object has been seen, so [`Self::skipped_members`]
    /// counts members, never every unmapped element inside one.
    inside_member: bool,
    /// Members of a class this reader does not read, by the element name the
    /// document spells (`tran:Track`, `dem:ReliefFeature`, …).
    skipped_members: BTreeMap<String, usize>,
}

impl FeatureReader {
    /// Open `path` for streaming.
    ///
    /// Reads the document once up front to index its CityModel-level
    /// appearance, its groups' referenced members and its relative
    /// geometries, then streams it again for the objects: two passes, unless
    /// the document spells none of `appearanceMember`, `groupMember` and
    /// `ImplicitGeometry`.
    pub fn open(path: &Path) -> Result<Self> {
        Self::open_inner(path, true)
    }

    /// Open `path` for streaming without the CityModel-level appearance.
    ///
    /// Objects stream identically apart from appearance defined at the
    /// CityModel level, which is left out; an object's own `app:appearance`
    /// is still read. Leaving it out spares holding every CityModel-level
    /// definition in memory, which a caller that only counts or measures
    /// geometry never consults, and the up-front pass altogether for a
    /// document without groups or implicit geometries.
    pub fn open_without_appearance(path: &Path) -> Result<Self> {
        Self::open_inner(path, false)
    }

    /// Stream objects from any `BufRead` positioned at the document's first
    /// byte, in one pass, without the CityModel-level appearance, without
    /// an object's referencing groups in [`GmlObject::groups`] and without
    /// its `ImplicitGeometry`s (each would need a second read). Nothing is
    /// read beyond what the objects pulled so far need.
    pub fn from_reader(input: Box<dyn BufRead + Send>) -> Self {
        Self::from_parts(input, ModelAppearance::default())
    }

    /// Every `cityObjectMember` object of a class this reader does not read,
    /// by document-spelled name (`tran:Track` -> 3). It records what has been
    /// passed over so far, so it is complete only once the stream is drained.
    pub fn skipped_members(&self) -> &BTreeMap<String, usize> {
        &self.skipped_members
    }

    /// How many objects this reader has yielded so far.
    pub fn emitted_members(&self) -> usize {
        self.index
    }

    fn open_inner(path: &Path, with_appearance: bool) -> Result<Self> {
        let file = File::open(path)
            .map_err(|e| Error::io(format!("cannot reopen {}", path.display()), e))?;
        let index = read_index(path, with_appearance)?;
        let mut reader = Self::from_parts(Box::new(BufReader::new(file)), index.appearance);
        reader.groups_of = index.groups_of;
        reader.templates = index.templates.index;
        Ok(reader)
    }

    fn from_parts(input: Box<dyn BufRead + Send>, model_appearance: ModelAppearance) -> Self {
        let mut reader = NsReader::from_reader(input);
        reader.config_mut().expand_empty_elements = true;
        Self {
            reader,
            buf: Vec::new(),
            index: 0,
            done: false,
            model_appearance,
            groups_of: GroupIndex::default(),
            templates: TemplateIndex::default(),
            xlink_budget: XlinkBudget::default(),
            inside_member: false,
            skipped_members: BTreeMap::new(),
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
                    let Some(class) = walker::top_level_class(&uri, local.as_ref()) else {
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
                        templates: &self.templates,
                    };
                    let builder =
                        walker::read_object(&mut self.reader, &mut self.buf, class, id, &end, cx)?;
                    self.index += 1;
                    self.inside_member = false;
                    let mut object = builder.finish(&self.xlink_budget)?;
                    object.scope_model_appearance(&self.model_appearance);
                    if !self.groups_of.is_empty() {
                        object.assign_groups(&self.groups_of);
                    }
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

#[cfg(feature = "cjseq")]
pub use features::Features;

#[cfg(feature = "cjseq")]
mod features {
    use citygml_core::Result;
    use citygml_core::convert::FromGml;
    use citygml_core::convert::cjseq::FeatureContext;
    use cjseq::{CityJSONFeature, Transform};

    use super::FeatureReader;

    impl FeatureReader {
        /// The objects as `cjseq::CityJSONFeature`s, each with its own vertex
        /// pool quantised against `transform`.
        pub fn features(self, transform: Transform) -> Features {
            Features {
                reader: self,
                transform,
            }
        }
    }

    /// See [`FeatureReader::features`].
    pub struct Features {
        reader: FeatureReader,
        transform: Transform,
    }

    impl Features {
        /// The reader underneath, for [`FeatureReader::skipped_members`] and
        /// [`FeatureReader::emitted_members`].
        pub fn reader(&self) -> &FeatureReader {
            &self.reader
        }
    }

    impl Iterator for Features {
        type Item = Result<CityJSONFeature>;

        fn next(&mut self) -> Option<Self::Item> {
            let object = match self.reader.next()? {
                Ok(object) => object,
                Err(e) => return Some(Err(e)),
            };
            let context = FeatureContext {
                transform: self.transform.clone(),
                index: self.reader.emitted_members(),
            };
            Some(CityJSONFeature::from_gml(&object, &context))
        }
    }
}

/// For each needle, whether the bytes of `path` may contain it: `false` only
/// when a document in an ASCII-compatible encoding certainly does not. A
/// document that may be UTF-16 (a byte-order mark or a zero byte at its
/// start) is assumed to contain every needle. One read of the file.
fn may_contain<const N: usize>(path: &Path, needles: [&[u8]; N]) -> Result<[bool; N]> {
    may_contain_in_chunks(path, needles, 8 << 20)
}

fn may_contain_in_chunks<const N: usize>(
    path: &Path,
    needles: [&[u8]; N],
    chunk: usize,
) -> Result<[bool; N]> {
    let io = |e| Error::io(format!("cannot read {}", path.display()), e);
    let mut file = File::open(path).map_err(io)?;
    let finders = needles.map(memchr::memmem::Finder::new);
    let keep = needles.iter().map(|n| n.len()).max().unwrap_or(1).max(1) - 1;
    let mut found = [false; N];
    let mut buf = vec![0u8; chunk.max(keep + 1)];
    let mut filled = 0;
    let mut first = true;
    loop {
        let read = file.read(&mut buf[filled..]).map_err(io)?;
        if first {
            let head = &buf[..read.min(4)];
            if head.starts_with(&[0xFE, 0xFF])
                || head.starts_with(&[0xFF, 0xFE])
                || head.contains(&0)
            {
                return Ok([true; N]);
            }
            first = false;
        }
        let end = filled + read;
        for (hit, finder) in found.iter_mut().zip(&finders) {
            *hit = *hit || finder.find(&buf[..end]).is_some();
        }
        if read == 0 || found.iter().all(|&hit| hit) {
            return Ok(found);
        }
        // Carry the tail over, so a needle across the boundary is found.
        let tail = end.saturating_sub(keep);
        buf.copy_within(tail..end, 0);
        filled = end - tail;
    }
}

/// The relative geometries of the document at `path` that its
/// `ImplicitGeometry`s instantiate, in document order, each with the
/// CityModel-level appearance that targets it. An [`Instance`]'s
/// `template` is a position in this list.
///
/// [`Instance`]: citygml_core::model::Instance
pub fn read_templates(path: &Path) -> Result<Vec<Template>> {
    let mut index = read_index(path, true)?;
    index.templates.scope_model_appearance(&index.appearance);
    Ok(index.templates.templates)
}

/// What the up-front pass over a document finds.
#[derive(Default)]
struct DocumentIndex {
    appearance: ModelAppearance,
    groups_of: GroupIndex,
    templates: Templates,
}

/// Read every CityModel-level `app:appearanceMember` (when `with_appearance`)
/// into one indexed [`ModelAppearance`], every group's `xlink:href` member
/// into member id -> group ids, and every inline relative geometry. Building
/// subtrees are skipped unless the document has implicit geometries, and an
/// object's own `app:appearance` (the other property name) is not promoted
/// to model scope.
fn read_index(path: &Path, with_appearance: bool) -> Result<DocumentIndex> {
    let [has_appearance, with_groups, with_templates] = may_contain(
        path,
        [b"appearanceMember", b"groupMember", b"ImplicitGeometry"],
    )?;
    let with_appearance = with_appearance && has_appearance;
    if !with_appearance && !with_groups && !with_templates {
        return Ok(DocumentIndex::default());
    }
    let file =
        File::open(path).map_err(|e| Error::io(format!("cannot reopen {}", path.display()), e))?;
    let mut reader = NsReader::from_reader(BufReader::new(file));
    reader.config_mut().expand_empty_elements = true;
    let mut buf = Vec::new();
    let mut appearance = Appearance::default();
    let mut groups_of = GroupIndex::default();
    let mut templates = Templates::default();
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
                if !with_templates && ns_is(&rr, ns::BUILDING) && local.as_ref() == b"Building" {
                    skip_element(&mut reader, &mut buf)?;
                } else if let Some(lod) = with_templates
                    .then(|| walker::implicit_lod(local.as_ref()))
                    .flatten()
                {
                    implicit_lod = Some(lod);
                } else if let Some(lod) = implicit_lod.clone().filter(|_| {
                    ns_is(&rr, ns::CORE)
                        && local.as_ref() == b"relativeGMLGeometry"
                        && get_attr_local(&e, b"href").is_none()
                }) {
                    // Only inside an `lodNImplicitRepresentation`, the one
                    // place the object pass reads a relative geometry.
                    templates.read(&mut reader, &mut buf, lod)?;
                } else if with_appearance
                    && ns_is(&rr, ns::APPEARANCE)
                    && local.as_ref() == b"appearanceMember"
                {
                    let app =
                        appearance::read_appearance(&mut reader, &mut buf, b"appearanceMember")?;
                    appearance.materials.extend(app.materials);
                    appearance.textures.extend(app.textures);
                } else if ns_is(&rr, ns::CITY_OBJECT_GROUP) {
                    match local.as_ref() {
                        b"CityObjectGroup" => open_groups.push(gml_id(&e)),
                        b"groupMember" => {
                            let member = get_attr_local(&e, b"href")
                                .and_then(|href| walker::member_id(&href));
                            if let (Some(member), Some(Some(group))) = (member, open_groups.last())
                            {
                                groups_of.add(group, &member);
                            }
                        }
                        _ => {}
                    }
                }
            }
            Event::End(e) => {
                if implicit_lod.is_some() && walker::implicit_lod(e.local_name().as_ref()).is_some()
                {
                    implicit_lod = None;
                }
                if ns_is(&rr, ns::CITY_OBJECT_GROUP)
                    && e.local_name().as_ref() == b"CityObjectGroup"
                {
                    open_groups.pop();
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(DocumentIndex {
        appearance: ModelAppearance::new(appearance),
        groups_of,
        templates,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(name: &str) -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/data")
            .join(name)
    }

    /// The byte search that lets the CityModel-appearance pass be skipped
    /// finds the element across the boundary between two reads, exercised
    /// with reads shorter than the needle, and reports a document without it.
    #[test]
    fn the_appearance_member_search_finds_it_across_read_boundaries() {
        let with = data("building_citymodel_appearance.gml");
        let without = data("b1_lod2_cs_w_sem.gml");
        for chunk in [5, 7, 4096, 8 << 20] {
            assert_eq!(
                may_contain_in_chunks(&with, [b"appearanceMember"], chunk).unwrap(),
                [true],
                "{chunk}"
            );
            assert_eq!(
                may_contain_in_chunks(&without, [b"appearanceMember"], chunk).unwrap(),
                [false],
                "{chunk}"
            );
        }
    }
}
