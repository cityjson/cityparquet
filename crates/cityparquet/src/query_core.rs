//! Batch-level core shared by [`crate::query`] (sync, `File`-backed) and
//! `crate::query_async` (async, `object_store`-backed): predicate
//! construction, projection/row-filter assembly, row-group pruning counts,
//! and per-batch aggregation. The two entry-point modules differ ONLY in
//! how a reader is opened and how batches are pulled (iterator vs stream,
//! plus the async path's per-batch restamp); everything else lives here
//! exactly once (review P3 — the former self-acknowledged duplication at
//! the top of `query_async.rs`).

use arrow_array::types::Int32Type;
use arrow_array::{
    Array, ArrayAccessor, BooleanArray, DictionaryArray, Float64Array, Int64Array, RecordBatch,
    StringArray, StructArray,
};
use arrow_schema::DataType;
use parquet::arrow::ProjectionMask;
use parquet::arrow::arrow_reader::{ArrowPredicateFn, RowFilter};
use parquet::file::metadata::{ParquetMetaData, RowGroupMetaData};
use parquet::file::statistics::Statistics;
use parquet::schema::types::{ColumnPath, SchemaDescriptor};

use cityparquet_schema::{CityMetadata, CityParquetError, Result};

use crate::decode::{DecodedObject, decode_batch};
use crate::reader::{box_intersects_query, row_group_intersects};
use crate::wkb_read::DecodedKind;

/// The result of a [`crate::query::full_read`]: the total feature (row) count
/// and a stable geometry-work metric (`boundary_count`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FullReadResult {
    pub feature_count: u64,
    /// Total number of decoded surfaces/faces across every non-null
    /// geometry cell read (`DecodedKind::MultiPolygon`/`PolyhedralSurface`'s
    /// outer `Vec` length, summed, recursing into
    /// `DecodedKind::GeometryCollection` members). `MultiPoint`/
    /// `MultiLineString` geometries contribute 0 — they have no surfaces.
    /// Deliberately simple and deterministic: this is the metric later
    /// cross-format ("full read forces materialisation") comparisons key
    /// off, so its definition must be stable across formats, not a
    /// CityParquet-specific detail.
    pub boundary_count: u64,
}

/// Total surface/face count in `kind`, recursing into
/// [`DecodedKind::GeometryCollection`] members.
fn surface_count(kind: &DecodedKind) -> u64 {
    match kind {
        DecodedKind::MultiPoint(_) | DecodedKind::MultiLineString(_) => 0,
        DecodedKind::MultiPolygon(surfaces) | DecodedKind::PolyhedralSurface(surfaces) => {
            surfaces.len() as u64
        }
        DecodedKind::GeometryCollection(members) => members.iter().map(surface_count).sum(),
    }
}

/// The result of an exact [`crate::query::bbox_query`]: the matching object
/// `id`s, plus how many of the table's row groups were pruned away vs.
/// actually touched.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BBoxQueryResult {
    /// `id`s of every row whose `bbox` truly 3D-intersects the query — an
    /// exact result, not the row-group-pruning superset.
    pub ids: Vec<String>,
    /// Total row groups in the table.
    pub row_groups_total: usize,
    /// Row groups [`row_group_intersects`] could not rule out (the same
    /// count [`crate::reader::CityParquetReaderBuilder::with_bbox_row_groups`]
    /// keeps for the scan below).
    pub row_groups_touched: usize,
}

/// One row's `bbox` struct leaves at `row`, or `None` if the struct itself
/// is null at that row (an object with no bbox has no extent to test, so it
/// can never match a bbox query).
fn row_bbox(bbox_col: &StructArray, row: usize) -> Result<Option<([f64; 3], [f64; 3])>> {
    if bbox_col.is_null(row) {
        return Ok(None);
    }
    let leaf = |name: &str| -> Result<f64> {
        Ok(bbox_col
            .column_by_name(name)
            .and_then(|c| c.as_any().downcast_ref::<Float64Array>())
            .ok_or_else(|| {
                CityParquetError::Schema(format!("bbox.{name} column missing or not Float64"))
            })?
            .value(row))
    };
    let min = [leaf("xmin")?, leaf("ymin")?, leaf("zmin")?];
    let max = [leaf("xmax")?, leaf("ymax")?, leaf("zmax")?];
    Ok(Some((min, max)))
}

/// An attribute-column predicate for [`crate::query::attr_filter`]. `Eq`
/// compares against a string (for a `Utf8`/`Dictionary<_, Utf8>` column) or a
/// number (for an `Int64`/`Float64` column) as appropriate to the target
/// column's actual Arrow type; `Ge`/`Le`/`Range` always compare numerically as
/// `f64` and only apply to `Int64`/`Float64` columns. A row whose value is
/// null never matches any variant.
#[derive(Debug, Clone, PartialEq)]
pub enum AttrPredicate {
    /// Equality against a string or number, dispatched on the column's Arrow
    /// type.
    Eq(serde_json::Value),
    /// `value >= bound`.
    Ge(f64),
    /// `value <= bound`.
    Le(f64),
    /// `lo <= value <= hi`.
    Range(f64, f64),
}

/// Build the `BooleanArray` deciding which rows of `array` (the single
/// projected `column`) satisfy `pred`, dispatching on `array`'s Arrow
/// `DataType`. Null cells always decide `false` (never match).
pub(crate) fn evaluate_attr_predicate(
    column: &str,
    array: &dyn Array,
    pred: &AttrPredicate,
) -> Result<BooleanArray> {
    let schema_err = |msg: String| CityParquetError::Schema(msg);

    match array.data_type() {
        DataType::Utf8 => {
            let values = array
                .as_any()
                .downcast_ref::<StringArray>()
                .ok_or_else(|| schema_err(format!("column '{column}' is not Utf8")))?;
            let AttrPredicate::Eq(serde_json::Value::String(want)) = pred else {
                return Err(schema_err(format!(
                    "column '{column}' is Utf8; only `Eq(<string>)` applies, got {pred:?}"
                )));
            };
            Ok(BooleanArray::from_iter((0..values.len()).map(|i| {
                Some(!values.is_null(i) && values.value(i) == want)
            })))
        }
        DataType::Dictionary(key_type, value_type) => {
            if key_type.as_ref() != &DataType::Int32 || value_type.as_ref() != &DataType::Utf8 {
                return Err(schema_err(format!(
                    "column '{column}' is Dictionary<{key_type:?}, {value_type:?}>; only \
                     Dictionary<Int32, Utf8> is supported"
                )));
            }
            let dict = array
                .as_any()
                .downcast_ref::<DictionaryArray<Int32Type>>()
                .ok_or_else(|| {
                    schema_err(format!(
                        "column '{column}' is not a Dictionary<Int32, Utf8>"
                    ))
                })?;
            let values = dict.downcast_dict::<StringArray>().ok_or_else(|| {
                schema_err(format!("column '{column}' dictionary values are not Utf8"))
            })?;
            let AttrPredicate::Eq(serde_json::Value::String(want)) = pred else {
                return Err(schema_err(format!(
                    "column '{column}' is a dictionary column; only `Eq(<string>)` applies, got {pred:?}"
                )));
            };
            // `TypedDictionaryArray::value` is unchecked w.r.t. nulls (a null
            // key position may point at any dictionary entry, or none), so
            // `dict.is_null(i)` must gate every lookup.
            Ok(BooleanArray::from_iter((0..dict.len()).map(|i| {
                Some(!dict.is_null(i) && values.value(i) == want.as_str())
            })))
        }
        DataType::Int64 => {
            let values = array
                .as_any()
                .downcast_ref::<Int64Array>()
                .ok_or_else(|| schema_err(format!("column '{column}' is not Int64")))?;
            evaluate_numeric_predicate(column, values.len(), pred, |i| {
                (!values.is_null(i)).then(|| values.value(i) as f64)
            })
        }
        DataType::Float64 => {
            let values = array
                .as_any()
                .downcast_ref::<Float64Array>()
                .ok_or_else(|| schema_err(format!("column '{column}' is not Float64")))?;
            evaluate_numeric_predicate(column, values.len(), pred, |i| {
                (!values.is_null(i)).then(|| values.value(i))
            })
        }
        other => Err(schema_err(format!(
            "column '{column}' has an arrow type attr_filter cannot filter on: {other:?}"
        ))),
    }
}

/// Shared numeric-column evaluation for `Int64`/`Float64` arrays: `get(i)`
/// returns `None` for a null cell (never matches), else the cell's `f64`
/// value to compare against `pred`. Rejects `Eq` with a non-numeric
/// [`serde_json::Value`] (a string `Eq` against a numeric column is a schema
/// mismatch, not a "no match").
fn evaluate_numeric_predicate(
    column: &str,
    len: usize,
    pred: &AttrPredicate,
    get: impl Fn(usize) -> Option<f64>,
) -> Result<BooleanArray> {
    let matches: Box<dyn Fn(f64) -> bool> = match pred {
        AttrPredicate::Eq(v) => {
            let want = v.as_f64().ok_or_else(|| {
                CityParquetError::Schema(format!(
                    "column '{column}' is numeric; `Eq` needs a JSON number, got {v:?}"
                ))
            })?;
            Box::new(move |x| x == want)
        }
        AttrPredicate::Ge(bound) => {
            let bound = *bound;
            Box::new(move |x| x >= bound)
        }
        AttrPredicate::Le(bound) => {
            let bound = *bound;
            Box::new(move |x| x <= bound)
        }
        AttrPredicate::Range(lo, hi) => {
            let (lo, hi) = (*lo, *hi);
            Box::new(move |x| x >= lo && x <= hi)
        }
    };
    Ok(BooleanArray::from_iter(
        (0..len).map(|i| Some(get(i).is_some_and(&*matches))),
    ))
}

/// The result of an [`crate::query::attr_stats`] columnar aggregation over a
/// numeric attribute column: `min`/`max`/`sum`/`count` (`count` of non-null
/// values; nulls are excluded from every field). `min <= max` whenever
/// `count > 0`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AttrStats {
    pub min: f64,
    pub max: f64,
    pub sum: f64,
    pub count: u64,
}

/// The `Statistics` for the top-level `column` chunk in `rg`, if the chunk
/// exists and carries statistics. Mirrors
/// [`crate::reader`]'s `bbox_leaf_statistics`, but over a single-part
/// [`ColumnPath`] (a plain attribute column, not a nested `bbox.<leaf>`
/// struct field).
fn column_statistics<'a>(rg: &'a RowGroupMetaData, column: &str) -> Option<&'a Statistics> {
    let path = ColumnPath::new(vec![column.to_string()]);
    rg.columns()
        .iter()
        .find(|c| c.column_path() == &path)?
        .statistics()
}

/// `stats`'s min/max as `f64`, for the two numeric Parquet physical types
/// CityParquet attribute columns use (`Int64`/`Double`). `None` if `stats`
/// is some other physical type, or if the chunk has no defined min/max (e.g.
/// every value in the chunk is null).
fn statistics_min_max(stats: &Statistics) -> Option<(f64, f64)> {
    match stats {
        Statistics::Int64(v) => Some((*v.min_opt()? as f64, *v.max_opt()? as f64)),
        Statistics::Double(v) => Some((*v.min_opt()?, *v.max_opt()?)),
        _ => None,
    }
}

/// Fail fast with a clear error when `column` is not in the file's schema
/// (both transports run this before consuming their builder).
pub(crate) fn require_column(schema: &arrow_schema::Schema, column: &str) -> Result<()> {
    schema.field_with_name(column).map(|_| ()).map_err(|_| {
        CityParquetError::Schema(format!("column '{column}' missing from the file's schema"))
    })
}

/// `(row_groups_total, row_groups_touched)` for a bbox query — the exact
/// same [`row_group_intersects`] predicate `with_bbox_row_groups` itself
/// uses, so the counts can never drift from what the scan reads.
pub(crate) fn bbox_row_group_counts(
    metadata: &ParquetMetaData,
    query_bbox: &[f64; 6],
) -> (usize, usize) {
    let total = metadata.num_row_groups();
    let touched = (0..total)
        .filter(|&i| row_group_intersects(metadata.row_group(i), query_bbox))
        .count();
    (total, touched)
}

/// Fold one (already restamped, on the async path) batch into a running
/// [`FullReadResult`]: row count plus decoded surface/face count.
pub(crate) fn accumulate_full_read(
    acc: &mut FullReadResult,
    batch: &RecordBatch,
    meta: &CityMetadata,
) -> Result<()> {
    acc.feature_count += batch.num_rows() as u64;
    let decoded = decode_batch(batch, meta)?;
    for object in &decoded {
        for (_, geometry, _) in &object.geometries {
            acc.boundary_count += surface_count(&geometry.kind);
        }
    }
    Ok(())
}

/// Exact row-level bbox filter over one `id`/`bbox`-projected batch,
/// appending matching ids.
pub(crate) fn collect_bbox_ids(
    batch: &RecordBatch,
    query_bbox: &[f64; 6],
    ids: &mut Vec<String>,
) -> Result<()> {
    let id_col = batch
        .column_by_name("id")
        .and_then(|c| c.as_any().downcast_ref::<StringArray>())
        .ok_or_else(|| CityParquetError::Schema("'id' column missing or not Utf8".to_string()))?;
    let bbox_col = batch
        .column_by_name("bbox")
        .and_then(|c| c.as_any().downcast_ref::<StructArray>())
        .ok_or_else(|| {
            CityParquetError::Schema("'bbox' column missing or not a struct".to_string())
        })?;
    for row in 0..batch.num_rows() {
        let Some((row_min, row_max)) = row_bbox(bbox_col, row)? else {
            continue;
        };
        if box_intersects_query(row_min, row_max, query_bbox) {
            ids.push(id_col.value(row).to_string());
        }
    }
    Ok(())
}

/// The single-column attribute-predicate [`RowFilter`] both transports
/// install: `pred` evaluated by [`evaluate_attr_predicate`] over a
/// `column`-only projection.
pub(crate) fn attr_predicate_row_filter(
    parquet_schema: &SchemaDescriptor,
    column: &str,
    pred: &AttrPredicate,
) -> RowFilter {
    let predicate_mask = ProjectionMask::columns(parquet_schema, [column]);
    let owned_column = column.to_string();
    let owned_pred = pred.clone();
    let predicate_fn = ArrowPredicateFn::new(predicate_mask, move |batch: RecordBatch| {
        let array = batch.column(0);
        evaluate_attr_predicate(&owned_column, array.as_ref(), &owned_pred)
            .map_err(arrow_schema::ArrowError::from)
    });
    RowFilter::new(vec![Box::new(predicate_fn)])
}

/// The `id == <target>` [`RowFilter`] both transports install for
/// `id_lookup`: the predicate's own projection is `id` alone; the output
/// projection stays untouched (the full row is decoded on a hit).
pub(crate) fn id_row_filter(parquet_schema: &SchemaDescriptor, id: &str) -> RowFilter {
    let predicate_mask = ProjectionMask::columns(parquet_schema, ["id"]);
    let owned_id = id.to_string();
    let predicate_fn = ArrowPredicateFn::new(predicate_mask, move |batch: RecordBatch| {
        let ids = batch
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .ok_or_else(|| {
                arrow_schema::ArrowError::SchemaError("'id' column is not Utf8".to_string())
            })?;
        Ok(BooleanArray::from_iter((0..ids.len()).map(|i| {
            Some(!ids.is_null(i) && ids.value(i) == owned_id)
        })))
    });
    RowFilter::new(vec![Box::new(predicate_fn)])
}

/// Decode the first object of a (row-filtered, restamped) batch — `None`
/// for an empty batch.
pub(crate) fn first_decoded_object(
    batch: &RecordBatch,
    meta: &CityMetadata,
) -> Result<Option<DecodedObject>> {
    if batch.num_rows() == 0 {
        return Ok(None);
    }
    Ok(decode_batch(batch, meta)?.into_iter().next())
}

/// Non-null cells in a single-column projected batch.
pub(crate) fn non_null_count(batch: &RecordBatch) -> u64 {
    let array = batch.column(0);
    (array.len() - array.null_count()) as u64
}

/// The `attr_stats` aggregation state: the statistics min/max fast path is
/// attempted at construction (abandoned whole if ANY row group lacks a
/// usable min/max — never a mixed answer), then every batch of the
/// single-column scan is folded in, and `finish` picks fast-path or scanned
/// min/max. Identical semantics to the two former inline copies.
pub(crate) struct AttrStatsAccumulator {
    stats_available: bool,
    stats_min: f64,
    stats_max: f64,
    sum: f64,
    count: u64,
    scan_min: f64,
    scan_max: f64,
}

impl AttrStatsAccumulator {
    pub(crate) fn new(metadata: &ParquetMetaData, column: &str) -> Self {
        let mut stats_available = true;
        let mut stats_min = f64::INFINITY;
        let mut stats_max = f64::NEG_INFINITY;
        for i in 0..metadata.num_row_groups() {
            match column_statistics(metadata.row_group(i), column).and_then(statistics_min_max) {
                Some((min, max)) => {
                    stats_min = stats_min.min(min);
                    stats_max = stats_max.max(max);
                }
                None => {
                    stats_available = false;
                    break;
                }
            }
        }
        Self {
            stats_available,
            stats_min,
            stats_max,
            sum: 0.0,
            count: 0,
            scan_min: f64::INFINITY,
            scan_max: f64::NEG_INFINITY,
        }
    }

    fn visit(&mut self, v: f64) {
        self.sum += v;
        self.count += 1;
        if !self.stats_available {
            self.scan_min = self.scan_min.min(v);
            self.scan_max = self.scan_max.max(v);
        }
    }

    pub(crate) fn visit_batch(&mut self, column: &str, batch: &RecordBatch) -> Result<()> {
        let array = batch.column(0);
        match array.data_type() {
            DataType::Int64 => {
                let values = array.as_any().downcast_ref::<Int64Array>().ok_or_else(|| {
                    CityParquetError::Schema(format!("column '{column}' is not Int64"))
                })?;
                for i in 0..values.len() {
                    if !values.is_null(i) {
                        self.visit(values.value(i) as f64);
                    }
                }
            }
            DataType::Float64 => {
                let values = array
                    .as_any()
                    .downcast_ref::<Float64Array>()
                    .ok_or_else(|| {
                        CityParquetError::Schema(format!("column '{column}' is not Float64"))
                    })?;
                for i in 0..values.len() {
                    if !values.is_null(i) {
                        self.visit(values.value(i));
                    }
                }
            }
            other => {
                return Err(CityParquetError::Schema(format!(
                    "column '{column}' has an arrow type attr_stats cannot aggregate: {other:?}"
                )));
            }
        }
        Ok(())
    }

    pub(crate) fn finish(self) -> AttrStats {
        let (min, max) = if self.stats_available {
            (self.stats_min, self.stats_max)
        } else {
            (self.scan_min, self.scan_max)
        };
        AttrStats {
            min,
            max,
            sum: self.sum,
            count: self.count,
        }
    }
}
