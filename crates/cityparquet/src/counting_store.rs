//! `CountingObjectStore<T>`: an `ObjectStore` decorator tallying request
//! count and bytes returned. Overrides only `get_opts` — every byte-range
//! fetch this crate's async reader (or `parquet`'s own `ParquetObjectReader`)
//! makes funnels through it: `ObjectStoreExt::get_range`'s default body is
//! `self.get_opts(...)`, and `ObjectStore::get_ranges`'s default body
//! coalesces into repeated `self.get_range` calls — so overriding `get_opts`
//! alone captures every request, whether a single range, a coalesced
//! multi-range fetch, or a suffix footer fetch. The other 6 required
//! `ObjectStore` methods (`put_opts`, `put_multipart_opts`, `delete_stream`,
//! `list`, `list_with_delimiter`, `copy_opts`) are pure passthroughs — this
//! benchmark never calls them, but the trait requires an impl regardless
//! (object_store 0.13's own docs: wrapper stores SHOULD implement every
//! method, `#[deny(clippy::missing_trait_methods)]`, to avoid silently
//! losing an override the inner store provides).

use std::fmt::Display;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use futures_util::stream::BoxStream;
use object_store::path::Path as ObjectPath;
use object_store::{
    CopyOptions, GetOptions, GetResult, ListResult, MultipartUpload, ObjectMeta, ObjectStore,
    PutMultipartOptions, PutOptions, PutPayload, PutResult, Result as OsResult,
};

/// A point-in-time snapshot of a [`CountingObjectStore`]'s tally.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IoStats {
    pub bytes: u64,
    pub requests: u64,
}

#[derive(Debug, Default)]
struct Tally {
    bytes: AtomicU64,
    requests: AtomicU64,
}

/// Wraps `inner`, tallying every `get_opts` call's request count and
/// returned byte range.
#[derive(Debug)]
pub struct CountingObjectStore<T> {
    inner: T,
    tally: Arc<Tally>,
}

impl<T> CountingObjectStore<T> {
    pub fn new(inner: T) -> Self {
        Self {
            inner,
            tally: Arc::new(Tally::default()),
        }
    }

    /// The current tally. Cheap (two atomic loads); safe to call at any
    /// point, including mid-scan.
    pub fn tally(&self) -> IoStats {
        IoStats {
            bytes: self.tally.bytes.load(Ordering::Relaxed),
            requests: self.tally.requests.load(Ordering::Relaxed),
        }
    }
}

impl<T: Display> Display for CountingObjectStore<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CountingObjectStore({})", self.inner)
    }
}

#[async_trait]
impl<T: ObjectStore> ObjectStore for CountingObjectStore<T> {
    async fn put_opts(
        &self,
        location: &ObjectPath,
        payload: PutPayload,
        opts: PutOptions,
    ) -> OsResult<PutResult> {
        self.inner.put_opts(location, payload, opts).await
    }

    async fn put_multipart_opts(
        &self,
        location: &ObjectPath,
        opts: PutMultipartOptions,
    ) -> OsResult<Box<dyn MultipartUpload>> {
        self.inner.put_multipart_opts(location, opts).await
    }

    async fn get_opts(&self, location: &ObjectPath, options: GetOptions) -> OsResult<GetResult> {
        let result = self.inner.get_opts(location, options).await?;
        self.tally.requests.fetch_add(1, Ordering::Relaxed);
        self.tally
            .bytes
            .fetch_add(result.range.end - result.range.start, Ordering::Relaxed);
        Ok(result)
    }

    fn delete_stream(
        &self,
        locations: BoxStream<'static, OsResult<ObjectPath>>,
    ) -> BoxStream<'static, OsResult<ObjectPath>> {
        self.inner.delete_stream(locations)
    }

    fn list(&self, prefix: Option<&ObjectPath>) -> BoxStream<'static, OsResult<ObjectMeta>> {
        self.inner.list(prefix)
    }

    async fn list_with_delimiter(&self, prefix: Option<&ObjectPath>) -> OsResult<ListResult> {
        self.inner.list_with_delimiter(prefix).await
    }

    async fn copy_opts(
        &self,
        from: &ObjectPath,
        to: &ObjectPath,
        options: CopyOptions,
    ) -> OsResult<()> {
        self.inner.copy_opts(from, to, options).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use object_store::ObjectStoreExt;
    use object_store::local::LocalFileSystem;

    #[tokio::test]
    async fn get_range_over_local_fs_tallies_exact_bytes_and_one_request_per_call() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("data.bin"), b"0123456789ABCDEFGHIJ").unwrap();
        let store = CountingObjectStore::new(LocalFileSystem::new_with_prefix(dir.path()).unwrap());

        let path = ObjectPath::from("data.bin");
        let bytes = store.get_range(&path, 5..10).await.unwrap();
        assert_eq!(bytes.as_ref(), b"56789");
        assert_eq!(
            store.tally(),
            IoStats {
                bytes: 5,
                requests: 1
            }
        );

        let _ = store.get_range(&path, 0..3).await.unwrap();
        assert_eq!(
            store.tally(),
            IoStats {
                bytes: 8,
                requests: 2
            }
        );
    }

    /// The real-HTTP proof: an `axum` + `tower-http::services::ServeDir`
    /// static file server (Range-serving out of the box — confirmed via
    /// `http-range-header` in `tower-http`'s own dependency tree), an
    /// `object_store::http::HttpStore` pointed at it, and a row-group-pruned
    /// Parquet read over the wrapped store — proving `CountingObjectStore`
    /// correctly tallies real HTTP range requests, and that `HttpStore`
    /// needs no WebDAV support for GET+Range.
    #[tokio::test]
    async fn parquet_row_group_pruning_over_real_http_reads_far_fewer_bytes_than_the_file() {
        use arrow_array::{ArrayRef, Int64Array, RecordBatch};
        use futures_util::TryStreamExt;
        use object_store::ClientOptions;
        use object_store::http::HttpBuilder;
        use parquet::arrow::ParquetRecordBatchStreamBuilder;
        use parquet::arrow::async_reader::ParquetObjectReader;
        use parquet::file::properties::WriterProperties;
        use tower_http::services::ServeDir;

        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("t.parquet");
        {
            let file = std::fs::File::create(&file_path).unwrap();
            let col: ArrayRef = Arc::new(Int64Array::from((0..10_000i64).collect::<Vec<_>>()));
            let batch = RecordBatch::try_from_iter(vec![("v", col)]).unwrap();
            let props = WriterProperties::builder()
                .set_max_row_group_row_count(Some(1000))
                .build();
            let mut writer =
                parquet::arrow::ArrowWriter::try_new(file, batch.schema(), Some(props)).unwrap();
            writer.write(&batch).unwrap();
            writer.close().unwrap();
        }
        let file_size = std::fs::metadata(&file_path).unwrap().len();

        let app = axum::Router::new().fallback_service(ServeDir::new(dir.path()));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let store = HttpBuilder::new()
            .with_url(format!("http://{addr}"))
            .with_client_options(ClientOptions::new().with_allow_http(true))
            .build()
            .unwrap();
        let counting = Arc::new(CountingObjectStore::new(store));

        let reader = ParquetObjectReader::new(
            Arc::clone(&counting) as Arc<dyn ObjectStore>,
            ObjectPath::from("t.parquet"),
        );
        let builder = ParquetRecordBatchStreamBuilder::new(reader).await.unwrap();
        let mut stream = builder.with_row_groups(vec![3]).build().unwrap();
        let mut total_rows = 0usize;
        while let Some(batch) = stream.try_next().await.unwrap() {
            total_rows += batch.num_rows();
        }
        assert_eq!(total_rows, 1000);

        let stats = counting.tally();
        assert!(stats.requests >= 1);
        assert!(
            stats.bytes < file_size / 2,
            "expected pruning to read well under half the file ({file_size} bytes), got {}",
            stats.bytes
        );
    }
}
