use std::path::{Path, PathBuf};

use cityparquet::source::Source;
use cityparquet::wkb_read::{DecodedKind, wkb_to_geometry};
use cityparquet::wkb_write::{VertexPool, geometry_to_wkb};
use cjseq::{Geometry, GeometryType};

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

/// WKB rings repeat their first coordinate as a closing vertex; the decoded
/// ring must have that stripped back off. This computes how many indices a
/// *source* ring boils down to once that round trip settles — almost always
/// `ring.len()` (source rings aren't pre-closed), but `ring.len() - 1` for
/// the rare non-conformant already-closed ring (see `wkb_write`'s
/// `pre_closed_ring_is_not_re_closed` test).
fn expected_ring_len(ring: &[usize]) -> usize {
    if ring.len() >= 2 && ring.first() == ring.last() {
        ring.len() - 1
    } else {
        ring.len()
    }
}

fn assert_ring_matches(
    decoded_ring: &[usize],
    src_ring: &[usize],
    coords: &[[f64; 3]],
    pool: &VertexPool,
) {
    let expected_len = expected_ring_len(src_ring);
    assert_eq!(
        decoded_ring.len(),
        expected_len,
        "ring vertex count must match the source ring exactly (closing vertex stripped)"
    );
    for i in 0..expected_len {
        let expected = pool.coord(src_ring[i]).unwrap();
        assert_eq!(
            coords[decoded_ring[i]], expected,
            "coordinate at ring position {i} must equal the VertexPool-dequantised source vertex bitwise"
        );
    }
}

fn assert_polygon_list_matches(
    decoded: &[Vec<Vec<usize>>],
    src: &[Vec<Vec<usize>>],
    coords: &[[f64; 3]],
    pool: &VertexPool,
) {
    assert_eq!(
        decoded.len(),
        src.len(),
        "polygon/face count must match source exactly"
    );
    for (d_poly, s_poly) in decoded.iter().zip(src.iter()) {
        assert_eq!(
            d_poly.len(),
            s_poly.len(),
            "ring count within a polygon must match source exactly"
        );
        for (d_ring, s_ring) in d_poly.iter().zip(s_poly.iter()) {
            assert_ring_matches(d_ring, s_ring, coords, pool);
        }
    }
}

/// Asserts the decoded kind corresponds to `geom`'s `GeometryType` per the
/// writer's mapping, and that every ring/face/line count and coordinate
/// (bitwise) matches the source boundaries exactly.
fn assert_kind_matches_source(
    kind: &DecodedKind,
    coords: &[[f64; 3]],
    geom: &Geometry,
    pool: &VertexPool,
) {
    match geom.thetype {
        GeometryType::GeometryInstance => unreachable!("filtered out by caller"),
        GeometryType::MultiPoint => {
            let idxs: Vec<usize> = serde_json::from_value(geom.boundaries.clone()).unwrap();
            let DecodedKind::MultiPoint(decoded_idxs) = kind else {
                panic!("expected MultiPoint for {:?}, got {kind:?}", geom.thetype);
            };
            assert_eq!(decoded_idxs.len(), idxs.len());
            for (i, &src_idx) in idxs.iter().enumerate() {
                assert_eq!(coords[decoded_idxs[i]], pool.coord(src_idx).unwrap());
            }
        }
        GeometryType::MultiLineString => {
            let lines: Vec<Vec<usize>> = serde_json::from_value(geom.boundaries.clone()).unwrap();
            let DecodedKind::MultiLineString(decoded_lines) = kind else {
                panic!(
                    "expected MultiLineString for {:?}, got {kind:?}",
                    geom.thetype
                );
            };
            assert_eq!(decoded_lines.len(), lines.len());
            for (d_line, s_line) in decoded_lines.iter().zip(lines.iter()) {
                // MultiLineString isn't ring-closed by the writer: exact match.
                assert_eq!(d_line.len(), s_line.len());
                for (i, &src_idx) in s_line.iter().enumerate() {
                    assert_eq!(coords[d_line[i]], pool.coord(src_idx).unwrap());
                }
            }
        }
        GeometryType::MultiSurface | GeometryType::CompositeSurface => {
            let surfaces: Vec<Vec<Vec<usize>>> =
                serde_json::from_value(geom.boundaries.clone()).unwrap();
            let DecodedKind::MultiPolygon(decoded_surfaces) = kind else {
                panic!("expected MultiPolygon for {:?}, got {kind:?}", geom.thetype);
            };
            assert_polygon_list_matches(decoded_surfaces, &surfaces, coords, pool);
        }
        GeometryType::Solid => {
            let shells: Vec<Vec<Vec<Vec<usize>>>> =
                serde_json::from_value(geom.boundaries.clone()).unwrap();
            let DecodedKind::PolyhedralSurface(decoded_faces) = kind else {
                panic!(
                    "expected PolyhedralSurface for {:?}, got {kind:?}",
                    geom.thetype
                );
            };
            let flat_faces: Vec<Vec<Vec<usize>>> = shells.into_iter().flatten().collect();
            assert_polygon_list_matches(decoded_faces, &flat_faces, coords, pool);
        }
        GeometryType::MultiSolid | GeometryType::CompositeSolid => {
            let solids: Vec<Vec<Vec<Vec<Vec<usize>>>>> =
                serde_json::from_value(geom.boundaries.clone()).unwrap();
            let DecodedKind::GeometryCollection(members) = kind else {
                panic!(
                    "expected GeometryCollection for {:?}, got {kind:?}",
                    geom.thetype
                );
            };
            assert_eq!(members.len(), solids.len());
            for (member, shells) in members.iter().zip(solids.iter()) {
                let DecodedKind::PolyhedralSurface(decoded_faces) = member else {
                    panic!(
                        "expected each MultiSolid member to decode as PolyhedralSurface, got {member:?}"
                    );
                };
                let flat_faces: Vec<Vec<Vec<usize>>> =
                    shells.clone().into_iter().flatten().collect();
                assert_polygon_list_matches(decoded_faces, &flat_faces, coords, pool);
            }
        }
    }
}

/// Streams every geometry of every feature in `path` through
/// `geometry_to_wkb` then `wkb_to_geometry`, checking the round trip is
/// lossless: kind matches the source `GeometryType`, every ring/face/line
/// count matches the source boundaries exactly, and every decoded
/// coordinate equals the `VertexPool`-dequantised source coordinate
/// bitwise. Returns the number of non-`GeometryInstance` geometries that
/// actually produced WKB bytes.
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
                if geom.thetype == GeometryType::GeometryInstance {
                    assert!(
                        geometry_to_wkb(geom, &pool).unwrap().is_none(),
                        "GeometryInstance must map to None"
                    );
                    continue;
                }

                let Some((bytes, _bbox)) = geometry_to_wkb(geom, &pool).unwrap() else {
                    // Empty boundaries: writer intentionally emits nothing.
                    continue;
                };
                let decoded = wkb_to_geometry(&bytes).unwrap();
                assert_kind_matches_source(&decoded.kind, &decoded.coords, geom, &pool);
                processed += 1;
            }
        }
    }

    processed
}

#[test]
fn delft_geometries_round_trip_through_wkb_reader() {
    let processed = process_all(&fixture("delft.city.jsonl"));
    assert!(
        processed > 2000,
        "expected >2000 geometries checked, got {processed}"
    );
}

#[test]
fn railway_geometries_round_trip_through_wkb_reader() {
    let processed = process_all(&fixture("lod3_railway.city.json"));
    assert!(
        processed > 100,
        "expected >100 geometries checked, got {processed}"
    );
}
