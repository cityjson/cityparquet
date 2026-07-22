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

use arrow_array::cast::AsArray;
use arrow_array::{Array, ArrayRef, StringArray};
use arrow_schema::{Schema, extension::EXTENSION_TYPE_NAME_KEY};
use city3d_stac_types::metadata::AttributeDefinition;
use city3d_stac_types::stac::types::Item;
use city3d_stac_types::stac::{City3dProperties, CityObjectsCount};
use cityparquet_schema::model::LOD_KEY;
use cityparquet_schema::types::Lod;
use cityparquet_schema::{AttributeType, CityParquetError, CityParquetMetadata, Result};
use parquet::arrow::ProjectionMask;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use serde_json::Value;

use crate::reader::CityParquetReaderBuilder;
use crate::stac::assets::{ROLE_OBJECT_TABLE, ROLE_SIDECAR};
use crate::stac::attribute_type::to_city3d;

/// Extension type name tagging a Utf8 column whose values are JSON text.
/// Mirrors the constant of the same value in [`crate::reader`]; a `Json`
/// attribute is *stored* as Utf8, so the raw Arrow type alone cannot
/// distinguish it from a plain string column.
const ARROW_JSON_EXTENSION: &str = "arrow.json";

/// The reserved column carrying each row's CityObject type (§5.1).
const OBJECT_TYPE_COLUMN: &str = "object_type";

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
    /// Reads `metadata.json` as a STAC Item and collects assets by role:
    /// [`ROLE_OBJECT_TABLE`] assets become `tables`, in the Item's asset
    /// order (an `IndexMap` preserves insertion order — the order
    /// [`crate::stac::build_item`]'s writer-facing caller inserted them in,
    /// which is itself first-appearance table order — see
    /// `package::TableWriters::finish`); [`ROLE_SIDECAR`] assets become
    /// `sidecar_files`. An asset's `href` (always `"./<name>"` for a package
    /// this crate writes) is the authoritative locator, not its map key —
    /// the STAC spec makes `href` the file reference, and deriving the path
    /// from it keeps `open` correct for a foreign writer that keys its
    /// assets differently.
    pub fn open(dir: &Path) -> Result<Self> {
        let manifest_path = dir.join("metadata.json");
        let text = fs::read_to_string(&manifest_path).map_err(|e| {
            CityParquetError::Io(format!("cannot read {}: {e}", manifest_path.display()))
        })?;
        let item: Item = serde_json::from_str(&text)?;

        let mut tables = Vec::new();
        let mut sidecar_files = Vec::new();
        // A package naming the same object table twice is corrupt: every
        // object in it would be counted twice. `crate::export` rejects this
        // for the same reason, and a describing pass must not be more
        // permissive than the pass that consumes the description.
        let mut seen_tables = BTreeSet::new();

        for asset in item.assets.values() {
            let is_object_table = asset.roles.iter().any(|r| r == ROLE_OBJECT_TABLE);
            let is_sidecar = asset.roles.iter().any(|r| r == ROLE_SIDECAR);
            if !is_object_table && !is_sidecar {
                continue;
            }
            let name = asset.href.trim_start_matches("./").to_string();
            if is_object_table {
                if !seen_tables.insert(name.clone()) {
                    return Err(CityParquetError::Metadata(format!(
                        "package lists duplicate object table '{name}'"
                    )));
                }
                tables.push(dir.join(&name));
            } else {
                sidecar_files.push(name);
            }
        }

        if tables.is_empty() {
            return Err(CityParquetError::Metadata(
                "package lists no object tables (no asset carries the cityparquet-objects role)"
                    .to_string(),
            ));
        }

        Ok(Self {
            dir: dir.to_path_buf(),
            tables,
            sidecar_files,
        })
    }

    /// Build a [`PackageTables`] directly from already-known table/sidecar
    /// names, without touching disk.
    ///
    /// Pure: `tables` are joined onto `dir` the same way [`Self::open`] joins
    /// the manifest's `tables` list, and `sidecars` are stored verbatim. No
    /// existence/duplicate/empty check is performed here — a writer calls
    /// this with the list it is ABOUT to write, before the files exist, so
    /// there is nothing on disk yet to validate against.
    pub fn from_lists(dir: &Path, tables: &[String], sidecars: &[String]) -> Self {
        Self {
            dir: dir.to_path_buf(),
            tables: tables.iter().map(|t| dir.join(t)).collect(),
            sidecar_files: sidecars.to_vec(),
        }
    }
}

/// The LoDs a package carries, as the LoD strings the writer used.
///
/// Read from each geometry field's `cityparquet:lod` tag rather than parsed
/// back out of column names — every real LoD, including LoD0, is suffixed
/// (`geometry_lod0_0`, `geometry_lod2_2`, …) and carries that tag directly.
/// The bare, un-suffixed `geometry` column only exists for the
/// zero-analysis-geometry fallback schema (a dataset with only
/// `GeometryInstance`s, or none — §9); it never carries a `cityparquet:lod`
/// tag, so matching its name here never contributes a LoD — it is included
/// only so the loop recognises it as a geometry column (rather than treating
/// it as unrelated) when deciding whether to look for the tag.
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

/// Whether one `geometry_properties` cell describes semantic surfaces.
///
/// **Non-null is not the question.** `crate::encode`'s
/// `geometry_properties_json` always writes at least `{"type": ...}` for every
/// stored geometry, and adds `surfaces` / `face_semantics` only when the
/// source geometry actually carried semantics (§8). So a null-count test
/// answers "does this package have geometry", which is true of essentially
/// every package — the cell's content has to be inspected.
fn cell_has_semantics(cell: &str) -> bool {
    let Ok(Value::Object(map)) = serde_json::from_str::<Value>(cell) else {
        return false;
    };
    // Either member alone is enough: `surfaces` carries the semantic-surface
    // definitions, `face_semantics` the per-face indices into them.
    map.contains_key("surfaces") || map.contains_key("face_semantics")
}

/// Whether any `geometry_properties*` column in this file describes semantic
/// surfaces.
///
/// Always reads: these are JSON columns, and no footer statistic can
/// distinguish `{"type":"Solid"}` from a cell that also carries `surfaces`.
/// Only the `geometry_properties*` columns are projected.
fn file_has_semantic_surfaces(path: &Path, schema: &Schema) -> Result<bool> {
    for field in schema.fields() {
        if !is_reserved_column_for(field.name(), "geometry_properties") {
            continue;
        }
        let file = fs::File::open(path)
            .map_err(|e| CityParquetError::Io(format!("cannot open {}: {e}", path.display())))?;
        let builder = ParquetRecordBatchReaderBuilder::try_new(file).map_err(|e| {
            CityParquetError::Parquet(format!("cannot open {}: {e}", path.display()))
        })?;
        let descr = builder.parquet_schema();
        let Some(root) = (0..descr.num_columns())
            .find(|&i| descr.column(i).path().parts().first() == Some(field.name()))
        else {
            continue;
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
            let Some(values) = batch.column(0).as_any().downcast_ref::<StringArray>() else {
                continue;
            };
            for i in 0..values.len() {
                if values.is_valid(i) && cell_has_semantics(values.value(i)) {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
}

/// Whether a field is one of the reserved columns for `base`, across every LoD.
///
/// **The footprint LoD's column has no suffix.** `geometry_column_name`
/// (`cityparquet_schema::types`) drops the suffix for the footprint LoD, so the
/// column is literally `geometry_properties`, not `geometry_properties_lod0`.
/// Matching only `geometry_properties_lod*` would make semantic surfaces on the
/// footprint LoD invisible, and the CLI enables LoD0 synthesis by default, so
/// that column exists in essentially every CLI-written package.
fn is_reserved_column_for(name: &str, base: &str) -> bool {
    name == base || name.starts_with(&format!("{base}_lod"))
}

/// The distinct CityObject types a package carries.
///
/// **Filenames cannot answer this, even under the by-type layout.** Spec §4.3
/// requires a 2nd-level type to be written into its 1st-level parent's table:
/// `building.parquet` carries `Building`, `BuildingPart`,
/// `BuildingInstallation` and the rest, and its name mentions only the first.
/// A filename-derived answer therefore systematically under-reports what is
/// present, so the `object_type` column is read in every case — one code path
/// for both layouts.
///
/// The column is dictionary-encoded (§5.1), so the distinct values come from
/// the dictionary rather than a per-row scan, and only that column is
/// projected.
///
/// `object_type` stores the CityGML 3.0 class name (spec "object_type
/// vocabulary"), but `city3d:co_types` is defined over the **source** type
/// vocabulary (spec "metadata.json — STAC Item": "`city3d:co_types` uses the
/// source type vocabulary ... not `object_type`'s stripped, CityGML-class
/// form"). Every collected value is therefore mapped back through
/// [`cityparquet_schema::cityjson_type_for_citygml_class`] — the same reverse
/// lookup `crate::decode` uses to restore the CityJSON `type` field — falling
/// back to the value verbatim when there is no taxonomy entry (an extension
/// class, which has none).
///
/// Returned sorted and deduplicated, so a derived Item is byte-stable.
pub fn derive_co_types(tables: &PackageTables) -> Result<Vec<String>> {
    let mut seen: BTreeSet<String> = BTreeSet::new();

    for path in &tables.tables {
        let file = fs::File::open(path)
            .map_err(|e| CityParquetError::Io(format!("cannot open {}: {e}", path.display())))?;
        let builder = ParquetRecordBatchReaderBuilder::try_new(file).map_err(|e| {
            CityParquetError::Parquet(format!("cannot open {}: {e}", path.display()))
        })?;

        let descr = builder.parquet_schema();
        let Some(root) = (0..descr.num_columns()).find(|&i| {
            descr.column(i).path().parts().first().map(String::as_str) == Some(OBJECT_TYPE_COLUMN)
        }) else {
            return Err(CityParquetError::Metadata(format!(
                "{} has no {OBJECT_TYPE_COLUMN} column",
                path.display()
            )));
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
            collect_string_values(batch.column(0), &mut seen)?;
        }
    }

    let source_vocabulary: BTreeSet<String> = seen
        .into_iter()
        .map(|stored| {
            cityparquet_schema::cityjson_type_for_citygml_class(&stored)
                .map(str::to_string)
                .unwrap_or(stored)
        })
        .collect();
    Ok(source_vocabulary.into_iter().collect())
}

/// Collect the distinct non-null string values of `array` into `seen`.
///
/// `object_type` is dictionary-encoded, so the fast path reads the dictionary
/// values directly. A plain string array is also accepted, so a
/// CityParquet-conformant file from a writer that chose not to dictionary-encode
/// the column still works — this crate reads other writers' packages too.
fn collect_string_values(array: &ArrayRef, seen: &mut BTreeSet<String>) -> Result<()> {
    if let Some(dict) = array.as_any_dictionary_opt() {
        let values = dict
            .values()
            .as_any()
            .downcast_ref::<StringArray>()
            .ok_or_else(|| {
                CityParquetError::Metadata(format!(
                    "{OBJECT_TYPE_COLUMN} dictionary values are not Utf8"
                ))
            })?;
        // Collect only values a row actually REFERENCES. A Parquet/Arrow
        // dictionary may legally carry entries no row uses, and reading the
        // dictionary wholesale would invent a `city3d:co_types` value that is
        // not in the data — which matters precisely because this crate aims to
        // describe packages from other conformant writers too.
        let keys = dict.keys();
        for i in 0..keys.len() {
            if !keys.is_valid(i) {
                continue;
            }
            let Some(idx) = dict.normalized_keys().get(i).copied() else {
                continue;
            };
            if idx < values.len() && values.is_valid(idx) {
                seen.insert(values.value(idx).to_string());
            }
        }
        return Ok(());
    }

    let values = array
        .as_any()
        .downcast_ref::<StringArray>()
        .ok_or_else(|| {
            CityParquetError::Metadata(format!(
                "{OBJECT_TYPE_COLUMN} is neither dictionary-encoded nor Utf8"
            ))
        })?;
    for i in 0..values.len() {
        if values.is_valid(i) {
            seen.insert(values.value(i).to_string());
        }
    }
    Ok(())
}

/// Derive every `city3d:*` field a written package can supply.
///
/// Most come from the footer and Arrow schema of the first object table.
/// `co_types` and `semantic_surfaces` additionally read one projected column;
/// `textures`/`materials` are the presence of the appearance sidecars.
pub fn derive_from_footer(tables: &PackageTables) -> Result<City3dProperties> {
    let mut props = City3dProperties::new();
    let mut total_rows: u64 = 0;
    // Semantic-surface presence is a property of the DATA, judged across every
    // object table, not just the first — a by-type package may carry semantics
    // on one type and none on another. Appearance is judged separately, from the
    // presence of the definition sidecars (after the loop).
    let mut semantics: Option<bool> = None;

    for (idx, path) in tables.tables.iter().enumerate() {
        let file = fs::File::open(path)
            .map_err(|e| CityParquetError::Io(format!("cannot open {}: {e}", path.display())))?;
        let builder = ParquetRecordBatchReaderBuilder::try_new(file).map_err(|e| {
            CityParquetError::Parquet(format!("cannot open {}: {e}", path.display()))
        })?;

        // A negative row count is a corrupt footer, not something to normalise
        // away; an overflowing total likewise means the package is not what it
        // claims. Both are reported rather than silently absorbed.
        let rows = u64::try_from(builder.metadata().file_metadata().num_rows()).map_err(|_| {
            CityParquetError::Metadata(format!("{} declares a negative row count", path.display()))
        })?;
        total_rows = total_rows.checked_add(rows).ok_or_else(|| {
            CityParquetError::Metadata("package row count overflows u64".to_string())
        })?;
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

        // Semantic surfaces live in the `geometry_properties*` columns (§8),
        // but a null-count test would answer the wrong question — see
        // `cell_has_semantics`.
        semantics = merge_presence(semantics, Some(file_has_semantic_surfaces(path, &schema)?));
    }

    // Appearance presence IS the presence of its definition sidecar. The
    // Compatibility profile writes `textures.parquet` / `materials.parquet` only
    // when the dataset has that appearance kind to store; the Core profile
    // writes neither. A package whose columns carry appearance INDICES but ships
    // no sidecar cannot render appearance, so it is reported `false`: the flag
    // means "usable from this package", not "existed upstream". Always a
    // definite boolean — an absent sidecar is a known negative, not "unknown".
    props.textures = Some(tables.sidecar_files.iter().any(|f| f == "textures.parquet"));
    props.materials = Some(
        tables
            .sidecar_files
            .iter()
            .any(|f| f == "materials.parquet"),
    );
    props.semantic_surfaces = semantics;
    props.co_types = derive_co_types(tables)?;
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

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{PackageTables, cell_has_semantics};

    /// `from_lists` is a pure path-join — it must round-trip a table/sidecar
    /// list without reading anything from disk (the directory it is given
    /// here does not even exist), which is exactly what Task 4's writer
    /// needs: build the inventory it is ABOUT to write, before any file
    /// exists for `PackageTables::open` to read back.
    #[test]
    fn from_lists_joins_dir_without_touching_disk() {
        let dir = Path::new("/does/not/exist/on/this/machine");
        let tables = vec!["building.parquet".to_string(), "road.parquet".to_string()];
        let sidecars = vec!["materials.parquet".to_string()];

        let resolved = PackageTables::from_lists(dir, &tables, &sidecars);

        assert_eq!(resolved.dir, dir);
        assert_eq!(
            resolved.tables,
            vec![dir.join("building.parquet"), dir.join("road.parquet")]
        );
        assert_eq!(resolved.sidecar_files, sidecars);
    }

    /// Every stored geometry gets a `geometry_properties` cell carrying at
    /// least `{"type": ...}` (`crate::encode::geometry_properties_json`), so a
    /// non-null test would report semantic surfaces for any package that has
    /// geometry at all. These cases are the exact shapes that function emits,
    /// pinning the distinction the integration tests cannot: every fixture in
    /// this repo carries some semantics, so the negative case has no real
    /// dataset to come from.
    #[test]
    fn a_type_only_cell_is_not_semantics() {
        assert!(!cell_has_semantics(r#"{"type":"Solid"}"#));
        assert!(!cell_has_semantics(r#"{"type":"MultiSurface"}"#));
        assert!(!cell_has_semantics(
            r#"{"type":"Solid","shells":[[6]],"dropped_degenerate":{"rings":0}}"#
        ));
    }

    #[test]
    fn surfaces_or_face_semantics_is_semantics() {
        assert!(cell_has_semantics(
            r#"{"type":"MultiSurface","surfaces":[{"type":"RoofSurface"}]}"#
        ));
        assert!(cell_has_semantics(
            r#"{"type":"MultiSurface","face_semantics":[0,0,1]}"#
        ));
    }

    #[test]
    fn a_malformed_or_non_object_cell_is_not_semantics() {
        assert!(!cell_has_semantics("not json"));
        assert!(!cell_has_semantics("[1,2,3]"));
        assert!(!cell_has_semantics("null"));
    }
}
