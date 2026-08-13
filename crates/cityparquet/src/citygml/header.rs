//! Synthesise a `cjseq::CityJSON` header for a CityGML 2.0 document.
//!
//! Scans only the document preamble (the `gml:Envelope`, which precedes the
//! first `cityObjectMember`) for the dataset CRS and extent, then picks the one
//! global quantisation transform every feature is encoded against:
//! `scale = [1 mm; 3]`, `translate = envelope lower corner` (or `[0, 0, 0]` when
//! there is no envelope — decided here, never per-feature).

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use cityparquet_schema::{CityParquetError, Result};
use cjseq::{CityJSON, Metadata, Transform};
use quick_xml::events::Event;
use quick_xml::reader::NsReader;

use super::crs::{self, CrsResolution};
use super::xml::{NS_GML, get_attr, ns_is, read_text, xml_err};

const MM: f64 = 0.001;

/// Build the CityJSON header (transform + metadata) for the document at `path`.
pub fn parse_header(path: &Path) -> Result<CityJSON> {
    let (srs_name, envelope) = scan_envelope(path)?;

    let translate = match &envelope {
        Some(e) => [e[0], e[1], e[2]],
        None => [0.0, 0.0, 0.0],
    };

    let mut header = CityJSON::new(); // version "2.0", identity transform
    header.transform = Transform {
        scale: vec![MM, MM, MM],
        translate: translate.to_vec(),
    };

    let mut metadata = Metadata {
        geographical_extent: None,
        identifier: None,
        point_of_contact: None,
        reference_date: None,
        reference_system: None,
        title: None,
    };
    let mut has_metadata = false;
    if let Some(e) = envelope {
        metadata.geographical_extent = Some(e);
        has_metadata = true;
    }
    if let Some(name) = &srs_name
        && let CrsResolution::Epsg(code) = crs::resolve(name)?
    {
        metadata.reference_system = Some(crs::reference_system(&code));
        has_metadata = true;
    }
    header.metadata = has_metadata.then_some(metadata);
    Ok(header)
}

/// Read the preamble and return `(srsName, [minx,miny,minz,maxx,maxy,maxz])`.
///
/// Corners are read from the preamble only. When the preamble declares no
/// `srsName`, the scan continues into city objects looking for one — and for
/// nothing else — because real exports declare the CRS per object and never
/// ahead of the first `cityObjectMember`.
fn scan_envelope(path: &Path) -> Result<(Option<String>, Option<[f64; 6]>)> {
    let file = File::open(path)
        .map_err(|e| CityParquetError::Io(format!("cannot open {}: {e}", path.display())))?;
    let mut reader = NsReader::from_reader(BufReader::new(file));
    let mut buf = Vec::new();

    let mut srs_name: Option<String> = None;
    let mut lower: Option<[f64; 3]> = None;
    let mut upper: Option<[f64; 3]> = None;

    // Once past the preamble we keep scanning for an `srsName` ONLY — never
    // for corners. Real exports (e.g. Freiburg's 1.86 GiB file) declare the CRS
    // on every object's own `gml:boundedBy` and never in the preamble; adopting
    // one of those envelopes as the DATASET extent would report a single
    // building's extent and skew the quantisation origin, so corners stay
    // preamble-only.
    let mut past_preamble = false;
    // Bound the fallback: a real file declares its CRS on the first object, so
    // this stops almost immediately. The cap keeps a pathological CRS-less file
    // from being read end-to-end just to fail.
    let mut fallback_events = 0usize;
    const MAX_FALLBACK_EVENTS: usize = 100_000;

    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(&mut buf).map_err(xml_err)?;
        match ev {
            Event::Start(e) => {
                let local = e.local_name();
                let name = local.as_ref().to_vec();
                // The dataset CRS can sit on the CityModel root or the Envelope.
                if srs_name.is_none()
                    && let Some(s) = get_attr(&e, b"srsName")
                {
                    srs_name = Some(s);
                    if past_preamble {
                        break; // fallback satisfied
                    }
                }
                if past_preamble {
                    fallback_events += 1;
                    if fallback_events >= MAX_FALLBACK_EVENTS {
                        break;
                    }
                    continue;
                }
                if ns_is(&rr, NS_GML) && name == b"lowerCorner" {
                    lower = parse_corner(&read_text(&mut reader, &mut buf)?);
                } else if ns_is(&rr, NS_GML) && name == b"upperCorner" {
                    upper = parse_corner(&read_text(&mut reader, &mut buf)?);
                } else if name == b"cityObjectMember" {
                    if srs_name.is_some() {
                        break; // preamble gave us everything
                    }
                    past_preamble = true;
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }

    let envelope = match (lower, upper) {
        (Some(l), Some(u)) => Some([l[0], l[1], l[2], u[0], u[1], u[2]]),
        _ => None,
    };
    Ok((srs_name, envelope))
}

fn parse_corner(text: &str) -> Option<[f64; 3]> {
    let v: Vec<f64> = text
        .split_whitespace()
        .filter_map(|t| t.parse().ok())
        .collect();
    match v.len() {
        3 => Some([v[0], v[1], v[2]]),
        2 => Some([v[0], v[1], 0.0]),
        _ => None,
    }
}
