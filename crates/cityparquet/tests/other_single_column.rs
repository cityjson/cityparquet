//! `other` is the single escape hatch: one column, one reader rule — every
//! entry is restored into the object's `attributes` on export.

use cityparquet::export::{ExportOptions, export};
use cityparquet::package::{ConvertOptions, convert};

/// A source attribute whose name collides with a reserved column survives a
/// full round trip, back inside `attributes`.
#[test]
fn colliding_attribute_round_trips_through_other() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("collide.city.jsonl");
    // Real Delft objects with one reserved-named attribute injected.
    let text = std::fs::read_to_string("../../tests/fixtures/delft.city.jsonl").unwrap();
    let mut lines = text.lines();
    let header = lines.next().unwrap().to_string();
    let mut feature: serde_json::Value = serde_json::from_str(lines.next().unwrap()).unwrap();
    for (_id, obj) in feature["CityObjects"].as_object_mut().unwrap() {
        obj["attributes"]["bbox"] = serde_json::json!("collides-with-reserved");
    }
    std::fs::write(&src, format!("{header}\n{feature}\n")).unwrap();

    let pkg = dir.path().join("pkg");
    let out = dir.path().join("out.city.jsonl");
    convert(&ConvertOptions::new(src.clone(), pkg.clone())).unwrap();
    export(&ExportOptions {
        package_dir: pkg.clone(),
        output: out.clone(),
    })
    .unwrap();

    let exported = std::fs::read_to_string(&out).unwrap();
    let line = exported.lines().nth(1).expect("one feature line");
    let feature: serde_json::Value = serde_json::from_str(line).unwrap();
    for (id, obj) in feature["CityObjects"].as_object().unwrap() {
        assert_eq!(
            obj["attributes"]["bbox"], "collides-with-reserved",
            "object {id} must recover its diverted attribute"
        );
    }
}

/// The table carries no `other_attributes` column at all.
#[test]
fn no_other_attributes_column_is_written() {
    let dir = tempfile::tempdir().unwrap();
    let pkg = dir.path().join("pkg");
    convert(&ConvertOptions::new(
        std::path::PathBuf::from("../../tests/fixtures/delft.city.jsonl"),
        pkg.clone(),
    ))
    .unwrap();

    let file = std::fs::File::open(pkg.join("building.parquet")).unwrap();
    let reader = parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder::try_new(file)
        .unwrap();
    let names: Vec<&str> = reader
        .schema()
        .fields()
        .iter()
        .map(|f| f.name().as_str())
        .collect();
    assert!(
        !names.contains(&"other_attributes"),
        "other_attributes must be gone, schema: {names:?}"
    );
    assert!(names.contains(&"other"), "other must remain: {names:?}");
}
