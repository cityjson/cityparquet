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
    assert_eq!(src.header().version, cjseq::CityJSONVersion::V2_0);
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

/// Write a copy of the real `lod3_railway.city.json` with the hierarchy under
/// its `CityObjectGroup` deepened by one level: the group's first member is
/// re-parented under its second member, so the dataset runs
/// group → member → member — three levels. Only the two `parents`/`children`
/// arrays change; every object, geometry and vertex is the fixture's own.
///
/// The returned `TempDir` MUST outlive the path.
fn railway_with_a_three_level_hierarchy() -> (tempfile::TempDir, PathBuf, String) {
    let mut doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(fixture("lod3_railway.city.json")).unwrap())
            .unwrap();
    let objects = doc["CityObjects"].as_object().unwrap();
    let group = objects
        .iter()
        .find(|(_, co)| co["type"] == "CityObjectGroup")
        .map(|(id, _)| id.clone())
        .expect("railway carries a CityObjectGroup");
    let members: Vec<String> = objects[&group]["children"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    let (grandchild, new_parent) = (members[0].clone(), members[1].clone());

    // The group keeps every member but the one being pushed a level down.
    let remaining: Vec<&String> = members.iter().filter(|m| **m != grandchild).collect();
    doc["CityObjects"][&group]["children"] = serde_json::json!(remaining);
    doc["CityObjects"][&new_parent]["children"] = serde_json::json!([grandchild]);
    doc["CityObjects"][&grandchild]["parents"] = serde_json::json!([new_parent]);

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("railway_three_level.city.json");
    std::fs::write(&path, serde_json::to_string(&doc).unwrap()).unwrap();
    (dir, path, grandchild)
}

/// A CityJSON document nested more than two levels deep streams every object:
/// a feature is a top-level object with ALL of its descendants, so the
/// deepened railway document's grandchild arrives exactly once, in its
/// grandparent's feature, and none of the 121 objects is lost.
///
/// The round-trip suite is structurally blind to a lost object: `convert` →
/// `export` → `compare` reads the source through the SAME iterator on both
/// sides, so both would drop it and report equality. Only an absolute object
/// count catches it — hence the count here rather than a comparison.
#[test]
fn a_three_level_document_streams_every_object() {
    let (_dir, path, grandchild) = railway_with_a_three_level_hierarchy();
    let src = Source::open(&path).unwrap();
    let features: Vec<_> = src.features().unwrap().map(|f| f.unwrap()).collect();
    let objects: usize = features.iter().map(|f| f.city_objects.len()).sum();
    assert_eq!(objects, 121, "every object of the deepened document");
    assert_eq!(
        features
            .iter()
            .filter(|f| f.city_objects.contains_key(&grandchild))
            .count(),
        1,
        "the grandchild arrives in exactly one feature"
    );
}

/// The two-level fixture the deepened one was derived from.
#[test]
fn a_two_level_document_still_reads_every_object() {
    let src = Source::open(&fixture("lod3_railway.city.json")).unwrap();
    let objects: usize = src
        .features()
        .unwrap()
        .map(|f| f.unwrap().city_objects.len())
        .sum();
    assert_eq!(objects, 121);
}

/// A document whose child reference names an object it does not contain is an
/// error naming the missing id, not a panic.
#[test]
fn a_document_with_a_dangling_child_reference_errors_rather_than_panicking() {
    let mut doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(fixture("lod3_railway.city.json")).unwrap())
            .unwrap();
    let group = doc["CityObjects"]
        .as_object()
        .unwrap()
        .iter()
        .find(|(_, co)| co["type"] == "CityObjectGroup")
        .map(|(id, _)| id.clone())
        .unwrap();
    doc["CityObjects"][&group]["children"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!("no-such-object"));

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("railway_dangling.city.json");
    std::fs::write(&path, serde_json::to_string(&doc).unwrap()).unwrap();

    let err = Source::open(&path)
        .err()
        .expect("a child reference with no target cannot be read");
    assert!(
        err.to_string().contains("no-such-object"),
        "the error must name the missing id, got: {err}"
    );
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
