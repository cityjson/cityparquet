//! Extension namespaces end to end (spec "Extensions"): every name a
//! CityJSON Extension adds — a `+` attribute, a `+` semantic surface type —
//! is stored with the extension's namespace as a `<namespace>_` prefix, the
//! footer's `city.extensions` declares that namespace, and export restores
//! CityJSON's `+` marker and `extensions` member so the round trip holds.
//!
//! The fixtures are hand-crafted, on-disk CityJSONSeq files under
//! `tests/data/` (this repository's testing discipline forbids inline
//! CityJSON in a `.rs` test), structurally mirroring the smallest committed
//! ones (`extension_module_resolves_to_core.city.jsonl`): one header
//! declaring the `Energy` extension, one `CityJSONFeature` with one box
//! `Solid` building.

use std::path::{Path, PathBuf};

use arrow_array::{Array, Float64Array};
use cityparquet::CityParquetError;
use cityparquet::compare::{CompareOptions, compare_datasets};
use cityparquet::export::{ExportOptions, export};
use cityparquet::package::{ConvertOptions, convert};
use cityparquet::reader::{CityParquetReaderBuilder, CityParquetRecordBatchReader};
use cityparquet::source::Source;
use cityparquet_schema::CityMetadata;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use serde_json::json;

fn data_fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(name);
    assert!(p.exists(), "missing committed fixture {name} in tests/data");
    p
}

/// The decoded objects and footer metadata of one object table.
fn read_table(path: &Path) -> (CityMetadata, Vec<cityparquet::decode::DecodedObject>) {
    let file = std::fs::File::open(path).unwrap();
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
    let meta = builder.cityparquet_metadata().unwrap();
    let schema = builder.cityparquet_arrow_schema().unwrap();
    let reader = CityParquetRecordBatchReader::new(builder.build().unwrap(), schema);
    let mut objects = Vec::new();
    for batch in reader {
        objects.extend(cityparquet::decode::decode_batch(&batch.unwrap(), &meta).unwrap());
    }
    (meta, objects)
}

/// The surface types of every `geometry_properties` cell of `objects`.
fn surface_types(objects: &[cityparquet::decode::DecodedObject]) -> Vec<String> {
    objects
        .iter()
        .flat_map(|o| &o.geometries)
        .filter_map(|(_, _, props)| props.as_ref()?.get("surfaces")?.as_array().cloned())
        .flatten()
        .map(|s| s["type"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn extension_names_carry_the_declared_namespace_prefix() {
    let out = tempfile::tempdir().unwrap();
    convert(&ConvertOptions::new(
        data_fixture("extension_energy.city.jsonl"),
        out.path().to_path_buf(),
    ))
    .expect("a source declaring one extension converts");

    let building = out.path().join("building.parquet");
    let file = std::fs::File::open(&building).unwrap();
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
    let field = builder
        .schema()
        .field_with_name("energy_heatCapacity")
        .expect("the +heatCapacity attribute is stored as energy_heatCapacity")
        .clone();
    assert_eq!(
        field.metadata().get("cityparquet:role").map(String::as_str),
        Some("extension")
    );
    assert!(
        builder
            .schema()
            .field_with_name("yearOfConstruction")
            .is_ok()
    );
    for absent in ["+heatCapacity", "heatCapacity"] {
        assert!(
            builder.schema().field_with_name(absent).is_err(),
            "no column may be named {absent}"
        );
    }
    let batch = builder.build().unwrap().next().unwrap().unwrap();
    let values = batch
        .column_by_name("energy_heatCapacity")
        .unwrap()
        .as_any()
        .downcast_ref::<Float64Array>()
        .unwrap()
        .clone();
    assert_eq!(values.value(0), 250000.5);

    let (meta, objects) = read_table(&building);
    assert_eq!(
        serde_json::to_value(meta.extensions.expect("city.extensions is declared")).unwrap(),
        json!({"energy": {
            "name": "Energy",
            "url": "https://example.org/energy.ext.json",
            "version": "3.0"
        }})
    );
    assert!(meta.attributes.contains(&"energy_heatCapacity".to_string()));
    assert_eq!(
        surface_types(&objects),
        [
            "GroundSurface",
            "RoofSurface",
            "WallSurface",
            "energy_PartyWallSurface"
        ],
        "an extension surface type carries the namespace prefix; core types stay bare"
    );
}

#[test]
fn export_restores_the_plus_marker_and_the_extensions_member() {
    let source_path = data_fixture("extension_energy.city.jsonl");
    let out = tempfile::tempdir().unwrap();
    convert(&ConvertOptions::new(
        source_path.clone(),
        out.path().to_path_buf(),
    ))
    .unwrap();

    let exported = out.path().join("export.city.jsonl");
    export(&ExportOptions {
        package_dir: out.path().to_path_buf(),
        output: exported.clone(),
    })
    .unwrap();

    let source = Source::open(&exported).unwrap();
    assert_eq!(
        source.header().extensions,
        Some(json!({"Energy": {"url": "https://example.org/energy.ext.json", "version": "3.0"}})),
        "the CityJSON extensions member is rebuilt from city.extensions"
    );
    let feature = source.features().unwrap().next().unwrap().unwrap();
    let co = &feature.city_objects["ENERGY_BUILDING_1"];
    let attributes = co.attributes.as_ref().unwrap();
    assert_eq!(attributes["+heatCapacity"], json!(250000.5));
    assert!(attributes.get("energy_heatCapacity").is_none());
    let geometry = &co.geometry.as_ref().unwrap()[0];
    let surfaces: Vec<&str> = geometry.semantics.as_ref().unwrap()["surfaces"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["type"].as_str().unwrap())
        .collect();
    assert_eq!(
        surfaces,
        [
            "GroundSurface",
            "RoofSurface",
            "WallSurface",
            "+PartyWallSurface"
        ]
    );

    let report = compare_datasets(&source_path, &exported, &CompareOptions::default()).unwrap();
    assert!(
        report.equal,
        "the round trip must be semantically lossless: {:#?}",
        report.differences
    );
}

/// CityJSON requires an extension to be declared before its `+` names are
/// used; a source that uses one without declaring any extension cannot be
/// attributed to a namespace and is rejected.
#[test]
fn a_plus_name_in_a_source_declaring_no_extension_is_rejected() {
    let out = tempfile::tempdir().unwrap();
    let err = convert(&ConvertOptions::new(
        data_fixture("extension_undeclared.city.jsonl"),
        out.path().to_path_buf(),
    ))
    .expect_err("an undeclared extension name must reject the conversion");
    assert!(matches!(err, CityParquetError::Schema(_)), "{err:?}");
    let msg = err.to_string();
    assert!(msg.contains("+heatCapacity"), "{msg}");
    assert!(msg.contains("declare"), "{msg}");
    assert!(!out.path().join("metadata.json").exists());
}

/// A core attribute that begins with a declared namespace prefix could not
/// be told apart from an extension attribute on export, so it is rejected.
#[test]
fn a_core_attribute_with_a_declared_namespace_prefix_is_rejected() {
    let out = tempfile::tempdir().unwrap();
    let err = convert(&ConvertOptions::new(
        data_fixture("extension_prefix_collision.city.jsonl"),
        out.path().to_path_buf(),
    ))
    .expect_err("a core attribute carrying the energy_ prefix must reject the conversion");
    assert!(matches!(err, CityParquetError::Schema(_)), "{err:?}");
    assert!(err.to_string().contains("energy_label"), "{err}");
    assert!(!out.path().join("metadata.json").exists());
}
