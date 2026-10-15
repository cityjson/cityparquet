//! 3DCityDB v5 input (the `citydb` feature), over the databases
//! `lib/citygml-rs/crates/citydb/tests/setup-db.sh` loads. Each test reads
//! `CITYDB_TEST_URL` and skips without it; with `CITYDB_TEST_REQUIRED` set,
//! a missing URL fails instead.
#![cfg(feature = "citydb")]

use std::path::{Path, PathBuf};

use citygml_citydb::Config;
use cityparquet::compare::{CompareOptions, compare_datasets};
use cityparquet::export::{ExportOptions, export};
use cityparquet::package::{ConvertOptions, convert_source};
use cityparquet::source::{Source, SourceFormat};
use cityparquet::validate::validate_package;

/// The database `tests/setup-db.sh` loaded `dataset` into, or `None` when
/// `CITYDB_TEST_URL` is not set.
fn database(dataset: &str) -> Option<Config> {
    let Ok(base) = std::env::var("CITYDB_TEST_URL") else {
        assert!(
            std::env::var_os("CITYDB_TEST_REQUIRED").is_none(),
            "CITYDB_TEST_REQUIRED is set but CITYDB_TEST_URL is not"
        );
        eprintln!("CITYDB_TEST_URL is not set: skipping");
        return None;
    };
    Some(Config::new(format!(
        "{}/{dataset}",
        base.trim_end_matches('/')
    )))
}

/// The source as this crate reads it, written out as CityJSONSeq.
fn write_as_seq(source: &Source, path: &Path) {
    let mut lines = vec![serde_json::to_string(source.header()).unwrap()];
    for feature in source.features().unwrap() {
        lines.push(serde_json::to_string(&feature.unwrap()).unwrap());
    }
    std::fs::write(path, lines.join("\n") + "\n").unwrap();
}

/// A database's header: the reference system its geometries are stored in,
/// and a transform whose step is the reference system's own (millimetres
/// for a projected one) from the lower corner of the database's extent.
#[test]
fn a_database_declares_its_reference_system_and_extent() {
    let Some(config) = database("delft") else {
        return;
    };
    let source = Source::open_citydb(&config).unwrap();
    assert_eq!(source.format(), SourceFormat::CityDb);
    let header = source.header();
    let metadata = header.metadata.as_ref().unwrap();
    assert_eq!(
        metadata.reference_system.as_ref().unwrap().to_url(),
        "https://www.opengis.net/def/crs/EPSG/0/7415"
    );
    let extent = metadata.geographical_extent.unwrap();
    assert_eq!(
        header.transform.translate,
        [extent[0], extent[1], extent[2]]
    );
    assert_eq!(header.transform.scale, [0.001; 3]);
}

/// A PLATEAU database is in EPSG:6697, latitude and longitude in degrees:
/// the step follows the units, as for a CityGML document.
#[test]
fn a_geographic_database_is_quantised_at_a_degree_step() {
    let Some(config) = database("plateau_bldg") else {
        return;
    };
    let source = Source::open_citydb(&config).unwrap();
    assert_eq!(
        source.header().transform.scale,
        [
            cityparquet_schema::crs::NANO_DEGREE,
            cityparquet_schema::crs::NANO_DEGREE,
            0.001
        ]
    );
}

/// Converting a database and exporting the package gives back the model
/// the database reader hands over, and the package is valid.
fn round_trips(dataset: &str) {
    let Some(config) = database(dataset) else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let source = Source::open_citydb(&config).unwrap();
    let read = dir.path().join("read.city.jsonl");
    write_as_seq(&source, &read);

    let pkg = dir.path().join("pkg");
    let report =
        convert_source(&source, &ConvertOptions::new(PathBuf::new(), pkg.clone())).unwrap();
    assert!(report.object_count > 0);
    let validation = validate_package(&pkg).unwrap();
    assert_eq!(
        validation.errors().count(),
        0,
        "{:#?}",
        validation.violations
    );

    let exported = dir.path().join("exported.city.jsonl");
    export(&ExportOptions::new(pkg, exported.clone())).unwrap();
    let round_trip = compare_datasets(&read, &exported, &CompareOptions::default()).unwrap();
    assert!(round_trip.equal, "{dataset}: {:#?}", round_trip.differences);
}

#[test]
fn delft_round_trips() {
    round_trips("delft");
}

#[test]
fn plateau_buildings_round_trip() {
    round_trips("plateau_bldg");
}

#[test]
fn the_railway_scene_round_trips() {
    round_trips("railway_scene");
}

/// What the database reader leaves out reaches the conversion report: the
/// PLATEAU buildings' textures have no images in the database.
#[test]
fn what_the_database_reader_leaves_out_is_reported() {
    let Some(config) = database("plateau_bldg") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let source = Source::open_citydb(&config).unwrap();
    let report = convert_source(
        &source,
        &ConvertOptions::new(PathBuf::new(), dir.path().join("pkg")),
    )
    .unwrap();
    assert!(
        report
            .source_left_out
            .get("app:ParameterizedTexture without an image")
            > Some(&0),
        "{:?}",
        report.source_left_out
    );
}
