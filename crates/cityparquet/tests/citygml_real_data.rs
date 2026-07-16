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

/// A committed (in-repo) fixture under `crates/cityparquet/tests/data/` — small
/// real fragments with provenance headers, not fetched by `just fixtures`.
fn data_fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(name)
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

// `savenow_ingolstadt_lod2.gml` (committed fragment, CC BY 4.0): 3 real German
// LoD2 buildings, CRS EPSG:25832, with `bldg:measuredHeight`/`roofType` typed
// attributes and building-level `gen:stringAttribute`s. Per-surface generic
// attributes (inside boundedBy) must NOT attach to the building.
#[test]
fn citygml2_real_building_attributes_and_crs() {
    let src = Source::open(&data_fixture("savenow_ingolstadt_lod2.gml")).unwrap();

    // Real EPSG:25832 envelope -> advertised CRS and transform translate at the
    // envelope's lower corner.
    let rs = src
        .header()
        .metadata
        .as_ref()
        .and_then(|m| m.reference_system.as_ref())
        .expect("EPSG:25832 -> reference system");
    assert_eq!(rs.to_url(), "https://www.opengis.net/def/crs/EPSG/0/25832");
    assert!((src.header().transform.translate[0] - 675864.55).abs() < 1e-3);

    let feats: Vec<_> = src
        .features()
        .unwrap()
        .collect::<cityparquet::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(feats.len(), 3);

    let co = feats
        .iter()
        .flat_map(|f| f.city_objects.iter())
        .find(|(id, _)| id.as_str() == "DEBY_LOD2_51985910")
        .map(|(_, co)| co)
        .expect("building DEBY_LOD2_51985910");
    let attrs = co
        .attributes
        .as_ref()
        .and_then(|v| v.as_object())
        .expect("building attributes");

    // bldg: typed attributes: measuredHeight is numeric, roofType a codelist string.
    assert_eq!(
        attrs.get("measuredHeight").and_then(|v| v.as_f64()),
        Some(3.448)
    );
    assert_eq!(attrs.get("roofType").and_then(|v| v.as_str()), Some("1000"));
    // gen: generic attributes keyed by their `name`.
    assert_eq!(
        attrs.get("Gemeindeschluessel").and_then(|v| v.as_str()),
        Some("09161000")
    );
    assert_eq!(
        attrs.get("citygml_function").and_then(|v| v.as_str()),
        Some("51009_1610")
    );
    // A per-surface generic attribute (inside a boundedBy RoofSurface) must not
    // be hoisted onto the Building.
    assert!(
        !attrs.contains_key("Dachneigung"),
        "surface-level generic attribute leaked onto the building"
    );
}

#[test]
fn citygml2_attributes_infer_columns() {
    use cityparquet::scan::scan;
    use cityparquet::schema::AttributeType;

    let src = Source::open(&data_fixture("savenow_ingolstadt_lod2.gml")).unwrap();
    let s = scan(&src).unwrap();
    let cols: std::collections::BTreeMap<String, AttributeType> =
        s.schema.attributes.iter().cloned().collect();
    assert_eq!(cols.get("measuredHeight"), Some(&AttributeType::Float64));
    assert_eq!(cols.get("roofType"), Some(&AttributeType::String));
    assert_eq!(cols.get("Gemeindeschluessel"), Some(&AttributeType::String));
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

// `railway_lod3_fragment.gml`: a Building ("Chapel") whose geometry lives ONLY
// in `boundedBy` semantic surfaces' `lod3MultiSurface` — NO `lodNSolid` (the
// M4 boundedBy-MultiSurface case). Counts hand-derived from the fixture: 19
// top-level semantic surfaces (11 Wall, 3 Roof, 1 Ground, 3 OuterCeiling, 1
// OuterFloor) + 17 openings' Door/Window (1 Door, 16 Window) = 36 semantic
// surfaces; 44 boundary polygons (the Bridge/vegetation objects AFTER the
// Building are separate features and must not be counted). The two `outerBuildingInstallation`
// lod3Geometry MultiSurfaces are NOT the Building's own geometry and must not
// leak as extra geometry.
#[test]
fn citygml2_boundedby_multisurface_building_no_solid() {
    let src = Source::open(&data_fixture("railway_lod3_fragment.gml")).unwrap();
    let feats: Vec<_> = src
        .features()
        .unwrap()
        .collect::<cityparquet::Result<Vec<_>>>()
        .unwrap();
    let bldg = feats
        .iter()
        .flat_map(|f| f.city_objects.values())
        .find(|co| co.thetype == "Building")
        .expect("a Building feature");

    let geoms = bldg.geometry.as_ref().expect("building has geometry");
    assert_eq!(
        geoms.len(),
        1,
        "one MultiSurface geometry; installations must not leak"
    );
    let gv = serde_json::to_value(&geoms[0]).unwrap();
    assert_eq!(gv["type"], "MultiSurface");
    assert_eq!(gv["lod"], "3");

    let surfaces = gv["boundaries"].as_array().unwrap();
    assert_eq!(
        surfaces.len(),
        44,
        "44 boundedBy polygons as MultiSurface members"
    );

    let stypes = gv["semantics"]["surfaces"].as_array().unwrap();
    assert_eq!(stypes.len(), 36, "19 top-level surfaces + 17 Door/Window");
    let values = gv["semantics"]["values"].as_array().unwrap();
    assert_eq!(
        values.len(),
        44,
        "one semantic value per MultiSurface member"
    );

    // Semantic MAPPING (not just counts): tally each member's surface type via
    // its value index. Expected histogram is hand-derived by walking the
    // fixture's boundedBy (each gml:Polygon -> its enclosing semantic surface).
    let mut hist: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for v in values {
        let i = v.as_u64().expect("semantic value is an index") as usize;
        assert!(i < stypes.len(), "every value indexes into surfaces");
        let ty = stypes[i]["type"]
            .as_str()
            .expect("surface has a type")
            .to_string();
        *hist.entry(ty).or_default() += 1;
    }
    let expected: std::collections::BTreeMap<String, usize> = [
        ("WallSurface", 12),
        ("RoofSurface", 10),
        ("GroundSurface", 1),
        ("OuterCeilingSurface", 3),
        ("OuterFloorSurface", 1),
        ("Window", 16),
        ("Door", 1),
    ]
    .iter()
    .map(|(k, v)| (k.to_string(), *v))
    .collect();
    assert_eq!(
        hist, expected,
        "per-member semantic-type histogram must match the fixture"
    );

    // Structure: each MultiSurface member is [ring][idx] with ≥1 ring of ≥3
    // vertex indices (the convert-to-WKB path separately proves the indices
    // form valid geometry).
    for surface in surfaces {
        let rings = surface.as_array().expect("member is an array of rings");
        assert!(
            !rings.is_empty(),
            "a surface has at least one (exterior) ring"
        );
        for ring in rings {
            let idxs = ring.as_array().expect("ring is an array of indices");
            assert!(idxs.len() >= 3, "a ring has at least 3 vertices");
            assert!(
                idxs.iter().all(|i| i.as_u64().is_some()),
                "indices are numbers"
            );
        }
    }
}

// W-M4: bldg:consistsOfBuildingPart -> parent Building + BuildingPart CityObjects
// in ONE feature with children/parents links. `building_with_parts.gml` is a
// hand-authored fixture: Building "B" (own lod1Solid) + parts "B_p1" (lod2Solid)
// and "B_p2" (boundedBy-only MultiSurface).
#[test]
fn citygml2_reads_building_parts() {
    let src = Source::open(&data_fixture("building_with_parts.gml")).unwrap();
    let feats: Vec<_> = src
        .features()
        .unwrap()
        .collect::<cityparquet::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(feats.len(), 1, "one feature (the Building assembly)");
    let f = &feats[0];
    assert_eq!(f.id, "B", "feature id is the parent Building id");
    assert_eq!(f.city_objects.len(), 3, "parent + 2 parts as CityObjects");

    let parent = &f.city_objects["B"];
    assert_eq!(parent.thetype, "Building");
    let children = parent.children.as_ref().expect("parent has children");
    assert_eq!(
        children,
        &vec!["B_p1".to_string(), "B_p2".to_string()],
        "children in doc order"
    );
    assert!(
        parent.geometry.as_ref().is_some_and(|g| !g.is_empty()),
        "parent has its own lod1 geometry"
    );

    let p1 = &f.city_objects["B_p1"];
    assert_eq!(p1.thetype, "BuildingPart");
    assert_eq!(p1.parents.as_ref().unwrap(), &vec!["B".to_string()]);
    let g1 = serde_json::to_value(&p1.geometry.as_ref().unwrap()[0]).unwrap();
    assert_eq!(g1["type"], "Solid");
    assert_eq!(g1["lod"], "2");

    let p2 = &f.city_objects["B_p2"];
    assert_eq!(p2.thetype, "BuildingPart");
    assert_eq!(p2.parents.as_ref().unwrap(), &vec!["B".to_string()]);
    let g2 = serde_json::to_value(&p2.geometry.as_ref().unwrap()[0]).unwrap();
    assert_eq!(g2["type"], "MultiSurface", "boundedBy-only part");
    let stypes = g2["semantics"]["surfaces"].as_array().unwrap();
    assert_eq!(stypes.len(), 2, "WallSurface + RoofSurface");
}

// W-M5a: app:X3DMaterial appearance. `building_with_materials.gml` is a
// hand-authored fixture: Building "BM" with a lod2Solid tetrahedron (inline
// polygons p0..p3) and an app:appearance theme "visual" — red -> {p0,p1},
// green -> {p2}, p3 untargeted (null), blue an unused (target-less) definition.
#[test]
fn citygml2_reads_x3d_materials() {
    let src = Source::open(&data_fixture("building_with_materials.gml")).unwrap();
    let feats: Vec<_> = src
        .features()
        .unwrap()
        .collect::<cityparquet::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(feats.len(), 1);
    let f = &feats[0];
    let co = f.city_objects.get("BM").expect("Building BM");
    let g = &co.geometry.as_ref().expect("geometry")[0];

    // Feature-local material table: red, green and the target-less blue are all
    // interned (blue distinct so the theme-blind interner keeps it separate).
    let app = f.appearance.as_ref().expect("feature appearance");
    let mats = app.materials.as_ref().expect("materials table");
    assert_eq!(mats.len(), 3, "red, green, blue interned");
    let by_name = |n: &str| {
        mats.iter()
            .find(|m| m["name"] == serde_json::json!(n))
            .unwrap_or_else(|| panic!("material {n}"))
    };
    assert_eq!(
        by_name("red")["diffuseColor"],
        serde_json::json!([1.0, 0.0, 0.0])
    );
    assert_eq!(
        by_name("green")["diffuseColor"],
        serde_json::json!([0.0, 1.0, 0.0])
    );
    assert_eq!(
        by_name("blue")["diffuseColor"],
        serde_json::json!([0.0, 0.0, 1.0])
    );

    // Per-face material in theme "visual": [[red, red, green, null]] (Solid ->
    // [shell][face]). Dereference indices so the assertion is index-permutation
    // independent.
    let mat = g.material.as_ref().expect("geometry material");
    let visual = mat.get("visual").expect("visual theme");
    let values = visual.values.as_ref().expect("values").as_array().unwrap();
    assert_eq!(values.len(), 1, "one shell");
    let faces = values[0].as_array().unwrap();
    assert_eq!(faces.len(), 4, "four faces");
    let name_at = |face: usize| -> Option<String> {
        match &faces[face] {
            serde_json::Value::Null => None,
            v => {
                let idx = v.as_u64().unwrap() as usize;
                Some(mats[idx]["name"].as_str().unwrap().to_string())
            }
        }
    };
    assert_eq!(name_at(0).as_deref(), Some("red"), "p0 -> red");
    assert_eq!(name_at(1).as_deref(), Some("red"), "p1 -> red");
    assert_eq!(name_at(2).as_deref(), Some("green"), "p2 -> green");
    assert_eq!(name_at(3), None, "p3 untargeted -> null");
}

// W-M5b: app:ParameterizedTexture. `building_with_appearance.gml` textures ring
// p0_r0 (theme "visual"): a JPG def + per-vertex UVs (closing pair dropped).
#[test]
fn citygml2_reads_parameterized_texture() {
    let src = Source::open(&data_fixture("building_with_appearance.gml")).unwrap();
    let feats: Vec<_> = src
        .features()
        .unwrap()
        .collect::<cityparquet::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(feats.len(), 1);
    let f = &feats[0];
    let co = f.city_objects.get("BA").expect("Building BA");
    let g = &co.geometry.as_ref().expect("geometry")[0];

    let app = f.appearance.as_ref().expect("feature appearance");
    // One texture def, mapped from CityGML.
    let texs = app.textures.as_ref().expect("textures table");
    assert_eq!(texs.len(), 1, "one ParameterizedTexture");
    let tex = &texs[0];
    assert_eq!(tex["type"], serde_json::json!("JPG"), "mimeType -> type");
    assert_eq!(tex["image"], serde_json::json!("textures/wall.jpg"));
    assert_eq!(tex["wrapMode"], serde_json::json!("wrap"));
    assert_eq!(tex["textureType"], serde_json::json!("unknown"));
    assert_eq!(tex["borderColor"], serde_json::json!([0.0, 0.0, 0.0, 1.0]));

    // The UV pool holds the ring's 3 vertices (closing pair dropped).
    let uvs = app.vertices_texture.as_ref().expect("vertices-texture");
    assert_eq!(uvs.len(), 3, "3 UVs after dropping the closing pair");

    // texture.visual.values = [[ [tex, uv0, uv1, uv2], [null], [null], [null] ]]
    // (Solid -> [shell][face][ring][tex, uv…]).
    let t = g.texture.as_ref().expect("geometry texture");
    let visual = t.get("visual").expect("visual theme");
    let values = visual.values.as_ref().expect("values").as_array().unwrap();
    let faces = values[0].as_array().unwrap();
    assert_eq!(faces.len(), 4, "four faces");
    // Face 0, ring 0: [texIdx, uv0, uv1, uv2] — dereference the UVs.
    let ring0 = faces[0].as_array().unwrap()[0].as_array().unwrap();
    assert_eq!(ring0.len(), 4, "texIdx + 3 UV indices");
    let uv_at = |k: usize| -> Vec<f64> {
        let idx = ring0[k].as_u64().unwrap() as usize;
        uvs[idx].clone()
    };
    assert_eq!(uv_at(1), vec![0.0, 0.0]);
    assert_eq!(uv_at(2), vec![1.0, 0.0]);
    assert_eq!(uv_at(3), vec![0.0, 1.0]);
    // Faces 1..3 untextured -> [null].
    for (k, face) in faces.iter().enumerate().skip(1) {
        let ring = face.as_array().unwrap()[0].as_array().unwrap();
        assert_eq!(ring, &vec![serde_json::Value::Null], "face {k} untextured");
    }
}

/// CG-1: a `bldg:boundedBy` semantic surface whose geometry is
/// `gml:surfaceMember xlink:href` (not inline) must still tag the referenced
/// solid faces. `building_xlink_boundedby.gml` has a lod2Solid of inline
/// p0..p3 and Ground->p0 / Wall->{p1,p2} / Roof->p3 attached purely by xlink.
#[test]
fn citygml2_resolves_xlinked_boundedby_semantics() {
    let src = Source::open(&data_fixture("building_xlink_boundedby.gml")).unwrap();
    let feats: Vec<_> = src
        .features()
        .unwrap()
        .collect::<cityparquet::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(feats.len(), 1);
    let f = &feats[0];
    let co = f.city_objects.get("BX").expect("Building BX");
    let g = &co.geometry.as_ref().expect("geometry")[0];
    assert!(
        matches!(g.thetype, cityparquet::cjseq::GeometryType::Solid),
        "expected Solid, got {:?}",
        g.thetype
    );

    let sem = g.semantics.as_ref().expect("solid semantics");
    let surfaces = sem["surfaces"].as_array().unwrap();
    // Solid values nesting: [shell][face]; expect [Ground, Wall, Wall, Roof].
    let shell = sem["values"].as_array().unwrap()[0].as_array().unwrap();
    let type_of = |face: &serde_json::Value| -> String {
        let idx = face.as_u64().expect("non-null semantic value") as usize;
        surfaces[idx]["type"].as_str().unwrap().to_string()
    };
    let got: Vec<String> = shell.iter().map(type_of).collect();
    assert_eq!(
        got,
        vec!["GroundSurface", "WallSurface", "WallSurface", "RoofSurface"],
        "xlinked boundedBy must tag solid faces by referenced polygon id"
    );
}

/// CG-1 robustness: a boundedBy-only Building whose only member is an xlink to
/// a missing id must NOT emit a faceless MultiSurface — the Building ends up
/// with no geometry rather than an empty one.
#[test]
fn citygml2_unresolved_boundedby_xlink_emits_no_empty_geometry() {
    let src = Source::open(&data_fixture("building_broken_xlink_boundedby.gml")).unwrap();
    let feats: Vec<_> = src
        .features()
        .unwrap()
        .collect::<cityparquet::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(feats.len(), 1);
    let co = feats[0].city_objects.get("BB").expect("Building BB");
    let n = co.geometry.as_ref().map(|g| g.len()).unwrap_or(0);
    assert_eq!(
        n, 0,
        "a broken xlink must not produce an empty MultiSurface"
    );
}

/// CG-2: a solid face reached through a reversed `gml:OrientableSurface`
/// (orientation="-") has its vertices wound backwards, so its texture UVs must
/// be reversed too — while a normal face's UVs stay in authored order.
/// `building_orientable_texture.gml`: face p0 (reversed) and p1 (normal), both
/// textured with UVs (0,0)(1,0)(0,1).
#[test]
fn citygml2_reverses_uvs_for_reversed_orientable_surface() {
    let src = Source::open(&data_fixture("building_orientable_texture.gml")).unwrap();
    let feats: Vec<_> = src
        .features()
        .unwrap()
        .collect::<cityparquet::Result<Vec<_>>>()
        .unwrap();
    let f = &feats[0];
    let co = f.city_objects.get("BO").expect("Building BO");
    let g = &co.geometry.as_ref().expect("geometry")[0];
    let uvs = f
        .appearance
        .as_ref()
        .expect("appearance")
        .vertices_texture
        .as_ref()
        .expect("vertices-texture");

    let t = g.texture.as_ref().expect("geometry texture");
    let values = t
        .get("visual")
        .expect("visual theme")
        .values
        .as_ref()
        .expect("values")
        .as_array()
        .unwrap();
    let faces = values[0].as_array().unwrap(); // Solid: [shell][face]
    // Dereference a face's ring-0 UV indices to coordinate pairs.
    let face_uvs = |i: usize| -> Vec<Vec<f64>> {
        let ring0 = faces[i].as_array().unwrap()[0].as_array().unwrap();
        ring0[1..]
            .iter()
            .map(|k| uvs[k.as_u64().unwrap() as usize].clone())
            .collect()
    };
    // p0 (face 0) went through a reversed OrientableSurface -> UVs reversed.
    assert_eq!(
        face_uvs(0),
        vec![vec![0.0, 1.0], vec![1.0, 0.0], vec![0.0, 0.0]],
        "reversed OrientableSurface face must have reversed UVs"
    );
    // p1 (face 1) is a plain polygon -> UVs in authored order.
    assert_eq!(
        face_uvs(1),
        vec![vec![0.0, 0.0], vec![1.0, 0.0], vec![0.0, 1.0]],
        "normal face keeps authored UV order"
    );
}

/// CG-3: a CityModel-level `app:appearance` (direct child of CityModel, here
/// AFTER the building) must be applied to the building's faces/rings by
/// polygon/ring gml:id. `building_citymodel_appearance.gml`: material red -> p0,
/// texture -> ring p1_r0.
#[test]
fn citygml2_reads_citymodel_level_appearance() {
    let src = Source::open(&data_fixture("building_citymodel_appearance.gml")).unwrap();
    let feats: Vec<_> = src
        .features()
        .unwrap()
        .collect::<cityparquet::Result<Vec<_>>>()
        .unwrap();
    let f = &feats[0];
    let co = f.city_objects.get("BM").expect("Building BM");
    let g = &co.geometry.as_ref().expect("geometry")[0];

    // Material "red" from the CityModel-level appearance tags face 0 (p0).
    let app = f
        .appearance
        .as_ref()
        .expect("feature appearance from CityModel level");
    let mats = app.materials.as_ref().expect("materials table");
    let red = mats
        .iter()
        .position(|m| m["diffuseColor"] == serde_json::json!([1.0, 0.0, 0.0]))
        .expect("red material interned");
    let mat = g.material.as_ref().expect("geometry material");
    let mvals = mat
        .get("visual")
        .unwrap()
        .values
        .as_ref()
        .unwrap()
        .as_array()
        .unwrap();
    let mshell = mvals[0].as_array().unwrap();
    assert_eq!(mshell[0], serde_json::json!(red), "p0 tagged red");
    assert!(mshell[1].is_null(), "p1 has no material");

    // Texture from the CityModel-level appearance tags face 1's ring (p1_r0).
    let tex = g.texture.as_ref().expect("geometry texture");
    let tvals = tex
        .get("visual")
        .unwrap()
        .values
        .as_ref()
        .unwrap()
        .as_array()
        .unwrap();
    let tshell = tvals[0].as_array().unwrap();
    let face1_ring0 = tshell[1].as_array().unwrap()[0].as_array().unwrap();
    assert!(face1_ring0.len() >= 2, "p1 ring textured [tex, uv...]");
    let face0_ring0 = tshell[0].as_array().unwrap()[0].as_array().unwrap();
    assert_eq!(face0_ring0, &vec![serde_json::Value::Null], "p0 untextured");
}
