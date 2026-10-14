//! Synthesise a `cjseq::CityJSON` header for a CityGML 2.0 document.
//!
//! Scans only the document preamble (the `gml:Envelope`, which precedes the
//! first `cityObjectMember`) for the dataset CRS and extent, then picks the one
//! global quantisation transform every feature is encoded against:
//! `translate = envelope lower corner` (or `[0, 0, 0]` when there is no
//! envelope — decided here, never per-feature), and a **per-axis `scale` taken
//! from the resolved CRS's own declared units**
//! ([`cityparquet_schema::crs::axis_scale`]).
//!
//! The scale cannot be a single constant. EPSG:6697 — what every PLATEAU
//! (Japan) export declares — is latitude/longitude in **degrees** over a
//! height in metres, so a uniform millimetre step would quantise 0.001 degree,
//! 90 to 111 m, and destroy the dataset while every object and attribute still
//! round-tripped perfectly. A document whose CRS does not resolve keeps the
//! millimetre step: there is nothing to derive a unit from, and the spec's
//! "an unresolvable CRS is declared, not fatal" rule writes such a package
//! with an explicit null CRS.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use cityparquet_schema::crs::{MM, axis_scale, resolve_to_projjson};
use cityparquet_schema::{CityParquetError, Result};
use cjseq::{CityJSON, Metadata, Transform};

use super::crs::{self, CrsResolution};
use super::reader::error;

/// Build the CityJSON header (transform + metadata) for the document at `path`.
pub fn parse_header(path: &Path) -> Result<CityJSON> {
    let file = File::open(path)
        .map_err(|e| CityParquetError::io_source(format!("cannot open {}", path.display()), e))?;
    parse_header_from(BufReader::new(file))
}

/// [`parse_header`] over any `BufRead`. It consumes the preamble (and, for a
/// document that declares its CRS only on its objects, a bounded run of them),
/// so a streaming caller that still needs those bytes records them and replays
/// them ahead of the rest.
pub fn parse_header_from<R: BufRead>(input: R) -> Result<CityJSON> {
    let citygml2::Preamble { srs_name, envelope } =
        citygml2::scan_preamble(input).map_err(error)?;

    let translate = match &envelope {
        Some(e) => [e[0], e[1], e[2]],
        None => [0.0, 0.0, 0.0],
    };

    // Resolve the CRS before the transform: the quantisation step depends on
    // the units it declares.
    let resolution = match &srs_name {
        Some(name) => crs::resolve(name)?,
        None => CrsResolution::Unresolved,
    };
    // The lookup key the vendored PROJJSON table is indexed by. A CRS this
    // reader understands but the table does not know is NOT fatal (spec: "an
    // unresolvable CRS is declared, not fatal"); it keeps the linear step and
    // the scan writes the package with an explicit null CRS.
    let key = match &resolution {
        CrsResolution::Epsg(code) => Some(code.clone()),
        CrsResolution::Crs84 => Some("OGC:CRS84".to_string()),
        CrsResolution::Unresolved => None,
    };
    let scale = match key.as_deref().and_then(|k| resolve_to_projjson(k).ok()) {
        Some(projjson) => axis_scale(&projjson)?,
        None => [MM; 3],
    };

    let mut header = CityJSON::new(); // version "2.0", identity transform
    header.transform = Transform { scale, translate };

    let mut metadata = Metadata {
        geographical_extent: None,
        identifier: None,
        point_of_contact: None,
        reference_date: None,
        reference_system: None,
        title: None,
        other: serde_json::Map::new(),
    };
    let mut has_metadata = false;
    if let Some(e) = envelope {
        metadata.geographical_extent = Some(e);
        has_metadata = true;
    }
    match &resolution {
        CrsResolution::Epsg(code) => {
            metadata.reference_system = Some(crs::reference_system(code));
            has_metadata = true;
        }
        CrsResolution::Crs84 => {
            metadata.reference_system = Some(crs::reference_system_crs84());
            has_metadata = true;
        }
        CrsResolution::Unresolved => {}
    }
    header.metadata = has_metadata.then_some(metadata);
    Ok(header)
}
