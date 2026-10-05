//! A query window in each artefact's own axis order, on a real
//! latitude-first dataset (`tests/fixtures/tokyo_chiyoda_40.city.jsonl`,
//! JGD2011 / EPSG:6697).
//!
//! The window is derived from the CityParquet package's `bbox` column, which
//! stores longitude first as GeoParquet requires; the CityJSONSeq artefact
//! keeps the source's latitude-first order. Applying one window to both
//! unchanged makes every source-order format select nothing.

use std::path::{Path, PathBuf};
use std::process::Command;

use cityparquet::package::{ConvertOptions, convert};
use cityparquet_readbench::params;
use serde_json::Value;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tokyo_chiyoda_40.city.jsonl")
}

/// A prepared directory as `readbench_prepare.sh` leaves it for this input:
/// `<base>.parquet` and `<base>.city.jsonl`, for base `tokyo_chiyoda_40`.
fn prepared() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let mut opts = ConvertOptions::new(fixture(), dir.path().join("tokyo_chiyoda_40.parquet"));
    opts.generate_lod0 = true;
    convert(&opts).unwrap();
    std::fs::copy(fixture(), dir.path().join("tokyo_chiyoda_40.city.jsonl")).unwrap();
    dir
}

fn table(dir: &Path) -> PathBuf {
    dir.join("tokyo_chiyoda_40.parquet")
        .join("building.parquet")
}

/// Each feature's box over every vertex its objects' geometries reference,
/// in the source's own (latitude-first) order.
fn feature_boxes() -> Vec<[f64; 6]> {
    let text = std::fs::read_to_string(fixture()).unwrap();
    let mut lines = text.lines();
    let header: Value = serde_json::from_str(lines.next().unwrap()).unwrap();
    let scale: Vec<f64> = serde_json::from_value(header["transform"]["scale"].clone()).unwrap();
    let translate: Vec<f64> =
        serde_json::from_value(header["transform"]["translate"].clone()).unwrap();
    lines
        .map(|line| {
            let feature: Value = serde_json::from_str(line).unwrap();
            let vertices = feature["vertices"].as_array().unwrap();
            let mut b = [
                f64::INFINITY,
                f64::INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
            ];
            fn walk(v: &Value, f: &mut dyn FnMut(usize)) {
                match v {
                    Value::Array(items) => items.iter().for_each(|i| walk(i, f)),
                    Value::Number(n) => f(n.as_u64().unwrap() as usize),
                    _ => {}
                }
            }
            for object in feature["CityObjects"].as_object().unwrap().values() {
                for geometry in object["geometry"].as_array().into_iter().flatten() {
                    walk(&geometry["boundaries"], &mut |i| {
                        for axis in 0..3 {
                            let c = vertices[i][axis].as_i64().unwrap() as f64 * scale[axis]
                                + translate[axis];
                            b[axis] = b[axis].min(c);
                            b[axis + 3] = b[axis + 3].max(c);
                        }
                    });
                }
            }
            b
        })
        .collect()
}

fn intersects(b: &[f64; 6], w: &[f64; 6]) -> bool {
    (0..3).all(|a| !(b[a + 3] < w[a] || b[a] > w[a + 3]))
}

fn swap_xy(w: &[f64; 6]) -> [f64; 6] {
    [w[1], w[0], w[2], w[4], w[3], w[5]]
}

#[test]
fn the_crs_declares_latitude_first_and_the_params_say_so() {
    let dir = prepared();
    let resolved = params::resolve(
        "tokyo_chiyoda_40.city.jsonl",
        &table(dir.path()),
        None,
        None,
    )
    .unwrap();
    assert!(resolved.swap_xy, "EPSG:6697 is latitude first");
    // The window is in the package's longitude-first order: x is near 139.7.
    let w = resolved.windows[2].window;
    assert!(w[0] > 139.0 && w[1] < 36.0, "{w:?}");
}

#[test]
fn a_projected_or_unknown_crs_needs_no_swap() {
    use serde_json::json;
    assert!(!params::crs_is_latitude_first(None));
    let projected = json!({"coordinate_system": {"axis": [
        {"direction": "east"}, {"direction": "north"}
    ]}});
    assert!(!params::crs_is_latitude_first(Some(&projected)));
    let compound = json!({"components": [
        {"coordinate_system": {"axis": [{"direction": "north"}, {"direction": "east"}]}},
        {"coordinate_system": {"axis": [{"direction": "up"}]}}
    ]});
    assert!(params::crs_is_latitude_first(Some(&compound)));
}

/// The CityJSONSeq runner, reading the latitude-first stream, selects the
/// same features as an oracle intersecting the source-order boxes with the
/// window swapped into source order — and without the swap it would select
/// none.
#[test]
fn the_source_order_runner_gets_the_window_in_source_order() {
    let dir = prepared();
    let out = dir.path().join("out.csv");
    let status = Command::new(env!("CARGO_BIN_EXE_cityparquet-readbench"))
        .args(["run", "--input"])
        .arg(fixture())
        .arg("--prepared-dir")
        .arg(dir.path())
        .arg("--out")
        .arg(&out)
        .args([
            "--repeat",
            "1",
            "--formats",
            "cityjsonseq",
            "--scenarios",
            "bbox-query",
        ])
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );

    let sidecar: params::ResolvedParams = serde_json::from_str(
        &std::fs::read_to_string(dir.path().join("out.csv.params.json")).unwrap(),
    )
    .unwrap();
    let boxes = feature_boxes();
    let csv = std::fs::read_to_string(&out).unwrap();
    let counts: Vec<u64> = csv
        .lines()
        .skip(1)
        .map(|row| row.split(',').nth(4).unwrap().parse().unwrap())
        .collect();
    assert_eq!(counts.len(), sidecar.windows.len());
    let mut any = false;
    for (window, count) in sidecar.windows.iter().zip(&counts) {
        let expected = boxes
            .iter()
            .filter(|b| intersects(b, &swap_xy(&window.window)))
            .count();
        let unswapped = boxes
            .iter()
            .filter(|b| intersects(b, &window.window))
            .count();
        assert_eq!(*count as usize, expected, "{}", window.tag);
        assert_eq!(unswapped, 0, "the unswapped window misses every feature");
        any |= expected > 0;
    }
    assert!(any, "at least one window selects something");
}
