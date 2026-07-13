//! Streaming feature reader: one `cjseq::CityJSONFeature` per top-level
//! `bldg:Building`.
//!
//! Buffers exactly one building subtree at a time (memory is O(largest single
//! building)); everything else — the CityModel wrapper, `cityObjectMember`s,
//! and non-building city objects — is walked past without buffering.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use cityparquet_schema::{CityParquetError, Result};
use cjseq::{CityJSONFeature, Transform};
use quick_xml::events::Event;
use quick_xml::reader::NsReader;

use super::building::read_building;
use super::xml::{NS_BLDG, gml_id, ns_is, xml_err};

pub struct FeatureReader {
    reader: NsReader<BufReader<File>>,
    buf: Vec<u8>,
    scale: [f64; 3],
    translate: [f64; 3],
    index: usize,
    done: bool,
}

impl FeatureReader {
    /// Open `path` for streaming, quantising vertices against `transform`
    /// (which the header exposes so scan/encode dequantise consistently).
    pub fn open(path: &Path, transform: &Transform) -> Result<Self> {
        let file = File::open(path)
            .map_err(|e| CityParquetError::Io(format!("cannot reopen {}: {e}", path.display())))?;
        let scale = triple(&transform.scale, "scale")?;
        let translate = triple(&transform.translate, "translate")?;
        Ok(Self {
            reader: NsReader::from_reader(BufReader::new(file)),
            buf: Vec::new(),
            scale,
            translate,
            index: 0,
            done: false,
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
                    let is_building = ns_is(&rr, NS_BLDG) && e.local_name().as_ref() == b"Building";
                    if is_building {
                        let id = gml_id(&e);
                        // Borrows of `e`/`rr` end here (NLL) before we re-borrow.
                        let raw = read_building(&mut self.reader, &mut self.buf, id)?;
                        self.index += 1;
                        let feature = raw.into_feature(&self.scale, &self.translate, self.index)?;
                        return Ok(Some(feature));
                    }
                    // Non-building start: descend. Containers (CityModel,
                    // cityObjectMember) hold the buildings we want, and a
                    // `BuildingPart` local name is not `Building`, so it is not
                    // matched here (handled within its parent in a later
                    // milestone). Nothing to do — the next read descends.
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
