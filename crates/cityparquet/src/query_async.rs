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

use std::sync::Arc;

use arrow_array::RecordBatch;
use arrow_schema::SchemaRef;
use futures::TryStreamExt;
use object_store::ObjectStore;
use object_store::path::Path as ObjectPath;
use parquet::arrow::ParquetRecordBatchStreamBuilder;
use parquet::arrow::async_reader::ParquetObjectReader;

use cityparquet_schema::{CityMetadata, CityParquetError, Result};

use crate::decode::decode_batch;
use crate::query::FullReadResult;
use crate::reader::CityParquetReaderBuilder;
use crate::wkb_read::DecodedKind;

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

/// Total surface/face count in `kind` — duplicated from `crate::query`'s
/// private `surface_count` (a two-line, stable recursive match, cheaper to
/// copy once than to widen `query.rs`'s visibility for a single shared leaf
/// helper).
fn surface_count(kind: &DecodedKind) -> u64 {
    match kind {
        DecodedKind::MultiPoint(_) | DecodedKind::MultiLineString(_) => 0,
        DecodedKind::MultiPolygon(surfaces) | DecodedKind::PolyhedralSurface(surfaces) => {
            surfaces.len() as u64
        }
        DecodedKind::GeometryCollection(members) => members.iter().map(surface_count).sum(),
    }
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

    let mut feature_count = 0u64;
    let mut boundary_count = 0u64;
    while let Some(batch) = stream.try_next().await.map_err(parquet_err)? {
        feature_count += batch.num_rows() as u64;
        let batch = restamp(batch, &schema)?;
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

use arrow_array::{Array, StringArray, StructArray};
use parquet::arrow::ProjectionMask;

use crate::query::BBoxQueryResult;
use crate::reader::{box_intersects_query, row_group_intersects};

/// Same row leaf-reader as `crate::query`'s private `row_bbox` (kept local:
/// a four-line struct-field read, cheaper to duplicate once than to widen
/// `query.rs`'s visibility for it alone).
fn row_bbox(bbox_col: &StructArray, row: usize) -> Result<Option<([f64; 3], [f64; 3])>> {
    if bbox_col.is_null(row) {
        return Ok(None);
    }
    let leaf = |name: &str| -> Result<f64> {
        Ok(bbox_col
            .column_by_name(name)
            .and_then(|c| c.as_any().downcast_ref::<arrow_array::Float64Array>())
            .ok_or_else(|| {
                CityParquetError::Schema(format!("bbox.{name} column missing or not Float64"))
            })?
            .value(row))
    };
    let min = [leaf("xmin")?, leaf("ymin")?, leaf("zmin")?];
    let max = [leaf("xmax")?, leaf("ymax")?, leaf("zmax")?];
    Ok(Some((min, max)))
}

/// The async mirror of [`crate::query::bbox_query`]. Row-group pruning
/// counts come from the SAME [`row_group_intersects`] predicate the sync
/// path uses (over the identical `parquet::file::metadata::RowGroupMetaData`
/// type — footer metadata has no sync/async distinction), then the
/// surviving rows are read via an `id`/`bbox`-only [`ProjectionMask`] and
/// filtered exactly via [`box_intersects_query`].
pub async fn bbox_query_async(
    store: Arc<dyn ObjectStore>,
    path: &ObjectPath,
    query_bbox: [f64; 6],
) -> Result<BBoxQueryResult> {
    let reader = ParquetObjectReader::new(Arc::clone(&store), path.clone());
    let builder = ParquetRecordBatchStreamBuilder::new(reader)
        .await
        .map_err(parquet_err)?;

    let metadata = Arc::clone(builder.metadata());
    let row_groups_total = metadata.num_row_groups();
    let row_groups_touched = (0..row_groups_total)
        .filter(|&i| row_group_intersects(metadata.row_group(i), &query_bbox))
        .count();

    let projection = ProjectionMask::columns(builder.parquet_schema(), ["id", "bbox"]);
    let pruned = builder
        .with_projection(projection)
        .with_bbox_row_groups(query_bbox)?;
    let mut stream = pruned.build().map_err(parquet_err)?;

    let mut ids = Vec::new();
    while let Some(batch) = stream.try_next().await.map_err(parquet_err)? {
        let id_col = batch
            .column_by_name("id")
            .and_then(|c| c.as_any().downcast_ref::<StringArray>())
            .ok_or_else(|| CityParquetError::Schema("'id' column missing or not Utf8".into()))?;
        let bbox_col = batch
            .column_by_name("bbox")
            .and_then(|c| c.as_any().downcast_ref::<StructArray>())
            .ok_or_else(|| {
                CityParquetError::Schema("'bbox' column missing or not a struct".into())
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

use parquet::arrow::arrow_reader::{ArrowPredicateFn, RowFilter};

use crate::query::{AttrPredicate, evaluate_attr_predicate};

/// The async mirror of [`crate::query::attr_filter`]: restricts the scan to
/// `column` alone and applies `pred` as a Parquet [`RowFilter`] — the SAME
/// `evaluate_attr_predicate` dispatch the sync path uses, so a predicate
/// that is legal/illegal against a given column's Arrow type behaves
/// identically on both transports.
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

    builder.schema().field_with_name(column).map_err(|_| {
        CityParquetError::Schema(format!("column '{column}' missing from the file's schema"))
    })?;

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

use arrow_array::{Float64Array, Int64Array};
use arrow_schema::DataType;

use crate::query::{AttrStats, column_statistics, statistics_min_max};

/// The async mirror of [`crate::query::attr_stats`]: the same
/// statistics-fast-path-then-scan structure, over `column_statistics`/
/// `statistics_min_max` from `crate::query` (Parquet row-group metadata is
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

    builder.schema().field_with_name(column).map_err(|_| {
        CityParquetError::Schema(format!("column '{column}' missing from the file's schema"))
    })?;

    let metadata = Arc::clone(builder.metadata());
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

    let projection = ProjectionMask::columns(builder.parquet_schema(), [column]);
    let mut stream = builder
        .with_projection(projection)
        .build()
        .map_err(parquet_err)?;

    let mut sum = 0f64;
    let mut count = 0u64;
    let mut scan_min = f64::INFINITY;
    let mut scan_max = f64::NEG_INFINITY;

    while let Some(batch) = stream.try_next().await.map_err(parquet_err)? {
        let array = batch.column(0);
        let mut visit = |v: f64| {
            sum += v;
            count += 1;
            if !stats_available {
                scan_min = scan_min.min(v);
                scan_max = scan_max.max(v);
            }
        };
        match array.data_type() {
            DataType::Int64 => {
                let values = array.as_any().downcast_ref::<Int64Array>().ok_or_else(|| {
                    CityParquetError::Schema(format!("column '{column}' is not Int64"))
                })?;
                for i in 0..values.len() {
                    if !values.is_null(i) {
                        visit(values.value(i) as f64);
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
                        visit(values.value(i));
                    }
                }
            }
            other => {
                return Err(CityParquetError::Schema(format!(
                    "column '{column}' has an arrow type attr_stats cannot aggregate: {other:?}"
                )));
            }
        }
    }

    let (min, max) = if stats_available {
        (stats_min, stats_max)
    } else {
        (scan_min, scan_max)
    };
    Ok(AttrStats {
        min,
        max,
        sum,
        count,
    })
}

use arrow_array::BooleanArray;

/// The async mirror of [`crate::query::id_lookup`]: filters to `id` via a
/// [`RowFilter`], then fully decodes the (expected exactly one) surviving
/// row.
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

    let predicate_mask = ProjectionMask::columns(builder.parquet_schema(), ["id"]);
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
    let row_filter = RowFilter::new(vec![Box::new(predicate_fn)]);

    let mut stream = builder
        .with_row_filter(row_filter)
        .build()
        .map_err(parquet_err)?;

    while let Some(batch) = stream.try_next().await.map_err(parquet_err)? {
        if batch.num_rows() == 0 {
            continue;
        }
        let batch = restamp(batch, &schema)?;
        let decoded = decode_batch(&batch, meta)?;
        if let Some(object) = decoded.into_iter().next() {
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

    builder.schema().field_with_name(column).map_err(|_| {
        CityParquetError::Schema(format!("column '{column}' missing from the file's schema"))
    })?;

    let projection = ProjectionMask::columns(builder.parquet_schema(), [column]);
    let mut stream = builder
        .with_projection(projection)
        .build()
        .map_err(parquet_err)?;

    let mut count = 0u64;
    while let Some(batch) = stream.try_next().await.map_err(parquet_err)? {
        let array = batch.column(0);
        count += (array.len() - array.null_count()) as u64;
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
