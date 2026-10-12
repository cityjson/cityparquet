//! FlatCityBuf input (the `fcb` feature), over the published Delft `.fcb`
//! and the Delft CityJSONSeq of the same 3DBAG area.
#![cfg(feature = "fcb")]

use std::path::PathBuf;

use cityparquet::compare::{CompareOptions, compare_datasets};
use cityparquet::export::{ExportOptions, export};
use cityparquet::package::{ConvertOptions, convert};
use cityparquet::source::{Source, SourceFormat};

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

/// The Delft `.fcb` yields exactly as many features as the Delft
/// CityJSONSeq has lines after its header: the stream stops at the header's
/// `features_count` rather than running past it.
#[test]
fn a_flatcitybuf_file_is_read_feature_by_feature_up_to_its_count() {
    let source = Source::open(&fixture("delft.fcb")).unwrap();
    assert_eq!(source.format(), SourceFormat::FlatCityBuf);
    let seq = Source::open(&fixture("delft.city.jsonl")).unwrap();
    let count = |s: &Source| {
        s.features()
            .unwrap()
            .collect::<cityparquet::Result<Vec<_>>>()
            .unwrap()
            .len()
    };
    // The iterator stops at the header's feature count.
    assert_eq!(count(&source), count(&seq));
}

/// The FlatCityBuf source as this crate reads it, written out as CityJSONSeq
/// (its header line, then one feature per line).
fn write_as_seq(source: &Source, path: &std::path::Path) {
    let mut lines = vec![serde_json::to_string(source.header()).unwrap()];
    for feature in source.features().unwrap() {
        lines.push(serde_json::to_string(&feature.unwrap()).unwrap());
    }
    std::fs::write(path, lines.join("\n") + "\n").unwrap();
}

/// Converting the `.fcb` and exporting the package gives back the model the
/// FlatCityBuf reader hands over; and that model is the Delft CityJSONSeq's,
/// but for one documented difference of the published file: every
/// `BuildingPart` carries an LoD 0 footprint the CityJSONSeq does not.
#[test]
fn delft_from_flatcitybuf_round_trips_and_matches_the_cityjsonseq() {
    let dir = tempfile::tempdir().unwrap();
    let source = Source::open(&fixture("delft.fcb")).unwrap();
    let read = dir.path().join("read.city.jsonl");
    write_as_seq(&source, &read);

    let pkg = dir.path().join("pkg");
    convert(&ConvertOptions::new(fixture("delft.fcb"), pkg.clone())).unwrap();
    let exported = dir.path().join("exported.city.jsonl");
    export(&ExportOptions {
        package_dir: pkg,
        output: exported.clone(),
    })
    .unwrap();
    let round_trip = compare_datasets(&read, &exported, &CompareOptions::default()).unwrap();
    assert!(round_trip.equal, "{:#?}", round_trip.differences);

    // Drop the BuildingParts' LoD 0 footprints, then the FlatCityBuf model is
    // the CityJSONSeq's.
    let text = std::fs::read_to_string(&read).unwrap();
    let mut part_footprints = 0;
    let stripped: Vec<String> = text
        .lines()
        .map(|line| {
            let mut v: serde_json::Value = serde_json::from_str(line).unwrap();
            if let Some(objects) = v.get_mut("CityObjects").and_then(|o| o.as_object_mut()) {
                for object in objects.values_mut() {
                    if object["type"] != "BuildingPart" {
                        continue;
                    }
                    let geometries = object["geometry"].as_array_mut().unwrap();
                    let before = geometries.len();
                    geometries.retain(|g| g["lod"] != "0.0");
                    part_footprints += before - geometries.len();
                }
            }
            v.to_string()
        })
        .collect();
    assert_eq!(part_footprints, 1116, "one footprint per BuildingPart");
    let without = dir.path().join("without_part_lod0.city.jsonl");
    std::fs::write(&without, stripped.join("\n") + "\n").unwrap();
    let report = compare_datasets(
        &fixture("delft.city.jsonl"),
        &without,
        &CompareOptions::default(),
    )
    .unwrap();
    assert!(report.equal, "{:#?}", report.differences);
}

/// The byte layout of a FlatCityBuf file, read with `fcb_core`'s public API:
/// magic (8), header size (4), header, R-tree index, attribute index, then
/// size-prefixed features.
struct Layout {
    bytes: Vec<u8>,
    header: std::ops::Range<usize>,
    attribute_index: std::ops::Range<usize>,
    /// Each feature's range, size prefix included.
    features: Vec<std::ops::Range<usize>>,
}

fn layout(path: &std::path::Path) -> Layout {
    let bytes = std::fs::read(path).unwrap();
    let reader = fcb_core::FcbReader::open(std::io::Cursor::new(&bytes)).unwrap();
    let header = reader.header();
    let count = header.features_count() as usize;
    let rtree_len = match header.index_node_size() {
        0 => 0,
        n => fcb_core::PackedRTree::index_size(count, n),
    };
    let attr_len: usize = header
        .attribute_index()
        .map(|ai| ai.iter().map(|a| a.length() as usize).sum())
        .unwrap_or(0);
    let header_len = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    let header = 12..12 + header_len;
    let rtree = header.end..header.end + rtree_len;
    let attribute_index = rtree.end..rtree.end + attr_len;
    let mut features = Vec::new();
    let mut at = attribute_index.end;
    while at < bytes.len() {
        let len = u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
        features.push(at..at + 4 + len);
        at += 4 + len;
    }
    assert_eq!(
        at,
        bytes.len(),
        "the feature section ends at a feature boundary"
    );
    assert_eq!(features.len(), count);
    Layout {
        bytes,
        header,
        attribute_index,
        features,
    }
}

/// The byte range of a scalar field of the FlatBuffers `Header` table, found
/// through its vtable. `None` when the field is absent (its default).
fn header_field(header: &[u8], vtable_offset: u16, width: usize) -> Option<std::ops::Range<usize>> {
    let u32_at = |i: usize| u32::from_le_bytes(header[i..i + 4].try_into().unwrap());
    let u16_at = |i: usize| u16::from_le_bytes(header[i..i + 2].try_into().unwrap());
    let table = u32_at(0) as usize;
    let vtable = (table as i64
        - i32::from_le_bytes(header[table..table + 4].try_into().unwrap()) as i64)
        as usize;
    if vtable_offset >= u16_at(vtable) {
        return None;
    }
    match u16_at(vtable + vtable_offset as usize) as usize {
        0 => None,
        field => Some(table + field..table + field + width),
    }
}

/// A file whose header declares no feature count (0, "unknown" to
/// `fcb_core`) is read to the end of its feature section. No such file is
/// published, so this one is derived from the real Delft `.fcb`: its
/// `features_count` set to 0 and its R-tree index removed, since `fcb_core`
/// sizes no R-tree for a count of 0.
#[test]
fn a_file_declaring_no_feature_count_is_read_to_its_end() {
    let real = layout(&fixture("delft.fcb"));
    let mut header = real.bytes[real.header.clone()].to_vec();
    let count = header_field(&header, fcb_core::Header::VT_FEATURES_COUNT, 8)
        .expect("the real header states its feature count");
    header[count].fill(0);
    let mut derived = real.bytes[..12].to_vec();
    derived.extend_from_slice(&header);
    derived.extend_from_slice(&real.bytes[real.attribute_index.clone()]);
    derived.extend_from_slice(&real.bytes[real.features[0].start..]);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("no_count.fcb");
    std::fs::write(&path, derived).unwrap();

    let source = Source::open(&path).unwrap();
    let features = source
        .features()
        .unwrap()
        .collect::<cityparquet::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(features.len(), real.features.len());
}

/// A file whose feature section ends, cleanly at a feature boundary, before
/// the header's `features_count` is reached is an error, not a shorter
/// dataset. Derived from the real Delft `.fcb` by cutting it after its tenth
/// feature.
#[test]
fn a_file_ending_before_its_declared_feature_count_is_an_error() {
    let real = layout(&fixture("delft.fcb"));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("truncated.fcb");
    std::fs::write(&path, &real.bytes[..real.features[9].end]).unwrap();

    let source = Source::open(&path).unwrap();
    let results: Vec<_> = source.features().unwrap().collect();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 10);
    let error = results
        .iter()
        .find_map(|r| r.as_ref().err())
        .expect("the missing features are an error");
    assert!(error.to_string().contains("1115"), "{error}");
}
