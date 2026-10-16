//! The CityJSON header of a CityGML document.
//!
//! Only the preamble decides the transform every feature is quantised
//! against: its `translate` is the envelope's lower corner (the origin when
//! there is no envelope), and its `scale` the step of each axis of the CRS
//! the document names ([`crate::crs`]), a millimetre on each when it names
//! none that resolves.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use cjseq::{CityJSON, Metadata, Transform};

use crate::crs::{self, MM};
use crate::reader::{Document, Preamble, scan_preamble};
use crate::{Error, Result};

/// The header of the document `input` starts: transform and metadata, from
/// its preamble. It consumes the preamble (and, for a document that declares
/// its CRS only on its objects, a bounded run of them), so a streaming caller
/// that still needs those bytes records them and replays them.
pub fn header_from<R: BufRead>(input: R) -> Result<CityJSON> {
    let Preamble { srs_name, envelope } = scan_preamble(input)?;
    let crs = srs_name.as_deref().and_then(crs::resolve);
    let scale = match &crs {
        Some(crs) => crs.scale()?,
        None => [MM; 3],
    };
    let translate = match &envelope {
        Some(e) => [e[0], e[1], e[2]],
        None => [0.0; 3],
    };

    let mut header = CityJSON::new();
    header.transform = Transform { scale, translate };
    if envelope.is_some() || crs.is_some() {
        header.metadata = Some(Metadata {
            geographical_extent: envelope,
            identifier: None,
            point_of_contact: None,
            reference_date: None,
            reference_system: crs.as_ref().map(crs::Crs::reference_system),
            title: None,
            other: serde_json::Map::new(),
        });
    }
    Ok(header)
}

/// The header of the document at `path`, with what `document`, read from
/// it beforehand, adds: the relative geometries its `ImplicitGeometry`s
/// place as `geometry-templates`, with the appearance they index, and the
/// Extensions its ADEs are declared as.
pub fn header_with(path: &Path, document: &Document) -> Result<CityJSON> {
    let file =
        File::open(path).map_err(|e| Error::io(format!("cannot open {}", path.display()), e))?;
    let mut header = header_from(BufReader::new(file))?;
    if let Some((templates, appearance)) = super::geometry_templates(document.templates())? {
        header.geometry_templates = Some(templates);
        header.appearance = appearance;
    }
    let extensions = super::extensions(document.ade());
    if !extensions.is_empty() {
        header.extensions = Some(extensions.into_iter().collect());
    }
    Ok(header)
}
