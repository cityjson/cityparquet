//! A CityGML file may declare its CRS only inside city objects.
//!
//! `scan_envelope` stopped at the first `cityObjectMember`, assuming an
//! envelope always precedes it. Freiburg's export declares
//! `urn:ogc:def:crs:EPSG::25832` 60,108 times — once per building — and never
//! in the preamble, so the scanner saw no CRS and the writer's CRS rule
//! hard-failed a file that plainly declares one.

use std::path::{Path, PathBuf};

use cityparquet::citygml::parse_header;

fn freiburg() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/freiburg_no_preamble_srs.gml")
}

#[test]
fn srs_name_is_found_when_declared_only_inside_city_objects() {
    let path = freiburg();
    let header = parse_header(&path).expect("header must parse");
    let metadata = header
        .metadata
        .expect("a file declaring a CRS must produce metadata");
    let rs = metadata
        .reference_system
        .expect("srsName inside a city object must still yield a reference system");
    let rs = serde_json::to_value(&rs).unwrap();
    assert!(
        rs.as_str().unwrap().contains("25832"),
        "expected EPSG:25832, got {rs}"
    );
}

#[test]
fn a_per_object_envelope_does_not_become_the_dataset_extent() {
    // The fallback must collect ONLY srsName. A per-object `gml:boundedBy`
    // adopted as the dataset envelope would set geographical_extent to one
    // building's extent and skew the quantisation origin.
    //
    // NOTE: this test alone does NOT exercise the guard. In this fixture the
    // first `srsName` (byte 2649) sits on the very `gml:Envelope` whose
    // `lowerCorner` follows it (byte 2721), so the fallback finds its CRS and
    // breaks before any corner is read — deleting the guard leaves this green.
    // The case that does exercise it is below.
    let header = parse_header(&freiburg()).expect("header must parse");
    assert_eq!(
        header.transform.translate,
        vec![0.0, 0.0, 0.0],
        "no preamble envelope means a zero translate, not a per-object corner"
    );
    let extent = header.metadata.and_then(|m| m.geographical_extent);
    assert!(
        extent.is_none(),
        "a per-object envelope must not become the dataset extent, got {extent:?}"
    );
}

/// The shape the guard is actually for: the first per-object envelope carries
/// corners but NO `srsName`, so the fallback runs on past them to the next
/// object's declaration. Without the `past_preamble` guard those corners
/// become the dataset envelope — one building's extent standing in for a
/// 1.86 GiB city, and a quantisation origin skewed to match.
///
/// Built by deleting the first `srsName` attribute from the real Freiburg
/// export rather than by hand: everything else is that file's own bytes, and
/// the fixture as published cannot present this shape (see the note above).
#[test]
fn corners_seen_while_hunting_for_a_late_srs_name_are_ignored() {
    let text =
        std::fs::read_to_string(freiburg()).expect("fixture must exist; run `just fixtures`");
    let first = text
        .find(r#" srsName="urn:ogc:def:crs:EPSG::25832""#)
        .expect("the fixture declares its CRS on the first object's envelope");
    let mut mutilated = text.clone();
    mutilated.replace_range(
        first..first + r#" srsName="urn:ogc:def:crs:EPSG::25832""#.len(),
        "",
    );
    assert!(
        mutilated.contains("lowerCorner"),
        "the first object's corners must survive the edit"
    );

    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("late_srs_name.gml");
    std::fs::write(&path, &mutilated).unwrap();

    let header = parse_header(&path).expect("header must parse");
    let metadata = header
        .metadata
        .expect("a later object still declares the CRS");
    let rs = serde_json::to_value(
        metadata
            .reference_system
            .expect("srsName must still be found"),
    )
    .unwrap();
    assert!(
        rs.as_str().unwrap().contains("25832"),
        "the CRS must come from the next object that declares one, got {rs}"
    );
    assert_eq!(
        header.transform.translate,
        vec![0.0, 0.0, 0.0],
        "a per-object corner must not become the quantisation origin"
    );
    assert!(
        metadata.geographical_extent.is_none(),
        "a per-object envelope must not become the dataset extent, got {:?}",
        metadata.geographical_extent
    );
}
