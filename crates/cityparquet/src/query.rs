//! Read/query primitives over a CityParquet package: the first primitives
//! of the cross-format read-benchmark milestone (later tasks add
//! attribute/id queries on top of these).
//!
//! [`count`] is O(1) — it reads the row count straight out of the Parquet
//! file metadata, no row scan. [`full_read`] is the opposite extreme: a
//! single-threaded scan of every row group that decodes every row's WKB
//! geometry (via [`crate::decode`]/[`crate::wkb_read`]), forcing full
//! materialisation — the metric later cross-format comparisons key off.
//! [`bbox_query`] sits in between: it prunes row groups via
//! [`crate::reader::CityParquetReaderBuilder::with_bbox_row_groups`] (a
//! superset — never wrong, but may over-select) and then applies a row-level
//! 3D bbox-intersection test on every surviving row, so its result is exact.

use std::fs::File;
use std::path::Path;
use std::sync::Arc;

use arrow_array::types::Int32Type;
use arrow_array::{
    Array, ArrayAccessor, BooleanArray, DictionaryArray, Float64Array, Int64Array, RecordBatch,
    StringArray, StructArray,
};
use arrow_schema::DataType;
use cityparquet_schema::{CityParquetError, CityParquetMetadata, Result};
use parquet::arrow::ProjectionMask;

use crate::decode::decode_batch;
use crate::reader::{
    CityParquetReaderBuilder, CityParquetRecordBatchReader, box_intersects_query,
    row_group_intersects,
};
use crate::wkb_read::DecodedKind;
use parquet::arrow::arrow_reader::{ArrowPredicateFn, ParquetRecordBatchReaderBuilder, RowFilter};

fn io_err(e: impl std::fmt::Display) -> CityParquetError {
    CityParquetError::Io(e.to_string())
}

fn parquet_err(e: impl std::fmt::Display) -> CityParquetError {
    CityParquetError::Parquet(e.to_string())
}

/// The result of a [`full_read`]: the total feature (row) count and a
/// stable geometry-work metric (`boundary_count`).
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

/// Opens `table_path`, scans every row group single-threaded (the
/// `parquet` crate's synchronous [`ParquetRecordBatchReaderBuilder`] path
/// never spreads batch iteration across a thread pool, unlike its async
/// counterpart), and decodes each row's WKB geometry, accumulating
/// [`FullReadResult::feature_count`] (total rows) and
/// [`FullReadResult::boundary_count`] (total decoded surfaces/faces).
/// Forces full geometry materialisation.
pub fn full_read(table_path: &Path, meta: &CityParquetMetadata) -> Result<FullReadResult> {
    let file = File::open(table_path).map_err(io_err)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).map_err(parquet_err)?;
    let schema = builder.cityparquet_arrow_schema()?;
    let parquet_reader = builder.build().map_err(parquet_err)?;
    let reader = CityParquetRecordBatchReader::new(parquet_reader, schema);

    let mut feature_count = 0u64;
    let mut boundary_count = 0u64;
    for batch in reader {
        let batch = batch?;
        feature_count += batch.num_rows() as u64;
        let decoded = decode_batch(&batch, meta)?;
        for object in &decoded {
            for (_, geometry, _) in &object.geometries {
                boundary_count += surface_count(&geometry.kind);
            }
        }
    }
    Ok(FullReadResult {
        feature_count,
        boundary_count,
    })
}

/// The table's row count straight from Parquet file metadata — O(1), no
/// row scan.
pub fn count(table_path: &Path) -> Result<u64> {
    let file = File::open(table_path).map_err(io_err)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).map_err(parquet_err)?;
    Ok(builder.metadata().file_metadata().num_rows() as u64)
}

/// The result of an exact [`bbox_query`]: the matching object `id`s, plus
/// how many of the table's row groups were pruned away vs. actually
/// touched.
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

/// Opens `table_path`, prunes row groups via
/// [`CityParquetReaderBuilder::with_bbox_row_groups`] (a superset — it never
/// wrongly drops a row group, but may keep groups with no true match), then
/// reads only the `id`/`bbox` columns of every surviving row and applies a
/// row-level 3D bbox-intersection test, so the returned `ids` are exact.
/// `row_groups_total`/`row_groups_touched` report the same pruning counts
/// the `cityparquet bench` CLI harness measures, via the identical shared
/// [`row_group_intersects`] predicate.
pub fn bbox_query(table_path: &Path, query_bbox: [f64; 6]) -> Result<BBoxQueryResult> {
    let file = File::open(table_path).map_err(io_err)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).map_err(parquet_err)?;

    // Row-group pruning counts, computed BEFORE `with_bbox_row_groups`
    // consumes `builder` — the exact same predicate `with_bbox_row_groups`
    // itself uses, so these counts can never drift from what the scan below
    // actually reads.
    let metadata = Arc::clone(builder.metadata());
    let row_groups_total = metadata.num_row_groups();
    let row_groups_touched = (0..row_groups_total)
        .filter(|&i| row_group_intersects(metadata.row_group(i), &query_bbox))
        .count();

    // Project down to just `id` and `bbox` — the row-level filter below
    // needs nothing else, and every other column (geometry, attributes) can
    // be arbitrarily large.
    let projection = ProjectionMask::columns(builder.parquet_schema(), ["id", "bbox"]);
    let pruned = builder
        .with_projection(projection)
        .with_bbox_row_groups(query_bbox)?;
    let reader = pruned.build().map_err(parquet_err)?;

    let mut ids = Vec::new();
    for batch in reader {
        let batch = batch.map_err(parquet_err)?;
        let id_col = batch
            .column_by_name("id")
            .and_then(|c| c.as_any().downcast_ref::<StringArray>())
            .ok_or_else(|| {
                CityParquetError::Schema("'id' column missing or not Utf8".to_string())
            })?;
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
            if box_intersects_query(row_min, row_max, &query_bbox) {
                ids.push(id_col.value(row).to_string());
            }
        }
    }

    Ok(BBoxQueryResult {
        ids,
        row_groups_total,
        row_groups_touched,
    })
}

/// An attribute-column predicate for [`attr_filter`]. `Eq` compares against a
/// string (for a `Utf8`/`Dictionary<_, Utf8>` column) or a number (for an
/// `Int64`/`Float64` column) as appropriate to the target column's actual
/// Arrow type; `Ge`/`Le`/`Range` always compare numerically as `f64` and only
/// apply to `Int64`/`Float64` columns. A row whose value is null never
/// matches any variant.
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
fn evaluate_attr_predicate(
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

/// Opens `table_path`, restricts the scan to `column` alone via a
/// [`ProjectionMask`], and applies `pred` as a Parquet [`RowFilter`]
/// (`ArrowPredicateFn`) so only `column` is ever decoded — nothing else in
/// the table is read — and the reader's row-group statistics can prune
/// entire row groups the predicate could not possibly match. Returns the
/// COUNT of surviving rows (never the rows themselves): the `RowFilter`
/// already drops every non-matching row before it reaches the returned
/// batches, so counting rows across the batches yielded IS the matching
/// count.
pub fn attr_filter(table_path: &Path, column: &str, pred: &AttrPredicate) -> Result<u64> {
    let file = File::open(table_path).map_err(io_err)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).map_err(parquet_err)?;

    // `data_type` is fetched now (before `builder` is consumed by
    // `with_projection`/`with_row_filter` below) purely to fail fast with a
    // clear "column not found" error; `evaluate_attr_predicate` re-derives
    // the type itself from the projected batch's own schema, so this lookup
    // is not load-bearing for correctness, only for a better error message.
    builder.schema().field_with_name(column).map_err(|_| {
        CityParquetError::Schema(format!("column '{column}' missing from the file's schema"))
    })?;

    // Same single-column mask twice: once as the predicate's own required
    // projection, once as the builder's overall output projection — `column`
    // is the only thing either the predicate or the final count needs.
    let predicate_mask = ProjectionMask::columns(builder.parquet_schema(), [column]);
    let output_mask = ProjectionMask::columns(builder.parquet_schema(), [column]);

    let owned_column = column.to_string();
    let owned_pred = pred.clone();
    let predicate_fn = ArrowPredicateFn::new(predicate_mask, move |batch: RecordBatch| {
        let array = batch.column(0);
        evaluate_attr_predicate(&owned_column, array.as_ref(), &owned_pred)
            .map_err(arrow_schema::ArrowError::from)
    });
    let row_filter = RowFilter::new(vec![Box::new(predicate_fn)]);

    let reader = builder
        .with_projection(output_mask)
        .with_row_filter(row_filter)
        .build()
        .map_err(parquet_err)?;

    let mut count = 0u64;
    for batch in reader {
        let batch = batch.map_err(parquet_err)?;
        count += batch.num_rows() as u64;
    }
    Ok(count)
}
