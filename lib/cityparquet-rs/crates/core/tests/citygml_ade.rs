//! A CityGML ADE survives CityGML → package → CityJSONSeq → package.
//!
//! `plateau_urf_fragment.gml` holds three objects of the i-UR `urf` ADE's
//! `FirePreventionDistrict` class, each with a `uro` data-quality property:
//! the classes go to the ADE's own file, `urf.parquet`, and both ADEs are
//! declared. `plateau_yokohama_bldg_fragment.gml` holds buildings with `uro`
//! properties: each becomes a `uro_`-prefixed column of `building.parquet`.
//! Every export compares equal with its source, and the urf export, which
//! declares two extensions, converts again by the names its declarations
//! list. PLATEAU's `ubld` tile, read beside the i-UR schemas as the PLATEAU
//! archive ships them, holds a `uro:UndergroundBuilding`, which its schema
//! substitutes for a building: it goes to `building.parquet`. Read the same
//! way, the `urf` tile's values take the types the schema declares.

use std::path::{Path, PathBuf};

use arrow_array::cast::AsArray;
use arrow_array::types::Int32Type;
use arrow_schema::DataType;
use cityparquet::compare::{CompareOptions, compare_datasets};
use cityparquet::export::{ExportOptions, export};
use cityparquet::package::{ConvertOptions, convert};
use cityparquet::reader::CityParquetReaderBuilder;
use cityparquet::validate::validate_package;
use cityparquet_schema::CityMetadata;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

/// A file of the PLATEAU archive, as the `citygml2` crate's tests keep it.
fn plateau(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../citygml-rs/crates/citygml2/tests/data/plateau")
        .join(path)
}

fn data_fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(name)
}

/// Converts `source` into `dir/pkg`, checks the package is conformant,
/// exports it to `dir/out.city.jsonl` and checks the export equals `source`.
fn round_trip(source: &Path, dir: &Path) -> (PathBuf, PathBuf) {
    let pkg = dir.join("pkg");
    convert(&ConvertOptions::new(source.to_path_buf(), pkg.clone())).unwrap();
    let report = validate_package(&pkg).unwrap();
    assert!(report.is_conformant(), "{report}");
    let out = dir.join("out.city.jsonl");
    export(&ExportOptions {
        package_dir: pkg.clone(),
        output: out.clone(),
    })
    .unwrap();
    let cmp = compare_datasets(source, &out, &CompareOptions::default()).unwrap();
    assert!(cmp.equal, "{:#?}", cmp.differences);
    (pkg, out)
}

/// The footer, the column names and the `object_type` values of a table.
fn table(path: &Path) -> (CityMetadata, Vec<String>, Vec<String>) {
    let builder =
        ParquetRecordBatchReaderBuilder::try_new(std::fs::File::open(path).unwrap()).unwrap();
    let meta = builder.cityparquet_metadata().unwrap();
    let columns = builder
        .schema()
        .fields()
        .iter()
        .map(|f| f.name().clone())
        .collect();
    let mut types = Vec::new();
    for batch in builder.build().unwrap() {
        let batch = batch.unwrap();
        let column = batch.column_by_name("object_type").unwrap();
        let dict = column.as_dictionary::<Int32Type>();
        let values = dict.values().as_string::<i32>();
        types.extend(
            dict.keys()
                .iter()
                .map(|k| values.value(k.unwrap() as usize).to_string()),
        );
    }
    (meta, columns, types)
}

#[test]
fn ade_classes_go_to_the_ades_own_file_and_convert_again() {
    let dir = tempfile::tempdir().unwrap();
    let (pkg, out) = round_trip(&data_fixture("plateau_urf_fragment.gml"), dir.path());

    let (meta, _, types) = table(&pkg.join("urf.parquet"));
    assert_eq!(types, ["urf_FirePreventionDistrict"; 3]);
    let extensions = meta.extensions.expect("the ADEs are declared");
    assert_eq!(extensions.keys().collect::<Vec<_>>(), ["urf", "uro"]);
    assert_eq!(
        extensions["urf"].xmlns.as_deref(),
        Some("https://www.geospatial.jp/iur/urf/3.2")
    );

    let again = dir.path().join("again");
    convert(&ConvertOptions::new(out, again.clone())).unwrap();
    assert_eq!(
        table(&again.join("urf.parquet")).2,
        ["urf_FirePreventionDistrict"; 3]
    );
}

#[test]
fn ade_properties_of_a_core_class_become_prefixed_columns() {
    let dir = tempfile::tempdir().unwrap();
    let (pkg, _) = round_trip(
        &data_fixture("plateau_yokohama_bldg_fragment.gml"),
        dir.path(),
    );
    let (meta, columns, _) = table(&pkg.join("building.parquet"));
    assert!(
        columns.iter().any(|c| c == "uro_buildingDetailAttribute"),
        "{columns:?}"
    );
    assert!(meta.extensions.unwrap().contains_key("uro"));
}

#[test]
fn an_ade_class_specialising_a_building_goes_to_the_building_file() {
    let dir = tempfile::tempdir().unwrap();
    let (pkg, out) = round_trip(&plateau("udx/ubld/53394611_ubld_6697_op.gml"), dir.path());
    assert!(!pkg.join("uro.parquet").exists());
    let (_, _, types) = table(&pkg.join("building.parquet"));
    assert!(
        types.iter().any(|t| t == "uro_UndergroundBuilding"),
        "{types:?}"
    );

    let again = dir.path().join("again");
    convert(&ConvertOptions::new(out, again.clone())).unwrap();
    assert!(!again.join("uro.parquet").exists());
    assert!(
        table(&again.join("building.parquet"))
            .2
            .iter()
            .any(|t| t == "uro_UndergroundBuilding")
    );
}

#[test]
fn ade_values_take_the_type_their_schema_declares() {
    let dir = tempfile::tempdir().unwrap();
    let (pkg, _) = round_trip(&plateau("udx/urf/533946_urf_6697_08_op.gml"), dir.path());
    let builder = ParquetRecordBatchReaderBuilder::try_new(
        std::fs::File::open(pkg.join("urf.parquet")).unwrap(),
    )
    .unwrap();
    let schema = builder.schema();
    let ty = |name: &str| schema.field_with_name(name).unwrap().data_type().clone();
    // An `xs:integer` and a `gml:MeasureType` in the urf schema.
    assert_eq!(ty("floorAreaRate"), DataType::Int64);
    assert_eq!(ty("minimumSiteArea"), DataType::Float64);
}
