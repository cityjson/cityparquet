//! The corpus normalisation's second rule: no geometry below LoD 2 carries
//! semantic surfaces, on the real fixtures.
//!
//! CityGML 2.0's `_BoundarySurface` has `lod2MultiSurface`,
//! `lod3MultiSurface` and `lod4MultiSurface` only, so a LoD 0 or LoD 1
//! geometry cannot carry semantics there, and `citygml-tools from-cityjson
//! -v 2.0` drops them. `delft.city.jsonl` is a 3DBAG tile whose LoD 1.2,
//! 1.3 and 2.2 solids all carry semantics, the shape of the 3DBAG slice;
//! `tokyo_chiyoda_40.city.jsonl` carries semantics at LoD 2 only, and
//! `lod3_railway.city.json` at LoD 3 only.

use std::path::{Path, PathBuf};
use std::process::Command;

use cityparquet_readbench::lod::{normalise, normalise_line};
use serde_json::Value;

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../lib/cityparquet-rs/tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

fn local_fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// The feature lines of a CityJSONSeq (header excluded).
fn features(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .skip(1)
        .filter(|l| !l.trim().is_empty())
        .map(str::to_string)
        .collect()
}

fn lod_major(geometry: &Value) -> Option<f64> {
    match geometry.get("lod")? {
        Value::String(s) => s.parse().ok(),
        Value::Number(n) => n.as_f64(),
        _ => None,
    }
}

fn geometries(doc: &Value) -> Vec<(String, Value)> {
    doc["CityObjects"]
        .as_object()
        .unwrap()
        .iter()
        .flat_map(|(id, o)| {
            o["geometry"]
                .as_array()
                .into_iter()
                .flatten()
                .map(move |g| (id.clone(), g.clone()))
        })
        .collect()
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "readbench-semantics-below-lod2-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn lod_1_geometries_lose_their_semantics_and_lod_2_keeps_it() {
    let lines = features(&fixture("delft.city.jsonl"));
    let mut removed = 0;
    let mut low_with_semantics = 0;
    for line in &lines {
        let before: Value = serde_json::from_str(line).unwrap();
        let (out, report) = normalise_line(line).unwrap();
        let after: Value = serde_json::from_str(&out).unwrap();
        removed += report.semantics_removed;
        assert_eq!(report.dropped, 0, "delft carries no duplicate LoD");
        assert_eq!(report.geometries_before, report.geometries_after);
        assert_eq!(report.vertices_before, report.vertices_after);
        // Vertices, transform-free feature members and every boundary are
        // untouched: only the `semantics` member of a LoD < 2 geometry goes.
        assert_eq!(after["vertices"], before["vertices"]);
        let (b, a) = (geometries(&before), geometries(&after));
        assert_eq!(b.len(), a.len());
        for ((id, g), (id2, h)) in b.iter().zip(&a) {
            assert_eq!(id, id2);
            assert_eq!(g["boundaries"], h["boundaries"], "object {id}");
            let mut expected = g.clone();
            if lod_major(g).unwrap() < 2.0 {
                if g.get("semantics").is_some() {
                    low_with_semantics += 1;
                }
                expected.as_object_mut().unwrap().remove("semantics");
                assert!(h.get("semantics").is_none(), "object {id} LoD {}", g["lod"]);
            }
            assert_eq!(h, &expected, "object {id} LoD {}", g["lod"]);
        }
        // Everything outside the geometries is as it was.
        let strip = |v: &Value| {
            let mut v = v.clone();
            for o in v["CityObjects"].as_object_mut().unwrap().values_mut() {
                o.as_object_mut().unwrap().remove("geometry");
            }
            v
        };
        assert_eq!(strip(&after), strip(&before));
    }
    assert!(
        low_with_semantics > 0,
        "delft's LoD 1.x solids carry semantics"
    );
    assert_eq!(removed, low_with_semantics);
    // Every LoD 2.2 solid kept its semantics.
    let lod22_with_semantics = |text: &str| {
        geometries(&serde_json::from_str(text).unwrap())
            .iter()
            .filter(|(_, g)| g["lod"] == "2.2" && g.get("semantics").is_some())
            .count()
    };
    let before: usize = lines.iter().map(|l| lod22_with_semantics(l)).sum();
    let after: usize = lines
        .iter()
        .map(|l| lod22_with_semantics(&normalise_line(l).unwrap().0))
        .sum();
    assert!(before > 0);
    assert_eq!(after, before);
}

#[test]
fn a_feature_with_semantics_at_lod_2_and_above_only_is_returned_byte_for_byte() {
    for line in features(&local_fixture("tokyo_chiyoda_40.city.jsonl")) {
        let (out, report) = normalise_line(&line).unwrap();
        assert_eq!(out, line);
        assert_eq!((report.dropped, report.semantics_removed), (0, 0));
    }
}

#[test]
fn a_document_with_semantics_at_lod_3_only_is_left_untouched() {
    let doc: Value =
        serde_json::from_str(&std::fs::read_to_string(fixture("lod3_railway.city.json")).unwrap())
            .unwrap();
    let mut normalised = doc.clone();
    let report = normalise(&mut normalised).unwrap();
    assert_eq!((report.dropped, report.semantics_removed), (0, 0));
    assert_eq!(normalised, doc);
}

#[test]
fn the_lod_is_read_numerically() {
    // A numeric `lod` and an integer LoD 1 are below 2 as much as "1.3";
    // "2" is not.
    let line = &features(&fixture("delft.city.jsonl"))[0];
    let mut f: Value = serde_json::from_str(line).unwrap();
    for o in f["CityObjects"].as_object_mut().unwrap().values_mut() {
        for g in o["geometry"].as_array_mut().into_iter().flatten() {
            match g["lod"].as_str() {
                Some("1.2") => g["lod"] = Value::from("1"),
                Some("1.3") => g["lod"] = serde_json::json!(1.3),
                Some("2.2") => g["lod"] = Value::from("2"),
                _ => {}
            }
        }
    }
    let (out, report) = normalise_line(&serde_json::to_string(&f).unwrap()).unwrap();
    let after: Value = serde_json::from_str(&out).unwrap();
    assert!(report.semantics_removed > 0);
    for (_, g) in geometries(&after) {
        let below = lod_major(&g).unwrap() < 2.0;
        assert_eq!(g.get("semantics").is_none(), below, "LoD {}", g["lod"]);
    }
}

#[test]
fn the_binary_reports_both_rules_and_copies_an_unchanged_source() {
    let dir = scratch("binary");
    let bin = env!("CARGO_BIN_EXE_lod-normalise");

    let out = dir.join("delft.city.jsonl");
    let run = Command::new(bin)
        .arg(fixture("delft.city.jsonl"))
        .arg(&out)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let summary = String::from_utf8(run.stdout).unwrap();
    let lines = features(&fixture("delft.city.jsonl"));
    let low: usize = lines
        .iter()
        .map(|l| {
            geometries(&serde_json::from_str(l).unwrap())
                .iter()
                .filter(|(_, g)| lod_major(g).unwrap() < 2.0 && g.get("semantics").is_some())
                .count()
        })
        .sum();
    assert!(
        summary.starts_with(&format!("dropped=0 semantics_removed={low} ")),
        "summary: {summary}"
    );
    for line in features(&out) {
        let f: Value = serde_json::from_str(&line).unwrap();
        assert!(
            geometries(&f)
                .iter()
                .all(|(_, g)| lod_major(g).unwrap() >= 2.0 || g.get("semantics").is_none())
        );
    }

    let tokyo = local_fixture("tokyo_chiyoda_40.city.jsonl");
    let out = dir.join("tokyo.city.jsonl");
    let run = Command::new(bin).arg(&tokyo).arg(&out).output().unwrap();
    assert!(run.status.success());
    let summary = String::from_utf8(run.stdout).unwrap();
    assert!(
        summary.starts_with("dropped=0 semantics_removed=0 "),
        "summary: {summary}"
    );
    assert_eq!(std::fs::read(&out).unwrap(), std::fs::read(&tokyo).unwrap());
}
