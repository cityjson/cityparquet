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

/// RED (G3): CityJSON 2.0 §3 requires a `lod` on every non-`GeometryInstance`
/// geometry. A source geometry without one is invalid input, and scan must
/// reject it naming the object — never silently drop it (mixed dataset) or
/// keep it in an un-suffixed column (uniformly lod-less dataset), the two
/// behaviours the old code chose between. Derived from delft: strip `lod`
/// from one object's geometry.
#[test]
fn lodless_non_instance_geometry_is_rejected() {
    let mut doc: serde_json::Value = serde_json::from_str(
        std::fs::read_to_string(fixture("delft.city.jsonl"))
            .unwrap()
            .lines()
            .nth(1)
            .expect("delft has feature lines"),
    )
    .unwrap();

    // Strip `lod` from the first geometry of the first object that has one.
    let mut target_id = None;
    for (id, co) in doc["CityObjects"].as_object_mut().unwrap() {
        if let Some(geoms) = co.get_mut("geometry").and_then(|g| g.as_array_mut())
            && let Some(g) = geoms.first_mut()
        {
            g.as_object_mut().unwrap().remove("lod");
            target_id = Some(id.clone());
            break;
        }
    }
    let target_id = target_id.expect("a delft feature must carry a geometry");

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("delft_lodless.city.jsonl");
    // A one-feature CityJSONSeq stream: header line + the mutated feature.
    let header = std::fs::read_to_string(fixture("delft.city.jsonl")).unwrap();
    let header_line = header.lines().next().unwrap();
    std::fs::write(
        &path,
        format!("{header_line}\n{}", serde_json::to_string(&doc).unwrap()),
    )
    .unwrap();

    let src = Source::open(&path).unwrap();
    let err = scan(&src).expect_err("lod-less non-instance geometry must be rejected");
    assert!(
        matches!(err, cityparquet::CityParquetError::Lod(_)),
        "must be a Lod error, got {err:?}"
    );
    let msg = err.to_string();
    assert!(
        msg.contains(&target_id) && msg.contains("no \"lod\""),
        "error must name the offending object and cite the missing lod, got: {msg}"
    );
}

/// The uniformly-lod-less case: a feature whose EVERY non-instance geometry
/// lost its `lod` must also be rejected — not silently kept in an un-suffixed
/// column, the old behaviour when no geometry had a lod. (The test above
/// covers the mixed case, where the object retains other LoD-bearing
/// geometries.)
#[test]
fn uniformly_lodless_dataset_is_rejected() {
    let mut doc: serde_json::Value = serde_json::from_str(
        std::fs::read_to_string(fixture("delft.city.jsonl"))
            .unwrap()
            .lines()
            .nth(1)
            .expect("delft has feature lines"),
    )
    .unwrap();

    // Strip `lod` from EVERY geometry of EVERY object.
    let mut stripped = 0usize;
    for (_, co) in doc["CityObjects"].as_object_mut().unwrap() {
        if let Some(geoms) = co.get_mut("geometry").and_then(|g| g.as_array_mut()) {
            for g in geoms {
                if g.as_object_mut().unwrap().remove("lod").is_some() {
                    stripped += 1;
                }
            }
        }
    }
    assert!(stripped > 0, "the feature must have had lods to strip");

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("delft_all_lodless.city.jsonl");
    let header = std::fs::read_to_string(fixture("delft.city.jsonl")).unwrap();
    std::fs::write(
        &path,
        format!(
            "{}\n{}",
            header.lines().next().unwrap(),
            serde_json::to_string(&doc).unwrap()
        ),
    )
    .unwrap();

    let src = Source::open(&path).unwrap();
    let err = scan(&src).expect_err("a uniformly lod-less dataset must be rejected");
    assert!(
        matches!(err, cityparquet::CityParquetError::Lod(_)),
        "must be a Lod error, got {err:?}"
    );
    assert!(
        err.to_string().contains("lod"),
        "error must cite the missing lod, got: {err}"
    );
}

/// A `GeometryInstance` is legitimately lod-less (its template carries the
/// lod, §12), so railway — which has 15 instances — must still scan cleanly.
#[test]
fn geometry_instances_are_not_rejected_as_lodless() {
    let src = Source::open(&fixture("lod3_railway.city.json")).unwrap();
    scan(&src).expect("GeometryInstance geometries are lod-less by design, not an error");
}

#[test]
fn delft_scan_matches_known_content() {
    let src = Source::open(&fixture("delft.city.jsonl")).unwrap();
    let s = scan(&src).unwrap();
    assert_eq!(s.object_count, 2231);
    let lod_strings: Vec<String> = s.lods.iter().map(ToString::to_string).collect();
    // "0" canonicalises to "0.0" — the LoD string always carries its minor
    // (spec "Levels of detail").
    assert_eq!(lod_strings, ["0.0", "1.2", "1.3", "2.2"]);
    // Recounted against the fixture with python3 (see report): 50 distinct
    // attribute names, not the 47 in the original brief.
    assert_eq!(s.schema.attributes.len(), 50);
    let meta = s.metadata(&[]).unwrap();
    // delft carries LoD0, so the default geometry is the suffixed LoD0
    // footprint column (preferred over the higher, Solid-family LoDs), not
    // an un-suffixed column.
    assert_eq!(meta.default_geometry, "geometry_lod0_0");
    assert_eq!(meta.bbox_column, "bbox");
    assert!(meta.crs.is_some());
    let arrow = s.schema.to_arrow_schema().unwrap();
    assert!(arrow.field_with_name("geometry_lod0_0").is_ok());
    assert!(arrow.field_with_name("geometry").is_err());
    assert!(arrow.field_with_name("geometry_lod0").is_err());
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
fn source_metadata_and_appearance_defaults_reach_scan_metadata() {
    // Derived fixture (same precedent as extensions_declarations_reach_metadata
    // above): neither shipped fixture sets appearance default-theme members,
    // so inject them into a mutated copy of the railway header (which already
    // has an `appearance` object with materials/textures) to exercise the
    // capture end to end.
    let mut doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(fixture("lod3_railway.city.json")).unwrap())
            .unwrap();
    doc["appearance"]["default-theme-material"] = serde_json::json!("theme-a");
    doc["appearance"]["default-theme-texture"] = serde_json::json!("theme-b");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("railway_theme.city.json");
    std::fs::write(&path, serde_json::to_string(&doc).unwrap()).unwrap();

    let src = Source::open(&path).unwrap();
    let s = scan(&src).unwrap();
    let meta = s.metadata(&[]).unwrap();

    let appearance_defaults = meta
        .appearance_defaults
        .expect("default-theme members must reach appearance_defaults");
    assert_eq!(appearance_defaults["default-theme-material"], "theme-a");
    assert_eq!(appearance_defaults["default-theme-texture"], "theme-b");

    let source_metadata = meta
        .source_metadata
        .expect("railway's header sets metadata; source_metadata must be populated");
    assert!(source_metadata.get("geographicalExtent").is_some());

    // delft's header carries no appearance key at all: that absence must be
    // preserved as None, not fabricated.
    let delft = Source::open(&fixture("delft.city.jsonl")).unwrap();
    let delft_meta = scan(&delft).unwrap().metadata(&[]).unwrap();
    assert!(delft_meta.appearance_defaults.is_none());
}

#[test]
fn railway_scan_is_representable() {
    let src = Source::open(&fixture("lod3_railway.city.json")).unwrap();
    let s = scan(&src).unwrap();
    assert_eq!(s.object_count, 121);
    assert!(!s.lods.is_empty());
    assert!(s.schema.to_arrow_schema().is_ok());
}
