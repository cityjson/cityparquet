//! FlatCityBuf input (the `fcb` feature), over the published Delft `.fcb`
//! and the Delft CityJSONSeq of the same 3DBAG area.
#![cfg(feature = "fcb")]

use std::path::PathBuf;

use cityparquet::compare::{CompareOptions, compare_datasets};
use cityparquet::export::{ExportOptions, export};
use cityparquet::package::{ConvertOptions, convert};
use cityparquet::source::{Source, SourceFormat};

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

#[test]
fn a_flatcitybuf_file_is_read_feature_by_feature_up_to_its_count() {
    let source = Source::open(&fixture("delft.fcb")).unwrap();
    assert_eq!(source.format(), SourceFormat::FlatCityBuf);
    let seq = Source::open(&fixture("delft.city.jsonl")).unwrap();
    let count = |s: &Source| {
        s.features()
            .unwrap()
            .collect::<cityparquet::Result<Vec<_>>>()
            .unwrap()
            .len()
    };
    // The iterator stops at the header's feature count.
    assert_eq!(count(&source), count(&seq));
}

/// The FlatCityBuf source as this crate reads it, written out as CityJSONSeq
/// (its header line, then one feature per line).
fn write_as_seq(source: &Source, path: &std::path::Path) {
    let mut lines = vec![serde_json::to_string(source.header()).unwrap()];
    for feature in source.features().unwrap() {
        lines.push(serde_json::to_string(&feature.unwrap()).unwrap());
    }
    std::fs::write(path, lines.join("\n") + "\n").unwrap();
}

/// Converting the `.fcb` and exporting the package gives back the model the
/// FlatCityBuf reader hands over; and that model is the Delft CityJSONSeq's,
/// but for one documented difference of the published file: every
/// `BuildingPart` carries an LoD 0 footprint the CityJSONSeq does not.
#[test]
fn delft_from_flatcitybuf_round_trips_and_matches_the_cityjsonseq() {
    let dir = tempfile::tempdir().unwrap();
    let source = Source::open(&fixture("delft.fcb")).unwrap();
    let read = dir.path().join("read.city.jsonl");
    write_as_seq(&source, &read);

    let pkg = dir.path().join("pkg");
    convert(&ConvertOptions::new(fixture("delft.fcb"), pkg.clone())).unwrap();
    let exported = dir.path().join("exported.city.jsonl");
    export(&ExportOptions {
        package_dir: pkg,
        output: exported.clone(),
    })
    .unwrap();
    let round_trip = compare_datasets(&read, &exported, &CompareOptions::default()).unwrap();
    assert!(round_trip.equal, "{:#?}", round_trip.differences);

    // Drop the BuildingParts' LoD 0 footprints, then the FlatCityBuf model is
    // the CityJSONSeq's.
    let text = std::fs::read_to_string(&read).unwrap();
    let mut part_footprints = 0;
    let stripped: Vec<String> = text
        .lines()
        .map(|line| {
            let mut v: serde_json::Value = serde_json::from_str(line).unwrap();
            if let Some(objects) = v.get_mut("CityObjects").and_then(|o| o.as_object_mut()) {
                for object in objects.values_mut() {
                    if object["type"] != "BuildingPart" {
                        continue;
                    }
                    let geometries = object["geometry"].as_array_mut().unwrap();
                    let before = geometries.len();
                    geometries.retain(|g| g["lod"] != "0.0");
                    part_footprints += before - geometries.len();
                }
            }
            v.to_string()
        })
        .collect();
    assert_eq!(part_footprints, 1116, "one footprint per BuildingPart");
    let without = dir.path().join("without_part_lod0.city.jsonl");
    std::fs::write(&without, stripped.join("\n") + "\n").unwrap();
    let report = compare_datasets(
        &fixture("delft.city.jsonl"),
        &without,
        &CompareOptions::default(),
    )
    .unwrap();
    assert!(report.equal, "{:#?}", report.differences);
}
