//! Face -> `gml:Polygon` serialisation.
//!
//! A decoded face is rings of coord indices into a shared coord pool
//! (`wkb_read::DecodedGeometry`'s shape). The WKB reader strips each ring's
//! closing duplicate vertex on decode (mirroring the CityGML reader, which
//! drops it too — see `citygml::geometry::read_linear_ring`), so rings here
//! are *open* (last != first). GML's `gml:LinearRing` requires a *closed*
//! ring, so [`pos_list`] re-appends the first coordinate.

use std::io::Write;

use cityparquet_schema::CityParquetError;
use quick_xml::Writer;
use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};

use crate::Result;

fn io_err(e: std::io::Error) -> CityParquetError {
    CityParquetError::Io(e.to_string())
}

/// One ring's `posList` text: `X Y Z` per vertex, world coords, **re-closed**
/// (the WKB reader strips the closing vertex, GML requires it back).
pub fn pos_list(coords: &[[f64; 3]], ring: &[usize]) -> String {
    let mut out = String::new();
    let mut push = |i: usize| {
        let c = coords[i];
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&format!("{} {} {}", c[0], c[1], c[2]));
    };
    for &i in ring {
        push(i);
    }
    if let Some(&first) = ring.first() {
        push(first); // re-close
    }
    out
}

/// Write a `gml:LinearRing` wrapping this ring's (re-closed) `posList`.
fn write_linear_ring<W: Write>(
    w: &mut Writer<W>,
    coords: &[[f64; 3]],
    ring: &[usize],
) -> Result<()> {
    w.write_event(Event::Start(BytesStart::new("gml:LinearRing")))
        .map_err(io_err)?;

    let mut pos_list_start = BytesStart::new("gml:posList");
    pos_list_start.push_attribute(("srsDimension", "3"));
    w.write_event(Event::Start(pos_list_start))
        .map_err(io_err)?;
    w.write_event(Event::Text(BytesText::new(&pos_list(coords, ring))))
        .map_err(io_err)?;
    w.write_event(Event::End(BytesEnd::new("gml:posList")))
        .map_err(io_err)?;

    w.write_event(Event::End(BytesEnd::new("gml:LinearRing")))
        .map_err(io_err)?;
    Ok(())
}

/// One face (rings of coord indices) -> a `<gml:Polygon>`: ring 0 exterior,
/// ring 1.. interior (holes). A face with no rings is a caller error
/// (upstream guarantees at least one exterior ring).
pub fn write_polygon<W: Write>(
    w: &mut Writer<W>,
    coords: &[[f64; 3]],
    face: &[Vec<usize>],
) -> Result<()> {
    w.write_event(Event::Start(BytesStart::new("gml:Polygon")))
        .map_err(io_err)?;

    let (exterior, interiors) = face
        .split_first()
        .ok_or_else(|| CityParquetError::Geometry("face has no rings to write".to_string()))?;

    w.write_event(Event::Start(BytesStart::new("gml:exterior")))
        .map_err(io_err)?;
    write_linear_ring(w, coords, exterior)?;
    w.write_event(Event::End(BytesEnd::new("gml:exterior")))
        .map_err(io_err)?;

    for hole in interiors {
        w.write_event(Event::Start(BytesStart::new("gml:interior")))
            .map_err(io_err)?;
        write_linear_ring(w, coords, hole)?;
        w.write_event(Event::End(BytesEnd::new("gml:interior")))
            .map_err(io_err)?;
    }

    w.write_event(Event::End(BytesEnd::new("gml:Polygon")))
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
    fn pos_list_reclose_appends_first_coord() {
        let coords = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0]];
        // open ring (reader-decoded shape: last != first)
        let ring = vec![0usize, 1, 2];
        assert_eq!(pos_list(&coords, &ring), "0 0 0 1 0 0 1 1 0 0 0 0");
    }

    #[test]
    fn write_polygon_emits_exterior_and_interior_rings() {
        let coords = vec![
            [0.0, 0.0, 0.0],
            [4.0, 0.0, 0.0],
            [4.0, 4.0, 0.0],
            [0.0, 4.0, 0.0], // outer
            [1.0, 1.0, 0.0],
            [2.0, 1.0, 0.0],
            [2.0, 2.0, 0.0], // hole
        ];
        let face = vec![vec![0usize, 1, 2, 3], vec![4usize, 5, 6]];
        let xml = emit(|w| write_polygon(w, &coords, &face));
        assert!(xml.contains("<gml:Polygon>"));
        assert!(xml.contains("<gml:exterior><gml:LinearRing><gml:posList srsDimension=\"3\">0 0 0 4 0 0 4 4 0 0 4 0 0 0 0</gml:posList>"));
        assert!(xml.contains("<gml:interior><gml:LinearRing><gml:posList srsDimension=\"3\">1 1 0 2 1 0 2 2 0 1 1 0</gml:posList>"));
    }
}
