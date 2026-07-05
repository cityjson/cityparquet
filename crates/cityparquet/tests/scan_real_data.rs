use std::path::PathBuf;

use cityparquet::scan::scan;
use cityparquet::source::Source;

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

#[test]
fn delft_scan_matches_known_content() {
    let src = Source::open(&fixture("delft.city.jsonl")).unwrap();
    let s = scan(&src).unwrap();
    assert_eq!(s.object_count, 2231);
    let lod_strings: Vec<String> = s.lods.iter().map(ToString::to_string).collect();
    assert_eq!(lod_strings, ["0", "1.2", "1.3", "2.2"]);
    // Recounted against the fixture with python3 (see report): 50 distinct
    // attribute names, not the 47 in the original brief.
    assert_eq!(s.schema.attributes.len(), 50);
    let meta = s.metadata(&[]).unwrap();
    assert_eq!(meta.default_geometry, "geometry_lod2_2");
    assert_eq!(meta.bbox_column, "bbox");
    assert!(meta.crs.is_some());
    let arrow = s.schema.to_arrow_schema().unwrap();
    assert!(arrow.field_with_name("geometry_lod0").is_ok());
}

#[test]
fn railway_scan_is_representable() {
    let src = Source::open(&fixture("lod3_railway.city.json")).unwrap();
    let s = scan(&src).unwrap();
    assert_eq!(s.object_count, 121);
    assert!(!s.lods.is_empty());
    assert!(s.schema.to_arrow_schema().is_ok());
}
