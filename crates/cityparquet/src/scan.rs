//! Pass 1: a single read-only scan over a [`Source`] that infers the
//! CityParquet schema (LoDs, attribute columns) and the dataset-level
//! metadata (CRS, transform, bbox) needed before writing any Parquet.
//!
//! This never retains WKB buffers or vertex data — it exists to answer "what
//! columns and metadata does this dataset need?" so pass 2 (the writer) can
//! allocate the right Arrow arrays up front.

use std::collections::BTreeSet;

use cityparquet_schema::{
    AttributeInferer, CITYPARQUET_VERSION, CityParquetError, CityParquetMetadata,
    CityParquetSchema, Lod, Result, SourceFormat as SchemaSourceFormat, footprint_lod,
    geometry_column_name, normalise_attribute_name,
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
    /// CityJSON header metadata), before PROJJSON resolution.
    pub crs_url: Option<String>,
    /// The dataset CRS resolved to **PROJJSON** (§13.3, G1), `None` when the
    /// source declared no CRS. This is what the `geo` metadata and the
    /// top-level `crs` mirror carry.
    pub crs: Option<serde_json::Value>,
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
    /// Source attribute names whose (normalised) name collides with a realised
    /// reserved/geometry column name (§5.2, G12). These get **no** attribute
    /// column; the writer diverts each object's value into the `other` column
    /// under `cityparquet:diverted_attributes` instead of aborting the whole
    /// conversion. Sorted for a deterministic diverted map.
    pub diverted_attribute_names: std::collections::BTreeSet<String>,
    /// The GeoParquet-legal geometry columns and their geometry types (§13.3,
    /// G1): one entry per LoD whose every geometry encodes to a WKB type in
    /// GeoParquet's `[1001,1007]` subset, ascending by LoD. A LoD with any
    /// `Solid`-family geometry (encoded as `PolyhedralSurfaceZ`, which
    /// GeoParquet cannot express) is CityParquet-only and is **absent** here,
    /// so it is never declared in `geo.columns` — declaring it would make the
    /// whole file unreadable to GeoParquet tools (§1.3). The `Vec<String>` is
    /// the GeoParquet `geometry_types` for that column (e.g. `["MultiPolygon Z"]`).
    pub geoparquet_columns: Vec<(Lod, Vec<String>)>,
    /// When `Some`, the encoder synthesises an LoD0 footprint into the primary
    /// `geometry` column for any object lacking a source LoD0 (§9 "LoD0
    /// synthesis"), using these thresholds. Set by `convert` from
    /// `ConvertOptions::generate_lod0`; `scan` always leaves it `None`.
    pub synthesize_lod0: Option<crate::lod0::Lod0Options>,
    source_format: SchemaSourceFormat,
    source_version: String,
}

/// The GeoParquet `geometry_types` string for a source geometry type, or `None`
/// when it has no GeoParquet-legal WKB encoding. `Solid`/`MultiSolid`/
/// `CompositeSolid` encode as `PolyhedralSurfaceZ` (1015) — or a
/// `GeometryCollectionZ` of them — which is outside GeoParquet's `[1001,1007]`
/// subset, so a column containing one is not a GeoParquet column (§7.2, §13.3).
fn geoparquet_geometry_type(thetype: &GeometryType) -> Option<&'static str> {
    match thetype {
        GeometryType::MultiPoint => Some("MultiPoint Z"),
        GeometryType::MultiLineString => Some("MultiLineString Z"),
        GeometryType::MultiSurface | GeometryType::CompositeSurface => Some("MultiPolygon Z"),
        GeometryType::Solid | GeometryType::MultiSolid | GeometryType::CompositeSolid => None,
        // A GeometryInstance produces no geometry column at all.
        GeometryType::GeometryInstance => None,
    }
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
    // Per-LoD GeoParquet legality: the set of GeoParquet geometry types seen,
    // and whether any geometry at that LoD is illegal (Solid-family).
    let mut lod_geo: std::collections::BTreeMap<Lod, (std::collections::BTreeSet<String>, bool)> =
        std::collections::BTreeMap::new();

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
                        let parsed = Lod::parse(lod).map_err(|_| {
                            CityParquetError::Lod(format!("object {id}: invalid lod {lod:?}"))
                        })?;
                        if !lod_strings.contains(lod) {
                            lod_strings.push(lod.clone());
                        }
                        geometries_with_lod += 1;
                        // Record this LoD's GeoParquet legality (§13.3, G1).
                        let entry = lod_geo.entry(parsed).or_default();
                        match geoparquet_geometry_type(&geom.thetype) {
                            Some(t) => {
                                entry.0.insert(t.to_string());
                            }
                            None => entry.1 = true,
                        }
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

    // The GeoParquet-legal columns: a LoD with any illegal (Solid-family)
    // geometry is excluded entirely (§13.3, G1), ascending by LoD.
    let geoparquet_columns: Vec<(Lod, Vec<String>)> = lod_geo
        .into_iter()
        .filter(|(_, (_, has_illegal))| !has_illegal)
        .map(|(lod, (types, _))| (lod, types.into_iter().collect()))
        .collect();

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

    // Resolve the source CRS to PROJJSON once (§13.3, G1) — used for both the
    // per-column `geo` CRS and the geoarrow.wkb field extension. An
    // unresolvable CRS is a hard error, never a silent omission.
    let crs = crs_url
        .as_deref()
        .map(cityparquet_schema::crs::resolve_to_projjson)
        .transpose()?;

    // Divert an attribute whose (normalised) name collides with a realised
    // reserved/geometry column name into `other` rather than aborting the whole
    // conversion (§5.2, G12). Divertedness is schema-relative — it depends on
    // the final `lods` — so this runs only after the LoDs are settled.
    // `CityParquetSchema::validate` stays strict; scan simply never hands it a
    // colliding attribute (the two share one reserved-name definition).
    let reserved = cityparquet_schema::model::reserved_and_geometry_column_names(&lods);
    let mut attributes = Vec::new();
    let mut diverted_attribute_names = BTreeSet::new();
    for (name, ty) in inferer.finish() {
        if reserved.contains(&name) {
            diverted_attribute_names.insert(name);
        } else {
            attributes.push((name, ty));
        }
    }

    let schema = CityParquetSchema {
        lods: lods.clone(),
        attributes,
        crs: crs.clone(),
    };

    Ok(ScanResult {
        schema,
        lods,
        object_count,
        dataset_bbox,
        crs_url,
        crs,
        transform,
        extensions: header.extensions.clone(),
        source_metadata,
        appearance_defaults,
        diverted_attribute_names,
        geoparquet_columns,
        synthesize_lod0: None,
        source_format: to_schema_source_format(source.format()),
        source_version: header.version.clone(),
    })
}

impl ScanResult {
    /// The GeoParquet-legal geometry columns as `(column name, geometry_types)`
    /// pairs, ascending by LoD (§13.3, G1) — the writer declares exactly these
    /// in `geo.columns`, and the highest-LoD one is the `primary_column`.
    pub fn geoparquet_geo_columns(&self) -> Vec<(String, Vec<String>)> {
        let fp = footprint_lod(&self.lods);
        self.geoparquet_columns
            .iter()
            .map(|(lod, types)| (geometry_column_name("geometry", lod, fp), types.clone()))
            .collect()
    }

    /// Reserve a synthesised LoD0 footprint column (§9 "LoD0 synthesis"): add
    /// LoD0 to `lods`/`schema` so the un-suffixed `geometry` column exists, and
    /// declare it GeoParquet-legal (`MultiPolygon Z`). No-op when the dataset has
    /// no analysis geometry (nothing to synthesise from) or already carries an
    /// LoD0 column. Because the bare `geometry`/`material`/… names become
    /// reserved once LoD0 is present (§5.2, G12), any attribute that now collides
    /// is diverted into `other` here, mirroring `scan`'s own diversion.
    pub fn add_synthesized_lod0_column(&mut self) {
        // No-op when there is nothing to synthesise from, or the dataset already
        // has a footprint (any `0.*` LoD — we use the highest, §9).
        if self.lods.is_empty() || footprint_lod(&self.lods).is_some() {
            return;
        }
        let lod0 = Lod::parse("0").expect("literal 0 is a valid LoD");
        self.lods.push(lod0);
        self.lods.sort();
        self.lods.dedup();
        self.schema.lods = self.lods.clone();

        // Divert attributes that collide with the now-reserved bare names.
        let reserved = cityparquet_schema::model::reserved_and_geometry_column_names(&self.lods);
        let mut kept = Vec::with_capacity(self.schema.attributes.len());
        for (name, ty) in std::mem::take(&mut self.schema.attributes) {
            if reserved.contains(&name) {
                self.diverted_attribute_names.insert(name);
            } else {
                kept.push((name, ty));
            }
        }
        self.schema.attributes = kept;

        // Declare the LoD0 footprint column as GeoParquet-legal, ascending.
        self.geoparquet_columns
            .push((lod0, vec!["MultiPolygon Z".to_string()]));
        self.geoparquet_columns.sort_by_key(|(lod, _)| *lod);
    }

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

        // Prefer the un-suffixed `geometry` (the highest 0.* footprint) as the
        // default; else the highest LoD present; else the plain `geometry`
        // fallback (zero-analysis-geometry case, §9).
        let fp = footprint_lod(&self.lods);
        let default_geometry = if fp.is_some() {
            "geometry".to_string()
        } else {
            match self.lods.last() {
                Some(highest) => geometry_column_name("geometry", highest, fp),
                None => "geometry".to_string(),
            }
        };

        Ok(CityParquetMetadata {
            cityparquet_version: CITYPARQUET_VERSION.to_string(),
            source_format: self.source_format,
            source_version: Some(self.source_version.clone()),
            crs: self.crs.clone(),
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
