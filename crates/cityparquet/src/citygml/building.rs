//! Parse one `bldg:Building` subtree and turn it into a `cjseq::CityJSONFeature`.
//!
//! M1 scope: LoD1/LoD2 `gml:Solid` geometry only (no semantics, no attributes,
//! no BuildingParts — those arrive in later milestones). The `CityObject` is
//! assembled as JSON and deserialised, because `cjseq::CityObject` has a private
//! field and no public constructor.

use std::io::BufRead;

use cityparquet_schema::{CityParquetError, Result};
use cjseq::{CityJSONFeature, CityObject};
use quick_xml::events::Event;
use quick_xml::reader::NsReader;
use serde_json::{Value, json};

use super::geometry::{self, Polygon, Solid};
use super::vertices::VertexBuilder;
use super::xml::{NS_BLDG, NS_GML, ns_is, skip_element, xml_err};

/// A parsed geometry with its LoD (as a CityJSON lod string, e.g. `"2"`).
struct RawGeometry {
    lod: String,
    solid: Solid,
}

/// A buffered building: its `gml:id` (if any) and its LoD geometries.
pub struct RawBuilding {
    pub id: Option<String>,
    geometries: Vec<RawGeometry>,
}

/// Read a `bldg:Building` subtree (positioned after its `Start`).
pub fn read_building<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    id: Option<String>,
) -> Result<RawBuilding> {
    let mut geometries = Vec::new();
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        let bldg = ns_is(&rr, NS_BLDG);
        match ev {
            Event::Start(e) => {
                let local = e.local_name();
                if let (true, Some(lod)) = (bldg, lod_solid(local.as_ref())) {
                    let end = local.as_ref().to_vec();
                    let solid = read_lod_solid(reader, buf, end)?;
                    geometries.push(RawGeometry { lod, solid });
                } else {
                    skip_element(reader, buf)?;
                }
            }
            Event::End(e) if e.local_name().as_ref() == b"Building" => break,
            Event::Eof => {
                return Err(CityParquetError::Schema(
                    "unexpected end of document inside <bldg:Building>".to_string(),
                ));
            }
            _ => {}
        }
    }
    Ok(RawBuilding { id, geometries })
}

/// `bldg:lodNSolid` -> the CityJSON lod string `"N"`, else `None`.
fn lod_solid(local: &[u8]) -> Option<String> {
    let s = std::str::from_utf8(local).ok()?;
    let digits = s.strip_prefix("lod")?.strip_suffix("Solid")?;
    if !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) {
        Some(digits.to_string())
    } else {
        None
    }
}

/// Inside a `bldg:lodNSolid`: read the wrapped `gml:Solid`.
fn read_lod_solid<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    end: Vec<u8>,
) -> Result<Solid> {
    let mut solid: Option<Solid> = None;
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        let gml = ns_is(&rr, NS_GML);
        match ev {
            Event::Start(e) => {
                let local = e.local_name();
                if gml && local.as_ref() == b"Solid" {
                    solid = Some(geometry::read_solid(reader, buf)?);
                } else {
                    skip_element(reader, buf)?;
                }
            }
            Event::End(e) if e.local_name().as_ref() == end.as_slice() => break,
            Event::Eof => {
                return Err(CityParquetError::Schema(
                    "unexpected end of document inside <bldg:lodNSolid>".to_string(),
                ));
            }
            _ => {}
        }
    }
    solid.ok_or_else(|| CityParquetError::Schema("bldg:lodNSolid without gml:Solid".to_string()))
}

impl RawBuilding {
    /// Build a `CityJSONFeature` with its own local integer vertex pool,
    /// quantised against `scale`/`translate`. `index` supplies a stable id when
    /// the building has no `gml:id`.
    pub fn into_feature(
        self,
        scale: &[f64; 3],
        translate: &[f64; 3],
        index: usize,
    ) -> Result<CityJSONFeature> {
        let mut vb = VertexBuilder::new(scale, translate);
        let mut geoms_json: Vec<Value> = Vec::new();
        for g in &self.geometries {
            let boundaries = solid_boundaries(&g.solid, &mut vb)?;
            geoms_json.push(json!({ "type": "Solid", "lod": g.lod, "boundaries": boundaries }));
        }

        let id = self.id.unwrap_or_else(|| format!("Building_{index}"));
        let co_json = json!({ "type": "Building", "geometry": geoms_json });
        let co: CityObject = serde_json::from_value(co_json).map_err(|e| {
            CityParquetError::Schema(format!("failed to build CityObject from CityGML: {e}"))
        })?;

        let mut feature = CityJSONFeature::new();
        feature.id = id.clone();
        feature.add_co(id, co);
        feature.vertices = vb.into_vertices();
        Ok(feature)
    }
}

/// Solid boundaries: `[ shell ][ surface ][ ring ][ vertex_index ]`.
fn solid_boundaries(solid: &Solid, vb: &mut VertexBuilder) -> Result<Value> {
    let mut shells = Vec::with_capacity(solid.shells.len());
    for shell in &solid.shells {
        let mut surfaces = Vec::with_capacity(shell.len());
        for poly in shell {
            surfaces.push(polygon_rings(poly, vb)?);
        }
        shells.push(Value::Array(surfaces));
    }
    Ok(Value::Array(shells))
}

/// A surface: exterior ring first, then interior (hole) rings.
fn polygon_rings(poly: &Polygon, vb: &mut VertexBuilder) -> Result<Value> {
    let mut rings = Vec::with_capacity(1 + poly.interiors.len());
    rings.push(ring_indices(&poly.exterior, vb)?);
    for hole in &poly.interiors {
        rings.push(ring_indices(hole, vb)?);
    }
    Ok(Value::Array(rings))
}

fn ring_indices(ring: &[[f64; 3]], vb: &mut VertexBuilder) -> Result<Value> {
    let mut idxs = Vec::with_capacity(ring.len());
    for &coord in ring {
        idxs.push(json!(vb.push(coord)?));
    }
    Ok(Value::Array(idxs))
}
