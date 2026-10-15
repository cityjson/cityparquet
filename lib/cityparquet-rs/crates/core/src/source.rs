//! Unified feature access over CityJSON documents, CityJSONSeq streams,
//! CityGML 2.0 and 3.0 documents (via [`crate::citygml`], which synthesises
//! a CityJSON header and streams top-level city objects as features), with
//! the `fcb` feature FlatCityBuf files (via `crate::fcb`), and with the
//! `citydb` feature 3DCityDB v5 databases (via `citygml-citydb`).

use std::collections::{BTreeSet, HashSet};
use std::fs;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;

use cityparquet_schema::{CityParquetError, Result};
use cjseq::{Appearance, CityJSON, CityJSONFeature};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceFormat {
    CityJson,
    CityJsonSeq,
    CityGml,
    FlatCityBuf,
    /// A 3DCityDB v5 database ([`Source::open_citydb`]).
    CityDb,
}

pub struct Source {
    path: PathBuf,
    format: SourceFormat,
    header: CityJSON,
    /// Parsed whole document (CityJson format only), pre-sorted for
    /// deterministic feature emission.
    doc: Option<CityJSON>,
    /// What a CityGML document says document-wide (CityModel appearance,
    /// implicit-geometry templates, ADEs), read once when the source opens
    /// and shared with every feature pass (CityGml format only).
    citygml: Option<(
        crate::citygml::CityGmlVersion,
        Arc<crate::citygml::Document>,
    )>,
    /// In-memory features + doc appearance, set by [`Source::from_parts`] for
    /// a synthetic source (the merge/partition pipeline). When present it is
    /// the sole feature source — `path`/`doc` are unused — so `features()`
    /// yields from it directly rather than reopening any file.
    buffered: Option<BufferedSource>,
    /// The inputs of a merged source ([`Source::chained`]), read one after
    /// another; like `buffered`, the sole feature source when present.
    chained: Option<ChainedSource>,
    /// The database a [`SourceFormat::CityDb`] source reads, connected and
    /// inside its read-only transaction for as long as the source lives, so
    /// that every pass sees the same snapshot.
    #[cfg(feature = "citydb")]
    citydb: Option<citygml_citydb::CityDb>,
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

/// The backing store for a merged [`Source`] (see [`Source::chained`]).
struct ChainedSource {
    parts: Vec<Source>,
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
        // for an XML file, which is actively misleading. The reader of the
        // document's version synthesises a CityJSON header (transform + CRS)
        // and streams its top-level city objects as features.
        // FlatCityBuf is binary: recognised by its magic bytes.
        if crate::fcb::is_flatcitybuf(path) {
            return Self::open_flatcitybuf(path);
        }
        if let Some(version) = crate::citygml::sniff_citygml(path) {
            let document = crate::citygml::read_document(path, &version)?;
            let header = crate::citygml::parse_header_with(path, &document)?;
            return Ok(Self {
                path: path.to_path_buf(),
                format: SourceFormat::CityGml,
                header,
                doc: None,
                citygml: Some((version, Arc::new(document))),
                buffered: None,
                chained: None,
                #[cfg(feature = "citydb")]
                citydb: None,
                crs_is_operator_supplied: false,
            });
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
                citygml: None,
                buffered: None,
                chained: None,
                #[cfg(feature = "citydb")]
                citydb: None,
                crs_is_operator_supplied: false,
            })
        } else {
            let text = fs::read_to_string(path).map_err(|e| {
                CityParquetError::io_source(format!("cannot read {}", path.display()), e)
            })?;
            let doc =
                CityJSON::from_str(&text).map_err(|e| err(format!("invalid CityJSON: {e}")))?;
            validate_document_hierarchy(&doc)?;
            let header = doc
                .header()
                .map_err(|e| err(format!("invalid CityJSON header: {e}")))?;
            Ok(Self {
                path: path.to_path_buf(),
                format: SourceFormat::CityJson,
                header,
                doc: Some(doc),
                citygml: None,
                buffered: None,
                chained: None,
                #[cfg(feature = "citydb")]
                citydb: None,
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
            citygml: None,
            buffered: None,
            chained: None,
            #[cfg(feature = "citydb")]
            citydb: None,
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

    /// Open the 3DCityDB v5 database `config` names, inside a read-only
    /// transaction that lasts as long as the source.
    ///
    /// The header is the database's own: its reference system is
    /// `EPSG:<srid>` of `database_srs` (none for SRID 0, which `--crs` can
    /// then supply), its extent the envelope of its top-level features, and
    /// the transform steps by the reference system's units, as for a CityGML
    /// document, from the grid point at or below the extent's lower corner.
    /// The geometry templates are the relative geometries its implicit
    /// geometries place.
    #[cfg(feature = "citydb")]
    pub fn open_citydb(config: &citygml_citydb::Config) -> Result<Self> {
        use crate::citygml::reader::error;
        let db = citygml_citydb::CityDb::connect(config).map_err(error)?;
        let extent = db.extent().map_err(error)?;
        let srid = db.srs().srid;
        let code = (srid > 0).then(|| srid.to_string());
        let scale = match code
            .as_deref()
            .and_then(|c| cityparquet_schema::crs::resolve_to_projjson(c).ok())
        {
            Some(projjson) => cityparquet_schema::crs::axis_scale(&projjson)?,
            None => [cityparquet_schema::crs::MM; 3],
        };
        // The extent is computed, so its corner is on no grid: the origin
        // is the grid point at or below it, so that coordinates on the
        // step's grid stay on it.
        let translate = extent.map_or([0.0; 3], |e| {
            std::array::from_fn(|axis| (e[axis] / scale[axis]).floor() * scale[axis])
        });
        let mut header = CityJSON::new();
        header.transform = cjseq::Transform { scale, translate };
        if extent.is_some() || code.is_some() {
            header.metadata = Some(cjseq::Metadata {
                geographical_extent: extent,
                identifier: None,
                point_of_contact: None,
                reference_date: None,
                reference_system: code.as_deref().map(crate::citygml::crs::reference_system),
                title: None,
                other: serde_json::Map::new(),
            });
        }
        let document = db.document().map_err(error)?;
        if let Some((templates, appearance)) =
            citygml_core::convert::cjseq::geometry_templates(document.templates()).map_err(error)?
        {
            header.geometry_templates = Some(templates);
            header.appearance = appearance;
        }
        Ok(Self {
            path: PathBuf::from(db.name()),
            format: SourceFormat::CityDb,
            header,
            doc: None,
            citygml: None,
            buffered: None,
            chained: None,
            citydb: Some(db),
            crs_is_operator_supplied: false,
        })
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
            citygml: None,
            buffered: Some(BufferedSource {
                features,
                doc_appearance,
            }),
            chained: None,
            #[cfg(feature = "citydb")]
            citydb: None,
            crs_is_operator_supplied: false,
        }
    }

    /// One source reading `parts` one after another under `header`, which
    /// carries their merged transform: a part whose own transform differs has
    /// its vertices requantised onto it as its features are read. Built by
    /// [`crate::merge::merge_sources`], which checks the parts agree first.
    pub(crate) fn chained(
        header: CityJSON,
        parts: Vec<Source>,
        doc_appearance: Option<Appearance>,
    ) -> Self {
        Self {
            path: PathBuf::new(),
            format: SourceFormat::CityJsonSeq,
            header,
            doc: None,
            citygml: None,
            buffered: None,
            chained: Some(ChainedSource {
                parts,
                doc_appearance,
            }),
            #[cfg(feature = "citydb")]
            citydb: None,
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
            other: serde_json::Map::new(),
        });
        if metadata.reference_system.is_some() {
            return false;
        }
        metadata.reference_system = Some(rs);
        self.crs_is_operator_supplied = true;
        // A CityGML or database source has not been quantised yet — `parse_header` only
        // chose a transform, and `features()` applies it when the document is
        // streamed — so an operator-supplied CRS must re-derive the step from
        // the units it declares, or a degree-valued override would be read at
        // a metre-sized one. A CityJSON/Seq source is the opposite case: its
        // vertices are ALREADY integers against its own `transform`, and
        // changing the scale would silently reinterpret every one of them.
        if matches!(self.format, SourceFormat::CityGml | SourceFormat::CityDb)
            && let Some(rs) = self.header.metadata.as_ref().and_then(|m| {
                m.reference_system
                    .as_ref()
                    .map(cjseq::ReferenceSystem::to_url)
            })
            && let Ok(projjson) = cityparquet_schema::crs::resolve_to_projjson(&rs)
            && let Ok(scale) = cityparquet_schema::crs::axis_scale(&projjson)
        {
            self.header.transform.scale = scale;
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

    /// The appearance array that this source's `header().geometry_templates`'
    /// template `material`/`texture` maps index into: the header's own,
    /// which for a whole CityJSON document is the document's appearance cut
    /// down to what the templates use and renumbered with them, and for a
    /// CityJSONSeq is the stream's first line. An in-memory source carries
    /// its own.
    pub fn doc_appearance(&self) -> Option<&cjseq::Appearance> {
        if let Some(buffered) = &self.buffered {
            return buffered.doc_appearance.as_ref();
        }
        if let Some(chained) = &self.chained {
            return chained.doc_appearance.as_ref();
        }
        self.header.appearance.as_ref()
    }

    /// [`Source::features`], with a CityJSONSeq file's lines parsed a batch
    /// at a time across cores — the same features in the same order, for a
    /// caller doing enough per feature to overlap with (a conversion). Every
    /// other source streams exactly as [`Source::features`] does.
    pub fn features_batched(&self) -> Result<FeatureIter<'_>> {
        if let Some(chained) = &self.chained {
            return Ok(FeatureIter::Chained(Box::new(ChainedFeatures::new(
                &chained.parts,
                &self.header.transform,
                true,
            ))));
        }
        if self.buffered.is_none() && self.format == SourceFormat::CityJsonSeq {
            let file = File::open(&self.path).map_err(|e| {
                CityParquetError::io_source(format!("cannot reopen {}", self.path.display()), e)
            })?;
            let mut lines = BufReader::new(file).lines();
            lines.next(); // skip header line
            return Ok(FeatureIter::SeqBatched(SeqFeatures {
                lines,
                ready: Vec::new().into_iter(),
            }));
        }
        self.features()
    }

    pub fn features(&self) -> Result<FeatureIter<'_>> {
        if let Some(buffered) = &self.buffered {
            return Ok(FeatureIter::Buffered(buffered.features.iter()));
        }
        if let Some(chained) = &self.chained {
            return Ok(FeatureIter::Chained(Box::new(ChainedFeatures::new(
                &chained.parts,
                &self.header.transform,
                false,
            ))));
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
            SourceFormat::CityJson => {
                let doc = self.doc.as_ref().expect("doc set");
                Ok(FeatureIter::Doc {
                    doc,
                    ids: feature_ids(doc).into_iter(),
                })
            }
            SourceFormat::CityGml => {
                let (version, document) = self.citygml.as_ref().expect("citygml set");
                Ok(FeatureIter::CityGml(Box::new(
                    crate::citygml::FeatureReader::with_document(
                        &self.path,
                        version,
                        &self.header.transform,
                        Arc::clone(document),
                    )?,
                )))
            }
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
            #[cfg(feature = "citydb")]
            SourceFormat::CityDb => {
                let db = self.citydb.as_ref().expect("citydb set");
                Ok(FeatureIter::CityDb(Box::new(
                    db.features(self.header.transform.clone())
                        .map_err(crate::citygml::reader::error)?,
                )))
            }
            // Only `Source::open_citydb` makes one.
            #[cfg(not(feature = "citydb"))]
            SourceFormat::CityDb => Err(err("reading 3DCityDB needs the `citydb` feature".into())),
        }
    }
}

/// Up to how many feature lines, and bytes of them, are parsed together.
const PARSE_BATCH_LINES: usize = 512;
const PARSE_BATCH_BYTES: usize = 8 << 20;

/// Parse feature lines on the rayon pool, results in line order.
fn parse_feature_lines(lines: Vec<String>) -> Vec<Result<CityJSONFeature>> {
    use rayon::prelude::*;
    lines
        .into_par_iter()
        .map(|line| {
            CityJSONFeature::from_str(&line)
                .map_err(|e| err(format!("invalid CityJSONFeature line: {e}")))
        })
        .collect()
}

/// A CityJSONSeq file's features, front to back: its non-blank lines after
/// the header, read a batch at a time and parsed across cores, handed out in
/// file order.
pub struct SeqFeatures {
    lines: std::io::Lines<BufReader<File>>,
    ready: std::vec::IntoIter<Result<CityJSONFeature>>,
}

impl Iterator for SeqFeatures {
    type Item = Result<CityJSONFeature>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(next) = self.ready.next() {
            return Some(next);
        }
        let mut batch = Vec::new();
        let mut bytes = 0;
        let mut failure = None;
        while batch.len() < PARSE_BATCH_LINES && bytes < PARSE_BATCH_BYTES {
            match self.lines.next() {
                None => break,
                Some(Err(e)) => {
                    failure = Some(CityParquetError::io_source("read error", e));
                    break;
                }
                Some(Ok(line)) if line.trim().is_empty() => {}
                Some(Ok(line)) => {
                    bytes += line.len();
                    batch.push(line);
                }
            }
        }
        let mut ready = parse_feature_lines(batch);
        ready.extend(failure.map(Err));
        self.ready = ready.into_iter();
        self.ready.next()
    }
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

/// A CityJSON document's feature ids — its top-level objects — sorted, so
/// the features come out in the same order whatever order the document lists
/// its objects in.
fn feature_ids(doc: &CityJSON) -> Vec<&str> {
    let mut ids: Vec<&str> = doc
        .city_objects
        .iter()
        .filter(|(_, co)| is_toplevel(co))
        .map(|(id, _)| id.as_str())
        .collect();
    ids.sort_unstable();
    ids
}

/// The feature rooted at top-level object `id`: it and all its descendants.
fn doc_feature(doc: &CityJSON, id: &str) -> Result<CityJSONFeature> {
    doc.feature(id)
        .map_err(|e| err(format!("cannot cut feature {id} from the document: {e}")))?
        .ok_or_else(|| err(format!("feature {id} is not in the document")))
}

/// Refuse a CityJSON document that [`FeatureIter::Doc`] cannot stream without
/// losing objects.
///
/// A feature is a top-level object and every object below it through
/// `children`. An object no top-level object reaches — its `parents` name an
/// object that is not there, or a parent/child cycle has no top-level object
/// above it — is emitted by nobody and would vanish without a word, and
/// nothing downstream would notice: `convert` → `export` → `compare` reads
/// the source through this same iterator on both sides. A child id that names
/// no object is refused here too, with the ids, before any feature is cut.
///
/// Only the whole-document path needs this. CityJSONSeq features arrive
/// already-formed, and [`crate::citygml`]'s reader descends recursively
/// (`emit_into`).
fn validate_document_hierarchy(doc: &CityJSON) -> Result<()> {
    let mut emitted: HashSet<&str> = HashSet::new();
    let mut missing: BTreeSet<&str> = BTreeSet::new();
    let mut pending: Vec<&str> = doc
        .city_objects
        .iter()
        .filter(|(_, co)| is_toplevel(co))
        .map(|(id, _)| id.as_str())
        .collect();
    while let Some(id) = pending.pop() {
        if !emitted.insert(id) {
            continue;
        }
        for child in doc.city_objects[id].children.iter().flatten() {
            match doc.city_objects.get_key_value(child) {
                Some((child_id, _)) => pending.push(child_id.as_str()),
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
            "CityJSON document has {} object(s) no top-level object reaches through \
             `children`, which would be dropped silently: {}",
            dropped.len(),
            name_a_few(&dropped)
        )));
    }

    Ok(())
}

pub enum FeatureIter<'a> {
    Seq(std::io::Lines<BufReader<File>>),
    /// [`Source::features_batched`]'s CityJSONSeq stream.
    SeqBatched(SeqFeatures),
    Doc {
        doc: &'a CityJSON,
        ids: std::vec::IntoIter<&'a str>,
    },
    CityGml(Box<crate::citygml::FeatureReader>),
    #[cfg(feature = "fcb")]
    FlatCityBuf(Box<crate::fcb::FcbFeatures>),
    #[cfg(feature = "citydb")]
    CityDb(Box<citygml_citydb::Features<'a>>),
    /// In-memory features (an [`Source::from_parts`] source); each is cloned
    /// on yield so the iterator can hand back owned `CityJSONFeature`s like
    /// every other arm while the buffer stays intact for re-iteration.
    Buffered(std::slice::Iter<'a, CityJSONFeature>),
    /// A merged source's parts, one after another.
    Chained(Box<ChainedFeatures<'a>>),
}

/// The features of a [`Source::chained`] source.
pub struct ChainedFeatures<'a> {
    parts: std::slice::Iter<'a, Source>,
    /// What the parts read so far left out ([`FeatureIter::left_out`]).
    left_out: std::collections::BTreeMap<String, usize>,
    merged: &'a cjseq::Transform,
    batched: bool,
    /// The part being read, with its own transform when that differs from
    /// the merged one.
    current: Option<(FeatureIter<'a>, Option<&'a cjseq::Transform>)>,
}

impl<'a> ChainedFeatures<'a> {
    fn new(parts: &'a [Source], merged: &'a cjseq::Transform, batched: bool) -> Self {
        Self {
            parts: parts.iter(),
            left_out: std::collections::BTreeMap::new(),
            merged,
            batched,
            current: None,
        }
    }
}

impl Iterator for ChainedFeatures<'_> {
    type Item = Result<CityJSONFeature>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some((features, own)) = &mut self.current {
                match features.next() {
                    Some(Ok(mut feature)) => {
                        if let Some(own) = own {
                            crate::merge::requantise_vertices(
                                &mut feature.vertices,
                                own,
                                self.merged,
                            );
                        }
                        return Some(Ok(feature));
                    }
                    Some(Err(e)) => return Some(Err(e)),
                    None => {
                        for (name, n) in features.left_out() {
                            *self.left_out.entry(name).or_insert(0) += n;
                        }
                        self.current = None;
                    }
                }
            }
            let part = self.parts.next()?;
            let features = if self.batched {
                part.features_batched()
            } else {
                part.features()
            };
            let own = &part.header().transform;
            let differs = !crate::merge::transform_eq(own, self.merged);
            match features {
                Ok(features) => self.current = Some((features, differs.then_some(own))),
                Err(e) => return Some(Err(e)),
            }
        }
    }
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
            FeatureIter::SeqBatched(features) => features.next(),
            FeatureIter::Doc { doc, ids } => ids.next().map(|id| doc_feature(doc, id)),
            FeatureIter::CityGml(reader) => reader.next(),
            #[cfg(feature = "fcb")]
            FeatureIter::FlatCityBuf(reader) => reader.next(),
            #[cfg(feature = "citydb")]
            FeatureIter::CityDb(features) => {
                Some(features.next()?.map_err(crate::citygml::reader::error))
            }
            FeatureIter::Buffered(iter) => iter.next().map(|f| Ok(f.clone())),
            FeatureIter::Chained(features) => features.next(),
        }
    }
}

impl FeatureIter<'_> {
    /// What the source's reader has left out so far, by name: members of a
    /// class it does not read (`tran:Track`), and for a database also
    /// properties, appearances and geometries the model has no place for
    /// (`app:GeoreferencedTexture`). Empty for a source that leaves nothing
    /// out. Complete once the stream is drained.
    pub fn left_out(&self) -> std::collections::BTreeMap<String, usize> {
        match self {
            FeatureIter::CityGml(reader) => reader.skipped_members().clone(),
            #[cfg(feature = "citydb")]
            FeatureIter::CityDb(features) => {
                use citygml_core::reader::ObjectStream;
                let objects = features.reader();
                let mut left_out = objects.dropped().clone();
                for (name, n) in objects.skipped_members() {
                    *left_out.entry(name.clone()).or_insert(0) += n;
                }
                left_out
            }
            FeatureIter::Chained(features) => features.left_out.clone(),
            _ => std::collections::BTreeMap::new(),
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
