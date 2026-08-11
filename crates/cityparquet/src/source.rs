//! Unified feature access over CityJSON documents, CityJSONSeq streams, and
//! CityGML 2.0 documents (the last via [`crate::citygml`], which synthesises a
//! CityJSON header and streams `bldg:Building`s as features).

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

fn io_err(msg: String) -> CityParquetError {
    CityParquetError::Io(msg)
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
        match crate::citygml::sniff_citygml(path) {
            Some(crate::citygml::CityGmlVersion::V2_0) => {
                let header = crate::citygml::parse_header(path)?;
                return Ok(Self {
                    path: path.to_path_buf(),
                    format: SourceFormat::CityGml,
                    header,
                    doc: None,
                    buffered: None,
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
        let file =
            File::open(path).map_err(|e| io_err(format!("cannot open {}: {e}", path.display())))?;
        let mut reader = BufReader::new(file);
        let mut first_line = String::new();
        reader
            .read_line(&mut first_line)
            .map_err(|e| io_err(format!("cannot read {}: {e}", path.display())))?;
        let first_line = first_line.trim_end_matches(['\n', '\r']);
        let has_feature_lines = {
            let mut has_more = false;
            for line in reader.lines() {
                let line =
                    line.map_err(|e| io_err(format!("cannot read {}: {e}", path.display())))?;
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
            })
        } else {
            let text = fs::read_to_string(path)
                .map_err(|e| io_err(format!("cannot read {}: {e}", path.display())))?;
            let mut doc =
                CityJSON::from_str(&text).map_err(|e| err(format!("invalid CityJSON: {e}")))?;
            doc.sort_cjfeatures(SortingStrategy::Lexicographical);
            let header = doc.get_metadata();
            Ok(Self {
                path: path.to_path_buf(),
                format: SourceFormat::CityJson,
                header,
                doc: Some(doc),
                buffered: None,
            })
        }
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
    /// no-op case. A caller sets
    /// [`crate::package::ConvertOptions::crs_override`] only when this returns
    /// `true`, so the footer's `crs_source` stamp never claims an operator
    /// supplied a CRS the source carried itself.
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
        true
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
                let file = File::open(&self.path)
                    .map_err(|e| io_err(format!("cannot reopen {}: {e}", self.path.display())))?;
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
        }
    }
}

pub enum FeatureIter<'a> {
    Seq(std::io::Lines<BufReader<File>>),
    Doc {
        doc: &'a CityJSON,
        i: usize,
    },
    CityGml(Box<crate::citygml::FeatureReader>),
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
                    Err(e) => return Some(Err(io_err(format!("read error: {e}")))),
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

    #[test]
    fn open_nonexistent_path_is_io_error() {
        match Source::open(Path::new("/no/such/path/city.jsonl")) {
            Ok(_) => panic!("expected an error opening a nonexistent path"),
            Err(e) => assert!(
                matches!(e, CityParquetError::Io(_)),
                "expected Io error, got {e:?}"
            ),
        }
    }
}
