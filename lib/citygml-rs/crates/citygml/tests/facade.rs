//! The facade reads and writes whichever CityGML version a document is,
//! with what the version's own crate reads and writes.

use std::path::PathBuf;

use citygml::{FeatureReader, Version, Writer, WriterOptions, header, open, sniff, sniff_from};
use cjseq::CityJSONFeature;

fn data(krate: &str, name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(krate)
        .join("tests/data")
        .join(name)
}

fn features(reader: impl Iterator<Item = citygml::Result<CityJSONFeature>>) -> Vec<String> {
    reader
        .map(|f| serde_json::to_string(&f.unwrap()).unwrap())
        .collect()
}

#[test]
fn a_document_is_read_by_the_reader_of_its_version() {
    for (krate, version) in [("citygml2", Version::V2_0), ("citygml3", Version::V3_0)] {
        let path = data(krate, "b1_lod2_cs_w_sem.gml");
        assert_eq!(sniff(&path), Some(version.clone()));
        let header = header(&path).unwrap();

        let ours = features(FeatureReader::open(&path, &header.transform).unwrap());
        let theirs = match version {
            Version::V2_0 => features(
                citygml2::FeatureReader::open(&path)
                    .unwrap()
                    .features(header.transform.clone()),
            ),
            _ => features(
                citygml3::FeatureReader::open(&path)
                    .unwrap()
                    .features(header.transform.clone()),
            ),
        };
        assert!(!ours.is_empty());
        assert_eq!(ours, theirs, "{krate}");
    }
}

#[test]
fn open_reads_the_header_and_features_header_and_open_read_apart() {
    // The PLATEAU building file has an ADE and a CityModel-level pass to make.
    let path = data("citygml2", "plateau/udx/bldg/53394519_bldg_6697_op.gml");
    let (together, reader) = open(&path).unwrap();
    let apart = header(&path).unwrap();
    assert_eq!(
        serde_json::to_string(&together).unwrap(),
        serde_json::to_string(&apart).unwrap()
    );
    assert_eq!(
        features(reader),
        features(FeatureReader::open(&path, &apart.transform).unwrap())
    );
}

#[test]
fn a_plateau_header_quantises_degrees_at_a_nano_degree_and_metres_at_a_millimetre() {
    // EPSG:6697 is latitude and longitude in degrees over a height in metres.
    for (krate, name) in [
        ("citygml2", "plateau/udx/bldg/53394519_bldg_6697_op.gml"),
        ("citygml3", "plateau/53394519_bldg_6697_op.gml"),
    ] {
        let header = header(&data(krate, name)).unwrap();
        assert_eq!(header.transform.scale, [1e-9, 1e-9, 1e-3], "{krate}");
        let metadata = header.metadata.unwrap();
        assert_eq!(
            metadata.reference_system.unwrap().to_url(),
            "https://www.opengis.net/def/crs/EPSG/0/6697"
        );
    }
}

#[test]
fn a_projected_header_quantises_every_axis_at_a_millimetre() {
    let header = header(&data("citygml2", "b1_lod2_cs_w_sem.gml")).unwrap();
    assert_eq!(header.transform.scale, [1e-3; 3]);
}

#[test]
fn a_writer_writes_the_version_it_is_given() {
    let path = data("citygml2", "b1_lod2_cs_w_sem.gml");
    let header = header(&path).unwrap();
    for version in [Version::V2_0, Version::V3_0] {
        let mut writer =
            Writer::new(&version, Vec::new(), &header, WriterOptions::default()).unwrap();
        for feature in FeatureReader::open(&path, &header.transform).unwrap() {
            writer.write_feature(&feature.unwrap()).unwrap();
        }
        let (out, report) = writer.finish().unwrap();
        assert!(report.objects > 0);
        assert_eq!(sniff_from(out.as_slice()), Some(version));
    }
}

#[test]
fn a_version_without_a_writer_is_refused() {
    let header = header(&data("citygml2", "b1_lod2_cs_w_sem.gml")).unwrap();
    let version = Version::Other("1.0".into());
    let refused = Writer::new(&version, Vec::new(), &header, WriterOptions::default());
    assert!(refused.is_err());
}
