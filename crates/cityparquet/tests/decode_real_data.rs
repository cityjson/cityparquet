//! RED (M3 task 5): decode — RecordBatch -> `DecodedObject` (`cjseq`-model
//! objects), exercised against real converted delft/railway packages.

use std::collections::HashMap;
use std::path::PathBuf;

use cityparquet::decode::decode_batch;
use cityparquet::package::{ConvertOptions, convert};
use cityparquet::reader::{CityParquetReaderBuilder, CityParquetRecordBatchReader};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

/// Convert `input` into a fresh tempdir and decode every row of
/// `cityobjects.parquet` back into `DecodedObject`s.
fn convert_and_decode(input: &str) -> Vec<cityparquet::decode::DecodedObject> {
    let out = tempfile::tempdir().unwrap();
    let report = convert(&ConvertOptions::new(
        fixture(input),
        out.path().to_path_buf(),
    ))
    .unwrap();
    eprintln!("{input}: converted {} objects", report.object_count);

    let file = std::fs::File::open(out.path().join("cityobjects.parquet")).unwrap();
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
    let meta = builder.cityparquet_metadata().unwrap();
    let schema = builder.cityparquet_arrow_schema().unwrap();
    let parquet_reader = builder.build().unwrap();
    let reader = CityParquetRecordBatchReader::new(parquet_reader, schema);

    let mut all = Vec::new();
    for batch in reader {
        let batch = batch.unwrap();
        all.extend(decode_batch(&batch, &meta).unwrap());
    }
    all
}

/// Recounted directly from `tests/fixtures/delft.city.jsonl` with python3:
/// `Counter(co['type'] for line in file for co in line['CityObjects'].values())`
/// over every non-header line -> `{'BuildingPart': 1116, 'Building': 1115}`,
/// 2231 objects total (matches `convert_real_data.rs`'s pinned object count).
#[test]
fn delft_decodes_every_object_with_correct_types_and_attributes() {
    let objects = convert_and_decode("delft.city.jsonl");
    assert_eq!(objects.len(), 2231);

    let mut type_counts: HashMap<String, usize> = HashMap::new();
    let mut total_geometries = 0usize;
    for obj in &objects {
        assert!(!obj.id.is_empty(), "every id must be non-empty");
        *type_counts.entry(obj.object.thetype.clone()).or_default() += 1;

        for (lod, _decoded, _props) in &obj.geometries {
            let lod = lod.as_ref().unwrap_or_else(|| {
                panic!(
                    "delft has per-LoD columns, so no unsuffixed geometry expected on object {}",
                    obj.id
                )
            });
            let s = lod.to_string();
            assert!(
                ["0", "1.2", "1.3", "2.2"].contains(&s.as_str()),
                "unexpected LoD {s} on object {}",
                obj.id
            );
        }
        total_geometries += obj.geometries.len();
    }
    assert_eq!(
        type_counts,
        HashMap::from([
            ("Building".to_string(), 1115),
            ("BuildingPart".to_string(), 1116),
        ]),
        "object_type multiset must match the recount above"
    );

    // Total decoded geometries must equal total non-null geometry cells
    // across every geometry_lod* column, counted independently from the
    // written Parquet file.
    let out = tempfile::tempdir().unwrap();
    convert(&ConvertOptions::new(
        fixture("delft.city.jsonl"),
        out.path().to_path_buf(),
    ))
    .unwrap();
    let file = std::fs::File::open(out.path().join("cityobjects.parquet")).unwrap();
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
    let schema = builder.cityparquet_arrow_schema().unwrap();
    let geom_cols: Vec<String> = schema
        .fields()
        .iter()
        .filter_map(|f| {
            let name = f.name();
            if name.starts_with("geometry_") && !name.starts_with("geometry_properties_") {
                Some(name.clone())
            } else {
                None
            }
        })
        .collect();
    assert!(
        !geom_cols.is_empty(),
        "delft must have geometry_lod* columns"
    );
    let reader = builder.build().unwrap();
    let mut non_null_cells = 0usize;
    for batch in reader {
        let batch = batch.unwrap();
        for name in &geom_cols {
            let col = batch.column_by_name(name).unwrap();
            non_null_cells += col.len() - col.null_count();
        }
    }
    assert_eq!(
        total_geometries, non_null_cells,
        "decoded geometry count must equal non-null geometry cells across all geometry_lod* columns"
    );

    // A known Date-inferred attribute column ("begingeldigheid", "YYYY-MM-DD"
    // shaped in the source fixture) survives decode as a "%Y-%m-%d" JSON
    // string on at least one object.
    let mut date_checked = 0usize;
    let mut int_checked = 0usize;
    let mut float_checked = 0usize;
    for obj in &objects {
        let Some(attrs) = obj.object.attributes.as_ref().and_then(|v| v.as_object()) else {
            continue;
        };
        if let Some(v) = attrs.get("begingeldigheid") {
            let s = v.as_str().unwrap_or_else(|| {
                panic!("begingeldigheid must decode as a JSON string, got {v:?}")
            });
            assert_eq!(s.len(), 10, "expected YYYY-MM-DD, got {s:?}");
            assert_eq!(&s[4..5], "-");
            assert_eq!(&s[7..8], "-");
            date_checked += 1;
        }
        if let Some(v) = attrs.get("oorspronkelijkbouwjaar") {
            assert!(
                v.is_i64(),
                "oorspronkelijkbouwjaar must decode as a JSON integer, got {v:?}"
            );
            int_checked += 1;
        }
        if let Some(v) = attrs.get("b3_h_dak_max") {
            assert!(
                v.is_f64() || v.is_i64(),
                "b3_h_dak_max must decode as a JSON number, got {v:?}"
            );
            float_checked += 1;
        }
    }
    assert!(date_checked > 0, "expected >0 objects with begingeldigheid");
    assert!(
        int_checked > 0,
        "expected >0 objects with oorspronkelijkbouwjaar"
    );
    assert!(float_checked > 0, "expected >0 objects with b3_h_dak_max");
}

/// Recounted from `tests/fixtures/lod3_railway.city.json` with python3: 121
/// `CityObjects` total (matches `convert_real_data.rs`'s pinned count);
/// exactly 15 objects carry a `GeometryInstance` geometry
/// (`sum(1 for co in data['CityObjects'].values() if any(g['type'] ==
/// 'GeometryInstance' for g in co.get('geometry', [])))` == 15, all
/// `SolitaryVegetationObject`s); exactly 8 STORED geometries carry a
/// `semantics` block (replaying the writer's binding rule in python — first
/// geometry per (object, LoD) slot kept, `GeometryInstance` and lod-less
/// entries excluded — also yields 8, so the raw count and the stored count
/// coincide for this fixture).
#[test]
fn railway_decodes_templates_and_semantics() {
    let objects = convert_and_decode("lod3_railway.city.json");
    assert_eq!(objects.len(), 121);

    let template_count = objects.iter().filter(|o| o.template.is_some()).count();
    assert_eq!(
        template_count, 15,
        "expected exactly 15 objects with a template (the recount above)"
    );

    let mut semantics_found = 0usize;
    for obj in &objects {
        for (_lod, _decoded, props) in &obj.geometries {
            if let Some(props) = props
                && props.get("semantics").is_some()
            {
                semantics_found += 1;
            }
        }
    }
    assert_eq!(
        semantics_found, 8,
        "expected exactly 8 geometry_properties entries carrying semantics (the recount above)"
    );
}

/// The zero-analysis-geometry case that G3 preserves: a dataset whose only
/// geometry is `GeometryInstance`s (plus objects with no geometry) has no LoD
/// to suffix, so it falls back to the un-suffixed `geometry` column — which is
/// entirely null (instances route to `template`, not a geometry column). This
/// is the ONLY way `lods` is empty now: a lod-less NON-instance geometry is
/// rejected at scan (§9, CityJSON 2.0 §3), covered in `scan_real_data.rs`.
///
/// Derived from `lod3_railway.city.json` by removing every non-instance
/// geometry, keeping its 15 `GeometryInstance`s. Decode must read the all-null
/// un-suffixed column without error and still route the instances to template.
#[test]
fn instances_only_dataset_uses_the_unsuffixed_geometry_column() {
    let text = std::fs::read_to_string(fixture("lod3_railway.city.json")).unwrap();
    let mut doc: serde_json::Value = serde_json::from_str(&text).unwrap();
    let mut kept_instances = 0usize;
    for (_, co) in doc["CityObjects"].as_object_mut().unwrap() {
        let Some(geoms) = co.get_mut("geometry").and_then(|g| g.as_array_mut()) else {
            continue;
        };
        geoms.retain(|g| {
            let is_instance = g.get("type").and_then(|t| t.as_str()) == Some("GeometryInstance");
            if is_instance {
                kept_instances += 1;
            }
            is_instance
        });
    }
    assert_eq!(
        kept_instances, 15,
        "railway must carry 15 GeometryInstances to keep"
    );
    let src_dir = tempfile::tempdir().unwrap();
    let path = src_dir.path().join("railway_instances_only.city.json");
    std::fs::write(&path, serde_json::to_string(&doc).unwrap()).unwrap();

    let out = tempfile::tempdir().unwrap();
    let report = convert(&ConvertOptions::new(path, out.path().to_path_buf())).unwrap();
    assert_eq!(report.object_count, 121);

    let file = std::fs::File::open(out.path().join("cityobjects.parquet")).unwrap();
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
    let meta = builder.cityparquet_metadata().unwrap();
    let schema = builder.cityparquet_arrow_schema().unwrap();

    // No LoD-bearing geometry: the un-suffixed geometry column, no per-LoD ones.
    assert!(
        schema.field_with_name("geometry").is_ok(),
        "a zero-analysis-geometry dataset uses the unsuffixed geometry column"
    );
    assert!(
        !schema
            .fields()
            .iter()
            .any(|f| f.name().starts_with("geometry_lod")),
        "a zero-analysis-geometry dataset must have no geometry_lod* columns"
    );

    let parquet_reader = builder.build().unwrap();
    let reader = CityParquetRecordBatchReader::new(parquet_reader, schema);

    let mut objects = Vec::new();
    let mut non_null_cells = 0usize;
    for batch in reader {
        let batch = batch.unwrap();
        let col = batch.column_by_name("geometry").unwrap();
        non_null_cells += col.len() - col.null_count();
        objects.extend(decode_batch(&batch, &meta).unwrap());
    }
    assert_eq!(objects.len(), 121);
    assert_eq!(
        non_null_cells, 0,
        "instances produce no analysis geometry, so the unsuffixed column is all null"
    );
    let total_geometries: usize = objects.iter().map(|o| o.geometries.len()).sum();
    assert_eq!(total_geometries, 0, "no non-instance geometry survives");

    // The 15 GeometryInstances still route to template.
    let template_count = objects.iter().filter(|o| o.template.is_some()).count();
    assert_eq!(template_count, 15);
}
