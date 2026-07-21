//! Derive the `city3d:*` property set from a written CityParquet package.
//!
//! Everything here is read back from the files on disk — the Parquet footer
//! and the rebuilt Arrow schema. Nothing is accumulated during conversion.
//! That is what makes spec §13.2's rule ("where the STAC Item and the Parquet
//! footer disagree, the footer is authoritative") true by construction, and
//! it means the encoder's own type decisions are reported rather than
//! independently re-inferred.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use arrow_schema::{Schema, extension::EXTENSION_TYPE_NAME_KEY};
use city3d_stac_types::metadata::AttributeDefinition;
use city3d_stac_types::stac::{City3dProperties, CityObjectsCount};
use cityparquet_schema::model::LOD_KEY;
use cityparquet_schema::types::Lod;
use cityparquet_schema::{
    AttributeType, CityParquetError, CityParquetMetadata, PackageManifest, Result,
};
use parquet::arrow::ProjectionMask;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::file::metadata::ParquetMetaData;
use parquet::schema::types::ColumnPath;

use crate::reader::CityParquetReaderBuilder;
use crate::stac::attribute_type::to_city3d;

/// Extension type name tagging a Utf8 column whose values are JSON text.
/// Mirrors the constant of the same value in [`crate::reader`]; a `Json`
/// attribute is *stored* as Utf8, so the raw Arrow type alone cannot
/// distinguish it from a plain string column.
const ARROW_JSON_EXTENSION: &str = "arrow.json";

/// The object tables of a package, resolved once and reused.
pub struct PackageTables {
    /// The package directory itself.
    pub dir: PathBuf,
    /// Absolute paths of the object tables, in manifest order.
    pub tables: Vec<PathBuf>,
    /// Sidecar file names the package declares.
    pub sidecar_files: Vec<String>,
}

impl PackageTables {
    /// Resolve a package's object tables and declared sidecars.
    ///
    /// TEMPORARY: reads the `tables` and `sidecar_files` lists from the
    /// package's `metadata.json` manifest, exactly as [`crate::export`] does.
    /// Plan 2b replaces `metadata.json` with a STAC Item and rebinds this to
    /// STAC asset roles. The binding — not the parsing — is the interesting
    /// part of that change, so it is deliberately not anticipated here, and
    /// no directory-scanning heuristic is invented in the meantime.
    pub fn open(dir: &Path) -> Result<Self> {
        let manifest_path = dir.join("metadata.json");
        let text = fs::read_to_string(&manifest_path).map_err(|e| {
            CityParquetError::Io(format!("cannot read {}: {e}", manifest_path.display()))
        })?;
        let manifest: PackageManifest = serde_json::from_str(&text)?;
        if manifest.tables.is_empty() {
            return Err(CityParquetError::Metadata(
                "package manifest lists no tables".to_string(),
            ));
        }
        Ok(Self {
            dir: dir.to_path_buf(),
            tables: manifest.tables.iter().map(|t| dir.join(t)).collect(),
            sidecar_files: manifest.sidecar_files.clone(),
        })
    }
}

/// The LoDs a package carries, as the LoD strings the writer used.
///
/// Read from each geometry field's `cityparquet:lod` tag rather than parsed
/// back out of column names. The un-suffixed `geometry` column holds the
/// footprint LoD (§9) and so has no suffix to parse, but
/// [`CityParquetReaderBuilder::cityparquet_arrow_schema`] re-attaches its
/// exact `0.*` value as field metadata — reusing that tag means this reader
/// cannot disagree with the writer that produced it.
///
/// Returned sorted by LoD order (not lexicographically) and deduplicated, so
/// a derived Item is byte-stable run to run.
fn lods_from_schema(schema: &Schema) -> Vec<String> {
    let mut lods: BTreeSet<Lod> = BTreeSet::new();
    for field in schema.fields() {
        let name = field.name();
        // `geometry_properties*` also starts with `geometry_` but is not a
        // geometry column; `material_lod*`/`texture_lod*` carry a lod tag too
        // and are not geometry either.
        let is_geometry = name == "geometry" || name.starts_with("geometry_lod");
        if !is_geometry {
            continue;
        }
        if let Some(lod) = field
            .metadata()
            .get(LOD_KEY)
            .and_then(|t| Lod::parse(t).ok())
        {
            lods.insert(lod);
        }
    }
    lods.into_iter().map(|l| l.to_string()).collect()
}

/// The attribute definitions a package carries.
///
/// The declared attribute columns come from the footer; each one's type comes
/// from the rebuilt schema. A `Json` attribute is stored as Utf8 and is only
/// distinguishable by its `arrow.json` extension tag, so that is checked
/// before falling back to [`AttributeType::from_arrow`] — otherwise every
/// JSON-valued attribute would be reported as a plain `String`. This mirrors
/// the same disambiguation in [`crate::reader`].
///
/// A column whose Arrow type maps to no CityParquet type is skipped rather
/// than guessed at; the count is returned so the caller can report it.
fn attributes_from_schema(
    schema: &Schema,
    meta: &CityParquetMetadata,
) -> (Vec<AttributeDefinition>, usize) {
    let mut defs = Vec::with_capacity(meta.attribute_columns.len());
    let mut skipped = 0usize;

    for name in &meta.attribute_columns {
        let Ok(field) = schema.field_with_name(name) else {
            skipped += 1;
            continue;
        };
        let tagged_json = field
            .metadata()
            .get(EXTENSION_TYPE_NAME_KEY)
            .map(String::as_str)
            == Some(ARROW_JSON_EXTENSION);
        let cp_type = if tagged_json {
            Some(AttributeType::Json)
        } else {
            AttributeType::from_arrow(field.data_type())
        };
        match cp_type {
            Some(t) => defs.push(AttributeDefinition::new(name, to_city3d(t))),
            None => skipped += 1,
        }
    }

    (defs, skipped)
}

/// Whether a top-level column holds at least one non-null value, judged from
/// Parquet column-chunk statistics alone.
///
/// `None` means "cannot tell": the column exists but no chunk carries
/// statistics, so answering would require reading the data. Callers report
/// that as an absent field rather than guessing — an absent `city3d:` flag is
/// honest, a wrong one is not.
///
/// This exists because **column presence proves nothing about content**: the
/// writer emits a `material_lod*` and `texture_lod*` column for every LoD
/// unconditionally (see `cityparquet_schema::model`), so a package with no
/// appearance at all still has those columns, entirely null.
pub(crate) fn column_has_any_non_null(metadata: &ParquetMetaData, name: &str) -> Option<bool> {
    let path = ColumnPath::new(vec![name.to_string()]);
    let mut saw_statistics = false;

    for rg in metadata.row_groups() {
        let Some(chunk) = rg.columns().iter().find(|c| c.column_path() == &path) else {
            // No such column in this file at all.
            return Some(false);
        };
        let Some(stats) = chunk.statistics() else {
            continue;
        };
        saw_statistics = true;
        let nulls = stats.null_count_opt().unwrap_or(0);
        if nulls < rg.num_rows().max(0) as u64 {
            return Some(true);
        }
    }

    if saw_statistics { Some(false) } else { None }
}

/// Whether a column holds a non-null value, reading the column when the
/// footer cannot say.
///
/// **Why a read is needed at all.** `crate::recipe` sets `statistics_for_json`
/// to `false` by default (statistics over serialised JSON blobs are not useful
/// for querying), and `material_lod*`, `texture_lod*` and
/// `geometry_properties*` are all JSON columns. So for exactly the columns
/// these `city3d:` flags depend on, the footer carries no statistics and the
/// data must be consulted. Only the single column is projected, so this reads
/// one column rather than the table.
pub(crate) fn column_populated(
    path: &Path,
    metadata: &ParquetMetaData,
    column: &str,
) -> Result<Option<bool>> {
    if let Some(known) = column_has_any_non_null(metadata, column) {
        return Ok(Some(known));
    }

    let file = fs::File::open(path)
        .map_err(|e| CityParquetError::Io(format!("cannot open {}: {e}", path.display())))?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)
        .map_err(|e| CityParquetError::Parquet(format!("cannot open {}: {e}", path.display())))?;

    let descr = builder.parquet_schema();
    let Some(root) = (0..descr.num_columns())
        .find(|&i| descr.column(i).path().parts().first().map(String::as_str) == Some(column))
    else {
        return Ok(Some(false));
    };
    let mask = ProjectionMask::leaves(descr, [root]);

    let reader = builder
        .with_projection(mask)
        .build()
        .map_err(|e| CityParquetError::Parquet(format!("cannot build reader: {e}")))?;

    for batch in reader {
        let batch = batch
            .map_err(|e| CityParquetError::Parquet(format!("read {}: {e}", path.display())))?;
        if batch.num_columns() == 0 {
            continue;
        }
        let col = batch.column(0);
        if col.null_count() < col.len() {
            return Ok(Some(true));
        }
    }
    Ok(Some(false))
}

/// Whether any column named `<prefix>*` holds a non-null value in this file.
fn any_prefixed_column_populated(
    path: &Path,
    metadata: &ParquetMetaData,
    schema: &Schema,
    prefix: &str,
) -> Result<Option<bool>> {
    let mut any_known = false;
    for field in schema.fields() {
        if !field.name().starts_with(prefix) {
            continue;
        }
        match column_populated(path, metadata, field.name())? {
            Some(true) => return Ok(Some(true)),
            Some(false) => any_known = true,
            None => {}
        }
    }
    Ok(if any_known { Some(false) } else { None })
}

/// Derive the footer- and schema-only `city3d:*` fields.
///
/// `co_types` and `semantic_surfaces` need further column work and are left
/// unset here; later tasks fill them.
pub fn derive_from_footer(tables: &PackageTables) -> Result<City3dProperties> {
    let mut props = City3dProperties::new();
    let mut total_rows: u64 = 0;
    // Appearance presence is a property of the DATA, so it is judged across
    // every object table, not just the first — a by-type package may carry
    // textures on one type and none on another.
    let mut textures: Option<bool> = None;
    let mut materials: Option<bool> = None;

    for (idx, path) in tables.tables.iter().enumerate() {
        let file = fs::File::open(path)
            .map_err(|e| CityParquetError::Io(format!("cannot open {}: {e}", path.display())))?;
        let builder = ParquetRecordBatchReaderBuilder::try_new(file).map_err(|e| {
            CityParquetError::Parquet(format!("cannot open {}: {e}", path.display()))
        })?;

        let file_meta = builder.metadata().clone();
        total_rows += file_meta.file_metadata().num_rows().max(0) as u64;
        let schema = builder.cityparquet_arrow_schema()?;

        // Every object table in a package carries the identical schema and KV
        // metadata (see `crate::package`'s writer), so the first table is
        // authoritative for schema-derived fields — the same assumption
        // `crate::export` makes. Row counts and data-presence flags are
        // per-table and accumulate.
        if idx == 0 {
            let meta = builder.cityparquet_metadata()?;
            props.version = meta.source_version.clone();
            props.lods = lods_from_schema(&schema);
            let (attributes, _skipped) = attributes_from_schema(&schema, &meta);
            props.attributes = attributes;
        }

        textures = merge_presence(
            textures,
            any_prefixed_column_populated(path, &file_meta, &schema, "texture_lod")?,
        );
        materials = merge_presence(
            materials,
            any_prefixed_column_populated(path, &file_meta, &schema, "material_lod")?,
        );
    }

    // A declared appearance sidecar is a positive signal in its own right:
    // the Compatibility profile only writes one when the dataset has that
    // appearance kind to store.
    if tables.sidecar_files.iter().any(|f| f == "textures.parquet") {
        textures = Some(true);
    }
    if tables
        .sidecar_files
        .iter()
        .any(|f| f == "materials.parquet")
    {
        materials = Some(true);
    }

    props.textures = textures;
    props.materials = materials;
    props.city_objects = Some(CityObjectsCount::Integer(total_rows));
    Ok(props)
}

/// Combine two presence verdicts: any `Some(true)` wins, `None` is "unknown"
/// and never overrides a known answer.
fn merge_presence(a: Option<bool>, b: Option<bool>) -> Option<bool> {
    match (a, b) {
        (Some(true), _) | (_, Some(true)) => Some(true),
        (Some(false), _) | (_, Some(false)) => Some(false),
        _ => None,
    }
}
