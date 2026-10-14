//! The document preamble: the dataset CRS and the `gml:Envelope` that
//! precedes the first `cityObjectMember`.

use std::io::BufRead;

use citygml_core::Result;
use citygml_core::xml::{NS_GML, get_attr, ns_is, read_text, xml_err};
use quick_xml::events::Event;
use quick_xml::reader::NsReader;

/// What a CityGML document declares about itself before its first object.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Preamble {
    /// The first `srsName` in the document.
    pub srs_name: Option<String>,
    /// The CityModel's envelope as `[minx, miny, minz, maxx, maxy, maxz]`; a
    /// two-dimensional corner has a z of 0.
    pub envelope: Option<[f64; 6]>,
}

/// Read the [`Preamble`] of the document `input` starts.
///
/// The CRS may sit on the `CityModel`, its envelope, or (in exports that
/// declare it nowhere else) on each object's own `gml:boundedBy`. Past the
/// first `cityObjectMember` this keeps reading for an `srsName` only, never a
/// corner, since one object's envelope is not the dataset's, and gives up
/// after a bounded number of elements so a CRS-less document is not read
/// end to end. It consumes what it reads: a streaming caller that needs
/// those bytes again records and replays them.
pub fn scan_preamble<R: BufRead>(input: R) -> Result<Preamble> {
    const MAX_FALLBACK_EVENTS: usize = 100_000;

    let mut reader = NsReader::from_reader(input);
    let mut buf = Vec::new();
    let mut srs_name: Option<String> = None;
    let mut lower: Option<[f64; 3]> = None;
    let mut upper: Option<[f64; 3]> = None;
    let mut past_preamble = false;
    let mut fallback_events = 0usize;

    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(&mut buf).map_err(xml_err)?;
        match ev {
            Event::Start(e) => {
                let name = e.local_name().as_ref().to_vec();
                if srs_name.is_none()
                    && let Some(s) = get_attr(&e, b"srsName")
                {
                    srs_name = Some(s);
                    if past_preamble {
                        break;
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
                        break;
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
    Ok(Preamble { srs_name, envelope })
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
