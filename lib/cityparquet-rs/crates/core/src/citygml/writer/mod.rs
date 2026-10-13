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
use cityparquet_schema::crs::AxisOrder;

use crate::citygml::crs::srs_name_for;
use crate::decode::decode_batch;
use crate::export::{
    AppearanceKind, appearance_columns, read_lod_keyed_appearance, table_display_name,
};
use crate::family::{Families, FamilyRow};
use crate::id_hash::id_hash;
use crate::reader::{CityParquetReaderBuilder, CityParquetRecordBatchReader};
use crate::sidecar::{read_materials, read_textures};
use crate::stac::properties::PackageTables;
use crate::wkb_read::{DecodedGeometry, DecodedKind};

/// Options for one CityParquet package -> CityGML 2.0 `.gml` conversion.
pub struct WriteOptions {
    pub package_dir: PathBuf,
    /// The `.gml` file to write (replaced if it exists, matching `export`).
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
/// `bldg:lod<major>Solid` directly. The first table supplies the genuinely
/// dataset-wide facts (`crs`/`attribute_columns`, both identical across every
/// table — see `crate::export::export`'s doc comment on the same point);
/// every table's ROWS are otherwise decoded against its own footer metadata
/// and its own rendered Arrow schema, since a module's own table legitimately
/// carries only the geometry/appearance columns its own rows need (spec
/// "object-table-schema").
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
            CityParquetError::io_source(format!("cannot open {}", first_path.display()), e)
        })?)
        .map_err(|e| CityParquetError::parquet_source("cannot open parquet reader", e))?;
    let meta = first_builder.cityparquet_metadata()?;
    let schema = first_builder.cityparquet_arrow_schema()?;
    // Both no-CRS states (explicit `null`, absent key) advertise no `srsName`
    // — the CityGML writer, like every other consumer, only ever states a CRS
    // it actually has. This inherits the known gap documented on
    // `crate::export`'s `reference_system`: a FOREIGN file carrying real
    // coordinates under an absent `crs` should, by the spec's reading rule, be
    // treated as OGC:CRS84, and is instead written with no `srsName`. Pending
    // a spec clarification — see that doc comment for why the two absent-`crs`
    // shapes cannot yet be told apart here.
    let srs_name = srs_name_for(meta.crs.known())?;
    // The package stores WKB in GeoParquet's (longitude, latitude) order,
    // while the `srsName` written above names the authority's own axis order.
    // Every decoded coordinate is put back into that order before it reaches a
    // `gml:posList`, or the document would contradict the CRS it declares.
    let axis_order = meta.crs.known().map(AxisOrder::of).unwrap_or_default();
    // Stored attribute column types drive attribute routing (not value shapes).
    let attr_types = attributes::attribute_types(&schema, &meta.attributes);

    // Global materials table (Compatibility profile): appearance definitions the
    // per-geometry material maps' global ids resolve against. Absent on a Core
    // package (no materials.parquet listed) — appearance is then skipped.
    let global_materials: Option<HashMap<i64, serde_json::Value>> = tables
        .sidecar_files
        .iter()
        .any(|f| f == "materials.parquet")
        .then(|| read_materials(&opts.package_dir.join("materials.parquet")))
        .transpose()?
        .map(|defs| defs.into_iter().collect());
    let global_textures: Option<HashMap<i64, serde_json::Value>> = tables
        .sidecar_files
        .iter()
        .any(|f| f == "textures.parquet")
        .then(|| read_textures(&opts.package_dir.join("textures.parquet")))
        .transpose()?
        .map(|defs| defs.into_iter().collect());

    // Members are spilled to a sibling file as each family is rendered: the
    // document's envelope precedes them but is known only at the end.
    let name = opts
        .output
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let spill = TempFile(
        opts.output
            .with_file_name(format!(".{name}.members.partial")),
    );
    let spill_file = fs::File::create(&spill.0).map_err(|e| {
        CityParquetError::io_source(format!("cannot create {}", spill.0.display()), e)
    })?;
    let mut render = FamilyRenderer {
        members: Writer::new(std::io::BufWriter::new(spill_file)),
        bounds: Bounds::new(),
        report: WriteReport::default(),
        next_feature_index: 0,
        types: &attr_types,
        materials: global_materials.as_ref(),
        textures: global_textures.as_ref(),
        strays: HashMap::new(),
    };
    // Counts made while reading rows, before any family is rendered.
    let mut read = WriteReport::default();
    let mut ids: HashSet<u128> = HashSet::new();
    let mut families = Families::<GmlRow>::default();
    let mut emit = |_: String, rows: Vec<GmlRow>| render.family(rows);

    let mut first_builder = Some(first_builder);
    for (idx, path) in tables.tables.iter().enumerate() {
        let (reader, table_meta) = if idx == 0 {
            let builder = first_builder.take().expect("first builder taken once");
            let pr = builder
                .build()
                .map_err(|e| CityParquetError::parquet_source("cannot build parquet reader", e))?;
            (
                CityParquetRecordBatchReader::new(pr, Arc::clone(&schema)),
                meta.clone(),
            )
        } else {
            let builder =
                ParquetRecordBatchReaderBuilder::try_new(fs::File::open(path).map_err(|e| {
                    CityParquetError::io_source(format!("cannot open {}", path.display()), e)
                })?)
                .map_err(|e| CityParquetError::parquet_source("cannot open parquet reader", e))?;
            let table_meta = builder.cityparquet_metadata()?;
            if table_meta.version != meta.version {
                return Err(CityParquetError::Metadata(format!(
                    "table '{}' has city.version {:?}, expected {:?} (matching '{}')",
                    table_display_name(path),
                    table_meta.version,
                    meta.version,
                    table_display_name(first_path)
                )));
            }
            // Each table's own rendered Arrow schema — never cross-checked
            // against the first table's, since a module's own table
            // legitimately carries only the geometry/appearance columns its
            // own rows need (spec "object-table-schema").
            let table_schema = builder.cityparquet_arrow_schema()?;
            let pr = builder
                .build()
                .map_err(|e| CityParquetError::parquet_source("cannot build parquet reader", e))?;
            (
                CityParquetRecordBatchReader::new(pr, Arc::clone(&table_schema)),
                table_meta,
            )
        };

        for batch in reader {
            let batch = batch?;
            let material_cols = appearance_columns(&batch, AppearanceKind::Material);
            let texture_cols = appearance_columns(&batch, AppearanceKind::Texture);
            let mut objects = decode_batch(&batch, &table_meta)?;
            if axis_order != AxisOrder::LonLat {
                for obj in &mut objects {
                    for (_, decoded, _) in &mut obj.geometries {
                        for c in &mut decoded.coords {
                            *c = axis_order.apply(*c);
                        }
                    }
                }
            }
            for (row, obj) in objects.into_iter().enumerate() {
                // A duplicate CityObject id would make two objects share a
                // `gml:id`; reject it.
                if !ids.insert(id_hash(&obj.id)) {
                    return Err(CityParquetError::Metadata(format!(
                        "duplicate CityObject id {:?} in the package",
                        obj.id
                    )));
                }
                let ty = crate::cjtypes::object_type_name(&obj.object.thetype);
                let children = obj.object.children.clone().unwrap_or_default();
                // Only Building and BuildingPart are written; other CityObject
                // types still pass through the family assembler, which needs
                // them to know when a family is complete.
                let content = if ty == "Building" || ty == "BuildingPart" {
                    Some(building_solids(
                        obj.id.clone(),
                        obj.object.attributes.clone(),
                        obj.geometries,
                        &batch,
                        &material_cols,
                        &texture_cols,
                        row,
                        &mut read,
                    )?)
                } else {
                    read.non_building_skipped += 1;
                    None
                };
                let gml_row = GmlRow {
                    id: obj.id,
                    feature_id: obj.feature_id,
                    ty,
                    children,
                    content,
                };
                families.push(path, gml_row, &mut emit)?;
            }
        }
        families.end_table(&mut emit)?;
    }
    families.finish(&mut emit)?;

    let FamilyRenderer {
        members,
        bounds,
        mut report,
        strays,
        ..
    } = render;
    // Parts no family claimed: their geometry has no home.
    report.building_parts_orphaned += strays
        .values()
        .filter(|row| row.ty == "BuildingPart")
        .count();
    members
        .into_inner()
        .into_inner()
        .map_err(|e| CityParquetError::io_source("write error", e.into_error()))?;
    report.non_building_skipped += read.non_building_skipped;
    report.multi_solids_skipped += read.multi_solids_skipped;
    report.lod_columns_skipped += read.lod_columns_skipped;
    report.semantic_surfaces_dropped += read.semantic_surfaces_dropped;

    // Assemble the document: xml decl + CityModel(open + envelope) + the
    // spilled members + CityModel close. It is written beside the output
    // and renamed into place, so a failure leaves any earlier output whole.
    let partial = TempFile(opts.output.with_file_name(format!(".{name}.partial")));
    let file = fs::File::create(&partial.0).map_err(|e| {
        CityParquetError::io_source(format!("cannot create {}", partial.0.display()), e)
    })?;
    let mut doc = Writer::new(std::io::BufWriter::new(file));
    doc.write_event(Event::Decl(BytesDecl::new("1.0", Some("UTF-8"), None)))?;
    write_city_model_open(&mut doc, srs_name.as_deref(), &bounds)?;
    let mut spilled = fs::File::open(&spill.0).map_err(|e| {
        CityParquetError::io_source(format!("cannot open {}", spill.0.display()), e)
    })?;
    std::io::copy(&mut spilled, doc.get_mut())?;
    write_city_model_close(&mut doc)?;
    doc.into_inner()
        .into_inner()
        .map_err(|e| CityParquetError::io_source("write error", e.into_error()))?;
    fs::rename(&partial.0, &opts.output).map_err(|e| {
        CityParquetError::io_source(format!("cannot write {}", opts.output.display()), e)
    })?;
    Ok(report)
}

/// One row the writer holds until its feature family is complete. `content`
/// is `None` for an object type the writer does not emit.
struct GmlRow {
    id: String,
    feature_id: Option<String>,
    ty: String,
    children: Vec<String>,
    content: Option<BuildingSolids>,
}

impl FamilyRow for GmlRow {
    fn id(&self) -> &str {
        &self.id
    }
    fn feature_id(&self) -> Option<&str> {
        self.feature_id.as_deref()
    }
    fn children(&self) -> &[String] {
        &self.children
    }
}

/// A Building or BuildingPart row's emittable solids, with this row's
/// appearance keyed out per geometry. Skipped geometries are counted in
/// `report`.
#[allow(clippy::too_many_arguments)]
fn building_solids(
    id: String,
    attributes: Option<serde_json::Map<String, serde_json::Value>>,
    geometries: Vec<(Option<Lod>, DecodedGeometry, Option<serde_json::Value>)>,
    batch: &arrow_array::RecordBatch,
    material_cols: &[(usize, String)],
    texture_cols: &[(usize, String)],
    row: usize,
    report: &mut WriteReport,
) -> Result<BuildingSolids> {
    // This row's appearance, rebuilt from the per-LoD `material_lod*` /
    // `texture_lod*` columns into a `{"<canonical-lod>": {...}}` map and
    // keyed out per geometry below. `decode_batch` excludes these columns.
    let material_col =
        read_lod_keyed_appearance(batch, material_cols, row, AppearanceKind::Material)?;
    let texture_col = read_lod_keyed_appearance(batch, texture_cols, row, AppearanceKind::Texture)?;
    let mut solids = Vec::new();
    for (lod, decoded, props) in geometries {
        // Real semantics on a geometry we are about to skip are dropped;
        // count them (computed before `props` is moved).
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
    Ok(BuildingSolids {
        id,
        attributes: attributes.unwrap_or_default(),
        solids,
    })
}

/// Renders each complete feature family's Buildings, with their parts
/// nested, into the spilled member stream.
struct FamilyRenderer<'a> {
    members: Writer<std::io::BufWriter<fs::File>>,
    bounds: Bounds,
    report: WriteReport,
    /// Namespaces each emitted object's polygon ids.
    next_feature_index: usize,
    types: &'a HashMap<String, cityparquet_schema::AttributeType>,
    materials: Option<&'a HashMap<i64, serde_json::Value>>,
    textures: Option<&'a HashMap<i64, serde_json::Value>>,
    /// Parts no Building of their own family reached, held for a later
    /// family that names them as children. A conforming package has none: a
    /// part's `feature_id` is its Building's root (spec 02 "feature_id
    /// rule"). A package that gives a part a family of its own still nests
    /// it, because the Building's family, naming a child it lacks, is parked
    /// until the end of the package.
    strays: HashMap<String, GmlRow>,
}

impl FamilyRenderer<'_> {
    /// Emit each Building of one family with its parts (bottom-up; parts in
    /// `children` order), drawing on [`Self::strays`] for children the
    /// family lacks.
    fn family(&mut self, rows: Vec<GmlRow>) -> Result<()> {
        let mut content_by_id: HashMap<String, BuildingSolids> = HashMap::new();
        let mut children_by_id: HashMap<String, Vec<String>> = HashMap::new();
        let mut type_by_id: HashMap<String, String> = HashMap::new();
        let mut root_ids: Vec<String> = Vec::new();
        let mut pending: Vec<GmlRow> = rows;
        while let Some(row) = pending.pop() {
            let Some(content) = row.content else {
                continue;
            };
            for child in &row.children {
                if !content_by_id.contains_key(child)
                    && let Some(stray) = self.strays.remove(child)
                {
                    pending.push(stray);
                }
            }
            if row.ty == "Building" {
                root_ids.push(row.id.clone());
            }
            type_by_id.insert(row.id.clone(), row.ty);
            children_by_id.insert(row.id.clone(), row.children);
            content_by_id.insert(row.id, content);
        }
        // Buildings in row order, whatever order the strays joined in.
        root_ids.reverse();
        let tree = BuildingTree {
            content_by_id: &content_by_id,
            children_by_id: &children_by_id,
            type_by_id: &type_by_id,
            types: self.types,
            materials: self.materials,
            textures: self.textures,
        };
        let mut seen_ids: HashSet<String> = HashSet::new();
        let mut reached_parts: HashSet<String> = HashSet::new();
        for root_id in &root_ids {
            let mut visited: HashSet<String> = HashSet::new();
            let (inner, non_empty) = render_abstract_object(
                root_id,
                &tree,
                0,
                &mut self.next_feature_index,
                &mut self.bounds,
                &mut self.report,
                &mut visited,
                &mut seen_ids,
                &mut reached_parts,
            )?;
            if non_empty {
                let members = &mut self.members;
                members.write_event(Event::Start(BytesStart::new("cityObjectMember")))?;
                let mut bldg = BytesStart::new("bldg:Building");
                bldg.push_attribute(("gml:id", root_id.as_str()));
                members.write_event(Event::Start(bldg))?;
                members.get_mut().write_all(&inner)?;
                members.write_event(Event::End(BytesEnd::new("bldg:Building")))?;
                members.write_event(Event::End(BytesEnd::new("cityObjectMember")))?;
                self.report.buildings_written += 1;
            } else {
                self.report.buildings_without_solid_skipped += 1;
            }
        }
        // A BuildingPart no Building reached waits as a stray; one no family
        // ever claims is orphaned when the package ends.
        for (id, ty) in type_by_id {
            if ty == "BuildingPart" && !reached_parts.contains(&id) {
                let content = content_by_id.remove(&id);
                let children = children_by_id.remove(&id).unwrap_or_default();
                self.strays.insert(
                    id.clone(),
                    GmlRow {
                        id,
                        feature_id: None,
                        ty,
                        children,
                        content,
                    },
                );
            }
        }
        Ok(())
    }
}

/// A temporary file removed when dropped, on success and failure alike.
struct TempFile(PathBuf);

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
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
