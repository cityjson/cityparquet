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

/// Test-side mirror of the writer's structural normalisation policy: strip
/// one trailing duplicate of the first vertex index (the WKB closure the
/// source pre-baked), drop the ring if fewer than 3 vertices remain, and
/// drop the whole surface when its EXTERIOR ring (index 0) is dropped.
/// Returns the surface's expected decoded rings, or `None` when the whole
/// surface is expected to be dropped.
fn normalise_expected_surface(surface: &[Vec<usize>]) -> Option<Vec<Vec<usize>>> {
    let mut kept = Vec::with_capacity(surface.len());
    for (i, ring) in surface.iter().enumerate() {
        let stripped = if ring.len() >= 2 && ring.first() == ring.last() {
            &ring[..ring.len() - 1]
        } else {
            &ring[..]
        };
        if stripped.len() >= 3 {
            kept.push(stripped.to_vec());
        } else if i == 0 {
            return None;
        }
    }
    Some(kept)
}

fn normalise_expected_surfaces(surfaces: &[Vec<Vec<usize>>]) -> Vec<Vec<Vec<usize>>> {
    surfaces
        .iter()
        .filter_map(|s| normalise_expected_surface(s))
        .collect()
}

fn assert_ring_matches(
    decoded_ring: &[usize],
    src_ring: &[usize],
    coords: &[[f64; 3]],
    pool: &VertexPool,
) {
    assert_eq!(
        decoded_ring.len(),
        src_ring.len(),
        "ring vertex count must match the normalised source ring exactly"
    );
    for (i, &src_idx) in src_ring.iter().enumerate() {
        let expected = pool.coord(src_idx).unwrap();
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
        "polygon/face count must match the normalised source exactly"
    );
    for (d_poly, s_poly) in decoded.iter().zip(src.iter()) {
        assert_eq!(
            d_poly.len(),
            s_poly.len(),
            "ring count within a polygon must match the normalised source exactly"
        );
        for (d_ring, s_ring) in d_poly.iter().zip(s_poly.iter()) {
            assert_ring_matches(d_ring, s_ring, coords, pool);
        }
    }
}

/// Asserts the decoded kind corresponds to `geom`'s `GeometryType` per the
/// writer's mapping, and that every ring/face/line count and coordinate
/// (bitwise) matches the source boundaries exactly, after applying the
/// writer's structural normalisation policy to the source.
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
                // MultiLineString isn't ring-normalised by the writer: exact match.
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
            let expected = normalise_expected_surfaces(&surfaces);
            assert_polygon_list_matches(decoded_surfaces, &expected, coords, pool);
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
            let expected = normalise_expected_surfaces(&flat_faces);
            assert_polygon_list_matches(decoded_faces, &expected, coords, pool);
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
                let expected = normalise_expected_surfaces(&flat_faces);
                assert_polygon_list_matches(decoded_faces, &expected, coords, pool);
            }
        }
    }
}

/// Round-trip tallies for one fixture.
#[derive(Debug, Default, PartialEq, Eq)]
struct Totals {
    /// Non-instance geometries that produced WKB and round-tripped losslessly.
    round_tripped: usize,
    /// Writer-reported structurally degenerate rings dropped.
    dropped_rings: usize,
    /// Writer-reported surfaces dropped (exterior ring degenerate).
    dropped_surfaces: usize,
    /// Geometries with at least one drop.
    geometries_with_drops: usize,
}

/// Streams every geometry of every feature in `path` through
/// `geometry_to_wkb` then `wkb_to_geometry`. EVERY geometry that yields WKB
/// must be accepted by the hardened reader (the writer's structural
/// normalisation guarantees this by construction) and round-trip losslessly
/// against the policy-normalised source boundaries.
fn process_all(path: &Path) -> Totals {
    let src = Source::open(path).unwrap();
    let header = src.header();
    let mut totals = Totals::default();

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

                let Some(outcome) = geometry_to_wkb(geom, &pool).unwrap() else {
                    // Empty boundaries: writer intentionally emits nothing.
                    continue;
                };

                let decoded = wkb_to_geometry(&outcome.bytes)
                    .expect("hardened reader must accept every writer output");
                assert_kind_matches_source(&decoded.kind, &decoded.coords, geom, &pool);

                totals.round_tripped += 1;
                totals.dropped_rings += outcome.dropped_rings;
                totals.dropped_surfaces += outcome.dropped_surfaces.len();
                if outcome.dropped_rings > 0 || !outcome.dropped_surfaces.is_empty() {
                    totals.geometries_with_drops += 1;
                }
            }
        }
    }

    totals
}

#[test]
fn delft_geometries_round_trip_through_wkb_reader() {
    let totals = process_all(&fixture("delft.city.jsonl"));
    assert!(
        totals.round_tripped > 2000,
        "expected >2000 geometries checked, got {}",
        totals.round_tripped
    );
    assert_eq!(totals.dropped_rings, 0, "delft has no degenerate rings");
    assert_eq!(totals.dropped_surfaces, 0);
    assert_eq!(totals.geometries_with_drops, 0);
}

#[test]
fn railway_geometries_round_trip_through_wkb_reader() {
    let totals = process_all(&fixture("lod3_railway.city.json"));
    // With degenerate rings dropped at write time, every geometry that
    // yields WKB round-trips: 105/105, zero reader rejections.
    assert_eq!(
        totals.round_tripped, 105,
        "all 105 railway geometries must round-trip"
    );
    // lod3_railway carries exactly 6 structurally degenerate [a,b,a] rings,
    // each the sole (exterior) ring of its surface, across 3 geometries
    // (one CompositeSurface with 4, two MultiSurfaces with 1 each).
    assert_eq!(totals.dropped_rings, 6);
    assert_eq!(totals.dropped_surfaces, 6);
    assert_eq!(totals.geometries_with_drops, 3);
}
