//! Bloom-filter lookups on files whose columns a reader must not assume the
//! shape of: an attribute whose name holds a literal `.`, and identifier and
//! attribute columns a writer handed back as `Dictionary<Int32, Utf8>`.
//! Built from the real delft fixture — a JSON rename, or a re-encoding of a
//! converted table — never hand-written CityJSON.

use std::collections::BTreeSet;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use arrow_array::types::Int32Type;
use arrow_array::{Array, ArrayRef, DictionaryArray, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema};
use cityparquet::package::{ConvertOptions, convert};
use cityparquet::query::{AttrPredicate, attr_filter_with_stats, id_lookup_with_stats};
use cityparquet::reader::CityParquetReaderBuilder;
use cityparquet::recipe::{BloomPolicy, WriterRecipe};
use cityparquet::schema::CityMetadata;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::arrow::{ArrowWriter, ProjectionMask};
use parquet::file::properties::{BloomFilterPosition, WriterProperties};
use parquet::file::reader::{FileReader, SerializedFileReader};
use parquet::schema::types::ColumnPath;

const MISS: &str = "NL.IMBAG.Pand.readbench-absent";
/// The columns the dictionary copy re-encodes.
const DICTIONARY_COLUMNS: [&str; 3] = ["id", "feature_id", "identificatie"];

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

fn recipe(bloom: bool) -> WriterRecipe {
    WriterRecipe {
        row_group_size: 64,
        bloom: BloomPolicy {
            enabled: bloom,
            ..BloomPolicy::default()
        },
        ..WriterRecipe::default()
    }
}

/// delft with every `identificatie` attribute renamed to `to`.
fn delft_renamed(to: &str) -> (tempfile::TempDir, PathBuf) {
    let text = std::fs::read_to_string(fixture("delft.city.jsonl")).unwrap();
    let mut out = String::new();
    for (index, line) in text.lines().enumerate() {
        if index == 0 || line.trim().is_empty() {
            out.push_str(line);
            out.push('\n');
            continue;
        }
        let mut feature: serde_json::Value = serde_json::from_str(line).unwrap();
        for (_, object) in feature["CityObjects"].as_object_mut().unwrap() {
            if let Some(attrs) = object.get_mut("attributes").and_then(|a| a.as_object_mut())
                && let Some(value) = attrs.remove("identificatie")
            {
                attrs.insert(to.to_string(), value);
            }
        }
        out.push_str(&serde_json::to_string(&feature).unwrap());
        out.push('\n');
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("delft_renamed.city.jsonl");
    std::fs::write(&path, out).unwrap();
    (dir, path)
}

fn convert_input(input: &Path, bloom: bool) -> (tempfile::TempDir, PathBuf) {
    let out = tempfile::tempdir().unwrap();
    let mut opts = ConvertOptions::new(input.to_path_buf(), out.path().to_path_buf());
    opts.recipe = recipe(bloom);
    convert(&opts).unwrap();
    let table = out.path().join("building.parquet");
    (out, table)
}

fn table_meta(table: &Path) -> CityMetadata {
    ParquetRecordBatchReaderBuilder::try_new(File::open(table).unwrap())
        .unwrap()
        .cityparquet_metadata()
        .unwrap()
}

/// Every non-null value of `column` (plain or dictionary Utf8), selected by
/// exact root index.
fn values(table: &Path, column: &str) -> Vec<String> {
    let builder = ParquetRecordBatchReaderBuilder::try_new(File::open(table).unwrap()).unwrap();
    let root = builder
        .parquet_schema()
        .root_schema()
        .get_fields()
        .iter()
        .position(|f| f.name() == column)
        .unwrap();
    let mask = ProjectionMask::roots(builder.parquet_schema(), [root]);
    let mut out = Vec::new();
    for batch in builder.with_projection(mask).build().unwrap() {
        let batch = batch.unwrap();
        let column = batch.column(0);
        let plain = match column.as_any().downcast_ref::<DictionaryArray<Int32Type>>() {
            Some(dict) => arrow_select::take::take(dict.values(), dict.keys(), None).unwrap(),
            None => Arc::clone(column),
        };
        let strings = plain.as_any().downcast_ref::<StringArray>().unwrap();
        out.extend(strings.iter().flatten().map(String::from));
    }
    out
}

fn filtered_columns(table: &Path) -> BTreeSet<String> {
    let reader = SerializedFileReader::new(File::open(table).unwrap()).unwrap();
    reader
        .metadata()
        .row_groups()
        .iter()
        .flat_map(|rg| rg.columns())
        .filter(|c| c.bloom_filter_offset().is_some())
        .map(|c| c.column_path().parts().join("/"))
        .collect()
}

/// A copy of `src` with [`DICTIONARY_COLUMNS`] re-encoded as
/// `Dictionary<Int32, Utf8>` (the embedded Arrow schema says so, so a reader
/// hands them back as dictionaries), 64-row groups, the `city`/`geo` footer
/// kept, and — when `bloom` — filters on those three columns.
fn dictionary_copy(src: &Path, dst: &Path, bloom: bool) {
    let builder = ParquetRecordBatchReaderBuilder::try_new(File::open(src).unwrap()).unwrap();
    let key_values: Vec<parquet::file::metadata::KeyValue> = builder
        .metadata()
        .file_metadata()
        .key_value_metadata()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|kv| kv.key != "ARROW:schema")
        .collect();
    let dictionary = DataType::Dictionary(Box::new(DataType::Int32), Box::new(DataType::Utf8));
    let source = builder.schema().clone();
    let fields: Vec<Field> = source
        .fields()
        .iter()
        .map(|f| {
            if DICTIONARY_COLUMNS.contains(&f.name().as_str()) {
                f.as_ref().clone().with_data_type(dictionary.clone())
            } else {
                f.as_ref().clone()
            }
        })
        .collect();
    let target = Arc::new(Schema::new_with_metadata(fields, source.metadata().clone()));
    let mut props = WriterProperties::builder()
        .set_max_row_group_row_count(Some(64))
        .set_key_value_metadata(Some(key_values));
    if bloom {
        props = props.set_bloom_filter_position(BloomFilterPosition::End);
        for name in DICTIONARY_COLUMNS {
            let path = ColumnPath::new(vec![name.to_string()]);
            props = props
                .set_column_bloom_filter_enabled(path.clone(), true)
                .set_column_bloom_filter_fpp(path, 0.01);
        }
    }
    let mut writer = ArrowWriter::try_new(
        File::create(dst).unwrap(),
        Arc::clone(&target),
        Some(props.build()),
    )
    .unwrap();
    for batch in builder.build().unwrap() {
        let batch = batch.unwrap();
        let columns: Vec<ArrayRef> = batch
            .schema()
            .fields()
            .iter()
            .zip(batch.columns())
            .map(|(field, column)| {
                if DICTIONARY_COLUMNS.contains(&field.name().as_str()) {
                    let strings = column.as_any().downcast_ref::<StringArray>().unwrap();
                    Arc::new(strings.iter().collect::<DictionaryArray<Int32Type>>()) as ArrayRef
                } else {
                    Arc::clone(column)
                }
            })
            .collect();
        writer
            .write(&RecordBatch::try_new(Arc::clone(&target), columns).unwrap())
            .unwrap();
    }
    writer.close().unwrap();
}

/// An attribute named `bag.identificatie`: its filter is written under the
/// single-part path, and `attr_filter` finds its rows by exact name — hit,
/// miss, with and without filters. Its Parquet leaf index differs from its
/// Arrow ordinal (every `bbox` and `geometry_properties` leaf precedes it),
/// so resolving by ordinal would probe the wrong column.
#[test]
fn a_dotted_attribute_is_filtered_and_found_by_its_exact_name() {
    let (_dir, input) = delft_renamed("bag.identificatie");
    let (_on, on_table) = convert_input(&input, true);
    let (_off, off_table) = convert_input(&input, false);
    assert!(filtered_columns(&on_table).contains("bag.identificatie"));

    let builder = ParquetRecordBatchReaderBuilder::try_new(File::open(&on_table).unwrap()).unwrap();
    let ordinal = builder.schema().index_of("bag.identificatie").unwrap();
    let leaf = builder
        .parquet_schema()
        .columns()
        .iter()
        .position(|c| c.path().parts() == ["bag.identificatie".to_string()])
        .unwrap();
    assert_ne!(ordinal, leaf);

    let target = values(&on_table, "bag.identificatie")[500].clone();
    for (value, expected) in [(target.as_str(), 1u64), (MISS, 0)] {
        let pred = AttrPredicate::Eq(serde_json::Value::String(value.to_string()));
        let (count, stats) = attr_filter_with_stats(&on_table, "bag.identificatie", &pred).unwrap();
        assert_eq!(count, expected, "{value}");
        assert!(stats.bloom_pruned >= 1, "{value}: {stats:?}");
        let (off_count, off_stats) =
            attr_filter_with_stats(&off_table, "bag.identificatie", &pred).unwrap();
        assert_eq!(off_count, expected, "{value}");
        assert_eq!(off_stats.bloom_pruned, 0);
    }
}

/// Dictionary-typed `id` and `identificatie` through complete lookups, with
/// and without filters: the same objects and counts as the plain package,
/// and pruning only where the filters are.
#[test]
fn dictionary_typed_identifiers_are_looked_up_like_plain_ones() {
    let plain_dir = tempfile::tempdir().unwrap();
    let mut opts = ConvertOptions::new(fixture("delft.city.jsonl"), plain_dir.path().to_path_buf());
    opts.recipe = recipe(false);
    convert(&opts).unwrap();
    let plain = plain_dir.path().join("building.parquet");
    let copies = tempfile::tempdir().unwrap();
    let on = copies.path().join("dictionary_bloom.parquet");
    let off = copies.path().join("dictionary_plain.parquet");
    dictionary_copy(&plain, &on, true);
    dictionary_copy(&plain, &off, false);
    assert_eq!(
        filtered_columns(&on),
        DICTIONARY_COLUMNS
            .iter()
            .map(|c| c.to_string())
            .collect::<BTreeSet<String>>()
    );

    let meta = table_meta(&plain);
    let ids = values(&plain, "id");
    let target = &ids[1000];
    let expected = id_lookup_with_stats(&plain, &meta, target)
        .unwrap()
        .0
        .unwrap();
    assert_eq!(&expected.id, target);
    let identificatie = values(&plain, "identificatie")[500].clone();

    for (table, filtered) in [(&on, true), (&off, false)] {
        let meta = table_meta(table);
        let (found, stats) = id_lookup_with_stats(table, &meta, target).unwrap();
        assert_eq!(found.map(|o| o.id), Some(target.clone()));
        assert_eq!(stats.row_groups_total, 35);
        let (missing, stats) = id_lookup_with_stats(table, &meta, MISS).unwrap();
        assert!(missing.is_none());
        assert_eq!(stats.bloom_pruned >= 1, filtered, "{stats:?}");

        let pred = AttrPredicate::Eq(serde_json::Value::String(identificatie.clone()));
        let (count, stats) = attr_filter_with_stats(table, "identificatie", &pred).unwrap();
        assert_eq!(count, 1);
        assert_eq!(stats.bloom_pruned >= 1, filtered, "{stats:?}");
        assert_eq!(stats.filter_bytes > 0, filtered);
    }
}

#[cfg(feature = "object-store")]
fn local_store(dir: &Path) -> Arc<dyn object_store::ObjectStore> {
    Arc::new(object_store::local::LocalFileSystem::new_with_prefix(dir).unwrap())
}

/// The dotted attribute over the async transport: the same counts and
/// statistics as the sync path.
#[cfg(feature = "object-store")]
#[tokio::test]
async fn a_dotted_attribute_is_found_by_its_exact_name_over_object_store() {
    let (_dir, input) = delft_renamed("bag.identificatie");
    let (on, on_table) = convert_input(&input, true);
    let target = values(&on_table, "bag.identificatie")[500].clone();
    let path = object_store::path::Path::from("building.parquet");
    for value in [target.as_str(), MISS] {
        let pred = AttrPredicate::Eq(serde_json::Value::String(value.to_string()));
        let sync = attr_filter_with_stats(&on_table, "bag.identificatie", &pred).unwrap();
        let async_result = cityparquet::query_async::attr_filter_async_with_stats(
            local_store(on.path()),
            &path,
            "bag.identificatie",
            &pred,
        )
        .await
        .unwrap();
        assert_eq!(async_result, sync, "{value}");
    }
}

/// Dictionary-typed identifiers over the async transport.
#[cfg(feature = "object-store")]
#[tokio::test]
async fn dictionary_typed_identifiers_are_looked_up_over_object_store() {
    let plain_dir = tempfile::tempdir().unwrap();
    let mut opts = ConvertOptions::new(fixture("delft.city.jsonl"), plain_dir.path().to_path_buf());
    opts.recipe = recipe(false);
    convert(&opts).unwrap();
    let plain = plain_dir.path().join("building.parquet");
    let copies = tempfile::tempdir().unwrap();
    dictionary_copy(
        &plain,
        &copies.path().join("dictionary_bloom.parquet"),
        true,
    );
    let table = copies.path().join("dictionary_bloom.parquet");
    let meta = table_meta(&table);
    let target = values(&plain, "id")[1000].clone();
    let path = object_store::path::Path::from("dictionary_bloom.parquet");
    for id in [target.as_str(), MISS] {
        let sync = id_lookup_with_stats(&table, &meta, id).unwrap();
        let async_result = cityparquet::query_async::id_lookup_async_with_stats(
            local_store(copies.path()),
            &path,
            &meta,
            id,
        )
        .await
        .unwrap();
        assert_eq!(async_result.1, sync.1, "{id}");
        assert_eq!(async_result.0.map(|o| o.id), sync.0.map(|o| o.id), "{id}");
    }
}
