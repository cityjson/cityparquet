//! End-to-end package conversion: CityJSON/CityJSONSeq in, a CityParquet
//! package directory out.
//!
//! Wires the three passes already shipped by this crate — [`crate::scan`]
//! (schema + dataset metadata), [`crate::encode`] (the `RecordBatch` stream),
//! and [`crate::recipe`] (per-column `WriterProperties`) — into a single
//! [`convert`] call that writes `cityobjects.parquet` plus the package-level
//! `metadata.json` manifest.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use arrow_schema::Schema;
use parquet::arrow::ArrowWriter;
use parquet::file::metadata::KeyValue;
use parquet::file::properties::WriterProperties;

use cityparquet_schema::{CITYPARQUET_VERSION, CityParquetError, PackageManifest, Profile, Result};

use crate::appearance::AppearanceInterner;
use crate::encode::{encode, rewrite_geometry_appearance};
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
}

impl ConvertOptions {
    /// Core profile, 4096-row batches, the default [`WriterRecipe`], and no
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
/// Not fully atomic against a crash mid-swap itself (a failure between two
/// `rename` calls, or between the purge and the first rename, could leave
/// `output_dir` with a mix of old and new files) — `rename` within the same
/// directory tree is normally an in-place metadata operation on the local
/// filesystems this crate targets, so that window is extremely narrow
/// compared to the whole encode pass it replaces; the property this fix
/// actually buys is that an encode-time failure (the realistic, common
/// failure mode this crash-safe overwrite exists for) never reaches this
/// function at all.
fn commit_package(tmp_dir: &Path, output_dir: &Path, files: &[String]) -> Result<()> {
    purge_stale_package_files(output_dir)?;
    for name in files {
        let from = tmp_dir.join(name);
        let to = output_dir.join(name);
        fs::rename(&from, &to).map_err(|e| {
            io_err(format!(
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
        let (material, texture, props) = rewrite_geometry_appearance(
            tpl,
            &outcome,
            interner,
            &local_materials,
            &local_textures,
            &local_uvs,
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
    let cityobjects_path = tmp_dir.join(CITYOBJECTS_TABLE);
    let file = fs::File::create(&cityobjects_path)
        .map_err(|e| io_err(format!("cannot create {}: {e}", cityobjects_path.display())))?;
    let mut writer = ArrowWriter::try_new(file, arrow_schema, Some(props))
        .map_err(|e| parquet_err(format!("cannot open parquet writer: {e}")))?;

    let mut batches = encode(source, scan_result, opts.batch_size)?;
    for batch in batches.by_ref() {
        let batch = batch?;
        writer
            .write(&batch)
            .map_err(|e| parquet_err(format!("parquet write error: {e}")))?;
    }
    // `by_ref()` above means `batches` is still ours to read stats/appearance
    // from — consuming it by value (e.g. plain `.collect()`) would have
    // dropped it (and its running totals) before we could ask.
    let encode_stats = batches.stats();

    // Sidecars are written while the main-table writer is still open (they
    // are separate files, so nothing conflicts), because the footer's
    // `sidecar_files` entry below must record what was ACTUALLY written.
    let mut file_names = vec![CITYOBJECTS_TABLE.to_string()];
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
            file_names.push(MATERIALS_TABLE.to_string());
        }

        let textures_path = tmp_dir.join(TEXTURES_TABLE);
        textures_written = write_textures(&textures_path, appearance.textures())?;
        if textures_written > 0 {
            sidecar_files_written.push(TEXTURES_TABLE.to_string());
            file_names.push(TEXTURES_TABLE.to_string());
        }

        if !template_rows.is_empty() {
            let templates_path = tmp_dir.join(TEMPLATES_TABLE);
            templates_written = write_templates(&templates_path, &template_rows)?;
            if templates_written > 0 {
                sidecar_files_written.push(TEMPLATES_TABLE.to_string());
                file_names.push(TEMPLATES_TABLE.to_string());
            }
        }
    }

    // Now that the actual sidecar list is known, record it in the parquet
    // footer (the pre-encode `WriterProperties` KV set omitted the key
    // entirely, so this cannot produce a duplicate; and even against a
    // foreign file that DID carry one, appended entries come after the
    // props entries in the footer and `CityParquetMetadata::from_key_values`
    // is last-wins). Encoded exactly as `to_key_values` renders a non-empty
    // list: JSON text.
    writer.append_key_value_metadata(KeyValue::new(
        "sidecar_files".to_string(),
        serde_json::to_string(&sidecar_files_written)?,
    ));
    writer
        .close()
        .map_err(|e| parquet_err(format!("cannot finalise parquet file: {e}")))?;

    let manifest = PackageManifest {
        cityparquet_version: CITYPARQUET_VERSION.to_string(),
        profile: opts.profile,
        lods: scan_result.lods.iter().map(|lod| lod.to_string()).collect(),
        tables: vec![CITYOBJECTS_TABLE.to_string()],
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

    if let Err(e) = commit_package(&tmp_dir, &opts.output_dir, &written.file_names) {
        let _ = fs::remove_dir_all(&tmp_dir);
        return Err(e);
    }

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
