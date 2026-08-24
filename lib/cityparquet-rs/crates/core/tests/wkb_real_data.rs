use std::path::{Path, PathBuf};

use cityparquet::source::Source;
use cityparquet::wkb_write::{VertexPool, geometry_bbox, geometry_to_wkb};
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

                let outcome = result.expect("every non-instance geometry must yield WKB bytes");
                let (bytes, bbox) = (outcome.bytes, outcome.bbox);
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
                                surfaces.len() - outcome.dropped_surfaces.len(),
                                "oracle polygon count must match source surface count minus writer-dropped degenerate surfaces"
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

/// Raw bits of a bbox: `assert_eq!` on `f64` is *value* equality, which would
/// let `-0.0` vs `0.0` (and NaN vs NaN) slip through. The walker's bbox must
/// be BITWISE-identical to the encoder's, so the sweep compares bit patterns.
fn bits(bbox: &[f64; 6]) -> [u64; 6] {
    std::array::from_fn(|i| bbox[i].to_bits())
}

/// Assert `geometry_bbox` and `geometry_to_wkb` agree on one geometry: same
/// `Some`/`None` outcome, bitwise-identical bbox when both produce one.
fn assert_walker_agrees(geom: &cjseq::Geometry, pool: &VertexPool<'_>, what: &str) {
    let via_wkb = geometry_to_wkb(geom, pool).unwrap().map(|o| o.bbox);
    let via_walker = geometry_bbox(geom, pool).unwrap();
    match (via_walker, via_wkb) {
        (Some(walker), Some(encoder)) => assert_eq!(
            bits(&walker),
            bits(&encoder),
            "{what}: walker bbox {walker:?} is not bitwise-equal to the encoder's {encoder:?} ({:?})",
            geom.thetype
        ),
        (walker, encoder) => assert!(
            walker.is_none() && encoder.is_none(),
            "{what}: walker/encoder disagree on whether {:?} yields a bbox \
             (walker: {walker:?}, encoder: {encoder:?})",
            geom.thetype
        ),
    }
}

/// P4: sweep every geometry of every feature in `path` through BOTH the
/// bbox/validate-only walker and the full WKB encoder, and require they agree
/// exactly — this is what licenses Task 5's callers to drop the throwaway
/// encode. Returns the number of geometries compared.
fn compare_walker_against_encoder(path: &Path) -> usize {
    let src = Source::open(path).unwrap();
    let header = src.header();
    let mut compared = 0usize;

    // Geometry templates first: they resolve through `VertexPool::raw`, the
    // walker's other vertex-storage path.
    if let Some(templates) = header.geometry_templates.as_ref() {
        let verts: Vec<Vec<f64>> =
            serde_json::from_value(templates.vertices_templates.clone()).unwrap();
        let pool = VertexPool::raw(&verts);
        for (i, tpl) in templates.templates.iter().enumerate() {
            assert_walker_agrees(tpl, &pool, &format!("template {i}"));
            compared += 1;
        }
    }

    for feature in src.features().unwrap() {
        let feature = feature.unwrap();
        let pool = VertexPool::new(&feature.vertices, &header.transform);
        for (id, co) in &feature.city_objects {
            let Some(geoms) = &co.geometry else {
                continue;
            };
            for (i, geom) in geoms.iter().enumerate() {
                assert_walker_agrees(geom, &pool, &format!("{id} geometry {i}"));
                compared += 1;
            }
        }
    }

    compared
}

#[test]
fn delft_walker_bbox_matches_the_encoder_bbox() {
    let compared = compare_walker_against_encoder(&fixture("delft.city.jsonl"));
    assert!(
        compared > 2000,
        "expected >2000 geometries compared, got {compared}"
    );
}

#[test]
fn railway_walker_bbox_matches_the_encoder_bbox() {
    let compared = compare_walker_against_encoder(&fixture("lod3_railway.city.json"));
    assert!(
        compared > 100,
        "expected >100 geometries compared, got {compared}"
    );
}

/// P4, the wiring guard: the sweeps above prove the walker and the encoder
/// agree geometry by geometry; this proves the SCAN PASS actually reports
/// what the encoder would have. `scan` no longer builds throwaway WKB — it
/// validates and bboxes through `geometry_bbox` — so its `dataset_bbox` is
/// re-derived here the old way, straight from `geometry_to_wkb` over exactly
/// the geometries the scan unions (lod-bearing ones; a `GeometryInstance` is
/// lod-less and contributes nothing), and must match BITWISE. Delft is the
/// fixture used because it is the one that carries its own
/// `referenceSystem`, which `scan` requires of coordinate-bearing input.
#[test]
fn scan_dataset_bbox_is_bitwise_the_wkb_encoder_union() {
    let path = fixture("delft.city.jsonl");

    let scanned = cityparquet::scan::scan(&Source::open(&path).unwrap()).unwrap();
    let got = scanned
        .dataset_bbox
        .expect("delft has lod-bearing geometry, so a dataset bbox");

    let src = Source::open(&path).unwrap();
    let header = src.header();
    let mut oracle: Option<[f64; 6]> = None;
    for feature in src.features().unwrap() {
        let feature = feature.unwrap();
        let pool = VertexPool::new(&feature.vertices, &header.transform);
        for co in feature.city_objects.values() {
            let Some(geoms) = &co.geometry else {
                continue;
            };
            for geom in geoms {
                if geom.lod.is_none() {
                    continue;
                }
                let Some(outcome) = geometry_to_wkb(geom, &pool).unwrap() else {
                    continue;
                };
                oracle = Some(match oracle.take() {
                    None => outcome.bbox,
                    Some(mut cur) => {
                        for i in 0..3 {
                            cur[i] = cur[i].min(outcome.bbox[i]);
                            cur[i + 3] = cur[i + 3].max(outcome.bbox[i + 3]);
                        }
                        cur
                    }
                });
            }
        }
    }
    let oracle = oracle.expect("the encoder oracle must produce a bbox for delft");

    assert_eq!(
        bits(&got),
        bits(&oracle),
        "scan's dataset bbox {got:?} is not bitwise-equal to the WKB encoder's union {oracle:?}"
    );
}
