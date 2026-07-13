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
