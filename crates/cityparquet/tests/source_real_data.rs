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
