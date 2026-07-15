//! Native CityGML 2.0 output writer (CityParquet package -> .gml).
//!
//! W-M1: CityModel + envelope + srsName, and bldg:Building with LoD gml:Solid.
//! Standalone — reuses wkb_read/reader/export shell helpers, no cjseq document.

pub mod attributes;
pub mod building;
pub mod document;
pub mod geometry;
pub mod semantics;

use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use cityparquet_schema::{CityParquetError, Lod, PackageManifest};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use quick_xml::Writer;
use quick_xml::events::{BytesDecl, Event};

use self::building::{BuildingSolids, write_building};
use self::document::{Bounds, write_city_model_close, write_city_model_open};
use crate::Result;
use crate::citygml::crs::srs_name_for;
use crate::decode::decode_batch;
use crate::export::first_schema_mismatch;
use crate::reader::{CityParquetReaderBuilder, CityParquetRecordBatchReader};
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
    let manifest_path = opts.package_dir.join("metadata.json");
    let manifest_text = fs::read_to_string(&manifest_path).map_err(|e| {
        CityParquetError::Io(format!("cannot read {}: {e}", manifest_path.display()))
    })?;
    let manifest: PackageManifest = serde_json::from_str(&manifest_text)?;
    if manifest.tables.is_empty() {
        return Err(CityParquetError::Metadata(
            "package manifest lists no tables".to_string(),
        ));
    }
    let mut seen_tables = HashSet::with_capacity(manifest.tables.len());
    for name in &manifest.tables {
        if !seen_tables.insert(name) {
            return Err(CityParquetError::Metadata(format!(
                "package manifest lists duplicate table '{name}'"
            )));
        }
    }

    // First table's footer metadata + rendered schema are authoritative.
    let first_name = &manifest.tables[0];
    let first_path = opts.package_dir.join(first_name);
    let first_builder =
        ParquetRecordBatchReaderBuilder::try_new(fs::File::open(&first_path).map_err(|e| {
            CityParquetError::Io(format!("cannot open {}: {e}", first_path.display()))
        })?)
        .map_err(|e| CityParquetError::Parquet(format!("cannot open parquet reader: {e}")))?;
    let meta = first_builder.cityparquet_metadata()?;
    let schema = first_builder.cityparquet_arrow_schema()?;
    let srs_name = srs_name_for(meta.crs.as_ref())?;
    // Stored attribute column types drive attribute routing (not value shapes).
    let attr_types = attributes::attribute_types(&schema, &meta.attribute_columns);

    let mut report = WriteReport::default();
    let mut bounds = Bounds::new();
    let mut seen_ids: HashSet<String> = HashSet::new();
    let mut members = Writer::new(Vec::new());
    // Monotonic per-Building index; namespaces each building's polygon gml:ids.
    let mut next_feature_index = 0usize;

    let mut first_builder = Some(first_builder);
    for (idx, name) in manifest.tables.iter().enumerate() {
        let reader = if idx == 0 {
            let builder = first_builder.take().expect("first builder taken once");
            let pr = builder.build().map_err(|e| {
                CityParquetError::Parquet(format!("cannot build parquet reader: {e}"))
            })?;
            CityParquetRecordBatchReader::new(pr, Arc::clone(&schema))
        } else {
            let path = opts.package_dir.join(name);
            let builder =
                ParquetRecordBatchReaderBuilder::try_new(fs::File::open(&path).map_err(|e| {
                    CityParquetError::Io(format!("cannot open {}: {e}", path.display()))
                })?)
                .map_err(|e| {
                    CityParquetError::Parquet(format!("cannot open parquet reader: {e}"))
                })?;
            let table_meta = builder.cityparquet_metadata()?;
            if table_meta.cityparquet_version != meta.cityparquet_version {
                return Err(CityParquetError::Metadata(format!(
                    "table '{name}' has cityparquet_version {:?}, expected {:?} (matching '{first_name}')",
                    table_meta.cityparquet_version, meta.cityparquet_version
                )));
            }
            let table_schema = builder.cityparquet_arrow_schema()?;
            if let Some(mismatch) = first_schema_mismatch(&schema, &table_schema) {
                return Err(CityParquetError::Metadata(format!(
                    "table '{name}' {mismatch} (matching '{first_name}')"
                )));
            }
            let pr = builder.build().map_err(|e| {
                CityParquetError::Parquet(format!("cannot build parquet reader: {e}"))
            })?;
            CityParquetRecordBatchReader::new(pr, Arc::clone(&schema))
        };

        for batch in reader {
            let batch = batch?;
            for obj in decode_batch(&batch, &meta)? {
                if obj.object.thetype != "Building" {
                    report.non_building_skipped += 1;
                    continue;
                }
                let feature_index = next_feature_index;
                next_feature_index += 1;
                let mut solids = Vec::new();
                for (lod, decoded, props) in obj.geometries {
                    // Real semantics on a geometry we are about to skip are
                    // dropped; count them (computed before `props` is moved).
                    let sem_count = semantics::droppable_surface_count(props.as_ref());
                    match route_geometry(lod, decoded, props) {
                        GeomRoute::Emit(lod, decoded, props) => solids.push((lod, decoded, props)),
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
                let building = BuildingSolids {
                    id: obj.id,
                    attributes,
                    solids,
                };
                // Buffer this member on its own so the document-unique `gml:id`
                // is reserved ONLY for a Building that actually emits — a
                // duplicate id shared by two skipped/no-solid Buildings must not
                // fail a document in which neither id ever appears.
                let mut member = Writer::new(Vec::new());
                if write_building(
                    &mut member,
                    &building,
                    &attr_types,
                    feature_index,
                    &mut bounds,
                    &mut report,
                )? {
                    if !seen_ids.insert(building.id.clone()) {
                        return Err(CityParquetError::Schema(format!(
                            "duplicate CityObject id {:?}; CityGML gml:id must be document-unique",
                            building.id
                        )));
                    }
                    members
                        .get_mut()
                        .write_all(&member.into_inner())
                        .map_err(io_err)?;
                    report.buildings_written += 1;
                } else {
                    report.buildings_without_solid_skipped += 1;
                }
            }
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
        let props = Some(json!({ "type": "MultiSolid", "solid_shell_faces": [[1]] }));
        let route = route_geometry(
            Some(Lod::parse("2").unwrap()),
            solid(geometry_collection()),
            props,
        );
        assert!(matches!(route, GeomRoute::MultiSolid));
    }

    #[test]
    fn composite_solid_is_routed_to_emit() {
        let props = Some(json!({ "type": "CompositeSolid", "solid_shell_faces": [[1]] }));
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
