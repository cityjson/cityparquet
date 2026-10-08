//! Unified feature access over CityJSON documents, CityJSONSeq streams,
//! CityGML 2.0 documents (via [`crate::citygml`], which synthesises a
//! CityJSON header and streams `bldg:Building`s as features) and, with the
//! `fcb` feature, FlatCityBuf files (via `crate::fcb`).

use std::collections::{BTreeSet, HashSet};
use std::fs;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use cityparquet_schema::{CityParquetError, Result};
use cjseq::{Appearance, CityJSON, CityJSONFeature, SortingStrategy};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceFormat {
    CityJson,
    CityJsonSeq,
    CityGml,
    FlatCityBuf,
}

pub struct Source {
    path: PathBuf,
    format: SourceFormat,
    header: CityJSON,
    /// Parsed whole document (CityJson format only), pre-sorted for
    /// deterministic feature emission.
    doc: Option<CityJSON>,
    /// In-memory features + doc appearance, set by [`Source::from_parts`] for
    /// a synthetic source (the merge/partition pipeline). When present it is
    /// the sole feature source — `path`/`doc` are unused — so `features()`
    /// yields from it directly rather than reopening any file.
    buffered: Option<BufferedSource>,
    /// Whether `header.metadata.reference_system` was declared by an OPERATOR
    /// ([`Source::set_reference_system`]) rather than carried by the source
    /// itself. The one place this fact lives: the writer's `crs_source`
    /// provenance stamp and the verbatim `source_metadata` passthrough both
    /// read it from here, so no caller can put the header and the provenance
    /// out of step.
    crs_is_operator_supplied: bool,
}

/// The backing store for an in-memory [`Source`] (see [`Source::from_parts`]):
/// buffered features plus the doc-level appearance array their
/// geometry-template `material`/`texture` maps resolve against.
struct BufferedSource {
    features: Vec<CityJSONFeature>,
    doc_appearance: Option<Appearance>,
}

fn err(msg: String) -> CityParquetError {
    CityParquetError::Schema(msg)
}

impl Source {
    pub fn open(path: &Path) -> Result<Self> {
        // CityGML is XML, not JSON — detect it by its root element before the
        // CityJSON/Seq sniff below. A CityGML document of an unsupported
        // version is reported as such: letting it fall through to the JSON
        // branch produced "invalid CityJSON: expected value at line 1 column 1"
        // for an XML file, which is actively misleading. For 2.0 the reader
        // synthesises a CityJSON header (transform + CRS) and streams
        // `bldg:Building`s as features.
        // FlatCityBuf is binary: recognised by its magic bytes.
        if crate::fcb::is_flatcitybuf(path) {
            return Self::open_flatcitybuf(path);
        }
        match crate::citygml::sniff_citygml(path) {
            Some(crate::citygml::CityGmlVersion::V2_0) => {
                let header = crate::citygml::parse_header(path)?;
                return Ok(Self {
                    path: path.to_path_buf(),
                    format: SourceFormat::CityGml,
                    header,
                    doc: None,
                    buffered: None,
                    crs_is_operator_supplied: false,
                });
            }
            Some(crate::citygml::CityGmlVersion::Other(version)) => {
                return Err(err(format!(
                    "unsupported CityGML version {version} (only CityGML 2.0 is supported)"
                )));
            }
            None => {}
        }

        // CityJSONSeq: first line is a CityJSON header, later lines are features.
        // A document only counts as Seq when a feature stream actually follows
        // the header — a minified CityJSON doc with a trailing newline must not
        // be misclassified (its lone line would be skipped as the "header").
        //
        // Sniffing only ever needs the first line plus proof that a further
        // non-empty line follows it, so this reads at most those two lines —
        // never the whole file. Only the CityJson (non-Seq) branch below
        // reads the full document, because it must parse it in one piece.
        let file = File::open(path).map_err(|e| {
            CityParquetError::io_source(format!("cannot open {}", path.display()), e)
        })?;
        let mut reader = BufReader::new(file);
        let mut first_line = String::new();
        reader.read_line(&mut first_line).map_err(|e| {
            CityParquetError::io_source(format!("cannot read {}", path.display()), e)
        })?;
        let first_line = first_line.trim_end_matches(['\n', '\r']);
        let has_feature_lines = {
            let mut has_more = false;
            for line in reader.lines() {
                let line = line.map_err(|e| {
                    CityParquetError::io_source(format!("cannot read {}", path.display()), e)
                })?;
                if !line.trim().is_empty() {
                    has_more = true;
                    break;
                }
            }
            has_more
        };
        let is_seq = path.extension().is_some_and(|e| e == "jsonl")
            || (has_feature_lines
                && serde_json::from_str::<serde_json::Value>(first_line)
                    .ok()
                    .and_then(|v| v.get("type").and_then(|t| t.as_str().map(String::from)))
                    .as_deref()
                    == Some("CityJSON"));
        if is_seq {
            let header = CityJSON::from_str(first_line)
                .map_err(|e| err(format!("invalid CityJSONSeq header: {e}")))?;
            Ok(Self {
                path: path.to_path_buf(),
                format: SourceFormat::CityJsonSeq,
                header,
                doc: None,
                buffered: None,
                crs_is_operator_supplied: false,
            })
        } else {
            let text = fs::read_to_string(path).map_err(|e| {
                CityParquetError::io_source(format!("cannot read {}", path.display()), e)
            })?;
            let mut doc =
                CityJSON::from_str(&text).map_err(|e| err(format!("invalid CityJSON: {e}")))?;
            validate_document_hierarchy(&doc)?;
            doc.sort_cjfeatures(SortingStrategy::Lexicographical);
            let header = doc.get_metadata();
            Ok(Self {
                path: path.to_path_buf(),
                format: SourceFormat::CityJson,
                header,
                doc: Some(doc),
                buffered: None,
                crs_is_operator_supplied: false,
            })
        }
    }

    #[cfg(feature = "fcb")]
    fn open_flatcitybuf(path: &Path) -> Result<Self> {
        Ok(Self {
            path: path.to_path_buf(),
            format: SourceFormat::FlatCityBuf,
            header: crate::fcb::read_header(path)?,
            doc: None,
            buffered: None,
            crs_is_operator_supplied: false,
        })
    }

    #[cfg(not(feature = "fcb"))]
    fn open_flatcitybuf(path: &Path) -> Result<Self> {
        Err(err(format!(
            "{} is a FlatCityBuf file; reading one needs the `fcb` feature",
            path.display()
        )))
    }

    /// Build an in-memory source from already-parsed parts: a `header`
    /// (transform + metadata + geometry templates), the `features` to stream,
    /// the doc-level `doc_appearance` array those features' template maps
    /// resolve against, and the `format` tag to report. Used by the
    /// merge/partition pipeline to feed a buffered feature subset through the
    /// same `scan`/`encode`/`convert` machinery a file-backed source drives —
    /// no file is ever opened. Callers pass [`SourceFormat::CityJsonSeq`]: a
    /// buffered feature is self-contained (feature-local appearance), the Seq
    /// convention.
    pub fn from_parts(
        header: CityJSON,
        features: Vec<CityJSONFeature>,
        doc_appearance: Option<Appearance>,
        format: SourceFormat,
    ) -> Self {
        Self {
            path: PathBuf::new(),
            format,
            header,
            doc: None,
            buffered: Some(BufferedSource {
                features,
                doc_appearance,
            }),
            crs_is_operator_supplied: false,
        }
    }

    pub fn format(&self) -> SourceFormat {
        self.format
    }

    pub fn header(&self) -> &CityJSON {
        &self.header
    }

    /// Declare `epsg_code` (e.g. `"EPSG:7415"`, or the bare `"7415"`) as this
    /// source's reference system when it has none.
    ///
    /// An operator-supplied CRS for a source that declares none — see
    /// [`crate::package::ConvertOptions::crs_override`]. Deliberately a no-op
    /// when the source already declares a CRS: an override must never
    /// silently reproject or relabel data that came with its own, correct
    /// CRS.
    ///
    /// Returns whether the declaration was actually applied — `false` for that
    /// no-op case — and records the same fact on the source itself
    /// ([`Source::crs_is_operator_supplied`]), which is what the writer stamps
    /// its `crs_source` provenance from. The footer therefore can never claim
    /// an operator supplied a CRS the source carried itself, whatever the
    /// caller does with [`crate::package::ConvertOptions::crs_override`].
    pub fn set_reference_system(&mut self, epsg_code: &str) -> bool {
        let code = epsg_code
            .trim()
            .trim_start_matches("EPSG:")
            .trim()
            .to_string();
        let rs = cjseq::ReferenceSystem::new(None, "EPSG".to_string(), "0".to_string(), code);
        let metadata = self.header.metadata.get_or_insert(cjseq::Metadata {
            geographical_extent: None,
            identifier: None,
            point_of_contact: None,
            reference_date: None,
            reference_system: None,
            title: None,
        });
        if metadata.reference_system.is_some() {
            return false;
        }
        metadata.reference_system = Some(rs);
        self.crs_is_operator_supplied = true;
        // A CityGML source has not been quantised yet — `parse_header` only
        // chose a transform, and `features()` applies it when the document is
        // streamed — so an operator-supplied CRS must re-derive the step from
        // the units it declares, or a degree-valued override would be read at
        // a metre-sized one. A CityJSON/Seq source is the opposite case: its
        // vertices are ALREADY integers against its own `transform`, and
        // changing the scale would silently reinterpret every one of them.
        if self.format == SourceFormat::CityGml
            && let Some(rs) = self.header.metadata.as_ref().and_then(|m| {
                m.reference_system
                    .as_ref()
                    .map(cjseq::ReferenceSystem::to_url)
            })
            && let Ok(projjson) = cityparquet_schema::crs::resolve_to_projjson(&rs)
            && let Ok(scale) = cityparquet_schema::crs::axis_scale(&projjson)
        {
            self.header.transform.scale = scale.to_vec();
        }
        true
    }

    /// Whether this source's declared CRS came from an operator
    /// ([`Source::set_reference_system`]) rather than from the source itself.
    ///
    /// Two things in the writer key off it: the footer's
    /// `city.other.crs_source` stamp, and the exclusion of the injected
    /// `referenceSystem` from the verbatim `city.other.source_metadata`
    /// passthrough (the source header `metadata` must stay exactly what the
    /// source carried).
    pub fn crs_is_operator_supplied(&self) -> bool {
        self.crs_is_operator_supplied
    }

    /// Carry an established operator-supplied-CRS provenance onto a source
    /// derived from this one.
    ///
    /// [`Source::from_parts`] rebuilds a `Source` around an already-merged or
    /// already-partitioned header; the provenance is a property of THAT
    /// header, so it has to travel with it — otherwise a merged or partitioned
    /// run writes the operator's CRS with no record of where it came from.
    pub fn with_crs_operator_supplied(mut self, operator_supplied: bool) -> Self {
        self.crs_is_operator_supplied = operator_supplied;
        self
    }

    /// The RAW (unsliced) appearance array that this source's
    /// `header().geometry_templates`' template `material`/`texture` maps
    /// actually index into.
    ///
    /// For a whole-document CityJSON source, `header()` is `doc.get_metadata()`:
    /// it slices `appearance` down to only the entries referenced by
    /// templates, renumbered to a local 0.. sequence — but it does so
    /// against a SEPARATE clone of the templates it builds internally and
    /// discards; `header().geometry_templates` itself is a bare clone of the
    /// document's original templates, whose `material`/`texture` maps still
    /// carry the document's GLOBAL indices (see `cjseq::CityJSON::get_metadata`:
    /// it mutates `gts2` — a clone — to compute the renumbering, while the
    /// header's own `geometry_templates` field is `self.geometry_templates.clone()`,
    /// untouched). So the only appearance array the header's template maps
    /// resolve against is the raw document's own `appearance`, never
    /// `header().appearance`.
    ///
    /// For a CityJSONSeq source there is no separate "raw document" — the
    /// header IS the stream's first line, and whatever produced the file is
    /// responsible for keeping that line's `appearance` and
    /// `geometry-templates` mutually consistent (sliced/remapped together,
    /// if at all). `header().appearance` is therefore already the right defs
    /// array in that case.
    pub fn doc_appearance(&self) -> Option<&cjseq::Appearance> {
        if let Some(buffered) = &self.buffered {
            return buffered.doc_appearance.as_ref();
        }
        match &self.doc {
            Some(doc) => doc.appearance.as_ref(),
            None => self.header.appearance.as_ref(),
        }
    }

    pub fn features(&self) -> Result<FeatureIter<'_>> {
        if let Some(buffered) = &self.buffered {
            return Ok(FeatureIter::Buffered(buffered.features.iter()));
        }
        match self.format {
            SourceFormat::CityJsonSeq => {
                let file = File::open(&self.path).map_err(|e| {
                    CityParquetError::io_source(format!("cannot reopen {}", self.path.display()), e)
                })?;
                let mut lines = BufReader::new(file).lines();
                lines.next(); // skip header line
                Ok(FeatureIter::Seq(lines))
            }
            SourceFormat::CityJson => Ok(FeatureIter::Doc {
                doc: self.doc.as_ref().expect("doc set"),
                i: 0,
            }),
            SourceFormat::CityGml => Ok(FeatureIter::CityGml(Box::new(
                crate::citygml::FeatureReader::open(&self.path, &self.header.transform)?,
            ))),
            #[cfg(feature = "fcb")]
            SourceFormat::FlatCityBuf => Ok(FeatureIter::FlatCityBuf(Box::new(
                crate::fcb::FcbFeatures::open(&self.path)?,
            ))),
            // `Source::open` refuses a FlatCityBuf file without the feature.
            #[cfg(not(feature = "fcb"))]
            SourceFormat::FlatCityBuf => Err(err(format!(
                "{}: reading FlatCityBuf needs the `fcb` feature",
                self.path.display()
            ))),
        }
    }
}

impl Source {
    /// The features [`Source::features`] yields, in the order `order` gives
    /// as indices into that sequence — without holding them all parsed at
    /// once where the source allows random access: a CityJSONSeq line is
    /// re-read by its byte span, a CityJSON document feature built from the
    /// parsed document by index, an in-memory one cloned. A CityGML or
    /// FlatCityBuf source streams only front to back, so its features are
    /// parsed once and handed out in order.
    pub fn features_in_order(&self, order: Vec<usize>) -> Result<OrderedFeatures<'_>> {
        let access = if let Some(buffered) = &self.buffered {
            Access::Buffered(&buffered.features)
        } else {
            match self.format {
                SourceFormat::CityJsonSeq => {
                    let (file, spans) = seq_line_spans(&self.path)?;
                    Access::Lines {
                        file,
                        spans,
                        line: String::new(),
                    }
                }
                SourceFormat::CityJson => Access::Doc(self.doc.as_ref().expect("doc set")),
                SourceFormat::CityGml | SourceFormat::FlatCityBuf => {
                    return Ok(OrderedFeatures::parsed(
                        self.features()?.collect::<Result<Vec<_>>>()?,
                        order,
                    ));
                }
            }
        };
        Ok(OrderedFeatures {
            order: order.into_iter(),
            access,
        })
    }

    /// Whether this source can only be read front to back — a CityGML or
    /// FlatCityBuf file — so that reading its features in another order
    /// means holding them all parsed.
    pub(crate) fn reads_front_to_back(&self) -> bool {
        self.buffered.is_none()
            && matches!(
                self.format,
                SourceFormat::CityGml | SourceFormat::FlatCityBuf
            )
    }
}

impl OrderedFeatures<'static> {
    /// `features` — every feature of a source, already parsed in its
    /// [`Source::features`] order — handed out in `order`.
    pub(crate) fn parsed(features: Vec<CityJSONFeature>, order: Vec<usize>) -> Self {
        OrderedFeatures {
            order: order.into_iter(),
            access: Access::Parsed(features.into_iter().map(Some).collect()),
        }
    }
}

/// The byte span (start, length) of every feature line of a CityJSONSeq
/// file — the lines [`FeatureIter::Seq`] parses: every line after the header
/// line that is not blank, without its line terminator.
fn seq_line_spans(path: &Path) -> Result<(File, Vec<(u64, usize)>)> {
    let io = |e| CityParquetError::io_source(format!("cannot read {}", path.display()), e);
    let file = File::open(path).map_err(io)?;
    let mut reader = BufReader::with_capacity(1 << 20, &file);
    let mut spans = Vec::new();
    let mut line = Vec::new();
    let mut at = 0u64;
    let mut first = true;
    loop {
        line.clear();
        let read = reader.read_until(b'\n', &mut line).map_err(io)?;
        if read == 0 {
            break;
        }
        let start = at;
        at += read as u64;
        if std::mem::take(&mut first) {
            continue;
        }
        let mut len = line.len();
        if line.ends_with(b"\n") {
            len -= 1;
            if line[..len].ends_with(b"\r") {
                len -= 1;
            }
        }
        if line[..len].iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        spans.push((start, len));
    }
    drop(reader);
    Ok((file, spans))
}

/// The iterator [`Source::features_in_order`] returns.
pub struct OrderedFeatures<'a> {
    order: std::vec::IntoIter<usize>,
    access: Access<'a>,
}

enum Access<'a> {
    Lines {
        file: File,
        spans: Vec<(u64, usize)>,
        line: String,
    },
    Doc(&'a CityJSON),
    Buffered(&'a [CityJSONFeature]),
    /// Taken out as they are handed over, so each is moved, never cloned.
    Parsed(Vec<Option<CityJSONFeature>>),
}

impl Iterator for OrderedFeatures<'_> {
    type Item = Result<CityJSONFeature>;

    fn next(&mut self) -> Option<Self::Item> {
        let i = self.order.next()?;
        let missing = || err(format!("feature {i} is not in the source"));
        Some(match &mut self.access {
            Access::Lines { file, spans, line } => {
                let Some(&(start, len)) = spans.get(i) else {
                    return Some(Err(missing()));
                };
                read_span(file, start, len, line).and_then(|()| {
                    CityJSONFeature::from_str(line)
                        .map_err(|e| err(format!("invalid CityJSONFeature line: {e}")))
                })
            }
            Access::Doc(doc) => doc.get_cjfeature(i).ok_or_else(missing),
            Access::Buffered(features) => features.get(i).cloned().ok_or_else(missing),
            Access::Parsed(features) => features
                .get_mut(i)
                .and_then(Option::take)
                .ok_or_else(missing),
        })
    }
}

fn read_span(file: &mut File, start: u64, len: usize, line: &mut String) -> Result<()> {
    use std::io::{Read, Seek, SeekFrom};
    let io = |e| CityParquetError::io_source("read error", e);
    file.seek(SeekFrom::Start(start)).map_err(io)?;
    let mut bytes = std::mem::take(line).into_bytes();
    bytes.clear();
    bytes.resize(len, 0);
    file.read_exact(&mut bytes).map_err(io)?;
    *line =
        String::from_utf8(bytes).map_err(|e| err(format!("invalid CityJSONFeature line: {e}")))?;
    Ok(())
}

/// cjseq's own rule (`CityObject::is_toplevel`, private): an object is
/// top-level when it declares no parents, an absent and an empty `parents`
/// array counting alike.
fn is_toplevel(co: &cjseq::CityObject) -> bool {
    co.parents.as_ref().is_none_or(std::vec::Vec::is_empty)
}

/// Render at most three ids, so a pathological document cannot produce a
/// screenful of error.
fn name_a_few(ids: &BTreeSet<&str>) -> String {
    let shown: Vec<&str> = ids.iter().take(3).copied().collect();
    match ids.len().saturating_sub(shown.len()) {
        0 => shown.join(", "),
        rest => format!("{} (and {rest} more)", shown.join(", ")),
    }
}

/// Refuse a CityJSON document whose hierarchy [`FeatureIter::Doc`] cannot
/// stream without losing objects.
///
/// cjseq 0.4.1 builds the feature stream from two rules: one feature per
/// TOP-LEVEL object, carrying that object plus **exactly one level** of its
/// `children` — `CityJSON::get_cjfeature` carries its own
/// `//-- TODO: to fix: children-of-children?`. An object that is neither
/// top-level nor the child of a top-level object is therefore emitted by
/// nobody and vanishes without a word. Nothing downstream notices: `convert` →
/// `export` → `compare` reads the source through this same iterator on both
/// sides, so both sides drop it and the round-trip reports equality.
///
/// This reproduces those two rules verbatim rather than approximating them
/// with a depth limit, and errors when any object falls outside the emitted
/// set. It also catches the dangling child key that would otherwise panic
/// inside `get_cjfeature` (`self.city_objects.get(&childkey).unwrap()`),
/// taking the process down with no diagnostic at all.
///
/// Only the whole-document path needs this. CityJSONSeq features arrive
/// already-formed, and [`crate::citygml`]'s reader descends recursively
/// (`emit_into`), so both carry arbitrarily deep hierarchies safely.
///
/// It guards [`Source`], which is every read path the CLI drives — convert,
/// export and compare all go through it. The read benchmark
/// (`cityparquet-readbench`'s `formats::cityjson`) deliberately does not: it
/// parses the document itself to measure parsing, and routing it through this
/// check would put validation work inside the timed section. A >2-level
/// document would still lose objects there; the benchmark corpus is
/// two-level.
///
/// Delete this the day cjseq descends transitively — the canary test
/// `cjseq_still_drops_grandchildren_so_this_guard_is_still_needed` fails on
/// that day and says so. Note the workspace depends on `cjseq = "0.4"`, not a
/// pinned patch, so the day can arrive without anyone editing this repo.
fn validate_document_hierarchy(doc: &CityJSON) -> Result<()> {
    let mut emitted: HashSet<&str> = HashSet::new();
    let mut missing: BTreeSet<&str> = BTreeSet::new();

    for (id, co) in &doc.city_objects {
        if !is_toplevel(co) {
            continue;
        }
        emitted.insert(id.as_str());
        for child in co.children.iter().flatten() {
            match doc.city_objects.get_key_value(child) {
                Some((child_id, _)) => {
                    emitted.insert(child_id.as_str());
                }
                None => {
                    missing.insert(child.as_str());
                }
            }
        }
    }

    if !missing.is_empty() {
        return Err(err(format!(
            "CityJSON document names {} child object(s) it does not contain: {}",
            missing.len(),
            name_a_few(&missing)
        )));
    }

    let dropped: BTreeSet<&str> = doc
        .city_objects
        .keys()
        .map(String::as_str)
        .filter(|id| !emitted.contains(id))
        .collect();
    if !dropped.is_empty() {
        return Err(err(format!(
            "CityJSON document nests city objects more than two levels deep: {} object(s) are \
             neither top-level nor the child of a top-level object and would be dropped \
             silently: {}. Convert the document to CityJSONSeq first, or flatten the hierarchy.",
            dropped.len(),
            name_a_few(&dropped)
        )));
    }

    Ok(())
}

pub enum FeatureIter<'a> {
    Seq(std::io::Lines<BufReader<File>>),
    Doc {
        doc: &'a CityJSON,
        i: usize,
    },
    CityGml(Box<crate::citygml::FeatureReader>),
    #[cfg(feature = "fcb")]
    FlatCityBuf(Box<crate::fcb::FcbFeatures>),
    /// In-memory features (an [`Source::from_parts`] source); each is cloned
    /// on yield so the iterator can hand back owned `CityJSONFeature`s like
    /// every other arm while the buffer stays intact for re-iteration.
    Buffered(std::slice::Iter<'a, CityJSONFeature>),
}

impl Iterator for FeatureIter<'_> {
    type Item = Result<CityJSONFeature>;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            FeatureIter::Seq(lines) => loop {
                match lines.next()? {
                    Err(e) => return Some(Err(CityParquetError::io_source("read error", e))),
                    Ok(line) if line.trim().is_empty() => continue,
                    Ok(line) => {
                        return Some(
                            CityJSONFeature::from_str(&line)
                                .map_err(|e| err(format!("invalid CityJSONFeature line: {e}"))),
                        );
                    }
                }
            },
            FeatureIter::Doc { doc, i } => {
                let f = doc.get_cjfeature(*i)?;
                *i += 1;
                Some(Ok(f))
            }
            FeatureIter::CityGml(reader) => reader.next(),
            #[cfg(feature = "fcb")]
            FeatureIter::FlatCityBuf(reader) => reader.next(),
            FeatureIter::Buffered(iter) => iter.next().map(|f| Ok(f.clone())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffered_source_round_trips_features_and_header() {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/delft.city.jsonl");
        let disk = Source::open(&path).unwrap();
        let feats: Vec<_> = disk
            .features()
            .unwrap()
            .map(|f| f.unwrap())
            .take(3)
            .collect();
        let mem = Source::from_parts(
            disk.header().clone(),
            feats.clone(),
            None,
            SourceFormat::CityJsonSeq,
        );
        let got: Vec<_> = mem.features().unwrap().map(|f| f.unwrap()).collect();
        assert_eq!(got.len(), 3);
        assert_eq!(got[0].id, feats[0].id);
        assert_eq!(mem.format(), SourceFormat::CityJsonSeq);
        // Re-iteration works (buffer is not consumed).
        assert_eq!(mem.features().unwrap().count(), 3);
    }

    /// `features_in_order` yields exactly the features `features()` does,
    /// in the order asked for — for every kind of source: a CityJSONSeq
    /// (read back line by line), a CityJSON document (by index), a CityGML
    /// document (parsed and reordered) and an in-memory source.
    #[test]
    fn features_in_order_yields_the_features_in_the_given_order() {
        let fixtures =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
        let delft = Source::open(&fixtures.join("delft.city.jsonl")).unwrap();
        let buffered = Source::from_parts(
            delft.header().clone(),
            delft
                .features()
                .unwrap()
                .map(|f| f.unwrap())
                .take(50)
                .collect(),
            None,
            SourceFormat::CityJsonSeq,
        );
        for source in [
            delft,
            Source::open(&fixtures.join("lod3_railway.city.json")).unwrap(),
            Source::open(&fixtures.join("b1_lod2_cs_w_sem.gml")).unwrap(),
            buffered,
        ] {
            let forward: Vec<CityJSONFeature> =
                source.features().unwrap().map(|f| f.unwrap()).collect();
            // Reversed, with every other feature first, so no order is
            // accidentally the natural one.
            let order: Vec<usize> = (0..forward.len())
                .rev()
                .filter(|i| i % 2 == 0)
                .chain((0..forward.len()).filter(|i| i % 2 == 1))
                .collect();
            let got: Vec<CityJSONFeature> = source
                .features_in_order(order.clone())
                .unwrap()
                .map(|f| f.unwrap())
                .collect();
            assert_eq!(got.len(), forward.len(), "{:?}", source.format());
            for (feature, &i) in got.iter().zip(&order) {
                assert_eq!(
                    serde_json::to_value(feature).unwrap(),
                    serde_json::to_value(&forward[i]).unwrap(),
                    "{:?} feature {i}",
                    source.format()
                );
            }
        }
    }

    #[test]
    fn open_nonexistent_path_is_io_error() {
        match Source::open(Path::new("/no/such/path/city.jsonl")) {
            Ok(_) => panic!("expected an error opening a nonexistent path"),
            Err(e) => assert!(
                matches!(e, CityParquetError::Io { .. }),
                "expected Io error, got {e:?}"
            ),
        }
    }
}
