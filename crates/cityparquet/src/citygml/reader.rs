//! Streaming feature reader: one `cjseq::CityJSONFeature` per top-level
//! CityObject — a `bldg:Building` (with its parts) or a mapped 1st-level
//! non-building object (WaterBody, LandUse, CityFurniture, … — CG-7).
//!
//! Buffers exactly one object subtree at a time (memory is O(largest single
//! object)); the CityModel wrapper and `cityObjectMember`s are walked past
//! without buffering. Non-building objects read their `lodN` solid/surface
//! geometry + generic attributes; semantic surfaces, parts, and appearance on
//! non-building objects are out of scope for this milestone.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use cityparquet_schema::{CityParquetError, Result};
use cjseq::{CityJSONFeature, Transform};
use quick_xml::events::Event;
use quick_xml::reader::NsReader;

use super::appearance::{ModelAppearance, ReadAppearance, read_appearance};
use super::building::{read_building, read_generic_object};
use super::xml::{NS_APP, NS_BLDG, gml_id, ns_is, skip_element, xml_err};

/// Map a CityGML 1st-level NON-building element local name to its CityJSON
/// CityObject type (CG-7). Matched by local name (unique across CityGML module
/// namespaces); `Building`/`BuildingPart` are handled separately. Returns
/// `None` for containers and unmapped/2nd-level elements.
fn citygml_object_type(local: &[u8]) -> Option<&'static str> {
    Some(match local {
        b"WaterBody" => "WaterBody",
        b"LandUse" => "LandUse",
        b"CityFurniture" => "CityFurniture",
        b"SolitaryVegetationObject" => "SolitaryVegetationObject",
        b"PlantCover" => "PlantCover",
        b"Bridge" => "Bridge",
        b"Tunnel" => "Tunnel",
        // ReliefFeature is deliberately NOT mapped: a CityGML ReliefFeature may
        // be raster/breakline/mass-point/TIN, and only a TIN maps cleanly to
        // CityJSON TINRelief; its `reliefComponent` geometry is nested
        // differently. Deferred to avoid misclassifying non-TIN reliefs.
        b"GenericCityObject" => "GenericCityObject",
        b"CityObjectGroup" => "CityObjectGroup",
        b"Road" => "Road",
        b"Railway" => "Railway",
        b"Square" => "TransportSquare",
        _ => return None,
    })
}

pub struct FeatureReader {
    reader: NsReader<BufReader<File>>,
    buf: Vec<u8>,
    scale: [f64; 3],
    translate: [f64; 3],
    index: usize,
    done: bool,
    /// CityModel-level appearance (`app:appearanceMember` on `<CityModel>`),
    /// collected up front in a separate pass so it can be applied to every
    /// building's faces/rings by `gml:id` regardless of whether it appears
    /// before or after the buildings, and indexed for O(building-ids) lookup
    /// (CG-3).
    model_appearance: ModelAppearance,
}

/// Pre-pass: read every CityModel-level `app:appearanceMember` (the conformant
/// CityGML 2.0 global-appearance property — a `_FeatureCollection` member of
/// `CityModel`, distinct from a feature's own `app:appearance`) into one
/// indexed [`ModelAppearance`]. Building subtrees are skipped, and a
/// feature-level `app:appearance` (which uses the other property name) is NOT
/// promoted to model scope.
fn read_model_appearance(path: &Path) -> Result<ModelAppearance> {
    let file = File::open(path)
        .map_err(|e| CityParquetError::io_source(format!("cannot reopen {}", path.display()), e))?;
    let mut reader = NsReader::from_reader(BufReader::new(file));
    reader.config_mut().expand_empty_elements = true;
    let mut buf = Vec::new();
    let mut out = ReadAppearance::default();
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(&mut buf).map_err(xml_err)?;
        match ev {
            Event::Start(e) => {
                if ns_is(&rr, NS_BLDG) && e.local_name().as_ref() == b"Building" {
                    skip_element(&mut reader, &mut buf)?;
                } else if ns_is(&rr, NS_APP) && e.local_name().as_ref() == b"appearanceMember" {
                    let app = read_appearance(&mut reader, &mut buf, b"appearanceMember")?;
                    out.materials.extend(app.materials);
                    out.textures.extend(app.textures);
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(ModelAppearance::build(out))
}

impl FeatureReader {
    /// Open `path` for streaming, quantising vertices against `transform`
    /// (which the header exposes so scan/encode dequantise consistently).
    pub fn open(path: &Path, transform: &Transform) -> Result<Self> {
        let file = File::open(path).map_err(|e| {
            CityParquetError::io_source(format!("cannot reopen {}", path.display()), e)
        })?;
        let scale = triple(&transform.scale, "scale")?;
        let translate = triple(&transform.translate, "translate")?;
        let model_appearance = read_model_appearance(path)?;
        let mut reader = NsReader::from_reader(BufReader::new(file));
        // Self-closing elements (`<gml:surfaceMember xlink:href=.../>`) must
        // arrive as Start+End so the geometry parsers see the xlink; otherwise
        // quick-xml emits Event::Empty, which the Start-matching loops drop.
        reader.config_mut().expand_empty_elements = true;
        Ok(Self {
            reader,
            buf: Vec::new(),
            scale,
            translate,
            index: 0,
            done: false,
            model_appearance,
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
                    let local = e.local_name();
                    let is_building = ns_is(&rr, NS_BLDG) && local.as_ref() == b"Building";
                    // A 1st-level non-building object (WaterBody, LandUse, …).
                    let generic_type = if is_building {
                        None
                    } else {
                        citygml_object_type(local.as_ref())
                    };
                    if is_building {
                        let id = gml_id(&e);
                        // Borrows of `e`/`rr` end here (NLL) before we re-borrow.
                        let raw = read_building(&mut self.reader, &mut self.buf, id)?;
                        self.index += 1;
                        let feature = raw.into_feature(
                            &self.scale,
                            &self.translate,
                            self.index,
                            &self.model_appearance,
                        )?;
                        return Ok(Some(feature));
                    } else if let Some(ty) = generic_type {
                        let id = gml_id(&e);
                        let end = local.as_ref().to_vec();
                        let raw =
                            read_generic_object(&mut self.reader, &mut self.buf, ty, id, &end)?;
                        self.index += 1;
                        let feature = raw.into_feature(
                            &self.scale,
                            &self.translate,
                            self.index,
                            &self.model_appearance,
                        )?;
                        return Ok(Some(feature));
                    }
                    // Otherwise descend. Containers (CityModel, cityObjectMember)
                    // hold the objects we want; a `BuildingPart` is handled
                    // within its parent Building. Nothing to do — next read
                    // descends.
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
