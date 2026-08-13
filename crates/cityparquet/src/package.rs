//! End-to-end package conversion: CityJSON/CityJSONSeq in, a CityParquet
//! package directory out.
//!
//! Wires the three passes already shipped by this crate — [`crate::scan`]
//! (schema + dataset metadata), [`crate::encode`] (the `RecordBatch` stream),
//! and [`crate::recipe`] (per-column `WriterProperties`) — into a single
//! [`convert`] call that writes one `<snake>.parquet` table per 1st-level
//! CityObject family plus the package-level `metadata.json` manifest.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use arrow_array::builder::BooleanBuilder;
use arrow_array::types::Int32Type;
use arrow_array::{Array, DictionaryArray, RecordBatch, StringArray};
use arrow_schema::Schema;
use arrow_select::filter::filter_record_batch;
use parquet::arrow::ArrowWriter;
use parquet::file::metadata::KeyValue;
use parquet::file::properties::WriterProperties;

use cityparquet_schema::{
    AttributeType, CityMetadata, CityParquetError, CityParquetSchema, ExtensionRegistry,
    GeometryEncoding, Lod, ModuleKey, ModuleKeyResolver, Result, geometry_column_name,
};
use cjseq::CityJSONFeature;

use crate::appearance::AppearanceInterner;
use crate::encode::{LocalDefs, encode, encode_buffered, rewrite_geometry_appearance};
use crate::lod0::Lod0Options;
use crate::order::feature_hilbert_key;
use crate::recipe::WriterRecipe;
use crate::scan::{ScanResult, city_and_geo_for_file, scan};
use crate::sidecar::{TemplateRow, write_materials, write_templates, write_textures};
use crate::source::Source;
use crate::stac::properties::PackageTables;
use crate::stac::{ItemOptions, build_item};
use crate::wkb_write::{VertexPool, geometry_to_wkb};

/// Compatibility-profile sidecar tables.
const MATERIALS_TABLE: &str = "materials.parquet";
const TEXTURES_TABLE: &str = "textures.parquet";
const TEMPLATES_TABLE: &str = "geometry_templates.parquet";
/// Files a by-type object table's derived name must never collide with —
/// every package sidecar/metadata file. Since `table_name_for_module` no
/// longer namespaces object tables under a `cityobjects_` prefix (Task 4),
/// this guard is the invariant that keeps a pathological object type from
/// shadowing a reserved file — enforced at
/// [`TableWriters::by_type_table_index`].
const RESERVED_PACKAGE_FILES: &[&str] = &[
    MATERIALS_TABLE,
    TEXTURES_TABLE,
    TEMPLATES_TABLE,
    "metadata.json",
];
/// Scratch directory a `convert` run writes every new file into before the
/// crash-safe commit swap (see [`commit_package`]) — hidden (dot-prefixed) so
/// it never shows up as a stray "extra file" to a casual directory listing,
/// and named distinctively enough that [`purge_stale_package_files`] (which
/// only ever removes `metadata.json` and direct `*.parquet` children) can
/// never mistake it for package output.
const TMP_DIR_NAME: &str = ".cityparquet-tmp";

/// Row-emission order for the main CityObject data tables.
///
/// `Source` streams features exactly as `Source::features()` yields them
/// (lexicographic for a whole CityJSON document — see `Source::open` —
/// or on-disk order for a CityJSONSeq stream), with no extra memory cost
/// beyond one feature at a time.
///
/// `Hilbert` reorders FEATURES (never splitting one feature's objects
/// across the reorder — see `crate::order`'s module doc and
/// `hilbert_ordered_features` below) by the Hilbert-curve index of each
/// feature's own bbox centroid, so spatially nearby features land in the
/// same or adjacent row groups and bbox row-group pruning
/// (`crate::reader::CityParquetReaderBuilder::with_bbox_row_groups`) skips
/// more of the file on a spatially-selective query. This buffers every
/// parsed feature in memory before encoding a single row — the same
/// full-load trade-off `crate::compare`'s comparator already makes,
/// documented rather than hidden; a national-scale external sort is out of
/// scope for this milestone (M6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RowOrder {
    #[default]
    Source,
    Hilbert,
}

/// Options controlling one end-to-end CityJSON/CityJSONSeq -> CityParquet
/// package conversion.
#[derive(Debug, Clone)]
pub struct ConvertOptions {
    pub input: PathBuf,
    pub output_dir: PathBuf,
    pub overwrite: bool,
    pub batch_size: usize,
    pub recipe: WriterRecipe,
    pub ordering: RowOrder,
    /// Write GeoParquet/GeoArrow self-description (the `geoarrow.wkb` field
    /// extension + the file-level `geo` key). OFF by default so DuckDB reads
    /// geometry columns as plain BLOB (works with `SELECT *` and the
    /// `three_d` extension's `ST_3DFromWKB(BLOB)` with zero setup); ON for
    /// GeoPandas/QGIS/GDAL interop.
    pub geoarrow: bool,
    /// Which physical Arrow encoding `geometry_lod*` columns use. `Wkb` (the
    /// default) is normative; `ArrowNative` is the experimental
    /// `arrow-native-type` branch encoding (nested Arrow `List`/`Struct`
    /// columns plus a `geometry_vertices_lod*` sibling, instead of a WKB
    /// `BLOB`) — see
    /// `docs/superpowers/specs/2026-07-25-arrow-native-geometry-design.md`.
    pub geometry_encoding: GeometryEncoding,
    /// Synthesise an LoD0 footprint into the `geometry_lod0_0` column when an
    /// object has no source LoD0 (§9 "LoD0 synthesis"). A synthesised footprint
    /// is marked in `geometry_properties` and exported with the canonical
    /// `"0.0"` LoD string (minor defaults to `0` per M1's canonicalisation).
    /// The reference **CLI enables this by default** (the writer's
    /// convenience for 2D consumers; `--no-lod0` disables it), but
    /// [`ConvertOptions::new`] leaves it **off** so a library round trip is
    /// source-faithful unless the caller opts in — synthesis is an additive
    /// enrichment, not part of losslessness.
    pub generate_lod0: bool,
    /// Thresholds for LoD0 synthesis (used only when `generate_lod0`).
    pub lod0: Lod0Options,
    /// An operator-supplied CRS (e.g. `"EPSG:25832"`) used ONLY when the source
    /// declares none. The spec's CRS rules forbid writing `city.crs` absent and
    /// forbid guessing; an explicit operator declaration is neither, so the
    /// conversion proceeds and `city.other.crs_source` records where the CRS
    /// came from. `None` (the default) leaves the hard failure in place.
    ///
    /// This field carries the VALUE only; it is validated here and it never
    /// decides what the footer says. [`convert`] applies it to the source it
    /// opens; a caller holding its own [`Source`] applies it with
    /// [`crate::source::Source::set_reference_system`] BEFORE
    /// [`convert_source`], so the scan resolves an ordinary CRS. Either way
    /// the `crs_source` provenance stamp is read from the source, so setting
    /// this field cannot by itself make the output claim anything.
    pub crs_override: Option<String>,
}

impl ConvertOptions {
    /// 4096-row batches, the default [`WriterRecipe`], [`RowOrder::Source`]
    /// emission order, no overwrite, and no GeoParquet/GeoArrow
    /// self-description — the sensible defaults for a first conversion of
    /// `input` into `output_dir`. Sidecars (`materials.parquet`,
    /// `textures.parquet`, `geometry_templates.parquet`) are written
    /// whenever the source has content for them (spec-alignment gap 19
    /// dropped the `Profile` choice this used to gate on).
    pub fn new(input: PathBuf, output_dir: PathBuf) -> Self {
        Self {
            input,
            output_dir,
            overwrite: false,
            batch_size: 4096,
            recipe: WriterRecipe::default(),
            ordering: RowOrder::default(),
            geoarrow: false,
            geometry_encoding: GeometryEncoding::default(),
            // Off here (source-faithful library default); the CLI turns it on.
            generate_lod0: false,
            lod0: Lod0Options::default(),
            crs_override: None,
        }
    }
}

/// Outcome of one [`convert`] call: how many `CityObject` rows were written,
/// which files make up the package, and the row-population edge cases
/// [`crate::encode::EncodeStats`] tracked along the way.
#[derive(Debug, Clone)]
pub struct ConvertReport {
    pub object_count: usize,
    pub files: Vec<PathBuf>,
    pub skipped_same_lod_geometries: usize,
    pub attribute_coercion_nulls: usize,
    /// Structurally degenerate rings the writer dropped at encode time.
    pub degenerate_rings_dropped: usize,
    /// Surfaces the writer dropped (exterior ring degenerate), with their
    /// semantics/material/texture entries realigned in the stored columns.
    pub degenerate_surfaces_dropped: usize,
    /// Rows written to `materials.parquet` (`0` for the Core profile, or a
    /// Compatibility dataset with no materials at all).
    pub materials_written: usize,
    /// Rows written to `textures.parquet` (see [`Self::materials_written`]).
    pub textures_written: usize,
    /// Rows written to `geometry_templates.parquet` (`0` for the Core
    /// profile, or a Compatibility dataset with no geometry templates at
    /// all).
    pub templates_written: usize,
}

fn err(msg: String) -> CityParquetError {
    CityParquetError::Schema(msg)
}

fn io_err(msg: String) -> CityParquetError {
    CityParquetError::Io(msg)
}

fn parquet_err(msg: String) -> CityParquetError {
    CityParquetError::Parquet(msg)
}

/// Removes every trace of a PRIOR `convert` run from `output_dir`, so an
/// overwrite can never leave a stale sidecar behind that the fresh
/// `metadata.json` no longer lists (e.g. a Compatibility convert's
/// `materials.parquet` surviving a subsequent Core convert into the same
/// directory).
///
/// The safety property this must hold is "delete only what a CityParquet
/// package writes": `metadata.json` by name, and every direct (non-recursive)
/// `*.parquet` child of `output_dir` — nothing else, so an unrelated file a
/// caller happens to keep alongside the package is left untouched (and the
/// hidden [`TMP_DIR_NAME`] scratch directory [`commit_package`] calls this
/// from is neither a file nor named `metadata.json`, so it is never a
/// candidate either). Called ONLY from [`commit_package`], at swap time —
/// after every fallible step of writing the NEW package (including
/// `metadata.json`) into the temp directory has already succeeded, so a
/// mid-encode (or mid-sidecar-write) failure never reaches this at all and
/// the previous package survives completely untouched (M5 debt item 5).
fn purge_stale_package_files(output_dir: &Path) -> Result<()> {
    let metadata_path = output_dir.join("metadata.json");
    if metadata_path.exists() {
        fs::remove_file(&metadata_path).map_err(|e| {
            io_err(format!(
                "cannot remove stale {}: {e}",
                metadata_path.display()
            ))
        })?;
    }
    let entries = fs::read_dir(output_dir).map_err(|e| {
        io_err(format!(
            "cannot read output directory {}: {e}",
            output_dir.display()
        ))
    })?;
    for entry in entries {
        let entry = entry.map_err(|e| io_err(format!("cannot read directory entry: {e}")))?;
        let path = entry.path();
        let is_parquet_file =
            path.is_file() && path.extension().and_then(|ext| ext.to_str()) == Some("parquet");
        if is_parquet_file {
            fs::remove_file(&path)
                .map_err(|e| io_err(format!("cannot remove stale {}: {e}", path.display())))?;
        }
    }
    Ok(())
}

/// The crash-safe swap itself (M5 debt item 5): purges whatever stale
/// package `output_dir` already held, then renames every file this run
/// produced from `tmp_dir` into `output_dir`. `files` are bare file names,
/// present in `tmp_dir` on entry and in `output_dir` on success. A
/// standalone, directly testable unit — the property `convert` relies on is
/// that EVERYTHING before this call (opening/scanning the source, encoding,
/// writing sidecars, writing `metadata.json`) happens entirely inside
/// `tmp_dir`, so any failure along the way returns before this function is
/// ever reached and `output_dir` is never touched at all; this function is
/// the only place `output_dir`'s old contents are ever removed.
///
/// Not atomic against a failure mid-swap itself, and the residual risk is
/// asymmetric: the purge runs BEFORE any rename, so a failure after this
/// function starts (a purge error partway through the old files, or a
/// `rename` error after some files already moved) leaves the OLD package
/// removed and the NEW one split between `output_dir` (the renames that
/// succeeded) and `tmp_dir` (everything not yet renamed) — at that point
/// the files still in `tmp_dir` are the only recoverable copy of the new
/// package. Every error this function returns therefore (a) names `tmp_dir`
/// as holding the recoverable remainder, and (b) obliges the caller
/// ([`convert`]) to PRESERVE `tmp_dir` — never delete it — so an operator
/// can finish the swap by hand. (`rename` within the same directory tree is
/// normally an in-place metadata operation on the local filesystems this
/// crate targets, so the window is narrow compared to the whole encode pass
/// it replaces; the main property the temp-then-swap design buys is that an
/// encode-time failure — the realistic, common failure mode — never reaches
/// this function at all and leaves the old package completely untouched.)
fn commit_package(tmp_dir: &Path, output_dir: &Path, files: &[String]) -> Result<()> {
    // From here on the old package is (partially) gone: every error must
    // point the operator at the recoverable remainder in `tmp_dir`.
    let recoverable = |detail: String| {
        io_err(format!(
            "{detail}; partial package recoverable at {}",
            tmp_dir.display()
        ))
    };
    purge_stale_package_files(output_dir).map_err(|e| recoverable(e.to_string()))?;
    for name in files {
        let from = tmp_dir.join(name);
        let to = output_dir.join(name);
        fs::rename(&from, &to).map_err(|e| {
            recoverable(format!(
                "cannot move {} into place at {}: {e}",
                from.display(),
                to.display()
            ))
        })?;
    }
    // Best-effort: every file this run wrote into `tmp_dir` was just renamed
    // out of it, so it should be empty — but a stray leftover (or the
    // directory having already vanished) must never fail the whole convert
    // over cleanup alone.
    let _ = fs::remove_dir(tmp_dir);
    Ok(())
}

/// Build one [`TemplateRow`] per entry in `templates.templates`, folding
/// their `material`/`texture` definitions into `interner` — the SAME
/// interner the main encode pass populated from `source`'s features — so
/// the materials/textures sidecars end up describing every definition in
/// the dataset, not just the ones a regular feature geometry reaches.
///
/// `id` is the template's position as a string, matching the main-table
/// `template.id` column (see `crate::encode`'s `build_template`). The
/// rewrite rules (drop-realignment, dataset-global rewrite) are IDENTICAL
/// to a regular feature geometry's — see [`rewrite_geometry_appearance`]'s
/// doc comment — because a template's `material`/`texture`/`semantics`
/// follow the exact same CityJSON shapes a feature geometry's do. The one
/// deliberate divergence: template rows also carry `"lod"` inside
/// `geometry_properties`. The main table encodes LoD in the geometry
/// COLUMN NAME (`geometry_lod2` etc.) so its properties JSON never needs
/// it; the templates sidecar has a single properties column, so the
/// template's `lod` would otherwise be lost.
///
/// `pub(crate)` so the sidecar round-trip test exercises THIS production
/// builder instead of a hand-built duplicate of its logic.
///
/// A template's own coordinates are template-LOCAL (CityJSON spec §3.4:
/// `vertices-templates` is not subject to the dataset transform), so they
/// are looked up through [`VertexPool::raw`] over `templates.vertices_templates`,
/// never the dataset's quantised [`VertexPool::new`].
pub(crate) fn build_template_rows(
    templates: &cjseq::GeometryTemplates,
    source: &Source,
    interner: &mut AppearanceInterner,
) -> Result<Vec<TemplateRow>> {
    let verts: Vec<Vec<f64>> = serde_json::from_value(templates.vertices_templates.clone())
        .map_err(|e| {
            err(format!(
                "invalid geometry-templates vertices-templates: {e}"
            ))
        })?;
    let pool = VertexPool::raw(&verts);

    // See `Source::doc_appearance`'s doc comment: the header's
    // `geometry_templates` still carry the RAW DOCUMENT's global
    // material/texture indices (cjseq's `get_metadata` slices/reindexes a
    // separate clone to build the header's own `appearance`, never the
    // templates it hands back) — so the raw document's own appearance, not
    // `header().appearance`, is the correct local defs array here.
    let doc_appearance = source.doc_appearance();
    let local_materials = doc_appearance
        .and_then(|a| a.materials.clone())
        .unwrap_or_default();
    let local_textures = doc_appearance
        .and_then(|a| a.textures.clone())
        .unwrap_or_default();
    let local_uvs = doc_appearance
        .and_then(|a| a.vertices_texture.clone())
        .unwrap_or_default();

    let mut rows = Vec::with_capacity(templates.templates.len());
    for (i, tpl) in templates.templates.iter().enumerate() {
        let outcome = geometry_to_wkb(tpl, &pool)?.ok_or_else(|| {
            err(format!(
                "geometry template {i}: produced no WKB (empty or fully degenerate boundaries)"
            ))
        })?;
        let defs = LocalDefs {
            materials: &local_materials,
            textures: &local_textures,
            uvs: &local_uvs,
        };
        let (material, texture, props) = rewrite_geometry_appearance(
            tpl,
            &outcome.dropped_surfaces,
            interner,
            &defs,
            &format!("geometry template {i}"),
        )?;
        // A template is a single geometry at a single LoD (spec
        // "geometry_templates.parquet"): its LoD picks which physical
        // per-LoD column set this row's data lands in — the sidecar's own
        // schema equivalent of the main table's `accumulate_geometry`
        // requiring a valid, parseable LoD for every stored geometry.
        let lod = tpl
            .lod
            .as_deref()
            .and_then(|s| Lod::parse(s).ok())
            .ok_or_else(|| {
                CityParquetError::Lod(format!(
                    "geometry template {i}: has no valid lod for a per-LoD sidecar column"
                ))
            })?;
        rows.push(TemplateRow {
            id: i.to_string(),
            lod,
            wkb: outcome.bytes,
            geometry_properties: Some(props.to_value()),
            material,
            texture,
        });
    }
    Ok(rows)
}

/// The pieces of [`ConvertReport`] [`write_package`] can compute on its own —
/// everything except `files`, which only gets its final `output_dir`-rooted
/// paths after [`commit_package`] has renamed them there.
struct WrittenPackage {
    /// Bare file names (no directory component), present in `tmp_dir` when
    /// this is returned and the caller's contract for what to hand
    /// [`commit_package`].
    file_names: Vec<String>,
    skipped_same_lod_geometries: usize,
    attribute_coercion_nulls: usize,
    degenerate_rings_dropped: usize,
    degenerate_surfaces_dropped: usize,
    materials_written: usize,
    textures_written: usize,
    templates_written: usize,
}

/// Buffers every feature `source` yields (RowOrder::Hilbert's documented
/// full-load trade-off — see [`RowOrder`]'s doc comment) and stably sorts
/// them by [`feature_hilbert_key`] of each feature's own vertex-pool
/// centroid against `scan_result.dataset_bbox`.
///
/// The ORDERING UNIT is the feature (never an individual `CityObject`): a
/// CityJSONFeature groups a parent and its children, which must stay
/// contiguous in the output exactly as the Source-order path already keeps
/// them (`crate::encode::BatchIter::advance` only ever moves to the next
/// feature once every object of the current one is pushed) — sorting whole
/// `CityJSONFeature`s, rather than re-deriving per-object bboxes and
/// somehow trying to interleave objects across features, is what preserves
/// that invariant for free.
///
/// A missing `dataset_bbox` (an empty dataset, or one with no geometry at
/// all) makes every feature's key `0` — the stable sort is then a no-op,
/// so `RowOrder::Hilbert` silently degrades to `RowOrder::Source`'s
/// output order rather than doing anything meaningless with an undefined
/// normalisation range.
fn hilbert_ordered_features(
    source: &Source,
    scan_result: &ScanResult,
) -> Result<Vec<CityJSONFeature>> {
    let dataset_bbox = scan_result.dataset_bbox.unwrap_or([0.0; 6]);
    let transform = &source.header().transform;
    let features: Vec<CityJSONFeature> = source.features()?.collect::<Result<Vec<_>>>()?;

    // Decorate-sort-undecorate: pairs each feature with its key up front so
    // the sort comparator itself never recomputes it, then `Vec::sort_by_key`
    // (a STABLE sort) reorders — features with equal keys (including the
    // shared key `0` for every feature with no vertices) keep their
    // original relative order, per this function's own doc comment.
    let mut keyed: Vec<(u32, CityJSONFeature)> = features
        .into_iter()
        .map(|f| {
            let key = feature_hilbert_key(&f.vertices, transform, &dataset_bbox);
            (key, f)
        })
        .collect();
    keyed.sort_by_key(|(key, _)| *key);
    Ok(keyed.into_iter().map(|(_, f)| f).collect())
}

/// The `<snake>.parquet` file name the by-module writer uses for a
/// [`ModuleKey`] (spec "By-module object-table layout" / "extensions"):
/// [`cityparquet_schema::module_file`] plus the `.parquet` extension.
/// Callers that open a writer for the result MUST check it against
/// [`RESERVED_PACKAGE_FILES`] first (see
/// [`TableWriters::by_type_table_index`]), since a pathological extension
/// module name could otherwise shadow a sidecar/metadata file.
fn table_name_for_module(key: &ModuleKey) -> String {
    format!("{}.parquet", cityparquet_schema::module_file(key))
}

/// Every by-module table FILE this run might open, mapped to the union of
/// its own rows' LoDs (spec "object-table-schema" — "a table carries exactly
/// the LoD columns its data needs"). Several [`ModuleKey`]s can legitimately
/// derive the SAME file (the documented `Generics`/`CityObjectGroup` fold —
/// see [`TableWriters::claimed_by`]'s doc comment), so this unions their
/// `module_lods` entries rather than assuming one key per file.
fn module_lods_by_file(
    module_lods: &std::collections::BTreeMap<ModuleKey, Vec<Lod>>,
) -> HashMap<String, Vec<Lod>> {
    let mut by_file: HashMap<String, std::collections::BTreeSet<Lod>> = HashMap::new();
    for (key, lods) in module_lods {
        by_file
            .entry(table_name_for_module(key))
            .or_default()
            .extend(lods.iter().copied());
    }
    by_file
        .into_iter()
        .map(|(file, set)| (file, set.into_iter().collect()))
        .collect()
}

/// Every by-module table FILE's own realised `(Lod -> WKB type set)` map,
/// mirroring [`module_lods_by_file`] but for [`crate::scan::ScanResult::module_geo`]
/// — unions every [`ModuleKey`]'s own map into its FILE's entry (the same
/// `Generics`/`CityObjectGroup` fold `module_lods_by_file` already performs),
/// so [`crate::scan::city_and_geo_for_file`] sees that file's OWN realised
/// types, never a dataset-wide stamp (spec "The footer describes the file it
/// lives in — nothing wider").
fn module_geo_by_file(
    module_geo: &std::collections::BTreeMap<
        ModuleKey,
        std::collections::BTreeMap<Lod, std::collections::BTreeSet<String>>,
    >,
) -> HashMap<String, std::collections::BTreeMap<Lod, std::collections::BTreeSet<String>>> {
    let mut by_file: HashMap<
        String,
        std::collections::BTreeMap<Lod, std::collections::BTreeSet<String>>,
    > = HashMap::new();
    for (key, per_lod) in module_geo {
        let entry = by_file.entry(table_name_for_module(key)).or_default();
        for (lod, types) in per_lod {
            entry.entry(*lod).or_default().extend(types.iter().cloned());
        }
    }
    by_file
}

/// Column names, in spec order, for a table whose own rows only need
/// `file_lods`: the fixed reserved names, then this table's own per-LoD
/// geometry/appearance columns (ascending) — none at all when `file_lods` is
/// empty (spec "object-table-schema": "a table whose objects have no
/// analysis geometry at all carries none of them" — no bare-name fallback,
/// unlike [`CityParquetSchema::to_arrow_schema`]'s dataset-wide
/// zero-analysis-geometry rendering, which is a DIFFERENT case this function
/// is never asked to reproduce — see [`TableWriters::projection_for`]'s doc
/// comment for why), then `template`, `other`, `other_attributes`, then the
/// dataset's attribute columns in scan order. Mirrors
/// `CityParquetSchema::to_arrow_schema`'s non-empty-lods field order exactly,
/// so every name here is guaranteed to resolve in the dataset-wide (wide)
/// rendered schema — including the `geometry_vertices_lod*` sibling
/// [`CityParquetSchema::to_arrow_schema_tagged`] adds right after each
/// `geometry_lod*` column under [`GeometryEncoding::ArrowNative`] (this
/// plan's Task 6): omitting it here would silently prune the arrow-native
/// geometry column's only vertex data out of every by-module table, leaving
/// `geometry_lod*` behind with indices into nothing.
fn module_column_names(
    file_lods: &[Lod],
    attributes: &[(String, AttributeType)],
    encoding: GeometryEncoding,
) -> Vec<String> {
    let mut names: Vec<String> = [
        "id",
        "feature_id",
        "object_type",
        "parents",
        "children",
        "children_roles",
        "address",
        "bbox",
    ]
    .into_iter()
    .map(str::to_string)
    .collect();
    for lod in file_lods {
        names.push(geometry_column_name("geometry", lod));
        if encoding == GeometryEncoding::ArrowNative {
            names.push(geometry_column_name("geometry_vertices", lod));
        }
        names.push(geometry_column_name("geometry_properties", lod));
        names.push(geometry_column_name("material", lod));
        names.push(geometry_column_name("texture", lod));
    }
    names.push("template".to_string());
    names.push("other".to_string());
    names.push("other_attributes".to_string());
    names.extend(attributes.iter().map(|(name, _)| name.clone()));
    names
}

/// `batch`'s `object_type` column downcast to its dictionary array and Utf8
/// dictionary values — the one shared decode both [`TableWriters::write_batch`]
/// and its tests need (`crate::encode::BatchBuilder` always dictionary-encodes
/// `object_type`; `crate::decode` downcasts it identically).
fn object_type_dictionary(
    batch: &RecordBatch,
) -> Result<(&DictionaryArray<Int32Type>, &StringArray)> {
    let column = batch
        .column_by_name("object_type")
        .ok_or_else(|| err("encoded batch is missing its 'object_type' column".to_string()))?;
    let dict = column
        .as_any()
        .downcast_ref::<DictionaryArray<Int32Type>>()
        .ok_or_else(|| err("'object_type' column is not a dictionary array".to_string()))?;
    let values = dict
        .values()
        .as_any()
        .downcast_ref::<StringArray>()
        .ok_or_else(|| err("'object_type' dictionary values are not Utf8".to_string()))?;
    Ok((dict, values))
}

/// Resolves every DISTINCT dictionary VALUE of `batch`'s `object_type`
/// column to its [`ModuleKey`] exactly once — via `resolver`, which itself
/// memoises by source-type string across the whole conversion (many
/// batches) — indexed by dictionary key so every later per-row lookup is a
/// plain `Vec` index, never a re-derivation from the string. `object_type`
/// stores the CityGML class name (spec "object_type vocabulary", gap 15);
/// [`cityparquet_schema::resolve_module_key`] recognises a core class by
/// either its CityJSON or CityGML spelling, so this is correct as-is.
fn resolve_dictionary_module_keys(
    values: &StringArray,
    resolver: &mut ModuleKeyResolver,
) -> Result<Vec<ModuleKey>> {
    (0..values.len())
        .map(|i| resolver.resolve(values.value(i)))
        .collect()
}

/// The main CityObject data tables a `convert` run writes: one writer per
/// distinct [`ModuleKey`] (spec "By-module object-table layout" —
/// `object_type` routed through [`cityparquet_schema::resolve_module_key`];
/// 2nd-level core types, and extension classes specialising an ancestor,
/// share their module's writer), opened LAZILY on that module's first row
/// (see [`Self::write_batch`]). Every table is opened against its OWN
/// pruned Arrow schema — the dataset-wide (wide) reserved/attribute columns
/// plus only the geometry/appearance columns that table's own rows need
/// (spec "object-table-schema": "a table carries exactly the LoD columns
/// its data needs") — computed once at [`Self::open_table`] time from
/// `module_lods_by_file`/`attributes`; only `props` is shared verbatim
/// across every writer. `order`/`index` together track FIRST-APPEARANCE
/// order across the whole call (not just within one batch), which becomes
/// `table_names` (passed to [`PackageTables::from_lists`], then the built
/// Item's `cityparquet-objects` asset order) verbatim once [`Self::finish`]
/// is called — unaffected by [`RowOrder`], which only reorders FEATURES
/// before encoding; partitioning by module happens strictly after.
struct TableWriters {
    tmp_dir: PathBuf,
    /// The dataset-wide (encode-pass) Arrow schema every batch handed to
    /// [`Self::write_batch`] conforms to — every table's own pruned schema is
    /// a column PROJECTION of this one (see [`Self::projection_for`]), never
    /// independently rendered, so a projected batch is always guaranteed to
    /// match the writer it is written to.
    wide_schema: Arc<Schema>,
    /// Each by-module table FILE's own LoD set — the union of every
    /// [`ModuleKey`] routed there (see `crate::package::module_lods_by_file`).
    /// Consulted whenever pruning runs (i.e. not `force_identity_projection`);
    /// a file absent from this map
    /// (no rows for it were seen during the dataset-wide scan — should not
    /// happen since `write_batch` only ever opens a table `crate::scan::scan`
    /// already resolved a [`ModuleKey`] for, but handled defensively) is
    /// treated as having no LoDs of its own.
    module_lods_by_file: HashMap<String, Vec<Lod>>,
    /// Each by-module table FILE's own realised WKB type sets, the
    /// `city_and_geo_for_file` (from `crate::scan`) input for that file — see
    /// [`module_geo_by_file`]. Consulted post-encode, in [`Self::finish`],
    /// exactly once per table (spec "The footer describes the file it lives
    /// in — nothing wider").
    module_geo_by_file:
        HashMap<String, std::collections::BTreeMap<Lod, std::collections::BTreeSet<String>>>,
    /// The dataset-wide portion of `city` (`version`/`source_format`/
    /// `source_version`/`crs`/`extensions`/`appearance_defaults`/
    /// `attributes`/`other`) — every table's footer starts from a clone of
    /// this, then [`Self::finish`] fills in `columns`/`primary_column` from
    /// that table's own realised column set.
    base_city: CityMetadata,
    /// The dataset's attribute columns (unchanged per module — only the
    /// geometry/appearance columns are pruned, spec "object-table-schema"),
    /// in scan order.
    attributes: Vec<(String, AttributeType)>,
    /// Test-only escape hatch forcing [`Self::projection_for`] to a full
    /// identity projection, bypassing per-module column pruning. The unit
    /// tests below drive `write_batch` with deliberately minimal synthetic
    /// batches (see [`object_type_only_batch`]) that carry none of the reserved
    /// columns `module_column_names` resolves against `wide_schema`, so pruning
    /// cannot run for them. Production ALWAYS prunes (`false`): a table carries
    /// exactly the geometry/appearance columns its own rows need, and a table
    /// whose objects have no analysis geometry carries none of them (spec
    /// "object-table-schema" / "Levels of detail") — including the dataset-wide
    /// zero-analysis-geometry case, where `module_column_names` (empty
    /// `file_lods`) correctly drops the bare
    /// `geometry`/`geometry_properties`/`material`/`texture` quartet.
    force_identity_projection: bool,
    props: WriterProperties,
    order: Vec<String>,
    index: HashMap<String, usize>,
    /// Which [`ModuleKey`] first claimed each by-module table FILE NAME.
    /// Two DIFFERENT `Core` keys sharing a file is always the documented,
    /// intentional `Generics`/`CityObjectGroup` fold (spec: "On
    /// `CityObjectGroup`") — `core_module_file`'s pinned table makes any
    /// OTHER core collision impossible, so [`Self::by_type_table_index`]
    /// allows any Core-vs-Core share. A collision involving an `Extension`
    /// key, though, is a genuine ambiguity (snake_case is not injective over
    /// arbitrary extension module names, and an extension module could
    /// collide with a core file) — that is a hard `Schema` error, never a
    /// silent merge of two distinct modules into one table.
    claimed_by: HashMap<String, ModuleKey>,
    /// The REAL [`GeometryEncoding`] `wide_schema`'s geometry columns were
    /// rendered under — [`Self::finish`] feeds it to
    /// [`crate::scan::city_and_geo_for_file`] so each table's
    /// `city.columns[].encoding` agrees with the physical schema, rather
    /// than the old hardcoded `"WKB"` regardless of caller (this plan's
    /// Task 2, step 4b).
    geometry_encoding: GeometryEncoding,
    writers: Vec<ArrowWriter<fs::File>>,
    /// Per-writer-index projection: `wide_schema` field indices selecting
    /// that table's own pruned column set, in that table's own column order
    /// — computed once at [`Self::open_table`] time and reused by every
    /// [`Self::write_batch`] call for that table (never recomputed per
    /// batch).
    projections: Vec<Vec<usize>>,
    /// Resolves `object_type` values to [`ModuleKey`]s, memoising by source
    /// type across every batch this run writes (see
    /// [`cityparquet_schema::ModuleKeyResolver`]).
    resolver: ModuleKeyResolver,
}

impl TableWriters {
    /// Opens nothing yet — tables are created lazily as new [`ModuleKey`]s
    /// are encountered (see [`Self::by_type_table_index`]). `extensions` is
    /// the source's parsed Extension/ADE declarations (spec "extensions");
    /// an empty [`ExtensionRegistry`] is legitimate for a source with none.
    /// `module_lods_by_file`/`attributes`/`force_identity_projection` drive the
    /// per-table column pruning; `module_geo_by_file`/`base_city` drive the
    /// per-table `city`/`geo` footer [`Self::finish`] builds — see the
    /// struct's own doc comment.
    #[allow(clippy::too_many_arguments)]
    fn new(
        tmp_dir: &Path,
        wide_schema: Arc<Schema>,
        props: WriterProperties,
        extensions: ExtensionRegistry,
        module_lods_by_file: HashMap<String, Vec<Lod>>,
        module_geo_by_file: HashMap<
            String,
            std::collections::BTreeMap<Lod, std::collections::BTreeSet<String>>,
        >,
        base_city: CityMetadata,
        attributes: Vec<(String, AttributeType)>,
        force_identity_projection: bool,
        geometry_encoding: GeometryEncoding,
    ) -> Result<Self> {
        Ok(Self {
            tmp_dir: tmp_dir.to_path_buf(),
            wide_schema,
            module_lods_by_file,
            module_geo_by_file,
            base_city,
            geometry_encoding,
            attributes,
            force_identity_projection,
            props,
            order: Vec::new(),
            index: HashMap::new(),
            claimed_by: HashMap::new(),
            writers: Vec::new(),
            projections: Vec::new(),
            resolver: ModuleKeyResolver::new(extensions),
        })
    }

    /// The `wide_schema` field indices selecting `name`'s own pruned column
    /// set, in that table's own spec order — see [`module_column_names`]. A
    /// geometry-less table (empty `file_lods`) therefore projects to NO
    /// geometry/appearance columns — whether other tables in the dataset carry
    /// geometry (ordinary per-module pruning) or none do (the dataset-wide
    /// zero-analysis-geometry case, spec "Levels of detail": such a table
    /// "carries no geometry column"). Only the test-only
    /// `force_identity_projection` escape (see that field's doc comment)
    /// bypasses pruning.
    fn projection_for(&self, name: &str) -> Result<Vec<usize>> {
        if self.force_identity_projection {
            return Ok((0..self.wide_schema.fields().len()).collect());
        }
        let empty = Vec::new();
        let file_lods = self.module_lods_by_file.get(name).unwrap_or(&empty);
        module_column_names(file_lods, &self.attributes, self.geometry_encoding)
            .iter()
            .map(|column| {
                self.wide_schema.index_of(column).map_err(|e| {
                    err(format!(
                        "table {name}: column {column:?} missing from the dataset-wide schema: {e}"
                    ))
                })
            })
            .collect()
    }

    fn open_table(&mut self, name: &str) -> Result<usize> {
        let path = self.tmp_dir.join(name);
        let file = fs::File::create(&path)
            .map_err(|e| io_err(format!("cannot create {}: {e}", path.display())))?;
        let projection = self.projection_for(name)?;
        let table_schema = Arc::new(
            self.wide_schema
                .project(&projection)
                .map_err(|e| err(format!("table {name}: cannot project its own schema: {e}")))?,
        );
        let writer = ArrowWriter::try_new(file, table_schema, Some(self.props.clone()))
            .map_err(|e| parquet_err(format!("cannot open parquet writer: {e}")))?;
        let idx = self.writers.len();
        self.writers.push(writer);
        self.projections.push(projection);
        self.order.push(name.to_string());
        self.index.insert(name.to_string(), idx);
        Ok(idx)
    }

    /// The writer index for `key`'s by-module table, opening it lazily on
    /// the module's first row and recording the module's CLAIM on the
    /// derived file name. The reserved-name guard runs first: a
    /// pathological extension module could otherwise derive a name that
    /// shadows a package sidecar/metadata file (see
    /// [`RESERVED_PACKAGE_FILES`]) — reject that as a clear error instead of
    /// silently overwriting the sidecar. See [`Self::claimed_by`] for the
    /// collision rule once past that guard.
    fn by_type_table_index(&mut self, key: &ModuleKey) -> Result<usize> {
        let name = table_name_for_module(key);
        if RESERVED_PACKAGE_FILES.contains(&name.as_str()) {
            return Err(err(format!(
                "module {key:?} maps to reserved package file {name:?}; rename the module"
            )));
        }
        match self.claimed_by.get(&name) {
            Some(claimant) if claimant == key => Ok(self.index[&name]),
            Some(ModuleKey::Core(_)) if matches!(key, ModuleKey::Core(_)) => {
                // The only way two DISTINCT Core keys can derive the same
                // file is the intentional Generics/CityObjectGroup fold —
                // see `Self::claimed_by`'s doc comment.
                Ok(self.index[&name])
            }
            Some(claimant) => Err(err(format!(
                "modules {claimant:?} and {key:?} both derive the table file '{name}': \
                 refusing to merge two distinct modules into one table"
            ))),
            None => {
                let idx = self.open_table(&name)?;
                self.claimed_by.insert(name, key.clone());
                Ok(idx)
            }
        }
    }

    /// Writes one encoded batch, partitioned by [`ModuleKey`] — one
    /// `filter_record_batch` call per distinct module present, each
    /// sub-batch going to that module's own (lazily opened) writer. A
    /// row's own `object_type` value is untouched by this: only which FILE
    /// it lands in is decided by its module, the `object_type` column
    /// itself still carries the row's real, literal type. Resolves each
    /// DISTINCT dictionary value's `ModuleKey` once (via `self.resolver`),
    /// then does one linear pass over rows comparing dictionary keys/
    /// `ModuleKey`s — never re-deriving a `ModuleKey` from a string per row.
    fn write_batch(&mut self, batch: &RecordBatch) -> Result<()> {
        let (dict, values) = object_type_dictionary(batch)?;
        let per_dict_key = resolve_dictionary_module_keys(values, &mut self.resolver)?;

        // Distinct ModuleKeys actually present in `batch`, first-appearance
        // order within this batch — mirrors the old `distinct_types_in_batch`
        // shape but keyed on `ModuleKey`, not a re-derived string.
        let mut order: Vec<ModuleKey> = Vec::new();
        for row in 0..batch.num_rows() {
            if dict.is_null(row) {
                // The schema declares `object_type` non-nullable — a null
                // here can only mean a corrupt/foreign batch; error loudly
                // rather than silently drop the row from every table.
                return Err(err(format!("row {row}: 'object_type' must not be null")));
            }
            let code = dict.keys().value(row) as usize;
            let key = &per_dict_key[code];
            if !order.iter().any(|k| k == key) {
                order.push(key.clone());
            }
        }

        for key in &order {
            let idx = self.by_type_table_index(key)?;
            let mut mask = BooleanBuilder::with_capacity(batch.num_rows());
            for row in 0..batch.num_rows() {
                let code = dict.keys().value(row) as usize;
                mask.append_value(&per_dict_key[code] == key);
            }
            let filtered = filter_record_batch(batch, &mask.finish())?;
            // Column projection (this table's own pruned schema) — rows
            // already filtered above; `self.projections[idx]` was computed
            // once at this table's `open_table` time, never per batch.
            let projected = filtered.project(&self.projections[idx])?;
            self.writers[idx]
                .write(&projected)
                .map_err(|e| parquet_err(format!("parquet write error: {e}")))?;
        }
        Ok(())
    }

    /// Builds and appends EACH table's own `city` (and, when it has any
    /// GeoParquet-legal column, `geo`) footer key-value metadata — genuinely
    /// per-file, computed here (post-encode) from that table's own realised
    /// `module_geo_by_file` entry via [`city_and_geo_for_file`], never a
    /// dataset-wide union stamped identically onto every file (spec "The
    /// footer describes the file it lives in — nothing wider"). No
    /// `sidecar_files` key any more (spec-alignment M3 dropped it — a reader
    /// lists the package directory, or reads the STAC Item's assets).
    /// Closes every writer afterwards. Returns [`Self::order`]: the bare file
    /// names in first-appearance order, ready to become `table_names` —
    /// [`PackageTables::from_lists`]' `tables` argument — verbatim.
    fn finish(mut self) -> Result<Vec<String>> {
        let empty = std::collections::BTreeMap::new();
        for (name, writer) in self.order.iter().zip(self.writers.iter_mut()) {
            let per_lod = self.module_geo_by_file.get(name).unwrap_or(&empty);
            let (columns, primary_column, geo) =
                city_and_geo_for_file(per_lod, self.base_city.crs.as_ref(), self.geometry_encoding);
            let mut city = self.base_city.clone();
            city.columns = columns;
            city.primary_column = primary_column;
            for (key, value) in city.to_key_values(geo.as_ref())? {
                writer.append_key_value_metadata(KeyValue::new(key, value));
            }
        }
        for writer in self.writers {
            writer
                .close()
                .map_err(|e| parquet_err(format!("cannot finalise parquet file: {e}")))?;
        }
        Ok(self.order)
    }
}

/// The `source`'s parsed Extension/ADE declarations, for [`ModuleKeyResolver`]
/// to route extension classes by (spec "extensions" — "The `ModuleKey`").
///
/// **Stub.** `source.header().extensions` only carries each extension's
/// `url`/`version` reference (`cjseq::CityJSON::extensions`), not the
/// per-class `module`/`parent` declarations an Extension/ADE schema document
/// itself defines — actually fetching and parsing those schema documents
/// (and the equivalent for a CityGML ADE identity) is the `city.extensions`
/// declaration-mapping work the spec describes, explicitly out of scope for
/// this change (a later task owns it). Until then this always returns an
/// empty [`ExtensionRegistry`], so a source with a genuine `+`-marked
/// extension type resolves via [`cityparquet_schema::resolve_module_key`]'s
/// hard-error path (spec: "A class with no resolvable `ModuleKey` ... is a
/// hard error") rather than being silently misfiled — every fixture this
/// crate round-trips today carries no extension types, so this is not yet
/// exercised end-to-end.
fn extension_registry(_source: &Source) -> ExtensionRegistry {
    ExtensionRegistry::new()
}

/// Validate an operator-supplied CRS ([`ConvertOptions::crs_override`]) before
/// any of it reaches the writer.
///
/// Accepted spellings are `EPSG:25832` and the bare `25832`; anything else is
/// refused rather than guessed at. A code naming a known **geographic**
/// (degree-valued) CRS is refused too: nothing in this pipeline reprojects,
/// and the CityGML reader quantises at a fixed 1 mm, so a degree-valued
/// override would silently destroy the coordinates. The known-geographic list
/// ([`cityparquet_schema::crs::is_geographic_epsg`], shared with the CityGML
/// `srsName` resolver) is common-but-not-exhaustive, the residual limitation
/// documented there. Refusing loudly is the same
/// stance the spec's CRS rules take on a source with no CRS at all — the
/// override exists to make a CRS resolvable, never to make a bad one
/// tolerable.
pub(crate) fn validate_crs_override(spec: &str) -> Result<()> {
    let code = spec.trim().trim_start_matches("EPSG:").trim();
    if code.is_empty() || !code.chars().all(|c| c.is_ascii_digit()) {
        return Err(CityParquetError::Schema(format!(
            "operator-supplied CRS {spec:?} is not an EPSG code \
             (expected \"EPSG:25832\" or \"25832\")"
        )));
    }
    if cityparquet_schema::crs::is_geographic_epsg(code) {
        return Err(CityParquetError::Schema(format!(
            "operator-supplied CRS {spec:?} is a geographic (degree-valued) CRS; this \
             writer never reprojects and quantises at millimetre scale, so a degree \
             coordinate would be destroyed — supply the projected CRS the coordinates \
             are actually in"
        )));
    }
    // Fail here rather than deep in the scan: an operator typo is worth a
    // message that names the flag.
    cityparquet_schema::crs::resolve_to_projjson(code)?;
    Ok(())
}

/// The footer's `city` object, plus the CRS-provenance stamp.
///
/// When the CRS came from an operator rather than the source, `city.other`
/// records `crs_source: "operator-supplied"`. `city.other` is free-form and
/// explicitly informational per the spec, so this cannot mislead a decoder —
/// but it does stop the output implying the SOURCE declared a CRS it never
/// carried.
///
/// The stamp is read from the SOURCE, never from
/// [`ConvertOptions::crs_override`]: the option is a value to validate and
/// apply, whereas only the source knows whether applying it did anything (it
/// is a no-op for a source that declares its own CRS). Keying off the option
/// would stamp a source-declared CRS as operator-supplied whenever a caller
/// set the option without the override taking effect.
fn city_metadata(scan_result: &ScanResult, source: &Source) -> Result<CityMetadata> {
    let mut meta = scan_result.base_city_metadata()?;
    if source.crs_is_operator_supplied() {
        let mut other = match meta.other.take() {
            Some(serde_json::Value::Object(map)) => map,
            _ => serde_json::Map::new(),
        };
        other.insert(
            "crs_source".to_string(),
            serde_json::Value::String("operator-supplied".to_string()),
        );
        meta.other = Some(serde_json::Value::Object(other));
    }
    Ok(meta)
}

/// Writes one full package (main table, Compatibility-profile sidecars,
/// `metadata.json` manifest) into `tmp_dir` — this is the entire body that
/// used to run directly against `opts.output_dir` before the M5 crash-safe-
/// overwrite fix. `opts.output_dir` itself is never touched here; the caller
/// ([`convert`]) is solely responsible for the [`commit_package`] swap once
/// this returns `Ok`, and for removing `tmp_dir` on `Err`.
fn write_package(
    opts: &ConvertOptions,
    source: &Source,
    scan_result: &ScanResult,
    arrow_schema: Arc<Schema>,
    props: WriterProperties,
    tmp_dir: &Path,
) -> Result<WrittenPackage> {
    let mut writers = TableWriters::new(
        tmp_dir,
        arrow_schema,
        props,
        extension_registry(source),
        module_lods_by_file(&scan_result.module_lods),
        module_geo_by_file(&scan_result.module_geo),
        city_metadata(scan_result, source)?,
        scan_result.schema.attributes.clone(),
        // Production always prunes: a geometry-less table (dataset-wide or
        // per-module) carries no geometry columns. Identity projection is a
        // test-only escape — see `TableWriters::force_identity_projection`.
        false,
        opts.geometry_encoding,
    )?;

    // `RowOrder::Hilbert` buffers every feature and re-sorts it BEFORE
    // handing it to `encode_buffered` (which shares `BatchIter`'s whole
    // batching loop with the `RowOrder::Source` path below — see
    // `crate::encode::FeatureStream`'s doc comment); `RowOrder::Source`
    // keeps the plain streaming `encode` entry point, unchanged. `TableWriters`
    // partitions strictly AFTER encode (see `TableWriters`'s doc comment), so
    // this composes with either ordering unchanged.
    let mut batches = match opts.ordering {
        RowOrder::Source => encode(source, scan_result, opts.batch_size, opts.geoarrow)?,
        RowOrder::Hilbert => {
            let features = hilbert_ordered_features(source, scan_result)?;
            encode_buffered(
                features,
                source.header(),
                scan_result,
                opts.batch_size,
                opts.geoarrow,
            )?
        }
    };
    for batch in batches.by_ref() {
        let batch = batch?;
        writers.write_batch(&batch)?;
    }
    // `by_ref()` above means `batches` is still ours to read stats/appearance
    // from — consuming it by value (e.g. plain `.collect()`) would have
    // dropped it (and its running totals) before we could ask.
    let encode_stats = batches.stats();

    // Sidecars are written while the main-table writer(s) are still open
    // (they are separate files, so nothing conflicts), because the footer's
    // `sidecar_files` entry below must record what was ACTUALLY written.
    // Sidecars are written whenever the source has content for them
    // (spec-alignment gap 19: the `Profile` choice this used to gate on is
    // gone — a writer no longer declares Core vs Compatibility up front).
    let mut sidecar_files_written: Vec<String> = Vec::new();
    let mut templates_written = 0usize;
    // Fold geometry-template appearance into the SAME interner the encode
    // pass populated BEFORE materials.parquet/textures.parquet are written,
    // so their totals include definitions reachable ONLY from a geometry
    // template (see `build_template_rows`).
    let template_rows = match source.header().geometry_templates.as_ref() {
        Some(templates) => build_template_rows(templates, source, batches.appearance_mut())?,
        None => Vec::new(),
    };

    let appearance = batches.appearance();

    let materials_path = tmp_dir.join(MATERIALS_TABLE);
    let materials_written = write_materials(&materials_path, appearance.materials())?;
    if materials_written > 0 {
        sidecar_files_written.push(MATERIALS_TABLE.to_string());
    }

    let textures_path = tmp_dir.join(TEXTURES_TABLE);
    let textures_written = write_textures(&textures_path, appearance.textures())?;
    if textures_written > 0 {
        sidecar_files_written.push(TEXTURES_TABLE.to_string());
    }

    if !template_rows.is_empty() {
        let templates_path = tmp_dir.join(TEMPLATES_TABLE);
        templates_written = write_templates(&templates_path, &template_rows)?;
        if templates_written > 0 {
            sidecar_files_written.push(TEMPLATES_TABLE.to_string());
        }
    }

    // Close every writer, appending each table's own `city`/`geo` footer —
    // see `TableWriters::finish`. `table_names` is every main-table file
    // this run actually opened, in first-appearance order: one
    // `<type>.parquet` per distinct family.
    let table_names = writers.finish()?;
    // M5 Codex review (Important finding 1): the by-type writer opens tables
    // LAZILY, on a family's first row (see `TableWriters::new`/
    // `by_type_table_index`) — an input that encodes to zero rows therefore
    // never opens ANY writer, and `finish` returns an empty `Vec`. Writing
    // that straight into `metadata.json` would produce a package with
    // `tables: []`, which `export` rejects outright
    // (`manifest.tables.is_empty()`).
    //
    // This used to paper over that by writing a standalone, empty, single
    // reserved-name fallback table so the package wasn't empty. Plan
    // decision (2026-07-21, mandatory-by-type-layout): a conversion that
    // encodes zero city objects is a hard error instead — there is no
    // reserved fallback table name now that the single-file layout is gone.
    // We gate on the actual encoded table set (`table_names.is_empty()`)
    // rather than `scan_result.object_count`: `table_names` is what `export`
    // requires to be non-empty (a committed package must have at least one
    // object table), and conversion runs two passes over the source — an
    // initial scan, then a separate encode that reopens the file (see the
    // `convert_source_impl` docstring and `source.rs`). Gating on the actual
    // encode result stays correct even if the source is mutated between
    // those two passes, whereas the scan count would not.
    if table_names.is_empty() {
        return Err(err(
            "input contains no city objects to convert; a CityParquet package must have at \
             least one object table"
                .to_string(),
        ));
    }

    let mut file_names = table_names.clone();
    file_names.extend(sidecar_files_written.iter().cloned());

    // `metadata.json` is a STAC Item (Plan 2b), built from the exact file
    // list this run just wrote — `PackageTables::from_lists` is the pure,
    // disk-free constructor for that, mirroring `PackageTables::open`'s
    // shape without reading anything back (the files exist now, but nothing
    // here needs to re-derive `tables`/`sidecar_files` from them). `id`
    // comes from `opts.output_dir`'s name, not `tmp_dir`'s — `tmp_dir` is the
    // hidden crash-safe scratch directory (see `TMP_DIR_NAME`), never the
    // package's real name. `datetime` is left `None` so `build_item`'s
    // resolution order (explicit -> source `referenceDate` -> conversion
    // timestamp) decides it; this writer has no explicit value to offer.
    let item_id = opts
        .output_dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "cityparquet".to_string());
    let item_tables = PackageTables::from_lists(tmp_dir, &table_names, &sidecar_files_written);
    let item = build_item(
        &item_tables,
        &ItemOptions {
            id: Some(item_id),
            datetime: None,
        },
    )?;
    let metadata_path = tmp_dir.join("metadata.json");
    fs::write(&metadata_path, serde_json::to_string_pretty(&item)?)
        .map_err(|e| io_err(format!("cannot write {}: {e}", metadata_path.display())))?;
    file_names.push("metadata.json".to_string());

    Ok(WrittenPackage {
        file_names,
        skipped_same_lod_geometries: encode_stats.skipped_same_lod_geometries,
        attribute_coercion_nulls: encode_stats.attribute_coercion_nulls,
        degenerate_rings_dropped: encode_stats.degenerate_rings_dropped,
        degenerate_surfaces_dropped: encode_stats.degenerate_surfaces_dropped,
        materials_written,
        textures_written,
        templates_written,
    })
}

/// Convert `opts.input` (CityJSON or CityJSONSeq) into a CityParquet package
/// directory at `opts.output_dir`: one scan pass to infer the schema and
/// dataset metadata, one encode pass streamed straight into an `ArrowWriter`
/// using `opts.recipe`'s per-column `WriterProperties`, then (Compatibility
/// profile only) folds `Source::header`'s `geometry_templates` into the SAME
/// [`crate::appearance::AppearanceInterner`] the encode pass built (template
/// definitions the encode pass never visits directly — see
/// [`crate::source::Source::doc_appearance`]) and writes the
/// `geometry_templates.parquet`/`materials.parquet`/`textures.parquet`
/// sidecars from it, then a `metadata.json` manifest alongside it all.
///
/// `opts.output_dir` is created if missing; if it already exists and is
/// non-empty, conversion fails unless `opts.overwrite` is set.
///
/// Crash-safe overwrite (M5 debt item 5): every new file this run produces
/// ([`write_package`]) is written into a hidden [`TMP_DIR_NAME`] scratch
/// directory under `opts.output_dir` FIRST; only once that has entirely
/// succeeded (including `metadata.json`) does [`commit_package`] purge the
/// old package and swap the new files into place. A failure at ANY point
/// before that swap — a bad input, a mid-encode error, a sidecar write
/// failure — therefore leaves a pre-existing package at `opts.output_dir`
/// completely untouched (the previous behaviour purged the old package
/// BEFORE encoding the new one, so a mid-encode failure destroyed the old
/// package and left no usable one behind at all).
pub fn convert(opts: &ConvertOptions) -> Result<ConvertReport> {
    let mut source = Source::open(&opts.input)?;
    // This entry point opens the source itself, so it must also apply
    // `crs_override` itself — a caller has no `Source` to apply it to. A
    // source that declares its own CRS is left alone (the call is a no-op),
    // and the provenance stamp follows the source, not the option, so nothing
    // is claimed for an override that did not take effect.
    if let Some(code) = &opts.crs_override {
        source.set_reference_system(code);
    }
    convert_source(&source, opts)
}

/// A schema (attribute columns + LoD set) computed once over a whole merged
/// dataset and stamped into every partition's scan so all partition packages
/// share ONE Parquet schema — without this a partition that happens to lack an
/// attribute (or an LoD) infers fewer columns than its siblings, and
/// `read_parquet('OUT/*/…')` across the partitions needs `union_by_name` or
/// errors. See [`convert_source_impl`].
#[derive(Debug, Clone)]
pub struct CanonicalSchema {
    pub schema: CityParquetSchema,
    pub lods: Vec<Lod>,
    /// The whole-merged-dataset per-[`ModuleKey`] LoD sets (see
    /// [`crate::scan::ScanResult::module_lods`]), stamped into every
    /// partition's own `scan_result` the same way `lods` already is —
    /// without this, a partition whose LOCAL rows happen to need fewer LoDs
    /// for some module than another partition's do would prune that
    /// module's table to a NARROWER column set, and
    /// `read_parquet('OUT/*/<module>.parquet')` across partitions would face
    /// a schema mismatch for that module, exactly the failure mode the
    /// dataset-wide `lods`/`schema` canonicalisation already exists to avoid
    /// for the whole-table case.
    pub module_lods: std::collections::BTreeMap<ModuleKey, Vec<Lod>>,
    /// The attribute names the whole-dataset scan diverts into `other` (§5.2,
    /// G12). Divertedness is schema-relative (it depends on the canonical
    /// `lods` — e.g. an attribute named `geometry` is reserved once the dataset
    /// has an LoD0), so a partition MUST divert exactly this set, not its own
    /// local set: a partition lacking LoD0 would otherwise treat `geometry` as
    /// a legal attribute the canonical schema has no column for, and drop it.
    pub diverted_attribute_names: std::collections::BTreeSet<String>,
    /// The whole-dataset GeoParquet-legal geometry columns (§13.3), including a
    /// synthesised LoD0. Each partition must declare THIS set in its `geo`
    /// metadata, not its own local set: a partition lacking a legal LoD locally
    /// would otherwise omit the synthesised footprint from `geo` and disagree
    /// with its `default_geometry`.
    pub geoparquet_columns: Vec<(Lod, Vec<String>)>,
    /// The whole-merged-dataset per-[`ModuleKey`] realised WKB type sets (see
    /// [`crate::scan::ScanResult::module_geo`]), stamped into every
    /// partition's own `scan_result` the same way `module_lods` already is —
    /// without this, two partitions could disagree on a shared module's
    /// `city.columns`/`geo` even though `module_lods` agrees on its LoD set.
    pub module_geo: std::collections::BTreeMap<
        ModuleKey,
        std::collections::BTreeMap<Lod, std::collections::BTreeSet<String>>,
    >,
}

/// Convert an already-open `source` into a package at `opts.output_dir` —
/// everything [`convert`] does after `Source::open`, so a caller holding a
/// [`Source`] (the merge/partition pipeline, which builds an in-memory
/// [`Source::from_parts`]) reuses the identical scan → encode → sidecar →
/// atomic-swap path. `opts.input` is ignored.
pub fn convert_source(source: &Source, opts: &ConvertOptions) -> Result<ConvertReport> {
    convert_source_impl(source, opts, None)
}

/// [`convert_source`] with an optional canonical-schema override. When
/// `schema_override` is `Some`, the per-source scan still supplies this
/// partition's own `dataset_bbox`/`object_count`/stats, but its inferred
/// `schema`/`lods` are replaced by the canonical set so every partition writes
/// an identical column layout (features missing an override attribute or LoD
/// simply get null cells there, exactly as they would in the merged package).
pub(crate) fn convert_source_impl(
    source: &Source,
    opts: &ConvertOptions,
    schema_override: Option<&CanonicalSchema>,
) -> Result<ConvertReport> {
    if let Some(spec) = &opts.crs_override {
        validate_crs_override(spec)?;
    }
    fs::create_dir_all(&opts.output_dir).map_err(|e| {
        io_err(format!(
            "cannot create output directory {}: {e}",
            opts.output_dir.display()
        ))
    })?;
    let has_entries = fs::read_dir(&opts.output_dir)
        .map_err(|e| {
            io_err(format!(
                "cannot read output directory {}: {e}",
                opts.output_dir.display()
            ))
        })?
        .next()
        .is_some();
    if has_entries && !opts.overwrite {
        return Err(err(format!(
            "output directory {} already exists and is not empty (pass overwrite to replace it)",
            opts.output_dir.display()
        )));
    }

    let mut scan_result = scan(source, opts.geometry_encoding)?;
    if let Some(canon) = schema_override {
        scan_result.schema = canon.schema.clone();
        scan_result.lods = canon.lods.clone();
        // Per-module LoDs, canonicalised exactly like `lods` above — see
        // `CanonicalSchema::module_lods`'s doc comment.
        scan_result.module_lods = canon.module_lods.clone();
        // Divert the canonical set, not this partition's local one — otherwise a
        // partition whose local LoDs differ (e.g. no LoD0) would keep an
        // attribute the canonical schema diverted, and the encoder would neither
        // column it nor route it to `other`, losing the value (§5.2, G12).
        scan_result.diverted_attribute_names = canon.diverted_attribute_names.clone();
        // Likewise the GeoParquet-legal column set (§13.3): every partition must
        // declare the whole-dataset set (incl. a synthesised LoD0) so their `geo`
        // metadata agrees on `primary_column`/`columns` and matches
        // `default_geometry`.
        scan_result.geoparquet_columns = canon.geoparquet_columns.clone();
        // And the per-module realised type sets (spec "The footer describes
        // the file it lives in"): every partition's shared module must
        // report the SAME `city.columns`/`geo`, not just the same LoD set.
        scan_result.module_geo = canon.module_geo.clone();
    } else if opts.generate_lod0 {
        // Non-partitioned convert: reserve the synthesised LoD0 column here (a
        // partitioned run does this once on the whole-dataset scan, so the
        // canonical schema already carries it).
        scan_result.add_synthesized_lod0_column();
    }
    if opts.generate_lod0 {
        scan_result.synthesize_lod0 = Some(opts.lod0);
    }

    // The exact schema the writer is told to expect must be the exact schema
    // the encoded batches conform to (field metadata included) — both come
    // from this one `to_arrow_schema_tagged` call, never hand-duplicated —
    // and it must use the SAME `opts.geoarrow` flag `encode`/`encode_buffered`
    // feed their batch schema, or Arrow rejects the batches at write time.
    // `opts.geometry_encoding` also has to match what `encode`/`encode_buffered`
    // (called below) actually write: they read it back off `scan_result`
    // itself (`ScanResult::encoding`, set from this SAME `opts.geometry_encoding`
    // by the `scan` call above), so `RowWriter` can never pick a different
    // encoding than the schema declared here.
    let arrow_schema = Arc::new(
        scan_result
            .schema
            .to_arrow_schema_tagged(opts.geoarrow, opts.geometry_encoding)?,
    );
    // `city`/`geo` footer key-value metadata is NOT built here any more
    // (spec-alignment M3, per-module footer emission): each by-module
    // table's `columns`/`primary_column`/`geo` can only be known once that
    // table's own realised column set is settled, post-encode — see
    // `write_package` -> `TableWriters::finish`. `writer_properties` is now
    // purely the per-column compression/encoding recipe.
    // Same `opts.geometry_encoding` the arrow schema above was rendered
    // under, so the recipe's per-column rules are keyed to the physical
    // column paths this file will ACTUALLY carry (see
    // `WriterRecipe::writer_properties`).
    let props = opts
        .recipe
        .writer_properties(&scan_result.schema, opts.geometry_encoding)?;

    // Everything above is fallible but never touches `opts.output_dir` at
    // all, so none of it needs any cleanup. From here on, every new file
    // goes into a fresh scratch directory instead — see this function's own
    // doc comment.
    let tmp_dir = opts.output_dir.join(TMP_DIR_NAME);
    if tmp_dir.exists() {
        // Leftover from a run that crashed before cleaning up after itself
        // (e.g. the process was killed between `write_package` succeeding
        // and `commit_package` finishing): start from a clean slate rather
        // than risk mixing files across unrelated runs.
        fs::remove_dir_all(&tmp_dir).map_err(|e| {
            io_err(format!(
                "cannot remove stale scratch directory {}: {e}",
                tmp_dir.display()
            ))
        })?;
    }
    fs::create_dir_all(&tmp_dir).map_err(|e| {
        io_err(format!(
            "cannot create scratch directory {}: {e}",
            tmp_dir.display()
        ))
    })?;

    let written = match write_package(opts, source, &scan_result, arrow_schema, props, &tmp_dir) {
        Ok(written) => written,
        Err(e) => {
            // Nothing in `opts.output_dir` was ever touched: only the
            // scratch directory needs cleaning up, best-effort.
            let _ = fs::remove_dir_all(&tmp_dir);
            return Err(e);
        }
    };

    // Deliberately NO tmp_dir cleanup on a commit_package failure — the
    // exact opposite of the write_package arm above. By the time
    // commit_package can fail, its purge has already begun removing the OLD
    // package, so the files still in tmp_dir are the only recoverable copy
    // of the NEW one; deleting them would destroy the last usable state. The
    // error itself names the recoverable tmp_dir path (see commit_package),
    // and the next `convert` into this directory clears the leftover scratch
    // dir before writing (the stale-tmp_dir removal above).
    commit_package(&tmp_dir, &opts.output_dir, &written.file_names)?;

    Ok(ConvertReport {
        object_count: scan_result.object_count,
        files: written
            .file_names
            .iter()
            .map(|name| opts.output_dir.join(name))
            .collect(),
        skipped_same_lod_geometries: written.skipped_same_lod_geometries,
        attribute_coercion_nulls: written.attribute_coercion_nulls,
        degenerate_rings_dropped: written.degenerate_rings_dropped,
        degenerate_surfaces_dropped: written.degenerate_surfaces_dropped,
        materials_written: written.materials_written,
        textures_written: written.textures_written,
        templates_written: written.templates_written,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use arrow_array::builder::StringDictionaryBuilder;
    use arrow_schema::{DataType, Field};
    use cityparquet_schema::{CityGmlModule, ExtensionClassDecl};

    /// A minimal one-column batch (`object_type` as dictionary-encoded
    /// Utf8, matching `crate::encode::BatchBuilder`'s real encoding) — all
    /// `TableWriters`' ByType bookkeeping needs, so these unit tests stay
    /// free of any hand-built CityJSON document.
    fn object_type_only_batch(types: &[&str]) -> RecordBatch {
        let mut builder: StringDictionaryBuilder<Int32Type> = StringDictionaryBuilder::new();
        for t in types {
            builder.append_value(t);
        }
        let array = builder.finish();
        let schema = Arc::new(Schema::new(vec![Field::new(
            "object_type",
            DataType::Dictionary(Box::new(DataType::Int32), Box::new(DataType::Utf8)),
            false,
        )]));
        RecordBatch::try_new(schema, vec![Arc::new(array)]).unwrap()
    }

    /// `force_identity_projection: true` — these tests' synthetic
    /// `object_type`-only batches (see [`object_type_only_batch`]) carry none
    /// of the reserved columns `module_column_names` would look for, so the
    /// per-module pruning path (which resolves those names against
    /// `wide_schema`) must stay off; `true` makes
    /// [`TableWriters::projection_for`] a plain identity projection,
    /// reproducing this module's pre-pruning behaviour exactly for every
    /// existing test below.
    fn writers_with(
        tmp: &std::path::Path,
        schema: Arc<Schema>,
        extensions: ExtensionRegistry,
    ) -> TableWriters {
        TableWriters::new(
            tmp,
            schema,
            WriterProperties::default(),
            extensions,
            HashMap::new(),
            HashMap::new(),
            CityMetadata::new(),
            Vec::new(),
            true,
            GeometryEncoding::Wkb,
        )
        .unwrap()
    }

    /// The `ModuleKey`-driven equivalent of the old lossy `table_name_for_type`
    /// collision test: since [`to_snake_case`](cityparquet_schema) is not
    /// injective over arbitrary extension module names, two DIFFERENT
    /// extension module names can still derive the same file
    /// (`"MyEnergy"` and `"My_energy"` both -> `my_energy.parquet`) — the
    /// writer bookkeeping must reject that as a `Schema` error naming both
    /// colliding `ModuleKey`s and the file, never silently merge two
    /// distinct modules into one table.
    #[test]
    fn by_type_write_rejects_two_extension_modules_colliding_on_snake_case() {
        let mut extensions = ExtensionRegistry::new();
        extensions.declare(
            "A",
            ExtensionClassDecl {
                module: Some("MyEnergy".to_string()),
                parent: None,
            },
        );
        extensions.declare(
            "B",
            ExtensionClassDecl {
                module: Some("My_energy".to_string()),
                parent: None,
            },
        );
        // Precondition the whole scenario rests on: the two module names
        // really do collide on the derived file name.
        assert_eq!(
            table_name_for_module(&ModuleKey::Extension("MyEnergy".to_string())),
            table_name_for_module(&ModuleKey::Extension("My_energy".to_string())),
            "fixture fact: 'MyEnergy' and 'My_energy' must derive the same table file name"
        );

        let tmp = tempfile::tempdir().unwrap();
        let batch = object_type_only_batch(&["+A", "+B"]);
        let mut writers = writers_with(tmp.path(), batch.schema(), extensions);

        let e = writers.write_batch(&batch).unwrap_err();
        assert!(
            matches!(e, CityParquetError::Schema(_)),
            "expected a Schema error, got {e:?}"
        );
        let msg = e.to_string();
        assert!(
            msg.contains("my_energy.parquet"),
            "the error must name the colliding file, got: {msg}"
        );

        // The SAME module re-appearing (across batches) is not a collision:
        // its claim matches, so writing proceeds.
        let mut ok_extensions = ExtensionRegistry::new();
        ok_extensions.declare(
            "A",
            ExtensionClassDecl {
                module: Some("MyEnergy".to_string()),
                parent: None,
            },
        );
        let mut ok_writers = writers_with(tmp.path(), batch.schema(), ok_extensions);
        ok_writers
            .write_batch(&object_type_only_batch(&["+A"]))
            .unwrap();
        ok_writers
            .write_batch(&object_type_only_batch(&["+A"]))
            .unwrap();
        let tables = ok_writers.finish().unwrap();
        assert_eq!(tables, vec!["my_energy.parquet".to_string()]);
    }

    /// Any two DISTINCT `Core` `ModuleKey`s sharing a file is always the
    /// documented `Generics`/`CityObjectGroup` fold — never a collision to
    /// reject, and never a silent merge of anything else, since
    /// `core_module_file`'s pinned table makes any other collision between
    /// two `Core` keys structurally impossible.
    #[test]
    fn by_type_write_allows_the_generics_city_object_group_fold_without_error() {
        let tmp = tempfile::tempdir().unwrap();
        let batch = object_type_only_batch(&["CityObjectGroup", "GenericOccupiedSpace"]);
        let mut writers = writers_with(tmp.path(), batch.schema(), ExtensionRegistry::new());
        writers.write_batch(&batch).unwrap();
        let tables = writers.finish().unwrap();
        assert_eq!(tables, vec!["generics.parquet".to_string()]);
    }

    /// The reserved-name guard inside [`TableWriters::by_type_table_index`]
    /// must actually fire when a module derives a [`RESERVED_PACKAGE_FILES`]
    /// name: an extension module literally named `Materials` snakes to
    /// `materials.parquet`, which IS reserved (it names the materials
    /// sidecar table), so writing it under by-module must be rejected before
    /// any writer for it is ever opened.
    #[test]
    fn by_type_write_rejects_a_module_deriving_a_reserved_package_file() {
        let mut extensions = ExtensionRegistry::new();
        extensions.declare(
            "Foo",
            ExtensionClassDecl {
                module: Some("Materials".to_string()),
                parent: None,
            },
        );
        // Precondition the whole scenario rests on: the module really does
        // derive a reserved file name.
        assert_eq!(
            table_name_for_module(&ModuleKey::Extension("Materials".to_string())),
            MATERIALS_TABLE,
            "fixture fact: module 'Materials' must derive the reserved materials table file name"
        );

        let tmp = tempfile::tempdir().unwrap();
        let batch = object_type_only_batch(&["+Foo"]);
        let mut writers = writers_with(tmp.path(), batch.schema(), extensions);

        let e = writers.write_batch(&batch).unwrap_err();
        assert!(
            matches!(e, CityParquetError::Schema(_)),
            "expected a Schema error, got {e:?}"
        );
        let msg = e.to_string();
        assert!(
            msg.contains("materials.parquet"),
            "the error must name the reserved file it collides with, got: {msg}"
        );
        assert!(
            !msg.contains("both derive the table file"),
            "must be the reserved-file collision error, not the two-modules-same-name error, \
             got: {msg}"
        );

        // No file was ever created for the rejected module — the guard runs
        // before `open_table`, so the reserved sidecar's name is never
        // claimed by a by-module writer.
        assert!(!tmp.path().join(MATERIALS_TABLE).exists());
    }

    /// An all-null `object_type` batch must be a hard error under
    /// by-module writing, not a silent drop — the schema declares the
    /// column non-nullable, so this is defence-in-depth against
    /// corrupt/foreign batches.
    #[test]
    fn write_batch_errors_on_a_null_object_type_instead_of_skipping_it() {
        let mut builder: StringDictionaryBuilder<Int32Type> = StringDictionaryBuilder::new();
        builder.append_null();
        builder.append_null();
        let array = builder.finish();
        let schema = Arc::new(Schema::new(vec![Field::new(
            "object_type",
            DataType::Dictionary(Box::new(DataType::Int32), Box::new(DataType::Utf8)),
            true, // nullable here so the corrupt batch can even be built
        )]));
        let batch = RecordBatch::try_new(schema, vec![Arc::new(array)]).unwrap();

        let tmp = tempfile::tempdir().unwrap();
        let mut writers = writers_with(tmp.path(), batch.schema(), ExtensionRegistry::new());
        let e = writers.write_batch(&batch).unwrap_err();
        assert!(
            matches!(e, CityParquetError::Schema(_)),
            "expected a Schema error, got {e:?}"
        );
        assert!(e.to_string().contains("must not be null"), "got: {e}");
    }

    /// [`table_name_for_module`] is [`cityparquet_schema::module_file`] plus
    /// `.parquet` — pinned as a pure-function unit test covering both a core
    /// module and an extension module.
    #[test]
    fn table_name_for_module_is_module_file_plus_extension() {
        assert_eq!(
            table_name_for_module(&ModuleKey::Core(CityGmlModule::Building)),
            "building.parquet"
        );
        assert_eq!(
            table_name_for_module(&ModuleKey::Core(CityGmlModule::WaterBody)),
            "water_body.parquet"
        );
        assert_eq!(
            table_name_for_module(&ModuleKey::Extension("Energy".to_string())),
            "energy.parquet"
        );
    }

    /// No core module's derived file name ever collides with a package
    /// sidecar/metadata file — proven for every file-bearing `CityGmlModule`
    /// variant so a future reserved file can't silently regress it.
    #[test]
    fn core_module_table_names_never_collide_with_reserved_package_files() {
        let core_modules = [
            CityGmlModule::Building,
            CityGmlModule::Bridge,
            CityGmlModule::Tunnel,
            CityGmlModule::Construction,
            CityGmlModule::Transportation,
            CityGmlModule::Vegetation,
            CityGmlModule::Relief,
            CityGmlModule::WaterBody,
            CityGmlModule::LandUse,
            CityGmlModule::CityFurniture,
            CityGmlModule::Generics,
            CityGmlModule::CityObjectGroup,
        ];
        for module in core_modules {
            let name = table_name_for_module(&ModuleKey::Core(module));
            assert!(
                !RESERVED_PACKAGE_FILES.contains(&name.as_str()),
                "core module {module:?} must never derive a reserved package file name, got \
                 {name:?}"
            );
        }
    }

    /// A [`TableWriters`]-level proof that partitioning routes by
    /// [`ModuleKey`], not by a per-row string re-derivation: `Road`,
    /// `Railway`, `Waterway`, and `Square` (the CityGML class name for
    /// CityJSON's `TransportSquare`, spec "object_type vocabulary") are 4
    /// distinct `object_type` values that all share the Transportation
    /// module, and must land in the SAME single table.
    #[test]
    fn write_batch_routes_every_transportation_type_into_one_table() {
        let tmp = tempfile::tempdir().unwrap();
        let batch = object_type_only_batch(&["Road", "Railway", "Waterway", "Square"]);
        let mut writers = writers_with(tmp.path(), batch.schema(), ExtensionRegistry::new());
        writers.write_batch(&batch).unwrap();
        let tables = writers.finish().unwrap();
        assert_eq!(tables, vec!["transportation.parquet".to_string()]);
    }

    /// Companion to the above: types belonging to DIFFERENT modules land in
    /// DIFFERENT tables, proving the `ModuleKey` partition actually
    /// discriminates rather than degenerating to one shared table for
    /// everything.
    #[test]
    fn write_batch_routes_different_modules_into_different_tables() {
        let tmp = tempfile::tempdir().unwrap();
        let batch = object_type_only_batch(&["TINRelief", "WaterBody", "CityFurniture"]);
        let mut writers = writers_with(tmp.path(), batch.schema(), ExtensionRegistry::new());
        writers.write_batch(&batch).unwrap();
        let tables: std::collections::HashSet<String> =
            writers.finish().unwrap().into_iter().collect();
        assert_eq!(
            tables,
            std::collections::HashSet::from([
                "relief.parquet".to_string(),
                "water_body.parquet".to_string(),
                "city_furniture.parquet".to_string(),
            ])
        );
    }

    /// M5 review follow-up: a mid-swap `rename` failure inside
    /// [`commit_package`] happens AFTER the purge has already removed the
    /// old package, so the files still sitting in `tmp_dir` are the ONLY
    /// recoverable copy of the new package — the error path must preserve
    /// `tmp_dir` (never delete it) and the error message must point an
    /// operator at it. Filesystem-mechanics unit test (no CityJSON
    /// involved): two real files in `tmp_dir`, a `files` list whose MIDDLE
    /// entry names a file that does not exist there, so the first rename
    /// succeeds and the second fails.
    #[test]
    fn commit_package_mid_swap_failure_preserves_the_tmp_dir_for_recovery() {
        let out = tempfile::tempdir().unwrap();
        // The old package the purge removes before the renames start.
        fs::write(out.path().join("metadata.json"), "old-manifest").unwrap();
        fs::write(out.path().join("stale.parquet"), "old-table").unwrap();

        let tmp = out.path().join(TMP_DIR_NAME);
        fs::create_dir_all(&tmp).unwrap();
        fs::write(tmp.join("a.parquet"), "new-a").unwrap();
        fs::write(tmp.join("b.parquet"), "new-b").unwrap();

        let files = [
            "a.parquet".to_string(),
            "missing.parquet".to_string(), // does not exist in tmp: rename fails here
            "b.parquet".to_string(),
        ];
        let err = commit_package(&tmp, out.path(), &files).unwrap_err();
        assert!(
            matches!(err, CityParquetError::Io(_)),
            "expected an Io error from the failed rename, got {err:?}"
        );
        let msg = err.to_string();
        assert!(
            msg.contains("partial package recoverable at")
                && msg.contains(&tmp.display().to_string()),
            "the error must tell the operator the tmp dir holds the recoverable remainder, \
             got: {msg}"
        );

        // The rename that already succeeded left its file in output_dir...
        assert_eq!(
            fs::read_to_string(out.path().join("a.parquet")).unwrap(),
            "new-a",
            "the successfully-renamed file must be in output_dir"
        );
        // ...and everything not yet renamed must still be in tmp_dir, which
        // itself must survive: it is the only recoverable copy.
        assert!(
            tmp.exists(),
            "tmp_dir must be preserved for manual recovery"
        );
        assert_eq!(
            fs::read_to_string(tmp.join("b.parquet")).unwrap(),
            "new-b",
            "the not-yet-renamed remainder must still be in tmp_dir"
        );
    }
}
