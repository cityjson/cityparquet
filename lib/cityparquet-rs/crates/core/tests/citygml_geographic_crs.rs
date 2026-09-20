//! A geographic (degree-valued) CRS end to end, on real PLATEAU data.
//!
//! `plateau_yokohama_bldg_fragment.gml` is a committed fragment of a Japanese
//! national export. It declares `EPSG:6697` — JGD2011 + JGD2011 (vertical) height,
//! whose axes are **latitude (degree), longitude (degree), height (metre)**.
//! Two properties follow from that, and neither is visible to any count-based
//! or structural check:
//!
//! 1. **The quantisation step is per axis.** A uniform millimetre scale
//!    quantises 0.001 *degree* — 90 to 111 m — so a whole city collapses onto
//!    a coarse lattice while every object, attribute and semantic surface
//!    still round-trips perfectly.
//! 2. **GeoParquet WKB is always `(x, y) = (longitude, latitude)`**, whatever
//!    axis order the authority declares. Storing 6697's own (lat, lon) order
//!    verbatim makes every GeoParquet consumer read the footprints transposed.
//!
//! Expected coordinates are hand-transcribed from the fixture's first
//! `gml:posList`, never snapshotted from the reader's own output.

use std::path::PathBuf;

use cityparquet::citygml::parse_header;
use cityparquet::package::{ConvertOptions, convert};
use cityparquet_schema::crs::{MM, NANO_DEGREE};

fn data_fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(name);
    assert!(p.exists(), "missing committed fixture {name}");
    p
}

/// The fixture's `gml:lowerCorner`, hand-transcribed: latitude, longitude,
/// height — in the document's own axis order.
const LOWER_CORNER: [f64; 3] = [35.45804463285104, 139.5998896941225, 0.0];

/// The first coordinate of the fixture's first `gml:posList`, hand-transcribed
/// in the document's own (latitude, longitude, height) order.
const FIRST_POINT: [f64; 3] = [35.465501054428856, 139.6123486302035, 0.0];

#[test]
fn the_quantisation_step_follows_the_declared_axis_units() {
    let header = parse_header(&data_fixture("plateau_yokohama_bldg_fragment.gml")).unwrap();

    // Degree, degree, metre — not [1 mm; 3].
    assert_eq!(header.transform.scale, vec![NANO_DEGREE, NANO_DEGREE, MM]);
    // The origin is still the envelope's lower corner, in document axis order.
    assert_eq!(header.transform.translate, LOWER_CORNER.to_vec());
}

#[test]
fn a_degree_coordinate_survives_the_quantiser() {
    let path = data_fixture("plateau_yokohama_bldg_fragment.gml");
    let header = parse_header(&path).unwrap();
    let scale = &header.transform.scale;
    let translate = &header.transform.translate;

    let mut reader = cityparquet::citygml::FeatureReader::open(&path, &header.transform).unwrap();
    let feature = reader.next().unwrap().unwrap();
    let v = &feature.vertices[0];
    let dequantised = [
        v[0] as f64 * scale[0] + translate[0],
        v[1] as f64 * scale[1] + translate[1],
        v[2] as f64 * scale[2] + translate[2],
    ];

    // A 1e-9 degree step is ~0.11 mm of latitude and never coarser in
    // longitude, so the quantiser must land within one step of the source.
    // (The fixture writes 17 decimals; the step, not the source, is the limit.)
    for axis in 0..2 {
        let error = (dequantised[axis] - FIRST_POINT[axis]).abs();
        assert!(
            error <= NANO_DEGREE,
            "axis {axis}: quantised to {} from {}, error {error:.3e} degrees \
             (~{:.3} m) exceeds one {NANO_DEGREE} step",
            dequantised[axis],
            FIRST_POINT[axis],
            error * 111_000.0,
        );
    }
}

#[test]
fn geoparquet_wkb_is_written_longitude_first() {
    let tmp = tempfile::tempdir().unwrap();
    let pkg = tmp.path().join("pkg");
    convert(&ConvertOptions::new(
        data_fixture("plateau_yokohama_bldg_fragment.gml"),
        pkg.clone(),
    ))
    .unwrap();

    let bounds = package_xy_bounds(&pkg);
    // Yokohama: longitude ~139.6, latitude ~35.5. Storing 6697's authority
    // order verbatim would put 35.5 in x.
    assert!(
        (139.0..140.0).contains(&bounds.0),
        "WKB x must be longitude, got {}",
        bounds.0
    );
    assert!(
        (35.0..36.0).contains(&bounds.1),
        "WKB y must be latitude, got {}",
        bounds.1
    );
}

/// `(min x, min y)` across every geometry column of every table in `pkg`,
/// read back through the WKB the package actually stores.
fn package_xy_bounds(pkg: &std::path::Path) -> (f64, f64) {
    use arrow_array::{Array, BinaryArray};
    use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

    let (mut min_x, mut min_y) = (f64::INFINITY, f64::INFINITY);
    let mut seen = false;
    for entry in std::fs::read_dir(pkg).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("parquet") {
            continue;
        }
        let file = std::fs::File::open(&path).unwrap();
        let reader = ParquetRecordBatchReaderBuilder::try_new(file)
            .unwrap()
            .build()
            .unwrap();
        for batch in reader {
            let batch = batch.unwrap();
            for (i, field) in batch.schema().fields().iter().enumerate() {
                if !field.name().starts_with("geometry_lod") {
                    continue;
                }
                let Some(col) = batch.column(i).as_any().downcast_ref::<BinaryArray>() else {
                    continue;
                };
                for row in 0..col.len() {
                    if col.is_null(row) {
                        continue;
                    }
                    let decoded = cityparquet::wkb_read::wkb_to_geometry(col.value(row)).unwrap();
                    for c in &decoded.coords {
                        seen = true;
                        min_x = min_x.min(c[0]);
                        min_y = min_y.min(c[1]);
                    }
                }
            }
        }
    }
    assert!(seen, "package carried no WKB geometry to check");
    (min_x, min_y)
}

#[test]
fn the_citygml_writer_emits_the_axis_order_its_srs_name_declares() {
    use cityparquet::citygml::writer::{WriteOptions, write_package};

    let tmp = tempfile::tempdir().unwrap();
    let pkg = tmp.path().join("pkg");
    let out_gml = tmp.path().join("out.gml");
    convert(&ConvertOptions::new(
        data_fixture("plateau_yokohama_bldg_fragment.gml"),
        pkg.clone(),
    ))
    .unwrap();
    write_package(&WriteOptions {
        package_dir: pkg,
        output: out_gml.clone(),
    })
    .unwrap();

    let gml = std::fs::read_to_string(&out_gml).unwrap();
    assert!(
        gml.contains("EPSG::6697") || gml.contains("EPSG/0/6697"),
        "the written document must declare the package's CRS"
    );

    // 6697 is (latitude, longitude): the document's own coordinates must be
    // in THAT order, not the (longitude, latitude) order the package stores.
    // The writer spells it `<gml:posList srsDimension="3">`.
    let open = gml.find("<gml:posList").expect("a posList");
    let start = open + gml[open..].find('>').unwrap() + 1;
    let end = gml[start..].find("</gml:posList>").unwrap() + start;
    let first: Vec<f64> = gml[start..end]
        .split_whitespace()
        .take(3)
        .map(|t| t.parse().unwrap())
        .collect();
    assert!(
        (35.0..36.0).contains(&first[0]),
        "first ordinate must be latitude, got {first:?}"
    );
    assert!(
        (139.0..140.0).contains(&first[1]),
        "second ordinate must be longitude, got {first:?}"
    );
}
