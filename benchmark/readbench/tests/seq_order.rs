//! The CityJSONSeq stage writes features in the source document's order, on
//! the real Delft fixture: a scrambled stream comes back in that order, every
//! line unchanged, and a feature foreign to the document is refused.

use std::path::PathBuf;

use cityparquet_readbench::seq_order::order_seq_lines;

fn delft_lines() -> Vec<String> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../lib/cityparquet-rs/tests/fixtures/delft.city.jsonl");
    assert!(
        p.exists(),
        "missing fixture delft.city.jsonl; run `just fixtures`"
    );
    std::fs::read_to_string(p)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect()
}

/// Every CityObject of `lines`, in stream order: the document order of a
/// stream already in document order.
fn doc_ids(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .flat_map(|l| {
            let v: serde_json::Value = serde_json::from_str(l).unwrap();
            v["CityObjects"]
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect::<Vec<_>>()
        })
        .collect()
}

fn id_of(line: &str) -> String {
    let v: serde_json::Value = serde_json::from_str(line).unwrap();
    v["id"].as_str().unwrap().to_string()
}

#[test]
fn scrambled_stream_returns_to_document_order_with_every_line_unchanged() {
    let lines = delft_lines();
    let doc_order = doc_ids(&lines[1..]);
    let mut scrambled = lines.clone();
    scrambled[1..].reverse();
    scrambled[1..].rotate_left(7);
    assert_ne!(scrambled, lines);
    let ordered = order_seq_lines(&doc_order, scrambled).unwrap();
    assert_eq!(ordered, lines);
}

#[test]
fn order_follows_the_document_not_the_stream() {
    let lines = delft_lines();
    let mut roots: Vec<String> = lines[1..].iter().map(|l| id_of(l)).collect();
    roots.reverse();
    let mut doc_order = roots.clone();
    doc_order.extend(
        doc_ids(&lines[1..])
            .into_iter()
            .filter(|id| !roots.contains(id)),
    );
    let ordered = order_seq_lines(&doc_order, lines.clone()).unwrap();
    assert_eq!(ordered[0], lines[0], "the header stays first");
    let ids: Vec<String> = ordered[1..].iter().map(|l| id_of(l)).collect();
    assert_eq!(ids, roots);
}

#[test]
fn a_feature_missing_from_the_document_is_refused() {
    let lines = delft_lines();
    let doc_order = doc_ids(&lines[2..]);
    let err = order_seq_lines(&doc_order, lines).unwrap_err().to_string();
    assert!(err.contains("source document"), "{err}");
}

#[test]
fn a_features_objects_follow_the_document_order() {
    let lines = delft_lines();
    let feature: serde_json::Value = serde_json::from_str(&lines[1]).unwrap();
    let mut keys: Vec<String> = feature["CityObjects"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    let mut doc_order = doc_ids(&lines[1..]);
    // The feature's own objects, listed in the document in reverse.
    keys.reverse();
    doc_order.retain(|id| !keys.contains(id));
    doc_order.extend(keys.iter().cloned());
    let ordered = order_seq_lines(&doc_order, vec![lines[0].clone(), lines[1].clone()]).unwrap();
    let out: serde_json::Value = serde_json::from_str(&ordered[1]).unwrap();
    let got: Vec<String> = out["CityObjects"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    assert_eq!(got, keys);
    assert_eq!(
        out["vertices"], feature["vertices"],
        "the geometry is untouched"
    );
}
