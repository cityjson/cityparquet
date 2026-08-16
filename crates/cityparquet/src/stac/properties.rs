//! Derive the `city3d:*` property set from a written CityParquet package.
//!
//! Everything here is read back from the files on disk — the Parquet footer
//! and the rebuilt Arrow schema. Nothing is accumulated during conversion.
//! That is what makes spec §13.2's rule ("where the STAC Item and the Parquet
//! footer disagree, the footer is authoritative") true by construction, and
//! it means the encoder's own type decisions are reported rather than
//! independently re-inferred.

use std::collections::{BTreeMap, BTreeSet};
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
use cityparquet_schema::{AttributeType, CityMetadata, CityParquetError, Result};
use parquet::arrow::ProjectionMask;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

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
            CityParquetError::io_source(format!("cannot read {}", manifest_path.display()), e)
        })?;
        let item: Item = serde_json::from_str(&text)?;
        let (table_names, sidecar_files) = classify_assets(&item)?;

        Ok(Self {
            dir: dir.to_path_buf(),
            tables: table_names.iter().map(|name| dir.join(name)).collect(),
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

/// Classifies `item`'s assets into (object table names, sidecar names), both
/// with any `"./"` href prefix stripped — the pure parsing core shared by
/// [`PackageTables::open`] (filesystem) and
/// [`table_names_from_manifest_bytes`] (an already-fetched manifest, e.g. via
/// `object_store` over HTTP). Errors on a duplicate object-table name or an
/// empty table list, exactly as `open` did before this was extracted.
fn classify_assets(item: &Item) -> Result<(Vec<String>, Vec<String>)> {
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
            tables.push(name);
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

    Ok((tables, sidecar_files))
}

/// The relative object-table names an already-fetched `metadata.json`'s
/// bytes declare, in manifest order — the HTTP-callable counterpart of
/// [`PackageTables::open`] for callers that already have the manifest bytes
/// (e.g. fetched via `object_store`) rather than a local directory.
pub fn table_names_from_manifest_bytes(bytes: &[u8]) -> Result<Vec<String>> {
    let item: Item = serde_json::from_slice(bytes)?;
    let (tables, _sidecars) = classify_assets(&item)?;
    Ok(tables)
}

/// The LoDs *one table's* schema carries, as a set.
///
/// Read from each geometry field's `cityparquet:lod` tag rather than parsed
/// back out of column names — every real LoD, including LoD0, is suffixed
/// (`geometry_lod0_0`, `geometry_lod2_2`, …) and carries that tag directly.
/// The bare, un-suffixed `geometry` column only appears in the wide-schema
/// scaffold and in legacy/foreign files (the current writer prunes it for a
/// geometry-less table; a dataset with only
/// `GeometryInstance`s, or none — §9); it never carries a `cityparquet:lod`
/// tag, so matching its name here never contributes a LoD — it is included
/// only so the loop recognises it as a geometry column (rather than treating
/// it as unrelated) when deciding whether to look for the tag.
///
/// Since M2, a module's table carries only the LoD/appearance columns its own
/// rows use (spec "The footer describes the file it lives in — nothing
/// wider"), so this is per-table by construction — [`derive_from_footer`]
/// unions the result across every table to get the dataset-level
/// `city3d:lods`, exactly as it already does for `co_types`.
fn lods_from_schema(schema: &Schema) -> BTreeSet<Lod> {
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
    lods
}

/// The attribute definitions *one table* carries.
///
/// The declared attribute columns come from that table's own footer
/// (`meta.attributes`); each one's type comes from the rebuilt schema. A
/// `Json` attribute is stored as Utf8 and is only distinguishable by its
/// `arrow.json` extension tag, so that is checked before falling back to
/// [`AttributeType::from_arrow`] — otherwise every JSON-valued attribute
/// would be reported as a plain `String`. This mirrors the same
/// disambiguation in [`crate::reader`].
///
/// A column whose Arrow type maps to no CityParquet type is skipped rather
/// than guessed at; the count is returned so the caller can report it.
///
/// Unlike [`lods_from_schema`], `meta.attributes` is **not** module-pruned by
/// this writer today: every table in a package is stamped with the same
/// dataset-wide attribute-column list (only geometry/appearance LoD columns
/// are pruned per M2), so calling this once, on any single table, already
/// returns the whole package's attributes. [`derive_from_footer`] still
/// unions this across every table for `city3d:attributes` — spec
/// `city.attributes` is a per-*file* field, so a conformant writer that DOES
/// prune attribute columns per module is legal, and reading only the first
/// table would silently under-report for one.
fn attributes_from_schema(
    schema: &Schema,
    meta: &CityMetadata,
) -> (Vec<AttributeDefinition>, usize) {
    let mut defs = Vec::with_capacity(meta.attributes.len());
    let mut skipped = 0usize;

    for name in &meta.attributes {
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

/// Whether one `geometry_properties` STRUCT row describes semantic surfaces:
/// its `surfaces` (child 1) or `face_semantics` (child 2) is non-null.
///
/// **The struct being non-null is not the question.** Every stored geometry
/// gets a non-null `geometry_properties` cell with at least `type` set
/// (spec), and adds `surfaces` / `face_semantics` only when the source
/// geometry actually carried semantics. So a struct-non-null-count test
/// answers "does this package have geometry", which is true of essentially
/// every package — the row's `surfaces`/`face_semantics` children have to be
/// inspected.
fn row_has_semantics(props: &arrow_array::StructArray, row: usize) -> bool {
    if props.is_null(row) {
        return false;
    }
    let surfaces = props.column(1);
    let face_semantics = props.column(2);
    surfaces.is_valid(row) || face_semantics.is_valid(row)
}

/// Whether any `geometry_properties*` column in this file describes semantic
/// surfaces.
///
/// Always reads: no footer statistic can distinguish a cell carrying only
/// `type` from one that also carries `surfaces`/`face_semantics`. A
/// `geometry_properties*` field now resolves to SEVERAL Parquet leaf columns
/// (`type`, `surfaces`, `face_semantics.list.item`,
/// `shells.list.item.list.item` — spec "Geometry properties and semantics"),
/// so every leaf under the field's top-level name is projected together,
/// which Arrow reconstructs back into one `StructArray` column.
fn file_has_semantic_surfaces(path: &Path, schema: &Schema) -> Result<bool> {
    for field in schema.fields() {
        if !is_reserved_column_for(field.name(), "geometry_properties") {
            continue;
        }
        let file = fs::File::open(path).map_err(|e| {
            CityParquetError::io_source(format!("cannot open {}", path.display()), e)
        })?;
        let builder = ParquetRecordBatchReaderBuilder::try_new(file).map_err(|e| {
            CityParquetError::parquet_source(format!("cannot open {}", path.display()), e)
        })?;
        let descr = builder.parquet_schema();
        let leaves: Vec<usize> = (0..descr.num_columns())
            .filter(|&i| descr.column(i).path().parts().first() == Some(field.name()))
            .collect();
        if leaves.is_empty() {
            continue;
        }
        let mask = ProjectionMask::leaves(descr, leaves);
        let reader = builder
            .with_projection(mask)
            .build()
            .map_err(|e| CityParquetError::parquet_source("cannot build reader", e))?;

        for batch in reader {
            let batch = batch.map_err(|e| {
                CityParquetError::parquet_source(format!("read {}", path.display()), e)
            })?;
            if batch.num_columns() == 0 {
                continue;
            }
            let Some(props) = batch
                .column(0)
                .as_any()
                .downcast_ref::<arrow_array::StructArray>()
            else {
                continue;
            };
            for row in 0..props.len() {
                if row_has_semantics(props, row) {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
}

/// Whether a field is one of the reserved columns for `base`, across every LoD.
///
/// Matches the LoD-suffixed form `base_lod<major>_<minor>` — which the reference
/// writer always emits, since every geometry column now carries an LoD suffix
/// (spec "Levels of detail"; there is no un-suffixed footprint column) — and
/// also a bare, un-suffixed `base`. The writer does not emit the bare form for
/// LoD-bearing geometry, but it stays matched defensively: the un-suffixed names
/// remain reserved for a geometry-less table (no LoD to suffix by), and files
/// from other tools may use them. Matching only `base_lod*` would make semantic
/// surfaces on such a column invisible.
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
        let file = fs::File::open(path).map_err(|e| {
            CityParquetError::io_source(format!("cannot open {}", path.display()), e)
        })?;
        let builder = ParquetRecordBatchReaderBuilder::try_new(file).map_err(|e| {
            CityParquetError::parquet_source(format!("cannot open {}", path.display()), e)
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
            .map_err(|e| CityParquetError::parquet_source("cannot build reader", e))?;

        for batch in reader {
            let batch = batch.map_err(|e| {
                CityParquetError::parquet_source(format!("read {}", path.display()), e)
            })?;
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
/// `lods` and `attributes` are genuine dataset-level UNIONS across every
/// object table — spec "The footer describes the file it lives in — nothing
/// wider": *"The `metadata.json` STAC Item is the dataset-level view: it
/// aggregates across every file in the package (e.g. `city3d:lods` is the
/// union over all tables)"*. `co_types` and `semantic_surfaces` already
/// aggregate the same way (the pattern this fix extends to
/// `lods`/`attributes`); `textures`/`materials` are the presence of the
/// appearance sidecars, not per-table at all.
///
/// **Why this matters concretely for `lods`, and only theoretically (today)
/// for `attributes`.** Since M2 each module's table is pruned to only the
/// **geometry/appearance** LoD columns its own rows use (spec
/// "object-table-schema"; `crate::scan`'s `module_lods` doc comment), so
/// reading only the first table — as this function used to — silently drops
/// any LoD that exists only in a later table (e.g. a railway-only module at
/// LoD 3 first, a building module spanning {0, 1.2, 1.3, 2.2, 3} later).
/// **Attribute columns are not module-pruned** by this writer — every table
/// in a package renders the same dataset-wide attribute column set — so
/// today `attributes` unioned across tables and `attributes` read from the
/// first table alone happen to agree. `attributes` is still unioned here:
/// `city.attributes` is a per-*file* field in the spec (a table's own
/// inferred attribute columns), so a differently-pruning writer is legal, and
/// reading only the first table would be a latent bug waiting for one.
pub fn derive_from_footer(tables: &PackageTables) -> Result<City3dProperties> {
    let mut props = City3dProperties::new();
    let mut total_rows: u64 = 0;
    // Semantic-surface presence is a property of the DATA, judged across every
    // object table, not just the first — a by-type package may carry semantics
    // on one type and none on another. Appearance is judged separately, from the
    // presence of the definition sidecars (after the loop).
    let mut semantics: Option<bool> = None;
    let mut lods: BTreeSet<Lod> = BTreeSet::new();
    // Keyed by name (see `merge_attributes`) so a `BTreeMap` also gives a
    // stable, sorted `city3d:attributes` for free, matching `lods`/
    // `co_types`'s own sorted-and-deduplicated output.
    let mut attributes: BTreeMap<String, AttributeDefinition> = BTreeMap::new();

    for path in &tables.tables {
        let file = fs::File::open(path).map_err(|e| {
            CityParquetError::io_source(format!("cannot open {}", path.display()), e)
        })?;
        let builder = ParquetRecordBatchReaderBuilder::try_new(file).map_err(|e| {
            CityParquetError::parquet_source(format!("cannot open {}", path.display()), e)
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
        let meta = builder.cityparquet_metadata()?;

        // `source_version` (the SOURCE schema version, e.g. CityJSON 2.0) is
        // genuinely dataset-wide and single-valued: every table in one
        // package came from the same conversion of the same source, so it is
        // read once, from the first table, rather than accumulated — unlike
        // `lods`/`attributes` below, which this function treats as per-table
        // and unions (see this function's doc comment for why that is a real
        // concern for `lods` and a defensive one, today, for `attributes`).
        if props.version.is_none() {
            props.version = meta.source_version.clone();
        }
        lods.extend(lods_from_schema(&schema));
        let (table_attributes, _skipped) = attributes_from_schema(&schema, &meta);
        merge_attributes(&mut attributes, table_attributes);

        // Semantic surfaces live in the `geometry_properties*` columns (§8),
        // but a null-count test would answer the wrong question — see
        // `cell_has_semantics`.
        semantics = merge_presence(semantics, Some(file_has_semantic_surfaces(path, &schema)?));
    }

    props.lods = lods.into_iter().map(|l| l.to_string()).collect();
    props.attributes = attributes.into_values().collect();

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

/// Fold one table's attribute definitions into the running dataset-level
/// union, keyed by name so the same attribute reported by two tables
/// collapses to a single entry.
///
/// Kept **first-seen** on a name collision: two modules disagreeing on the
/// SAME attribute name's inferred type would be surprising (the same
/// attribute name is expected to mean the same thing dataset-wide), and
/// there is no principled way to pick a "better" type between two tables'
/// independent inferences — so whichever table `derive_from_footer` visits
/// first (manifest/table order) wins, rather than a later table silently
/// overwriting it.
fn merge_attributes(
    dest: &mut BTreeMap<String, AttributeDefinition>,
    defs: Vec<AttributeDefinition>,
) {
    for def in defs {
        dest.entry(def.name.clone()).or_insert(def);
    }
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
    use std::collections::BTreeMap;
    use std::path::Path;

    use arrow_array::StructArray;
    use city3d_stac_types::metadata::{AttributeDefinition, AttributeType};

    use super::{PackageTables, merge_attributes, row_has_semantics};
    use crate::geometry_properties::{GeometryProperties, GeometryPropertiesBuilder};

    /// The union/dedup semantics [`merge_attributes`] promises, proven
    /// directly against two synthetic tables' worth of definitions — plain
    /// Rust data, not CityJSON, so this isn't subject to this repo's
    /// real-fixture-only discipline for *model content*. It has to be proven
    /// this way: today's writer never module-prunes attribute columns (only
    /// geometry/appearance columns are pruned per M2), so no real converted
    /// package can produce two tables with genuinely different attribute
    /// sets for an end-to-end test to exercise — see
    /// `stac_derive_real_data.rs::city3d_attributes_spans_every_module_present_in_the_package`'s
    /// doc comment for the same point from the integration-test side.
    #[test]
    fn merge_attributes_unions_by_name_and_keeps_first_seen_on_conflict() {
        let mut dest: BTreeMap<String, AttributeDefinition> = BTreeMap::new();

        merge_attributes(
            &mut dest,
            vec![
                AttributeDefinition::new("function", AttributeType::String),
                AttributeDefinition::new("class", AttributeType::String),
            ],
        );
        // A second table: one brand-new name (`b3_bouwlagen`, from a
        // different module) and one NAME COLLISION with a conflicting type
        // (`class` as `Number` here, `String` above) that must NOT win.
        merge_attributes(
            &mut dest,
            vec![
                AttributeDefinition::new("class", AttributeType::Number),
                AttributeDefinition::new("b3_bouwlagen", AttributeType::Number),
            ],
        );

        let names: Vec<&str> = dest.keys().map(String::as_str).collect();
        assert_eq!(
            names,
            vec!["b3_bouwlagen", "class", "function"],
            "the union must contain every distinct name from both tables"
        );
        assert_eq!(
            dest["class"].attr_type,
            AttributeType::String,
            "the FIRST table's type for a colliding name must win, not the second's"
        );
    }

    /// Builds a one-row `geometry_properties` `StructArray` from `props`, the
    /// SAME builder the writer uses — so these tests exercise `row_has_semantics`
    /// against the real physical shape, not a hand-rolled stand-in.
    fn one_row(props: &GeometryProperties) -> StructArray {
        let mut b = GeometryPropertiesBuilder::new();
        b.append_value(props).unwrap();
        let array = b.finish();
        array
            .as_any()
            .downcast_ref::<StructArray>()
            .unwrap()
            .clone()
    }

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
    /// least `type` (`crate::encode::compute_geometry_properties`), so a
    /// struct-non-null test would report semantic surfaces for any package
    /// that has geometry at all. These cases pin the distinction the
    /// integration tests cannot: every fixture in this repo carries some
    /// semantics, so the negative case has no real dataset to come from.
    #[test]
    fn a_type_only_row_is_not_semantics() {
        for type_name in ["Solid", "MultiSurface"] {
            let row = one_row(&GeometryProperties {
                type_name: type_name.to_string(),
                surfaces: None,
                face_semantics: None,
                shells: None,
            });
            assert!(!row_has_semantics(&row, 0));
        }
        // A Solid with `shells` populated but no semantics at all.
        let row = one_row(&GeometryProperties {
            type_name: "Solid".to_string(),
            surfaces: None,
            face_semantics: None,
            shells: Some(vec![vec![6]]),
        });
        assert!(!row_has_semantics(&row, 0));
    }

    #[test]
    fn surfaces_or_face_semantics_is_semantics() {
        let row = one_row(&GeometryProperties {
            type_name: "MultiSurface".to_string(),
            surfaces: Some(serde_json::json!([{"type": "RoofSurface"}])),
            face_semantics: None,
            shells: None,
        });
        assert!(row_has_semantics(&row, 0));

        let row = one_row(&GeometryProperties {
            type_name: "MultiSurface".to_string(),
            surfaces: None,
            face_semantics: Some(vec![Some(0), Some(0), Some(1)]),
            shells: None,
        });
        assert!(row_has_semantics(&row, 0));
    }

    #[test]
    fn a_null_struct_row_is_not_semantics() {
        let mut b = GeometryPropertiesBuilder::new();
        b.append_null();
        let array = b.finish();
        let row = array.as_any().downcast_ref::<StructArray>().unwrap();
        assert!(!row_has_semantics(row, 0));
    }
}
