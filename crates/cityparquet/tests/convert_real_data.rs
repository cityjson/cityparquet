use std::path::PathBuf;

use cityparquet::package::{ConvertOptions, convert};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

#[test]
fn delft_full_convert_round_trips_through_parquet() {
    let out = tempfile::tempdir().unwrap();
    let report = convert(&ConvertOptions::new(
        fixture("delft.city.jsonl"),
        out.path().to_path_buf(),
    ))
    .unwrap();
    assert_eq!(report.object_count, 2231);

    let file = std::fs::File::open(out.path().join("cityobjects.parquet")).unwrap();
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
    let pq_meta = builder.metadata().file_metadata();
    let kvs = pq_meta.key_value_metadata().unwrap();
    assert!(kvs.iter().any(|kv| kv.key == "cityparquet_version"));
    assert!(kvs.iter().any(|kv| kv.key == "geo"));
    // bbox stats exist for row-group pruning
    let rg = builder.metadata().row_group(0);
    let bbox_xmin_col = (0..rg.num_columns())
        .map(|i| rg.column(i))
        .find(|c| c.column_path().string() == "bbox.xmin")
        .expect("bbox.xmin column chunk");
    assert!(bbox_xmin_col.statistics().is_some());
    let rows: usize = builder
        .build()
        .unwrap()
        .map(|b| b.unwrap().num_rows())
        .sum();
    assert_eq!(rows, 2231);

    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(out.path().join("metadata.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["profile"], "core");
}

#[test]
fn railway_full_convert_succeeds() {
    let out = tempfile::tempdir().unwrap();
    let report = convert(&ConvertOptions::new(
        fixture("lod3_railway.city.json"),
        out.path().to_path_buf(),
    ))
    .unwrap();
    assert_eq!(report.object_count, 121);
    assert!(out.path().join("cityobjects.parquet").exists());
}
