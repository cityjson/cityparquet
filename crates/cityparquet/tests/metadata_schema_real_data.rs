//! Validates the `city` Parquet footer key-value object -- as written by a
//! real conversion of a real fixture -- against the published JSON Schema
//! (`crates/cityparquet-schema/assets/city.schema.json`, mirroring how
//! GeoParquet ships `geoparquet.org/releases/v1.1.0/schema.json` for its own
//! `geo` object). No inline hand-written CityJSON or footer JSON: the valid
//! instance always comes from `convert()` run on `tests/fixtures/delft.city.jsonl`.

use std::path::PathBuf;

use cityparquet::package::{ConvertOptions, convert};
use jsonschema::Validator;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use serde_json::Value;

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

/// The `city.schema.json` asset lives alongside `cityparquet-schema`'s other
/// spec-as-code artifacts (`crates/cityparquet-schema/assets/`), not under
/// this crate -- read straight off disk (like `fixture()` above) rather than
/// `include_str!`, so an edit to the schema is picked up without a rebuild.
fn city_schema() -> Validator {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../cityparquet-schema/assets/city.schema.json");
    assert!(
        p.exists(),
        "missing crates/cityparquet-schema/assets/city.schema.json"
    );
    let schema: Value =
        serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).expect("schema is valid JSON");
    jsonschema::validator_for(&schema).expect("city.schema.json compiles")
}

/// Convert the real `delft.city.jsonl` fixture and read back the `city` key
/// from `building.parquet`'s footer key-value metadata -- the same real
/// footer other tests (e.g. `convert_real_data.rs`) assert keys on.
fn delft_city_footer() -> Value {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("pkg");
    convert(&ConvertOptions::new(
        fixture("delft.city.jsonl"),
        out.clone(),
    ))
    .expect("convert delft");

    let file = std::fs::File::open(out.join("building.parquet")).unwrap();
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
    let kvs = builder
        .metadata()
        .file_metadata()
        .key_value_metadata()
        .expect("footer key-value metadata present")
        .clone();
    let city_raw = kvs
        .iter()
        .find(|kv| kv.key == "city")
        .and_then(|kv| kv.value.clone())
        .expect("city key present in footer");
    serde_json::from_str(&city_raw).expect("city value is JSON")
}

/// A real footer -- version, source_format, crs, primary_column, per-LoD
/// `columns` (each with `orientation_3d`), and `attributes` -- must validate
/// clean against the published schema.
#[test]
fn real_delft_footer_validates_against_city_schema() {
    let validator = city_schema();
    let instance = delft_city_footer();

    let errors: Vec<String> = validator
        .iter_errors(&instance)
        .map(|e| format!("{e} at {}", e.instance_path))
        .collect();
    assert!(
        errors.is_empty(),
        "real delft `city` footer violates city.schema.json:\n{}\n\ninstance:\n{}",
        errors.join("\n"),
        serde_json::to_string_pretty(&instance).unwrap()
    );

    // Sanity: this is a non-trivial object-table footer, not a degenerate
    // sidecar-shaped one -- otherwise the schema's `columns`/`orientation_3d`
    // branches above would go untested by the "valid" case.
    assert_eq!(instance["version"], "0.1.0-draft");
    assert!(
        instance["columns"]
            .as_array()
            .is_some_and(|c| !c.is_empty())
    );
    assert!(
        instance["columns"][0]["orientation_3d"].is_string(),
        "a real column entry must carry orientation_3d"
    );
}

/// A `city` object missing the required `version` field must be rejected --
/// proving the schema has teeth, not that it accepts everything.
#[test]
fn missing_version_is_rejected() {
    let validator = city_schema();
    let mut instance = delft_city_footer();
    instance
        .as_object_mut()
        .unwrap()
        .remove("version")
        .expect("real footer has a version field to remove");

    let errors: Vec<Value> = validator
        .iter_errors(&instance)
        .map(|e| Value::String(e.to_string()))
        .collect();
    assert!(
        !errors.is_empty(),
        "a `city` object with no `version` must be invalid"
    );
}

/// A `columns` entry missing the now-required `orientation_3d` must be
/// rejected.
#[test]
fn column_entry_missing_orientation_3d_is_rejected() {
    let validator = city_schema();
    let mut instance = delft_city_footer();
    instance["columns"][0]
        .as_object_mut()
        .unwrap()
        .remove("orientation_3d")
        .expect("real column entry has orientation_3d to remove");

    assert!(
        !validator.is_valid(&instance),
        "a columns entry with no orientation_3d must be invalid"
    );
}

/// `columns` present without `primary_column` must be rejected -- the
/// `if columns then primary_column` rule (05-metadata.mdx: "Required whenever
/// the table has any geometry column").
#[test]
fn columns_without_primary_column_is_rejected() {
    let validator = city_schema();
    let mut instance = delft_city_footer();
    instance
        .as_object_mut()
        .unwrap()
        .remove("primary_column")
        .expect("real footer has a primary_column field to remove");

    assert!(
        !validator.is_valid(&instance),
        "columns present with no primary_column must be invalid"
    );
}
