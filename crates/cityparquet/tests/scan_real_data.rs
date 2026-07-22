use std::path::PathBuf;

use cityparquet::scan::{city_and_geo_for_file, scan};
use cityparquet::source::Source;

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

/// The real `lod3_railway.city.json` fixture carries no `referenceSystem` at
/// all (a genuine open-data limitation, not something this crate may
/// hand-fabricate). Since `scan` now hard-fails on coordinate-bearing input
/// with no resolvable CRS (spec `05-metadata.mdx` "CRS rules"), every test
/// below that needs a clean railway scan writes a small on-disk COPY with a
/// CRS injected via JSON mutation of the real fixture — the same technique
/// [`extensions_declarations_reach_metadata`] already used for extensions,
/// never hand-written CityJSON. EPSG:7415 (Amersfoort/RD New + NAP), the same
/// CRS delft already carries, so railway-derived fixtures stay resolvable
/// against the same vendored PROJJSON table.
fn railway_source_with_crs() -> (tempfile::TempDir, PathBuf) {
    let mut doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(fixture("lod3_railway.city.json")).unwrap())
            .unwrap();
    doc["metadata"]["referenceSystem"] =
        serde_json::json!("https://www.opengis.net/def/crs/EPSG/0/7415");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("railway_with_crs.city.json");
    std::fs::write(&path, serde_json::to_string(&doc).unwrap()).unwrap();
    (dir, path)
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
    let (_dir, path) = railway_source_with_crs();
    let src = Source::open(&path).unwrap();
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
    let meta = s.base_city_metadata().unwrap();
    assert!(meta.crs.is_some());
    // `columns`/`primary_column` are no longer part of the dataset-wide
    // metadata (spec-alignment M3: they only exist per FILE, from that
    // file's own realised column set — see `city_and_geo_for_file`, exercised
    // below against delft's single (Building) module).
    assert!(meta.columns.is_empty());
    assert!(meta.primary_column.is_none());

    let arrow = s.schema.to_arrow_schema().unwrap();
    assert!(arrow.field_with_name("geometry_lod0_0").is_ok());
    assert!(arrow.field_with_name("geometry").is_err());
    assert!(arrow.field_with_name("geometry_lod0").is_err());
}

/// `city_and_geo_for_file` — the per-file `city.columns`/two-selector
/// primary-column logic (spec-alignment M3) — against delft's own single
/// (Building) module: delft carries LoD0, so the CityParquet primary is the
/// suffixed LoD0 footprint column (preferred over the higher, legal LoDs),
/// matching the pre-M3 `default_geometry` pin this replaces.
#[test]
fn delft_city_and_geo_for_file_prefers_lod0() {
    let src = Source::open(&fixture("delft.city.jsonl")).unwrap();
    let s = scan(&src).unwrap();
    assert_eq!(
        s.module_geo.len(),
        1,
        "delft is Building-only: exactly one module's worth of geometry"
    );
    let per_lod = s.module_geo.values().next().unwrap();
    let (columns, primary_column, geo) = city_and_geo_for_file(per_lod);
    assert!(!columns.is_empty());
    assert_eq!(primary_column.as_deref(), Some("geometry_lod0_0"));
    let geo = geo.expect("delft's LoD0 footprint is GeoParquet-legal");
    assert_eq!(geo.primary_column, "geometry_lod0_0");
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
    doc["metadata"]["referenceSystem"] =
        serde_json::json!("https://www.opengis.net/def/crs/EPSG/0/7415");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("railway_noise.city.json");
    std::fs::write(&path, serde_json::to_string(&doc).unwrap()).unwrap();

    let src = Source::open(&path).unwrap();
    let s = scan(&src).unwrap();
    let meta = s.base_city_metadata().unwrap();
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
    let delft_meta = scan(&delft).unwrap().base_city_metadata().unwrap();
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
    doc["metadata"]["referenceSystem"] =
        serde_json::json!("https://www.opengis.net/def/crs/EPSG/0/7415");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("railway_theme.city.json");
    std::fs::write(&path, serde_json::to_string(&doc).unwrap()).unwrap();

    let src = Source::open(&path).unwrap();
    let s = scan(&src).unwrap();
    let meta = s.base_city_metadata().unwrap();

    let appearance_defaults = meta
        .appearance_defaults
        .expect("default-theme members must reach appearance_defaults");
    assert_eq!(appearance_defaults["default-theme-material"], "theme-a");
    assert_eq!(appearance_defaults["default-theme-texture"], "theme-b");

    // `source_metadata` is folded into `city.other` now (spec-alignment M3,
    // gap 16) rather than its own footer key.
    let other = meta
        .other
        .expect("railway's header sets metadata; `other` must be populated");
    let source_metadata = other
        .get("source_metadata")
        .expect("railway's header metadata must be preserved under other.source_metadata");
    assert!(source_metadata.get("geographicalExtent").is_some());
    assert!(
        other.get("transform").is_some(),
        "other must also carry the source transform"
    );

    // delft's header carries no appearance key at all: that absence must be
    // preserved as None, not fabricated.
    let delft = Source::open(&fixture("delft.city.jsonl")).unwrap();
    let delft_meta = scan(&delft).unwrap().base_city_metadata().unwrap();
    assert!(delft_meta.appearance_defaults.is_none());
}

#[test]
fn railway_scan_is_representable() {
    let (_dir, path) = railway_source_with_crs();
    let src = Source::open(&path).unwrap();
    let s = scan(&src).unwrap();
    assert_eq!(s.object_count, 121);
    assert!(!s.lods.is_empty());
    assert!(s.schema.to_arrow_schema().is_ok());
}

/// spec-alignment M3, checklist item 5: a CRS-less coordinate-bearing input
/// (the real railway fixture, unmodified) must error cleanly at `scan` time,
/// never silently omit `city.crs`.
#[test]
fn railway_without_a_crs_is_a_hard_conversion_error() {
    let src = Source::open(&fixture("lod3_railway.city.json")).unwrap();
    let err = scan(&src).expect_err("coordinate-bearing input with no CRS must fail the scan");
    assert!(
        matches!(err, cityparquet::CityParquetError::Schema(_)),
        "expected a Schema error, got {err:?}"
    );
    assert!(
        err.to_string().contains("CRS"),
        "error must explain the missing CRS, got: {err}"
    );
}
