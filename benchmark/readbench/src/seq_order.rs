//! The CityJSONSeq artefact's feature order: the SOURCE DOCUMENT'S order.
//!
//! `cjseq cat` writes one CityJSONFeature per root CityObject, but in an
//! order that differs between runs, so the stream — and the FlatCityBuf file
//! cut from it — would differ byte for byte on every rebuild. The prepare
//! chain therefore reorders the feature lines by the position at which each
//! feature's root CityObject (the feature's `id`) appears in the CityJSON
//! document's `CityObjects`, and the objects inside a feature (a root and its
//! children, also listed in a varying order) in that same document order. A
//! line already in order is kept byte for byte; the header line (the
//! `"type": "CityJSON"` metadata line) stays first.

use std::collections::HashMap;

use anyhow::{Context, Result, bail};
use serde_json::Value;

/// The `id` of a CityJSONFeature line, and the line with its `CityObjects`
/// in document order: `cjseq cat` also lists a feature's objects (a root and
/// its children) in an order that changes between runs. A line already in
/// document order is returned byte for byte; only one that is not is
/// re-serialised.
fn feature_id_and_line(line: String, position: &HashMap<&str, usize>) -> Result<(String, String)> {
    let mut v: Value = serde_json::from_str(&line).context("parsing a CityJSONSeq line")?;
    let id = match v.get("id").and_then(Value::as_str) {
        Some(id) => id.to_string(),
        None => bail!("a CityJSONSeq feature line has no string `id`"),
    };
    let Some(objects) = v.get_mut("CityObjects").and_then(Value::as_object_mut) else {
        return Ok((id, line));
    };
    let mut keyed = Vec::with_capacity(objects.len());
    for key in objects.keys() {
        match position.get(key.as_str()) {
            Some(&pos) => keyed.push((pos, key.clone())),
            None => bail!("CityObject '{key}' of feature '{id}' is not in the source document"),
        }
    }
    if keyed.windows(2).all(|w| w[0].0 < w[1].0) {
        return Ok((id, line));
    }
    keyed.sort();
    let mut ordered = serde_json::Map::with_capacity(keyed.len());
    for (_, key) in keyed {
        let value = objects.remove(&key).expect("a listed key");
        ordered.insert(key, value);
    }
    *objects = ordered;
    Ok((id, serde_json::to_string(&v)?))
}

/// Reorder `lines` (a whole CityJSONSeq stream: header first, then one
/// feature per line) so the features follow `doc_order`, the document's
/// `CityObjects` keys in document order. Fails on a feature whose root is not
/// in the document or on a root that two features claim.
pub fn order_seq_lines(doc_order: &[String], lines: Vec<String>) -> Result<Vec<String>> {
    let position: HashMap<&str, usize> = doc_order
        .iter()
        .enumerate()
        .map(|(i, id)| (id.as_str(), i))
        .collect();
    let mut lines = lines.into_iter();
    let Some(header) = lines.next() else {
        bail!("the CityJSONSeq stream is empty");
    };
    let mut keyed = Vec::new();
    for line in lines {
        let (id, line) = feature_id_and_line(line, &position)?;
        let Some(&pos) = position.get(id.as_str()) else {
            bail!("feature '{id}' has no CityObject in the source document");
        };
        keyed.push((pos, line));
    }
    keyed.sort_by_key(|(pos, _)| *pos);
    if let Some(w) = keyed.windows(2).find(|w| w[0].0 == w[1].0) {
        bail!("two features share the root '{}'", doc_order[w[0].0]);
    }
    let mut out = Vec::with_capacity(keyed.len() + 1);
    out.push(header);
    out.extend(keyed.into_iter().map(|(_, line)| line));
    Ok(out)
}

/// The `CityObjects` keys of a CityJSON document, in document order.
pub fn document_order(doc: &Value) -> Result<Vec<String>> {
    match doc.get("CityObjects").and_then(Value::as_object) {
        Some(objects) => Ok(objects.keys().cloned().collect()),
        None => bail!("the CityJSON document has no `CityObjects` object"),
    }
}
