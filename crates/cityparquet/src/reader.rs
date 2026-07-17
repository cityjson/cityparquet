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
    AttributeType, CityParquetError, CityParquetMetadata, CityParquetSchema, Lod, Result,
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
    /// Parse this file's CityParquet key-value metadata (spec `notes/spec.md`
    /// § metadata keys) back into a typed [`CityParquetMetadata`].
    fn cityparquet_metadata(&self) -> Result<CityParquetMetadata>;

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
    fn cityparquet_metadata(&self) -> Result<CityParquetMetadata> {
        let kvs = self
            .metadata()
            .file_metadata()
            .key_value_metadata()
            .ok_or_else(|| {
                CityParquetError::Metadata("parquet file has no key-value metadata".to_string())
            })?;
        CityParquetMetadata::from_key_values(
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
        let mut lods: Vec<Lod> = actual
            .fields()
            .iter()
            .filter_map(|f| f.name().strip_prefix("geometry_"))
            .filter_map(Lod::from_column_suffix)
            .collect();
        lods.sort();
        lods.dedup();

        let mut attributes = Vec::with_capacity(meta.attribute_columns.len());
        for name in &meta.attribute_columns {
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
