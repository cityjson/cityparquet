//! Batch-level core shared by [`crate::query`] (sync, `File`-backed) and
//! `crate::query_async` (async, `object_store`-backed): predicate
//! construction, projection/row-filter assembly, row-group pruning counts,
//! and per-batch aggregation. The two entry-point modules differ ONLY in
//! how a reader is opened and how batches are pulled (iterator vs stream,
//! plus the async path's per-batch restamp); everything else lives here
//! exactly once (review P3 — the former self-acknowledged duplication at
//! the top of `query_async.rs`).

use arrow_array::{
    Array, BooleanArray, Float64Array, Int64Array, RecordBatch, StringArray, StructArray,
    new_empty_array,
};
use arrow_schema::{DataType, Schema};
use parquet::arrow::ProjectionMask;
use parquet::arrow::arrow_reader::{ArrowPredicateFn, RowFilter};
use parquet::basic::{ConvertedType, LogicalType, Type as PhysicalType};
use parquet::bloom_filter::Sbbf;
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

/// One candidate row group's bloom filter for the target column, located
/// from footer metadata alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BloomTarget {
    pub row_group: usize,
    pub offset: u64,
    /// `bloom_filter_length`, when the writer declared it.
    pub length: Option<u64>,
}

/// Where the target column's filters are, per candidate row group.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BloomTargets {
    /// The column's Parquet leaf index, resolved by path from the file
    /// schema (never an Arrow field ordinal); `None` when the file has no
    /// such leaf.
    pub leaf: Option<usize>,
    pub with_filter: Vec<BloomTarget>,
    /// Candidates whose chunk carries no filter. They are always kept.
    pub without_filter: Vec<usize>,
}

/// The outcome of probing a column's bloom filters for a set of values
/// (IN semantics): which candidate row groups can still hold one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BloomPrune {
    /// Row groups to read, ascending — for `with_row_groups`.
    pub keep: Vec<usize>,
    /// Candidates considered.
    pub total: usize,
    /// Candidates whose filter rejected every value.
    pub pruned: usize,
    /// Candidates kept unexamined: no filter on their chunk, or no probe made.
    pub without_filter: usize,
    /// Bitset bytes of every filter examined (32 per block), the same on the
    /// sync and async paths. Header bytes and transport overhead are not
    /// counted; `crate::counting_store::CountingObjectStore` reports the latter.
    pub filter_bytes: u64,
}

impl BloomPrune {
    /// Assembles the outcome from `targets` and one verdict per
    /// `targets.with_filter` entry (`true`: keep).
    pub(crate) fn from_verdicts(
        targets: &BloomTargets,
        candidates: &[usize],
        verdicts: &[bool],
        filter_bytes: u64,
    ) -> Self {
        let mut keep = targets.without_filter.clone();
        keep.extend(
            targets
                .with_filter
                .iter()
                .zip(verdicts)
                .filter(|(_, keep)| **keep)
                .map(|(target, _)| target.row_group),
        );
        keep.sort_unstable();
        Self {
            keep,
            total: candidates.len(),
            pruned: verdicts.iter().filter(|keep| !**keep).count(),
            without_filter: targets.without_filter.len(),
            filter_bytes,
        }
    }

    /// Every candidate kept, none examined — a predicate no filter answers.
    pub(crate) fn unpruned(candidates: &[usize]) -> Self {
        Self {
            keep: candidates.to_vec(),
            total: candidates.len(),
            pruned: 0,
            without_filter: candidates.len(),
            filter_bytes: 0,
        }
    }
}

/// What the bloom filters did for one lookup or filter.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LookupStats {
    pub row_groups_total: usize,
    pub bloom_pruned: usize,
    /// [`BloomPrune::filter_bytes`].
    pub filter_bytes: u64,
}

impl LookupStats {
    pub(crate) fn from_prune(prune: &BloomPrune) -> Self {
        Self {
            row_groups_total: prune.total,
            bloom_pruned: prune.pruned,
            filter_bytes: prune.filter_bytes,
        }
    }
}

impl std::ops::AddAssign for LookupStats {
    fn add_assign(&mut self, other: Self) {
        self.row_groups_total += other.row_groups_total;
        self.bloom_pruned += other.bloom_pruned;
        self.filter_bytes += other.filter_bytes;
    }
}

/// Resolves `column`'s leaf by path and, for each candidate row group, where
/// its filter is — from footer metadata alone, no I/O. Shared verbatim by
/// the sync and async pruners.
pub fn bloom_targets(
    meta: &ParquetMetaData,
    column: &ColumnPath,
    candidates: &[usize],
) -> BloomTargets {
    let leaf = meta
        .file_metadata()
        .schema_descr()
        .columns()
        .iter()
        .position(|descr| descr.path() == column);
    let Some(leaf) = leaf else {
        return BloomTargets {
            leaf: None,
            with_filter: Vec::new(),
            without_filter: candidates.to_vec(),
        };
    };
    let mut targets = BloomTargets {
        leaf: Some(leaf),
        ..BloomTargets::default()
    };
    for &row_group in candidates {
        let chunk = meta.row_group(row_group).column(leaf);
        match chunk
            .bloom_filter_offset()
            .and_then(|offset| u64::try_from(offset).ok())
        {
            Some(offset) => targets.with_filter.push(BloomTarget {
                row_group,
                offset,
                length: chunk
                    .bloom_filter_length()
                    .and_then(|length| u64::try_from(length).ok()),
            }),
            None => targets.without_filter.push(row_group),
        }
    }
    targets
}

/// A filter keeps its row group when ANY value may be present (IN
/// semantics). Values are probed as their raw UTF-8 bytes, which is what
/// the writer hashed.
pub(crate) fn sbbf_matches_any(sbbf: &Sbbf, values: &[&str]) -> bool {
    values.iter().any(|value| sbbf.check(*value))
}

/// The [`BloomPrune::filter_bytes`] one filter contributes.
pub(crate) fn filter_bitset_bytes(sbbf: &Sbbf) -> u64 {
    32 * sbbf.num_blocks() as u64
}

/// A top-level column's single-part leaf path.
pub(crate) fn top_level_path(name: &str) -> ColumnPath {
    ColumnPath::new(vec![name.to_string()])
}

/// The mask selecting top-level `column`, matched by its exact name.
/// `ProjectionMask::columns` splits a name on `.`, so an attribute whose name
/// holds a literal `.` would select no leaf at all.
pub(crate) fn root_mask(parquet_schema: &SchemaDescriptor, column: &str) -> Result<ProjectionMask> {
    let root = parquet_schema
        .root_schema()
        .get_fields()
        .iter()
        .position(|field| field.name() == column)
        .ok_or_else(|| {
            CityParquetError::Schema(format!("column '{column}' missing from the file's schema"))
        })?;
    Ok(ProjectionMask::roots(parquet_schema, [root]))
}

/// The value `pred` probes `column`'s bloom filter with, or `None` when no
/// probe applies. The predicate's type rules come first — checked on an
/// empty array of the column's own type, so the error is the scan's own and
/// never depends on whether the file carries filters. A probe is made only
/// for string equality on a string column (`Utf8` or `Dictionary<Int32,
/// Utf8>`) whose Parquet leaf is `BYTE_ARRAY` with a UTF-8 annotation: the
/// numeric predicates compare through `f64` and never consult a filter.
pub(crate) fn string_probe<'a>(
    arrow_schema: &Schema,
    parquet_schema: &SchemaDescriptor,
    column: &str,
    pred: &'a AttrPredicate,
) -> Result<Option<&'a str>> {
    let field = arrow_schema.field_with_name(column).map_err(|_| {
        CityParquetError::Schema(format!("column '{column}' missing from the file's schema"))
    })?;
    evaluate_attr_predicate(column, new_empty_array(field.data_type()).as_ref(), pred)?;
    let AttrPredicate::Eq(serde_json::Value::String(value)) = pred else {
        return Ok(None);
    };
    let string_column = match field.data_type() {
        DataType::Utf8 => true,
        DataType::Dictionary(key, value) => {
            key.as_ref() == &DataType::Int32 && value.as_ref() == &DataType::Utf8
        }
        _ => false,
    };
    if !string_column {
        return Ok(None);
    }
    let root = parquet_schema
        .root_schema()
        .get_fields()
        .iter()
        .position(|f| f.name() == column);
    let leaves: Vec<usize> = match root {
        Some(root) => (0..parquet_schema.num_columns())
            .filter(|&leaf| parquet_schema.get_column_root_idx(leaf) == root)
            .collect(),
        None => Vec::new(),
    };
    let [leaf] = leaves[..] else {
        return Ok(None);
    };
    let descr = parquet_schema.column(leaf);
    let utf8 = descr.physical_type() == PhysicalType::BYTE_ARRAY
        && (matches!(descr.logical_type_ref(), Some(LogicalType::String))
            || descr.converted_type() == ConvertedType::UTF8);
    Ok(utf8.then_some(value.as_str()))
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
        // Tolerant of either physical representation a writer may have
        // chosen for a string column (spec: a reader must accept plain
        // `Utf8` or `Dictionary<Int32, Utf8>`) — routed through
        // [`crate::arrow_compat::string_view`] so this site cannot drift
        // from `decode`/`package`'s identical tolerance.
        DataType::Utf8 | DataType::Dictionary(_, _) => {
            let AttrPredicate::Eq(serde_json::Value::String(want)) = pred else {
                return Err(schema_err(format!(
                    "column '{column}' is a string column; only `Eq(<string>)` applies, got {pred:?}"
                )));
            };
            let view = crate::arrow_compat::string_view(array, column)?;
            Ok(BooleanArray::from_iter((0..array.len()).map(|i| {
                Some(!view.is_null(i) && view.value(i) == want.as_str())
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
/// install: `pred` evaluated by [`evaluate_attr_predicate`] over `column`
/// alone, selected by exact name ([`root_mask`]).
pub(crate) fn attr_predicate_row_filter(
    parquet_schema: &SchemaDescriptor,
    column: &str,
    pred: &AttrPredicate,
) -> Result<RowFilter> {
    let predicate_mask = root_mask(parquet_schema, column)?;
    let owned_column = column.to_string();
    let owned_pred = pred.clone();
    let predicate_fn = ArrowPredicateFn::new(predicate_mask, move |batch: RecordBatch| {
        let array = batch.column(0);
        evaluate_attr_predicate(&owned_column, array.as_ref(), &owned_pred)
            .map_err(arrow_schema::ArrowError::from)
    });
    Ok(RowFilter::new(vec![Box::new(predicate_fn)]))
}

/// The `column == value` [`RowFilter`] both transports install for the
/// identifier lookups (`id`, `feature_id`): the predicate's own projection is
/// `column` alone; the output projection stays untouched (the full row is
/// decoded on a hit). Accepts `Utf8` and `Dictionary<Int32, Utf8>` — a
/// writer's physical choice a reader must not depend on. A positive bloom
/// result is never a match: this filter decides every row.
pub(crate) fn utf8_eq_row_filter(
    parquet_schema: &SchemaDescriptor,
    column: &str,
    value: &str,
) -> Result<RowFilter> {
    let predicate_mask = root_mask(parquet_schema, column)?;
    let owned_column = column.to_string();
    let owned_value = value.to_string();
    let predicate_fn = ArrowPredicateFn::new(predicate_mask, move |batch: RecordBatch| {
        let values = crate::arrow_compat::string_view(batch.column(0).as_ref(), &owned_column)
            .map_err(arrow_schema::ArrowError::from)?;
        Ok(BooleanArray::from_iter((0..batch.num_rows()).map(|i| {
            Some(!values.is_null(i) && values.value(i) == owned_value)
        })))
    });
    Ok(RowFilter::new(vec![Box::new(predicate_fn)]))
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
