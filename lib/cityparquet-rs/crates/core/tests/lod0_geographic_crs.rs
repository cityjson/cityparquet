//! LoD0 synthesis on a degree-valued CRS.
//!
//! Every threshold `Lod0Options` carries is a length in metres — `snap`
//! 1 mm, `eps_z` 1 cm, `h_step` 1.5 m, `plane_reject` 0.5 m — and the
//! predicates behind them (Newell normals, the downward-face angle, the
//! planarity deviation) all assume the three axes share one unit. A
//! geographic CRS satisfies neither: `snap` becomes a ~100 m grid, and a
//! degree of longitude is ~19% shorter than a degree of latitude at
//! Yokohama's 35.5°N, so no per-threshold rescaling can make the angle and
//! planarity tests right in x and y at once.
//!
//! So the synthesis runs in a local metric frame about the object's own
//! centroid and the result is mapped back. Nothing stored changes CRS: it is
//! an internal frame for a computation whose input and output are both in the
//! file CRS.
//!
//! The yardstick is the source's own `lod0FootPrint`, which the reader reads
//! (it is not the synthesiser's output, so the comparison is not circular): a
//! footprint synthesised from the building's LoD1 solid must come out close to
//! the footprint the surveyors published.

use std::path::PathBuf;

use cityparquet::citygml::{FeatureReader, parse_header};
use cityparquet::lod0::{Lod0Options, Point, faces_from_geometry, synthesize_lod0};
use cityparquet::wkb_write::VertexPool;
use cityparquet_schema::crs::AxisOrder;

fn data_fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(name)
}

/// Shoelace area of a ring, in whatever unit its x/y carry.
fn ring_area(ring: &[Point]) -> f64 {
    let n = ring.len();
    let mut acc = 0.0;
    for i in 0..n {
        let a = ring[i];
        let b = ring[(i + 1) % n];
        acc += a[0] * b[1] - b[0] * a[1];
    }
    acc.abs() / 2.0
}

/// Degrees to metres about `(lon0, lat0)` — the test's own yardstick, so that
/// it measures the synthesiser rather than sharing its arithmetic.
fn to_metres(p: Point, lon0: f64, lat0: f64) -> Point {
    const R: f64 = 6_378_137.0;
    let rad = std::f64::consts::PI / 180.0;
    [
        (p[0] - lon0) * R * rad * lat0.to_radians().cos(),
        (p[1] - lat0) * R * rad,
        p[2],
    ]
}

#[test]
fn a_footprint_synthesised_from_a_degree_valued_solid_matches_the_source_footprint() {
    let path = data_fixture("plateau_yokohama_bldg_fragment.gml");
    let header = parse_header(&path).unwrap();
    let mut reader = FeatureReader::open(&path, &header.transform).unwrap();
    let feature = reader.next().unwrap().unwrap();

    // EPSG:6697 is latitude-first, so the pool hands out (lon, lat, h).
    let pool = VertexPool::new(&feature.vertices, &header.transform, AxisOrder::LatLon);
    let co = feature.city_objects.values().next().unwrap();
    let geoms = co.geometry.as_ref().unwrap();

    let source_lod0 = geoms
        .iter()
        .find(|g| g.lod.as_deref() == Some("0"))
        .expect("the source lod0FootPrint");
    let solid = geoms
        .iter()
        .find(|g| g.lod.as_deref() == Some("1"))
        .expect("the source lod1Solid");

    let (truth_faces, _) = faces_from_geometry(source_lod0, &pool).unwrap();
    let (faces, mask) = faces_from_geometry(solid, &pool).unwrap();

    let centre = truth_faces[0].exterior()[0];
    let (lon0, lat0) = (centre[0], centre[1]);
    let truth: f64 = truth_faces
        .iter()
        .map(|f| {
            let ring: Vec<Point> = f
                .exterior()
                .iter()
                .map(|&p| to_metres(p, lon0, lat0))
                .collect();
            ring_area(&ring)
        })
        .sum();
    assert!(truth > 20.0, "the fixture's footprint must have real area");

    // Told that the CRS is angular — read from its declared units, never
    // guessed from the coordinates.
    let opts = Lod0Options {
        angular: true,
        ..Lod0Options::default()
    };
    let fp = synthesize_lod0(&faces, mask.as_deref(), &opts)
        .expect("a degree-valued solid must still yield a footprint");
    let got: f64 = fp
        .surfaces
        .iter()
        .map(|f| {
            let ring: Vec<Point> = f
                .exterior()
                .iter()
                .map(|&p| to_metres(p, lon0, lat0))
                .collect();
            ring_area(&ring)
        })
        .sum();

    let error = (got - truth).abs() / truth;
    assert!(
        error < 0.05,
        "synthesised {got:.1} m2 against the source's {truth:.1} m2 ({:.0}% out)",
        error * 100.0
    );
}
