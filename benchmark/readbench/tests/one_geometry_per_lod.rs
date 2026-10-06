//! `keep_first_per_lod`: the corpus normalisation that leaves every
//! CityObject with at most one geometry per LoD, on the real fixtures.
//!
//! CityParquet stores one geometry column per LoD, so its writer keeps the
//! first geometry at a given LoD on an object and counts the rest in
//! `ConvertReport::skipped_same_lod_geometries`. The benchmark corpus is
//! normalised the same way before any artefact is built, so all five
//! formats hold the same geometries. The fixtures carry no duplicate LoD, so
//! the duplicate cases relabel or move REAL geometries of the fixture; the
//! writer itself is the oracle for what "the same LoD" means.

use std::path::{Path, PathBuf};

use cityparquet::package::{ConvertOptions, convert};
use cityparquet_readbench::lod::{keep_first_per_lod, keep_first_per_lod_line};
use serde_json::Value;

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../lib/cityparquet-rs/tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

fn delft() -> (String, Vec<String>) {
    let text = std::fs::read_to_string(fixture("delft.city.jsonl")).unwrap();
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let header = lines.next().unwrap().to_string();
    (header, lines.map(str::to_string).collect())
}

fn railway() -> Value {
    serde_json::from_str(&std::fs::read_to_string(fixture("lod3_railway.city.json")).unwrap())
        .unwrap()
}

fn collect_indices(value: &Value, out: &mut Vec<u64>) {
    match value {
        Value::Array(items) => items.iter().for_each(|v| collect_indices(v, out)),
        Value::Number(n) => out.push(n.as_u64().unwrap()),
        _ => {}
    }
}

fn resolve(value: &Value, vertices: &[Value]) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.iter().map(|v| resolve(v, vertices)).collect()),
        Value::Number(n) => vertices[n.as_u64().unwrap() as usize].clone(),
        other => other.clone(),
    }
}

/// Each object's geometries with boundaries resolved to coordinates, so two
/// documents compare independently of vertex indexing.
fn resolved(doc: &Value) -> Vec<(String, Vec<Value>)> {
    let vertices = doc["vertices"].as_array().unwrap();
    doc["CityObjects"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(id, o)| {
            let geoms = o["geometry"]
                .as_array()
                .map(|gs| {
                    gs.iter()
                        .map(|g| {
                            let mut g = g.clone();
                            g["boundaries"] = resolve(&g["boundaries"], vertices);
                            g
                        })
                        .collect()
                })
                .unwrap_or_default();
            (id.clone(), geoms)
        })
        .collect()
}

/// Every vertex is referenced by some boundary.
fn assert_compact(doc: &Value) {
    let mut used = Vec::new();
    for o in doc["CityObjects"].as_object().unwrap().values() {
        for g in o["geometry"].as_array().into_iter().flatten() {
            collect_indices(&g["boundaries"], &mut used);
        }
    }
    used.sort_unstable();
    used.dedup();
    assert_eq!(used.len(), doc["vertices"].as_array().unwrap().len());
}

/// The delft features with every LoD `from` geometry relabelled `to`.
fn relabelled(features: &[String], from: &str, to: &str) -> Vec<String> {
    features
        .iter()
        .map(|line| {
            let mut f: Value = serde_json::from_str(line).unwrap();
            for o in f["CityObjects"].as_object_mut().unwrap().values_mut() {
                for g in o["geometry"].as_array_mut().into_iter().flatten() {
                    if g["lod"] == from {
                        g["lod"] = Value::from(to);
                    }
                }
            }
            serde_json::to_string(&f).unwrap()
        })
        .collect()
}

fn skipped(input: &Path, dir: &Path) -> usize {
    let mut opts = ConvertOptions::new(input.to_path_buf(), dir.join("pkg"));
    opts.overwrite = true;
    convert(&opts).unwrap().skipped_same_lod_geometries
}

fn write_seq(path: &Path, header: &str, features: &[String]) {
    let mut text = format!("{header}\n");
    for f in features {
        text.push_str(f);
        text.push('\n');
    }
    std::fs::write(path, text).unwrap();
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "readbench-one-per-lod-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn a_feature_without_duplicate_lods_is_returned_byte_for_byte() {
    let (_, features) = delft();
    for line in &features {
        let (out, report) = keep_first_per_lod_line(line).unwrap();
        assert_eq!(&out, line);
        assert_eq!(report.dropped, 0);
        assert_eq!(report.geometries_before, report.geometries_after);
        assert_eq!(report.vertices_before, report.vertices_after);
    }
}

#[test]
fn a_document_without_duplicate_lods_is_left_untouched() {
    let doc = railway();
    let mut normalised = doc.clone();
    let report = keep_first_per_lod(&mut normalised).unwrap();
    assert_eq!(report.dropped, 0);
    assert_eq!(normalised, doc);
}

#[test]
fn the_first_geometry_at_a_lod_is_kept_and_its_orphaned_vertices_removed() {
    let (_, features) = delft();
    // LoD 1.2 precedes LoD 1.3 on every building part, so relabelling 1.2 as
    // 1.3 makes the former 1.2 solid the first 1.3 geometry.
    let dup = relabelled(&features, "1.2", "1.3");
    let mut total = 0;
    for (line, original) in dup.iter().zip(&features) {
        let before: Value = serde_json::from_str(line).unwrap();
        let (out, report) = keep_first_per_lod_line(line).unwrap();
        let after: Value = serde_json::from_str(&out).unwrap();
        total += report.dropped;
        assert_eq!(
            report.geometries_before - report.dropped,
            report.geometries_after
        );
        assert!(report.vertices_after <= report.vertices_before);
        assert_compact(&after);
        // Per object: the first geometry of each LoD survives unchanged,
        // semantics included, and a later one at the same LoD is gone.
        let original: Value = serde_json::from_str(original).unwrap();
        for ((id, b), (id2, a)) in resolved(&before).iter().zip(resolved(&after)) {
            assert_eq!(id, &id2);
            let mut seen = Vec::new();
            let expected: Vec<&Value> = b
                .iter()
                .filter(|g| {
                    let lod = g["lod"].as_str().unwrap().to_string();
                    let first = !seen.contains(&lod);
                    seen.push(lod);
                    first
                })
                .collect();
            assert_eq!(a.iter().collect::<Vec<_>>(), expected, "object {id}");
            let had_12 = original["CityObjects"][id]["geometry"]
                .as_array()
                .is_some_and(|gs| gs.iter().any(|g| g["lod"] == "1.2"));
            if had_12 {
                assert!(a.iter().all(|g| g["lod"] != "1.2"));
                assert_eq!(a.iter().filter(|g| g["lod"] == "1.3").count(), 1);
            }
        }
    }
    assert!(total > 0, "the relabelled fixture must carry duplicates");
}

#[test]
fn lod_strings_are_keyed_as_the_writer_keys_them() {
    // "1" and "1.0" are the same LoD to the writer (`Lod::parse`), so the
    // normalisation must treat them as one, and agree with the writer's
    // skip count on the same input.
    let (header, features) = delft();
    let dup = relabelled(&relabelled(&features, "1.2", "1"), "1.3", "1.0");
    let dir = scratch("keying");
    let src = dir.join("dup.city.jsonl");
    write_seq(&src, &header, &dup);
    let mut dropped = 0;
    let normalised: Vec<String> = dup
        .iter()
        .map(|l| {
            let (out, r) = keep_first_per_lod_line(l).unwrap();
            dropped += r.dropped;
            out
        })
        .collect();
    assert!(dropped > 0);
    assert_eq!(
        skipped(&src, &dir),
        dropped,
        "writer and normalisation disagree"
    );
    let norm = dir.join("norm.city.jsonl");
    write_seq(&norm, &header, &normalised);
    assert_eq!(skipped(&norm, &dir), 0);
}

#[test]
fn appearance_and_instances_of_a_document_survive() {
    // Move one real textured LoD 3 MultiSurface onto another object that
    // already carries a LoD 3 MultiSurface: the moved copy is the duplicate.
    let mut doc = railway();
    let objects = doc["CityObjects"].as_object().unwrap();
    let textured: Vec<String> = objects
        .iter()
        .filter(|(_, o)| {
            o["geometry"].as_array().is_some_and(|gs| {
                gs.len() == 1 && gs[0]["type"] == "MultiSurface" && gs[0].get("texture").is_some()
            })
        })
        .map(|(id, _)| id.clone())
        .take(2)
        .collect();
    let with_instance = objects
        .iter()
        .find(|(_, o)| {
            o["geometry"]
                .as_array()
                .is_some_and(|gs| gs.iter().any(|g| g["type"] == "GeometryInstance"))
        })
        .map(|(id, _)| id.clone())
        .unwrap();
    let (keeper, donor) = (&textured[0], &textured[1]);
    let moved = doc["CityObjects"][donor]["geometry"][0].clone();
    let kept = doc["CityObjects"][keeper]["geometry"][0].clone();
    doc["CityObjects"][keeper]["geometry"]
        .as_array_mut()
        .unwrap()
        .push(moved);
    let instances_before = doc["CityObjects"][&with_instance]["geometry"].clone();
    let textures_before = doc["appearance"].clone();

    let dir = scratch("appearance");
    let src = dir.join("dup.city.json");
    std::fs::write(&src, serde_json::to_string(&doc).unwrap()).unwrap();
    assert_eq!(skipped(&src, &dir), 1);

    let report = keep_first_per_lod(&mut doc).unwrap();
    assert_eq!(report.dropped, 1);
    assert_eq!(
        doc["CityObjects"][keeper]["geometry"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    // The donor still references the moved geometry's vertices, so none is
    // orphaned and the kept geometry is byte-for-byte what it was.
    assert_eq!(doc["CityObjects"][keeper]["geometry"][0], kept);
    assert_eq!(
        doc["CityObjects"][&with_instance]["geometry"],
        instances_before
    );
    assert_eq!(doc["appearance"], textures_before);
    assert_compact(&doc);

    let norm = dir.join("norm.city.json");
    std::fs::write(&norm, serde_json::to_string(&doc).unwrap()).unwrap();
    assert_eq!(skipped(&norm, &dir), 0);
}
