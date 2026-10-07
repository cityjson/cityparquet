//! `FeatureReader::from_reader`, `sniff_citygml_from` and `parse_header_from`:
//! CityGML read from any `BufRead` (an HTTP body stream, say) rather than a
//! path, on the committed real fixtures.

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use cityparquet::citygml::{
    CityGmlVersion, FeatureReader, parse_header, parse_header_from, sniff_citygml,
    sniff_citygml_from,
};

fn data_fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(name)
}

/// Every feature's id and its CityObjects' ids, in stream order.
fn ids(reader: FeatureReader) -> Vec<(String, Vec<String>)> {
    reader
        .map(|f| {
            let f = f.expect("feature");
            let mut cos: Vec<String> = f.city_objects.keys().cloned().collect();
            cos.sort();
            (f.id, cos)
        })
        .collect()
}

/// A `Read` that counts every byte it hands out, readable after it is dropped.
struct Counting<R> {
    inner: R,
    count: Arc<AtomicU64>,
}

impl<R: Read> Read for Counting<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.count.fetch_add(n as u64, Ordering::Relaxed);
        Ok(n)
    }
}

#[test]
fn a_reader_built_from_a_bufread_yields_the_same_features_as_the_path_reader() {
    for name in [
        "savenow_ingolstadt_lod2.gml",
        "plateau_yokohama_bldg_fragment.gml",
    ] {
        let path = data_fixture(name);
        let header = parse_header(&path).unwrap();
        let by_path = FeatureReader::open_without_appearance(&path, &header.transform).unwrap();
        let file = BufReader::new(File::open(&path).unwrap());
        let by_reader = FeatureReader::from_reader(Box::new(file), &header.transform).unwrap();
        let (a, b) = (ids(by_path), ids(by_reader));
        assert!(!a.is_empty(), "{name}: no features");
        assert_eq!(a, b, "{name}");
    }
}

#[test]
fn the_reader_based_sniff_and_header_agree_with_the_path_based_ones() {
    let path = data_fixture("savenow_ingolstadt_lod2.gml");
    let file = || BufReader::new(File::open(&path).unwrap());
    assert_eq!(sniff_citygml_from(file()), Some(CityGmlVersion::V2_0));
    assert_eq!(sniff_citygml_from(file()), sniff_citygml(&path));
    let (a, b) = (
        parse_header(&path).unwrap(),
        parse_header_from(file()).unwrap(),
    );
    assert_eq!(a.transform.scale, b.transform.scale);
    assert_eq!(a.transform.translate, b.transform.translate);
}

#[test]
fn a_reader_dropped_after_the_first_feature_does_not_read_the_rest() {
    let path = data_fixture("savenow_ingolstadt_lod2.gml");
    let len = std::fs::metadata(&path).unwrap().len();
    let header = parse_header(&path).unwrap();
    let count = Arc::new(AtomicU64::new(0));
    let counting = Counting {
        inner: File::open(&path).unwrap(),
        count: Arc::clone(&count),
    };
    let mut reader = FeatureReader::from_reader(
        Box::new(BufReader::with_capacity(512, counting)),
        &header.transform,
    )
    .unwrap();
    reader.next().expect("one feature").expect("parses");
    drop(reader);
    let read = count.load(Ordering::Relaxed);
    assert!(read > 0);
    assert!(
        read < len / 2,
        "read {read} of {len} bytes for the first of several features"
    );
}
