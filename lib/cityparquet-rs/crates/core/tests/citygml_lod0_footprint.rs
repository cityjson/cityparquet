//! `bldg:lod0FootPrint` and `bldg:lod0RoofEdge` are LoD0 geometry.
//!
//! CityGML 2.0 spells a building's LoD0 representation with two dedicated
//! elements rather than the `lodNMultiSurface` the other LoDs use
//! (CityGML 2.0 §10.3.1): `lod0FootPrint` (the ground outline) and
//! `lod0RoofEdge` (the roof outline). Both hold a `gml:MultiSurface`.
//!
//! Every building in a PLATEAU (Japan) export carries a `lod0FootPrint`, and
//! so does most national LoD1 data elsewhere, so a reader that matches only
//! the `lodN*` spellings silently drops the whole LoD0 level of a national
//! dataset while every object and attribute still round-trips.
//!
//! Expected coordinates are hand-transcribed from the fixture's own
//! `lod0FootPrint`, never snapshotted from the reader's output.

use std::path::PathBuf;

use cityparquet::citygml::{FeatureReader, parse_header};

fn data_fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(name);
    assert!(p.exists(), "missing committed fixture {name}");
    p
}

/// The first point of the 16-point `lod0FootPrint` of
/// `bldg_05e35e6d-e88e-49a1-bb37-a921263899ea`, in the document's own
/// (latitude, longitude, height) order.
const FIRST_FOOTPRINT_POINT: [f64; 3] = [35.465501054428856, 139.6123486302035, 0.0];

#[test]
fn a_lod0_footprint_is_read_as_lod0_geometry() {
    let path = data_fixture("plateau_yokohama_bldg_fragment.gml");
    let header = parse_header(&path).unwrap();
    let scale = &header.transform.scale;
    let translate = &header.transform.translate;
    let mut reader = FeatureReader::open(&path, &header.transform).unwrap();

    let mut lod0_rings: Vec<Vec<[f64; 3]>> = Vec::new();
    for feature in reader.by_ref() {
        let feature = feature.unwrap();
        for co in feature.city_objects.values() {
            for geom in co.geometry.iter().flatten() {
                if geom.lod.as_deref() != Some("0") {
                    continue;
                }
                let boundaries: Vec<Vec<Vec<usize>>> =
                    serde_json::from_value(geom.boundaries.clone()).expect("MultiSurface shape");
                for surface in boundaries {
                    for ring in surface {
                        lod0_rings.push(
                            ring.iter()
                                .map(|&i| {
                                    let v = &feature.vertices[i];
                                    [
                                        v[0] as f64 * scale[0] + translate[0],
                                        v[1] as f64 * scale[1] + translate[1],
                                        v[2] as f64 * scale[2] + translate[2],
                                    ]
                                })
                                .collect(),
                        );
                    }
                }
            }
        }
    }

    // The fixture's two buildings carry one `lod0FootPrint` each, of 16 and 5
    // points — each a closed ring, so 15 and 4 distinct entries.
    let mut lengths: Vec<usize> = lod0_rings.iter().map(Vec::len).collect();
    lengths.sort_unstable();
    assert_eq!(
        lengths,
        vec![4, 15],
        "both footprints must be read, closure stripped"
    );

    let long_ring = lod0_rings
        .iter()
        .find(|r| r.len() == 15)
        .expect("the 16-point footprint");
    for axis in 0..3 {
        let error = (long_ring[0][axis] - FIRST_FOOTPRINT_POINT[axis]).abs();
        assert!(
            error <= scale[axis],
            "axis {axis}: read {} from {}, off by {error:.3e}",
            long_ring[0][axis],
            FIRST_FOOTPRINT_POINT[axis]
        );
    }
}
