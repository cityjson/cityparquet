use std::path::{Path, PathBuf};

use cityparquet::source::Source;
use cityparquet::wkb_write::{VertexPool, geometry_to_wkb};
use cjseq::GeometryType;
use geo_traits::{GeometryTrait, GeometryType as GtGeometryType, MultiPolygonTrait};

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

/// Stream every geometry of every feature in `path` through `geometry_to_wkb`.
/// Every non-`GeometryInstance` geometry must yield WKB bytes starting with
/// the little-endian marker and a finite, min<=max bbox. Every
/// `MultiSurface` buffer is round-tripped through the `wkb` crate oracle and
/// must read back as a `MultiPolygon` with the same polygon count as the
/// source boundaries. Returns the number of non-instance geometries
/// processed.
fn process_all(path: &Path) -> usize {
    let src = Source::open(path).unwrap();
    let header = src.header();
    let mut processed = 0usize;

    for feature in src.features().unwrap() {
        let feature = feature.unwrap();
        let pool = VertexPool::new(&feature.vertices, &header.transform);

        for co in feature.city_objects.values() {
            let Some(geoms) = &co.geometry else {
                continue;
            };
            for geom in geoms {
                let result = geometry_to_wkb(geom, &pool).unwrap();

                if geom.thetype == GeometryType::GeometryInstance {
                    assert!(result.is_none(), "GeometryInstance must map to None");
                    continue;
                }

                let (bytes, bbox) =
                    result.expect("every non-instance geometry must yield WKB bytes");
                processed += 1;

                assert_eq!(
                    bytes[0], 0x01,
                    "WKB buffer must start with the little-endian marker"
                );
                for i in 0..3 {
                    assert!(bbox[i].is_finite(), "bbox min[{i}] not finite: {bbox:?}");
                    assert!(
                        bbox[i + 3].is_finite(),
                        "bbox max[{i}] not finite: {bbox:?}"
                    );
                    assert!(
                        bbox[i] <= bbox[i + 3],
                        "bbox min > max on axis {i}: {bbox:?}"
                    );
                }

                if geom.thetype == GeometryType::MultiSurface {
                    let surfaces: Vec<Vec<Vec<usize>>> =
                        serde_json::from_value(geom.boundaries.clone()).unwrap();
                    let parsed = wkb::reader::read_wkb(&bytes)
                        .expect("wkb crate oracle must parse our own MultiPolygonZ output");
                    match parsed.as_type() {
                        GtGeometryType::MultiPolygon(mp) => {
                            assert_eq!(
                                mp.num_polygons(),
                                surfaces.len(),
                                "oracle polygon count must match source boundary surface count"
                            );
                        }
                        _ => panic!(
                            "expected the oracle to read a MultiPolygon for a MultiSurface geometry"
                        ),
                    }
                }
            }
        }
    }

    processed
}

#[test]
fn delft_geometries_round_trip_through_wkb() {
    let processed = process_all(&fixture("delft.city.jsonl"));
    assert!(
        processed > 2000,
        "expected >2000 geometries processed, got {processed}"
    );
}

#[test]
fn railway_geometries_round_trip_through_wkb() {
    let processed = process_all(&fixture("lod3_railway.city.json"));
    assert!(
        processed > 100,
        "expected >100 geometries processed, got {processed}"
    );
}
