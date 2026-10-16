//! Two properties of the CityGML 2.0 feature reader that a *measuring* caller
//! (the readbench `citygml` runner) depends on, both exercised against real /
//! committed fixtures, never inline hand-written GML.
//!
//! 1. **What the reader did not read.** A `cityObjectMember` of a type the
//!    reader does not map is skipped and tallied by element name (the
//!    `citygml2` crate's own tests prove the tally on a real file). A
//!    document whose members are all mapped must tally nothing.
//! 2. **An appearance-free open.** `FeatureReader::open` re-reads the whole
//!    document up front to index every CityModel-level `app:appearanceMember`
//!    (so a material can be applied to a building regardless of where in the
//!    file it is declared). A caller that never looks at appearance pays that
//!    second full pass for nothing, so `open_without_appearance` skips it —
//!    and must otherwise yield byte-identical features.

use std::path::PathBuf;

use cityparquet::citygml::{FeatureReader, parse_header};
use cityparquet::cjseq::CityJSONFeature;

/// A committed (in-repo) fixture under `crates/core/tests/data/`.
fn data_fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(name);
    assert!(p.exists(), "missing committed fixture {name}");
    p
}

/// Streams `path` and returns `(features, skipped tally)`.
fn read(
    path: &std::path::Path,
    with_appearance: bool,
) -> (Vec<CityJSONFeature>, Vec<(String, usize)>) {
    let header = parse_header(path).unwrap();
    let mut reader = if with_appearance {
        FeatureReader::open(path, &header.transform).unwrap()
    } else {
        FeatureReader::open_without_appearance(path, &header.transform).unwrap()
    };
    let features: Vec<_> = reader.by_ref().map(|f| f.unwrap()).collect();
    let skipped = reader
        .skipped_members()
        .iter()
        .map(|(k, v)| (k.clone(), *v))
        .collect();
    (features, skipped)
}

/// A document whose members are all mapped tallies nothing, or every valid
/// input would be refused.
#[test]
fn a_fully_mapped_document_tallies_nothing() {
    for (name, expected_features) in [
        ("railway_lod3_fragment.gml", 4),
        ("savenow_ingolstadt_lod2.gml", 3),
        ("nonbuilding_objects.gml", 2),
        ("building_with_parts.gml", 1),
    ] {
        let (features, skipped) = read(&data_fixture(name), false);
        assert_eq!(features.len(), expected_features, "{name} feature count");
        assert!(
            skipped.is_empty(),
            "{name} has only mapped member types, but the reader tallied {skipped:?}"
        );
    }
}

/// `open_without_appearance` must change exactly one thing: the appearance.
/// Ids, CityObject types, geometry boundaries and vertices — everything a
/// counting/filtering caller reads — must be identical to `open`'s, or the
/// cheaper open would be measuring a different document.
#[test]
fn open_without_appearance_changes_only_the_appearance() {
    let path = data_fixture("building_citymodel_appearance.gml");
    let (with, _) = read(&path, true);
    let (without, _) = read(&path, false);

    assert_eq!(with.len(), without.len(), "same number of features");
    assert!(
        !with.is_empty(),
        "the fixture must actually yield a feature"
    );

    for (a, b) in with.iter().zip(without.iter()) {
        assert_eq!(a.id, b.id);
        assert_eq!(a.vertices, b.vertices, "vertex pool must be identical");
        let mut a_types: Vec<_> = a
            .city_objects
            .iter()
            .map(|(k, v)| (k.clone(), String::from(v.thetype.clone())))
            .collect();
        let mut b_types: Vec<_> = b
            .city_objects
            .iter()
            .map(|(k, v)| (k.clone(), String::from(v.thetype.clone())))
            .collect();
        a_types.sort();
        b_types.sort();
        assert_eq!(a_types, b_types, "same CityObjects, same types");

        for (id, co_a) in &a.city_objects {
            let co_b = &b.city_objects[id];
            let bounds_a: Vec<_> = co_a
                .geometry
                .iter()
                .flatten()
                .map(|g| serde_json::to_value(g).unwrap()["boundaries"].clone())
                .collect();
            let bounds_b: Vec<_> = co_b
                .geometry
                .iter()
                .flatten()
                .map(|g| serde_json::to_value(g).unwrap()["boundaries"].clone())
                .collect();
            assert_eq!(bounds_a, bounds_b, "{id}: geometry boundaries must match");
        }
    }

    // ... and the appearance itself IS the difference, so the fixture is
    // proving something rather than being appearance-free to begin with.
    assert!(
        with[0].appearance.is_some(),
        "this fixture carries a CityModel-level appearance; if it stops doing \
         so the comparison above proves nothing"
    );
    assert!(
        without[0].appearance.is_none(),
        "skipping the pre-pass must actually skip the appearance"
    );
}
