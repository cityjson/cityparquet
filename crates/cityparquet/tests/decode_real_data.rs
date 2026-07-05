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
/// `SolitaryVegetationObject`s); 8 geometries (raw, any LoD) carry a
/// `semantics` block.
#[test]
fn railway_decodes_templates_and_semantics() {
    let objects = convert_and_decode("lod3_railway.city.json");
    assert_eq!(objects.len(), 121);

    let template_count = objects.iter().filter(|o| o.template.is_some()).count();
    assert!(
        template_count >= 15,
        "expected at least 15 objects with a template, got {template_count}"
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
    assert!(
        semantics_found > 0,
        "expected >0 geometry_properties entries carrying semantics"
    );
}
