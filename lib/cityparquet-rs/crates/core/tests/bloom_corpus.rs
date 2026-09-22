//! Bloom-filter acceptance at corpus scale (spec Acceptance 2, 3, 4, 4b).
//! Ignored by default: they need the 3DBAG scaling slices and minutes of
//! conversion. Run from the repository root with
//!
//! ```sh
//! mkdir -p benchmark/runs/work/bloom-acceptance
//! CITYPARQUET_SCALING_DIR=$PWD/benchmark/runs/data/scaling \
//! CITYPARQUET_BLOOM_SCRATCH=$PWD/benchmark/runs/work/bloom-acceptance \
//! cargo test --release --all-features --manifest-path lib/cityparquet-rs/Cargo.toml \
//!   -p cityparquet --test bloom_corpus -- --ignored --nocapture --test-threads 1
//! ```
#![cfg(feature = "object-store")]

use std::collections::BTreeMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use arrow_array::{Array, StringArray};
use cityparquet::counting_store::CountingObjectStore;
use cityparquet::package::{ConvertOptions, convert};
use cityparquet::query::{feature_lookup_with_stats, id_lookup, id_lookup_with_stats};
use cityparquet::query_async::{bloom_keep_row_groups_async, id_lookup_async};
use cityparquet::reader::CityParquetReaderBuilder;
use cityparquet::recipe::{BloomPolicy, WriterRecipe};
use cityparquet::schema::CityMetadata;
use object_store::ObjectStore;
use object_store::local::LocalFileSystem;
use object_store::path::Path as ObjectPath;
use parquet::arrow::ProjectionMask;
use parquet::arrow::arrow_reader::{
    ArrowReaderMetadata, ArrowReaderOptions, ParquetRecordBatchReaderBuilder,
};
use parquet::arrow::async_reader::{AsyncFileReader, ParquetObjectReader};
use parquet::file::metadata::ParquetMetaData;
use parquet::schema::types::ColumnPath;

const MISS: &str = "NL.IMBAG.Pand.readbench-absent";

fn slice(name: &str) -> PathBuf {
    let dir = std::env::var("CITYPARQUET_SCALING_DIR")
        .expect("set CITYPARQUET_SCALING_DIR to the directory holding the 3DBAG slices");
    let path = Path::new(&dir).join(name);
    assert!(path.exists(), "missing {}", path.display());
    path
}

fn scratch() -> tempfile::TempDir {
    let dir = std::env::var("CITYPARQUET_BLOOM_SCRATCH")
        .expect("set CITYPARQUET_BLOOM_SCRATCH to a directory under benchmark/runs/");
    tempfile::tempdir_in(dir).unwrap()
}

/// Converts `input` at `row_group_size` rows per group, with or without
/// filters, into a fresh scratch package; returns it and its main table.
fn convert_slice(input: &Path, row_group_size: usize, bloom: bool) -> (tempfile::TempDir, PathBuf) {
    let out = scratch();
    let mut opts = ConvertOptions::new(input.to_path_buf(), out.path().to_path_buf());
    opts.recipe = WriterRecipe {
        row_group_size,
        bloom: BloomPolicy {
            enabled: bloom,
            ..BloomPolicy::default()
        },
        ..WriterRecipe::default()
    };
    convert(&opts).unwrap();
    let table = out.path().join("building.parquet");
    assert!(table.exists(), "3DBAG slices are single-table (building)");
    (out, table)
}

fn column_values(table: &Path, column: &str) -> Vec<String> {
    let builder = ParquetRecordBatchReaderBuilder::try_new(File::open(table).unwrap()).unwrap();
    let mask = ProjectionMask::columns(builder.parquet_schema(), [column]);
    let mut out = Vec::new();
    for batch in builder.with_projection(mask).build().unwrap() {
        let batch = batch.unwrap();
        let values = batch
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        out.extend((0..values.len()).map(|i| values.value(i).to_string()));
    }
    out
}

fn table_meta(table: &Path) -> CityMetadata {
    ParquetRecordBatchReaderBuilder::try_new(File::open(table).unwrap())
        .unwrap()
        .cityparquet_metadata()
        .unwrap()
}

/// Acceptance 2: no false negatives — every `id` of `3dbag_n10000` written
/// at 512-row groups is found by `id_lookup` and `id_lookup_async`.
#[test]
#[ignore]
fn every_id_of_n10000_at_rg512_is_found_sync_and_async() {
    let (out, table) = convert_slice(&slice("3dbag_n10000.city.jsonl"), 512, true);
    let meta = table_meta(&table);
    let ids = column_values(&table, "id");
    println!("n10000 at rg512: {} ids", ids.len());
    let store: Arc<dyn ObjectStore> =
        Arc::new(LocalFileSystem::new_with_prefix(out.path()).unwrap());
    let path = ObjectPath::from("building.parquet");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    for id in &ids {
        let found = id_lookup(&table, &meta, id).unwrap();
        assert_eq!(found.map(|o| o.id), Some(id.clone()), "sync lost {id}");
        let found = runtime
            .block_on(id_lookup_async(Arc::clone(&store), &path, &meta, id))
            .unwrap();
        assert_eq!(found.map(|o| o.id), Some(id.clone()), "async lost {id}");
    }
}

/// An `AsyncFileReader` that counts its `get_byte_ranges` calls.
#[derive(Clone)]
struct RangeCallCounter<R> {
    inner: R,
    calls: Arc<AtomicUsize>,
}

impl<R: AsyncFileReader> AsyncFileReader for RangeCallCounter<R> {
    fn get_bytes(
        &mut self,
        range: std::ops::Range<u64>,
    ) -> futures::future::BoxFuture<'_, parquet::errors::Result<bytes::Bytes>> {
        self.inner.get_bytes(range)
    }

    fn get_byte_ranges(
        &mut self,
        ranges: Vec<std::ops::Range<u64>>,
    ) -> futures::future::BoxFuture<'_, parquet::errors::Result<Vec<bytes::Bytes>>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inner.get_byte_ranges(ranges)
    }

    fn get_metadata<'a>(
        &'a mut self,
        options: Option<&'a ArrowReaderOptions>,
    ) -> futures::future::BoxFuture<'a, parquet::errors::Result<Arc<ParquetMetaData>>> {
        self.inner.get_metadata(options)
    }
}

/// Acceptance 3 and 4 on the 1M slice at 512-row groups: a miss is ruled out
/// by the filters in at least 95 % of the row groups, for `id` and for
/// `feature_id`; and the async prune fetches every `id` filter through one
/// `get_byte_ranges` call that the store serves in at most 8 requests.
#[test]
#[ignore]
fn a_miss_on_the_1m_slice_at_rg512_is_pruned_and_fetched_in_few_requests() {
    let (out, table) = convert_slice(&slice("3dbag_n1000000.city.jsonl"), 512, true);
    let meta = table_meta(&table);

    let (found, id_stats) = id_lookup_with_stats(&table, &meta, MISS).unwrap();
    assert!(found.is_none());
    let (objects, feature_stats) = feature_lookup_with_stats(&table, &meta, MISS).unwrap();
    assert!(objects.is_empty());
    println!("id: {id_stats:?}\nfeature_id: {feature_stats:?}");
    for stats in [id_stats, feature_stats] {
        assert!(stats.row_groups_total >= 1950, "{stats:?}");
        assert!(
            stats.bloom_pruned * 100 >= stats.row_groups_total * 95,
            "pruned {} of {}",
            stats.bloom_pruned,
            stats.row_groups_total
        );
    }

    let counting = Arc::new(CountingObjectStore::new(
        LocalFileSystem::new_with_prefix(out.path()).unwrap(),
    ));
    let calls = Arc::new(AtomicUsize::new(0));
    let mut reader = RangeCallCounter {
        inner: ParquetObjectReader::new(
            Arc::clone(&counting) as Arc<dyn ObjectStore>,
            ObjectPath::from("building.parquet"),
        ),
        calls: Arc::clone(&calls),
    };
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let arrow_meta = runtime
        .block_on(ArrowReaderMetadata::load_async(
            &mut reader,
            ArrowReaderOptions::new(),
        ))
        .unwrap();
    let candidates: Vec<usize> = (0..arrow_meta.metadata().num_row_groups()).collect();
    let before = counting.tally();
    calls.store(0, Ordering::SeqCst);
    let prune = runtime
        .block_on(bloom_keep_row_groups_async(
            &mut reader,
            &arrow_meta,
            &ColumnPath::new(vec!["id".to_string()]),
            &[MISS],
            &candidates,
        ))
        .unwrap();
    let after = counting.tally();
    println!(
        "async prune: {} filters, {} requests, {} bytes, {prune:?}",
        candidates.len(),
        after.requests - before.requests,
        after.bytes - before.bytes
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(after.requests - before.requests <= 8);
}

/// Acceptance 4b at 3DBAG scale: `feature_lookup` returns every row of a
/// multi-part feature (a Building and its BuildingParts), identical with and
/// without filters.
#[test]
#[ignore]
fn feature_lookup_returns_every_part_of_multi_part_3dbag_features() {
    let input = slice("3dbag_n10000.city.jsonl");
    let (_on, on_table) = convert_slice(&input, 512, true);
    let (_off, off_table) = convert_slice(&input, 512, false);
    let mut expected: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (id, feature) in column_values(&on_table, "id")
        .into_iter()
        .zip(column_values(&on_table, "feature_id"))
    {
        expected.entry(feature).or_default().push(id);
    }
    let multi: Vec<(&String, &Vec<String>)> =
        expected.iter().filter(|(_, ids)| ids.len() >= 2).collect();
    assert!(
        !multi.is_empty(),
        "3DBAG features are Building + BuildingPart"
    );
    for (feature, ids) in multi.iter().step_by((multi.len() / 200).max(1)) {
        for table in [&on_table, &off_table] {
            let (objects, _) =
                feature_lookup_with_stats(table, &table_meta(table), feature).unwrap();
            let got: Vec<String> = objects.into_iter().map(|o| o.id).collect();
            assert_eq!(&got, *ids, "{feature} in {}", table.display());
        }
    }
}
