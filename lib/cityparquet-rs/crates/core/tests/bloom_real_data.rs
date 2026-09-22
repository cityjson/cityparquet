//! Bloom filters in written packages, read back from the Parquet footer:
//! which columns carry one, where the filters sit, and which files carry
//! none. Real fixtures only.

use std::collections::BTreeMap;
use std::fs::File;
use std::path::{Path, PathBuf};

use cityparquet::package::{ConvertOptions, convert};
use cityparquet::recipe::{BloomPolicy, RecipePreset, WriterRecipe};
use cityparquet::stac::properties::PackageTables;
use parquet::file::reader::{FileReader, SerializedFileReader};

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
fn the_default_recipe_filters_the_identifiers_in_every_row_group() {
    let out = convert_with("delft.city.jsonl", delft_recipe(512));
    let table = out.path().join("building.parquet");
    let groups = row_group_count(&table);
    assert_eq!(groups, 5, "2231 rows at 512 per group");
    let expected: BTreeMap<String, usize> = [("feature_id", groups), ("id", groups)]
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
