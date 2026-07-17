//! Pass 1: a single read-only scan over a [`Source`] that infers the
//! CityParquet schema (LoDs, attribute columns) and the dataset-level
//! metadata (CRS, transform, bbox) needed before writing any Parquet.
//!
//! This never retains WKB buffers or vertex data — it exists to answer "what
//! columns and metadata does this dataset need?" so pass 2 (the writer) can
//! allocate the right Arrow arrays up front.

use cityparquet_schema::{
    AttributeInferer, CITYPARQUET_VERSION, CityParquetError, CityParquetMetadata,
    CityParquetSchema, Lod, Result, SourceFormat as SchemaSourceFormat, normalise_attribute_name,
};

use cjseq::GeometryType;

use crate::source::{Source, SourceFormat};
use crate::wkb_write::{VertexPool, geometry_to_wkb};

/// Outcome of scanning a [`Source`] once: the inferred schema plus the
/// dataset-level facts ([`CityParquetMetadata`] needs) that only a full scan
/// can answer.
#[derive(Debug, Clone)]
pub struct ScanResult {
    pub schema: CityParquetSchema,
    /// LoDs present, ascending; empty means the dataset has no analysis
    /// geometry (only `GeometryInstance`s, or no geometry at all) and uses the
    /// un-suffixed `geometry` column. Every non-instance source geometry has a
    /// lod — [`scan`] rejects any that does not (§9, CityJSON 2.0 §3).
    pub lods: Vec<Lod>,
    pub object_count: usize,
    /// Union of every analysis geometry's bbox, `None` if none contributed one
    /// (`GeometryInstance`s produce no WKB, so they contribute nothing here).
    pub dataset_bbox: Option<[f64; 6]>,
    /// The dataset's reference system as the raw OGC CRS URL string (from
    /// CityJSON header metadata), not yet resolved to full PROJJSON.
    pub crs_url: Option<String>,
    /// The CityJSON header's `transform`, kept for the writer's requantisation.
    pub transform: serde_json::Value,
    /// The CityJSON header's `extensions` declarations, verbatim (absent
    /// stays `None`; an empty object stays an empty object).
    pub extensions: Option<serde_json::Value>,
    /// The CityJSON header's `metadata` object, re-serialised verbatim. See
    /// [`CityParquetMetadata::source_metadata`] for the passthrough
    /// limitation (e.g. `fullMetadataUrl` is never preserved).
    pub source_metadata: Option<serde_json::Value>,
    /// `{"default-theme-material": ..., "default-theme-texture": ...}` from
    /// the header's `appearance` default-theme members, `None` if neither is
    /// set.
    pub appearance_defaults: Option<serde_json::Value>,
    source_format: SchemaSourceFormat,
    source_version: String,
}

fn to_schema_source_format(format: SourceFormat) -> SchemaSourceFormat {
    match format {
        SourceFormat::CityJson => SchemaSourceFormat::CityJson,
        SourceFormat::CityJsonSeq => SchemaSourceFormat::CityJsonSeq,
        SourceFormat::CityGml => SchemaSourceFormat::CityGml,
    }
}

/// Expand `acc` to also cover `bbox`.
fn union_bbox(acc: &mut Option<[f64; 6]>, bbox: [f64; 6]) {
    *acc = Some(match acc.take() {
        None => bbox,
        Some(mut cur) => {
            for i in 0..3 {
                cur[i] = cur[i].min(bbox[i]);
                cur[i + 3] = cur[i + 3].max(bbox[i + 3]);
            }
            cur
        }
    });
}

/// Scan every feature and object in `source` once, inferring the attribute
/// and LoD schema and accumulating the dataset-level bbox, CRS, and transform.
pub fn scan(source: &Source) -> Result<ScanResult> {
    let header = source.header();
    let mut inferer = AttributeInferer::default();
    let mut lod_strings: Vec<String> = Vec::new();
    let mut object_count = 0usize;
    let mut bbox_with_lod: Option<[f64; 6]> = None;
    let mut geometries_with_lod = 0usize;

    for feature in source.features()? {
        let feature = feature?;
        let pool = VertexPool::new(&feature.vertices, &header.transform);

        for (id, co) in &feature.city_objects {
            object_count += 1;

            if let Some(attrs) = co.attributes.as_ref().and_then(|v| v.as_object()) {
                for (name, value) in attrs {
                    inferer.observe(&normalise_attribute_name(name), value);
                }
            }

            let Some(geoms) = &co.geometry else {
                continue;
            };
            for geom in geoms {
                let bbox = geometry_to_wkb(geom, &pool)?.map(|outcome| outcome.bbox);
                match &geom.lod {
                    Some(lod) => {
                        Lod::parse(lod).map_err(|_| {
                            CityParquetError::Lod(format!("object {id}: invalid lod {lod:?}"))
                        })?;
                        if !lod_strings.contains(lod) {
                            lod_strings.push(lod.clone());
                        }
                        geometries_with_lod += 1;
                        if let Some(bbox) = bbox {
                            union_bbox(&mut bbox_with_lod, bbox);
                        }
                    }
                    None => {
                        // A `GeometryInstance` is lod-less by design — its
                        // referenced template carries the lod (§12) and it
                        // routes to the `template` column, not a geometry
                        // column. Any OTHER lod-less geometry is invalid
                        // CityJSON 2.0 (§3 requires `lod` on every
                        // non-instance geometry) and MUST be rejected here,
                        // not silently dropped (in a mixed dataset) nor kept
                        // in an un-suffixed column (in a uniformly lod-less
                        // one) — the two behaviours the old code chose
                        // between depending on the rest of the dataset.
                        if geom.thetype != GeometryType::GeometryInstance {
                            return Err(CityParquetError::Lod(format!(
                                "object {id}: geometry has no \"lod\" (CityJSON 2.0 §3 \
                                 requires it on every non-GeometryInstance geometry)"
                            )));
                        }
                    }
                }
            }
        }
    }

    // A dataset with at least one LoD-bearing geometry uses per-LoD columns
    // (every non-instance geometry now has a lod — the loop above rejected any
    // that did not). A dataset with none — only `GeometryInstance`s, or an
    // attributes-only dataset with no geometry at all — has no analysis
    // geometry, hence no LoDs and no dataset bbox; it uses the un-suffixed
    // geometry columns (the zero-analysis-geometry case, §9 / Appendix B).
    let (lods, dataset_bbox) = if geometries_with_lod > 0 {
        let mut lods: Vec<Lod> = lod_strings
            .iter()
            .map(|s| Lod::parse(s).expect("already validated above"))
            .collect();
        lods.sort();
        lods.dedup();
        (lods, bbox_with_lod)
    } else {
        (Vec::new(), None)
    };

    let crs_url = header
        .metadata
        .as_ref()
        .and_then(|m| m.reference_system.clone())
        .map(|rs| match serde_json::to_value(&rs)? {
            serde_json::Value::String(s) => Ok(s),
            other => Err(CityParquetError::Schema(format!(
                "ReferenceSystem serialised to non-string JSON: {other}"
            ))),
        })
        .transpose()?;

    let transform = serde_json::to_value(&header.transform)?;

    let source_metadata = header
        .metadata
        .as_ref()
        .map(serde_json::to_value)
        .transpose()?;

    let appearance_defaults = header.appearance.as_ref().and_then(|a| {
        let material = a.default_theme_material.clone();
        let texture = a.default_theme_texture.clone();
        if material.is_none() && texture.is_none() {
            return None;
        }
        let mut map = serde_json::Map::new();
        if let Some(m) = material {
            map.insert(
                "default-theme-material".to_string(),
                serde_json::Value::String(m),
            );
        }
        if let Some(t) = texture {
            map.insert(
                "default-theme-texture".to_string(),
                serde_json::Value::String(t),
            );
        }
        Some(serde_json::Value::Object(map))
    });

    let schema = CityParquetSchema {
        lods: lods.clone(),
        attributes: inferer.finish(),
        crs: None,
    };

    Ok(ScanResult {
        schema,
        lods,
        object_count,
        dataset_bbox,
        crs_url,
        transform,
        extensions: header.extensions.clone(),
        source_metadata,
        appearance_defaults,
        source_format: to_schema_source_format(source.format()),
        source_version: header.version.clone(),
    })
}

impl ScanResult {
    /// Build the full `CityParquetMetadata` for this scan, filling every
    /// spec key: attribute/reserved column lists come from the schema's own
    /// rendered Arrow schema (never hand-duplicated), the default geometry
    /// column is the highest LoD present (or the plain `geometry` column if
    /// `lods` is empty), and `sidecars` are the compatibility-profile sidecar
    /// file names to record (empty for the core profile).
    pub fn metadata(&self, sidecars: &[String]) -> Result<CityParquetMetadata> {
        // Only the attribute list is stored (§13.1): a reader recovers the
        // reserved columns as "everything not listed here".
        let (_reserved_columns, attribute_columns) = self.schema.column_lists()?;

        let default_geometry = match self.lods.last() {
            Some(highest) => format!("geometry_{}", highest.column_suffix()),
            None => "geometry".to_string(),
        };

        Ok(CityParquetMetadata {
            cityparquet_version: CITYPARQUET_VERSION.to_string(),
            source_format: self.source_format,
            source_version: Some(self.source_version.clone()),
            crs: self.crs_url.clone().map(serde_json::Value::String),
            transform: Some(self.transform.clone()),
            extensions: self.extensions.clone(),
            attribute_columns,
            default_geometry,
            bbox_column: "bbox".to_string(),
            sidecar_files: sidecars.to_vec(),
            source_metadata: self.source_metadata.clone(),
            appearance_defaults: self.appearance_defaults.clone(),
            other: None,
        })
    }
}
