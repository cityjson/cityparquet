//! Streaming feature reader: one `cjseq::CityJSONFeature` per top-level
//! CityObject — a `bldg:Building` (with its parts) or a mapped 1st-level
//! non-building object (WaterBody, LandUse, CityFurniture, … — CG-7).
//!
//! Buffers exactly one object subtree at a time (memory is O(largest single
//! object)); the CityModel wrapper and `cityObjectMember`s are walked past
//! without buffering. Non-building objects read their `lodN` solid/surface
//! geometry + generic attributes; semantic surfaces, parts, and appearance on
//! non-building objects are out of scope for this milestone.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use cityparquet_schema::{CityParquetError, Result};
use cjseq::{CityJSONFeature, Transform};
use quick_xml::events::Event;
use quick_xml::reader::NsReader;

use super::appearance::{ModelAppearance, ReadAppearance, read_appearance};
use super::building::{read_building, read_generic_object};
use super::xml::{NS_APP, NS_BLDG, gml_id, ns_is, skip_element, xml_err};

/// Map a CityGML 1st-level NON-building element local name to its CityJSON
/// CityObject type (CG-7). Matched by local name (unique across CityGML module
/// namespaces); `Building`/`BuildingPart` are handled separately. Returns
/// `None` for containers and unmapped/2nd-level elements.
fn citygml_object_type(local: &[u8]) -> Option<&'static str> {
    Some(match local {
        b"WaterBody" => "WaterBody",
        b"LandUse" => "LandUse",
        b"CityFurniture" => "CityFurniture",
        b"SolitaryVegetationObject" => "SolitaryVegetationObject",
        b"PlantCover" => "PlantCover",
        b"Bridge" => "Bridge",
        b"Tunnel" => "Tunnel",
        // ReliefFeature is deliberately NOT mapped: a CityGML ReliefFeature may
        // be raster/breakline/mass-point/TIN, and only a TIN maps cleanly to
        // CityJSON TINRelief; its `reliefComponent` geometry is nested
        // differently. Deferred to avoid misclassifying non-TIN reliefs.
        b"GenericCityObject" => "GenericCityObject",
        b"CityObjectGroup" => "CityObjectGroup",
        b"Road" => "Road",
        b"Railway" => "Railway",
        b"Square" => "TransportSquare",
        _ => return None,
    })
}

pub struct FeatureReader {
    reader: NsReader<Box<dyn BufRead + Send>>,
    buf: Vec<u8>,
    scale: [f64; 3],
    translate: [f64; 3],
    index: usize,
    done: bool,
    /// CityModel-level appearance (`app:appearanceMember` on `<CityModel>`),
    /// collected up front in a separate pass so it can be applied to every
    /// building's faces/rings by `gml:id` regardless of whether it appears
    /// before or after the buildings, and indexed for O(building-ids) lookup
    /// (CG-3). Empty when the caller opened via
    /// [`FeatureReader::open_without_appearance`].
    model_appearance: ModelAppearance,
    /// The document's allowance for expanding xlinks to surface aggregates,
    /// shared by every object it holds.
    xlink_budget: std::sync::Arc<super::building::XlinkBudget>,
    /// Whether the reader is currently positioned directly inside a
    /// `cityObjectMember`, i.e. the next `Start` is that member's own object.
    /// Cleared as soon as that object has been seen (mapped or not), so
    /// [`Self::skipped_members`] counts MEMBERS, never every unmapped element
    /// in a member's subtree.
    inside_member: bool,
    /// `cityObjectMember` objects whose element name this reader does not map,
    /// keyed by the name exactly as the document spells it (`tran:Track`,
    /// `dem:ReliefFeature`, …) — a `BTreeMap` so a diagnostic built from it is
    /// deterministic.
    ///
    /// Skipping is deliberate and stays that way: the conversion pipeline must
    /// keep ingesting the mapped part of a real national export. But a caller
    /// that publishes a COUNT cannot tell "nothing here is mapped" from "this
    /// document is empty" without being told, so the reader records it and lets
    /// the caller decide (the readbench `citygml` runner refuses outright).
    skipped_members: std::collections::BTreeMap<String, usize>,
}

/// Whether the bytes of `path` may contain `needle` — `false` only when a
/// document in an ASCII-compatible encoding certainly does not. A document
/// that may be UTF-16 (a byte-order mark or a zero byte at its start) is
/// assumed to contain it, since its element names are not stored as these
/// bytes.
fn may_contain(path: &Path, needle: &[u8]) -> Result<bool> {
    may_contain_in_chunks(path, needle, 8 << 20)
}

fn may_contain_in_chunks(path: &Path, needle: &[u8], chunk: usize) -> Result<bool> {
    use std::io::Read;
    let io = |e| CityParquetError::io_source(format!("cannot read {}", path.display()), e);
    let mut file = File::open(path).map_err(io)?;
    let finder = memchr::memmem::Finder::new(needle);
    let keep = needle.len() - 1;
    let mut buf = vec![0u8; chunk.max(needle.len())];
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
                return Ok(true);
            }
            first = false;
        }
        let end = filled + read;
        if finder.find(&buf[..end]).is_some() {
            return Ok(true);
        }
        if read == 0 {
            return Ok(false);
        }
        // Carry the tail over, so a needle across the boundary is found.
        let tail = end.saturating_sub(keep);
        buf.copy_within(tail..end, 0);
        filled = end - tail;
    }
}

/// Pre-pass: read every CityModel-level `app:appearanceMember` (the conformant
/// CityGML 2.0 global-appearance property — a `_FeatureCollection` member of
/// `CityModel`, distinct from a feature's own `app:appearance`) into one
/// indexed [`ModelAppearance`]. Building subtrees are skipped, and a
/// feature-level `app:appearance` (which uses the other property name) is NOT
/// promoted to model scope.
fn read_model_appearance(path: &Path) -> Result<ModelAppearance> {
    // A document that never spells `appearanceMember` has no CityModel-level
    // appearance; a byte search says so far faster than parsing it.
    if !may_contain(path, b"appearanceMember")? {
        return Ok(ModelAppearance::build(ReadAppearance::default()));
    }
    let file = File::open(path)
        .map_err(|e| CityParquetError::io_source(format!("cannot reopen {}", path.display()), e))?;
    let mut reader = NsReader::from_reader(BufReader::new(file));
    reader.config_mut().expand_empty_elements = true;
    let mut buf = Vec::new();
    let mut out = ReadAppearance::default();
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(&mut buf).map_err(xml_err)?;
        match ev {
            Event::Start(e) => {
                if ns_is(&rr, NS_BLDG) && e.local_name().as_ref() == b"Building" {
                    skip_element(&mut reader, &mut buf)?;
                } else if ns_is(&rr, NS_APP) && e.local_name().as_ref() == b"appearanceMember" {
                    let app = read_appearance(&mut reader, &mut buf, b"appearanceMember")?;
                    out.materials.extend(app.materials);
                    out.textures.extend(app.textures);
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(ModelAppearance::build(out))
}

impl FeatureReader {
    /// Open `path` for streaming, quantising vertices against `transform`
    /// (which the header exposes so scan/encode dequantise consistently).
    ///
    /// Reads the whole document once up front to index its CityModel-level
    /// appearance (see [`read_model_appearance`]) before streaming it again for
    /// features — two full passes. [`Self::open_without_appearance`] is the
    /// one-pass form for callers that never look at appearance.
    pub fn open(path: &Path, transform: &Transform) -> Result<Self> {
        Self::open_inner(path, transform, true)
    }

    /// Open `path` for streaming WITHOUT the CityModel-level appearance
    /// pre-pass.
    ///
    /// The pre-pass exists so a `app:appearanceMember` declared anywhere in the
    /// document can be applied to a building by `gml:id`; it costs a second
    /// full read of the file and holds every material/texture definition in
    /// memory. A caller that only counts, filters or measures geometry never
    /// consults any of it, and on a real 117 MB PLATEAU tile that pass was
    /// ~35-45% of the elapsed time and ~20x the peak heap — pure harness
    /// overhead in a number meant to describe CityGML.
    ///
    /// Features stream identically apart from appearance: same ids, same
    /// CityObjects, same geometry boundaries, same vertex pool, with
    /// `feature.appearance` left `None` and no `material`/`texture` map
    /// stamped on a geometry. Pinned by
    /// `tests/citygml_reader_profile.rs::open_without_appearance_changes_only_the_appearance`.
    pub fn open_without_appearance(path: &Path, transform: &Transform) -> Result<Self> {
        Self::open_inner(path, transform, false)
    }

    /// Stream features from any `BufRead` — an HTTP response body, say —
    /// quantising vertices against `transform`.
    ///
    /// Always one pass, WITHOUT the CityModel-level appearance pre-pass (it
    /// would need a second read of a stream that can be read once): features
    /// stream exactly as from [`Self::open_without_appearance`]. Nothing is
    /// read beyond what the features pulled so far need, so a caller that
    /// stops early and drops the reader never reads the rest of the input.
    /// `input` must start at the document's first byte; a caller that ran
    /// [`super::sniff_citygml_from`]/[`super::parse_header_from`] on the same
    /// stream replays the bytes they consumed ahead of the rest.
    pub fn from_reader(input: Box<dyn BufRead + Send>, transform: &Transform) -> Result<Self> {
        Self::from_parts(input, transform, ModelAppearance::default())
    }

    /// Every `cityObjectMember` object whose element name this reader does not
    /// map, by document-spelled name (`tran:Track` -> 3). Empty for a document
    /// every member of which was read.
    ///
    /// Only meaningful once the stream has been driven: it records what has
    /// been passed over SO FAR, so a caller that stops early sees a partial
    /// tally. The readbench runner therefore drains to EOF before consulting
    /// it.
    pub fn skipped_members(&self) -> &std::collections::BTreeMap<String, usize> {
        &self.skipped_members
    }

    /// How many features this reader has emitted so far — the companion to
    /// [`Self::skipped_members`], so a caller can report "n of m members".
    pub fn emitted_members(&self) -> usize {
        self.index
    }

    fn open_inner(path: &Path, transform: &Transform, with_appearance: bool) -> Result<Self> {
        let file = File::open(path).map_err(|e| {
            CityParquetError::io_source(format!("cannot reopen {}", path.display()), e)
        })?;
        let model_appearance = if with_appearance {
            read_model_appearance(path)?
        } else {
            ModelAppearance::default()
        };
        Self::from_parts(Box::new(BufReader::new(file)), transform, model_appearance)
    }

    fn from_parts(
        input: Box<dyn BufRead + Send>,
        transform: &Transform,
        model_appearance: ModelAppearance,
    ) -> Result<Self> {
        let scale = triple(&transform.scale, "scale")?;
        let translate = triple(&transform.translate, "translate")?;
        let mut reader = NsReader::from_reader(input);
        // Self-closing elements (`<gml:surfaceMember xlink:href=.../>`) must
        // arrive as Start+End so the geometry parsers see the xlink; otherwise
        // quick-xml emits Event::Empty, which the Start-matching loops drop.
        reader.config_mut().expand_empty_elements = true;
        Ok(Self {
            reader,
            buf: Vec::new(),
            scale,
            translate,
            index: 0,
            done: false,
            model_appearance,
            xlink_budget: std::sync::Arc::default(),
            inside_member: false,
            skipped_members: std::collections::BTreeMap::new(),
        })
    }

    fn next_feature(&mut self) -> Result<Option<CityJSONFeature>> {
        loop {
            self.buf.clear();
            let (rr, ev) = self
                .reader
                .read_resolved_event_into(&mut self.buf)
                .map_err(xml_err)?;
            match ev {
                Event::Start(e) => {
                    let local = e.local_name();
                    if local.as_ref() == b"cityObjectMember" {
                        // The next object element is this member's own; see
                        // `inside_member`.
                        self.inside_member = true;
                        continue;
                    }
                    let is_building = ns_is(&rr, NS_BLDG) && local.as_ref() == b"Building";
                    // A 1st-level non-building object (WaterBody, LandUse, …).
                    let generic_type = if is_building {
                        None
                    } else {
                        citygml_object_type(local.as_ref())
                    };
                    if self.inside_member && !is_building && generic_type.is_none() {
                        // A member of a type this reader does not map. Record
                        // it (by the name the document spells, so a diagnostic
                        // can quote it) and clear the flag so nothing deeper in
                        // its subtree is counted as a member too.
                        //
                        // Deliberately NOT skipped past: descending is what
                        // this reader has always done here, and a mapped object
                        // nested inside an unmapped member is still emitted.
                        // Only the tally is new.
                        let name = String::from_utf8_lossy(e.name().as_ref()).into_owned();
                        *self.skipped_members.entry(name).or_insert(0) += 1;
                        self.inside_member = false;
                        continue;
                    }
                    if is_building {
                        let id = gml_id(&e);
                        // Borrows of `e`/`rr` end here (NLL) before we re-borrow.
                        let raw = read_building(&mut self.reader, &mut self.buf, id)?;
                        self.index += 1;
                        let feature = raw.into_feature(
                            &self.scale,
                            &self.translate,
                            self.index,
                            &self.model_appearance,
                            &self.xlink_budget,
                        )?;
                        // This member's object has been consumed whole; the
                        // member's own `End` is next.
                        self.inside_member = false;
                        return Ok(Some(feature));
                    } else if let Some(ty) = generic_type {
                        let id = gml_id(&e);
                        let end = local.as_ref().to_vec();
                        let raw =
                            read_generic_object(&mut self.reader, &mut self.buf, ty, id, &end, 0)?;
                        self.index += 1;
                        let feature = raw.into_feature(
                            &self.scale,
                            &self.translate,
                            self.index,
                            &self.model_appearance,
                            &self.xlink_budget,
                        )?;
                        // This member's object has been consumed whole; the
                        // member's own `End` is next.
                        self.inside_member = false;
                        return Ok(Some(feature));
                    }
                    // Otherwise descend. Containers (CityModel, cityObjectMember)
                    // hold the objects we want; a `BuildingPart` is handled
                    // within its parent Building. Nothing to do — next read
                    // descends.
                }
                Event::End(e) => {
                    // Belt and braces for the flag above: an empty
                    // `<cityObjectMember/>` (expanded to Start+End) never
                    // reaches an object element, and a CityModel child that
                    // FOLLOWS the last member (e.g. `app:appearanceMember`)
                    // must not be mistaken for a member of an unmapped type.
                    if e.local_name().as_ref() == b"cityObjectMember" {
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
    type Item = Result<CityJSONFeature>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        self.next_feature().transpose()
    }
}

fn triple(v: &[f64], what: &str) -> Result<[f64; 3]> {
    if v.len() == 3 {
        Ok([v[0], v[1], v[2]])
    } else {
        Err(CityParquetError::Schema(format!(
            "CityGML header transform {what} must have 3 components, got {}",
            v.len()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(name: &str) -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(name)
    }

    /// The byte search that lets the CityModel-appearance pre-pass be skipped
    /// finds the element wherever it lies — including across the boundary
    /// between two reads, exercised with reads shorter than the needle — and
    /// reports a document without it.
    #[test]
    fn the_appearance_member_search_finds_it_across_read_boundaries() {
        let with = data("tests/data/building_citymodel_appearance.gml");
        let without = data("../../tests/fixtures/b1_lod2_s.gml");
        for chunk in [5, 7, 4096, 8 << 20] {
            assert!(
                may_contain_in_chunks(&with, b"appearanceMember", chunk).unwrap(),
                "{chunk}"
            );
            assert!(
                !may_contain_in_chunks(&without, b"appearanceMember", chunk).unwrap(),
                "{chunk}"
            );
        }
    }
}
