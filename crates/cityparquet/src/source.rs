//! Unified feature access over CityJSON documents and CityJSONSeq streams.

use std::fs;
use std::fs::File;
use std::io::{BufRead, BufReader};
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

fn io_err(msg: String) -> CityParquetError {
    CityParquetError::Io(msg)
}

impl Source {
    pub fn open(path: &Path) -> Result<Self> {
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
                    .map_err(|e| io_err(format!("cannot reopen {}: {e}", self.path.display())))?;
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
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
