//! Streaming reader: one [`GmlObject`] per top-level city object of a class
//! the walker reads, with its parts and sub-features.
//!
//! It holds one object subtree at a time, so memory is bounded by the largest
//! single object; the `CityModel` and its `cityObjectMember`s are walked past
//! without buffering.

mod ade;
mod appearance;
mod attributes;
pub mod document;
mod templates;
mod walker;

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::sync::Arc;

use citygml_core::gml::XlinkBudget;
use citygml_core::model::{
    AdeIndex, Appearance, GroupIndex, ModelAppearance, Namespace, Template, XmlContent, XmlNode,
};
use citygml_core::xml::{NS_GML, get_attr_local, gml_id, ns_is, ns_of, skip_element, xml_err};
use citygml_core::{Error, GmlObject, Result};
use quick_xml::events::Event;
use quick_xml::reader::NsReader;

use crate::ns;
use templates::Templates;

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
    /// as soon as that object has been seen, so [`Self::skipped_members`]
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
    /// Reads the document once up front ([`Document::read`]), then streams
    /// it again for the objects: two passes, unless the document has nothing
    /// the up-front pass reads.
    pub fn open(path: &Path) -> Result<Self> {
        Self::with_document(path, Arc::new(Document::read(path)?))
    }

    /// Open `path` for streaming without the CityModel-level appearance and
    /// the ADE index.
    ///
    /// Objects stream identically apart from appearance defined at the
    /// CityModel level, which is left out (an object's own `app:appearance`
    /// is still read), and an ADE property's XML attributes, which a
    /// conversion then states where they occur rather than once for the
    /// document. Leaving both out spares holding every CityModel-level
    /// definition in memory, which a caller that only counts or measures
    /// geometry never consults, and the up-front pass altogether for a
    /// document without groups or implicit geometries.
    pub fn open_without_appearance(path: &Path) -> Result<Self> {
        Self::with_document(path, Arc::new(Document::read_without_appearance(path)?))
    }

    /// Open `path` for streaming with what an earlier [`Document::read`] of
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

    /// Every `cityObjectMember` object of a class this reader does not read,
    /// by document-spelled name (`dem:RasterRelief` -> 3). It records what
    /// has been passed over so far, so it is complete only once the stream
    /// is drained.
    pub fn skipped_members(&self) -> &BTreeMap<String, usize> {
        &self.skipped_members
    }

    /// Every `cityObjectMember` object of an ADE's class read so far, by
    /// document-spelled name (`urf:UseDistrict` -> 3): what a CityGML tool
    /// that does not know the ADE leaves out.
    pub fn ade_members(&self) -> &BTreeMap<String, usize> {
        &self.ade_members
    }

    /// Every ADE property read so far that holds GML geometry, which the
    /// CityJSON conversion leaves out, by property name (`uro:lod0Point` ->
    /// 3), the prefix being the one the document binds.
    pub fn ade_geometries(&self) -> &BTreeMap<String, usize> {
        &self.ade_geometries
    }

    /// How many objects this reader has yielded so far.
    pub fn emitted_members(&self) -> usize {
        self.index
    }

    /// What the up-front pass read.
    pub fn document(&self) -> &Arc<Document> {
        &self.document
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
                        templates: &self.document.templates.index,
                    };
                    let builder =
                        walker::read_object(&mut self.reader, &mut self.buf, class, id, &end, cx)?;
                    self.index += 1;
                    self.inside_member = false;
                    let mut object = builder.finish(&self.xlink_budget)?;
                    object.scope_model_appearance(&self.document.appearance);
                    if !self.document.groups_of.is_empty() {
                        object.assign_groups(&self.document.groups_of);
                    }
                    tally_ade_geometries(&object, &self.document.ade, &mut self.ade_geometries);
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

/// Count `object`'s ADE properties, and its children's, that hold GML.
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
                ade: self.reader.document().ade().clone(),
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

/// Whether the file at `path` may contain any of `needles`.
fn may_contain_any(path: &Path, needles: &[&[u8]]) -> Result<bool> {
    let io = |e| Error::io(format!("cannot read {}", path.display()), e);
    let mut file = File::open(path).map_err(io)?;
    let finders: Vec<_> = needles.iter().map(memchr::memmem::Finder::new).collect();
    let keep = needles.iter().map(|n| n.len()).max().unwrap_or(1).max(1) - 1;
    let mut buf = vec![0u8; (8 << 20).max(keep + 1)];
    let mut filled = 0;
    loop {
        let read = file.read(&mut buf[filled..]).map_err(io)?;
        let end = filled + read;
        if finders.iter().any(|f| f.find(&buf[..end]).is_some()) {
            return Ok(true);
        }
        if read == 0 {
            return Ok(false);
        }
        let tail = end.saturating_sub(keep);
        buf.copy_within(tail..end, 0);
        filled = end - tail;
    }
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

/// What one pass over a whole document finds ahead of its objects, which
/// an object needs but may come after it in the document: the
/// CityModel-level appearance, which groups reference which members, the
/// relative geometries its `ImplicitGeometry`s place, and the ADEs it uses.
#[derive(Debug, Default)]
pub struct Document {
    appearance: ModelAppearance,
    groups_of: GroupIndex,
    templates: Templates,
    ade: Arc<AdeIndex>,
}

impl Document {
    /// Read `path` once. A document that spells none of `appearanceMember`,
    /// `groupMember` and `ImplicitGeometry`, and binds no ADE namespace on
    /// its root element, is not read past its root.
    pub fn read(path: &Path) -> Result<Self> {
        read_index(path, true)
    }

    /// [`Self::read`] without the CityModel-level appearance and the ADE
    /// index (see [`FeatureReader::open_without_appearance`]).
    pub fn read_without_appearance(path: &Path) -> Result<Self> {
        read_index(path, false)
    }

    /// The relative geometries the document's `ImplicitGeometry`s place, in
    /// document order, each with the CityModel-level appearance that targets
    /// it (none when read without it). An [`Instance`]'s `template` is a
    /// position in this list.
    ///
    /// [`Instance`]: citygml_core::model::Instance
    pub fn templates(&self) -> &[Template] {
        &self.templates.templates
    }

    /// The ADEs the document uses.
    pub fn ade(&self) -> &Arc<AdeIndex> {
        &self.ade
    }
}

/// Whether the document at `path` binds an ADE's namespace on any element
/// and spells an element in it, judged from its bytes: every `xmlns`
/// declaration whose URI is an ADE's ([`ns::is_ade`]), then an element of a
/// prefix one binds (`<uro:`), or any element at all for an ADE bound as the
/// default namespace.
fn uses_ade(path: &Path) -> Result<bool> {
    // Enough to see a whole namespace declaration after `xmlns`.
    const LOOKAHEAD: usize = 4096;
    let io = |e| Error::io(format!("cannot read {}", path.display()), e);
    let mut file = File::open(path).map_err(io)?;
    let finder = memchr::memmem::Finder::new(b"xmlns");
    let mut prefixes: Vec<Vec<u8>> = Vec::new();
    let mut buf = vec![0u8; 8 << 20];
    let mut filled = 0;
    let mut first = true;
    loop {
        let read = file.read(&mut buf[filled..]).map_err(io)?;
        if first {
            // Not ASCII-compatible (UTF-16): the bytes cannot tell.
            let head = &buf[..read.min(4)];
            if head.starts_with(&[0xFE, 0xFF])
                || head.starts_with(&[0xFF, 0xFE])
                || head.contains(&0)
            {
                return Ok(true);
            }
            first = false;
        }
        let end = filled + read;
        let eof = read == 0;
        // A declaration starting in the last `LOOKAHEAD` bytes waits for the
        // next chunk, which starts with them.
        let limit = if eof {
            end
        } else {
            end.saturating_sub(LOOKAHEAD)
        };
        for at in finder.find_iter(&buf[..end]).take_while(|&at| at < limit) {
            match namespace_declaration(&buf[at + 5..end]) {
                Some((prefix, uri)) if ns::is_ade(uri) => {
                    if prefix.is_empty() {
                        return Ok(true);
                    }
                    if !prefixes.iter().any(|p| p == prefix) {
                        prefixes.push(prefix.to_vec());
                    }
                }
                _ => {}
            }
        }
        if eof {
            break;
        }
        buf.copy_within(limit..end, 0);
        filled = end - limit;
    }
    if prefixes.is_empty() {
        return Ok(false);
    }
    let needles: Vec<Vec<u8>> = prefixes
        .iter()
        .map(|p| [b"<".as_slice(), p, b":"].concat())
        .collect();
    let needles: Vec<&[u8]> = needles.iter().map(Vec::as_slice).collect();
    may_contain_any(path, &needles)
}

/// The prefix (empty for a default namespace) and URI of the namespace
/// declaration `rest` begins with, just after its `xmlns`.
fn namespace_declaration(rest: &[u8]) -> Option<(&[u8], &[u8])> {
    let (prefix, rest) = match rest.first()? {
        b':' => {
            let name = rest[1..]
                .iter()
                .position(|&b| b == b'=' || b.is_ascii_whitespace())?;
            (&rest[1..1 + name], &rest[1 + name..])
        }
        _ => (&rest[..0], rest),
    };
    let rest = rest
        .trim_ascii_start()
        .strip_prefix(b"=")?
        .trim_ascii_start();
    let quote = *rest.first()?;
    if quote != b'"' && quote != b'\'' {
        return None;
    }
    let len = rest[1..].iter().position(|&b| b == quote)?;
    Some((prefix, &rest[1..1 + len]))
}

/// Read every CityModel-level `app:appearanceMember` (when `full`)
/// into one indexed [`ModelAppearance`], every group's `xlink:href` member
/// into member id -> group ids, every inline relative geometry, and the
/// ADEs. Building subtrees are skipped unless the document has implicit
/// geometries or ADEs, GML elements are skipped unless they are a relative
/// geometry, and an object's own `app:appearance` (the other property name)
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
                } else {
                    if with_templates && let Some(lod) = walker::implicit_lod(local.as_ref()) {
                        implicit_lod = Some(lod);
                    }
                    if ns_is(&rr, ns::CITY_OBJECT_GROUP) {
                        match local.as_ref() {
                            b"CityObjectGroup" => open_groups.push(gml_id(&e)),
                            b"groupMember" => {
                                let member = get_attr_local(&e, b"href")
                                    .and_then(|href| walker::member_id(&href));
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
                if implicit_lod.is_some() && walker::implicit_lod(e.local_name().as_ref()).is_some()
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
    Ok(Document {
        appearance,
        groups_of,
        templates,
        ade: Arc::new(ade.finish()),
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
