//! Reader extension trait — no wrapper builders (geoparquet's lesson).
//!
//! `CityParquetReaderBuilder` is blanket-impl'd directly on
//! `parquet::arrow::arrow_reader::ArrowReaderBuilder<T>`, so every existing
//! builder method (`with_batch_size`, `with_projection`, `with_row_selection`,
//! ...) keeps working untouched alongside the three CityParquet-specific
//! methods added here. `CityParquetRecordBatchReader` is a thin wrapper
//! around the built `ParquetRecordBatchReader` that re-applies the rendered
//! schema (field metadata included) to every emitted batch, since a bare
//! `parquet`-crate read does not otherwise guarantee that metadata survives
//! (e.g. a `with_projection` reorders/subsets columns; files written by a
//! non-arrow-rs CityParquet writer may carry no embedded `ARROW:schema` at
//! all).

use std::sync::Arc;

use arrow_array::RecordBatch;
use arrow_schema::extension::EXTENSION_TYPE_NAME_KEY;
use arrow_schema::{Schema, SchemaRef};
use parquet::arrow::arrow_reader::{ArrowReaderBuilder, ParquetRecordBatchReader};
use parquet::file::metadata::RowGroupMetaData;
use parquet::file::statistics::Statistics;
use parquet::schema::types::ColumnPath;

use cityparquet_schema::{
    AttributeType, CityMetadata, CityParquetError, CityParquetSchema, GeoMetadata, Lod, Result,
};

/// The six fixed `bbox` struct leaves, matching [`crate::recipe::WriterRecipe`]
/// and [`cityparquet_schema::model::bbox_data_type`].
const BBOX_LEAVES: [&str; 6] = ["xmin", "ymin", "zmin", "xmax", "ymax", "zmax"];

/// Extension type name tagging a Utf8 column whose values are JSON text —
/// see [`cityparquet_schema::model`]'s `json_field` helper.
const ARROW_JSON_EXTENSION: &str = "arrow.json";

/// Extension trait adding CityParquet-aware reads directly to
/// `parquet::arrow::arrow_reader::ArrowReaderBuilder` — no wrapper builder.
pub trait CityParquetReaderBuilder: Sized {
    /// Parse this file's `city` (required) and `geo` (conditional) footer
    /// key-value metadata (spec `05-metadata.mdx`) back into their typed
    /// forms.
    fn cityparquet_footer(&self) -> Result<(CityMetadata, Option<GeoMetadata>)>;

    /// Convenience: just the `city` half of [`Self::cityparquet_footer`] —
    /// the common case, since most callers only need CityParquet's own
    /// metadata, not the GeoParquet mirror.
    fn cityparquet_metadata(&self) -> Result<CityMetadata> {
        self.cityparquet_footer().map(|(city, _geo)| city)
    }

    /// Rebuild the CityParquet-described Arrow schema (LoD/attribute/role
    /// field metadata re-attached) from this file's own KV metadata plus its
    /// actual column types — independent of whether the file happens to
    /// carry an embedded `ARROW:schema` entry, so it works for any
    /// CityParquet-conformant writer, not just this crate's. One caveat: the
    /// Json-vs-String disambiguation of Utf8 attribute columns relies on the
    /// `arrow.json` field tag inside the embedded `ARROW:schema` metadata, so
    /// files from writers that do not embed it degrade Json attributes to
    /// String (the reserved JSON columns are unaffected — those are re-tagged
    /// by name).
    fn cityparquet_arrow_schema(&self) -> Result<Arc<Schema>>;

    /// Restrict this builder to row groups whose `bbox.{x,y,z}{min,max}`
    /// column statistics 3D-intersect `bbox`. A row group missing any of the
    /// six leaf statistics is kept rather than pruned: this never silently
    /// drops rows it cannot prove are out of range.
    fn with_bbox_row_groups(self, bbox: [f64; 6]) -> Result<Self>;
}

impl<T> CityParquetReaderBuilder for ArrowReaderBuilder<T> {
    fn cityparquet_footer(&self) -> Result<(CityMetadata, Option<GeoMetadata>)> {
        let kvs = self
            .metadata()
            .file_metadata()
            .key_value_metadata()
            .ok_or_else(|| {
                CityParquetError::Metadata("parquet file has no key-value metadata".to_string())
            })?;
        CityMetadata::from_key_values(
            kvs.iter()
                .map(|kv| (kv.key.as_str(), kv.value.as_deref().unwrap_or(""))),
        )
    }

    fn cityparquet_arrow_schema(&self) -> Result<Arc<Schema>> {
        let meta = self.cityparquet_metadata()?;
        // The arrow schema `parquet` itself reconstructs for this file (from
        // an embedded `ARROW:schema` if present, else converted from the raw
        // Parquet physical schema) — used only to read back each attribute
        // column's *actual* arrow type and any `arrow.json` extension tag it
        // may carry, never trusted wholesale.
        let actual = self.schema();

        // LoDs: derived from the `geometry_<suffix>` names in the file's OWN
        // schema (never `geometry_properties_<suffix>`, which also starts with
        // `geometry_` but does not parse as a LoD suffix). The metadata no
        // longer carries a `reserved_columns` list — §13.1: reserved names are
        // fixed by the spec, so they are read straight off the schema.
        //
        // A column listed in `attributes` is an attribute, NOT reserved
        // (§13.1), even if it happens to be named like a geometry column for
        // some LoD the dataset does not otherwise use (e.g. a `geometry_lod3`
        // attribute in a LoD-2-only dataset — legal, since only geometry
        // columns for the dataset's actual LoDs are reserved). Exclude the
        // declared attributes first, so such a name is not mistaken for a LoD.
        let attribute_names: std::collections::HashSet<&str> =
            meta.attributes.iter().map(String::as_str).collect();
        // Every LoD, including LoD0, is suffixed (spec "Levels of detail"),
        // so a single `geometry_<suffix>` scan recovers the full LoD set —
        // there is no separate un-suffixed "footprint" column to special-case.
        let mut lods: Vec<Lod> = actual
            .fields()
            .iter()
            .filter(|f| !attribute_names.contains(f.name().as_str()))
            .filter_map(|f| f.name().strip_prefix("geometry_"))
            .filter_map(Lod::from_column_suffix)
            .collect();
        lods.sort();
        lods.dedup();

        let mut attributes = Vec::with_capacity(meta.attributes.len());
        for name in &meta.attributes {
            let field = actual.field_with_name(name).map_err(|_| {
                CityParquetError::Metadata(format!(
                    "attribute column '{name}' listed in metadata but absent from the file's schema"
                ))
            })?;
            let mut attr_type = AttributeType::from_arrow(field.data_type()).ok_or_else(|| {
                CityParquetError::Metadata(format!(
                    "attribute column '{name}' has an arrow type CityParquet cannot represent: {:?}",
                    field.data_type()
                ))
            })?;
            // Utf8 is ambiguous between String and Json; the field's own
            // `arrow.json` extension metadata (present at write time via
            // `cityparquet_schema::model::json_field`) is the only signal
            // that resolves it.
            if attr_type == AttributeType::String
                && field
                    .metadata()
                    .get(EXTENSION_TYPE_NAME_KEY)
                    .map(String::as_str)
                    == Some(ARROW_JSON_EXTENSION)
            {
                attr_type = AttributeType::Json;
            }
            attributes.push((name.clone(), attr_type));
        }

        // Whether THIS table's own physical schema carries the bare
        // un-suffixed `geometry` column — the dataset-wide
        // zero-analysis-geometry fallback (spec "Levels of detail" / §9):
        // every table in a dataset with NO LoD-bearing geometry anywhere
        // carries this pair. Distinguishes that case from a table that
        // simply carries NO geometry columns of its own (empty `lods` for a
        // DIFFERENT reason: spec "object-table-schema" — "a table whose
        // objects have no analysis geometry at all carries none of them" —
        // the per-module pruning case, where sibling tables DO have
        // LoD-bearing geometry) — both render `lods.is_empty()` here, but
        // only the former should get the synthesised bare quartet back.
        let has_bare_geometry = actual.field_with_name("geometry").is_ok();

        if lods.is_empty() && !has_bare_geometry {
            // This table needs no geometry section at all. Rendering the
            // real `attributes` through `CityParquetSchema::to_arrow_schema`
            // here would be UNSAFE: with `lods` empty, its `validate` reserves
            // the BARE `geometry`/`geometry_properties`/`material`/`texture`
            // names (the zero-analysis-geometry fallback's own vocabulary),
            // and an attribute legitimately named e.g. `material` — legal
            // dataset-wide, since the dataset's REAL LoDs only reserve
            // `material_lod<suffix>`, never the bare name — would spuriously
            // collide and error out (proven by the real-fixture regression
            // `attributes_named_like_appearance_columns_do_not_corrupt_export`).
            // So: render the reserved/template/other shape with NO attributes
            // (nothing to collide with), strip the synthesised bare quartet
            // `to_arrow_schema` always adds for empty `lods` (the ONLY shape
            // it knows for that case — never teach `CityParquetSchema` a
            // third "no geometry at all" `lods` state just for this
            // reader-side reconstruction), then splice in each attribute's
            // own ALREADY-RESOLVED physical field from `actual` (its type,
            // including the Json-vs-String disambiguation, was already
            // settled above) rather than re-deriving it through the
            // collision-prone path.
            let reserved_only = CityParquetSchema {
                lods: Vec::new(),
                attributes: Vec::new(),
                crs: None,
            }
            .to_arrow_schema()?;
            const BARE_GEOMETRY_NAMES: [&str; 4] =
                ["geometry", "geometry_properties", "material", "texture"];
            let mut fields: Vec<arrow_schema::Field> = reserved_only
                .fields()
                .iter()
                .filter(|f| !BARE_GEOMETRY_NAMES.contains(&f.name().as_str()))
                .map(|f| f.as_ref().clone())
                .collect();
            for (name, _) in &attributes {
                let field = actual.field_with_name(name).map_err(|_| {
                    CityParquetError::Metadata(format!(
                        "attribute column '{name}' listed in metadata but absent from the \
                         file's schema"
                    ))
                })?;
                fields.push(field.as_ref().clone());
            }
            return Ok(Arc::new(Schema::new(fields)));
        }

        let schema = CityParquetSchema {
            lods,
            attributes,
            crs: None,
        };
        // Tagged (zero-arg `to_arrow_schema`): this rendered schema is the
        // CANONICAL, self-describing view, not a reflection of the physical
        // file's on-disk self-description — a plain-BLOB file (written with
        // `--geoarrow` off) still reports its geometry columns here as
        // `geoarrow.wkb`-tagged. Harmless: the underlying Arrow data type is
        // `Binary` either way, so callers reading values see no difference.
        Ok(Arc::new(schema.to_arrow_schema()?))
    }

    fn with_bbox_row_groups(self, bbox: [f64; 6]) -> Result<Self> {
        let metadata = Arc::clone(self.metadata());
        let keep: Vec<usize> = (0..metadata.num_row_groups())
            .filter(|&i| row_group_intersects(metadata.row_group(i), &bbox))
            .collect();
        Ok(self.with_row_groups(keep))
    }
}

/// The `Statistics` for the `bbox.<leaf>` column chunk in `rg`, if the chunk
/// exists and carries statistics. Column path built with `ColumnPath::new`
/// over the two nested parts — `ColumnPath::from("bbox.xmin")` does NOT split
/// on `.` in parquet 58 and would never match any real column (this bit the
/// writer side in M2; see `crate::recipe::WriterRecipe`).
fn bbox_leaf_statistics<'a>(rg: &'a RowGroupMetaData, leaf: &str) -> Option<&'a Statistics> {
    let path = ColumnPath::new(vec!["bbox".to_string(), leaf.to_string()]);
    rg.columns()
        .iter()
        .find(|c| c.column_path() == &path)?
        .statistics()
}

fn bbox_leaf_min(rg: &RowGroupMetaData, leaf: &str) -> Option<f64> {
    match bbox_leaf_statistics(rg, leaf)? {
        Statistics::Double(v) => v.min_opt().copied(),
        _ => None,
    }
}

fn bbox_leaf_max(rg: &RowGroupMetaData, leaf: &str) -> Option<f64> {
    match bbox_leaf_statistics(rg, leaf)? {
        Statistics::Double(v) => v.max_opt().copied(),
        _ => None,
    }
}

/// Whether row group `rg`'s bbox column statistics 3D-intersect `query` (a
/// `[xmin, ymin, zmin, xmax, ymax, zmax]` bbox). Missing any of the six leaf
/// statistics keeps the row group (returns `true`) — pruning must never
/// silently drop rows it cannot prove are out of range.
///
/// `pub` (not `pub(crate)`): this is the exact row-group-intersection
/// predicate [`CityParquetReaderBuilder::with_bbox_row_groups`] uses to
/// prune, and `cityparquet-cli`'s bench harness needs to recompute the same
/// `row_groups_touched` count it reports — as a separate downstream crate it
/// cannot reach a `pub(crate)` item, and re-deriving its own copy of this
/// logic is exactly the duplication this signature closes.
pub fn row_group_intersects(rg: &RowGroupMetaData, query: &[f64; 6]) -> bool {
    let mins = [
        bbox_leaf_min(rg, BBOX_LEAVES[0]),
        bbox_leaf_min(rg, BBOX_LEAVES[1]),
        bbox_leaf_min(rg, BBOX_LEAVES[2]),
    ];
    let maxs = [
        bbox_leaf_max(rg, BBOX_LEAVES[3]),
        bbox_leaf_max(rg, BBOX_LEAVES[4]),
        bbox_leaf_max(rg, BBOX_LEAVES[5]),
    ];
    let (Some(min0), Some(min1), Some(min2)) = (mins[0], mins[1], mins[2]) else {
        return true;
    };
    let (Some(max0), Some(max1), Some(max2)) = (maxs[0], maxs[1], maxs[2]) else {
        return true;
    };
    box_intersects_query([min0, min1, min2], [max0, max1, max2], query)
}

/// Axis-aligned 3D interval-overlap test: whether the box `[box_min, box_max]`
/// intersects `query` (`[xmin, ymin, zmin, xmax, ymax, zmax]`). The single
/// numeric predicate shared by [`row_group_intersects`] (bounds read from
/// Parquet column statistics) and [`crate::query::bbox_query`]'s row-level
/// exactness filter (bounds decoded from each row's own `bbox` struct
/// column) — one implementation of "do these two boxes overlap", reused by
/// both the coarse (row-group) and exact (row) tests.
pub(crate) fn box_intersects_query(box_min: [f64; 3], box_max: [f64; 3], query: &[f64; 6]) -> bool {
    for axis in 0..3 {
        if box_max[axis] < query[axis] || box_min[axis] > query[axis + 3] {
            return false;
        }
    }
    true
}

/// Thin wrapper around a built `ParquetRecordBatchReader` that re-applies the
/// rendered [`CityParquetReaderBuilder::cityparquet_arrow_schema`] (field
/// metadata included) to every emitted batch, since the bare reader's own
/// `RecordBatch::schema()` is not guaranteed to carry it (see the module docs).
pub struct CityParquetRecordBatchReader {
    inner: ParquetRecordBatchReader,
    schema: SchemaRef,
}

impl CityParquetRecordBatchReader {
    /// Wrap `inner`, stamping every emitted batch with `schema`.
    pub fn new(inner: ParquetRecordBatchReader, schema: SchemaRef) -> Self {
        Self { inner, schema }
    }

    /// The schema every batch from this reader carries.
    pub fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }
}

impl Iterator for CityParquetRecordBatchReader {
    type Item = Result<RecordBatch>;

    fn next(&mut self) -> Option<Self::Item> {
        let batch = match self.inner.next()? {
            Ok(batch) => batch,
            Err(e) => return Some(Err(CityParquetError::from(e))),
        };
        let rebuilt = RecordBatch::try_new(Arc::clone(&self.schema), batch.columns().to_vec());
        Some(rebuilt.map_err(CityParquetError::from))
    }
}
