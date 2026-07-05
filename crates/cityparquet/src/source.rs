//! Unified feature access over CityJSON documents and CityJSONSeq streams.

use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};

use cityparquet_schema::{CityParquetError, Result};
use cjseq::{CityJSON, CityJSONFeature, SortingStrategy};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceFormat {
    CityJson,
    CityJsonSeq,
}

pub struct Source {
    path: PathBuf,
    format: SourceFormat,
    header: CityJSON,
    /// Parsed whole document (CityJson format only), pre-sorted for
    /// deterministic feature emission.
    doc: Option<CityJSON>,
}

fn err(msg: String) -> CityParquetError {
    CityParquetError::Schema(msg)
}

impl Source {
    pub fn open(path: &Path) -> Result<Self> {
        let mut text = String::new();
        File::open(path)
            .and_then(|mut f| f.read_to_string(&mut text))
            .map_err(|e| err(format!("cannot read {}: {e}", path.display())))?;
        // CityJSONSeq: first line is a CityJSON header, later lines are features.
        // A document only counts as Seq when a feature stream actually follows
        // the header — a minified CityJSON doc with a trailing newline must not
        // be misclassified (its lone line would be skipped as the "header").
        let mut lines = text.lines();
        let first_line = lines.next().unwrap_or_default();
        let has_feature_lines = lines.any(|l| !l.trim().is_empty());
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
            })
        } else {
            let mut doc =
                CityJSON::from_str(&text).map_err(|e| err(format!("invalid CityJSON: {e}")))?;
            doc.sort_cjfeatures(SortingStrategy::Lexicographical);
            let header = doc.get_metadata();
            Ok(Self {
                path: path.to_path_buf(),
                format: SourceFormat::CityJson,
                header,
                doc: Some(doc),
            })
        }
    }

    pub fn format(&self) -> SourceFormat {
        self.format
    }

    pub fn header(&self) -> &CityJSON {
        &self.header
    }

    pub fn features(&self) -> Result<FeatureIter<'_>> {
        match self.format {
            SourceFormat::CityJsonSeq => {
                let file = File::open(&self.path)
                    .map_err(|e| err(format!("cannot reopen {}: {e}", self.path.display())))?;
                let mut lines = BufReader::new(file).lines();
                lines.next(); // skip header line
                Ok(FeatureIter::Seq(lines))
            }
            SourceFormat::CityJson => Ok(FeatureIter::Doc {
                doc: self.doc.as_ref().expect("doc set"),
                i: 0,
            }),
        }
    }
}

pub enum FeatureIter<'a> {
    Seq(std::io::Lines<BufReader<File>>),
    Doc { doc: &'a CityJSON, i: usize },
}

impl Iterator for FeatureIter<'_> {
    type Item = Result<CityJSONFeature>;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            FeatureIter::Seq(lines) => loop {
                match lines.next()? {
                    Err(e) => return Some(Err(err(format!("read error: {e}")))),
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
        }
    }
}
