//! A non-building object's solid may compose polygons defined in its own
//! `boundedBy` semantic surfaces.
//!
//! CityGML 2.0 builds a `lodNSolid` out of `gml:surfaceMember xlink:href`
//! references to polygons that live wherever the schema allows them, and for
//! every `_CityObject` with boundary surfaces that is inside `boundedBy`.
//! `read_abstract_building` already reads `bldg:boundedBy`; the generic reader
//! that serves Bridge, WaterBody, Road, CityFurniture and the rest consumed
//! such a property as an unrecognised structural element and dropped its
//! contents, so every one of the solid's references dangled and the conversion
//! aborted — one object stopping a whole corpus.
//!
//! The polygons are the solid's own faces, so they are never emitted again as a
//! second, standalone MultiSurface. Their **semantic types travel with them**:
//! a `brid:OuterFloorSurface` is as much a semantic surface as a
//! `bldg:RoofSurface`, and reading it needs nothing bridge-specific — only
//! that the surface be matched in the object's own module namespace rather
//! than the Building module's.

use std::path::PathBuf;

use cityparquet::citygml::{FeatureReader, parse_header};

fn data_fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(name);
    assert!(p.exists(), "missing committed fixture {name}");
    p
}

#[test]
fn a_solid_resolves_xlinks_into_its_own_bounded_by_surfaces() {
    let path = data_fixture("plateau_yokohama_brid_fragment.gml");
    let header = parse_header(&path).unwrap();
    let mut reader = FeatureReader::open(&path, &header.transform).unwrap();

    let feature = reader
        .next()
        .expect("the fragment holds one Bridge")
        .expect("it must read, not error on a dangling reference");
    let co = feature
        .city_objects
        .values()
        .next()
        .expect("one city object");
    assert_eq!(co.thetype, "Bridge");

    let geometries = co.geometry.as_ref().expect("the Bridge's lod2Solid");
    // ONE geometry: the solid. The boundary polygons are its faces, not a
    // second MultiSurface beside it.
    assert_eq!(
        geometries.len(),
        1,
        "expected only the solid, got {:?}",
        geometries
            .iter()
            .map(|g| (g.thetype.clone(), g.lod.clone()))
            .collect::<Vec<_>>()
    );
    let solid = &geometries[0];
    assert_eq!(solid.lod.as_deref(), Some("2"));

    // The fixture's solid names exactly 21 `surfaceMember`s, hand-counted from
    // the source, and every one is defined inside the member's own
    // `brid:boundedBy`.
    let shells: Vec<Vec<Vec<Vec<usize>>>> =
        serde_json::from_value(solid.boundaries.clone()).expect("Solid shape");
    let faces: usize = shells.iter().map(Vec::len).sum();
    assert_eq!(faces, 21, "every boundary polygon must reach the solid");

    // …and each face carries the type of the `brid:boundedBy` surface it came
    // from. Counted from the fixture by hand: 11 WallSurface, 6
    // OuterFloorSurface, 3 OuterCeilingSurface, 1 GroundSurface.
    let semantics = solid
        .semantics
        .as_ref()
        .expect("the solid's faces carry their boundedBy types");
    let types: Vec<String> = semantics["surfaces"]
        .as_array()
        .expect("a surfaces array")
        .iter()
        .map(|s| s["type"].as_str().unwrap_or_default().to_string())
        .collect();
    let mut tally: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    let values = semantics["values"].as_array().expect("a values tree");
    for shell in values {
        for face in shell.as_array().expect("one entry per face") {
            let idx = face.as_u64().expect("every face is tagged") as usize;
            *tally.entry(types[idx].as_str()).or_default() += 1;
        }
    }
    assert_eq!(
        tally,
        [
            ("GroundSurface", 1),
            ("OuterCeilingSurface", 3),
            ("OuterFloorSurface", 6),
            ("WallSurface", 11),
        ]
        .into_iter()
        .collect()
    );
}

#[test]
fn an_unresolvable_solid_reference_names_the_object() {
    // The abort is deliberate — a solid that quietly loses a face is not the
    // shape it claims to be, and this reader is the geometry-resolution
    // oracle. What the message owes an operator is somewhere to look: on a
    // 1 798-file national corpus the polygon id alone means grepping 27 GB.
    let path = data_fixture("generic_object_broken_solid_xlink.gml");
    let header = parse_header(&path).unwrap();
    let mut reader = FeatureReader::open(&path, &header.transform).unwrap();
    let err = reader
        .next()
        .expect("a feature is attempted")
        .expect_err("a dangling solid reference is fatal");
    let message = err.to_string();
    for needle in ["#missing", "the-bridge", "Bridge"] {
        assert!(
            message.contains(needle),
            "{needle:?} missing from: {message}"
        );
    }
}

#[test]
fn a_bridge_emits_its_construction_elements_as_child_objects() {
    // `brid:outerBridgeConstruction` is the bridge counterpart of
    // `bldg:outerBuildingInstallation`: a 2nd-level CityObject, not geometry
    // of the parent. CityGML 2.0 spells the type `BridgeConstructionElement`
    // and CityJSON spells it `BridgeConstructiveElement`, so the reader remaps
    // it the way it already remaps `TransportSquare` to `Square`.
    let path = data_fixture("plateau_yokohama_brid_children_fragment.gml");
    let header = parse_header(&path).unwrap();
    let mut reader = FeatureReader::open(&path, &header.transform).unwrap();
    let feature = reader.next().expect("one member").expect("it must read");

    let mut by_type: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for co in feature.city_objects.values() {
        *by_type.entry(co.thetype.as_str()).or_default() += 1;
    }
    assert_eq!(
        by_type,
        [("Bridge", 1), ("BridgeConstructiveElement", 3)]
            .into_iter()
            .collect(),
        "the fixture's Bridge has three construction elements"
    );

    // The parent names them as children, and each names the parent — the same
    // contract `encode` checks for a Building and its installations.
    let (parent_id, parent) = feature
        .city_objects
        .iter()
        .find(|(_, co)| co.thetype == "Bridge")
        .expect("the Bridge");
    let children = parent.children.as_ref().expect("the Bridge lists children");
    assert_eq!(children.len(), 3);
    for child_id in children {
        let child = &feature.city_objects[child_id];
        assert_eq!(child.thetype, "BridgeConstructiveElement");
        assert_eq!(
            child.parents.as_deref(),
            Some(std::slice::from_ref(parent_id)),
            "every child names the Bridge as its parent"
        );
    }
}

#[test]
fn a_parent_solid_resolves_a_face_defined_inside_a_child() {
    // A child that the type map recognises becomes a CityObject of its own,
    // and its polygons go to its own registry — but the parent's solid may
    // still name them, and CityGML says it may: the construction element IS
    // that face of the bridge. Registering them as the parent's xlink targets
    // resolves the reference; it emits nothing, so the face is not duplicated.
    let path = data_fixture("bridge_solid_xlinks_into_child.gml");
    let header = parse_header(&path).unwrap();
    let mut reader = FeatureReader::open(&path, &header.transform).unwrap();
    let feature = reader
        .next()
        .expect("one member")
        .expect("the parent's reference into its child must resolve");

    let bridge = feature
        .city_objects
        .values()
        .find(|co| co.thetype == "Bridge")
        .expect("the Bridge");
    let solid = bridge
        .geometry
        .as_ref()
        .and_then(|g| {
            g.iter()
                .find(|g| g.thetype == cityparquet::cjseq::GeometryType::Solid)
        })
        .expect("its lod2Solid");
    let shells: Vec<Vec<Vec<Vec<usize>>>> =
        serde_json::from_value(solid.boundaries.clone()).unwrap();
    assert_eq!(shells.iter().map(Vec::len).sum::<usize>(), 1);

    // The child keeps its own geometry; the shared face is not moved.
    let child = feature
        .city_objects
        .values()
        .find(|co| co.thetype == "BridgeConstructiveElement")
        .expect("the construction element");
    assert!(
        child.geometry.as_ref().is_some_and(|g| !g.is_empty()),
        "the child still owns what it declares"
    );
}

#[test]
fn nesting_deeper_than_the_limit_is_an_error_not_a_crash() {
    // Recursion through child objects has to be bounded like
    // `consistsOfBuildingPart` already is; without a limit a pathological
    // document takes the process down with a stack overflow rather than an
    // error. Generated rather than committed: the point is the depth.
    let depth = 200;
    let mut xml = String::from(
        r#"<?xml version="1.0" encoding="utf-8"?>
<CityModel xmlns:brid="http://www.opengis.net/citygml/bridge/2.0"
           xmlns:gml="http://www.opengis.net/gml"
           xmlns="http://www.opengis.net/citygml/2.0">
<cityObjectMember><brid:Bridge gml:id="b0">"#,
    );
    for _ in 0..depth {
        xml.push_str("<brid:consistsOfBridgePart><brid:BridgePart>");
    }
    for _ in 0..depth {
        xml.push_str("</brid:BridgePart></brid:consistsOfBridgePart>");
    }
    xml.push_str("</brid:Bridge></cityObjectMember></CityModel>");

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("deep.gml");
    std::fs::write(&path, xml).unwrap();

    let header = parse_header(&path).unwrap();
    let mut reader = FeatureReader::open(&path, &header.transform).unwrap();
    let err = reader
        .next()
        .expect("a feature is attempted")
        .expect_err("nesting past the limit must be refused");
    assert!(err.to_string().contains("nested deeper than"), "got: {err}");
}

#[test]
fn one_lod_yields_one_geometry_and_it_is_the_semantic_one() {
    // `encode` keeps the FIRST geometry per object and LoD, so emitting the
    // standalone MultiSurface beside the boundedBy one would not add
    // information — it would throw the semantics away.
    let path = data_fixture("bridge_boundedby_and_standalone_ms.gml");
    let header = parse_header(&path).unwrap();
    let mut reader = FeatureReader::open(&path, &header.transform).unwrap();
    let feature = reader.next().expect("one member").expect("it must read");
    let co = feature.city_objects.values().next().unwrap();
    let geoms = co.geometry.as_ref().expect("geometry");

    let lod2: Vec<_> = geoms
        .iter()
        .filter(|g| g.lod.as_deref() == Some("2"))
        .collect();
    assert_eq!(lod2.len(), 1, "one geometry per LoD");
    assert!(
        lod2[0].semantics.is_some(),
        "and it is the one carrying the boundedBy types"
    );
}
