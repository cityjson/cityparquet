//! Bloom filters in written packages, read back from the Parquet footer:
//! which columns carry one, where the filters sit, and which files carry
//! none. Real fixtures only.

use std::collections::BTreeMap;
use std::fs::File;
use std::path::{Path, PathBuf};

use arrow_array::{Array, StringArray};
use cityparquet::package::{ConvertOptions, convert};
use cityparquet::partition::{PartitionSpec, convert_partitioned};
use cityparquet::query::{
    AttrPredicate, attr_filter, attr_filter_with_stats, bloom_keep_row_groups, bloom_targets,
    id_lookup, id_lookup_with_stats,
};
use cityparquet::reader::CityParquetReaderBuilder;
use cityparquet::recipe::{BloomPolicy, RecipePreset, WriterRecipe};
use cityparquet::schema::CityMetadata;
use cityparquet::source::Source;
use cityparquet::stac::properties::PackageTables;
use parquet::arrow::ProjectionMask;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::file::reader::{FileReader, SerializedFileReader};
use parquet::schema::types::ColumnPath;

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

/// Converts `fixture_name` with `recipe` (source order, LoD0 synthesis on)
/// into a fresh temporary package.
fn convert_with(fixture_name: &str, recipe: WriterRecipe) -> tempfile::TempDir {
    let out = tempfile::tempdir().unwrap();
    let mut opts = ConvertOptions::new(fixture(fixture_name), out.path().to_path_buf());
    opts.recipe = recipe;
    convert(&opts).unwrap();
    out
}

/// Dotted column path -> number of row groups whose chunk declares a filter.
fn filtered_columns(table: &Path) -> BTreeMap<String, usize> {
    let reader = SerializedFileReader::new(File::open(table).unwrap()).unwrap();
    let mut out = BTreeMap::new();
    for rg in reader.metadata().row_groups() {
        for chunk in rg.columns() {
            if chunk.bloom_filter_offset().is_some() {
                *out.entry(chunk.column_path().string()).or_insert(0) += 1;
            }
        }
    }
    out
}

fn row_group_count(table: &Path) -> usize {
    SerializedFileReader::new(File::open(table).unwrap())
        .unwrap()
        .metadata()
        .num_row_groups()
}

/// Every filter starts at or after the last byte of column data, and
/// declares its length.
fn assert_filters_follow_the_data(table: &Path) {
    let reader = SerializedFileReader::new(File::open(table).unwrap()).unwrap();
    let meta = reader.metadata();
    let data_end = meta
        .row_groups()
        .iter()
        .flat_map(|rg| rg.columns())
        .map(|c| {
            let (start, len) = c.byte_range();
            start + len
        })
        .max()
        .expect("a table has at least one column chunk");
    for rg in meta.row_groups() {
        for chunk in rg.columns() {
            if let Some(offset) = chunk.bloom_filter_offset() {
                assert!(
                    offset as u64 >= data_end,
                    "{}: filter of {} at {offset} precedes the end of data {data_end}",
                    table.display(),
                    chunk.column_path().string()
                );
                assert!(
                    chunk.bloom_filter_length().is_some(),
                    "{}: filter of {} declares no length",
                    table.display(),
                    chunk.column_path().string()
                );
            }
        }
    }
}

fn delft_recipe(row_group_size: usize) -> WriterRecipe {
    WriterRecipe {
        row_group_size,
        ..WriterRecipe::default()
    }
}

#[test]
fn the_default_recipe_filters_the_identifiers_and_high_cardinality_attributes() {
    let out = convert_with("delft.city.jsonl", delft_recipe(512));
    let table = out.path().join("building.parquet");
    let groups = row_group_count(&table);
    assert_eq!(groups, 5, "2231 rows at 512 per group");
    let expected: BTreeMap<String, usize> = [
        ("documentnummer", groups),
        ("feature_id", groups),
        ("id", groups),
        ("identificatie", groups),
    ]
    .into_iter()
    .map(|(name, n)| (name.to_string(), n))
    .collect();
    assert_eq!(filtered_columns(&table), expected);
    assert_filters_follow_the_data(&table);
}

#[test]
fn nobloom_and_parquet_defaults_write_no_filter() {
    let nobloom = WriterRecipe {
        bloom: BloomPolicy {
            enabled: false,
            ..BloomPolicy::default()
        },
        ..delft_recipe(512)
    };
    let parquet_defaults = WriterRecipe {
        row_group_size: 512,
        ..RecipePreset::ParquetDefaults.recipe()
    };
    for recipe in [nobloom, parquet_defaults] {
        let out = convert_with("delft.city.jsonl", recipe);
        assert!(
            filtered_columns(&out.path().join("building.parquet")).is_empty(),
            "{recipe:?}"
        );
    }
}

#[test]
fn sidecars_carry_no_filter_and_every_object_table_filters_its_identifiers() {
    let out = convert_with("lod3_railway.city.json", WriterRecipe::default());
    let tables = PackageTables::open(out.path()).unwrap();
    assert!(
        !tables.sidecar_files.is_empty(),
        "railway must write sidecars, or this test proves nothing"
    );
    for sidecar in &tables.sidecar_files {
        let path = out.path().join(sidecar);
        assert!(
            filtered_columns(&path).is_empty(),
            "sidecar {sidecar} carries a filter"
        );
    }
    for table in &tables.tables {
        let filtered = filtered_columns(table);
        for name in ["id", "feature_id"] {
            assert_eq!(
                filtered.get(name).copied(),
                Some(row_group_count(table)),
                "{}: {name}",
                table.display()
            );
        }
        assert_filters_follow_the_data(table);
    }
}

/// 1115 features in 300 contiguous chunks leaves 3 or 4 Buildings per
/// partition, where a partition-local rule would select `status` everywhere
/// (one distinct value of at most four is at least 0.2). Every partition
/// must carry the dataset-wide decision instead.
#[test]
fn partitions_share_the_dataset_wide_attribute_decision() {
    let out = tempfile::tempdir().unwrap();
    let src = Source::open(&fixture("delft.city.jsonl")).unwrap();
    let opts = ConvertOptions::new(fixture("delft.city.jsonl"), out.path().to_path_buf());
    let report = convert_partitioned(
        std::slice::from_ref(&src),
        &PartitionSpec::Count(300),
        &opts,
    )
    .unwrap();
    assert_eq!(report.partitions.len(), 300);
    for (label, _) in &report.partitions {
        let filtered = filtered_columns(&out.path().join(label).join("building.parquet"));
        for name in ["id", "feature_id", "identificatie", "documentnummer"] {
            assert!(
                filtered.contains_key(name),
                "{label} lacks {name}: {filtered:?}"
            );
        }
        assert!(!filtered.contains_key("status"), "{label}: {filtered:?}");
    }
}

/// A miss for every delft `id`/`feature_id`/`identificatie`: the 3DBAG
/// prefix with a suffix no BAG identifier carries.
const MISS: &str = "NL.IMBAG.Pand.readbench-absent";

/// Every non-null value of top-level Utf8 `column`, with its row group. The
/// column is selected by its exact root index, never by a dotted name.
fn values_by_row_group(table: &Path, column: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    for rg in 0..row_group_count(table) {
        let builder = ParquetRecordBatchReaderBuilder::try_new(File::open(table).unwrap()).unwrap();
        let root = builder
            .parquet_schema()
            .root_schema()
            .get_fields()
            .iter()
            .position(|f| f.name() == column)
            .unwrap_or_else(|| panic!("no column {column}"));
        let mask = ProjectionMask::roots(builder.parquet_schema(), [root]);
        let reader = builder
            .with_projection(mask)
            .with_row_groups(vec![rg])
            .build()
            .unwrap();
        for batch in reader {
            let batch = batch.unwrap();
            let values = batch
                .column(0)
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap();
            for i in 0..values.len() {
                if !values.is_null(i) {
                    out.push((rg, values.value(i).to_string()));
                }
            }
        }
    }
    out
}

fn table_meta(table: &Path) -> CityMetadata {
    ParquetRecordBatchReaderBuilder::try_new(File::open(table).unwrap())
        .unwrap()
        .cityparquet_metadata()
        .unwrap()
}

fn footer(table: &Path) -> std::sync::Arc<parquet::file::metadata::ParquetMetaData> {
    ParquetRecordBatchReaderBuilder::try_new(File::open(table).unwrap())
        .unwrap()
        .metadata()
        .clone()
}

fn id_path() -> ColumnPath {
    ColumnPath::new(vec!["id".to_string()])
}

fn nobloom(row_group_size: usize) -> WriterRecipe {
    WriterRecipe {
        bloom: BloomPolicy {
            enabled: false,
            ..BloomPolicy::default()
        },
        ..delft_recipe(row_group_size)
    }
}

#[test]
fn bloom_targets_locate_one_filter_per_candidate_in_row_group_order() {
    let out = convert_with("delft.city.jsonl", delft_recipe(64));
    let table = out.path().join("building.parquet");
    let meta = footer(&table);
    assert_eq!(meta.num_row_groups(), 35, "2231 rows at 64 per group");

    let targets = bloom_targets(&meta, &id_path(), &[1, 3, 34]);
    assert!(targets.leaf.is_some());
    assert!(targets.without_filter.is_empty());
    let groups: Vec<usize> = targets.with_filter.iter().map(|t| t.row_group).collect();
    assert_eq!(groups, vec![1, 3, 34]);
    assert!(targets.with_filter.iter().all(|t| t.length.is_some()));
    assert!(
        targets
            .with_filter
            .windows(2)
            .all(|w| w[0].offset < w[1].offset),
        "End placement lays filters out row group by row group"
    );

    let absent = bloom_targets(
        &meta,
        &ColumnPath::new(vec!["no_such".to_string()]),
        &[0, 1],
    );
    assert_eq!(absent.leaf, None);
    assert_eq!(absent.without_filter, vec![0, 1]);

    let off = convert_with("delft.city.jsonl", nobloom(64));
    let off_targets = bloom_targets(
        &footer(&off.path().join("building.parquet")),
        &id_path(),
        &[0, 1, 2],
    );
    assert!(off_targets.leaf.is_some());
    assert!(off_targets.with_filter.is_empty());
    assert_eq!(off_targets.without_filter, vec![0, 1, 2]);
}

/// No false negatives: for every id in the table, the pruner keeps the row
/// group that holds it.
#[test]
fn the_bloom_prune_never_drops_the_row_group_holding_an_id() {
    let out = convert_with("delft.city.jsonl", delft_recipe(64));
    let table = out.path().join("building.parquet");
    let meta = footer(&table);
    let file = File::open(&table).unwrap();
    let all: Vec<usize> = (0..meta.num_row_groups()).collect();
    let ids = values_by_row_group(&table, "id");
    assert_eq!(ids.len(), 2231);
    for (rg, id) in &ids {
        let prune = bloom_keep_row_groups(&file, &meta, &id_path(), &[id.as_str()], &all).unwrap();
        assert!(
            prune.keep.contains(rg),
            "{id} lives in row group {rg}: {prune:?}"
        );
        assert_eq!(prune.total, all.len());
        assert_eq!(prune.without_filter, 0);
        assert_eq!(prune.keep.len() + prune.pruned, all.len());
        assert_eq!(prune.filter_bytes % 32, 0, "bitset bytes are whole blocks");
    }
}

/// IN semantics: probing several values keeps exactly the union of what each
/// value keeps on its own.
#[test]
fn a_multi_value_probe_keeps_the_union_of_its_values() {
    let out = convert_with("delft.city.jsonl", delft_recipe(64));
    let table = out.path().join("building.parquet");
    let meta = footer(&table);
    let file = File::open(&table).unwrap();
    let all: Vec<usize> = (0..meta.num_row_groups()).collect();
    let ids = values_by_row_group(&table, "id");
    let (first_rg, first) = &ids[0];
    let (last_rg, last) = ids.last().unwrap();
    assert_ne!(first_rg, last_rg);
    let keep = |values: &[&str]| {
        bloom_keep_row_groups(&file, &meta, &id_path(), values, &all)
            .unwrap()
            .keep
    };
    let mut union = keep(&[first.as_str()]);
    union.extend(keep(&[last.as_str()]));
    union.extend(keep(&[MISS]));
    union.sort_unstable();
    union.dedup();
    let both = keep(&[first.as_str(), last.as_str(), MISS]);
    assert_eq!(both, union);
    assert!(both.contains(first_rg) && both.contains(last_rg));
    let misses = bloom_keep_row_groups(
        &file,
        &meta,
        &id_path(),
        &[MISS, "NL.IMBAG.Pand.readbench-absent-2"],
        &all,
    )
    .unwrap();
    assert!(misses.pruned >= 1, "{misses:?}");
}

#[test]
fn id_lookup_finds_sampled_ids_and_reports_the_prune() {
    let out = convert_with("delft.city.jsonl", delft_recipe(64));
    let table = out.path().join("building.parquet");
    let meta = table_meta(&table);
    for (_, id) in values_by_row_group(&table, "id").iter().step_by(97) {
        let (found, stats) = id_lookup_with_stats(&table, &meta, id).unwrap();
        assert_eq!(found.as_ref().map(|o| o.id.as_str()), Some(id.as_str()));
        assert_eq!(stats.row_groups_total, 35);
        assert!(stats.bloom_pruned < stats.row_groups_total);
        assert!(stats.filter_bytes > 0);
        assert_eq!(
            id_lookup(&table, &meta, id).unwrap().map(|o| o.id),
            Some(id.clone())
        );
    }
}

#[test]
fn a_miss_is_pruned_by_the_filters_and_unpruned_without_them() {
    let on = convert_with("delft.city.jsonl", delft_recipe(64));
    let table = on.path().join("building.parquet");
    let (found, stats) = id_lookup_with_stats(&table, &table_meta(&table), MISS).unwrap();
    assert!(found.is_none());
    assert_eq!(stats.row_groups_total, 35);
    assert!(stats.bloom_pruned >= 1, "{stats:?}");

    let off = convert_with("delft.city.jsonl", nobloom(64));
    let table = off.path().join("building.parquet");
    let (found, stats) = id_lookup_with_stats(&table, &table_meta(&table), MISS).unwrap();
    assert!(found.is_none());
    assert_eq!(
        stats,
        cityparquet::query::LookupStats {
            row_groups_total: 35,
            bloom_pruned: 0,
            filter_bytes: 0,
        }
    );
}

/// `attr_filter` through its own pruning: a unique filtered string
/// (`identificatie`), a repeated filtered string (`documentnummer`, its most
/// frequent value), a miss, and an unfiltered column (`status`). Every count
/// equals an independent count and the unfiltered package's count; the
/// statistics show the filters at work.
#[test]
fn attr_filter_prunes_filtered_string_columns_and_counts_exactly() {
    let on = convert_with("delft.city.jsonl", delft_recipe(64));
    let off = convert_with("delft.city.jsonl", nobloom(64));
    let on_table = on.path().join("building.parquet");
    let off_table = off.path().join("building.parquet");
    let filtered = filtered_columns(&on_table);
    assert!(filtered.contains_key("identificatie") && filtered.contains_key("documentnummer"));

    let count_of = |column: &str, value: &str| -> u64 {
        values_by_row_group(&on_table, column)
            .iter()
            .filter(|(_, v)| v == value)
            .count() as u64
    };
    let (_, unique) = values_by_row_group(&on_table, "identificatie")
        .into_iter()
        .nth(500)
        .unwrap();
    let mut documents: BTreeMap<String, u64> = BTreeMap::new();
    for (_, value) in values_by_row_group(&on_table, "documentnummer") {
        *documents.entry(value).or_insert(0) += 1;
    }
    let (repeated, repeated_count) = documents
        .iter()
        .max_by_key(|(_, n)| **n)
        .map(|(v, n)| (v.clone(), *n))
        .unwrap();
    assert!(repeated_count > 1);

    // A value in few row groups must be pruned from some others; a repeated
    // value may legitimately sit in every row group, so for it only the
    // no-false-negative bound is asserted.
    for (column, value, expected, must_prune) in [
        ("identificatie", unique.as_str(), 1, true),
        ("documentnummer", repeated.as_str(), repeated_count, false),
        ("identificatie", MISS, 0, true),
    ] {
        assert_eq!(count_of(column, value), expected, "{column}={value}");
        let holding: std::collections::BTreeSet<usize> = values_by_row_group(&on_table, column)
            .into_iter()
            .filter(|(_, v)| v == value)
            .map(|(rg, _)| rg)
            .collect();
        let pred = AttrPredicate::Eq(serde_json::Value::String(value.to_string()));
        let (count, stats) = attr_filter_with_stats(&on_table, column, &pred).unwrap();
        assert_eq!(count, expected, "{column}={value}");
        assert_eq!(attr_filter(&on_table, column, &pred).unwrap(), expected);
        assert_eq!(stats.row_groups_total, 35);
        assert!(
            stats.bloom_pruned + holding.len() <= 35,
            "{column}={value}: {stats:?}"
        );
        if must_prune {
            assert!(stats.bloom_pruned >= 1, "{column}={value}: {stats:?}");
        }
        assert!(stats.filter_bytes > 0);
        let (off_count, off_stats) = attr_filter_with_stats(&off_table, column, &pred).unwrap();
        assert_eq!(off_count, expected);
        assert_eq!((off_stats.bloom_pruned, off_stats.filter_bytes), (0, 0));
    }

    let status = AttrPredicate::Eq(serde_json::Value::String("Pand in gebruik".to_string()));
    let (count, stats) = attr_filter_with_stats(&on_table, "status", &status).unwrap();
    assert_eq!(count, count_of("status", "Pand in gebruik"));
    assert!(count > 0);
    assert_eq!(
        (stats.bloom_pruned, stats.filter_bytes),
        (0, 0),
        "status carries no filter"
    );

    let parts = AttrPredicate::Eq(serde_json::Value::String("BuildingPart".to_string()));
    assert_eq!(attr_filter(&on_table, "object_type", &parts).unwrap(), 1116);
}

/// The predicate's type rules are checked on the column's own type before
/// any filter is read, so the same error comes back with and without filters.
#[test]
fn attr_filter_type_errors_do_not_depend_on_filters() {
    let on = convert_with("delft.city.jsonl", delft_recipe(64));
    let off = convert_with("delft.city.jsonl", nobloom(64));
    for (column, pred, message) in [
        (
            "identificatie",
            AttrPredicate::Ge(1.0),
            "only `Eq(<string>)` applies",
        ),
        (
            "identificatie",
            AttrPredicate::Eq(serde_json::json!(1)),
            "only `Eq(<string>)` applies",
        ),
        (
            "oorspronkelijkbouwjaar",
            AttrPredicate::Eq(serde_json::Value::String("1994".to_string())),
            "is numeric",
        ),
    ] {
        for out in [&on, &off] {
            let err = attr_filter(&out.path().join("building.parquet"), column, &pred)
                .unwrap_err()
                .to_string();
            assert!(err.contains(message), "{column} {pred:?}: {err}");
        }
    }
}
