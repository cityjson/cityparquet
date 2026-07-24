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
        .map_err(|e| CityParquetError::Parquet(e.to_string()))
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

/// The table's row count straight from Parquet file metadata — O(1), one
/// footer fetch (a suffix range request), no row scan. The async mirror of
/// [`crate::query::count`].
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
        assert_eq!(async_result.row_groups_touched, sync_result.row_groups_touched);
        let mut sync_ids = sync_result.ids.clone();
        let mut async_ids = async_result.ids.clone();
        sync_ids.sort();
        async_ids.sort();
        assert_eq!(async_ids, sync_ids);
    }
}
