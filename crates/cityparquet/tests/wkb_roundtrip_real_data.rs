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
/// trailing duplicates of the first vertex index to a fixpoint (the WKB
/// closure the source pre-baked, possibly more than once), drop the ring if
/// fewer than 3 vertices remain, and drop the whole surface when its
/// EXTERIOR ring (index 0) is dropped. Returns the surface's expected
/// decoded rings, or `None` when the whole surface is expected to be
/// dropped.
///
/// This is an independent reimplementation of the production policy in
/// [`crate::wkb_write::normalise_ring`] and [`crate::wkb_write::normalise_surface`].
/// Module policy: this test-side mirror is deliberately separate so that
/// a bug in the production policy would not be hidden by both sides sharing
/// the same implementation. During M4, this mirror drifted from the
/// production code (single-strip vs fixpoint iteration) and had to be
/// patched — future changes to the production normalisation rules must
/// update both sites to keep them in sync.
fn normalise_expected_surface(surface: &[Vec<usize>]) -> Option<Vec<Vec<usize>>> {
    let mut kept = Vec::with_capacity(surface.len());
    for (i, ring) in surface.iter().enumerate() {
        let mut stripped = &ring[..];
        while stripped.len() >= 2 && stripped.first() == stripped.last() {
            stripped = &stripped[..stripped.len() - 1];
        }
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

/// Derived-from-real-fixture (sanctioned pattern): take delft's header line
/// plus ONE feature line carrying a real `Solid`, replace that Solid's
/// first face's first ring with the `[a, b, a, a]` shape built from the
/// ring's own first two real indices, write it to a tempdir, and confirm
/// the writer/reader pipeline drops exactly that one ring and its surface
/// while everything else round-trips.
#[test]
fn delft_derived_double_baked_closure_ring_is_dropped_and_still_round_trips() {
    let text = std::fs::read_to_string(fixture("delft.city.jsonl")).unwrap();
    let mut lines = text.lines();
    let header_line = lines.next().unwrap().to_string();

    let mut mutated_line = None;
    let mut mutated_ring: Option<(usize, usize)> = None;
    // Identifies exactly which CityObject/geometry-index was mutated, so the
    // checking loop below inspects THAT geometry rather than asserting on
    // whichever Solid a HashMap happens to iterate first (delft's feature
    // lines carry many CityObjects, most of them untouched).
    let mut mutated_co_id: Option<String> = None;
    let mut mutated_geom_index: Option<usize> = None;
    for line in lines {
        let mut feature: serde_json::Value = serde_json::from_str(line).unwrap();
        let Some(cos) = feature["CityObjects"].as_object_mut() else {
            continue;
        };
        let mut found = false;
        for (co_id, co) in cos.iter_mut() {
            let Some(geoms) = co.get_mut("geometry").and_then(|g| g.as_array_mut()) else {
                continue;
            };
            for (geom_index, geom) in geoms.iter_mut().enumerate() {
                if geom.get("type").and_then(|t| t.as_str()) != Some("Solid") {
                    continue;
                }
                // shells -> faces -> rings -> indices
                let ring = geom["boundaries"]
                    .get_mut(0) // first shell
                    .and_then(|shell| shell.get_mut(0)) // first face
                    .and_then(|face| face.get_mut(0)) // first (exterior) ring
                    .expect("real delft Solid must have a shell/face/ring to mutate");
                let indices: Vec<usize> = serde_json::from_value(ring.clone()).unwrap();
                let (a, b) = (indices[0], indices[1]);
                *ring = serde_json::json!([a, b, a, a]);
                mutated_ring = Some((a, b));
                mutated_co_id = Some(co_id.clone());
                mutated_geom_index = Some(geom_index);
                found = true;
                break;
            }
            if found {
                break;
            }
        }
        if found {
            mutated_line = Some(serde_json::to_string(&feature).unwrap());
            break;
        }
    }
    let mutated_line = mutated_line.expect("delft.city.jsonl must contain a Solid geometry");
    let (a, b) = mutated_ring.unwrap();
    assert_ne!(a, b, "a real ring's first two indices must be distinct");
    let mutated_co_id = mutated_co_id.unwrap();
    let mutated_geom_index = mutated_geom_index.unwrap();

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("delft_double_baked_closure.city.jsonl");
    std::fs::write(&path, format!("{header_line}\n{mutated_line}\n")).unwrap();

    let src = Source::open(&path).unwrap();
    let header = src.header();
    let feature = src.features().unwrap().next().unwrap().unwrap();
    let pool = VertexPool::new(&feature.vertices, &header.transform);

    let co = feature
        .city_objects
        .get(&mutated_co_id)
        .expect("mutated CityObject id must round-trip through the tempdir file");
    let geom = co
        .geometry
        .as_ref()
        .and_then(|geoms| geoms.get(mutated_geom_index))
        .expect("mutated geometry index must still be present");
    assert_eq!(geom.thetype, GeometryType::Solid);

    let outcome = geometry_to_wkb(geom, &pool)
        .unwrap()
        .expect("the Solid still has surviving faces after dropping one degenerate ring");
    assert_eq!(
        outcome.dropped_rings, 1,
        "exactly the mutated [a,b,a,a] ring must be dropped"
    );
    assert_eq!(
        outcome.dropped_surfaces,
        vec![0],
        "its face (the first, position 0) must be dropped with it"
    );
    wkb_to_geometry(&outcome.bytes)
        .expect("hardened reader must accept the writer's normalised output");
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
