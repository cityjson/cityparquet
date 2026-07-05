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

use crate::source::{Source, SourceFormat};
use crate::wkb_write::{VertexPool, geometry_to_wkb};

/// Outcome of scanning a [`Source`] once: the inferred schema plus the
/// dataset-level facts ([`CityParquetMetadata`] needs) that only a full scan
/// can answer.
#[derive(Debug, Clone)]
pub struct ScanResult {
    pub schema: CityParquetSchema,
    /// LoDs present, ascending; empty means every kept geometry goes in a
    /// single un-suffixed `geometry` column.
    pub lods: Vec<Lod>,
    pub object_count: usize,
    /// Union of every kept geometry's bbox (see `lodless_geometries` for what
    /// "kept" excludes), `None` if no geometry contributed one.
    pub dataset_bbox: Option<[f64; 6]>,
    /// The dataset's reference system as the raw OGC CRS URL string (from
    /// CityJSON header metadata), not yet resolved to full PROJJSON.
    pub crs_url: Option<String>,
    /// The CityJSON header's `transform`, kept for the writer's requantisation.
    pub transform: serde_json::Value,
    /// The CityJSON header's `extensions` declarations, verbatim (absent
    /// stays `None`; an empty object stays an empty object).
    pub extensions: Option<serde_json::Value>,
    /// Count of geometries with no `lod` string, on a dataset that also has
    /// LoD-bearing geometries. These are skipped from `lods` and
    /// `dataset_bbox` because there is no per-LoD column to place them in;
    /// see the module docs on the "mixed" binding rule for why a dataset
    /// that is *uniformly* LoD-less does not skip anything.
    pub lodless_geometries: usize,
    source_format: SchemaSourceFormat,
    source_version: String,
}

fn to_schema_source_format(format: SourceFormat) -> SchemaSourceFormat {
    match format {
        SourceFormat::CityJson => SchemaSourceFormat::CityJson,
        SourceFormat::CityJsonSeq => SchemaSourceFormat::CityJsonSeq,
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
    // Kept separately because whether the lod-less bucket is "skipped" (mixed
    // dataset) or "the whole dataset" (uniformly lod-less) is only knowable
    // after the full scan.
    let mut bbox_with_lod: Option<[f64; 6]> = None;
    let mut bbox_lodless: Option<[f64; 6]> = None;
    let mut geometries_with_lod = 0usize;
    let mut geometries_without_lod = 0usize;

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
                        geometries_without_lod += 1;
                        if let Some(bbox) = bbox {
                            union_bbox(&mut bbox_lodless, bbox);
                        }
                    }
                }
            }
        }
    }

    // Binding rule: a dataset with at least one LoD-bearing geometry uses
    // per-LoD columns, and any lod-less geometry is skipped (counted) since
    // it has no column to go in. A dataset with none at all (uniformly
    // lod-less, including the no-geometry-at-all case) uses the plain
    // un-suffixed `geometry` column and keeps every geometry.
    let (lods, dataset_bbox, lodless_geometries) = if geometries_with_lod > 0 {
        let mut lods: Vec<Lod> = lod_strings
            .iter()
            .map(|s| Lod::parse(s).expect("already validated above"))
            .collect();
        lods.sort();
        lods.dedup();
        (lods, bbox_with_lod, geometries_without_lod)
    } else {
        (Vec::new(), bbox_lodless, 0)
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
        lodless_geometries,
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
        let (reserved_columns, attribute_columns) = self.schema.column_lists()?;

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
            reserved_columns,
            default_geometry,
            bbox_column: "bbox".to_string(),
            sidecar_files: sidecars.to_vec(),
        })
    }
}
