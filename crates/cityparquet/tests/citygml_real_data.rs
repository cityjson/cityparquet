//! CityGML 2.0 input reader — real-fixture tests.
//!
//! Fixtures are real CityGML 2.0 files fetched by `just fixtures`; no inline
//! hand-written GML. Expected coordinates below are hand-transcribed from the
//! fixture (never snapshot the reader's own output) so the assertions stay
//! non-circular.

use std::path::PathBuf;

use cityparquet::source::{Source, SourceFormat};

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

/// Dequantise a feature vertex back to world coordinates using the header
/// transform (the reader must set `header.transform` to whatever it quantised
/// the feature vertices against).
fn dequantise(v: &[i64], tr: &cityparquet::cjseq::Transform) -> [f64; 3] {
    [
        v[0] as f64 * tr.scale[0] + tr.translate[0],
        v[1] as f64 * tr.scale[1] + tr.translate[1],
        v[2] as f64 * tr.scale[2] + tr.translate[2],
    ]
}

// `b1_lod2_s.gml` (jklimke/libcitygml): one Building, a single lod2 `gml:Solid`
// (house shape: 100x100 footprint, walls to z=100, ridge at z=150), 9 exterior
// surfaces, 10 distinct vertices, coordinates in `<gml:pos>` elements, no
// `srsName`, no attributes, no semantics.
#[test]
fn citygml2_solid_building_streams_one_feature() {
    let src = Source::open(&fixture("b1_lod2_s.gml")).unwrap();
    assert_eq!(src.format(), SourceFormat::CityGml);
    assert_eq!(src.header().version, "2.0");
    // No `srsName` in the fixture -> no reference system advertised.
    assert!(
        src.header()
            .metadata
            .as_ref()
            .and_then(|m| m.reference_system.as_ref())
            .is_none(),
        "fixture has no srsName; reader must not invent a CRS"
    );

    // Collect via Result<Vec<_>>: `count()` would also count Err items.
    let feats: Vec<_> = src
        .features()
        .unwrap()
        .collect::<cityparquet::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(feats.len(), 1, "expected exactly one Building feature");
    let f = &feats[0];
    assert_eq!(f.city_objects.len(), 1);
    let co = f.city_objects.values().next().unwrap();
    assert_eq!(co.thetype.as_str(), "Building");

    let geoms = co.geometry.as_ref().expect("building has geometry");
    assert_eq!(geoms.len(), 1);
    let g = &geoms[0];
    assert_eq!(g.lod.as_deref(), Some("2"));

    // Solid boundaries: exactly one exterior shell of 9 surfaces.
    let shells = g
        .boundaries
        .as_array()
        .expect("solid boundaries are an array");
    assert_eq!(shells.len(), 1, "one exterior shell");
    let surfaces = shells[0].as_array().expect("shell is an array of surfaces");
    assert_eq!(surfaces.len(), 9, "9 surface members");

    // 10 distinct vertices; a few hand-transcribed world coordinates must be
    // recoverable after dequantisation (validates gml:pos parsing, ring-close
    // drop, quantisation, and that header.transform matches the quantiser).
    assert_eq!(f.vertices.len(), 10, "10 distinct vertices after dedup");
    let tr = &src.header().transform;
    let world: Vec<[f64; 3]> = f.vertices.iter().map(|v| dequantise(v, tr)).collect();
    let has = |p: [f64; 3]| {
        world
            .iter()
            .any(|c| (0..3).all(|i| (c[i] - p[i]).abs() < 1e-6))
    };
    assert!(has([0.0, 0.0, 0.0]), "base corner");
    assert!(has([100.0, 100.0, 0.0]), "opposite base corner");
    assert!(has([50.0, 0.0, 150.0]), "front ridge apex");
    assert!(has([50.0, 100.0, 150.0]), "back ridge apex");
}

// Exercise the scan -> WKB-encode path (not just Source::features): scanning
// runs every geometry through `geometry_to_wkb`, so this also proves the
// vertices/boundaries the reader produced form valid WKB and a correct bbox.
#[test]
fn citygml2_scan_infers_lod2_and_bbox() {
    use cityparquet::scan::scan;
    use cityparquet::schema::SourceFormat as SchemaSourceFormat;

    let src = Source::open(&fixture("b1_lod2_s.gml")).unwrap();
    let s = scan(&src).unwrap();
    assert_eq!(s.object_count, 1);
    let lods: Vec<String> = s.lods.iter().map(ToString::to_string).collect();
    assert_eq!(lods, ["2"]);

    // House solid extent: x,y in [0,100], z in [0,150].
    let bbox = s.dataset_bbox.expect("dataset bbox from WKB");
    for (got, exp) in bbox.iter().zip([0.0, 0.0, 0.0, 100.0, 100.0, 150.0]) {
        assert!((got - exp).abs() < 1e-6, "bbox component {got} != {exp}");
    }

    let meta = s.metadata(&[]).unwrap();
    assert_eq!(meta.default_geometry, "geometry_lod2");
    assert_eq!(meta.source_format, SchemaSourceFormat::CityGml);
}

/// Newell's method normal of a ring of world coordinates (unnormalised).
fn newell(ring: &[[f64; 3]]) -> [f64; 3] {
    let mut n = [0.0f64; 3];
    for i in 0..ring.len() {
        let a = ring[i];
        let b = ring[(i + 1) % ring.len()];
        n[0] += (a[1] - b[1]) * (a[2] + b[2]);
        n[1] += (a[2] - b[2]) * (a[0] + b[0]);
        n[2] += (a[0] - b[0]) * (a[1] + b[1]);
    }
    n
}

// `b1_lod2_cs_w_sem.gml` (libcitygml): one Building whose lod2Solid is a
// `gml:CompositeSolid` of TWO solids. Every surfaceMember is an `xlink:href`
// into a polygon defined LATER in the file (in `boundedBy` semantic surfaces,
// or the standalone `lod2MultiSurface` ceiling) — forward references the reader
// must resolve. Solid B reuses the ceiling via `OrientableSurface
// orientation="-"` (flipped). Semantic surfaces (doc order): 0 Ground, 1-4
// Wall, 5-8 Roof; the ceiling has no semantic surface -> null.
#[test]
fn citygml2_composite_solid_with_semantics_and_xlinks() {
    let src = Source::open(&fixture("b1_lod2_cs_w_sem.gml")).unwrap();
    let feats: Vec<_> = src
        .features()
        .unwrap()
        .collect::<cityparquet::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(feats.len(), 1);
    let f = &feats[0];
    let co = f.city_objects.values().next().unwrap();
    assert_eq!(co.thetype.as_str(), "Building");

    // Exactly one LoD2 geometry: the CompositeSolid (the standalone ceiling
    // MultiSurface and the boundedBy surfaces must NOT leak as extra geometry).
    let geoms = co.geometry.as_ref().unwrap();
    assert_eq!(geoms.len(), 1, "one geometry (no leaked MultiSurface)");
    let g = &geoms[0];
    assert!(
        matches!(g.thetype, cityparquet::cjseq::GeometryType::CompositeSolid),
        "expected CompositeSolid, got {:?}",
        g.thetype
    );
    assert_eq!(g.lod.as_deref(), Some("2"));

    // Boundaries: [solid][shell][surface][ring][idx]. Two solids, one shell
    // each, face counts 6 and 5, one ring per face.
    let solids = g.boundaries.as_array().unwrap();
    assert_eq!(solids.len(), 2, "CompositeSolid of 2 solids");
    let shells: Vec<&Vec<serde_json::Value>> =
        solids.iter().map(|s| s.as_array().unwrap()).collect();
    assert_eq!(shells[0].len(), 1, "solid A one shell");
    assert_eq!(shells[1].len(), 1, "solid B one shell");
    let faces_a = shells[0][0].as_array().unwrap();
    let faces_b = shells[1][0].as_array().unwrap();
    assert_eq!(faces_a.len(), 6, "solid A faces");
    assert_eq!(faces_b.len(), 5, "solid B faces");

    // Per-face vertex counts fingerprint the right polygon in each slot
    // (roof_1/roof_2 are triangles).
    let ring_len = |face: &serde_json::Value| face.as_array().unwrap()[0].as_array().unwrap().len();
    let counts_a: Vec<usize> = faces_a.iter().map(ring_len).collect();
    let counts_b: Vec<usize> = faces_b.iter().map(ring_len).collect();
    assert_eq!(counts_a, [4, 4, 4, 4, 4, 4]);
    assert_eq!(counts_b, [4, 3, 3, 4, 4]);

    // 10 distinct vertices: 8 box corners + 2 ridge points (ceiling reuse and
    // every shared edge deduped).
    assert_eq!(f.vertices.len(), 10, "10 distinct vertices");

    let tr = &src.header().transform;
    // Resolve a face (solid s, face i) to its world-coordinate exterior ring.
    let face_ring = |s: usize, i: usize| -> Vec<[f64; 3]> {
        let ring = solids[s].as_array().unwrap()[0].as_array().unwrap()[i]
            .as_array()
            .unwrap()[0]
            .as_array()
            .unwrap();
        ring.iter()
            .map(|v| {
                let idx = v.as_u64().unwrap() as usize;
                dequantise(&f.vertices[idx], tr)
            })
            .collect()
    };

    // Reversal (non-circular): the ceiling is solid A face 5 and solid B face 0.
    // Same vertex set, opposite winding -> Newell normals +z and -z.
    let ceil_a = face_ring(0, 5);
    let ceil_b = face_ring(1, 0);
    assert!(
        ceil_a.iter().all(|c| (c[2] - 100.0).abs() < 1e-6),
        "ceiling A at z=100"
    );
    assert!(
        ceil_b.iter().all(|c| (c[2] - 100.0).abs() < 1e-6),
        "ceiling B at z=100"
    );
    assert!(newell(&ceil_a)[2] > 0.0, "ceiling A normal +z");
    assert!(newell(&ceil_b)[2] < 0.0, "ceiling B normal -z (flipped)");

    // Semantics: surfaces histogram + null ceiling positions, checked by TYPE
    // lookup (robust to intern order), with a geometric cross-check.
    let sem = g.semantics.as_ref().expect("semantics");
    let surfaces = sem["surfaces"].as_array().unwrap();
    assert_eq!(surfaces.len(), 9, "9 boundedBy semantic surfaces");
    let mut hist = std::collections::BTreeMap::new();
    for s in surfaces {
        *hist
            .entry(s["type"].as_str().unwrap().to_string())
            .or_insert(0) += 1;
    }
    assert_eq!(hist.get("GroundSurface"), Some(&1));
    assert_eq!(hist.get("WallSurface"), Some(&4));
    assert_eq!(hist.get("RoofSurface"), Some(&4));

    // values nesting [solid][shell][face]; ceiling faces are null.
    let values = sem["values"].as_array().unwrap();
    let va = values[0].as_array().unwrap()[0].as_array().unwrap();
    let vb = values[1].as_array().unwrap()[0].as_array().unwrap();
    assert_eq!(va.len(), 6);
    assert_eq!(vb.len(), 5);
    assert!(va[5].is_null(), "solid A ceiling face has no semantics");
    assert!(vb[0].is_null(), "solid B ceiling face has no semantics");

    // Cross-check each non-null face's semantic TYPE against its geometry.
    let type_of = |sv: &serde_json::Value| -> Option<String> {
        sv.as_u64()
            .map(|i| surfaces[i as usize]["type"].as_str().unwrap().to_string())
    };
    // Exact semantic values (fixture-derived), proving the xlink map is not
    // merely type-consistent but points at the correct polygon in each slot.
    let leaf = |sv: &serde_json::Value| sv.as_i64();
    assert_eq!(
        va.iter().map(leaf).collect::<Vec<_>>(),
        [Some(0), Some(1), Some(2), Some(3), Some(4), None]
    );
    assert_eq!(
        vb.iter().map(leaf).collect::<Vec<_>>(),
        [None, Some(5), Some(6), Some(7), Some(8)]
    );

    // Solid A face 0 is the ground (all z==0) and must map to GroundSurface.
    assert_eq!(type_of(&va[0]).as_deref(), Some("GroundSurface"));
    assert!(
        face_ring(0, 0).iter().all(|c| c[2].abs() < 1e-6),
        "ground at z=0"
    );
    // Solid A faces 1..=4 are the four walls, each on a distinct plane (this
    // pins the exact wall-polygon-to-slot mapping, not just "is a WallSurface").
    let on_plane =
        |ring: &[[f64; 3]], axis: usize, v: f64| ring.iter().all(|c| (c[axis] - v).abs() < 1e-6);
    for (i, (axis, v)) in [(1usize, 0.0), (0, 100.0), (1, 100.0), (0, 0.0)]
        .into_iter()
        .enumerate()
    {
        let face = i + 1;
        assert_eq!(type_of(&va[face]).as_deref(), Some("WallSurface"));
        assert!(
            on_plane(&face_ring(0, face), axis, v),
            "wall face {face} lies on axis {axis} = {v}"
        );
        // Solid B faces 1..=4 are the roofs (z in [100,150]).
        assert_eq!(type_of(&vb[face]).as_deref(), Some("RoofSurface"));
        assert!(
            face_ring(1, face).iter().all(|c| c[2] >= 100.0 - 1e-6),
            "roof face {face} at z>=100"
        );
    }
    assert!(
        face_ring(1, 3).iter().any(|c| (c[2] - 150.0).abs() < 1e-6),
        "a roof face reaches z=150"
    );
}

// The CompositeSolid must also survive scan -> WKB encode with the right bbox
// (exercises the CompositeSolid path in geometry_to_wkb, not just Solid).
#[test]
fn citygml2_composite_solid_scans_to_correct_bbox() {
    use cityparquet::scan::scan;
    let src = Source::open(&fixture("b1_lod2_cs_w_sem.gml")).unwrap();
    let s = scan(&src).unwrap();
    assert_eq!(s.object_count, 1);
    assert_eq!(
        s.lods.iter().map(ToString::to_string).collect::<Vec<_>>(),
        ["2"]
    );
    let bbox = s.dataset_bbox.expect("bbox");
    for (got, exp) in bbox.iter().zip([0.0, 0.0, 0.0, 100.0, 100.0, 150.0]) {
        assert!((got - exp).abs() < 1e-6, "bbox {got} != {exp}");
    }
}

#[test]
fn citygml2_features_restartable() {
    let src = Source::open(&fixture("b1_lod2_s.gml")).unwrap();
    let count = || {
        src.features()
            .unwrap()
            .collect::<cityparquet::Result<Vec<_>>>()
            .unwrap()
            .len()
    };
    assert_eq!(count(), 1);
    assert_eq!(count(), 1, "features() must be restartable");
}
