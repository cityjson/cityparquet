//! Integration tests against real CityJSON data (never artificial documents).
//! Fixtures: run `just fixtures` from the workspace root first.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;

use cityparquet_schema::{AttributeInferer, CityParquetSchema, Lod, class_info, is_extension_type};
use serde_json::Value;

fn fixture(name: &str) -> PathBuf {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    assert!(
        path.exists(),
        "missing fixture {name}; run `just fixtures` first"
    );
    path
}

/// Walk every CityObject in a CityJSON / CityJSONFeature JSON value.
fn city_objects(doc: &Value) -> impl Iterator<Item = (&String, &Value)> {
    doc.get("CityObjects")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
}

struct Scan {
    types: std::collections::BTreeSet<String>,
    lods: std::collections::BTreeSet<String>,
    inferer: AttributeInferer,
    object_count: usize,
}

fn scan(docs: impl Iterator<Item = Value>) -> Scan {
    let mut s = Scan {
        types: Default::default(),
        lods: Default::default(),
        inferer: AttributeInferer::default(),
        object_count: 0,
    };
    for doc in docs {
        for (_id, obj) in city_objects(&doc) {
            s.object_count += 1;
            s.types.insert(obj["type"].as_str().unwrap().to_string());
            if let Some(attrs) = obj.get("attributes").and_then(Value::as_object) {
                for (name, value) in attrs {
                    s.inferer.observe(name, value);
                }
            }
            for geom in obj
                .get("geometry")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if let Some(lod) = geom.get("lod") {
                    // CityJSON allows string or number LoDs in the wild.
                    let text = match lod {
                        Value::String(t) => t.clone(),
                        other => other.to_string(),
                    };
                    s.lods.insert(text);
                }
            }
        }
    }
    s
}

fn jsonl_docs(name: &str) -> impl Iterator<Item = Value> {
    let reader = BufReader::new(File::open(fixture(name)).unwrap());
    reader
        .lines()
        .map(|l| serde_json::from_str::<Value>(&l.unwrap()).unwrap())
}

#[test]
fn delft_jsonl_types_lods_and_attributes_are_representable() {
    let s = scan(jsonl_docs("delft.city.jsonl"));
    assert!(s.object_count > 0, "no CityObjects found");
    // Every real type must be taxonomy-covered or an extension type.
    for t in &s.types {
        assert!(
            is_extension_type(t) || class_info(t).is_some(),
            "type {t} not covered by the CM taxonomy"
        );
    }
    // Every real LoD string must parse.
    let lods: Vec<Lod> = s.lods.iter().map(|l| Lod::parse(l).unwrap()).collect();
    assert!(!lods.is_empty());
    // Real attributes must infer to typed columns.
    let attributes = s.inferer.finish();
    assert!(
        !attributes.is_empty(),
        "expected inferred attribute columns"
    );
    // The whole scan must yield a buildable Arrow schema.
    let mut lods = lods;
    lods.sort();
    lods.dedup();
    let schema = CityParquetSchema {
        lods: lods.clone(),
        geoparquet_lods: lods.clone(),
        attributes,
        crs: None,
    }
    .to_arrow_schema()
    .unwrap();
    for lod in &lods {
        // Every LoD — including LoD0 — occupies its own suffixed column;
        // there is no un-suffixed "footprint" column (spec "Levels of detail").
        let col = cityparquet_schema::geometry_column_name("geometry", lod);
        assert!(schema.field_with_name(&col).is_ok(), "missing {col}");
    }
}

#[test]
fn lod3_railway_types_are_representable() {
    let doc: Value = serde_json::from_reader(BufReader::new(
        File::open(fixture("lod3_railway.city.json")).unwrap(),
    ))
    .unwrap();
    let s = scan(std::iter::once(doc));
    assert!(s.object_count > 0);
    assert!(
        s.types.contains("Railway"),
        "railway demo should contain Railway objects"
    );
    for t in &s.types {
        assert!(
            is_extension_type(t) || class_info(t).is_some(),
            "type {t} not covered by the CM taxonomy"
        );
    }
    for l in &s.lods {
        Lod::parse(l).unwrap_or_else(|_| panic!("LoD {l} failed to parse"));
    }
}
