//! Merge tests over the real `delft.city.jsonl` fixture (and derived copies of
//! it — never hand-authored synthetic CityJSON).

use std::fs;
use std::path::{Path, PathBuf};

use cityparquet::merge::merge_sources;
use cityparquet::source::Source;
use cjseq::{CityJSON, CityJSONFeature, Transform};

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

/// Real-world bbox of `features` dequantised through `t` (x,y,z min/max).
fn real_bbox(features: &[CityJSONFeature], t: &Transform) -> [f64; 6] {
    let mut b = [
        f64::INFINITY,
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for f in features {
        for v in &f.vertices {
            for i in 0..3 {
                let r = v[i] as f64 * t.scale[i] + t.translate[i];
                b[i] = b[i].min(r);
                b[i + 3] = b[i + 3].max(r);
            }
        }
    }
    b
}

/// Write a derived CityJSONSeq copy of `orig`'s first `take` features: header
/// `transform.scale` divided by `scale_div`, every vertex multiplied by
/// `vert_mul`. With `(scale_div, vert_mul) = (10.0, 10)` this is an EXACT
/// re-quantisation (same real coordinates, finer grid).
fn write_derived_copy(orig: &Path, dst: &Path, scale_div: f64, vert_mul: i64, take: usize) {
    let src = Source::open(orig).unwrap();
    let mut header: CityJSON = src.header().clone();
    for s in header.transform.scale.iter_mut() {
        *s /= scale_div;
    }
    let feats: Vec<CityJSONFeature> = src
        .features()
        .unwrap()
        .map(|f| f.unwrap())
        .take(take)
        .collect();
    let mut out = String::new();
    out.push_str(&serde_json::to_string(&header).unwrap());
    out.push('\n');
    for mut f in feats {
        for v in f.vertices.iter_mut() {
            for c in v.iter_mut() {
                *c *= vert_mul;
            }
        }
        out.push_str(&serde_json::to_string(&f).unwrap());
        out.push('\n');
    }
    fs::write(dst, out).unwrap();
}

#[test]
fn single_source_merge_preserves_transform_and_count() {
    let src = Source::open(&fixture("delft.city.jsonl")).unwrap();
    let n = src.features().unwrap().count();
    let merged = merge_sources(std::slice::from_ref(&src)).unwrap();
    assert_eq!(merged.header.transform.scale, src.header().transform.scale);
    assert_eq!(
        merged.header.transform.translate,
        src.header().transform.translate
    );
    assert_eq!(merged.features.len(), n);
    assert_eq!(merged.duplicate_ids, 0);
}

#[test]
fn heterogeneous_transforms_requantise_to_same_real_coords() {
    let d = tempfile::tempdir().unwrap();
    // A: first 5 features at the original scale. B: same 5, scale/10 + verts*10
    // (exact re-quantisation) so the two inputs describe identical real coords
    // on different grids -> the requantise path (not the all-equal fast path).
    let a = d.path().join("a.city.jsonl");
    let b = d.path().join("b.city.jsonl");
    write_derived_copy(&fixture("delft.city.jsonl"), &a, 1.0, 1, 5);
    write_derived_copy(&fixture("delft.city.jsonl"), &b, 10.0, 10, 5);

    let sa = Source::open(&a).unwrap();
    let sb = Source::open(&b).unwrap();
    let expect = real_bbox(
        &sa.features()
            .unwrap()
            .map(|f| f.unwrap())
            .collect::<Vec<_>>(),
        &sa.header().transform,
    );

    let merged = merge_sources(&[sa, sb]).unwrap();
    // Merged transform must be the finer (min) scale, i.e. B's.
    assert!(merged.header.transform.scale[0] < 0.001 + 1e-12);
    let got = real_bbox(&merged.features, &merged.header.transform);
    let tol = merged.header.transform.scale[0]; // <= mergedScale bound
    for i in 0..6 {
        assert!(
            (got[i] - expect[i]).abs() <= tol.max(1e-6),
            "axis {i}: {} vs {} (tol {tol})",
            got[i],
            expect[i]
        );
    }
}

#[test]
fn crs_mismatch_is_error() {
    let d = tempfile::tempdir().unwrap();
    let a = d.path().join("a.city.jsonl");
    let b = d.path().join("b.city.jsonl");
    write_derived_copy(&fixture("delft.city.jsonl"), &a, 1.0, 1, 3);
    // B: strip referenceSystem so its CRS differs from A's.
    {
        let src = Source::open(&fixture("delft.city.jsonl")).unwrap();
        let mut header = src.header().clone();
        if let Some(m) = header.metadata.as_mut() {
            m.reference_system = None;
        }
        let feats: Vec<CityJSONFeature> = src
            .features()
            .unwrap()
            .map(|f| f.unwrap())
            .take(3)
            .collect();
        let mut out = serde_json::to_string(&header).unwrap();
        out.push('\n');
        for f in feats {
            out.push_str(&serde_json::to_string(&f).unwrap());
            out.push('\n');
        }
        fs::write(&b, out).unwrap();
    }
    let sa = Source::open(&a).unwrap();
    let sb = Source::open(&b).unwrap();
    assert!(merge_sources(&[sa, sb]).is_err(), "CRS mismatch must error");
}

#[test]
fn duplicate_ids_are_counted() {
    let d = tempfile::tempdir().unwrap();
    let a = d.path().join("a.city.jsonl");
    write_derived_copy(&fixture("delft.city.jsonl"), &a, 1.0, 1, 5);
    let s1 = Source::open(&a).unwrap();
    let s2 = Source::open(&a).unwrap();
    let merged = merge_sources(&[s1, s2]).unwrap();
    assert_eq!(merged.features.len(), 10);
    assert_eq!(
        merged.duplicate_ids, 5,
        "same 5 features twice = 5 collisions"
    );
}
