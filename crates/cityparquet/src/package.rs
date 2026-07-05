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
use crate::source::Source;

/// The one Core-profile data table this pass writes; the compatibility
/// profile's sidecar tables (`materials.parquet`, `textures.parquet`,
/// `geometry_templates.parquet`) land in M4.
const CITYOBJECTS_TABLE: &str = "cityobjects.parquet";

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
}

fn err(msg: String) -> CityParquetError {
    CityParquetError::Schema(msg)
}

/// Convert `opts.input` (CityJSON or CityJSONSeq) into a CityParquet package
/// directory at `opts.output_dir`: one scan pass to infer the schema and
/// dataset metadata, one encode pass streamed straight into an `ArrowWriter`
/// using `opts.recipe`'s per-column `WriterProperties`, then a `metadata.json`
/// manifest alongside it.
///
/// Only the Core profile is implemented here — the compatibility profile
/// (sidecar `materials`/`textures`/`geometry_templates` tables) lands in M4
/// and is rejected up front. `opts.output_dir` is created if missing; if it
/// already exists and is non-empty, conversion fails unless `opts.overwrite`
/// is set.
pub fn convert(opts: &ConvertOptions) -> Result<ConvertReport> {
    if opts.profile == Profile::Compatibility {
        return Err(err("compatibility profile lands in M4".to_string()));
    }

    fs::create_dir_all(&opts.output_dir).map_err(|e| {
        err(format!(
            "cannot create output directory {}: {e}",
            opts.output_dir.display()
        ))
    })?;
    let has_entries = fs::read_dir(&opts.output_dir)
        .map_err(|e| {
            err(format!(
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
        .map_err(|e| err(format!("cannot create {}: {e}", cityobjects_path.display())))?;
    let mut writer = ArrowWriter::try_new(file, arrow_schema, Some(props))
        .map_err(|e| err(format!("cannot open parquet writer: {e}")))?;

    let mut batches = encode(&source, &scan_result, opts.batch_size)?;
    for batch in batches.by_ref() {
        let batch = batch?;
        writer
            .write(&batch)
            .map_err(|e| err(format!("parquet write error: {e}")))?;
    }
    // `by_ref()` above means `batches` is still ours to read stats from —
    // consuming it by value (e.g. plain `.collect()`) would have dropped it
    // (and its running totals) before we could ask.
    let encode_stats = batches.stats();
    writer
        .close()
        .map_err(|e| err(format!("cannot finalise parquet file: {e}")))?;

    let manifest = PackageManifest {
        cityparquet_version: CITYPARQUET_VERSION.to_string(),
        profile: opts.profile,
        lods: scan_result.lods.iter().map(|lod| lod.to_string()).collect(),
        tables: vec![CITYOBJECTS_TABLE.to_string()],
        sidecar_files: Vec::new(),
    };
    let metadata_path = opts.output_dir.join("metadata.json");
    fs::write(&metadata_path, serde_json::to_string_pretty(&manifest)?)
        .map_err(|e| err(format!("cannot write {}: {e}", metadata_path.display())))?;

    Ok(ConvertReport {
        object_count: scan_result.object_count,
        files: vec![cityobjects_path, metadata_path],
        skipped_same_lod_geometries: encode_stats.skipped_same_lod_geometries,
        attribute_coercion_nulls: encode_stats.attribute_coercion_nulls,
    })
}
