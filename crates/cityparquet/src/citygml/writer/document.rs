//! `CityModel` document skeleton (root element + namespaces) and the
//! `gml:boundedBy/gml:Envelope` accumulated from written geometry.
//!
//! The root is **unprefixed** (default `xmlns` bound to the CityGML 2.0 core
//! namespace) — see `tests/fixtures/*.gml`, which all use `<CityModel
//! xmlns="http://www.opengis.net/citygml/2.0" ...>` rather than a `core:`
//! prefix. A default `xmlns` does not bind a `core:` prefix, so element names
//! elsewhere in the writer must stay unprefixed for the core namespace too.

use std::io::Write;

use cityparquet_schema::CityParquetError;
use quick_xml::Writer;
use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};

use crate::Result;

fn io_err(e: std::io::Error) -> CityParquetError {
    CityParquetError::Io(e.to_string())
}

/// Accumulates the min/max corner of every coordinate written, for the
/// document's `gml:boundedBy/gml:Envelope`. `any` stays `false` until the
/// first [`Bounds::add`], distinguishing "no geometry written" (no envelope)
/// from a degenerate envelope at the origin.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    pub min: [f64; 3],
    pub max: [f64; 3],
    pub any: bool,
}

impl Bounds {
    pub fn new() -> Self {
        Bounds {
            min: [0.0, 0.0, 0.0],
            max: [0.0, 0.0, 0.0],
            any: false,
        }
    }

    /// Fold one coordinate into the running min/max.
    pub fn add(&mut self, c: [f64; 3]) {
        if !self.any {
            self.min = c;
            self.max = c;
            self.any = true;
            return;
        }
        for ((min, max), v) in self.min.iter_mut().zip(self.max.iter_mut()).zip(c) {
            if v < *min {
                *min = v;
            }
            if v > *max {
                *max = v;
            }
        }
    }
}

impl Default for Bounds {
    fn default() -> Self {
        Self::new()
    }
}

/// `X Y Z` for one corner, matching `geometry::pos_list`'s formatting
/// (Rust `{}` floats, space-joined).
fn corner(c: [f64; 3]) -> String {
    format!("{} {} {}", c[0], c[1], c[2])
}

/// Open the `<CityModel>` root element (four namespaces: unprefixed core,
/// `bldg`, `gml`, `xlink`) and, when `bounds.any`, its
/// `gml:boundedBy/gml:Envelope`.
pub fn write_city_model_open<W: Write>(
    w: &mut Writer<W>,
    srs_name: Option<&str>,
    bounds: &Bounds,
) -> Result<()> {
    let mut root = BytesStart::new("CityModel");
    root.push_attribute(("xmlns", "http://www.opengis.net/citygml/2.0"));
    root.push_attribute(("xmlns:bldg", "http://www.opengis.net/citygml/building/2.0"));
    root.push_attribute(("xmlns:gml", "http://www.opengis.net/gml"));
    root.push_attribute(("xmlns:xlink", "http://www.w3.org/1999/xlink"));
    w.write_event(Event::Start(root)).map_err(io_err)?;

    if bounds.any {
        w.write_event(Event::Start(BytesStart::new("gml:boundedBy")))
            .map_err(io_err)?;

        let mut envelope = BytesStart::new("gml:Envelope");
        if let Some(srs) = srs_name {
            envelope.push_attribute(("srsName", srs));
        }
        envelope.push_attribute(("srsDimension", "3"));
        w.write_event(Event::Start(envelope)).map_err(io_err)?;

        w.write_event(Event::Start(BytesStart::new("gml:lowerCorner")))
            .map_err(io_err)?;
        w.write_event(Event::Text(BytesText::new(&corner(bounds.min))))
            .map_err(io_err)?;
        w.write_event(Event::End(BytesEnd::new("gml:lowerCorner")))
            .map_err(io_err)?;

        w.write_event(Event::Start(BytesStart::new("gml:upperCorner")))
            .map_err(io_err)?;
        w.write_event(Event::Text(BytesText::new(&corner(bounds.max))))
            .map_err(io_err)?;
        w.write_event(Event::End(BytesEnd::new("gml:upperCorner")))
            .map_err(io_err)?;

        w.write_event(Event::End(BytesEnd::new("gml:Envelope")))
            .map_err(io_err)?;
        w.write_event(Event::End(BytesEnd::new("gml:boundedBy")))
            .map_err(io_err)?;
    }

    Ok(())
}

/// Close the `</CityModel>` root element.
pub fn write_city_model_close<W: Write>(w: &mut Writer<W>) -> Result<()> {
    w.write_event(Event::End(BytesEnd::new("CityModel")))
        .map_err(io_err)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use quick_xml::Writer;

    fn emit<F: Fn(&mut Writer<Vec<u8>>) -> crate::Result<()>>(f: F) -> String {
        let mut w = Writer::new(Vec::new());
        f(&mut w).unwrap();
        String::from_utf8(w.into_inner()).unwrap()
    }

    #[test]
    fn envelope_from_bounds_exact_corners_with_srs() {
        let mut b = Bounds::new();
        b.add([1.0, 2.0, 3.0]);
        b.add([4.0, 5.0, 6.0]);
        let xml = emit(|w| write_city_model_open(w, Some("urn:ogc:def:crs:EPSG::28992"), &b));
        assert!(xml.contains("<CityModel xmlns=\"http://www.opengis.net/citygml/2.0\""));
        assert!(xml.contains("xmlns:bldg=\"http://www.opengis.net/citygml/building/2.0\""));
        assert!(xml.contains(
            "<gml:Envelope srsName=\"urn:ogc:def:crs:EPSG::28992\" srsDimension=\"3\">"
        ));
        assert!(xml.contains("<gml:lowerCorner>1 2 3</gml:lowerCorner>"));
        assert!(xml.contains("<gml:upperCorner>4 5 6</gml:upperCorner>"));
    }

    #[test]
    fn no_geometry_means_no_envelope() {
        let b = Bounds::new(); // nothing added
        let xml = emit(|w| write_city_model_open(w, None, &b));
        assert!(!xml.contains("gml:boundedBy"));
        assert!(xml.contains("<CityModel"));
    }

    #[test]
    fn close_emits_end_tag() {
        let xml = emit(write_city_model_close);
        assert_eq!(xml, "</CityModel>");
    }

    #[test]
    fn bounds_add_expands_min_and_max_over_multiple_points() {
        let mut b = Bounds::new();
        b.add([5.0, 5.0, 5.0]);
        b.add([1.0, 9.0, 3.0]);
        b.add([9.0, 1.0, 7.0]);
        assert_eq!(b.min, [1.0, 1.0, 3.0]);
        assert_eq!(b.max, [9.0, 9.0, 7.0]);
    }
}
