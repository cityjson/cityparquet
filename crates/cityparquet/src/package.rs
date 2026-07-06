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

use cityparquet_schema::{CITYPARQUET_VERSION, CityParquetError, PackageManifest, Profile, Result};

use crate::encode::encode;
use crate::recipe::WriterRecipe;
use crate::scan::scan;
use crate::sidecar::{write_materials, write_textures};
use crate::source::Source;

/// The one data table every profile writes.
const CITYOBJECTS_TABLE: &str = "cityobjects.parquet";
/// Compatibility-profile sidecar tables. `geometry_templates.parquet` is not
/// listed here: template appearance folding lands in Task 8, so this pass
/// never writes it (`ConvertReport::templates_written` stays `0`).
const MATERIALS_TABLE: &str = "materials.parquet";
const TEXTURES_TABLE: &str = "textures.parquet";

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
    /// Rows written to `geometry_templates.parquet`. Always `0` until Task 8
    /// folds geometry-template appearance into the interner sweep.
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

/// Convert `opts.input` (CityJSON or CityJSONSeq) into a CityParquet package
/// directory at `opts.output_dir`: one scan pass to infer the schema and
/// dataset metadata, one encode pass streamed straight into an `ArrowWriter`
/// using `opts.recipe`'s per-column `WriterProperties`, then (Compatibility
/// profile only) the `materials.parquet`/`textures.parquet` sidecars built
/// from the encode pass's [`crate::appearance::AppearanceInterner`], then a
/// `metadata.json` manifest alongside it all.
///
/// `geometry_templates.parquet` is not written by this pass yet (Task 8), so
/// a Compatibility conversion of a dataset whose appearance is reachable only
/// from geometry templates will under-count `materials_written`/
/// `textures_written` relative to the dataset's true definitions — see
/// [`ConvertReport::templates_written`].
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

    // This is the STATIC per-profile expectation (`Profile::sidecar_files`),
    // not yet the files this run will actually produce: how many
    // materials/textures a Compatibility dataset has (and therefore whether
    // a sidecar file is written at all — an empty sidecar is skipped, see
    // `crate::sidecar`) is only known after the encode pass below runs to
    // completion, but the Parquet key-value metadata is embedded into the
    // WriterProperties *before* the writer opens. There is no ordering that
    // lets the KV `sidecar_files` entry reflect the true post-encode file
    // list, so it stays the static profile expectation; `metadata.json`
    // (built after encode, from the ACTUAL sidecar files written below) is
    // the source of truth for what the package really contains.
    let sidecars: Vec<String> = opts
        .profile
        .sidecar_files()
        .iter()
        .map(|s| s.to_string())
        .collect();
    let metadata = scan_result.metadata(&sidecars)?;
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
    writer
        .close()
        .map_err(|e| parquet_err(format!("cannot finalise parquet file: {e}")))?;

    let mut files = vec![cityobjects_path];
    let mut sidecar_files_written: Vec<String> = Vec::new();
    let mut materials_written = 0usize;
    let mut textures_written = 0usize;
    if opts.profile == Profile::Compatibility {
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
    }

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
        templates_written: 0,
    })
}
