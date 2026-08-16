//! Async mirrors of `crate::query`'s primitives, over an `object_store`
//! `ObjectStore` instead of a bare filesystem path — the transport-agnostic
//! reference query code local and HTTP callers both share.
//!
//! Reuses `crate::reader::CityParquetReaderBuilder`/`row_group_intersects`/
//! `box_intersects_query` UNCHANGED: `parquet::arrow::async_reader::store::
//! ParquetRecordBatchStreamBuilder<T>` is a type alias for
//! `ArrowReaderBuilder<AsyncReader<T>>`, and every setter/accessor those use
//! is defined generically as `impl<T> ArrowReaderBuilder<T>` in `parquet`
//! itself — there is no sync/async split to bridge.
//!
//! This module is the ASYNC transport only: opening a
//! [`ParquetObjectReader`], awaiting a [`ParquetRecordBatchStreamBuilder`],
//! and pulling batches off a stream (plus the per-batch `restamp` the sync
//! reader wrapper does for itself). Every batch-level decision — predicate
//! evaluation, projection/row-filter assembly, row-group pruning counts,
//! aggregation — is shared verbatim with the sync path via
//! `crate::query_core`, so the two can no longer drift.

use std::sync::Arc;

use arrow_array::RecordBatch;
use arrow_schema::SchemaRef;
use futures::TryStreamExt;
use object_store::ObjectStore;
use object_store::path::Path as ObjectPath;
use parquet::arrow::ParquetRecordBatchStreamBuilder;
use parquet::arrow::ProjectionMask;
use parquet::arrow::async_reader::ParquetObjectReader;

use cityparquet_schema::{CityMetadata, CityParquetError, Result};

use crate::query::{AttrPredicate, AttrStats, BBoxQueryResult, FullReadResult};
use crate::query_core;
use crate::reader::CityParquetReaderBuilder;

/// Used only by this module's `mod tests` (via its `use super::*`), which
/// reads a real id straight off the local table with the sync reader.
#[cfg(test)]
use arrow_array::StringArray;

fn parquet_err(e: impl std::fmt::Display) -> CityParquetError {
    CityParquetError::Parquet(e.to_string())
}

/// Re-stamps `batch` with `schema` (field metadata included) — the async
/// analogue of [`crate::reader::CityParquetRecordBatchReader`]'s per-batch
/// rewrap, inlined here rather than as its own stream-wrapper type since
/// only [`full_read_async`]/`id_lookup_async` need it.
fn restamp(batch: RecordBatch, schema: &SchemaRef) -> Result<RecordBatch> {
    RecordBatch::try_new(SchemaRef::clone(schema), batch.columns().to_vec())
        .map_err(CityParquetError::from)
}

/// The table's row count straight from Parquet file metadata — O(1) in row
/// count (a suffix range request for the footer, plus a second request only
/// if the footer exceeds the reader's initial prefetch size), no row scan.
/// The async mirror of [`crate::query::count`].
pub async fn count_async(store: Arc<dyn ObjectStore>, path: &ObjectPath) -> Result<u64> {
    let reader = ParquetObjectReader::new(store, path.clone());
    let builder = ParquetRecordBatchStreamBuilder::new(reader)
        .await
        .map_err(parquet_err)?;
    Ok(builder.metadata().file_metadata().num_rows() as u64)
}

/// The async mirror of [`crate::query::full_read`]: scans every row group
/// (via the stream, sequential — no `.with_row_groups` restriction),
/// decoding every row's geometry. `meta` is the caller's already-resolved
/// [`CityMetadata`] (callers typically get it once via
/// [`CityParquetReaderBuilder::cityparquet_metadata`] on their own builder
/// before consuming it, mirroring the sync path's own two-open pattern in
/// `formats::cityparquet::open_metadata`).
pub async fn full_read_async(
    store: Arc<dyn ObjectStore>,
    path: &ObjectPath,
    meta: &CityMetadata,
) -> Result<FullReadResult> {
    let reader = ParquetObjectReader::new(store, path.clone());
    let builder = ParquetRecordBatchStreamBuilder::new(reader)
        .await
        .map_err(parquet_err)?;
    let schema = builder.cityparquet_arrow_schema()?;
    let mut stream = builder.build().map_err(parquet_err)?;

    let mut acc = FullReadResult::default();
    while let Some(batch) = stream.try_next().await.map_err(parquet_err)? {
        let batch = restamp(batch, &schema)?;
        query_core::accumulate_full_read(&mut acc, &batch, meta)?;
    }
    Ok(acc)
}

/// The async mirror of [`crate::query::bbox_query`]. Row-group pruning
/// counts come from the SAME `row_group_intersects` predicate the sync
/// path uses (over the identical `parquet::file::metadata::RowGroupMetaData`
/// type — footer metadata has no sync/async distinction), then the
/// surviving rows are read via an `id`/`bbox`-only [`ProjectionMask`] and
/// filtered exactly via the shared `box_intersects_query` row test.
pub async fn bbox_query_async(
    store: Arc<dyn ObjectStore>,
    path: &ObjectPath,
    query_bbox: [f64; 6],
) -> Result<BBoxQueryResult> {
    let reader = ParquetObjectReader::new(Arc::clone(&store), path.clone());
    let builder = ParquetRecordBatchStreamBuilder::new(reader)
        .await
        .map_err(parquet_err)?;

    let (row_groups_total, row_groups_touched) =
        query_core::bbox_row_group_counts(builder.metadata(), &query_bbox);

    let projection = ProjectionMask::columns(builder.parquet_schema(), ["id", "bbox"]);
    let pruned = builder
        .with_projection(projection)
        .with_bbox_row_groups(query_bbox)?;
    let mut stream = pruned.build().map_err(parquet_err)?;

    // No restamp: the `id`/`bbox` projection carries no extension metadata
    // the row test depends on.
    let mut ids = Vec::new();
    while let Some(batch) = stream.try_next().await.map_err(parquet_err)? {
        query_core::collect_bbox_ids(&batch, &query_bbox, &mut ids)?;
    }

    Ok(BBoxQueryResult {
        ids,
        row_groups_total,
        row_groups_touched,
    })
}

/// The async mirror of [`crate::query::attr_filter`]: restricts the scan to
/// `column` alone and applies `pred` as a Parquet row filter — the SAME
/// `query_core::attr_predicate_row_filter` (hence the same
/// `evaluate_attr_predicate` dispatch) the sync path installs, so a
/// predicate that is legal/illegal against a given column's Arrow type
/// behaves identically on both transports.
pub async fn attr_filter_async(
    store: Arc<dyn ObjectStore>,
    path: &ObjectPath,
    column: &str,
    pred: &AttrPredicate,
) -> Result<u64> {
    let reader = ParquetObjectReader::new(store, path.clone());
    let builder = ParquetRecordBatchStreamBuilder::new(reader)
        .await
        .map_err(parquet_err)?;

    query_core::require_column(builder.schema(), column)?;

    let output_mask = ProjectionMask::columns(builder.parquet_schema(), [column]);
    let row_filter = query_core::attr_predicate_row_filter(builder.parquet_schema(), column, pred);

    let mut stream = builder
        .with_projection(output_mask)
        .with_row_filter(row_filter)
        .build()
        .map_err(parquet_err)?;

    let mut count = 0u64;
    while let Some(batch) = stream.try_next().await.map_err(parquet_err)? {
        count += batch.num_rows() as u64;
    }
    Ok(count)
}

/// The async mirror of [`crate::query::attr_stats`]: the same
/// statistics-fast-path-then-scan structure, over the shared
/// `query_core::AttrStatsAccumulator` (Parquet row-group metadata is
/// identical between sync and async readers — no transport-specific
/// re-derivation needed).
pub async fn attr_stats_async(
    store: Arc<dyn ObjectStore>,
    path: &ObjectPath,
    column: &str,
) -> Result<AttrStats> {
    let reader = ParquetObjectReader::new(store, path.clone());
    let builder = ParquetRecordBatchStreamBuilder::new(reader)
        .await
        .map_err(parquet_err)?;

    query_core::require_column(builder.schema(), column)?;

    let mut acc = query_core::AttrStatsAccumulator::new(builder.metadata(), column);

    let projection = ProjectionMask::columns(builder.parquet_schema(), [column]);
    let mut stream = builder
        .with_projection(projection)
        .build()
        .map_err(parquet_err)?;

    while let Some(batch) = stream.try_next().await.map_err(parquet_err)? {
        acc.visit_batch(column, &batch)?;
    }
    Ok(acc.finish())
}

/// The async mirror of [`crate::query::id_lookup`]: filters to `id` via the
/// shared `query_core::id_row_filter`, then fully decodes the (expected
/// exactly one) surviving row.
pub async fn id_lookup_async(
    store: Arc<dyn ObjectStore>,
    path: &ObjectPath,
    meta: &CityMetadata,
    id: &str,
) -> Result<Option<crate::decode::DecodedObject>> {
    let reader = ParquetObjectReader::new(store, path.clone());
    let builder = ParquetRecordBatchStreamBuilder::new(reader)
        .await
        .map_err(parquet_err)?;
    let schema = builder.cityparquet_arrow_schema()?;

    let row_filter = query_core::id_row_filter(builder.parquet_schema(), id);
    let mut stream = builder
        .with_row_filter(row_filter)
        .build()
        .map_err(parquet_err)?;

    while let Some(batch) = stream.try_next().await.map_err(parquet_err)? {
        if batch.num_rows() == 0 {
            continue;
        }
        let batch = restamp(batch, &schema)?;
        if let Some(object) = query_core::first_decoded_object(&batch, meta)? {
            return Ok(Some(object));
        }
    }
    Ok(None)
}

/// The async mirror of [`crate::query::project_column`]: a single-column
/// projected scan across every row, counting non-null values.
pub async fn project_column_async(
    store: Arc<dyn ObjectStore>,
    path: &ObjectPath,
    column: &str,
) -> Result<u64> {
    let reader = ParquetObjectReader::new(store, path.clone());
    let builder = ParquetRecordBatchStreamBuilder::new(reader)
        .await
        .map_err(parquet_err)?;

    query_core::require_column(builder.schema(), column)?;

    let projection = ProjectionMask::columns(builder.parquet_schema(), [column]);
    let mut stream = builder
        .with_projection(projection)
        .build()
        .map_err(parquet_err)?;

    let mut count = 0u64;
    while let Some(batch) = stream.try_next().await.map_err(parquet_err)? {
        count += query_core::non_null_count(&batch);
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use object_store::local::LocalFileSystem;
    use std::path::Path;

    fn fixture_dir() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
    }

    /// Converts `delft.city.jsonl` into a fresh package under `dir` and
    /// returns `(store, relative table path)` ready for the async reader —
    /// `LocalFileSystem::new_with_prefix(dir)` roots every `ObjectPath` at
    /// `dir`, so paths passed to the async functions are always relative.
    pub(super) async fn delft_table(dir: &Path) -> (Arc<dyn ObjectStore>, ObjectPath) {
        let fixture = fixture_dir().join("delft.city.jsonl");
        assert!(fixture.exists(), "missing fixture; run `just fixtures`");
        let opts = crate::package::ConvertOptions::new(fixture, dir.to_path_buf());
        crate::package::convert(&opts).unwrap();
        let tables = crate::stac::properties::PackageTables::open(dir).unwrap();
        let table_name = tables.tables[0].file_name().unwrap().to_str().unwrap();
        let store = LocalFileSystem::new_with_prefix(dir).unwrap();
        (Arc::new(store), ObjectPath::from(table_name))
    }

    #[tokio::test]
    async fn count_async_matches_sync_count_on_a_real_fixture() {
        let dir = tempfile::tempdir().unwrap();
        let (store, path) = delft_table(dir.path()).await;
        let table_file = dir.path().join(path.as_ref());

        let sync_count = crate::query::count(&table_file).unwrap();
        let async_count = count_async(store, &path).await.unwrap();
        assert_eq!(async_count, sync_count);
        assert_eq!(async_count, 2231);
    }

    #[tokio::test]
    async fn full_read_async_matches_sync_full_read_on_a_real_fixture() {
        let dir = tempfile::tempdir().unwrap();
        let (store, path) = delft_table(dir.path()).await;
        let table_file = dir.path().join(path.as_ref());

        let meta = {
            let file = std::fs::File::open(&table_file).unwrap();
            let builder =
                parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder::try_new(file)
                    .unwrap();
            crate::reader::CityParquetReaderBuilder::cityparquet_metadata(&builder).unwrap()
        };
        let sync_result = crate::query::full_read(&table_file, &meta).unwrap();
        let async_result = full_read_async(store, &path, &meta).await.unwrap();
        assert_eq!(async_result.feature_count, sync_result.feature_count);
        assert_eq!(async_result.boundary_count, sync_result.boundary_count);
    }

    #[tokio::test]
    async fn bbox_query_async_matches_sync_bbox_query_on_a_real_fixture() {
        let dir = tempfile::tempdir().unwrap();
        let (store, path) = delft_table(dir.path()).await;
        let table_file = dir.path().join(path.as_ref());

        // A generous window covering the whole delft fixture's extent plus
        // margin, so the exact same rows match on both paths regardless of
        // the fixture's real coordinates.
        let bbox = [-1e9, -1e9, -1e9, 1e9, 1e9, 1e9];
        let sync_result = crate::query::bbox_query(&table_file, bbox).unwrap();
        let async_result = bbox_query_async(store, &path, bbox).await.unwrap();
        assert_eq!(async_result.row_groups_total, sync_result.row_groups_total);
        assert_eq!(
            async_result.row_groups_touched,
            sync_result.row_groups_touched
        );
        let mut sync_ids = sync_result.ids.clone();
        let mut async_ids = async_result.ids.clone();
        sync_ids.sort();
        async_ids.sort();
        assert_eq!(async_ids, sync_ids);
    }

    #[tokio::test]
    async fn attr_filter_async_matches_sync_attr_filter_on_a_real_fixture() {
        use crate::query::AttrPredicate;

        let dir = tempfile::tempdir().unwrap();
        let (store, path) = delft_table(dir.path()).await;
        let table_file = dir.path().join(path.as_ref());

        let pred = AttrPredicate::Eq(serde_json::Value::String("BuildingPart".to_string()));
        let sync_count = crate::query::attr_filter(&table_file, "object_type", &pred).unwrap();
        let async_count = attr_filter_async(store, &path, "object_type", &pred)
            .await
            .unwrap();
        assert_eq!(async_count, sync_count);
        assert_eq!(async_count, 1116);
    }

    #[tokio::test]
    async fn attr_stats_async_matches_sync_attr_stats_on_a_real_fixture() {
        let dir = tempfile::tempdir().unwrap();
        let (store, path) = delft_table(dir.path()).await;
        let table_file = dir.path().join(path.as_ref());

        // delft's own numeric attribute (year built), confirmed against
        // `crates/cityparquet/tests/query_real_data.rs`'s own
        // `attr_filter_numeric_predicates_match_year_built_attribute_column`.
        let column = "oorspronkelijkbouwjaar";
        let sync_stats = crate::query::attr_stats(&table_file, column).unwrap();
        let async_stats = attr_stats_async(store, &path, column).await.unwrap();
        assert_eq!(async_stats.count, sync_stats.count);
        assert_eq!(async_stats.min, sync_stats.min);
        assert_eq!(async_stats.max, sync_stats.max);
        assert!((async_stats.sum - sync_stats.sum).abs() < 1e-6);
    }

    #[tokio::test]
    async fn id_lookup_async_matches_sync_id_lookup_on_a_real_fixture() {
        let dir = tempfile::tempdir().unwrap();
        let (store, path) = delft_table(dir.path()).await;
        let table_file = dir.path().join(path.as_ref());
        let meta = {
            let file = std::fs::File::open(&table_file).unwrap();
            let builder =
                parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder::try_new(file)
                    .unwrap();
            crate::reader::CityParquetReaderBuilder::cityparquet_metadata(&builder).unwrap()
        };

        // A real id from the fixture: read the first row's id straight off
        // the local table, rather than hardcoding one.
        let first_id = {
            let file = std::fs::File::open(&table_file).unwrap();
            let builder =
                parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder::try_new(file)
                    .unwrap();
            let mut reader = builder.build().unwrap();
            let batch = reader.next().unwrap().unwrap();
            let ids = batch
                .column_by_name("id")
                .unwrap()
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap();
            ids.value(0).to_string()
        };

        let sync_result = crate::query::id_lookup(&table_file, &meta, &first_id).unwrap();
        let async_result = id_lookup_async(store, &path, &meta, &first_id)
            .await
            .unwrap();
        assert!(sync_result.is_some());
        assert!(async_result.is_some());
        assert_eq!(sync_result.unwrap().id, async_result.unwrap().id);
    }

    #[tokio::test]
    async fn project_column_async_matches_sync_project_column_on_a_real_fixture() {
        let dir = tempfile::tempdir().unwrap();
        let (store, path) = delft_table(dir.path()).await;
        let table_file = dir.path().join(path.as_ref());

        let sync_count = crate::query::project_column(&table_file, "object_type").unwrap();
        let async_count = project_column_async(store, &path, "object_type")
            .await
            .unwrap();
        assert_eq!(async_count, sync_count);
        assert_eq!(async_count, 2231);
    }
}
