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
//!
//! **Bloom filters.** [`bloom_keep_row_groups`] probes a column's Parquet
//! bloom filters for a set of values and keeps only the row groups that can
//! hold one; a chunk without a filter is always kept, and a positive result
//! is never a match — the exact `RowFilter` still decides every row. The
//! lookups and string-equality [`attr_filter`] read the kept row groups with
//! one reader (`with_row_groups`), so an `id` hit still stops at its first
//! match. Every single-column mask is resolved by exact column name.

use std::fs::File;
use std::path::Path;

use cityparquet_schema::{CityMetadata, CityParquetError, Result};
use parquet::arrow::ProjectionMask;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::bloom_filter::Sbbf;
use parquet::file::metadata::ParquetMetaData;
use parquet::file::reader::ChunkReader;
use parquet::schema::types::ColumnPath;

use crate::decode::DecodedObject;
use crate::query_core;
use crate::reader::{CityParquetReaderBuilder, CityParquetRecordBatchReader};

pub use crate::query_core::{
    AttrPredicate, AttrStats, BBoxQueryResult, BloomPrune, BloomTarget, BloomTargets,
    FullReadResult, LookupStats, bloom_targets,
};

/// Opens `table_path`, scans every row group single-threaded (the
/// `parquet` crate's synchronous [`ParquetRecordBatchReaderBuilder`] path
/// never spreads batch iteration across a thread pool, unlike its async
/// counterpart), and decodes each row's WKB geometry, accumulating
/// [`FullReadResult::feature_count`] (total rows) and
/// [`FullReadResult::boundary_count`] (total decoded surfaces/faces).
/// Forces full geometry materialisation.
pub fn full_read(table_path: &Path, meta: &CityMetadata) -> Result<FullReadResult> {
    let file = File::open(table_path)?;
    let builder =
        ParquetRecordBatchReaderBuilder::try_new(file).map_err(CityParquetError::parquet_from)?;
    let schema = builder.cityparquet_arrow_schema()?;
    let parquet_reader = builder.build().map_err(CityParquetError::parquet_from)?;
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
    let file = File::open(table_path)?;
    let builder =
        ParquetRecordBatchReaderBuilder::try_new(file).map_err(CityParquetError::parquet_from)?;
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
    let file = File::open(table_path)?;
    let builder =
        ParquetRecordBatchReaderBuilder::try_new(file).map_err(CityParquetError::parquet_from)?;

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
    let reader = pruned.build().map_err(CityParquetError::parquet_from)?;

    let mut ids = Vec::new();
    for batch in reader {
        query_core::collect_bbox_ids(
            &batch.map_err(CityParquetError::parquet_from)?,
            &query_bbox,
            &mut ids,
        )?;
    }
    Ok(BBoxQueryResult {
        ids,
        row_groups_total,
        row_groups_touched,
    })
}

/// The number of rows of `column` that satisfy `pred`. See
/// [`attr_filter_with_stats`].
pub fn attr_filter(table_path: &Path, column: &str, pred: &AttrPredicate) -> Result<u64> {
    attr_filter_with_stats(table_path, column, pred).map(|(count, _)| count)
}

/// Opens `table_path`, restricts the scan to `column` alone (selected by its
/// exact name) and applies `pred` as a Parquet
/// [`RowFilter`](parquet::arrow::arrow_reader::RowFilter) (`ArrowPredicateFn`)
/// so only `column` is ever decoded. Column statistics prune nothing here.
/// The predicate's type rules are checked on the column's own type first, so
/// a mismatch fails the same way with or without filters. For a string
/// equality on a UTF-8 string column, the column's bloom filters — where the
/// file carries them — then drop every row group that cannot hold the value
/// ([`bloom_keep_row_groups`]); a positive bloom result is never a match, so
/// the `RowFilter` still decides every row. Returns the COUNT of surviving
/// rows (the `RowFilter` drops every non-matching row before it reaches the
/// batches) and what the filters did.
pub fn attr_filter_with_stats(
    table_path: &Path,
    column: &str,
    pred: &AttrPredicate,
) -> Result<(u64, LookupStats)> {
    let file = File::open(table_path)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file.try_clone()?)
        .map_err(CityParquetError::parquet_from)?;

    let probe = query_core::string_probe(builder.schema(), builder.parquet_schema(), column, pred)?;
    let output_mask = query_core::root_mask(builder.parquet_schema(), column)?;
    let row_filter = query_core::attr_predicate_row_filter(builder.parquet_schema(), column, pred)?;
    let candidates: Vec<usize> = (0..builder.metadata().num_row_groups()).collect();
    let prune = match probe {
        Some(value) => bloom_keep_row_groups(
            &file,
            builder.metadata(),
            &query_core::top_level_path(column),
            &[value],
            &candidates,
        )?,
        None => BloomPrune::unpruned(&candidates),
    };
    let stats = LookupStats::from_prune(&prune);

    let reader = builder
        .with_projection(output_mask)
        .with_row_filter(row_filter)
        .with_row_groups(prune.keep)
        .build()
        .map_err(CityParquetError::parquet_from)?;
    let mut count = 0u64;
    for batch in reader {
        count += batch.map_err(CityParquetError::parquet_from)?.num_rows() as u64;
    }
    Ok((count, stats))
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
    let file = File::open(table_path)?;
    let builder =
        ParquetRecordBatchReaderBuilder::try_new(file).map_err(CityParquetError::parquet_from)?;

    // Fail fast with a clear "column not found" error before `builder` is
    // consumed by `with_projection` below.
    query_core::require_column(builder.schema(), column)?;

    // Stats fast-path attempt (abandoned whole the moment any row group fails
    // to supply a min/max), then the single-column scan — always needed for
    // sum/count, and, only on the fast-path's failure, for min/max too
    // (folded into the same pass rather than a second scan).
    let mut acc = query_core::AttrStatsAccumulator::new(builder.metadata(), column);

    let projection = query_core::root_mask(builder.parquet_schema(), column)?;
    let reader = builder
        .with_projection(projection)
        .build()
        .map_err(CityParquetError::parquet_from)?;
    for batch in reader {
        acc.visit_batch(column, &batch.map_err(CityParquetError::parquet_from)?)?;
    }
    Ok(acc.finish())
}

/// Probes `column`'s bloom filter in each of `candidates` (row-group
/// indices) for `values`, with one `pread` per filter on `reader` — a local
/// file, not the builder's own reader, which the arrow builders keep
/// private. Keeps a row group when its chunk has no filter or when any value
/// may be present.
pub fn bloom_keep_row_groups<R: ChunkReader>(
    reader: &R,
    meta: &ParquetMetaData,
    column: &ColumnPath,
    values: &[&str],
    candidates: &[usize],
) -> Result<BloomPrune> {
    let targets = query_core::bloom_targets(meta, column, candidates);
    let mut verdicts = Vec::with_capacity(targets.with_filter.len());
    let mut filter_bytes = 0u64;
    if let Some(leaf) = targets.leaf {
        for target in &targets.with_filter {
            let chunk = meta.row_group(target.row_group).column(leaf);
            match Sbbf::read_from_column_chunk(chunk, reader)
                .map_err(CityParquetError::parquet_from)?
            {
                Some(sbbf) => {
                    filter_bytes += query_core::filter_bitset_bytes(&sbbf);
                    verdicts.push(query_core::sbbf_matches_any(&sbbf, values));
                }
                None => verdicts.push(true),
            }
        }
    }
    Ok(BloomPrune::from_verdicts(
        &targets,
        candidates,
        &verdicts,
        filter_bytes,
    ))
}

/// Finds and fully materialises the one object whose `id` column equals
/// `id`; `None` if no row matches. See [`id_lookup_with_stats`].
pub fn id_lookup(
    table_path: &Path,
    meta: &CityMetadata,
    id: &str,
) -> Result<Option<DecodedObject>> {
    id_lookup_with_stats(table_path, meta, id).map(|(object, _)| object)
}

/// [`id_lookup`] with what the filters did. The `id` bloom filters prune the
/// row groups first ([`bloom_keep_row_groups`]); ONE reader then scans the
/// kept row groups with `id` as an exact `Eq`
/// [`RowFilter`](parquet::arrow::arrow_reader::RowFilter) projected to `id`
/// alone, and the first matching row is decoded in full via
/// [`crate::decode::decode_batch`] — the read stops there.
pub fn id_lookup_with_stats(
    table_path: &Path,
    meta: &CityMetadata,
    id: &str,
) -> Result<(Option<DecodedObject>, LookupStats)> {
    let file = File::open(table_path)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file.try_clone()?)
        .map_err(CityParquetError::parquet_from)?;
    let schema = builder.cityparquet_arrow_schema()?;
    let candidates: Vec<usize> = (0..builder.metadata().num_row_groups()).collect();
    let prune = bloom_keep_row_groups(
        &file,
        builder.metadata(),
        &query_core::top_level_path("id"),
        &[id],
        &candidates,
    )?;
    let stats = LookupStats::from_prune(&prune);

    let row_filter = query_core::utf8_eq_row_filter(builder.parquet_schema(), "id", id)?;
    let parquet_reader = builder
        .with_row_groups(prune.keep)
        .with_row_filter(row_filter)
        .build()
        .map_err(CityParquetError::parquet_from)?;
    let reader = CityParquetRecordBatchReader::new(parquet_reader, schema);
    for batch in reader {
        if let Some(object) = query_core::first_decoded_object(&batch?, meta)? {
            return Ok((Some(object), stats));
        }
    }
    Ok((None, stats))
}

/// Projected single-column read of `column` across every row, via a
/// [`ProjectionMask`] restricting the scan to that column alone (nothing
/// else in the table is ever decoded — the columnar-projection primitive).
/// Returns the count of NON-NULL values in `column`.
pub fn project_column(table_path: &Path, column: &str) -> Result<u64> {
    let file = File::open(table_path)?;
    let builder =
        ParquetRecordBatchReaderBuilder::try_new(file).map_err(CityParquetError::parquet_from)?;

    // Fail fast with a clear "column not found" error before `builder` is
    // consumed by `with_projection` below.
    query_core::require_column(builder.schema(), column)?;

    let projection = query_core::root_mask(builder.parquet_schema(), column)?;
    let reader = builder
        .with_projection(projection)
        .build()
        .map_err(CityParquetError::parquet_from)?;

    let mut count = 0u64;
    for batch in reader {
        count += query_core::non_null_count(&batch.map_err(CityParquetError::parquet_from)?);
    }
    Ok(count)
}
