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
//! identifier lookups ([`id_lookup_with_stats`], [`feature_lookup_with_stats`],
//! [`package_feature_lookup_with_stats`]) and string-equality [`attr_filter`]
//! read the kept row groups with one reader per table (`with_row_groups`), so
//! an `id` hit still stops at its first match. Every single-column mask is
//! resolved by exact column name.

use std::fs::File;
use std::path::Path;

use cityparquet_schema::{CityMetadata, CityParquetError, Result};
use parquet::arrow::ProjectionMask;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::bloom_filter::Sbbf;
use parquet::file::metadata::ParquetMetaData;
use parquet::file::reader::ChunkReader;
use parquet::schema::types::ColumnPath;

use crate::decode::{DecodedObject, decode_batch};
use crate::query_core;
use crate::reader::{CityParquetReaderBuilder, CityParquetRecordBatchReader};

pub use crate::query_core::{
    AttrPredicate, AttrStats, BBoxGeometryResult, BBoxQueryResult, BBoxVisitResult, BloomPrune,
    BloomTarget, BloomTargets, FullReadResult, LookupStats, bloom_targets,
};
pub use crate::visit::VisitTotals;

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

/// Scans every row group of `table_path` single-threaded and visits every
/// object natively ([`crate::visit::visit_batch`]): every WKB vertex, every
/// semantic reference and semantic-surface object, and every attribute value,
/// read in place from Arrow and WKB rather than decoded into CityJSON-shaped
/// objects. Appearance is not read.
pub fn full_read_visit(table_path: &Path) -> Result<VisitTotals> {
    let file = File::open(table_path)?;
    let reader = ParquetRecordBatchReaderBuilder::try_new(file)
        .map_err(CityParquetError::parquet_from)?
        .build()
        .map_err(CityParquetError::parquet_from)?;
    let mut totals = VisitTotals::default();
    for batch in reader {
        crate::visit::visit_batch(&batch.map_err(CityParquetError::parquet_from)?, &mut totals)?;
    }
    Ok(totals)
}

/// Visits every row of a row-filtered reader natively; see
/// `query_core::visit_into`.
fn visit_all(
    reader: impl Iterator<
        Item = std::result::Result<arrow_array::RecordBatch, arrow_schema::ArrowError>,
    >,
    first_only: bool,
) -> Result<VisitTotals> {
    let mut totals = VisitTotals::default();
    for batch in reader {
        let batch = batch.map_err(CityParquetError::parquet_from)?;
        if query_core::visit_into(&mut totals, &batch, first_only)? {
            break;
        }
    }
    Ok(totals)
}

/// [`bbox_query`] that VISITS every matching object natively rather than
/// returning ids. Row groups are pruned as in [`bbox_query`]; within the
/// survivors a `RowFilter` reads the `bbox` struct alone, so rows outside
/// the window never have their geometry or attribute columns decoded.
pub fn bbox_query_visit(table_path: &Path, query_bbox: [f64; 6]) -> Result<BBoxVisitResult> {
    let file = File::open(table_path)?;
    let builder =
        ParquetRecordBatchReaderBuilder::try_new(file).map_err(CityParquetError::parquet_from)?;
    let (row_groups_total, row_groups_touched) =
        query_core::bbox_row_group_counts(builder.metadata(), &query_bbox);
    let row_filter = query_core::bbox_row_filter(builder.parquet_schema(), query_bbox);
    let reader = builder
        .with_bbox_row_groups(query_bbox)?
        .with_row_filter(row_filter)
        .build()
        .map_err(CityParquetError::parquet_from)?;
    Ok(BBoxVisitResult {
        totals: visit_all(reader, false)?,
        row_groups_total,
        row_groups_touched,
    })
}

/// The ids of the objects whose `bbox` intersects `query_bbox` (edges
/// count), each with its geometry at its HIGHEST LoD walked in place.
///
/// Row groups are pruned as in [`bbox_query`]. The read projects `id`,
/// `bbox` and the geometry columns; a `RowFilter` evaluates the `bbox`
/// struct alone first, so rows outside the window never have a geometry
/// decoded. For each kept row the most detailed non-null geometry column —
/// LoDs ordered numerically from the column names, `lod2_2 > lod1_3 >
/// lod0_0`, the un-suffixed primary `geometry` last — is walked with
/// [`crate::wkb_read::visit_wkb`]: every stored coordinate read as an
/// `f64`, nothing converted into another representation. An object without
/// geometry is returned with its id only.
pub fn bbox_query_geometry(table_path: &Path, query_bbox: [f64; 6]) -> Result<BBoxGeometryResult> {
    let file = File::open(table_path)?;
    let builder =
        ParquetRecordBatchReaderBuilder::try_new(file).map_err(CityParquetError::parquet_from)?;
    let (row_groups_total, row_groups_touched) =
        query_core::bbox_row_group_counts(builder.metadata(), &query_bbox);
    let mask = query_core::bbox_geometry_mask(builder.schema(), builder.parquet_schema())?;
    let row_filter = query_core::bbox_row_filter(builder.parquet_schema(), query_bbox);
    let reader = builder
        .with_projection(mask)
        .with_bbox_row_groups(query_bbox)?
        .with_row_filter(row_filter)
        .build()
        .map_err(CityParquetError::parquet_from)?;
    let mut acc = BBoxGeometryResult::empty(row_groups_total, row_groups_touched);
    for batch in reader {
        query_core::fold_bbox_geometry(&batch.map_err(CityParquetError::parquet_from)?, &mut acc)?;
    }
    Ok(acc)
}

/// [`attr_filter_with_stats`] returning the `id` of every matching row, in
/// row order, instead of their count. Row groups are pruned exactly as
/// there (bloom filters for a string equality, then min/max statistics); the
/// predicate `RowFilter` reads `column` alone and only `id` is projected for
/// the surviving rows.
pub fn attr_filter_ids(
    table_path: &Path,
    column: &str,
    pred: &AttrPredicate,
) -> Result<(Vec<String>, LookupStats)> {
    let file = File::open(table_path)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file.try_clone()?)
        .map_err(CityParquetError::parquet_from)?;
    let probe = query_core::string_probe(builder.schema(), builder.parquet_schema(), column, pred)?;
    let output_mask = query_core::root_mask(builder.parquet_schema(), "id")?;
    let row_filter = query_core::attr_predicate_row_filter(builder.parquet_schema(), column, pred)?;
    let (row_groups, stats) = prune_row_groups(&file, builder.metadata(), column, probe, pred)?;
    let reader = builder
        .with_projection(output_mask)
        .with_row_filter(row_filter)
        .with_row_groups(row_groups)
        .build()
        .map_err(CityParquetError::parquet_from)?;
    let mut ids = Vec::new();
    for batch in reader {
        query_core::collect_ids(&batch.map_err(CityParquetError::parquet_from)?, &mut ids)?;
    }
    Ok((ids, stats))
}

/// The row groups to read for `pred` on `column`: the bloom filters drop
/// every row group that cannot hold `probe` (where the file carries them and
/// a probe exists), then the min/max statistics drop the survivors that
/// cannot satisfy `pred`.
fn prune_row_groups(
    file: &File,
    metadata: &parquet::file::metadata::ParquetMetaData,
    column: &str,
    probe: Option<&str>,
    pred: &AttrPredicate,
) -> Result<(Vec<usize>, LookupStats)> {
    let candidates: Vec<usize> = (0..metadata.num_row_groups()).collect();
    let prune = match probe {
        Some(value) => bloom_keep_row_groups(
            file,
            metadata,
            &query_core::top_level_path(column),
            &[value],
            &candidates,
        )?,
        None => BloomPrune::unpruned(&candidates),
    };
    Ok(LookupStats::from_prune_and_statistics(
        prune, metadata, column, pred,
    ))
}

/// [`attr_filter_with_stats`] that VISITS every matching object natively;
/// `totals.objects` is the count. Row groups are pruned exactly as in
/// [`attr_filter_with_stats`].
pub fn attr_filter_visit(
    table_path: &Path,
    column: &str,
    pred: &AttrPredicate,
) -> Result<(VisitTotals, LookupStats)> {
    let file = File::open(table_path)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file.try_clone()?)
        .map_err(CityParquetError::parquet_from)?;
    let probe = query_core::string_probe(builder.schema(), builder.parquet_schema(), column, pred)?;
    let row_filter = query_core::attr_predicate_row_filter(builder.parquet_schema(), column, pred)?;
    let (row_groups, stats) = prune_row_groups(&file, builder.metadata(), column, probe, pred)?;
    let reader = builder
        .with_row_filter(row_filter)
        .with_row_groups(row_groups)
        .build()
        .map_err(CityParquetError::parquet_from)?;
    Ok((visit_all(reader, false)?, stats))
}

/// [`id_lookup_with_stats`] that VISITS the matching object natively instead
/// of decoding it; `totals.objects` is 0 or 1. The `id` bloom filters, then
/// the `id` statistics, prune the row groups; the read stops at the first
/// batch holding a match.
pub fn id_lookup_visit(table_path: &Path, id: &str) -> Result<(VisitTotals, LookupStats)> {
    let file = File::open(table_path)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file.try_clone()?)
        .map_err(CityParquetError::parquet_from)?;
    let (row_groups, stats) = prune_row_groups(
        &file,
        builder.metadata(),
        "id",
        Some(id),
        &query_core::eq_str(id),
    )?;
    let row_filter = query_core::utf8_eq_row_filter(builder.parquet_schema(), "id", id)?;
    let reader = builder
        .with_row_groups(row_groups)
        .with_row_filter(row_filter)
        .build()
        .map_err(CityParquetError::parquet_from)?;
    Ok((visit_all(reader, true)?, stats))
}

/// [`feature_lookup_with_stats`] that VISITS every matching object natively;
/// `totals.objects` is the feature's row count. The `feature_id` bloom
/// filters, then its statistics, prune the row groups; every survivor is
/// read to the end.
pub fn feature_lookup_visit(
    table_path: &Path,
    feature_id: &str,
) -> Result<(VisitTotals, LookupStats)> {
    let file = File::open(table_path)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file.try_clone()?)
        .map_err(CityParquetError::parquet_from)?;
    let (row_groups, stats) = prune_row_groups(
        &file,
        builder.metadata(),
        "feature_id",
        Some(feature_id),
        &query_core::eq_str(feature_id),
    )?;
    let row_filter =
        query_core::utf8_eq_row_filter(builder.parquet_schema(), "feature_id", feature_id)?;
    let reader = builder
        .with_row_groups(row_groups)
        .with_row_filter(row_filter)
        .build()
        .map_err(CityParquetError::parquet_from)?;
    Ok((visit_all(reader, false)?, stats))
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
/// so only `column` is ever decoded. Row groups whose `column` statistics
/// rule out `pred` are skipped ([`LookupStats::stats_pruned`]; rules on
/// `query_core::attr_row_groups`, page-index pruning is out of scope).
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
    let (row_groups, stats) = prune_row_groups(&file, builder.metadata(), column, probe, pred)?;

    let reader = builder
        .with_projection(output_mask)
        .with_row_filter(row_filter)
        .with_row_groups(row_groups)
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
    let (row_groups, stats) = prune_row_groups(
        &file,
        builder.metadata(),
        "id",
        Some(id),
        &query_core::eq_str(id),
    )?;

    let row_filter = query_core::utf8_eq_row_filter(builder.parquet_schema(), "id", id)?;
    let parquet_reader = builder
        .with_row_groups(row_groups)
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

/// Every object whose `feature_id` equals `feature_id` — a feature and all
/// its parts — in table row order; empty if none. See
/// [`feature_lookup_with_stats`].
pub fn feature_lookup(
    table_path: &Path,
    meta: &CityMetadata,
    feature_id: &str,
) -> Result<Vec<DecodedObject>> {
    feature_lookup_with_stats(table_path, meta, feature_id).map(|(objects, _)| objects)
}

/// [`feature_lookup`] with what it cost. The `feature_id` bloom filters
/// prune the row groups first; the survivors are then read to the end — a
/// feature's rows may span row groups, so there is no early stop — with an
/// exact `feature_id` equality `RowFilter`, and every matching row is
/// decoded in full. One reader reads every kept row group.
pub fn feature_lookup_with_stats(
    table_path: &Path,
    meta: &CityMetadata,
    feature_id: &str,
) -> Result<(Vec<DecodedObject>, LookupStats)> {
    let file = File::open(table_path)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file.try_clone()?)
        .map_err(CityParquetError::parquet_from)?;
    let schema = builder.cityparquet_arrow_schema()?;
    let (row_groups, stats) = prune_row_groups(
        &file,
        builder.metadata(),
        "feature_id",
        Some(feature_id),
        &query_core::eq_str(feature_id),
    )?;

    let row_filter =
        query_core::utf8_eq_row_filter(builder.parquet_schema(), "feature_id", feature_id)?;
    let parquet_reader = builder
        .with_row_groups(row_groups)
        .with_row_filter(row_filter)
        .build()
        .map_err(CityParquetError::parquet_from)?;
    let reader = CityParquetRecordBatchReader::new(parquet_reader, schema);

    let mut objects = Vec::new();
    for batch in reader {
        objects.extend(decode_batch(&batch?, meta)?);
    }
    Ok((objects, stats))
}

/// [`feature_lookup`] over a whole package: every object table the
/// package's `metadata.json` names, in manifest order. See
/// [`package_feature_lookup_with_stats`].
pub fn package_feature_lookup(package_dir: &Path, feature_id: &str) -> Result<Vec<DecodedObject>> {
    package_feature_lookup_with_stats(package_dir, feature_id).map(|(objects, _)| objects)
}

/// [`feature_lookup_with_stats`] per object table, the objects concatenated
/// in manifest order and the statistics summed. Each table's own footer
/// supplies the metadata its rows decode with.
pub fn package_feature_lookup_with_stats(
    package_dir: &Path,
    feature_id: &str,
) -> Result<(Vec<DecodedObject>, LookupStats)> {
    let tables = crate::stac::properties::PackageTables::open(package_dir)?;
    let mut objects = Vec::new();
    let mut stats = LookupStats::default();
    for table in &tables.tables {
        let meta = ParquetRecordBatchReaderBuilder::try_new(File::open(table)?)
            .map_err(CityParquetError::parquet_from)?
            .cityparquet_metadata()?;
        let (found, table_stats) = feature_lookup_with_stats(table, &meta, feature_id)?;
        objects.extend(found);
        stats += table_stats;
    }
    Ok((objects, stats))
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
