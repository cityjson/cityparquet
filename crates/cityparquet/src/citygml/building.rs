//! Parse one `bldg:Building` subtree and turn it into a `cjseq::CityJSONFeature`.
//!
//! Two passes over the buffered building:
//! 1. Collect the solid geometry as an unresolved [`SolidGeom`] (its surface
//!    members may `xlink` forward), every inline `gml:Polygon` by `gml:id`
//!    (from `boundedBy` semantic surfaces and any standalone
//!    `lodNMultiSurface`), the semantic-surface list, and each boundary
//!    polygon's semantic index.
//! 2. Resolve the solid's xlinks against the polygon registry and emit exactly
//!    one CityJSON geometry (`Solid`/`CompositeSolid`) with its `semantics`.
//!
//! Scope so far: LoD1/LoD2 solids with semantic surfaces referenced by xlink,
//! plus building-level typed `bldg:` and `gen:` generic attributes (repeats
//! accumulate into arrays). BuildingParts and the boundedBy-only /
//! `lodNMultiSurface`-primary geometry cases (no `lodNSolid`) arrive in M4.

use std::collections::HashMap;
use std::io::BufRead;

use cityparquet_schema::{CityParquetError, Result};
use cjseq::{CityJSONFeature, CityObject};
use quick_xml::events::Event;
use quick_xml::reader::NsReader;
use serde_json::{Value, json};

use super::attributes;
use super::geometry::{self, Polygon, RawSolid, RefTarget, SolidGeom, SurfaceRef};
use super::vertices::VertexBuilder;
use super::xml::{NS_BLDG, NS_GEN, NS_GML, get_attr_local, ns_is, skip_element, xml_err};

/// A buffered building: everything pass 1 collects before resolution.
pub struct RawBuilding {
    pub id: Option<String>,
    /// One solid geometry per distinct LoD (`bldg:lodNSolid`); each LoD maps to
    /// its own CityParquet geometry column, so all are emitted.
    solids: Vec<(String, SolidGeom)>,
    /// Every inline polygon by `gml:id`, resolvable by the solid's xlinks.
    polygons: HashMap<String, Polygon>,
    /// Semantic surface kinds (`"WallSurface"`, ...), in document order.
    surfaces: Vec<String>,
    /// `gml:id` of a boundary polygon -> its index into `surfaces`.
    semantic_of_polygon: HashMap<String, usize>,
    /// Building-level attributes (typed `bldg:` + `gen:` generic), by name.
    attributes: serde_json::Map<String, Value>,
}

/// Read a `bldg:Building` subtree (positioned after its `Start`).
pub fn read_building<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    id: Option<String>,
) -> Result<RawBuilding> {
    let mut b = RawBuilding {
        id,
        solids: Vec::new(),
        polygons: HashMap::new(),
        surfaces: Vec::new(),
        semantic_of_polygon: HashMap::new(),
        attributes: serde_json::Map::new(),
    };

    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        let bldg = ns_is(&rr, NS_BLDG);
        match ev {
            Event::Start(e) => {
                let local = e.local_name();
                let name = local.as_ref().to_vec();
                if bldg && let Some(lod) = lod_suffix(&name, b"Solid") {
                    let geom = read_lod_solid(reader, buf, name)?;
                    // Keep one solid per distinct LoD (the encoder keeps only the
                    // first geometry per object+LoD anyway).
                    if !b.solids.iter().any(|(l, _)| *l == lod) {
                        b.solids.push((lod, geom));
                    }
                } else if bldg && lod_suffix(&name, b"MultiSurface").is_some() {
                    // A standalone lodNMultiSurface under the Building: harvest
                    // its polygons (they may be xlink targets, e.g. an internal
                    // ceiling). Emitting it as primary geometry is M4.
                    for (id, poly) in geometry::collect_polygons(reader, buf)? {
                        if let Some(id) = id {
                            b.polygons.insert(id, poly);
                        }
                    }
                } else if bldg && name == b"boundedBy" {
                    read_bounded_by(reader, buf, &mut b)?;
                } else if bldg && let Some(ty) = attributes::typed_building_attr(&name) {
                    if let Some((k, v)) = attributes::read_typed_attribute(reader, buf, &name, ty)?
                    {
                        attributes::accumulate(&mut b.attributes, k, v);
                    }
                } else if let (true, Some(ty)) =
                    (ns_is(&rr, NS_GEN), attributes::generic_attr(&name))
                {
                    let attr_name = get_attr_local(&e, b"name");
                    if let Some((k, v)) =
                        attributes::read_generic_attribute(reader, buf, attr_name, ty)?
                    {
                        attributes::accumulate(&mut b.attributes, k, v);
                    }
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
    Ok(b)
}

/// `bldg:lodN<suffix>` -> the CityJSON lod string `"N"`, else `None`.
fn lod_suffix(local: &[u8], suffix: &[u8]) -> Option<String> {
    let s = std::str::from_utf8(local).ok()?;
    let suffix = std::str::from_utf8(suffix).ok()?;
    let digits = s.strip_prefix("lod")?.strip_suffix(suffix)?;
    if !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) {
        Some(digits.to_string())
    } else {
        None
    }
}

/// Inside a `bldg:lodNSolid`: read the wrapped `gml:Solid` or
/// `gml:CompositeSolid`.
fn read_lod_solid<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    end: Vec<u8>,
) -> Result<SolidGeom> {
    let mut geom: Option<SolidGeom> = None;
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        let gml = ns_is(&rr, NS_GML);
        match ev {
            Event::Start(e) => {
                let local = e.local_name();
                match (gml, local.as_ref()) {
                    (true, b"Solid") => {
                        geom = Some(SolidGeom::Solid(geometry::read_solid(reader, buf)?))
                    }
                    (true, b"CompositeSolid") => {
                        geom = Some(SolidGeom::Composite(geometry::read_composite_solid(
                            reader, buf,
                        )?))
                    }
                    _ => skip_element(reader, buf)?,
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
    geom.ok_or_else(|| {
        CityParquetError::Schema(
            "bldg:lodNSolid without gml:Solid or gml:CompositeSolid".to_string(),
        )
    })
}

/// A `bldg:boundedBy`: its single semantic surface child (`WallSurface`, ...)
/// contributes one entry to `surfaces`; every polygon inside it is registered
/// and mapped to that entry.
fn read_bounded_by<R: BufRead>(
    reader: &mut NsReader<R>,
    buf: &mut Vec<u8>,
    b: &mut RawBuilding,
) -> Result<()> {
    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(buf).map_err(xml_err)?;
        let bldg = ns_is(&rr, NS_BLDG);
        match ev {
            Event::Start(e) => {
                let local = e.local_name();
                if bldg {
                    // The semantic surface: its element name is the CityJSON
                    // surface type.
                    let kind = String::from_utf8_lossy(local.as_ref()).into_owned();
                    let idx = b.surfaces.len();
                    b.surfaces.push(kind);
                    for (id, poly) in geometry::collect_polygons(reader, buf)? {
                        if let Some(id) = id {
                            b.semantic_of_polygon.insert(id.clone(), idx);
                            b.polygons.insert(id, poly);
                        }
                    }
                } else {
                    skip_element(reader, buf)?;
                }
            }
            Event::End(e) if e.local_name().as_ref() == b"boundedBy" => break,
            Event::Eof => {
                return Err(CityParquetError::Schema(
                    "unexpected end of document inside <bldg:boundedBy>".to_string(),
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

impl RawBuilding {
    /// Resolve xlinks and emit a `CityJSONFeature` with its own local vertex
    /// pool, quantised against `scale`/`translate`. `index` names the building
    /// when it has no `gml:id`.
    pub fn into_feature(
        self,
        scale: &[f64; 3],
        translate: &[f64; 3],
        index: usize,
    ) -> Result<CityJSONFeature> {
        let mut vb = VertexBuilder::new(scale, translate);
        let mut geoms_json: Vec<Value> = Vec::new();
        for (lod, geom) in &self.solids {
            geoms_json.push(self.build_solid_geometry(geom, lod, &mut vb)?);
        }

        let id = self
            .id
            .clone()
            .unwrap_or_else(|| format!("Building_{index}"));
        let mut co_obj = serde_json::Map::new();
        co_obj.insert("type".to_string(), json!("Building"));
        co_obj.insert("geometry".to_string(), Value::Array(geoms_json));
        if !self.attributes.is_empty() {
            co_obj.insert("attributes".to_string(), Value::Object(self.attributes));
        }
        let co: CityObject = serde_json::from_value(Value::Object(co_obj)).map_err(|e| {
            CityParquetError::Schema(format!("failed to build CityObject from CityGML: {e}"))
        })?;

        let mut feature = CityJSONFeature::new();
        feature.id = id.clone();
        feature.add_co(id, co);
        feature.vertices = vb.into_vertices();
        Ok(feature)
    }

    /// Build one CityJSON `Solid`/`CompositeSolid` geometry object, emitting
    /// boundaries and matching `semantics.values` in a single walk (so their
    /// per-face order is aligned by construction).
    fn build_solid_geometry(
        &self,
        geom: &SolidGeom,
        lod: &str,
        vb: &mut VertexBuilder,
    ) -> Result<Value> {
        let (gtype, boundaries, values) = match geom {
            SolidGeom::Solid(solid) => {
                let (b, v) = self.build_solid(solid, vb)?;
                ("Solid", Value::Array(b), Value::Array(v))
            }
            SolidGeom::Composite(solids) => {
                let mut sb = Vec::with_capacity(solids.len());
                let mut sv = Vec::with_capacity(solids.len());
                for solid in solids {
                    let (b, v) = self.build_solid(solid, vb)?;
                    sb.push(Value::Array(b));
                    sv.push(Value::Array(v));
                }
                ("CompositeSolid", Value::Array(sb), Value::Array(sv))
            }
        };

        let mut g = json!({ "type": gtype, "lod": lod, "boundaries": boundaries });
        if !self.surfaces.is_empty() {
            let surfaces: Vec<Value> = self.surfaces.iter().map(|k| json!({ "type": k })).collect();
            g["semantics"] = json!({ "surfaces": surfaces, "values": values });
        }
        Ok(g)
    }

    /// A `RawSolid` -> (`[shell][surface][ring][idx]`, `[shell][face]` values).
    fn build_solid(
        &self,
        solid: &RawSolid,
        vb: &mut VertexBuilder,
    ) -> Result<(Vec<Value>, Vec<Value>)> {
        let mut shells_b = Vec::with_capacity(solid.shells.len());
        let mut shells_v = Vec::with_capacity(solid.shells.len());
        for shell in &solid.shells {
            let mut faces_b = Vec::with_capacity(shell.len());
            let mut faces_v = Vec::with_capacity(shell.len());
            for sref in shell {
                let poly = self.resolve(sref)?;
                faces_b.push(surface_rings(poly, sref.reverse, vb)?);
                faces_v.push(self.semantic_value(sref));
            }
            shells_b.push(Value::Array(faces_b));
            shells_v.push(Value::Array(faces_v));
        }
        Ok((shells_b, shells_v))
    }

    /// The polygon a surface reference points at (xlink resolved against the
    /// registry, or inline).
    fn resolve<'a>(&'a self, sref: &'a SurfaceRef) -> Result<&'a Polygon> {
        match &sref.target {
            RefTarget::Inline(poly) => Ok(poly),
            RefTarget::Xlink(id) => self.polygons.get(id).ok_or_else(|| {
                CityParquetError::Schema(format!(
                    "CityGML solid references #{id}, which is not defined in this building \
                     (cross-building/shared geometry is out of scope)"
                ))
            }),
        }
    }

    /// The `semantics.values` leaf for a surface reference: the semantic index
    /// of the referenced boundary polygon, or `null` (inline / no boundary
    /// surface, e.g. an internal ceiling).
    fn semantic_value(&self, sref: &SurfaceRef) -> Value {
        match &sref.target {
            RefTarget::Xlink(id) => match self.semantic_of_polygon.get(id) {
                Some(&i) => json!(i),
                None => Value::Null,
            },
            RefTarget::Inline(_) => Value::Null,
        }
    }
}

/// A surface's rings `[exterior, hole...]`, each a list of vertex indices; when
/// `reverse`, every ring is wound backwards so the outward normal flips.
fn surface_rings(poly: &Polygon, reverse: bool, vb: &mut VertexBuilder) -> Result<Value> {
    let mut rings = Vec::with_capacity(1 + poly.interiors.len());
    rings.push(ring_indices(&poly.exterior, reverse, vb)?);
    for hole in &poly.interiors {
        rings.push(ring_indices(hole, reverse, vb)?);
    }
    Ok(Value::Array(rings))
}

fn ring_indices(ring: &[[f64; 3]], reverse: bool, vb: &mut VertexBuilder) -> Result<Value> {
    let mut idxs = Vec::with_capacity(ring.len());
    if reverse {
        for &coord in ring.iter().rev() {
            idxs.push(json!(vb.push(coord)?));
        }
    } else {
        for &coord in ring {
            idxs.push(json!(vb.push(coord)?));
        }
    }
    Ok(Value::Array(idxs))
}
