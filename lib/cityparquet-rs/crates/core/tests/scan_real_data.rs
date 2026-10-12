use std::collections::BTreeSet;
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
/// hand-fabricate). Coordinate-bearing input with no resolvable CRS now
/// converts to an explicit `city.crs: null` rather than failing (spec
/// `05-metadata.mdx` "CRS rules": "an unresolvable CRS is declared, not
/// fatal") — pinned by
/// [`railway_without_a_crs_scans_to_an_explicit_null_crs_and_a_diagnostic`] —
/// so every test below that wants a GEOREFERENCED railway scan writes a small
/// on-disk COPY with a CRS injected via JSON mutation of the real fixture —
/// the same technique
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

/// CityJSON 2.0 §3 requires a `lod` on every non-`GeometryInstance`
/// geometry. A source geometry without one is invalid input and is refused
/// when the feature is read — never dropped, never kept in an un-suffixed
/// column. Derived from delft: strip `lod` from one object's geometry.
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
    let mut stripped = false;
    for co in doc["CityObjects"].as_object_mut().unwrap().values_mut() {
        if let Some(geoms) = co.get_mut("geometry").and_then(|g| g.as_array_mut())
            && let Some(g) = geoms.first_mut()
        {
            stripped = g.as_object_mut().unwrap().remove("lod").is_some();
            break;
        }
    }
    assert!(stripped, "a delft feature must carry a geometry with a lod");

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
        err.to_string().contains("`lod`"),
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
    assert!(meta.crs.is_known());
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
/// (Building) module. The two selectors are INDEPENDENT and, for delft,
/// genuinely differ: `city_primary` is the highest LoD present regardless of
/// legality (delft's highest is 2.2, a real `Solid`), while `geo_primary`
/// prefers the `0.*` family when present (delft's native LoD0 footprint) —
/// spec "Why city.primary_column and geo.primary_column can differ".
#[test]
fn delft_city_and_geo_for_file_has_independent_primaries() {
    let src = Source::open(&fixture("delft.city.jsonl")).unwrap();
    let s = scan(&src).unwrap();
    assert_eq!(
        s.module_geo.len(),
        1,
        "delft is Building-only: exactly one module's worth of geometry"
    );
    let per_lod = s.module_geo.values().next().unwrap();
    let (columns, primary_column, geo) = city_and_geo_for_file(per_lod, &s.crs);
    assert!(!columns.is_empty());
    assert_eq!(
        primary_column.as_deref(),
        Some("geometry_lod2_2"),
        "city.primary_column is the highest LoD present, solids included"
    );
    assert!(
        columns
            .iter()
            .any(|c| c.name == "geometry_lod2_2" && c.geometry_types == ["PolyhedralSurface Z"]),
        "geometry_lod2_2 must be recorded as the real Solid-family type it is: {columns:?}"
    );
    let geo = geo.expect("delft's LoD0 footprint is GeoParquet-legal");
    assert_eq!(
        geo.primary_column, "geometry_lod0_0",
        "geo.primary_column prefers the 0.* family when present"
    );
    assert!(
        !geo.columns.contains_key("geometry_lod2_2"),
        "the Solid column must NOT be declared in geo.columns"
    );
    let footprint_crs = geo.columns["geometry_lod0_0"].crs.known().expect(
        "geo.columns[].crs must be explicit (GeoParquet's own absent-crs-means-CRS84 \
                 rule, spec CRS rules \"A writer never relies on the absent-CRS default\")",
    );
    assert_eq!(footprint_crs["id"]["code"], serde_json::json!(7415));
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
    // Keyed by the derived namespace, carrying the source name, the schema
    // document's URL and its version (spec "Extensions").
    let noise = &extensions["noise"];
    assert_eq!(noise.name, "Noise");
    assert_eq!(
        noise.url,
        "https://www.cityjson.org/extensions/download/noise.ext.json"
    );
    assert_eq!(noise.version.as_deref(), Some("1.1.0"));
    assert_eq!(extensions.len(), 1);

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

/// Spec §metadata "CRS rules", as amended: a CRS-less coordinate-bearing input
/// (the real railway fixture, unmodified) is **declared, not fatal**. The scan
/// succeeds, `city.crs` becomes an explicit `null` — never absent, which per
/// GeoParquet would assert OGC:CRS84 over coordinates that have no
/// georeference — and a conversion diagnostic explains it.
///
/// This replaces `railway_without_a_crs_is_a_hard_conversion_error`, which
/// pinned the earlier draft's hard-error rule.
#[test]
fn railway_without_a_crs_scans_to_an_explicit_null_crs_and_a_diagnostic() {
    let src = Source::open(&fixture("lod3_railway.city.json")).unwrap();
    let s = scan(&src).expect("a CRS-less coordinate-bearing input is declared, not fatal");
    assert_eq!(
        s.crs,
        cityparquet_schema::CrsState::Unknown,
        "a CRS-bearing-coordinate source with no CRS is `null`, never absent"
    );
    let diagnostic = s
        .crs_diagnostic
        .as_deref()
        .expect("the writer SHOULD surface a conversion diagnostic");
    assert!(
        diagnostic.contains("declares no CRS"),
        "the diagnostic must say what is missing, got: {diagnostic}"
    );

    // ... and the footer it feeds says the same thing.
    let meta = s.base_city_metadata().unwrap();
    assert_eq!(meta.crs, cityparquet_schema::CrsState::Unknown);
}

/// The other unresolvable shape: the source DOES declare a reference system,
/// but under an authority this writer has no PROJJSON for (IGNF, the French
/// national registry — the resolver deliberately refuses a numeric code under
/// a non-EPSG authority rather than mis-reading it as EPSG). Same outcome — an
/// explicit `null` plus a diagnostic that names the identifier — because the
/// spec's rule is about what the writer can resolve, not about whether the
/// source said anything at all. Derived from the real Delft fixture rather
/// than hand-written CityJSON.
#[test]
fn an_unresolvable_reference_system_scans_to_an_explicit_null_crs() {
    let text = std::fs::read_to_string(fixture("delft.city.jsonl")).unwrap();
    let mut lines = text.lines();
    let mut header: serde_json::Value = serde_json::from_str(lines.next().unwrap()).unwrap();
    header["metadata"]["referenceSystem"] =
        serde_json::json!("https://www.opengis.net/def/crs/IGNF/0/5490");
    let mut out = serde_json::to_string(&header).unwrap();
    for line in lines {
        out.push('\n');
        out.push_str(line);
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ignf_crs.city.jsonl");
    std::fs::write(&path, out).unwrap();

    let src = Source::open(&path).unwrap();
    let s = scan(&src).expect("an unresolvable CRS is declared, not fatal (spec CRS rules)");
    assert_eq!(s.crs, cityparquet_schema::CrsState::Unknown);
    let diagnostic = s.crs_diagnostic.as_deref().expect("a diagnostic");
    assert!(
        diagnostic.contains("IGNF") && diagnostic.contains("could not be resolved"),
        "the diagnostic must name the identifier it could not resolve, got: {diagnostic}"
    );
}

/// The estimator's hasher is seeded deterministically, so the selection —
/// and with it the package layout — is identical run to run.
///
/// Two scans in ONE process would agree under a per-process random seed too,
/// which is exactly the failure this is meant to catch, so both scans are
/// checked against the hard-coded set rather than only against each other.
#[test]
fn the_bloom_attribute_selection_is_deterministic() {
    let expected: BTreeSet<String> = ["documentnummer", "identificatie"]
        .into_iter()
        .map(String::from)
        .collect();
    let first = scan(&Source::open(&fixture("delft.city.jsonl")).unwrap()).unwrap();
    let second = scan(&Source::open(&fixture("delft.city.jsonl")).unwrap()).unwrap();
    assert_eq!(first.bloom_attributes, expected);
    assert_eq!(second.bloom_attributes, expected);
}

/// A copy of delft with every LoD `0` geometry dropped and every
/// `identificatie` attribute renamed to `to` — a JSON mutation of the real
/// fixture, never hand-written CityJSON. Without its LoD0 footprints delft
/// keeps LoDs 1.2, 1.3 and 2.2, so LoD0 synthesis has work to do.
fn delft_without_lod0_and_identificatie_renamed(to: &str) -> (tempfile::TempDir, PathBuf) {
    let text = std::fs::read_to_string(fixture("delft.city.jsonl")).unwrap();
    let mut out = String::new();
    for (index, line) in text.lines().enumerate() {
        if index == 0 || line.trim().is_empty() {
            out.push_str(line);
            out.push('\n');
            continue;
        }
        let mut feature: serde_json::Value = serde_json::from_str(line).unwrap();
        for (_, object) in feature["CityObjects"].as_object_mut().unwrap() {
            if let Some(geometries) = object.get_mut("geometry").and_then(|g| g.as_array_mut()) {
                geometries.retain(|g| g["lod"] != "0");
            }
            if let Some(attrs) = object.get_mut("attributes").and_then(|a| a.as_object_mut())
                && let Some(value) = attrs.remove("identificatie")
            {
                attrs.insert(to.to_string(), value);
            }
        }
        out.push_str(&serde_json::to_string(&feature).unwrap());
        out.push('\n');
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("delft_renamed.city.jsonl");
    std::fs::write(&path, out).unwrap();
    (dir, path)
}

/// Stripped of its LoD0, delft has no `0.*` LoD, yet an attribute named
/// `geometry_lod0_0` still collides: reservation follows the LoD suffix
/// grammar, not the dataset's LoDs (spec "Column naming and reservation
/// rules"). `scan` stores it escaped as `_geometry_lod0_0` — a qualifying
/// bloom column — and LoD0 synthesis, which then adds the real
/// `geometry_lod0_0`, leaves it untouched.
#[test]
fn an_attribute_named_like_a_per_lod_column_is_escaped_at_scan() {
    let (_dir, path) = delft_without_lod0_and_identificatie_renamed("geometry_lod0_0");
    let mut result = scan(&Source::open(&path).unwrap()).unwrap();
    let names = |r: &cityparquet::scan::ScanResult| -> Vec<String> {
        r.schema.attributes.iter().map(|(n, _)| n.clone()).collect()
    };
    assert!(names(&result).contains(&"_geometry_lod0_0".to_string()));
    assert!(!names(&result).contains(&"geometry_lod0_0".to_string()));
    assert!(
        result.bloom_attributes.contains("_geometry_lod0_0"),
        "before synthesis: {:?}",
        result.bloom_attributes
    );
    result.add_synthesized_lod0_column();
    assert!(names(&result).contains(&"_geometry_lod0_0".to_string()));
    assert!(
        result.bloom_attributes.contains("_geometry_lod0_0"),
        "after synthesis: {:?}",
        result.bloom_attributes
    );
}
