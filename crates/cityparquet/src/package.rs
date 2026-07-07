//! End-to-end package conversion: CityJSON/CityJSONSeq in, a CityParquet
//! package directory out.
//!
//! Wires the three passes already shipped by this crate — [`crate::scan`]
//! (schema + dataset metadata), [`crate::encode`] (the `RecordBatch` stream),
//! and [`crate::recipe`] (per-column `WriterProperties`) — into a single
//! [`convert`] call that writes `cityobjects.parquet` plus the package-level
//! `metadata.json` manifest.

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

use cityparquet_schema::{CITYPARQUET_VERSION, CityParquetError, PackageManifest, Profile, Result};
use cjseq::CityJSONFeature;

use crate::appearance::AppearanceInterner;
use crate::encode::{LocalDefs, encode, encode_buffered, rewrite_geometry_appearance};
use crate::order::feature_hilbert_key;
use crate::recipe::WriterRecipe;
use crate::scan::{ScanResult, scan};
use crate::sidecar::{TemplateRow, write_materials, write_templates, write_textures};
use crate::source::Source;
use crate::wkb_write::{VertexPool, geometry_to_wkb};

/// The one data table every profile writes.
const CITYOBJECTS_TABLE: &str = "cityobjects.parquet";
/// Compatibility-profile sidecar tables.
const MATERIALS_TABLE: &str = "materials.parquet";
const TEXTURES_TABLE: &str = "textures.parquet";
const TEMPLATES_TABLE: &str = "geometry_templates.parquet";
/// Scratch directory a `convert` run writes every new file into before the
/// crash-safe commit swap (see [`commit_package`]) — hidden (dot-prefixed) so
/// it never shows up as a stray "extra file" to a casual directory listing,
/// and named distinctively enough that [`purge_stale_package_files`] (which
/// only ever removes `metadata.json` and direct `*.parquet` children) can
/// never mistake it for package output.
const TMP_DIR_NAME: &str = ".cityparquet-tmp";

/// Row-emission order for the main `cityobjects.parquet` table.
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

/// Table layout for the main CityObject data (M5 task 5): [`Self::Single`]
/// writes every object into the one [`CITYOBJECTS_TABLE`], exactly as every
/// milestone before M5 did. [`Self::ByType`] instead writes one table PER
/// DISTINCT `object_type` value the dataset contains — file name
/// `cityobjects_<snake>.parquet`, `<snake>` being the type lower-cased with
/// every non-alphanumeric character replaced by `_`, and a leading `+`
/// (CityJSON extension object types) rewritten to the prefix `ext_` rather
/// than folded into an ugly leading underscore (e.g. `+Foo` becomes
/// `cityobjects_ext_foo.parquet`, never `cityobjects__foo.parquet`) — see
/// [`table_name_for_type`]. Every table this produces shares the IDENTICAL
/// Arrow schema (no per-type column pruning — a different, out-of-scope
/// experiment): only which ROWS land in which file differs, decided purely
/// by each row's own `object_type`. [`PackageManifest::tables`] lists every
/// file [`TableWriters`] actually opened, in first-appearance order (the
/// order distinct `object_type` values are first encountered in the encoded
/// row stream) — this is unaffected by [`RowOrder`], which only reorders
/// FEATURES before encoding; partitioning by type happens strictly after.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TableLayout {
    #[default]
    Single,
    ByType,
}

/// Options controlling one end-to-end CityJSON/CityJSONSeq -> CityParquet
/// package conversion.
#[derive(Debug, Clone)]
pub struct ConvertOptions {
    pub input: PathBuf,
    pub output_dir: PathBuf,
    pub profile: Profile,
    pub overwrite: bool,
    pub batch_size: usize,
    pub recipe: WriterRecipe,
    pub ordering: RowOrder,
    pub layout: TableLayout,
}

impl ConvertOptions {
    /// Core profile, 4096-row batches, the default [`WriterRecipe`],
    /// [`RowOrder::Source`] emission order, [`TableLayout::Single`], and no
    /// overwrite — the sensible defaults for a first conversion of `input`
    /// into `output_dir`.
    pub fn new(input: PathBuf, output_dir: PathBuf) -> Self {
        Self {
            input,
            output_dir,
            profile: Profile::Core,
            overwrite: false,
            batch_size: 4096,
            recipe: WriterRecipe::default(),
            ordering: RowOrder::default(),
            layout: TableLayout::default(),
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
            &outcome,
            interner,
            &defs,
            &format!("geometry template {i}"),
        )?;
        let mut geometry_properties: serde_json::Value = serde_json::from_str(&props)?;
        // The shared helper omits "lod" by main-table design (LoD lives in
        // the geometry column name there); the sidecar's single properties
        // column must carry it or the template's LoD is lost.
        if let (Some(obj), Some(lod)) = (geometry_properties.as_object_mut(), &tpl.lod) {
            obj.insert("lod".to_string(), serde_json::Value::String(lod.clone()));
        }
        rows.push(TemplateRow {
            id: i.to_string(),
            wkb: outcome.bytes,
            geometry_properties: Some(geometry_properties),
            material,
            texture,
            other: None,
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

/// The `cityobjects_<snake>.parquet` file name [`TableLayout::ByType`] writes
/// for `object_type` — see [`TableLayout::ByType`]'s own doc comment for the
/// exact rule (lower-case, non-alphanumeric -> `_`, a leading `+` becomes the
/// prefix `ext_` rather than folding into the snake-cased body).
fn table_name_for_type(object_type: &str) -> String {
    let (prefix, body) = match object_type.strip_prefix('+') {
        Some(rest) => ("ext_", rest),
        None => ("", object_type),
    };
    let snake: String = body
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    format!("cityobjects_{prefix}{snake}.parquet")
}

/// The boolean mask selecting exactly the rows of `batch` whose
/// `object_type` column equals `target` (`batch`'s `object_type` column is
/// always a dictionary-encoded Utf8 — see `crate::encode::BatchBuilder` and
/// `crate::decode`'s identical downcast). A row with a null `object_type` is
/// a schema violation (the column is non-nullable), so a null there is a
/// `Schema` error rather than a silently-false mask entry.
fn object_type_mask(batch: &RecordBatch, target: &str) -> Result<arrow_array::BooleanArray> {
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
    let mut mask = BooleanBuilder::with_capacity(batch.num_rows());
    for row in 0..batch.num_rows() {
        if dict.is_null(row) {
            return Err(err(format!("row {row}: 'object_type' must not be null")));
        }
        let key = dict.keys().value(row) as usize;
        mask.append_value(values.value(key) == target);
    }
    Ok(mask.finish())
}

/// Every distinct `object_type` string appearing in `batch`, in
/// first-appearance order WITHIN this batch (a plain linear scan — the
/// number of distinct types is always small, at most a few dozen even for
/// the richest real dataset, so an O(rows * types) scan costs nothing next
/// to the WKB/attribute work the same batch already went through).
fn distinct_types_in_batch(batch: &RecordBatch) -> Result<Vec<String>> {
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
    let mut order: Vec<String> = Vec::new();
    for row in 0..batch.num_rows() {
        if dict.is_null(row) {
            // Defence-in-depth, aligned with `object_type_mask`'s identical
            // guard: the schema declares `object_type` non-nullable (see
            // `cityparquet_schema::model`), so a null can only mean a
            // corrupt/foreign batch — error loudly rather than skip, or an
            // all-null batch would be silently dropped under ByType (no
            // distinct types found -> no writer ever sees its rows).
            return Err(err(format!("row {row}: 'object_type' must not be null")));
        }
        let key = dict.keys().value(row) as usize;
        let type_str = values.value(key);
        if !order.iter().any(|t| t == type_str) {
            order.push(type_str.to_string());
        }
    }
    Ok(order)
}

/// The main CityObject data table(s) a `convert` run writes: [`TableLayout::Single`]
/// opens exactly one writer up front for [`CITYOBJECTS_TABLE`];
/// [`TableLayout::ByType`] opens one writer per distinct `object_type`,
/// LAZILY on that type's first row (see [`Self::write_batch`]). Every table
/// shares the identical `schema`/`props` this was constructed with — only
/// which rows land in which file differs. `order`/`index` together track
/// FIRST-APPEARANCE order across the whole call (not just within one
/// batch), which becomes [`PackageManifest::tables`] verbatim once
/// [`Self::finish`] is called.
struct TableWriters {
    layout: TableLayout,
    tmp_dir: PathBuf,
    schema: Arc<Schema>,
    props: WriterProperties,
    order: Vec<String>,
    index: HashMap<String, usize>,
    /// Which `object_type` first claimed each ByType table FILE NAME.
    /// [`table_name_for_type`] is lossy (case-folding, `_`-folding, the
    /// `ext_` prefix), so two DISTINCT object types can derive the same
    /// file name (e.g. a literal type `"Ext_A"` and the extension type
    /// `"+A"` both become `cityobjects_ext_a.parquet`) — silently sharing
    /// the writer would merge them into one table, violating the
    /// one-table-per-type invariant. [`Self::by_type_table_index`] consults
    /// this to turn any such collision into a `Schema` error instead.
    claimed_by: HashMap<String, String>,
    writers: Vec<ArrowWriter<fs::File>>,
}

impl TableWriters {
    /// For [`TableLayout::Single`], opens [`CITYOBJECTS_TABLE`] immediately
    /// (matching every layout's writer being open-and-ready before the
    /// first `write_batch` call, pre-M5 behaviour preserved exactly). For
    /// [`TableLayout::ByType`], opens nothing yet — tables are created lazily
    /// as new `object_type` values are encountered.
    fn new(
        layout: TableLayout,
        tmp_dir: &Path,
        schema: Arc<Schema>,
        props: WriterProperties,
    ) -> Result<Self> {
        let mut this = Self {
            layout,
            tmp_dir: tmp_dir.to_path_buf(),
            schema,
            props,
            order: Vec::new(),
            index: HashMap::new(),
            claimed_by: HashMap::new(),
            writers: Vec::new(),
        };
        if layout == TableLayout::Single {
            this.open_table(CITYOBJECTS_TABLE)?;
        }
        Ok(this)
    }

    fn open_table(&mut self, name: &str) -> Result<usize> {
        let path = self.tmp_dir.join(name);
        let file = fs::File::create(&path)
            .map_err(|e| io_err(format!("cannot create {}: {e}", path.display())))?;
        let writer = ArrowWriter::try_new(file, Arc::clone(&self.schema), Some(self.props.clone()))
            .map_err(|e| parquet_err(format!("cannot open parquet writer: {e}")))?;
        let idx = self.writers.len();
        self.writers.push(writer);
        self.order.push(name.to_string());
        self.index.insert(name.to_string(), idx);
        Ok(idx)
    }

    /// The writer index for `object_type`'s ByType table, opening it lazily
    /// on the type's first row and recording the type's CLAIM on the derived
    /// file name. Because [`table_name_for_type`] is lossy, a DIFFERENT
    /// object type deriving an already-claimed name is a hard `Schema` error
    /// naming both types and the colliding file — never a silent merge of
    /// two distinct types into one table (see [`Self::claimed_by`]).
    fn by_type_table_index(&mut self, object_type: &str) -> Result<usize> {
        let name = table_name_for_type(object_type);
        match self.claimed_by.get(&name) {
            Some(claimant) if claimant == object_type => Ok(self.index[&name]),
            Some(claimant) => Err(err(format!(
                "object types '{claimant}' and '{object_type}' both derive the table file \
                 '{name}': refusing to merge two distinct object types into one table"
            ))),
            None => {
                let idx = self.open_table(&name)?;
                self.claimed_by.insert(name, object_type.to_string());
                Ok(idx)
            }
        }
    }

    /// Writes one encoded batch: handed straight to the (single, already
    /// open) writer under [`TableLayout::Single`], or partitioned by
    /// `object_type` — one `filter_record_batch` call per distinct type
    /// present, each sub-batch going to that type's own (lazily opened)
    /// writer — under [`TableLayout::ByType`].
    fn write_batch(&mut self, batch: &RecordBatch) -> Result<()> {
        match self.layout {
            TableLayout::Single => self.writers[0]
                .write(batch)
                .map_err(|e| parquet_err(format!("parquet write error: {e}"))),
            TableLayout::ByType => {
                for object_type in distinct_types_in_batch(batch)? {
                    let idx = self.by_type_table_index(&object_type)?;
                    let mask = object_type_mask(batch, &object_type)?;
                    let filtered = filter_record_batch(batch, &mask)?;
                    self.writers[idx]
                        .write(&filtered)
                        .map_err(|e| parquet_err(format!("parquet write error: {e}")))?;
                }
                Ok(())
            }
        }
    }

    /// Appends the (now-known-final) `sidecar_files` key-value entry to
    /// EVERY table's footer — not just one — so a reader opening any table
    /// this run produced (see `crate::export`'s multi-table read loop) sees
    /// the same `sidecar_files` list regardless of which one it opens first,
    /// then closes every writer. Returns [`Self::order`]: the bare file
    /// names in first-appearance order, ready to become
    /// [`PackageManifest::tables`] verbatim.
    fn finish(mut self, sidecar_files: &[String]) -> Result<Vec<String>> {
        let kv = serde_json::to_string(sidecar_files)?;
        for writer in &mut self.writers {
            writer
                .append_key_value_metadata(KeyValue::new("sidecar_files".to_string(), kv.clone()));
        }
        for writer in self.writers {
            writer
                .close()
                .map_err(|e| parquet_err(format!("cannot finalise parquet file: {e}")))?;
        }
        Ok(self.order)
    }
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
    let mut writers = TableWriters::new(opts.layout, tmp_dir, arrow_schema, props)?;

    // `RowOrder::Hilbert` buffers every feature and re-sorts it BEFORE
    // handing it to `encode_buffered` (which shares `BatchIter`'s whole
    // batching loop with the `RowOrder::Source` path below — see
    // `crate::encode::FeatureStream`'s doc comment); `RowOrder::Source`
    // keeps the plain streaming `encode` entry point, unchanged. `TableWriters`
    // partitions strictly AFTER encode (see `TableLayout`'s doc comment), so
    // this composes with either ordering unchanged.
    let mut batches = match opts.ordering {
        RowOrder::Source => encode(source, scan_result, opts.batch_size)?,
        RowOrder::Hilbert => {
            let features = hilbert_ordered_features(source, scan_result)?;
            encode_buffered(features, source.header(), scan_result, opts.batch_size)?
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
    let mut sidecar_files_written: Vec<String> = Vec::new();
    let mut materials_written = 0usize;
    let mut textures_written = 0usize;
    let mut templates_written = 0usize;
    if opts.profile == Profile::Compatibility {
        // Fold geometry-template appearance into the SAME interner the
        // encode pass populated BEFORE materials.parquet/textures.parquet
        // are written, so their totals include definitions reachable ONLY
        // from a geometry template (see `build_template_rows`).
        let template_rows = match source.header().geometry_templates.as_ref() {
            Some(templates) => build_template_rows(templates, source, batches.appearance_mut())?,
            None => Vec::new(),
        };

        let appearance = batches.appearance();

        let materials_path = tmp_dir.join(MATERIALS_TABLE);
        materials_written = write_materials(&materials_path, appearance.materials())?;
        if materials_written > 0 {
            sidecar_files_written.push(MATERIALS_TABLE.to_string());
        }

        let textures_path = tmp_dir.join(TEXTURES_TABLE);
        textures_written = write_textures(&textures_path, appearance.textures())?;
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
    }

    // Now that the actual sidecar list is known, record it in EVERY main
    // table's parquet footer (the pre-encode `WriterProperties` KV set
    // omitted the key entirely, so this cannot produce a duplicate; and even
    // against a foreign file that DID carry one, appended entries come after
    // the props entries in the footer and `CityParquetMetadata::from_key_values`
    // is last-wins), then close every writer — see `TableWriters::finish`.
    // `table_names` is every main-table file this run actually opened, in
    // first-appearance order: `[CITYOBJECTS_TABLE]` under `TableLayout::Single`,
    // one `cityobjects_<type>.parquet` per distinct `object_type` under
    // `TableLayout::ByType`.
    let table_names = writers.finish(&sidecar_files_written)?;

    let mut file_names = table_names.clone();
    file_names.extend(sidecar_files_written.iter().cloned());

    let manifest = PackageManifest {
        cityparquet_version: CITYPARQUET_VERSION.to_string(),
        profile: opts.profile,
        lods: scan_result.lods.iter().map(|lod| lod.to_string()).collect(),
        tables: table_names,
        sidecar_files: sidecar_files_written,
    };
    let metadata_path = tmp_dir.join("metadata.json");
    fs::write(&metadata_path, serde_json::to_string_pretty(&manifest)?)
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

    let source = Source::open(&opts.input)?;
    let scan_result = scan(&source)?;

    // `sidecar_files` is intentionally EXCLUDED from this pre-encode
    // key-value set (an empty list serialises to no key at all — see
    // `CityParquetMetadata::sidecar_files`): which sidecar files this run
    // actually produces (an empty sidecar is skipped, see `crate::sidecar`)
    // is only known after the encode pass below runs to completion. The
    // real, actually-written list is appended to the footer via
    // `ArrowWriter::append_key_value_metadata` after the sidecars are
    // written and before `writer.close()`, so the parquet footer and
    // `metadata.json` always agree.
    let metadata = scan_result.metadata(&[])?;
    // The exact schema the writer is told to expect must be the exact schema
    // the encoded batches conform to (field metadata included) — both come
    // from this one `to_arrow_schema()` call, never hand-duplicated.
    let arrow_schema = Arc::new(scan_result.schema.to_arrow_schema()?);
    let props = opts
        .recipe
        .writer_properties(&scan_result.schema, &metadata)?;

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

    let written = match write_package(opts, &source, &scan_result, arrow_schema, props, &tmp_dir) {
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

    /// M5 review follow-up (Important): [`table_name_for_type`] is lossy, so
    /// the literal type `"Ext_A"` and the extension type `"+A"` both derive
    /// `cityobjects_ext_a.parquet` — the writer bookkeeping must reject that
    /// collision as a `Schema` error naming both types and the colliding
    /// file, never silently merge two distinct object types into one table.
    #[test]
    fn by_type_write_rejects_two_object_types_deriving_the_same_table_name() {
        // Precondition the whole scenario rests on: the two types really do
        // collide on the derived file name.
        assert_eq!(
            table_name_for_type("Ext_A"),
            table_name_for_type("+A"),
            "fixture fact: 'Ext_A' and '+A' must derive the same table file name"
        );

        let tmp = tempfile::tempdir().unwrap();
        let batch = object_type_only_batch(&["Ext_A", "+A"]);
        let mut writers = TableWriters::new(
            TableLayout::ByType,
            tmp.path(),
            batch.schema(),
            WriterProperties::default(),
        )
        .unwrap();

        let e = writers.write_batch(&batch).unwrap_err();
        assert!(
            matches!(e, CityParquetError::Schema(_)),
            "expected a Schema error, got {e:?}"
        );
        let msg = e.to_string();
        assert!(
            msg.contains("Ext_A")
                && msg.contains("+A")
                && msg.contains("cityobjects_ext_a.parquet"),
            "the error must name both colliding types and the derived file, got: {msg}"
        );

        // The SAME type re-appearing (across batches) is not a collision:
        // its claim matches, so writing proceeds.
        let mut ok_writers = TableWriters::new(
            TableLayout::ByType,
            tmp.path(),
            batch.schema(),
            WriterProperties::default(),
        )
        .unwrap();
        ok_writers
            .write_batch(&object_type_only_batch(&["+A"]))
            .unwrap();
        ok_writers
            .write_batch(&object_type_only_batch(&["+A"]))
            .unwrap();
        let tables = ok_writers.finish(&[]).unwrap();
        assert_eq!(tables, vec!["cityobjects_ext_a.parquet".to_string()]);
    }

    /// M5 review follow-up (Minor b): an all-null `object_type` batch must
    /// be a hard error under ByType, not a silent drop — aligned with
    /// `object_type_mask`'s identical guard (the schema declares the column
    /// non-nullable, so this is defence-in-depth against corrupt/foreign
    /// batches).
    #[test]
    fn distinct_types_errors_on_a_null_object_type_instead_of_skipping_it() {
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

        let e = distinct_types_in_batch(&batch).unwrap_err();
        assert!(
            matches!(e, CityParquetError::Schema(_)),
            "expected a Schema error, got {e:?}"
        );
        assert!(e.to_string().contains("must not be null"), "got: {e}");
    }

    /// M5 task 5: the `cityobjects_<snake>.parquet` naming rule, including
    /// the leading-`+` (CityJSON extension type) case neither shipped
    /// fixture exercises — pinned as a pure-function unit test since it
    /// needs no real CityJSON at all.
    #[test]
    fn table_name_for_type_snake_cases_and_prefixes_extension_types() {
        assert_eq!(
            table_name_for_type("Building"),
            "cityobjects_building.parquet"
        );
        assert_eq!(
            table_name_for_type("BuildingPart"),
            "cityobjects_buildingpart.parquet"
        );
        assert_eq!(
            table_name_for_type("+Foo"),
            "cityobjects_ext_foo.parquet",
            "a leading '+' must become the prefix 'ext_', not fold into a leading underscore"
        );
        assert_eq!(
            table_name_for_type("+My Extension-Type"),
            "cityobjects_ext_my_extension_type.parquet",
            "every non-alphanumeric character (after stripping the leading '+') becomes '_'"
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
