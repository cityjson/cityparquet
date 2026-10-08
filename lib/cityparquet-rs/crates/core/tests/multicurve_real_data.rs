//! `geometry_properties_lod*.type` is the CityGML CM geometry type (spec 03
//! "Geometry-type mapping"): a CityJSON `MultiLineString` is stored as
//! `MultiCurve`, and exported as `MultiLineString` again.
//!
//! None of the fixtures carries a MultiLineString, so this derives one from
//! real data: Delft's Building footprints (LoD 0 MultiSurfaces) become their
//! outlines — each exterior ring as a closed linestring — in a copy of
//! `delft.city.jsonl`.

mod support;

use std::path::Path;

use arrow_array::Array;
use arrow_array::cast::AsArray;
use cityparquet::compare::{CompareOptions, compare_datasets};
use cityparquet::export::{ExportOptions, export};
use cityparquet::package::{ConvertOptions, convert};
use cityparquet::validate::validate_package;
use serde_json::Value;

use support::{FileContent, fixture};

fn delft_with_outlines(path: &Path) {
    let text = std::fs::read_to_string(fixture("delft.city.jsonl")).unwrap();
    let lines: Vec<String> = text
        .lines()
        .map(|line| {
            let mut v: Value = serde_json::from_str(line).unwrap();
            if let Some(objects) = v.get_mut("CityObjects").and_then(Value::as_object_mut) {
                for object in objects.values_mut() {
                    for g in object["geometry"].as_array_mut().into_iter().flatten() {
                        if g["type"] != "MultiSurface" {
                            continue;
                        }
                        let outlines: Vec<Value> = g["boundaries"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|surface| {
                                let mut ring = surface[0].as_array().unwrap().clone();
                                ring.push(ring[0].clone());
                                Value::Array(ring)
                            })
                            .collect();
                        g["type"] = "MultiLineString".into();
                        g["boundaries"] = Value::Array(outlines);
                    }
                }
            }
            v.to_string()
        })
        .collect();
    std::fs::write(path, lines.join("\n") + "\n").unwrap();
}

#[test]
fn a_multilinestring_is_stored_as_multicurve_and_exported_back() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("outlines.city.jsonl");
    delft_with_outlines(&source);
    let pkg = dir.path().join("pkg");
    convert(&ConvertOptions::new(source.clone(), pkg.clone())).unwrap();

    let content = FileContent::read(&pkg.join("building.parquet"));
    let mut types = std::collections::BTreeSet::new();
    for batch in &content.batches {
        let props = batch
            .column_by_name("geometry_properties_lod0_0")
            .unwrap()
            .as_struct();
        let t = props.column_by_name("type").unwrap().as_string::<i32>();
        types.extend(
            (0..t.len())
                .filter(|&i| !props.is_null(i))
                .map(|i| t.value(i).to_string()),
        );
    }
    assert_eq!(types.into_iter().collect::<Vec<_>>(), vec!["MultiCurve"]);
    let report = validate_package(&pkg).unwrap();
    assert!(report.violations.is_empty(), "{report}");

    let exported = dir.path().join("exported.city.jsonl");
    export(&ExportOptions {
        package_dir: pkg,
        output: exported.clone(),
    })
    .unwrap();
    let equal = compare_datasets(&source, &exported, &CompareOptions::default()).unwrap();
    assert!(equal.equal, "{:#?}", equal.differences);
}
