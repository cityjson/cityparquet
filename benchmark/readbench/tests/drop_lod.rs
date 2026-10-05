//! `drop_lods`: removing every geometry of a given LoD from a CityJSONSeq
//! feature, on the real 3DBAG-derived `delft.city.jsonl` fixture (LoD 0, 1.2,
//! 1.3 and 2.2 per building part).
//!
//! The 3DBAG scaling slices are cut without LoD 1.2 because CityGML 2.0 has
//! integer LoDs only: `citygml-tools from-cityjson` keeps one LoD-1 solid
//! (the 1.3 one) and drops the other, so a slice with both would give the
//! CityGML artefact less geometry than the other four formats.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use cityparquet_readbench::lod::drop_lods;
use cityparquet_readbench::scaling::write_scaling_slices;
use serde_json::Value;

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../lib/cityparquet-rs/tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

/// The fixture's header line and its feature lines.
fn delft() -> (String, Vec<String>) {
    let text = std::fs::read_to_string(fixture("delft.city.jsonl")).unwrap();
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let header = lines.next().unwrap().to_string();
    (header, lines.map(str::to_string).collect())
}

fn drop_12(line: &str) -> String {
    drop_lods(line, &["1.2".to_string()]).unwrap()
}

fn lod_of(geometry: &Value) -> String {
    match &geometry["lod"] {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn collect_indices(value: &Value, out: &mut Vec<u64>) {
    match value {
        Value::Array(items) => items.iter().for_each(|v| collect_indices(v, out)),
        Value::Number(n) => out.push(n.as_u64().unwrap()),
        _ => {}
    }
}

/// Every boundary resolved to its coordinates, so two features can be
/// compared independently of how their vertices are indexed.
fn resolve(value: &Value, vertices: &[Value]) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.iter().map(|v| resolve(v, vertices)).collect()),
        Value::Number(n) => vertices[n.as_u64().unwrap() as usize].clone(),
        other => other.clone(),
    }
}

/// (object id, lod) -> the geometry with its boundaries resolved.
fn resolved_geometries(feature: &Value) -> BTreeMap<(String, String), Value> {
    let vertices = feature["vertices"].as_array().unwrap();
    let mut out = BTreeMap::new();
    for (id, object) in feature["CityObjects"].as_object().unwrap() {
        for geometry in object["geometry"].as_array().into_iter().flatten() {
            let mut resolved = geometry.clone();
            resolved["boundaries"] = resolve(&geometry["boundaries"], vertices);
            out.insert((id.clone(), lod_of(geometry)), resolved);
        }
    }
    out
}

#[test]
fn the_fixture_carries_lod_1_2_beside_1_3() {
    let (_, features) = delft();
    let lods: BTreeSet<String> = features
        .iter()
        .flat_map(|line| resolved_geometries(&serde_json::from_str(line).unwrap()).into_keys())
        .map(|(_, lod)| lod)
        .collect();
    assert!(lods.contains("1.2") && lods.contains("1.3"), "{lods:?}");
}

#[test]
fn the_dropped_lod_is_gone_and_every_other_geometry_is_coordinate_identical() {
    let (_, features) = delft();
    for line in &features {
        let before: Value = serde_json::from_str(line).unwrap();
        let after: Value = serde_json::from_str(&drop_12(line)).unwrap();
        let mut expected = resolved_geometries(&before);
        expected.retain(|(_, lod), _| lod != "1.2");
        assert_eq!(resolved_geometries(&after), expected);
    }
}

#[test]
fn no_orphan_vertex_remains_and_every_index_is_in_range() {
    let (_, features) = delft();
    let (mut before_total, mut after_total) = (0, 0);
    for line in &features {
        let after: Value = serde_json::from_str(&drop_12(line)).unwrap();
        before_total += serde_json::from_str::<Value>(line).unwrap()["vertices"]
            .as_array()
            .unwrap()
            .len();
        after_total += after["vertices"].as_array().unwrap().len();
        let count = after["vertices"].as_array().unwrap().len() as u64;
        let mut used = Vec::new();
        for object in after["CityObjects"].as_object().unwrap().values() {
            for geometry in object["geometry"].as_array().into_iter().flatten() {
                collect_indices(&geometry["boundaries"], &mut used);
            }
        }
        let used: BTreeSet<u64> = used.into_iter().collect();
        assert_eq!(used, (0..count).collect::<BTreeSet<u64>>(), "{line}");
    }
    // LoD 1.2 has vertices of its own on this fixture, so compaction is
    // exercised, not vacuous.
    assert!(
        after_total < before_total,
        "{after_total} >= {before_total}"
    );
}

#[test]
fn every_city_object_and_its_attributes_survive() {
    let (_, features) = delft();
    for line in &features {
        let before: Value = serde_json::from_str(line).unwrap();
        let after: Value = serde_json::from_str(&drop_12(line)).unwrap();
        let (b, a) = (
            before["CityObjects"].as_object().unwrap(),
            after["CityObjects"].as_object().unwrap(),
        );
        assert_eq!(b.keys().collect::<Vec<_>>(), a.keys().collect::<Vec<_>>());
        for (id, object) in b {
            assert_eq!(object["attributes"], a[id]["attributes"], "{id}");
            assert_eq!(object["type"], a[id]["type"], "{id}");
        }
        assert_eq!(before["id"], after["id"]);
    }
}

#[test]
fn without_a_lod_to_drop_the_feature_is_byte_identical() {
    let (_, features) = delft();
    for line in &features {
        assert_eq!(&drop_lods(line, &[]).unwrap(), line);
        // A LoD the feature does not carry changes nothing either.
        assert_eq!(&drop_lods(line, &["3.0".to_string()]).unwrap(), line);
    }
}

/// The reason for the flag: after it, every CityObject carries at most one
/// geometry per integer LoD — what CityGML 2.0's `lod{0..4}*` properties can
/// hold. Before it, LoD 1.2 and 1.3 share level 1.
#[test]
fn after_the_drop_every_object_fits_citygml_2_integer_lods() {
    fn clashes(feature: &Value) -> usize {
        feature["CityObjects"]
            .as_object()
            .unwrap()
            .values()
            .map(|object| {
                let levels: Vec<String> = object["geometry"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|g| lod_of(g).split('.').next().unwrap().to_string())
                    .collect();
                levels.len() - levels.iter().collect::<BTreeSet<_>>().len()
            })
            .sum()
    }
    let (_, features) = delft();
    let before: usize = features
        .iter()
        .map(|l| clashes(&serde_json::from_str(l).unwrap()))
        .sum();
    let after: usize = features
        .iter()
        .map(|l| clashes(&serde_json::from_str(&drop_12(l)).unwrap()))
        .sum();
    assert!(
        before > 0,
        "the fixture must show the clash the flag removes"
    );
    assert_eq!(after, 0);
}

#[test]
fn slicing_with_the_drop_keeps_the_object_counts() {
    let (header, features) = delft();
    let slice = |transform: bool| {
        let dir = tempfile::tempdir().unwrap();
        let mut iter = features.iter();
        let summaries = write_scaling_slices(
            &header,
            || {
                Ok(iter.next().map(|line| {
                    let value: Value = serde_json::from_str(line).unwrap();
                    let objects = value["CityObjects"].as_object().unwrap().len();
                    let out = if transform {
                        drop_12(line)
                    } else {
                        line.clone()
                    };
                    (out, objects)
                }))
            },
            &[500, 2000],
            dir.path(),
            "delft",
        )
        .unwrap();
        let counts: Vec<(usize, usize)> = summaries
            .iter()
            .map(|s| (s.features, s.city_objects))
            .collect();
        let bytes = std::fs::metadata(&summaries[1].path).unwrap().len();
        (counts, bytes)
    };
    let (plain, plain_bytes) = slice(false);
    let (dropped, dropped_bytes) = slice(true);
    assert_eq!(plain, dropped);
    assert!(
        dropped_bytes < plain_bytes,
        "the dropped geometry and its vertices are gone"
    );
}
