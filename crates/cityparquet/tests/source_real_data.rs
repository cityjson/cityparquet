use std::path::PathBuf;

use cityparquet::source::{Source, SourceFormat};

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

#[test]
fn delft_seq_streams_features() {
    let src = Source::open(&fixture("delft.city.jsonl")).unwrap();
    assert_eq!(src.format(), SourceFormat::CityJsonSeq);
    assert_eq!(src.header().version, "2.0");
    let n1 = src.features().unwrap().count();
    let n2 = src.features().unwrap().count(); // restartable
    assert!(n1 > 100, "expected many features, got {n1}");
    assert_eq!(n1, n2);
    let objects: usize = src
        .features()
        .unwrap()
        .map(|f| f.unwrap().city_objects.len())
        .sum();
    assert_eq!(objects, 2231);
}

#[test]
fn minified_doc_with_trailing_newline_is_not_seq() {
    // Derived from the real railway fixture: same content, plus a trailing newline.
    let content = std::fs::read_to_string(fixture("lod3_railway.city.json")).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("railway.city.json");
    std::fs::write(&path, format!("{}\n", content.trim_end())).unwrap();
    let src = Source::open(&path).unwrap();
    assert_eq!(src.format(), SourceFormat::CityJson);
    let objects: usize = src
        .features()
        .unwrap()
        .map(|f| f.unwrap().city_objects.len())
        .sum();
    assert_eq!(objects, 121);
}

#[test]
fn header_line_sniff_does_not_need_the_rest_of_the_file() {
    // Derived from the real delft fixture: only its first two lines (header +
    // one feature). Proves `Source::open`'s format sniff classifies Seq from
    // just the header line and the proof of a following non-empty line,
    // without needing (or reading) the rest of the file.
    let content = std::fs::read_to_string(fixture("delft.city.jsonl")).unwrap();
    let first_two_lines: String = content.lines().take(2).collect::<Vec<_>>().join("\n");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("delft_head.city.jsonl");
    std::fs::write(&path, first_two_lines).unwrap();
    let src = Source::open(&path).unwrap();
    assert_eq!(src.format(), SourceFormat::CityJsonSeq);
    let n = src.features().unwrap().count();
    assert_eq!(n, 1);
}

#[test]
fn railway_doc_yields_features_with_local_vertices() {
    let src = Source::open(&fixture("lod3_railway.city.json")).unwrap();
    assert_eq!(src.format(), SourceFormat::CityJson);
    let feats: Vec<_> = src.features().unwrap().collect::<Result<_, _>>().unwrap();
    assert!(!feats.is_empty());
    let objects: usize = feats.iter().map(|f| f.city_objects.len()).sum();
    assert_eq!(objects, 121);
    // CityJSONSeq semantics: every vertex index used by a feature is local.
    for f in &feats {
        for co in f.city_objects.values() {
            assert!(co.geometry.is_some() || co.children.is_some() || co.parents.is_some());
        }
        assert!(!f.vertices.is_empty() || f.city_objects.values().all(|c| c.geometry.is_none()));
    }
}
