//! Native CityGML 2.0 output writer (CityParquet package -> .gml).
//!
//! W-M1: CityModel + envelope + srsName, and bldg:Building with LoD gml:Solid.
//! Standalone — reuses wkb_read/reader/export shell helpers, no cjseq document.

pub mod appearance;
pub mod attributes;
pub mod building;
pub mod document;
pub mod geometry;
pub mod semantics;

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use cityparquet_schema::{CityParquetError, Lod};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use quick_xml::Writer;
use quick_xml::events::{BytesDecl, BytesEnd, BytesStart, Event};

use self::building::{BuildingSolids, BuildingTree, render_abstract_object};
use self::document::{Bounds, write_city_model_close, write_city_model_open};
use crate::Result;
use crate::citygml::crs::srs_name_for;
use crate::decode::decode_batch;
use crate::export::{
    appearance_columns, first_schema_mismatch, read_lod_keyed_appearance, table_display_name,
};
use crate::reader::{CityParquetReaderBuilder, CityParquetRecordBatchReader};
use crate::sidecar::{read_materials, read_textures};
use crate::stac::properties::PackageTables;
use crate::wkb_read::{DecodedGeometry, DecodedKind};

/// Options for one CityParquet package -> CityGML 2.0 `.gml` conversion.
pub struct WriteOptions {
    pub package_dir: PathBuf,
    /// The `.gml` file to write (truncated if it exists, matching `export`).
    pub output: PathBuf,
}

/// Per-conversion counts, mirroring the export report's drop-counter style.
/// Populated by [`write_package`] and the sub-writers.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct WriteReport {
    /// `bldg:Building` elements emitted.
    pub buildings_written: usize,
    /// Rows skipped because `object_type` is not `Building` (W-M1 scope).
    pub non_building_skipped: usize,
    /// Building rows with no emittable Solid in any major LoD — skipped whole.
    pub buildings_without_solid_skipped: usize,
    /// `gml:CompositeSolid` geometries emitted.
    pub composite_solids_written: usize,
    /// MultiSolid geometry columns skipped: CityGML 2.0 `Building` has no
    /// `lodNMultiSolid` slot and `gml:MultiSolid` is not a `gml:_Solid`.
    pub multi_solids_skipped: usize,
    /// LoD columns skipped because they collided on a major LoD already kept
    /// for that building, or mapped to an unrepresentable LoD (0, >4, lodless),
    /// or held a non-Solid geometry.
    pub lod_columns_skipped: usize,
    /// Attribute values emitted as `bldg:`/`gen:` elements.
    pub attributes_written: usize,
    /// Attribute values skipped as unrepresentable in CityGML 2.0 (Boolean,
    /// nested/heterogeneous `Json`, string lists with an unwritable item,
    /// empty/whitespace strings, XML-illegal strings/names, un-typed columns).
    pub attributes_skipped: usize,
    /// `bldg:boundedBy` semantic surfaces emitted (W-M3).
    pub semantic_surfaces_written: usize,
    /// Semantic surfaces dropped when a geometry falls back to geometry-only
    /// (an extension surface type, a `values`/shape mismatch, or a MultiSolid
    /// whose geometry is skipped). Counted so the loss is reported, not silent.
    pub semantic_surfaces_dropped: usize,
    /// `null`-value faces of a no-solid MultiSurface, which have no CityGML 2.0
    /// home (the reader's MultiSurface path never produces a null value).
    /// Unreachable for CityGML-sourced input.
    pub multisurface_null_faces_dropped: usize,
    /// `bldg:BuildingPart`s emitted (nested under their parent Building).
    pub building_parts_written: usize,
    /// BuildingParts skipped because they rendered empty (no geometry, no
    /// writable attribute, no non-empty sub-part).
    pub building_parts_skipped: usize,
    /// BuildingPart rows whose `parents[0]` names no Building/BuildingPart row.
    pub building_parts_orphaned: usize,
    /// A Building's `children` entry with no matching part row (explains a
    /// shrunken re-read `children` array).
    pub children_unresolved: usize,
    /// `app:X3DMaterial` elements emitted (one per used material def per theme).
    pub materials_written: usize,
    /// Geometries whose `material` map could not be resolved (dangling/out-of-
    /// range global id, values/faces shape mismatch) — appearance dropped for
    /// that geometry, geometry itself still emitted.
    pub material_geometries_dropped: usize,
    /// Geometries carrying a `material` map on a Core-profile package (no
    /// `materials.parquet`): the definitions are unavailable, so appearance is
    /// skipped.
    pub appearance_skipped_core_profile: usize,
    /// `app:ParameterizedTexture` elements emitted (one per used texture def per
    /// theme).
    pub textures_written: usize,
    /// Geometries whose `texture` map could not be resolved (dangling id, shape
    /// mismatch) — texture appearance dropped for that geometry.
    pub texture_geometries_dropped: usize,
}

fn io_err(e: std::io::Error) -> CityParquetError {
    CityParquetError::Io(e.to_string())
}

/// How one decoded geometry column of a Building is routed by the driver.
enum GeomRoute {
    /// Emit as a `bldg:lodNSolid` — a Solid (`PolyhedralSurface`) or a
    /// CompositeSolid (`GeometryCollection`). Carries the geometry onward.
    Emit(Lod, DecodedGeometry, Option<serde_json::Value>),
    /// A MultiSolid: CityGML 2.0 `Building` has no `lodNMultiSolid` slot and
    /// `gml:MultiSolid` is not a `gml:_Solid`, so it is skipped-with-counter.
    MultiSolid,
    /// A lodless geometry cannot be a `lod<n>Solid`; skipped-with-counter.
    Lodless,
}

/// Classify one geometry column. A `GeometryCollection` (CompositeSolid or
/// MultiSolid) is distinguished by its `geometry_properties.type`: only
/// `MultiSolid` is skipped; a CompositeSolid (or any other lodded shape) is
/// emitted and `write_building` decides representability.
fn route_geometry(
    lod: Option<Lod>,
    decoded: DecodedGeometry,
    props: Option<serde_json::Value>,
) -> GeomRoute {
    match (&decoded.kind, lod) {
        (DecodedKind::GeometryCollection(_), Some(lod)) => {
            let is_multi = props
                .as_ref()
                .and_then(|p| p.get("type"))
                .and_then(|t| t.as_str())
                == Some("MultiSolid");
            if is_multi {
                GeomRoute::MultiSolid
            } else {
                GeomRoute::Emit(lod, decoded, props)
            }
        }
        (_, None) => GeomRoute::Lodless,
        (_, Some(lod)) => GeomRoute::Emit(lod, decoded, props),
    }
}

/// Serialise a CityParquet package directory into a CityGML 2.0 document.
///
/// Standalone: reads the package via the low-level `reader`/`decode`
/// primitives (never the CityJSON `export` reconstruction), decodes each row's
/// WKB into world-coordinate geometry, and emits `bldg:Building` +
/// `bldg:lod<major>Solid` directly. The first table's footer metadata + Arrow
/// schema are authoritative; later tables must match (same as `export`).
pub fn write_package(opts: &WriteOptions) -> Result<WriteReport> {
    // `PackageTables::open` reads the manifest's `tables`/`sidecar_files`
    // lists and already rejects an empty or duplicate-naming manifest, the
    // same as `crate::export` relies on.
    let tables = PackageTables::open(&opts.package_dir)?;

    // First table's footer metadata + rendered schema are authoritative.
    // `tables.tables` entries are already absolute paths.
    let first_path = &tables.tables[0];
    let first_builder =
        ParquetRecordBatchReaderBuilder::try_new(fs::File::open(first_path).map_err(|e| {
            CityParquetError::Io(format!("cannot open {}: {e}", first_path.display()))
        })?)
        .map_err(|e| CityParquetError::Parquet(format!("cannot open parquet reader: {e}")))?;
    let meta = first_builder.cityparquet_metadata()?;
    let schema = first_builder.cityparquet_arrow_schema()?;
    let srs_name = srs_name_for(meta.crs.as_ref())?;
    // Stored attribute column types drive attribute routing (not value shapes).
    let attr_types = attributes::attribute_types(&schema, &meta.attribute_columns);

    // Global materials table (Compatibility profile): appearance definitions the
    // per-geometry material maps' global ids resolve against. Absent on a Core
    // package (no materials.parquet listed) — appearance is then skipped.
    let global_materials: Option<Vec<serde_json::Value>> = tables
        .sidecar_files
        .iter()
        .any(|f| f == "materials.parquet")
        .then(|| read_materials(&opts.package_dir.join("materials.parquet")))
        .transpose()?;
    let global_textures: Option<Vec<serde_json::Value>> = tables
        .sidecar_files
        .iter()
        .any(|f| f == "textures.parquet")
        .then(|| read_textures(&opts.package_dir.join("textures.parquet")))
        .transpose()?;

    let mut report = WriteReport::default();
    let mut bounds = Bounds::new();
    let mut seen_ids: HashSet<String> = HashSet::new();
    let mut members = Writer::new(Vec::new());
    // First pass collects every Building/BuildingPart object so parts can be
    // nested under their parent (in the parent's stored `children` order).
    let mut content_by_id: HashMap<String, BuildingSolids> = HashMap::new();
    let mut children_by_id: HashMap<String, Vec<String>> = HashMap::new();
    let mut type_by_id: HashMap<String, String> = HashMap::new();
    let mut root_ids: Vec<String> = Vec::new();

    let mut first_builder = Some(first_builder);
    for (idx, path) in tables.tables.iter().enumerate() {
        let reader = if idx == 0 {
            let builder = first_builder.take().expect("first builder taken once");
            let pr = builder.build().map_err(|e| {
                CityParquetError::Parquet(format!("cannot build parquet reader: {e}"))
            })?;
            CityParquetRecordBatchReader::new(pr, Arc::clone(&schema))
        } else {
            let builder =
                ParquetRecordBatchReaderBuilder::try_new(fs::File::open(path).map_err(|e| {
                    CityParquetError::Io(format!("cannot open {}: {e}", path.display()))
                })?)
                .map_err(|e| {
                    CityParquetError::Parquet(format!("cannot open parquet reader: {e}"))
                })?;
            let table_meta = builder.cityparquet_metadata()?;
            if table_meta.cityparquet_version != meta.cityparquet_version {
                return Err(CityParquetError::Metadata(format!(
                    "table '{}' has cityparquet_version {:?}, expected {:?} (matching '{}')",
                    table_display_name(path),
                    table_meta.cityparquet_version,
                    meta.cityparquet_version,
                    table_display_name(first_path)
                )));
            }
            let table_schema = builder.cityparquet_arrow_schema()?;
            if let Some(mismatch) = first_schema_mismatch(&schema, &table_schema) {
                return Err(CityParquetError::Metadata(format!(
                    "table '{}' {mismatch} (matching '{}')",
                    table_display_name(path),
                    table_display_name(first_path)
                )));
            }
            let pr = builder.build().map_err(|e| {
                CityParquetError::Parquet(format!("cannot build parquet reader: {e}"))
            })?;
            CityParquetRecordBatchReader::new(pr, Arc::clone(&schema))
        };

        for batch in reader {
            let batch = batch?;
            let material_cols = appearance_columns(&batch, "material");
            let texture_cols = appearance_columns(&batch, "texture");
            let objects = decode_batch(&batch, &meta)?;
            for (row, obj) in objects.into_iter().enumerate() {
                let ty = obj.object.thetype.clone();
                // Only Building and BuildingPart are handled; other CityObject
                // types are out of scope (BuildingPart no longer counts here).
                if ty != "Building" && ty != "BuildingPart" {
                    report.non_building_skipped += 1;
                    continue;
                }
                // This row's appearance, rebuilt from the per-LoD
                // `material_lod*` / `texture_lod*` columns into a
                // `{"<canonical-lod>": {...}}` map and keyed out per geometry
                // below. `decode_batch` excludes these columns.
                let material_col = read_lod_keyed_appearance(&batch, &material_cols, row)?;
                let texture_col = read_lod_keyed_appearance(&batch, &texture_cols, row)?;
                let mut solids = Vec::new();
                for (lod, decoded, props) in obj.geometries {
                    // Real semantics on a geometry we are about to skip are
                    // dropped; count them (computed before `props` is moved).
                    let sem_count = semantics::droppable_surface_count(props.as_ref());
                    let lod_key = lod.map(|l| l.to_string()).unwrap_or_default();
                    let material = material_col.as_ref().and_then(|m| m.get(&lod_key)).cloned();
                    let texture = texture_col.as_ref().and_then(|t| t.get(&lod_key)).cloned();
                    match route_geometry(lod, decoded, props) {
                        GeomRoute::Emit(lod, decoded, props) => {
                            solids.push((lod, decoded, props, material, texture))
                        }
                        // No CityGML 2.0 Building slot for a MultiSolid.
                        GeomRoute::MultiSolid => {
                            report.multi_solids_skipped += 1;
                            report.semantic_surfaces_dropped += sem_count;
                        }
                        // A lodless geometry cannot be a lod<n>Solid.
                        GeomRoute::Lodless => {
                            report.lod_columns_skipped += 1;
                            report.semantic_surfaces_dropped += sem_count;
                        }
                    }
                }
                let attributes = obj
                    .object
                    .attributes
                    .as_ref()
                    .and_then(serde_json::Value::as_object)
                    .cloned()
                    .unwrap_or_default();
                let id = obj.id;
                // A duplicate CityObject id would silently overwrite the earlier
                // row (and evade the emit-time `seen_ids` check) — reject it.
                if type_by_id.contains_key(&id) {
                    return Err(CityParquetError::Metadata(format!(
                        "duplicate CityObject id {id:?} in the package"
                    )));
                }
                let children = obj.object.children.clone().unwrap_or_default();
                if ty == "Building" {
                    root_ids.push(id.clone());
                }
                type_by_id.insert(id.clone(), ty);
                children_by_id.insert(id.clone(), children);
                content_by_id.insert(
                    id.clone(),
                    BuildingSolids {
                        id,
                        attributes,
                        solids,
                    },
                );
            }
        }
    }

    // Emit each root Building with its parts (bottom-up; parts in `children`
    // order). `next_feature_index` namespaces each emitted object's polygon ids;
    // `reached_parts` collects every BuildingPart nested under some Building.
    let tree = BuildingTree {
        content_by_id: &content_by_id,
        children_by_id: &children_by_id,
        type_by_id: &type_by_id,
        types: &attr_types,
        materials: global_materials.as_deref(),
        textures: global_textures.as_deref(),
    };
    let mut next_feature_index = 0usize;
    let mut reached_parts: HashSet<String> = HashSet::new();
    for root_id in &root_ids {
        let mut visited: HashSet<String> = HashSet::new();
        let (inner, non_empty) = render_abstract_object(
            root_id,
            &tree,
            0,
            &mut next_feature_index,
            &mut bounds,
            &mut report,
            &mut visited,
            &mut seen_ids,
            &mut reached_parts,
        )?;
        if non_empty {
            members
                .write_event(Event::Start(BytesStart::new("cityObjectMember")))
                .map_err(io_err)?;
            let mut bldg = BytesStart::new("bldg:Building");
            bldg.push_attribute(("gml:id", root_id.as_str()));
            members.write_event(Event::Start(bldg)).map_err(io_err)?;
            members.get_mut().write_all(&inner).map_err(io_err)?;
            members
                .write_event(Event::End(BytesEnd::new("bldg:Building")))
                .map_err(io_err)?;
            members
                .write_event(Event::End(BytesEnd::new("cityObjectMember")))
                .map_err(io_err)?;
            report.buildings_written += 1;
        } else {
            report.buildings_without_solid_skipped += 1;
        }
    }

    // A BuildingPart never nested under any Building (not reached via any
    // parent's `children`) is orphaned — its geometry has no home.
    for (id, ty) in &type_by_id {
        if ty == "BuildingPart" && !reached_parts.contains(id) {
            report.building_parts_orphaned += 1;
        }
    }

    // Assemble the document: xml decl + CityModel(open + envelope) + buffered
    // members + CityModel close. Envelope-before-members ordering is why the
    // members are buffered first.
    let file = fs::File::create(&opts.output).map_err(|e| {
        CityParquetError::Io(format!("cannot create {}: {e}", opts.output.display()))
    })?;
    let mut doc = Writer::new(file);
    doc.write_event(Event::Decl(BytesDecl::new("1.0", Some("UTF-8"), None)))
        .map_err(io_err)?;
    write_city_model_open(&mut doc, srs_name.as_deref(), &bounds)?;
    doc.get_mut()
        .write_all(&members.into_inner())
        .map_err(io_err)?;
    write_city_model_close(&mut doc)?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn solid(kind: DecodedKind) -> DecodedGeometry {
        DecodedGeometry {
            coords: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0]],
            kind,
        }
    }

    fn geometry_collection() -> DecodedKind {
        DecodedKind::GeometryCollection(vec![DecodedKind::PolyhedralSurface(vec![vec![vec![
            0usize, 1, 2,
        ]]])])
    }

    #[test]
    fn multi_solid_is_routed_to_skip() {
        let props = Some(json!({ "type": "MultiSolid", "shells": [[1]] }));
        let route = route_geometry(
            Some(Lod::parse("2").unwrap()),
            solid(geometry_collection()),
            props,
        );
        assert!(matches!(route, GeomRoute::MultiSolid));
    }

    #[test]
    fn composite_solid_is_routed_to_emit() {
        let props = Some(json!({ "type": "CompositeSolid", "shells": [[1]] }));
        let route = route_geometry(
            Some(Lod::parse("2").unwrap()),
            solid(geometry_collection()),
            props,
        );
        assert!(matches!(route, GeomRoute::Emit(..)));
    }

    #[test]
    fn plain_solid_is_routed_to_emit() {
        let props = Some(json!({ "type": "Solid" }));
        let route = route_geometry(
            Some(Lod::parse("2").unwrap()),
            solid(DecodedKind::PolyhedralSurface(vec![vec![vec![
                0usize, 1, 2,
            ]]])),
            props,
        );
        assert!(matches!(route, GeomRoute::Emit(..)));
    }

    #[test]
    fn lodless_geometry_is_routed_to_skip() {
        let route = route_geometry(
            None,
            solid(DecodedKind::PolyhedralSurface(vec![vec![vec![
                0usize, 1, 2,
            ]]])),
            Some(json!({ "type": "Solid" })),
        );
        assert!(matches!(route, GeomRoute::Lodless));
    }
}
