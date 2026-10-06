//! Cross-WRITER bloom interoperability: a package DuckDB wrote, probed
//! through this crate's lookups.
//!
//! Every other bloom test in this tree writes with parquet-rs and reads with
//! parquet-rs, so a hash or layout disagreement between writers could not show
//! up in any of them. It would not show up as an error either: a filter whose
//! bits were set by a different hash simply says "absent" for a value that IS
//! present, the reader prunes the row group that holds it, and the lookup
//! returns nothing. A silent false negative — and `lib/duckdb-cityjson` is
//! CityLake's only CityJSON writer, so `cityparquet_write` packages read back
//! by `id_lookup` are a production path.
//!
//! Ignored by default and driven by `scripts/interop.sh` (`just interop`),
//! which is where DuckDB is already a dependency and already skips when it is
//! absent. Run by hand with the object table of a `cityparquet_write` package:
//!
//! ```sh
//! CITYPARQUET_DUCKDB_TABLE=/path/to/pkg/building.parquet \
//!   cargo test -p cityparquet --test bloom_duckdb_interop -- --ignored --nocapture
//! ```
//!
//! What this does NOT cover: `cityparquet_write` fixes the row-group size at
//! 122 880, so any fixture small enough to convert quickly is a single row
//! group. The hash agreement — the thing a cross-writer test is for — is
//! fully exercised there; pruning ACROSS many groups of a DuckDB-written file
//! is not, and `crates/core/tests/bloom_corpus.rs` covers that only for
//! packages this crate wrote.

use std::collections::BTreeSet;
use std::fs::File;
use std::path::{Path, PathBuf};

use arrow_array::{Array, StringArray};
use cityparquet::query::{feature_lookup_with_stats, id_lookup_with_stats};
use cityparquet::reader::CityParquetReaderBuilder;
use cityparquet::schema::CityMetadata;
use parquet::arrow::ProjectionMask;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::file::reader::{FileReader, SerializedFileReader};

/// Absent from `id` and from `feature_id`: the 3DBAG identifiers this fixture
/// carries are `NL.IMBAG.Pand.<digits>`, never a `readbench-absent` suffix.
const MISS: &str = "NL.IMBAG.Pand.readbench-absent";

fn table() -> PathBuf {
    let path = PathBuf::from(
        std::env::var("CITYPARQUET_DUCKDB_TABLE")
            .expect("set CITYPARQUET_DUCKDB_TABLE to a cityparquet_write package's object table"),
    );
    assert!(path.exists(), "missing {}", path.display());
    path
}

fn meta(table: &Path) -> CityMetadata {
    ParquetRecordBatchReaderBuilder::try_new(File::open(table).unwrap())
        .unwrap()
        .cityparquet_metadata()
        .unwrap()
}

fn column(table: &Path, name: &str) -> Vec<String> {
    let builder = ParquetRecordBatchReaderBuilder::try_new(File::open(table).unwrap()).unwrap();
    let mask = ProjectionMask::columns(builder.parquet_schema(), [name]);
    let mut out = Vec::new();
    for batch in builder.with_projection(mask).build().unwrap() {
        let batch = batch.unwrap();
        let values = batch
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap_or_else(|| panic!("{name} is not a string column"));
        for i in 0..values.len() {
            if values.is_valid(i) {
                out.push(values.value(i).to_string());
            }
        }
    }
    out
}

/// Row groups whose chunk for `name` declares a filter. Zero would make every
/// assertion below vacuous, so the test asserts on it first.
fn filtered_row_groups(table: &Path, name: &str) -> usize {
    let reader = SerializedFileReader::new(File::open(table).unwrap()).unwrap();
    reader
        .metadata()
        .row_groups()
        .iter()
        .flat_map(|rg| rg.columns())
        .filter(|c| c.column_path().string() == name && c.bloom_filter_offset().is_some())
        .count()
}

/// Every `id` DuckDB wrote is still found through this crate's `id_lookup`,
/// and the row group holding it is never pruned. A hash disagreement between
/// the two writers' filters would show up exactly here: `bloom_pruned` on the
/// group that holds the value, and no object returned.
#[test]
#[ignore]
fn every_duckdb_written_id_survives_the_rust_bloom_prune() {
    let table = table();
    let meta = meta(&table);
    let groups = filtered_row_groups(&table, "id");
    assert!(groups > 0, "the package carries no `id` bloom filter");

    let ids = column(&table, "id");
    assert!(!ids.is_empty(), "the package has no rows to probe");
    for id in &ids {
        let (found, stats) = id_lookup_with_stats(&table, &meta, id).unwrap();
        assert_eq!(
            found.as_ref().map(|o| o.id.as_str()),
            Some(id.as_str()),
            "lost by the prune: {id} ({stats:?})"
        );
        assert_eq!(stats.bloom_pruned, 0, "{id} ({stats:?})");
        assert!(stats.filter_bytes > 0, "{id} ({stats:?})");
    }
    println!("{} DuckDB-written ids probed, none lost", ids.len());
}

/// The same for `feature_id`, which `feature_lookup` joins a feature's parts
/// on: every distinct value returns at least the row that carries it.
#[test]
#[ignore]
fn every_duckdb_written_feature_id_survives_the_rust_bloom_prune() {
    let table = table();
    let meta = meta(&table);
    let groups = filtered_row_groups(&table, "feature_id");
    assert!(
        groups > 0,
        "the package carries no `feature_id` bloom filter"
    );

    let feature_ids: BTreeSet<String> = column(&table, "feature_id").into_iter().collect();
    assert!(!feature_ids.is_empty(), "the package has no feature ids");
    for feature_id in &feature_ids {
        let (objects, stats) = feature_lookup_with_stats(&table, &meta, feature_id).unwrap();
        assert!(
            !objects.is_empty(),
            "lost by the prune: {feature_id} ({stats:?})"
        );
        assert_eq!(stats.bloom_pruned, 0, "{feature_id} ({stats:?})");
        assert!(stats.filter_bytes > 0, "{feature_id} ({stats:?})");
    }
    println!(
        "{} DuckDB-written feature ids probed, none lost",
        feature_ids.len()
    );
}

/// The other half of the contract: a value DuckDB never wrote is ruled out by
/// DuckDB's own filters, read by this crate — every row group pruned, nothing
/// returned. Without this an all-ones filter would pass the two tests above.
#[test]
#[ignore]
fn a_miss_is_pruned_by_the_duckdb_written_filters() {
    let table = table();
    let meta = meta(&table);

    let (found, stats) = id_lookup_with_stats(&table, &meta, MISS).unwrap();
    assert!(found.is_none(), "{MISS} is supposed to be absent");
    assert_eq!(stats.bloom_pruned, stats.row_groups_total, "{stats:?}");

    let (objects, stats) = feature_lookup_with_stats(&table, &meta, MISS).unwrap();
    assert!(objects.is_empty(), "{MISS} is supposed to be absent");
    assert_eq!(stats.bloom_pruned, stats.row_groups_total, "{stats:?}");
}
