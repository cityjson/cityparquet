//! FlatCityBuf (`.fcb`) input: the header becomes the CityJSON header
//! `fcb_core` reconstructs from it, and the features stream front to back,
//! each handed over as the CityJSONFeature `fcb_core` decodes it to.
//!
//! `fcb_core` speaks `cjseq2`'s CityJSON types, a different crate from the
//! `cjseq` this library uses; the two meet through JSON, one feature at a
//! time. The sniff ([`is_flatcitybuf`]) needs no feature, so a FlatCityBuf
//! file given to a build without the `fcb` feature is named as such.

use std::fs::File;
use std::io::Read;
use std::path::Path;

/// The FlatCityBuf magic: `fcb`, a version byte, `fcb`, a zero byte.
pub(crate) fn is_flatcitybuf(path: &Path) -> bool {
    let mut magic = [0u8; 8];
    File::open(path)
        .and_then(|mut f| f.read_exact(&mut magic))
        .is_ok()
        && magic[0..3] == *b"fcb"
        && magic[4..7] == *b"fcb"
}

#[cfg(feature = "fcb")]
pub use reader::FcbFeatures;
#[cfg(feature = "fcb")]
pub(crate) use reader::read_header;

#[cfg(feature = "fcb")]
mod reader {
    use super::*;

    use std::io::BufReader;

    use cityparquet_schema::{CityParquetError, Result};
    use cjseq::{CityJSON, CityJSONFeature};
    use fcb_core::FcbReader;
    use fcb_core::deserializer::to_cj_metadata;
    use fcb_core::reader_trait::NotSeekable;

    fn err(path: &Path, what: &str, e: impl std::fmt::Display) -> CityParquetError {
        CityParquetError::Schema(format!("{}: {what}: {e}", path.display()))
    }

    fn open(path: &Path) -> Result<FcbReader<BufReader<File>>> {
        let file = File::open(path).map_err(|e| {
            CityParquetError::io_source(format!("cannot open {}", path.display()), e)
        })?;
        FcbReader::open(BufReader::new(file))
            .map_err(|e| err(path, "invalid FlatCityBuf header", e))
    }

    /// The CityJSON header (transform, metadata, extensions, geometry
    /// templates) the file's FlatCityBuf header encodes.
    pub(crate) fn read_header(path: &Path) -> Result<CityJSON> {
        let reader = open(path)?;
        let metadata =
            to_cj_metadata(&reader.header()).map_err(|e| err(path, "invalid header", e))?;
        let json = serde_json::to_string(&metadata)?;
        CityJSON::from_str(&json).map_err(|e| err(path, "invalid CityJSON header", e))
    }

    /// A FlatCityBuf file's features, in file order, as CityJSONFeatures.
    pub struct FcbFeatures {
        path: std::path::PathBuf,
        iter: fcb_core::FeatureIter<BufReader<File>, NotSeekable>,
        /// The header's `features_count`; `None` when it is 0, which
        /// `fcb_core` reads as "not stated": the stream then runs to the end
        /// of the feature section.
        declared: Option<u64>,
        /// Features yielded so far.
        read: u64,
        done: bool,
    }

    impl FcbFeatures {
        pub(crate) fn open(path: &Path) -> Result<Self> {
            let iter = open(path)?
                .select_all_seq()
                .map_err(|e| err(path, "cannot read the features", e))?;
            let declared = Some(iter.header().features_count()).filter(|&n| n > 0);
            Ok(Self {
                path: path.to_path_buf(),
                iter,
                declared,
                read: 0,
                done: false,
            })
        }
    }

    impl Iterator for FcbFeatures {
        type Item = Result<CityJSONFeature>;

        fn next(&mut self) -> Option<Self::Item> {
            if self.done || self.declared.is_some_and(|n| self.read >= n) {
                return None;
            }
            let path = &self.path;
            let feature = match self.iter.next() {
                Err(e) => {
                    self.done = true;
                    return Some(Err(err(path, "cannot read a feature", e)));
                }
                Ok(None) => {
                    self.done = true;
                    // A stated count the feature section falls short of is a
                    // truncated file, not a smaller dataset.
                    return self.declared.map(|n| {
                        Err(CityParquetError::Schema(format!(
                            "{}: the header declares {n} features but the file holds {}",
                            path.display(),
                            self.read
                        )))
                    });
                }
                Ok(Some(iter)) => iter.cur_cj_feature(),
            };
            self.read += 1;
            Some(
                feature
                    .map_err(|e| err(path, "cannot decode a feature", e))
                    .and_then(|f| Ok(serde_json::to_string(&f)?))
                    .and_then(|json| {
                        CityJSONFeature::from_str(&json)
                            .map_err(|e| err(path, "invalid CityJSONFeature", e))
                    }),
            )
        }
    }
}
