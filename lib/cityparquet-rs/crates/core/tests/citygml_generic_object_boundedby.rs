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
//! The polygons are harvested as **xlink targets only**: they are the solid's
//! own faces, and emitting them again as a second, standalone MultiSurface
//! would double the object's geometry.

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
