//! A non-2.0 CityGML document must fail with a CityGML *version* error.
//!
//! Before this, `is_citygml` returned false for any non-2.0 namespace, so the
//! file fell through to the CityJSON branch and reported
//! `invalid CityJSON: expected value at line 1 column 1` — a JSON parse error
//! for an XML file. That message sent every reader hunting the wrong problem,
//! and it is what 11 collections of the City3D catalogue hit.

use std::path::Path;

use cityparquet::source::Source;

#[test]
fn citygml_1_0_reports_a_version_error_not_a_json_error() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/berlin_citygml1.gml");
    // `Source` is not `Debug`, so discard the Ok value before `expect_err`.
    let err = Source::open(&path)
        .map(|_| ())
        .expect_err("CityGML 1.0 must not open as a source");
    let msg = err.to_string();
    assert!(
        msg.contains("unsupported CityGML version"),
        "error must name the CityGML version problem, got: {msg}"
    );
    assert!(
        msg.contains("1.0"),
        "error must name the detected version, got: {msg}"
    );
    assert!(
        !msg.contains("invalid CityJSON"),
        "must NOT report a JSON parse error for an XML file, got: {msg}"
    );
}
