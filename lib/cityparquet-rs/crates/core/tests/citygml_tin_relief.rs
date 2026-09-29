//! A `dem:ReliefFeature`'s TIN component is a CityJSON `TINRelief`.
//!
//! CityGML 2.0 wraps a terrain in two features: a `dem:ReliefFeature`
//! aggregating one or more `dem:reliefComponent`s, of which a `dem:TINRelief`
//! holds its triangles in `dem:tin` — a `gml:TriangulatedSurface` (or
//! `gml:Tin`) of `gml:Triangle` patches. CityJSON flattens this into one
//! `TINRelief` City Object per TIN component, whose geometry can only be a
//! `CompositeSurface` of triangles (CityJSON 2.0 §2.11).
//!
//! Counts and coordinates are hand-transcribed from the fixture, not
//! snapshotted from the reader.

use std::path::PathBuf;

use cityparquet::citygml::{FeatureReader, parse_header};
use cityparquet::export::{ExportOptions, export};
use cityparquet::package::{ConvertOptions, convert};
use serde_json::Value;

fn data_fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(name);
    assert!(p.exists(), "missing committed fixture {name}");
    p
}

const FIXTURE: &str = "montreal_tin_relief_fragment.gml";

/// The `gml:id` of the fixture's `dem:TINRelief` component.
const TIN_ID: &str = "UUID_eed63079-f1a9-4ca8-980c-97e45aeda5be";

/// The first triangle's three corners, in document order.
const FIRST_TRIANGLE: [[f64; 3]; 3] = [
    [295554.37, 5038697.35, 125.23],
    [295551.806739, 5038696.437653, 123.752867],
    [295552.228443, 5038696.169388, 123.815941],
];

#[test]
fn a_tin_relief_component_is_read_as_a_tinrelief_of_triangles() {
    let path = data_fixture(FIXTURE);
    let header = parse_header(&path).unwrap();
    let scale = header.transform.scale.clone();
    let translate = header.transform.translate.clone();
    let mut reader = FeatureReader::open(&path, &header.transform).unwrap();
    let feature = reader
        .next()
        .expect("the relief must be read")
        .expect("and read cleanly");
    assert!(reader.next().is_none(), "one member, one feature");
    assert!(
        reader.skipped_members().is_empty(),
        "a mapped relief is not a skipped member: {:?}",
        reader.skipped_members()
    );

    assert_eq!(feature.city_objects.len(), 1);
    let (id, tin) = feature.city_objects.iter().next().unwrap();
    assert_eq!(
        id, TIN_ID,
        "the TIN component's own gml:id names the object"
    );
    assert_eq!(tin.thetype, "TINRelief");
    assert!(
        tin.attributes
            .as_ref()
            .is_none_or(|a| a.as_object().is_none_or(|m| m.is_empty())),
        "`dem:lod` is the geometry's LoD, not an attribute: {:?}",
        tin.attributes
    );

    let geoms = tin.geometry.as_ref().expect("geometry");
    assert_eq!(geoms.len(), 1, "one TIN, one geometry");
    let g = &geoms[0];
    assert_eq!(
        serde_json::to_value(&g.thetype).unwrap(),
        Value::from("CompositeSurface"),
        "CityJSON allows a TINRelief only a CompositeSurface"
    );
    assert_eq!(g.lod.as_deref(), Some("2"));

    let faces: Vec<Vec<Vec<usize>>> = serde_json::from_value(g.boundaries.clone()).unwrap();
    assert_eq!(faces.len(), 3, "the fixture keeps three triangles");
    for face in &faces {
        assert_eq!(face.len(), 1, "a triangle has no interior ring");
        assert_eq!(face[0].len(), 3, "three corners, closure stripped");
    }
    // The second triangle reuses the first's first and third corners.
    assert_eq!(faces[1][0][1], faces[0][0][0]);
    assert_eq!(faces[1][0][2], faces[0][0][2]);

    let corner = |i: usize| -> [f64; 3] {
        let v = &feature.vertices[i];
        [
            v[0] as f64 * scale[0] + translate[0],
            v[1] as f64 * scale[1] + translate[1],
            v[2] as f64 * scale[2] + translate[2],
        ]
    };
    for (k, expected) in FIRST_TRIANGLE.iter().enumerate() {
        let got = corner(faces[0][0][k]);
        for axis in 0..3 {
            assert!(
                (got[axis] - expected[axis]).abs() < 1e-3,
                "corner {k} axis {axis}: {} vs {}",
                got[axis],
                expected[axis]
            );
        }
    }
}

#[test]
fn a_tin_relief_converts_into_relief_parquet_and_exports_as_a_composite_surface() {
    let tmp = tempfile::tempdir().unwrap();
    let pkg = tmp.path().join("pkg");
    let mut opts = ConvertOptions::new(data_fixture(FIXTURE), pkg.clone());
    opts.crs_override = Some("EPSG:2950".to_string());
    let report = convert(&opts).unwrap();
    assert_eq!(report.object_count, 1);
    assert!(
        pkg.join("relief.parquet").is_file(),
        "a TINRelief belongs to the Relief module's table"
    );

    let out = tmp.path().join("back.city.jsonl");
    export(&ExportOptions {
        package_dir: pkg,
        output: out.clone(),
    })
    .unwrap();
    let text = std::fs::read_to_string(&out).unwrap();
    let feature: Value = serde_json::from_str(text.lines().nth(1).expect("one feature")).unwrap();
    let tin = &feature["CityObjects"][TIN_ID];
    assert_eq!(tin["type"], "TINRelief");
    let g = &tin["geometry"][0];
    assert_eq!(g["type"], "CompositeSurface");
    // Export spells every LoD in its canonical `major.minor` form.
    assert_eq!(g["lod"], "2.0");
    assert_eq!(g["boundaries"].as_array().unwrap().len(), 3);
}

#[test]
fn each_tin_component_is_a_tinrelief_and_other_components_are_reported_unmapped() {
    let path = data_fixture("relief_feature_mixed_components.gml");
    let header = parse_header(&path).unwrap();
    let reader = FeatureReader::open(&path, &header.transform).unwrap();
    let mut reader = reader;
    let features: Vec<_> = reader.by_ref().map(|f| f.unwrap()).collect();

    // Two TIN components, two 1st-level TINRelief objects, one per feature.
    let ids: Vec<&str> = features.iter().map(|f| f.id.as_str()).collect();
    assert_eq!(
        ids,
        [TIN_ID, "UUID_3b70702a-06ca-485b-89de-30b6891b4537"],
        "each TIN component in document order, named by its own gml:id"
    );
    for f in &features {
        assert_eq!(f.city_objects.len(), 1);
        let tin = &f.city_objects[&f.id];
        assert_eq!(tin.thetype, "TINRelief");
        let g = &tin.geometry.as_ref().unwrap()[0];
        let faces: Vec<Vec<Vec<usize>>> = serde_json::from_value(g.boundaries.clone()).unwrap();
        assert_eq!(faces.len(), 3);
    }

    // The mass points have no CityJSON type: tallied, by their own name, so a
    // caller can tell the document was not read whole.
    let skipped: Vec<(&str, usize)> = reader
        .skipped_members()
        .iter()
        .map(|(k, v)| (k.as_str(), *v))
        .collect();
    assert_eq!(skipped, [("dem:MassPointRelief", 1)]);
}
