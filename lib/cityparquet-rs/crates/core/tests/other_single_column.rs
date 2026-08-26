//! `other` is the single escape hatch: one column, one reader rule — every
//! entry is restored into the object's `attributes` on export.

use cityparquet::export::{ExportOptions, export};
use cityparquet::package::{ConvertOptions, convert};

/// A source attribute whose name collides with a reserved column survives a
/// full round trip, back inside `attributes`.
#[test]
fn colliding_attribute_round_trips_through_other() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("collide.city.jsonl");
    // Real Delft objects with one reserved-named attribute injected.
    let text = std::fs::read_to_string("../../tests/fixtures/delft.city.jsonl").unwrap();
    let mut lines = text.lines();
    let header = lines.next().unwrap().to_string();
    let mut feature: serde_json::Value = serde_json::from_str(lines.next().unwrap()).unwrap();
    for (_id, obj) in feature["CityObjects"].as_object_mut().unwrap() {
        obj["attributes"]["bbox"] = serde_json::json!("collides-with-reserved");
    }
    std::fs::write(&src, format!("{header}\n{feature}\n")).unwrap();

    let pkg = dir.path().join("pkg");
    let out = dir.path().join("out.city.jsonl");
    convert(&ConvertOptions::new(src.clone(), pkg.clone())).unwrap();
    export(&ExportOptions {
        package_dir: pkg.clone(),
        output: out.clone(),
    })
    .unwrap();

    let exported = std::fs::read_to_string(&out).unwrap();
    let line = exported.lines().nth(1).expect("one feature line");
    let feature: serde_json::Value = serde_json::from_str(line).unwrap();
    for (id, obj) in feature["CityObjects"].as_object().unwrap() {
        assert_eq!(
            obj["attributes"]["bbox"], "collides-with-reserved",
            "object {id} must recover its diverted attribute"
        );
    }
}

/// A top-level unmapped member and a RESERVED-named attribute (itself
/// diverted into `other`) share a key: the attribute wins, `other` carries
/// exactly one entry for it, and the drop is counted and diagnosed
/// (decided behaviour — "warn and prefer attribute").
#[test]
fn colliding_unmapped_member_and_reserved_attribute_keeps_the_attribute() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("collide-reserved.city.jsonl");
    let text = std::fs::read_to_string("../../tests/fixtures/delft.city.jsonl").unwrap();
    let mut lines = text.lines();
    let header = lines.next().unwrap().to_string();
    let mut feature: serde_json::Value = serde_json::from_str(lines.next().unwrap()).unwrap();
    let (target_id, target) = feature["CityObjects"]
        .as_object_mut()
        .unwrap()
        .iter_mut()
        .next()
        .expect("fixture has at least one object");
    let target_id = target_id.clone();
    // `attributes.bbox` collides with the RESERVED `bbox` column, so it is
    // diverted into `other`; a genuinely unmapped top-level member also
    // named `bbox` (not a real CityObject member — cjseq's `#[serde(flatten)]`
    // catch-all carries it through as unmapped) collides with it there.
    target["attributes"]["bbox"] = serde_json::json!("attribute-wins");
    target["bbox"] = serde_json::json!("unmapped-loses");
    std::fs::write(&src, format!("{header}\n{feature}\n")).unwrap();

    let pkg = dir.path().join("pkg");
    let out = dir.path().join("out.city.jsonl");
    let report = convert(&ConvertOptions::new(src.clone(), pkg.clone())).unwrap();

    assert_eq!(
        report.dropped_colliding_members, 1,
        "exactly one unmapped member must be dropped for the collision"
    );
    assert_eq!(
        report.dropped_colliding_member_diagnostics.len(),
        1,
        "must warn once for the dropped member"
    );
    let diagnostic = &report.dropped_colliding_member_diagnostics[0];
    assert!(
        diagnostic.contains(&target_id),
        "diagnostic must name the object id: {diagnostic}"
    );
    assert!(
        diagnostic.contains("bbox"),
        "diagnostic must name the colliding key: {diagnostic}"
    );

    export(&ExportOptions {
        package_dir: pkg.clone(),
        output: out.clone(),
    })
    .unwrap();
    let exported = std::fs::read_to_string(&out).unwrap();
    let line = exported.lines().nth(1).expect("one feature line");
    let feature: serde_json::Value = serde_json::from_str(line).unwrap();
    assert_eq!(
        feature["CityObjects"][&target_id]["attributes"]["bbox"], "attribute-wins",
        "the attribute must win over the colliding unmapped member"
    );
}

/// A top-level unmapped member and an ORDINARY (column-backed) attribute
/// share a key: the member is dropped so `other` never duplicates the
/// attribute's own column — a reader MUST-errors on that (`merge_other_members`
/// in `src/decode.rs`), so keeping the member would make this writer produce
/// a file its own reader rejects. Export must therefore succeed.
#[test]
fn colliding_unmapped_member_and_ordinary_attribute_is_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("collide-ordinary.city.jsonl");
    let text = std::fs::read_to_string("../../tests/fixtures/delft.city.jsonl").unwrap();
    let mut lines = text.lines();
    let header = lines.next().unwrap().to_string();
    let mut feature: serde_json::Value = serde_json::from_str(lines.next().unwrap()).unwrap();
    let (target_id, target) = feature["CityObjects"]
        .as_object_mut()
        .unwrap()
        .iter_mut()
        .next()
        .expect("fixture has at least one object");
    let target_id = target_id.clone();
    // `height` is an ordinary attribute name — not reserved, so it gets its
    // own column — colliding with a genuinely unmapped top-level member of
    // the same name.
    target["attributes"]["height"] = serde_json::json!(12.5);
    target["height"] = serde_json::json!("unmapped-loses");
    std::fs::write(&src, format!("{header}\n{feature}\n")).unwrap();

    let pkg = dir.path().join("pkg");
    let out = dir.path().join("out.city.jsonl");
    let report = convert(&ConvertOptions::new(src.clone(), pkg.clone())).unwrap();

    assert_eq!(
        report.dropped_colliding_members, 1,
        "exactly one unmapped member must be dropped for the collision"
    );
    assert_eq!(
        report.dropped_colliding_member_diagnostics.len(),
        1,
        "must warn once for the dropped member"
    );
    let diagnostic = &report.dropped_colliding_member_diagnostics[0];
    assert!(
        diagnostic.contains(&target_id),
        "diagnostic must name the object id: {diagnostic}"
    );
    assert!(
        diagnostic.contains("height"),
        "diagnostic must name the colliding key: {diagnostic}"
    );

    // Export must succeed — `other` must not carry an entry duplicating the
    // `height` column's own attribute (the reader hard-errors on that).
    export(&ExportOptions {
        package_dir: pkg.clone(),
        output: out.clone(),
    })
    .unwrap();
    let exported = std::fs::read_to_string(&out).unwrap();
    let line = exported.lines().nth(1).expect("one feature line");
    let feature: serde_json::Value = serde_json::from_str(line).unwrap();
    assert_eq!(
        feature["CityObjects"][&target_id]["attributes"]["height"], 12.5,
        "the ordinary attribute's value must survive the round trip"
    );
}

/// The table carries no `other_attributes` column at all.
#[test]
fn no_other_attributes_column_is_written() {
    let dir = tempfile::tempdir().unwrap();
    let pkg = dir.path().join("pkg");
    convert(&ConvertOptions::new(
        std::path::PathBuf::from("../../tests/fixtures/delft.city.jsonl"),
        pkg.clone(),
    ))
    .unwrap();

    let file = std::fs::File::open(pkg.join("building.parquet")).unwrap();
    let reader =
        parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
    let names: Vec<&str> = reader
        .schema()
        .fields()
        .iter()
        .map(|f| f.name().as_str())
        .collect();
    assert!(
        !names.contains(&"other_attributes"),
        "other_attributes must be gone, schema: {names:?}"
    );
    assert!(names.contains(&"other"), "other must remain: {names:?}");
}
