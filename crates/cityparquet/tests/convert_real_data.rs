use std::path::PathBuf;

use arrow_array::{Array, StringArray};
use cityparquet::package::{ConvertOptions, convert};
use cityparquet::schema::Profile;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::basic::Encoding;
use serde_json::Value;

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

/// Recursively asserts every material index in a rewritten `material` map is
/// a dataset-global id `< limit` (mirrors `cityparquet::appearance`'s own
/// walk, duplicated here since this is a separate integration-test crate
/// with no access to that module's private test helpers).
fn assert_every_material_index_below(v: &Value, limit: usize) {
    match v {
        Value::Object(map) => {
            for val in map.values() {
                assert_every_material_index_below(val, limit);
            }
        }
        Value::Array(items) => {
            for x in items {
                assert_every_material_index_below(x, limit);
            }
        }
        Value::Number(n) => {
            let idx = n.as_u64().expect("material index must be a u64") as usize;
            assert!(idx < limit, "global material id {idx} must be < {limit}");
        }
        Value::Null => {}
        other => panic!("unexpected material map node: {other}"),
    }
}

/// Recursively asserts every innermost ring in a rewritten `texture` map:
/// the texture id (first element) is `< limit` (or the ring is `[null]`),
/// and every following element is an inlined `[u, v]` pair.
fn assert_every_texture_ring_valid(v: &Value, limit: usize) {
    match v {
        Value::Object(map) => {
            for val in map.values() {
                assert_every_texture_ring_valid(val, limit);
            }
        }
        Value::Array(items) => {
            let is_ring = !items.is_empty() && matches!(items[0], Value::Number(_) | Value::Null);
            if is_ring {
                if let Value::Number(n) = &items[0] {
                    let t = n.as_u64().expect("global texture id must be a u64") as usize;
                    assert!(t < limit, "global texture id {t} must be < {limit}");
                }
                for uv in &items[1..] {
                    let pair = uv
                        .as_array()
                        .unwrap_or_else(|| panic!("expected inlined [u, v] pair, got {uv}"));
                    assert_eq!(pair.len(), 2, "UV pair must have exactly 2 coordinates");
                    assert!(pair[0].is_number() && pair[1].is_number());
                }
            } else {
                for x in items {
                    assert_every_texture_ring_valid(x, limit);
                }
            }
        }
        _ => {}
    }
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
    // The recipe pins bbox leaves to BYTE_STREAM_SPLIT with dictionary
    // encoding off; this only holds if the recipe's per-column
    // `ColumnPath` for "bbox.xmin" actually matches the physical column's
    // nested path (`["bbox", "xmin"]`), not a single dotted-string part.
    let bbox_xmin_encodings: Vec<Encoding> = bbox_xmin_col.encodings().collect();
    assert!(
        bbox_xmin_encodings.contains(&Encoding::BYTE_STREAM_SPLIT),
        "expected BYTE_STREAM_SPLIT on bbox.xmin, got {bbox_xmin_encodings:?}"
    );
    assert!(
        !bbox_xmin_encodings.contains(&Encoding::RLE_DICTIONARY),
        "bbox.xmin should have dictionary encoding disabled, got {bbox_xmin_encodings:?}"
    );
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

/// Core-profile convert of railway: the main table's `material`/`texture`
/// columns must carry dataset-GLOBAL ids (rewritten by the appearance
/// interner), not the feature-local indices the source CityJSON uses.
/// railway's feature-only appearance sweep resolves 83 materials / 33
/// textures (2 materials + 1 texture are referenced only from
/// geometry-templates, which this encode loop does not visit yet — Task 8).
#[test]
fn railway_core_convert_rewrites_appearance_maps_to_global_ids() {
    let out = tempfile::tempdir().unwrap();
    convert(&ConvertOptions::new(
        fixture("lod3_railway.city.json"),
        out.path().to_path_buf(),
    ))
    .unwrap();

    let file = std::fs::File::open(out.path().join("cityobjects.parquet")).unwrap();
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
    let reader = builder.build().unwrap();

    let mut checked_material = false;
    let mut checked_texture = false;
    for batch in reader {
        let batch = batch.unwrap();
        let material_col: &StringArray = batch
            .column_by_name("material")
            .unwrap()
            .as_any()
            .downcast_ref()
            .unwrap();
        let texture_col: &StringArray = batch
            .column_by_name("texture")
            .unwrap()
            .as_any()
            .downcast_ref()
            .unwrap();
        for row in 0..batch.num_rows() {
            if !material_col.is_null(row) {
                let map: Value = serde_json::from_str(material_col.value(row)).unwrap();
                assert_every_material_index_below(&map, 83);
                checked_material = true;
            }
            if !texture_col.is_null(row) {
                let map: Value = serde_json::from_str(texture_col.value(row)).unwrap();
                assert_every_texture_ring_valid(&map, 33);
                checked_texture = true;
            }
        }
    }
    assert!(
        checked_material,
        "expected at least one row with a material map"
    );
    assert!(
        checked_texture,
        "expected at least one row with a texture map"
    );
}

/// Compatibility profile: railway's feature-only appearance sweep (83
/// materials / 33 textures — see the module doc on
/// `railway_core_convert_rewrites_appearance_maps_to_global_ids`) must land
/// in `materials.parquet`/`textures.parquet`, with `metadata.json`'s
/// `sidecar_files` listing exactly the files actually written.
#[test]
fn railway_compatibility_convert_writes_materials_and_textures_sidecars() {
    let out = tempfile::tempdir().unwrap();
    let mut opts = ConvertOptions::new(fixture("lod3_railway.city.json"), out.path().to_path_buf());
    opts.profile = Profile::Compatibility;
    let report = convert(&opts).unwrap();

    assert_eq!(report.materials_written, 83);
    assert_eq!(report.textures_written, 33);
    assert_eq!(report.templates_written, 0);
    assert!(out.path().join("materials.parquet").exists());
    assert!(out.path().join("textures.parquet").exists());

    let manifest: Value =
        serde_json::from_str(&std::fs::read_to_string(out.path().join("metadata.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["profile"], "compatibility");
    assert_eq!(
        manifest["sidecar_files"],
        serde_json::json!(["materials.parquet", "textures.parquet"])
    );
}

/// Compatibility profile on a dataset with no appearance at all (delft):
/// no sidecar files are written, and the manifest says so.
#[test]
fn delft_compatibility_convert_writes_no_sidecars() {
    let out = tempfile::tempdir().unwrap();
    let mut opts = ConvertOptions::new(fixture("delft.city.jsonl"), out.path().to_path_buf());
    opts.profile = Profile::Compatibility;
    let report = convert(&opts).unwrap();

    assert_eq!(report.materials_written, 0);
    assert_eq!(report.textures_written, 0);
    assert_eq!(report.templates_written, 0);
    assert!(!out.path().join("materials.parquet").exists());
    assert!(!out.path().join("textures.parquet").exists());

    let manifest: Value =
        serde_json::from_str(&std::fs::read_to_string(out.path().join("metadata.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["profile"], "compatibility");
    assert_eq!(manifest["sidecar_files"], serde_json::json!([]));
}
