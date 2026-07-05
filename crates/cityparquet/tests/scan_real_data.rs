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
fn extensions_declarations_reach_metadata() {
    // Derived from the real railway fixture (same precedent as the Task 2
    // sniff test): same content, plus a realistic extensions declaration —
    // the shipped fixture's own `extensions` key is an empty object.
    let mut doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(fixture("lod3_railway.city.json")).unwrap())
            .unwrap();
    doc["extensions"] = serde_json::json!({
        "Noise": {
            "url": "https://www.cityjson.org/extensions/download/noise.ext.json",
            "version": "1.1.0"
        }
    });
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("railway_noise.city.json");
    std::fs::write(&path, serde_json::to_string(&doc).unwrap()).unwrap();

    let src = Source::open(&path).unwrap();
    let s = scan(&src).unwrap();
    let meta = s.metadata(&[]).unwrap();
    let extensions = meta
        .extensions
        .expect("extensions declaration must survive the scan");
    assert!(
        extensions.get("Noise").is_some(),
        "expected the Noise declaration in {extensions}"
    );

    // The delft header carries no extensions key at all; that absence must be
    // preserved as None, not fabricated.
    let delft = Source::open(&fixture("delft.city.jsonl")).unwrap();
    let delft_meta = scan(&delft).unwrap().metadata(&[]).unwrap();
    assert!(delft_meta.extensions.is_none());
}

#[test]
fn railway_scan_is_representable() {
    let src = Source::open(&fixture("lod3_railway.city.json")).unwrap();
    let s = scan(&src).unwrap();
    assert_eq!(s.object_count, 121);
    assert!(!s.lods.is_empty());
    assert!(s.schema.to_arrow_schema().is_ok());
}
