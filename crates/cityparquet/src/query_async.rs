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

#![cfg(feature = "object-store")]

use std::sync::Arc;

use object_store::ObjectStore;
use object_store::path::Path as ObjectPath;
use parquet::arrow::ParquetRecordBatchStreamBuilder;
use parquet::arrow::async_reader::ParquetObjectReader;

use cityparquet_schema::{CityParquetError, Result};

fn parquet_err(e: impl std::fmt::Display) -> CityParquetError {
    CityParquetError::Parquet(e.to_string())
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
}
