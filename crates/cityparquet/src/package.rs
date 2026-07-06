//! End-to-end package conversion: CityJSON/CityJSONSeq in, a CityParquet
//! package directory out.
//!
//! Wires the three passes already shipped by this crate — [`crate::scan`]
//! (schema + dataset metadata), [`crate::encode`] (the `RecordBatch` stream),
//! and [`crate::recipe`] (per-column `WriterProperties`) — into a single
//! [`convert`] call that writes `cityobjects.parquet` plus the package-level
//! `metadata.json` manifest.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use parquet::arrow::ArrowWriter;
use parquet::file::metadata::KeyValue;

use cityparquet_schema::{CITYPARQUET_VERSION, CityParquetError, PackageManifest, Profile, Result};

use crate::appearance::AppearanceInterner;
use crate::encode::{encode, rewrite_geometry_appearance};
use crate::recipe::WriterRecipe;
use crate::scan::scan;
use crate::sidecar::{TemplateRow, write_materials, write_templates, write_textures};
use crate::source::Source;
use crate::wkb_write::{VertexPool, geometry_to_wkb};

/// The one data table every profile writes.
const CITYOBJECTS_TABLE: &str = "cityobjects.parquet";
/// Compatibility-profile sidecar tables.
const MATERIALS_TABLE: &str = "materials.parquet";
const TEXTURES_TABLE: &str = "textures.parquet";
const TEMPLATES_TABLE: &str = "geometry_templates.parquet";

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

/// Build one [`TemplateRow`] per entry in `templates.templates`, folding
/// their `material`/`texture` definitions into `interner` — the SAME
/// interner the main encode pass populated from `source`'s features — so
/// the materials/textures sidecars end up describing every definition in
/// the dataset, not just the ones a regular feature geometry reaches.
///
/// `id` is the template's position as a string, matching the main-table
/// `template.id` column (see `crate::encode`'s `build_template`). The
/// rewrite rules (drop-realignment, dataset-global rewrite,
/// `geometry_properties` shape) are IDENTICAL to a regular feature
/// geometry's — see [`rewrite_geometry_appearance`]'s doc comment — because
/// a template's `material`/`texture`/`semantics` follow the exact same
/// CityJSON shapes a feature geometry's do.
///
/// A template's own coordinates are template-LOCAL (CityJSON spec §3.4:
/// `vertices-templates` is not subject to the dataset transform), so they
/// are looked up through [`VertexPool::raw`] over `templates.vertices_templates`,
/// never the dataset's quantised [`VertexPool::new`].
fn build_template_rows(
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
        let geometry_properties: serde_json::Value = serde_json::from_str(&props)?;
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
    // TODO(M4): purge stale files on overwrite once sidecars exist — a stale
    // sidecar must not outlive a manifest that says sidecar_files: [].

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

    let cityobjects_path = opts.output_dir.join(CITYOBJECTS_TABLE);
    let file = fs::File::create(&cityobjects_path)
        .map_err(|e| io_err(format!("cannot create {}: {e}", cityobjects_path.display())))?;
    let mut writer = ArrowWriter::try_new(file, arrow_schema, Some(props))
        .map_err(|e| parquet_err(format!("cannot open parquet writer: {e}")))?;

    let mut batches = encode(&source, &scan_result, opts.batch_size)?;
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
    let mut files = vec![cityobjects_path];
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
            Some(templates) => build_template_rows(templates, &source, batches.appearance_mut())?,
            None => Vec::new(),
        };

        let appearance = batches.appearance();

        let materials_path = opts.output_dir.join(MATERIALS_TABLE);
        materials_written = write_materials(&materials_path, appearance.materials())?;
        if materials_written > 0 {
            sidecar_files_written.push(MATERIALS_TABLE.to_string());
            files.push(materials_path);
        }

        let textures_path = opts.output_dir.join(TEXTURES_TABLE);
        textures_written = write_textures(&textures_path, appearance.textures())?;
        if textures_written > 0 {
            sidecar_files_written.push(TEXTURES_TABLE.to_string());
            files.push(textures_path);
        }

        if !template_rows.is_empty() {
            let templates_path = opts.output_dir.join(TEMPLATES_TABLE);
            templates_written = write_templates(&templates_path, &template_rows)?;
            if templates_written > 0 {
                sidecar_files_written.push(TEMPLATES_TABLE.to_string());
                files.push(templates_path);
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
    let metadata_path = opts.output_dir.join("metadata.json");
    fs::write(&metadata_path, serde_json::to_string_pretty(&manifest)?)
        .map_err(|e| io_err(format!("cannot write {}: {e}", metadata_path.display())))?;
    files.push(metadata_path);

    Ok(ConvertReport {
        object_count: scan_result.object_count,
        files,
        skipped_same_lod_geometries: encode_stats.skipped_same_lod_geometries,
        attribute_coercion_nulls: encode_stats.attribute_coercion_nulls,
        degenerate_rings_dropped: encode_stats.degenerate_rings_dropped,
        degenerate_surfaces_dropped: encode_stats.degenerate_surfaces_dropped,
        materials_written,
        textures_written,
        templates_written,
    })
}
