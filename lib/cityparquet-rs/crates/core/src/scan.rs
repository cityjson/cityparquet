//! Pass 1: a single read-only scan over a [`Source`] that infers the
//! CityParquet schema (LoDs, attribute columns) and the dataset-level
//! metadata (CRS, transform, bbox) needed before writing any Parquet.
//!
//! This never retains WKB buffers or vertex data — it exists to answer "what
//! columns and metadata does this dataset need?" so pass 2 (the writer) can
//! allocate the right Arrow arrays up front.

use std::collections::{BTreeMap, BTreeSet};

use cityparquet_schema::{
    AttributeInferer, AttributeType, CITYPARQUET_VERSION, CityColumnEntry, CityMetadata,
    CityParquetError, CityParquetSchema, CrsState, ExtensionRegistry, GeoColumnEntry, GeoMetadata,
    GeometryEncoding, Lod, ModuleKey, ModuleKeyResolver, Result,
    SourceFormat as SchemaSourceFormat, geometry_column_name, normalise_attribute_name,
};

use cardinality_estimator::CardinalityEstimator;

use cjseq::GeometryType;

use crate::source::{Source, SourceFormat};
use cityparquet_schema::crs::AxisOrder;

use crate::wkb_write::{VertexPool, geometry_bbox};

/// A scalar string attribute column gets a bloom filter when its estimated
/// distinct count is at least this fraction of its non-null count
/// (specification `05-metadata.mdx`, "Recommended writer defaults").
pub const BLOOM_DISTINCT_RATIO: f64 = 0.2;

/// Distinct-value sketch for one attribute column's JSON-string values: the
/// non-null count and a HyperLogLog++ estimate (precision 12: exact below
/// 129 distinct values, about 1.6 % standard error above, at most ~3 KiB
/// whatever the row count).
#[derive(Default)]
struct StringCardinality {
    non_null: u64,
    distinct: CardinalityEstimator<str>,
}

impl StringCardinality {
    fn observe(&mut self, value: &str) {
        self.non_null += 1;
        self.distinct.insert(value);
    }

    /// `estimated_distinct >= 0.2 × non_null`, never for an all-null column.
    fn qualifies(&self) -> bool {
        self.non_null > 0
            && self.distinct.estimate() as f64 >= BLOOM_DISTINCT_RATIO * self.non_null as f64
    }
}

/// Outcome of scanning a [`Source`] once: the inferred schema plus the
/// dataset-level facts ([`CityParquetMetadata`] needs) that only a full scan
/// can answer.
#[derive(Debug, Clone)]
pub struct ScanResult {
    pub schema: CityParquetSchema,
    /// LoDs present, ascending; empty means the dataset has no analysis
    /// geometry (only `GeometryInstance`s, or no geometry at all), so its
    /// tables carry NO geometry column at all (spec "Levels of detail"; the
    /// writer prunes them). Every non-instance source geometry has a lod —
    /// [`scan`] rejects any that does not (§9, CityJSON 2.0 §3).
    pub lods: Vec<Lod>,
    pub object_count: usize,
    /// Union of every analysis geometry's bbox, `None` if none contributed one
    /// (`GeometryInstance`s produce no WKB, so they contribute nothing here).
    pub dataset_bbox: Option<[f64; 6]>,
    /// The dataset's reference system as the raw OGC CRS URL string (from
    /// CityJSON header metadata), before PROJJSON resolution.
    pub crs_url: Option<String>,
    /// The dataset's axis order, from the resolved CRS. Every coordinate the
    /// writer puts into WKB or into the `bbox` column is stored through it, so
    /// a latitude-first CRS (EPSG:4326, EPSG:6697 — what a Japanese national
    /// export declares) still yields GeoParquet-conformant
    /// `(x, y) = (longitude, latitude)` geometry.
    pub axis_order: AxisOrder,
    /// Whether the dataset's horizontal axes are angular (degrees), from the
    /// resolved CRS's declared units. Read by LoD0 synthesis, whose thresholds
    /// and geometric predicates are metre-valued throughout.
    pub horizontal_is_angular: bool,
    /// The dataset CRS as the footer's **tri-state** `crs`
    /// ([`CrsState`], spec §metadata "CRS rules"): the resolved PROJJSON when
    /// the source declared a CRS this writer could resolve, an explicit
    /// `null` ([`CrsState::Unknown`]) when it carries CRS-bearing coordinates
    /// but no resolvable CRS, and absent ([`CrsState::Unspecified`]) only when
    /// it carries no CRS-bearing coordinate at all. This is what `city.crs`
    /// and the `geo.columns[].crs` mirror carry.
    pub crs: CrsState,
    /// Set when [`Self::crs`] came out [`CrsState::Unknown`]: the human-facing
    /// explanation of *why* the file was written with an explicit null CRS
    /// (spec: a writer "SHOULD surface a conversion diagnostic"). Carried
    /// onward on [`crate::package::ConvertReport::crs_diagnostic`], which the
    /// CLI prints as a warning — the conversion itself succeeds.
    pub crs_diagnostic: Option<String>,
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
    /// When `Some`, the encoder synthesises an LoD0 footprint into the
    /// `geometry_lod0_0` column for any object lacking a source LoD0 (§9 "LoD0
    /// synthesis"), using these thresholds. Set by `convert` from
    /// `ConvertOptions::generate_lod0`; `scan` always leaves it `None`.
    pub synthesize_lod0: Option<crate::lod0::Lod0Options>,
    /// Per-[`ModuleKey`] LoDs present, ascending — the subset of `lods` that
    /// module's own rows actually populate (spec "object-table-schema": "a
    /// table carries exactly the LoD columns its data needs"). A module with
    /// no analysis geometry of its own (only `GeometryInstance`s, or none)
    /// still has an entry here, mapped to an empty `Vec` — it is resolved for
    /// every distinct `object_type` this scan encounters, whether or not that
    /// object carries geometry, so `crate::package::TableWriters` never needs
    /// to fall back to "module absent from the map means empty". Resolved
    /// with an EMPTY [`ExtensionRegistry`] — see [`scan`]'s doc comment.
    pub module_lods: BTreeMap<ModuleKey, Vec<Lod>>,
    /// Per-[`ModuleKey`] realised WKB type sets, mirroring
    /// [`Self::geoparquet_columns`] but broken out per module rather than
    /// dataset-wide: for each LoD a module's own rows populate, the actual
    /// WKB `geometry_types` seen (§metadata "city.columns entries") —
    /// Solid-family types included, unlike [`Self::geoparquet_columns`].
    /// `crate::package` unions this across every [`ModuleKey`] sharing one
    /// output FILE (the `Generics`/`CityObjectGroup` fold) to build that
    /// file's own `city.columns`/`geo` footer entries from ITS OWN realised
    /// type sets — never a dataset-wide union stamped onto every file (spec
    /// "The footer describes the file it lives in").
    pub module_geo: BTreeMap<ModuleKey, BTreeMap<Lod, BTreeSet<String>>>,
    /// The `String` attribute columns that get a bloom filter under an
    /// enabled [`crate::recipe::BloomPolicy`]: those whose estimated distinct
    /// count is at least [`BLOOM_DISTINCT_RATIO`] of their non-null count.
    /// Always a subset of `schema.attributes` of type `String`: a diverted
    /// name leaves it, both here and in
    /// [`Self::add_synthesized_lod0_column`]. Decided dataset-wide — a
    /// partitioned conversion stamps the whole-dataset set into every
    /// partition (`crate::package::CanonicalSchema::bloom_attributes`).
    pub bloom_attributes: BTreeSet<String>,
    source_format: SchemaSourceFormat,
    source_version: String,
}

/// The WKB type name `city.columns[].geometry_types` records for a source
/// geometry type — every non-instance source type has one (`crate::wkb_write`'s
/// encoder always produces exactly this type on the wire), unlike the
/// GeoParquet-legal subset [`is_geoparquet_legal_type`] tests against.
/// `Solid` encodes as `PolyhedralSurface Z` (1015); `MultiSolid`/
/// `CompositeSolid` as a `GeometryCollection Z` of them (see
/// `crate::wkb_write::geometry_to_wkb`) — both outside GeoParquet's
/// `[1001,1007]` subset (§7.2, §13.3), so a column carrying either is
/// CityParquet-only, declared in `city.columns` but never `geo.columns`.
fn city_geometry_type(thetype: &GeometryType) -> Option<&'static str> {
    match thetype {
        GeometryType::MultiPoint => Some("MultiPoint Z"),
        GeometryType::MultiLineString => Some("MultiLineString Z"),
        GeometryType::MultiSurface | GeometryType::CompositeSurface => Some("MultiPolygon Z"),
        GeometryType::Solid => Some("PolyhedralSurface Z"),
        GeometryType::MultiSolid | GeometryType::CompositeSolid => Some("GeometryCollection Z"),
        // A GeometryInstance produces no geometry column at all.
        GeometryType::GeometryInstance => None,
    }
}

/// Whether `type_name` (one of [`city_geometry_type`]'s outputs) falls inside
/// GeoParquet's `[1001,1007]` legal subset (§metadata "The declaration rule").
/// The Solid-family names are the only ones this encoder produces that fall
/// outside it.
fn is_geoparquet_legal_type(type_name: &str) -> bool {
    !matches!(type_name, "PolyhedralSurface Z" | "GeometryCollection Z")
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
///
/// [`ScanResult::module_lods`] is resolved with an EMPTY [`ExtensionRegistry`]
/// — matching `crate::package::extension_registry`'s present-day stub (no
/// Extension/ADE schema parsing yet, tracked separately: spec
/// `city.extensions` declaration storage). A genuine `+`-marked extension
/// class therefore hard-errors here exactly as it already does at write time
/// (see `extension_module_real_data.rs`'s
/// `unresolvable_extension_type_is_a_clean_schema_error`), just one pass
/// earlier — fail fast rather than fail during encode.
pub fn scan(source: &Source) -> Result<ScanResult> {
    let header = source.header();
    let mut inferer = AttributeInferer::default();
    let mut string_cardinality: BTreeMap<String, StringCardinality> = BTreeMap::new();
    let mut lod_strings: Vec<String> = Vec::new();
    let mut object_count = 0usize;
    let mut bbox_with_lod: Option<[f64; 6]> = None;
    let mut geometries_with_lod = 0usize;
    // Per-LoD WKB type set (§metadata "city.columns entries") — the real
    // types actually written to each LoD's geometry column, dataset-wide.
    let mut lod_geo: std::collections::BTreeMap<Lod, std::collections::BTreeSet<String>> =
        std::collections::BTreeMap::new();
    let mut resolver = ModuleKeyResolver::new(ExtensionRegistry::new());
    let mut module_lod_sets: BTreeMap<ModuleKey, BTreeSet<Lod>> = BTreeMap::new();
    let mut module_geo: BTreeMap<ModuleKey, BTreeMap<Lod, BTreeSet<String>>> = BTreeMap::new();
    // Whether the source carries any `GeometryInstance` — its `template.point`
    // (the placement anchor, in DATASET coordinates — see
    // `crate::encode::build_template`) is a CRS-bearing coordinate in its own
    // right (spec "CRS rules"), distinct from `geometries_with_lod` (which
    // only counts LoD-bearing analysis geometry).
    let mut has_geometry_instance = false;

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

    // Resolved before the scan proper, because both the coordinates it reads
    // and the diagnostics it may raise depend on the CRS.
    //
    // A degree-valued CRS is representable: the quantisation step is derived
    // per axis from the CRS's own declared units, so a degree axis gets a
    // degree-sized step. What is NOT representable is a CRS whose units the
    // encoder has no step for (feet, grads), and that is refused HERE — before
    // any output is touched — rather than in the writer, because a failure
    // written as a success is the worst outcome this pipeline can produce.
    //
    // The axis order comes from the same resolution: every coordinate this
    // scan turns into a `bbox` goes through a `VertexPool`, and so is stored
    // in WKB's (longitude, latitude) order, exactly as the geometry columns
    // are.
    // An identifier that does not resolve at all is NOT fatal — the spec's
    // "an unresolvable CRS is declared, not fatal" rule writes such a package
    // with an explicit `city.crs: null` further down. Only a CRS that DOES
    // resolve and then turns out to be unencodable is refused.
    let (axis_order, horizontal_is_angular) = match crs_url
        .as_deref()
        .and_then(|url| cityparquet_schema::crs::resolve_to_projjson(url).ok())
    {
        Some(projjson) => {
            let scale = cityparquet_schema::crs::axis_scale(&projjson)?;
            (
                AxisOrder::of(&projjson),
                scale[0] == cityparquet_schema::crs::NANO_DEGREE,
            )
        }
        None => (AxisOrder::default(), false),
    };

    for feature in source.features()? {
        let feature = feature?;
        let pool = VertexPool::new(&feature.vertices, &header.transform, axis_order);

        for (id, co) in &feature.city_objects {
            object_count += 1;
            let module_key = resolver.resolve(&co.thetype)?;
            // Every module actually encountered gets an entry, even one with
            // no analysis geometry at all — see the field's doc comment.
            module_lod_sets.entry(module_key.clone()).or_default();

            if let Some(attrs) = co.attributes.as_ref().and_then(|v| v.as_object()) {
                for (name, value) in attrs {
                    let name = normalise_attribute_name(name);
                    // Only JSON strings: a column that ends up `String`
                    // holds nothing else (numbers, booleans and arrays
                    // promote it to another type).
                    if let serde_json::Value::String(text) = value {
                        match string_cardinality.get_mut(&name) {
                            Some(sketch) => sketch.observe(text),
                            None => {
                                let mut sketch = StringCardinality::default();
                                sketch.observe(text);
                                string_cardinality.insert(name.clone(), sketch);
                            }
                        }
                    }
                    inferer.observe(&name, value);
                }
            }

            let Some(geoms) = &co.geometry else {
                continue;
            };
            for geom in geoms {
                let bbox = geometry_bbox(geom, &pool)?;
                match &geom.lod {
                    Some(lod) => {
                        let parsed = Lod::parse(lod).map_err(|_| {
                            CityParquetError::Lod(format!("object {id}: invalid lod {lod:?}"))
                        })?;
                        if !lod_strings.contains(lod) {
                            lod_strings.push(lod.clone());
                        }
                        geometries_with_lod += 1;
                        // Record this LoD's realised WKB type(s) — both
                        // dataset-wide and per-module (§metadata "The footer
                        // describes the file it lives in").
                        if let Some(t) = city_geometry_type(&geom.thetype) {
                            lod_geo.entry(parsed).or_default().insert(t.to_string());
                            module_geo
                                .entry(module_key.clone())
                                .or_default()
                                .entry(parsed)
                                .or_default()
                                .insert(t.to_string());
                        }
                        if let Some(bbox) = bbox {
                            union_bbox(&mut bbox_with_lod, bbox);
                        }
                        module_lod_sets
                            .entry(module_key.clone())
                            .or_default()
                            .insert(parsed);
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
                        has_geometry_instance = true;
                    }
                }
            }
        }
    }

    // A dataset with at least one LoD-bearing geometry uses per-LoD columns
    // (every non-instance geometry now has a lod — the loop above rejected any
    // that did not). A dataset with none — only `GeometryInstance`s, or an
    // attributes-only dataset with no geometry at all — has no analysis
    // geometry, hence no LoDs and no dataset bbox; its tables carry no
    // geometry column at all (the zero-analysis-geometry case, §9 / Appendix
    // B; the writer prunes the geometry/appearance columns).
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
    // type present is excluded entirely (§13.3, G1), ascending by LoD.
    let geoparquet_columns: Vec<(Lod, Vec<String>)> = lod_geo
        .into_iter()
        .filter(|(_, types)| types.iter().all(|t| is_geoparquet_legal_type(t)))
        .map(|(lod, types)| (lod, types.into_iter().collect()))
        .collect();

    let transform = serde_json::to_value(&header.transform)?;

    // `city.other.source_metadata` is the source header `metadata` VERBATIM.
    // An operator-supplied CRS is injected into the in-memory header so this
    // scan can resolve it (see `Source::set_reference_system`), but it is not
    // something the source carried, so it is removed again here — otherwise
    // the passthrough would assert the SOURCE declared a CRS it never had,
    // exactly the untruth `city.other.crs_source` exists to prevent. A header
    // whose ONLY metadata was that injected CRS carried no source metadata at
    // all, so nothing is written for it.
    let source_metadata = header
        .metadata
        .as_ref()
        .map(serde_json::to_value)
        .transpose()?
        .and_then(|value| {
            if !source.crs_is_operator_supplied() {
                return Some(value);
            }
            let mut map = match value {
                serde_json::Value::Object(map) => map,
                other => return Some(other),
            };
            map.remove("referenceSystem");
            (!map.is_empty()).then_some(serde_json::Value::Object(map))
        });

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

    // Resolve the source CRS to PROJJSON once (§metadata "CRS rules") — used
    // for both the per-column `geo`/`city` CRS and the geoarrow.wkb field
    // extension.
    //
    // An unresolvable CRS is DECLARED, not fatal (spec "CRS rules": "An
    // unresolvable CRS is declared, not fatal", which amends the earlier
    // draft's hard conversion error). Two shapes reach the same place: a
    // source with no CRS identifier at all, and one whose identifier this
    // writer cannot resolve to PROJJSON. When such a source carries any
    // CRS-bearing coordinate (analysis geometry, or a `GeometryInstance`
    // template placement point — `bbox` and an address `location` are the
    // other two named sources, tracked once those land) the footer says so
    // explicitly, with `city.crs: null`. Never an ABSENT key: per GeoParquet
    // absence asserts OGC:CRS84, which would silently mis-georeference a
    // projected national city model — the very failure the old hard error
    // existed to prevent, now prevented by declaring the truth instead.
    //
    // A degree-valued CRS remains a hard error (above): that is not "we
    // cannot name the CRS" but "this writer cannot represent these
    // coordinates at all".
    let mut crs_diagnostic = None;
    let resolved = match crs_url.as_deref() {
        Some(url) => match cityparquet_schema::crs::resolve_to_projjson(url) {
            Ok(projjson) => Some(projjson),
            Err(e) => {
                crs_diagnostic = Some(format!(
                    "source CRS {url:?} could not be resolved to PROJJSON ({e}); \
                     `city.crs` is written as an explicit null (CRS unknown) and the \
                     coordinates carry no georeference — supply the CRS explicitly to \
                     georeference them"
                ));
                None
            }
        },
        None => None,
    };
    let has_crs_bearing_coordinate = geometries_with_lod > 0 || has_geometry_instance;
    let crs = CrsState::from_resolution(resolved, has_crs_bearing_coordinate);
    if crs.is_unknown() && crs_diagnostic.is_none() {
        crs_diagnostic = Some(
            "source carries a CRS-bearing coordinate (geometry, or a GeometryInstance \
             template placement) but declares no CRS; `city.crs` is written as an \
             explicit null (CRS unknown) and the coordinates carry no georeference — \
             supply the CRS explicitly to georeference them"
                .to_string(),
        );
    }
    // The converse never happens by construction, but the two fields are read
    // independently downstream (the report prints one, the footer writes the
    // other), so keep them from ever disagreeing.
    debug_assert_eq!(crs.is_unknown(), crs_diagnostic.is_some());

    // Divert an attribute whose (normalised) name collides with a realised
    // reserved/geometry column name into `other` rather than aborting the
    // whole conversion (§5.2, G12). Divertedness is schema-relative — it
    // depends on the final `lods` — so this runs only after the LoDs are
    // settled. `CityParquetSchema::validate` stays strict; scan simply never
    // hands it a colliding attribute (the two share one reserved-name
    // definition).
    //
    // `other` is ITSELF the divert target, so an attribute literally named
    // `other` has nowhere further to divert to (diverting it into the very
    // column named after it would be circular) and is rejected outright —
    // the same self-collision rule `other_attributes` used to get when IT
    // was the divert target, inherited unchanged. Every OTHER reserved-name
    // collision still diverts into `other` as usual.
    let reserved = cityparquet_schema::model::reserved_and_geometry_column_names(&lods);
    let mut attributes = Vec::new();
    let mut diverted_attribute_names = BTreeSet::new();
    for (name, ty) in inferer.finish() {
        if name == "other" {
            return Err(CityParquetError::Schema(format!(
                "attribute column '{name}' collides with a reserved or geometry column name"
            )));
        }
        if reserved.contains(&name) {
            diverted_attribute_names.insert(name);
        } else {
            attributes.push((name, ty));
        }
    }

    // The high-cardinality rule, decided on the FINAL attribute set: a
    // diverted name has no column, and a column that inferred any type but
    // `String` (a Date or Timestamp, a Json mix) is never a candidate.
    let bloom_attributes: BTreeSet<String> = attributes
        .iter()
        .filter(|(name, ty)| {
            *ty == AttributeType::String
                && string_cardinality
                    .get(name)
                    .is_some_and(StringCardinality::qualifies)
        })
        .map(|(name, _)| name.clone())
        .collect();

    let schema = CityParquetSchema {
        lods: lods.clone(),
        // Exactly the LoDs `geo` will declare, so the GEOMETRY annotation and
        // the `geo` declaration can never disagree about which columns a
        // GeoParquet reader may touch.
        geoparquet_lods: geoparquet_columns.iter().map(|(lod, _)| *lod).collect(),
        attributes,
        // The `geoarrow.wkb` field extension can only carry a CRS it can
        // name, so both no-CRS states collapse to `None` here — the
        // three-way distinction lives in the footer's `city`/`geo` objects,
        // which is where a reader looks for it.
        crs: crs.known().cloned(),
    };

    let module_lods: BTreeMap<ModuleKey, Vec<Lod>> = module_lod_sets
        .into_iter()
        .map(|(key, set)| (key, set.into_iter().collect()))
        .collect();

    Ok(ScanResult {
        schema,
        lods,
        object_count,
        dataset_bbox,
        crs_url,
        axis_order,
        horizontal_is_angular,
        crs,
        crs_diagnostic,
        transform,
        extensions: header.extensions.clone(),
        source_metadata,
        appearance_defaults,
        diverted_attribute_names,
        geoparquet_columns,
        synthesize_lod0: None,
        module_lods,
        module_geo,
        bloom_attributes,
        source_format: to_schema_source_format(source.format()),
        source_version: header.version.clone(),
    })
}

impl ScanResult {
    /// The GeoParquet-legal geometry columns as `(column name, geometry_types)`
    /// pairs, ascending by LoD (§13.3, G1) — the writer declares exactly these
    /// in `geo.columns`, and the highest-LoD one is the `primary_column`.
    pub fn geoparquet_geo_columns(&self) -> Vec<(String, Vec<String>)> {
        self.geoparquet_columns
            .iter()
            .map(|(lod, types)| (geometry_column_name("geometry", lod), types.clone()))
            .collect()
    }

    /// Reserve a synthesised LoD0 footprint column (spec "LoD0 synthesis"):
    /// add LoD0 to `lods`/`schema` so the `geometry_lod0_0` column exists,
    /// and declare it GeoParquet-legal (`MultiPolygon Z`). No-op
    /// when the dataset has no analysis geometry (nothing to synthesise from) or
    /// already carries some `0.*` LoD. Because `geometry_lod0_0`/`material_lod0_0`/…
    /// become reserved once LoD0 is present (§5.2, G12), any attribute that
    /// now collides is diverted into `other` here, mirroring `scan`'s own
    /// diversion.
    ///
    /// Also extends `module_lods`: a writer synthesises a footprint from an
    /// object's own highest available LoD (§9), so any module that already
    /// has at least one LoD of its own is eligible for a synthesised one too
    /// — mirrored here rather than left to derive LoD0 only in the
    /// dataset-wide set. A module with NO LoDs of its own (no analysis
    /// geometry to flatten) gets none either, matching the whole-dataset
    /// no-op guard above.
    pub fn add_synthesized_lod0_column(&mut self) {
        // No-op when there is nothing to synthesise from, or the dataset
        // already has some `0.*` LoD.
        if self.lods.is_empty() || self.lods.iter().any(|l| l.major() == 0) {
            return;
        }
        let lod0 = Lod::parse("0").expect("literal 0 is a valid LoD");
        self.lods.push(lod0);
        self.lods.sort();
        self.lods.dedup();
        self.schema.lods = self.lods.clone();

        for (key, lods) in self.module_lods.iter_mut() {
            if !lods.is_empty() && !lods.iter().any(|l| l.major() == 0) {
                lods.push(lod0);
                lods.sort();
                lods.dedup();
                // Mirror into `module_geo` too, so this module's own
                // synthesised footprint is declared in ITS file's
                // `city.columns`/`geo` (never a dataset-wide stamp) — see
                // `Self::module_geo`'s doc comment.
                self.module_geo
                    .entry(key.clone())
                    .or_default()
                    .entry(lod0)
                    .or_insert_with(|| ["MultiPolygon Z".to_string()].into());
            }
        }

        // Divert attributes that collide with the now-reserved suffixed names.
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
        // A diverted attribute has no column any more, so no filter either.
        let attributes = &self.schema.attributes;
        self.bloom_attributes
            .retain(|name| attributes.iter().any(|(kept, _)| kept == name));

        // The synthesised footprint is a `MultiPolygon Z`, which is inside
        // GeoParquet's legal subset, so it is declared like any other legal
        // column — ascending by LoD.
        self.geoparquet_columns
            .push((lod0, vec!["MultiPolygon Z".to_string()]));
        self.geoparquet_columns.sort_by_key(|(lod, _)| *lod);
    }

    /// Build the DATASET-WIDE portion of `city` — the fields genuinely
    /// identical across every by-module table this scan feeds
    /// (`version`/`source_format`/`source_version`/`crs`/`extensions`/
    /// `appearance_defaults`/`attributes`/`other`) — everything EXCEPT
    /// `columns`/`primary_column`, which only exist per FILE and are added by
    /// [`crate::package`] once that file's own realised column set is known
    /// post-encode (spec "The footer describes the file it lives in — nothing
    /// wider").
    ///
    /// `other` is built from a small, fixed set of informational-only things
    /// (the source `transform`, the source's own header `metadata`) — never
    /// anything `export`/`decode` needs to do its job (spec "Informational
    /// only").
    pub fn base_city_metadata(&self) -> Result<CityMetadata> {
        // Only the attribute list is stored (§13.1): a reader recovers the
        // reserved columns as "everything not listed here". Dataset-wide
        // (not per-file): every module's table shares the same attribute
        // columns — only the geometry/appearance columns are pruned per
        // module (spec "object-table-schema").
        let (_reserved_columns, attributes) = self.schema.column_lists()?;

        let mut other = serde_json::Map::new();
        other.insert("transform".to_string(), self.transform.clone());
        if let Some(source_metadata) = &self.source_metadata {
            other.insert("source_metadata".to_string(), source_metadata.clone());
        }

        Ok(CityMetadata {
            version: CITYPARQUET_VERSION.to_string(),
            source_format: Some(self.source_format.clone()),
            source_version: Some(self.source_version.clone()),
            crs: self.crs.clone(),
            primary_column: None,
            columns: Vec::new(),
            attributes,
            extensions: self.extensions.clone(),
            appearance_defaults: self.appearance_defaults.clone(),
            other: (!other.is_empty()).then(|| serde_json::Value::Object(other)),
        })
    }
}

/// Build one file's `city.columns` (every geometry column present, Solid
/// included) plus its two INDEPENDENT primary-column selectors, from that
/// file's own realised `(Lod -> WKB type set)` map — never a dataset-wide
/// union stamped onto every file (spec "The footer describes the file it
/// lives in").
///
/// - `city_primary`: the highest LoD present, solids included.
/// - `geo_primary`/[`GeoMetadata`]: the highest **legal** `0.*`-family LoD if
///   any exist, else the highest legal LoD overall — computed independently
///   of `city_primary`, never derived from it (spec "Why `city.primary_column`
///   and `geo.primary_column` can differ"). `None` when the file has zero
///   legal columns (a solid-only table) — no `geo` key is written at all.
///
/// `crs` is the dataset's tri-state `city.crs`. `city.columns[].crs` is left
/// [`CrsState::Unspecified`] (per the spec, it "defaults to the file-level
/// `city.crs`", a sibling in the SAME object, so a CityParquet reader never
/// needs it repeated). `geo.columns[].crs`, however, MIRRORS the file-level
/// state verbatim — **including a `null`**: GeoParquet's own rule treats an
/// absent column `crs` as `OGC:CRS84`, and a GeoParquet-only consumer has no
/// access to the foreign `city` key to fall back to, so leaving it absent
/// over an unknown CRS would silently mis-georeference a projected city
/// model. GeoParquet defines `null` to mean exactly "unknown", so the mirror
/// is legal in all three states.
pub fn city_and_geo_for_file(
    per_lod: &std::collections::BTreeMap<Lod, BTreeSet<String>>,
    crs: &CrsState,
) -> (Vec<CityColumnEntry>, Option<String>, Option<GeoMetadata>) {
    if per_lod.is_empty() {
        return (Vec::new(), None, None);
    }

    let mut columns = Vec::with_capacity(per_lod.len());
    let mut geo_columns = std::collections::BTreeMap::new();
    let mut legal_lods: Vec<Lod> = Vec::new();

    for (lod, types) in per_lod {
        let name = geometry_column_name("geometry", lod);
        let geometry_types: Vec<String> = types.iter().cloned().collect();
        columns.push(CityColumnEntry::new(
            name.clone(),
            geometry_types.clone(),
            GeometryEncoding::Wkb,
        ));

        let legal = types.iter().all(|t| is_geoparquet_legal_type(t));
        if legal {
            legal_lods.push(*lod);
            geo_columns.insert(
                name,
                GeoColumnEntry {
                    encoding: "WKB".to_string(),
                    geometry_types,
                    crs: crs.clone(),
                    edges: Some("planar".to_string()),
                    bbox: None,
                    epoch: None,
                },
            );
        }
    }

    // city_primary: highest LoD present, solids included — independent of
    // legality.
    let city_primary = per_lod
        .keys()
        .next_back()
        .map(|lod| geometry_column_name("geometry", lod));

    // geo_primary: highest legal `0.*`-family LoD if any, else the highest
    // legal LoD overall.
    let geo = if legal_lods.is_empty() {
        None
    } else {
        let zero_family_highest = legal_lods.iter().copied().filter(|l| l.major() == 0).max();
        let primary = zero_family_highest.unwrap_or_else(|| {
            *legal_lods
                .iter()
                .max()
                .expect("legal_lods checked non-empty above")
        });
        Some(GeoMetadata {
            version: cityparquet_schema::GEOPARQUET_VERSION.to_string(),
            primary_column: geometry_column_name("geometry", &primary),
            columns: geo_columns,
        })
    };

    (columns, city_primary, geo)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// GeoParquet's `geometry_types` vocabulary is the `[1001,1007]` subset:
    /// a `MultiSurface` (CM type `"MultiPolygon Z"`) is inside it, while the
    /// Solid family — which CityParquet encodes as `PolyhedralSurface Z` and
    /// `GeometryCollection Z` — is not, so a column carrying either is
    /// declared in `city.columns` but never in `geo.columns`.
    #[test]
    fn only_the_geoparquet_subset_is_declared_legal() {
        assert!(is_geoparquet_legal_type("MultiPolygon Z"));
        assert!(is_geoparquet_legal_type("MultiPoint Z"));
        assert!(is_geoparquet_legal_type("MultiLineString Z"));
        assert!(!is_geoparquet_legal_type("PolyhedralSurface Z"));
        assert!(!is_geoparquet_legal_type("GeometryCollection Z"));
    }
}
