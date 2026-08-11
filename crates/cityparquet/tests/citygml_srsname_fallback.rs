//! A CityGML file may declare its CRS only inside city objects.
//!
//! `scan_envelope` stopped at the first `cityObjectMember`, assuming an
//! envelope always precedes it. Freiburg's export declares
//! `urn:ogc:def:crs:EPSG::25832` 60,108 times — once per building — and never
//! in the preamble, so the scanner saw no CRS and the writer's CRS rule
//! hard-failed a file that plainly declares one.

use std::path::Path;

use cityparquet::citygml::parse_header;

#[test]
fn srs_name_is_found_when_declared_only_inside_city_objects() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/freiburg_no_preamble_srs.gml");
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
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/freiburg_no_preamble_srs.gml");
    let header = parse_header(&path).expect("header must parse");
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
