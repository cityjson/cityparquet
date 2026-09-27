//! A ring and its texture coordinates lose their closing entry together.
//!
//! GML pairs `app:textureCoordinates` with the ring's positions one for one,
//! closing position included. CityJSON drops the closing entry from both. The
//! decision is the ring's: a sliver ring whose last position misses its first
//! by a fraction of a millimetre keeps every position, and its UVs must then
//! keep every pair too — even though they open and close on the same pair.
//! Deciding each list by its own equality test leaves one UV short, and the
//! whole package fails to encode.
//!
//! The fixture is building 1351447 of Montréal's 2020 model, whose ring
//! `UUID_578673a6-3812-4bad-87e1-38f605fd82da` has exactly that shape (5
//! positions, the first and last apart by micrometres; 5 UV pairs, the first
//! and last equal). 52 buildings of the Outremont borough alone have one.

use std::path::PathBuf;

use cityparquet::citygml::{FeatureReader, parse_header};
use cityparquet::export::{ExportOptions, export};
use cityparquet::package::{ConvertOptions, convert};
use serde_json::Value;

fn data_fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(name);
    assert!(p.exists(), "missing committed fixture {name}");
    p
}

/// Every textured ring leaf `[texture, uv…]` beside its boundary ring
/// `[vertex…]`, walked in lockstep.
fn pairs<'a>(boundary: &'a Value, texture: &'a Value, out: &mut Vec<(&'a Value, &'a Value)>) {
    match (boundary, texture) {
        (Value::Array(b), Value::Array(_)) if b.first().is_some_and(Value::is_number) => {
            out.push((boundary, texture));
        }
        (Value::Array(b), Value::Array(t)) => {
            for (bi, ti) in b.iter().zip(t) {
                pairs(bi, ti, out);
            }
        }
        _ => {}
    }
}

#[test]
fn the_reader_gives_a_sliver_ring_one_uv_per_position() {
    let path = data_fixture("montreal_2020_bldg_sliver_ring_fragment.gml");
    let header = parse_header(&path).unwrap();
    let mut reader = FeatureReader::open(&path, &header.transform).unwrap();
    let feature = reader.next().expect("one member").expect("it must read");

    let mut textured = 0;
    for co in feature.city_objects.values() {
        for g in co.geometry.iter().flatten() {
            let Some(textures) = g.texture.as_ref() else {
                continue;
            };
            let boundaries = &g.boundaries;
            let textures = serde_json::to_value(textures).unwrap();
            for theme in textures.as_object().unwrap().values() {
                let mut rings = Vec::new();
                pairs(boundaries, &theme["values"], &mut rings);
                for (ring, tex) in rings {
                    let tex = tex.as_array().unwrap();
                    if tex[0].is_null() {
                        continue;
                    }
                    textured += 1;
                    assert_eq!(
                        tex.len() - 1,
                        ring.as_array().unwrap().len(),
                        "one UV per ring position: {ring} vs {tex:?}"
                    );
                }
            }
        }
    }
    assert!(textured > 0, "the building is textured");
}

#[test]
fn a_sliver_ring_converts_and_exports_one_uv_per_vertex() {
    let tmp = tempfile::tempdir().unwrap();
    let pkg = tmp.path().join("pkg");
    let mut opts = ConvertOptions::new(
        data_fixture("montreal_2020_bldg_sliver_ring_fragment.gml"),
        pkg.clone(),
    );
    opts.crs_override = Some("EPSG:2950".to_string());
    let report = convert(&opts).expect("a sliver ring must not fail the package");
    assert_eq!(report.object_count, 1);

    let out = tmp.path().join("back.city.jsonl");
    export(&ExportOptions {
        package_dir: pkg,
        output: out.clone(),
    })
    .unwrap();
    let text = std::fs::read_to_string(&out).unwrap();
    let feature: Value = serde_json::from_str(text.lines().nth(1).unwrap()).unwrap();
    for co in feature["CityObjects"].as_object().unwrap().values() {
        for g in co["geometry"].as_array().into_iter().flatten() {
            for theme in g["texture"]
                .as_object()
                .into_iter()
                .flat_map(|t| t.values())
            {
                let mut rings = Vec::new();
                pairs(&g["boundaries"], &theme["values"], &mut rings);
                for (ring, tex) in rings {
                    let tex = tex.as_array().unwrap();
                    if !tex[0].is_null() {
                        assert_eq!(tex.len() - 1, ring.as_array().unwrap().len());
                    }
                }
            }
        }
    }
}
