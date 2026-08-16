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
//!
//! This module is the SYNC transport only: opening a [`File`], building a
//! [`ParquetRecordBatchReaderBuilder`], and pulling batches from an iterator.
//! Everything batch-level — predicate evaluation, projection/row-filter
//! assembly, row-group pruning counts, aggregation — lives once in
//! `crate::query_core` and is shared verbatim with the async mirrors in
//! `crate::query_async`.

use std::fs::File;
use std::path::Path;

use cityparquet_schema::{CityMetadata, CityParquetError, Result};
use parquet::arrow::ProjectionMask;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

use crate::query_core;
use crate::reader::{CityParquetReaderBuilder, CityParquetRecordBatchReader};

pub use crate::query_core::{AttrPredicate, AttrStats, BBoxQueryResult, FullReadResult};

fn io_err(e: impl std::fmt::Display) -> CityParquetError {
    CityParquetError::Io(e.to_string())
}

fn parquet_err(e: impl std::fmt::Display) -> CityParquetError {
    CityParquetError::Parquet(e.to_string())
}

/// Opens `table_path`, scans every row group single-threaded (the
/// `parquet` crate's synchronous [`ParquetRecordBatchReaderBuilder`] path
/// never spreads batch iteration across a thread pool, unlike its async
/// counterpart), and decodes each row's WKB geometry, accumulating
/// [`FullReadResult::feature_count`] (total rows) and
/// [`FullReadResult::boundary_count`] (total decoded surfaces/faces).
/// Forces full geometry materialisation.
pub fn full_read(table_path: &Path, meta: &CityMetadata) -> Result<FullReadResult> {
    let file = File::open(table_path).map_err(io_err)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).map_err(parquet_err)?;
    let schema = builder.cityparquet_arrow_schema()?;
    let parquet_reader = builder.build().map_err(parquet_err)?;
    let reader = CityParquetRecordBatchReader::new(parquet_reader, schema);

    let mut acc = FullReadResult::default();
    for batch in reader {
        query_core::accumulate_full_read(&mut acc, &batch?, meta)?;
    }
    Ok(acc)
}

/// The table's row count straight from Parquet file metadata — O(1), no
/// row scan.
pub fn count(table_path: &Path) -> Result<u64> {
    let file = File::open(table_path).map_err(io_err)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).map_err(parquet_err)?;
    Ok(builder.metadata().file_metadata().num_rows() as u64)
}

/// Opens `table_path`, prunes row groups via
/// [`CityParquetReaderBuilder::with_bbox_row_groups`] (a superset — it never
/// wrongly drops a row group, but may keep groups with no true match), then
/// reads only the `id`/`bbox` columns of every surviving row and applies a
/// row-level 3D bbox-intersection test, so the returned `ids` are exact.
/// `row_groups_total`/`row_groups_touched` report the same pruning counts
/// the `cityparquet bench` CLI harness measures, via the identical shared
/// [`crate::reader::row_group_intersects`] predicate.
pub fn bbox_query(table_path: &Path, query_bbox: [f64; 6]) -> Result<BBoxQueryResult> {
    let file = File::open(table_path).map_err(io_err)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).map_err(parquet_err)?;

    // Row-group pruning counts, computed BEFORE `with_bbox_row_groups`
    // consumes `builder`.
    let (row_groups_total, row_groups_touched) =
        query_core::bbox_row_group_counts(builder.metadata(), &query_bbox);

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
        query_core::collect_bbox_ids(&batch.map_err(parquet_err)?, &query_bbox, &mut ids)?;
    }
    Ok(BBoxQueryResult {
        ids,
        row_groups_total,
        row_groups_touched,
    })
}

/// Opens `table_path`, restricts the scan to `column` alone via a
/// [`ProjectionMask`], and applies `pred` as a Parquet
/// [`RowFilter`](parquet::arrow::arrow_reader::RowFilter)
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

    // Checked now (before `builder` is consumed by `with_projection`/
    // `with_row_filter` below) purely to fail fast with a clear "column not
    // found" error; `evaluate_attr_predicate` re-derives the type itself from
    // the projected batch's own schema, so this lookup is not load-bearing
    // for correctness, only for a better error message.
    query_core::require_column(builder.schema(), column)?;

    // Same single-column mask twice: once as the predicate's own required
    // projection (inside the row filter), once as the builder's overall
    // output projection — `column` is the only thing either the predicate or
    // the final count needs.
    let output_mask = ProjectionMask::columns(builder.parquet_schema(), [column]);
    let row_filter = query_core::attr_predicate_row_filter(builder.parquet_schema(), column, pred);

    let reader = builder
        .with_projection(output_mask)
        .with_row_filter(row_filter)
        .build()
        .map_err(parquet_err)?;

    let mut count = 0u64;
    for batch in reader {
        count += batch.map_err(parquet_err)?.num_rows() as u64;
    }
    Ok(count)
}

/// Opens `table_path` and computes [`AttrStats`] for the numeric (`Int64` or
/// `Float64`) attribute column named `column`:
///
/// - **`min`/`max`**: taken from each touched row group's Parquet
///   column-chunk `Statistics` (near-free — no row scan) when *every* row
///   group carries a usable min/max for the column. If even one row group
///   lacks statistics (or has none defined, e.g. every value in that chunk
///   is null), the stats fast-path is abandoned entirely and min/max are
///   instead derived honestly from the same single-column scan `sum`/`count`
///   already require — never a silently wrong statistics-only answer mixed
///   with a scan-only answer.
/// - **`sum`/`count`**: always from a single-column [`ProjectionMask`] scan
///   (Parquet has no chunk-level sum statistic to short-circuit this).
///   `count` is the number of non-null cells; `sum` is over those same
///   non-null cells; nulls never contribute to either.
pub fn attr_stats(table_path: &Path, column: &str) -> Result<AttrStats> {
    let file = File::open(table_path).map_err(io_err)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).map_err(parquet_err)?;

    // Fail fast with a clear "column not found" error before `builder` is
    // consumed by `with_projection` below.
    query_core::require_column(builder.schema(), column)?;

    // Stats fast-path attempt (abandoned whole the moment any row group fails
    // to supply a min/max), then the single-column scan — always needed for
    // sum/count, and, only on the fast-path's failure, for min/max too
    // (folded into the same pass rather than a second scan).
    let mut acc = query_core::AttrStatsAccumulator::new(builder.metadata(), column);

    let projection = ProjectionMask::columns(builder.parquet_schema(), [column]);
    let reader = builder
        .with_projection(projection)
        .build()
        .map_err(parquet_err)?;
    for batch in reader {
        acc.visit_batch(column, &batch.map_err(parquet_err)?)?;
    }
    Ok(acc.finish())
}

/// Finds and fully materialises the one object whose `id` column equals
/// `id`. Parquet carries no id index, so this applies `id` as an `Eq`
/// [`RowFilter`](parquet::arrow::arrow_reader::RowFilter) (via
/// `ArrowPredicateFn`, projected to just the `id` column for the predicate's
/// own evaluation) and then, on every surviving row — expected to be exactly
/// one, since ids are unique — decodes the FULL row (every column, not just
/// `id`) via [`crate::decode::decode_batch`], returning the first decoded
/// object. `None` if no row matches.
pub fn id_lookup(
    table_path: &Path,
    meta: &CityMetadata,
    id: &str,
) -> Result<Option<crate::decode::DecodedObject>> {
    let file = File::open(table_path).map_err(io_err)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).map_err(parquet_err)?;
    let schema = builder.cityparquet_arrow_schema()?;

    // The predicate's own required projection is `id` alone; the builder's
    // overall output projection is left untouched (every column) since
    // `decode_batch` needs the full row to materialise the object.
    let row_filter = query_core::id_row_filter(builder.parquet_schema(), id);
    let parquet_reader = builder
        .with_row_filter(row_filter)
        .build()
        .map_err(parquet_err)?;
    let reader = CityParquetRecordBatchReader::new(parquet_reader, schema);

    for batch in reader {
        if let Some(object) = query_core::first_decoded_object(&batch?, meta)? {
            return Ok(Some(object));
        }
    }
    Ok(None)
}

/// Projected single-column read of `column` across every row, via a
/// [`ProjectionMask`] restricting the scan to that column alone (nothing
/// else in the table is ever decoded — the columnar-projection primitive).
/// Returns the count of NON-NULL values in `column`.
pub fn project_column(table_path: &Path, column: &str) -> Result<u64> {
    let file = File::open(table_path).map_err(io_err)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).map_err(parquet_err)?;

    // Fail fast with a clear "column not found" error before `builder` is
    // consumed by `with_projection` below.
    query_core::require_column(builder.schema(), column)?;

    let projection = ProjectionMask::columns(builder.parquet_schema(), [column]);
    let reader = builder
        .with_projection(projection)
        .build()
        .map_err(parquet_err)?;

    let mut count = 0u64;
    for batch in reader {
        count += query_core::non_null_count(&batch.map_err(parquet_err)?);
    }
    Ok(count)
}
