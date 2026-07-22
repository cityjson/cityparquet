//! Derive `city3d:*` properties from real converted CityParquet packages.
//!
//! Every input here is a real CityJSON file converted by this crate — no
//! inline artificial CityJSON, per this repo's testing discipline.

use std::path::{Path, PathBuf};

use city3d_stac_types::metadata::AttributeType;
use city3d_stac_types::stac::CityObjectsCount;
use cityparquet::package::{ConvertOptions, convert};
use cityparquet::stac::properties::{PackageTables, derive_co_types, derive_from_footer};
use cityparquet::stac::{ItemOptions, item_for_package, package_bbox};

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

/// A committed, in-tree fixture under `crates/cityparquet/tests/data/` (small
/// hand-derived inputs with no public download URL — see `roundtrip_real_data`).
fn data_fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(name);
    assert!(p.exists(), "missing committed fixture {name} in tests/data");
    p
}

/// Convert a fetched `tests/fixtures/` input into a temp package.
fn convert_fixture(name: &str, dir: &tempfile::TempDir) -> PathBuf {
    convert_fixture_path(&fixture(name), dir)
}

/// Convert a resolved path (so a committed [`data_fixture`] can be passed).
fn convert_fixture_path(input: &Path, dir: &tempfile::TempDir) -> PathBuf {
    let out = dir.path().join("pkg");
    convert(&ConvertOptions::new(input.to_path_buf(), out.clone())).expect("convert fixture");
    out
}

/// The real `lod3_railway.city.json` fixture carries no `referenceSystem` at
/// all. Since `scan` now hard-fails on coordinate-bearing input with no
/// resolvable CRS (spec "CRS rules"), tests below write a small on-disk COPY
/// with a CRS injected via JSON mutation of the real fixture — never
/// hand-written CityJSON.
fn railway_fixture_with_crs() -> (tempfile::TempDir, PathBuf) {
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

/// The footer- and schema-derived fields for delft.
///
/// `object_count` 2231 is the same figure pinned by `decode_real_data.rs` and
/// `encode_real_data.rs` — deriving it from the Parquet footer's `num_rows`
/// must agree with what conversion reported, because each row is one
/// CityObject.
#[test]
fn footer_properties_from_delft() {
    let dir = tempfile::tempdir().unwrap();
    let pkg = convert_fixture("delft.city.jsonl", &dir);

    let tables = PackageTables::open(&pkg).expect("resolve tables");
    let props = derive_from_footer(&tables).expect("derive");

    assert_eq!(
        props.city_objects,
        Some(CityObjectsCount::Integer(2231)),
        "city_objects must count CityObjects (rows), matching the pinned figure"
    );
    assert!(
        !props.lods.is_empty(),
        "delft has analysis geometry, so at least one LoD must be reported"
    );
    assert!(
        !props.attributes.is_empty(),
        "delft carries source attributes; they must be derived from the schema"
    );
    // The Core profile writes no appearance sidecars, so both flags are a
    // definite `false`: the derivation keys off sidecar presence, not the
    // per-geometry appearance columns (which a Core package still carries).
    assert_eq!(props.textures, Some(false));
    assert_eq!(props.materials, Some(false));
    assert_eq!(
        props.semantic_surfaces,
        Some(true),
        "delft carries semantic surfaces"
    );
}

/// LoDs must come back as the LoD strings the writer used, not column names.
#[test]
fn lods_are_reported_as_lod_strings() {
    let dir = tempfile::tempdir().unwrap();
    let pkg = convert_fixture("delft.city.jsonl", &dir);

    let tables = PackageTables::open(&pkg).expect("resolve tables");
    let props = derive_from_footer(&tables).expect("derive");

    for lod in &props.lods {
        assert!(
            !lod.starts_with("geometry"),
            "lods must be LoD strings like \"2.2\", got a column name: {lod}"
        );
        assert!(
            lod.chars().next().is_some_and(|c| c.is_ascii_digit()),
            "a LoD string must start with a digit, got {lod}"
        );
    }
    // Deduplicated, and ordered by LoD rather than lexicographically. delft's
    // own set (0, 1.2, 1.3, 2.2) sorts the same either way, so comparing
    // against a lexicographic sort of itself would be vacuous — instead assert
    // the property directly by parsing each LoD's numeric value.
    let mut deduped = props.lods.clone();
    deduped.dedup();
    assert_eq!(props.lods, deduped, "lods must be deduplicated");

    let numeric: Vec<f64> = props
        .lods
        .iter()
        .map(|l| {
            l.parse::<f64>()
                .unwrap_or_else(|_| panic!("LoD {l} is not numeric"))
        })
        .collect();
    assert!(
        numeric.windows(2).all(|w| w[0] < w[1]),
        "lods must ascend by LoD value, got {:?}",
        props.lods
    );
}

/// delft is `Building` + `BuildingPart`. Under the by-type layout both share
/// `building.parquet` (spec §4.3 forbids a 2nd-level type its own file), so a
/// filename-derived answer would report only `Building` and silently lose
/// `BuildingPart`. Reading the `object_type` column is the only correct source.
#[test]
fn co_types_include_second_level_types_a_filename_would_hide() {
    let dir = tempfile::tempdir().unwrap();
    let pkg = convert_fixture("delft.city.jsonl", &dir);

    let tables = PackageTables::open(&pkg).expect("resolve tables");
    let types = derive_co_types(&tables).expect("co_types");

    assert_eq!(
        types,
        vec!["Building".to_string(), "BuildingPart".to_string()],
        "both the 1st-level and the 2nd-level type must be reported, sorted"
    );
}

/// A dataset with many types, four of them 2nd-level. Pins the full set so a
/// regression that drops or invents a type fails loudly.
#[test]
fn co_types_cover_a_many_type_dataset() {
    let dir = tempfile::tempdir().unwrap();
    let (_crs_dir, railway_path) = railway_fixture_with_crs();
    let pkg = convert_fixture_path(&railway_path, &dir);

    let tables = PackageTables::open(&pkg).expect("resolve tables");
    let types = derive_co_types(&tables).expect("co_types");

    let expected: Vec<String> = [
        "Bridge",
        "BridgeConstructiveElement",
        "BridgeInstallation",
        "Building",
        "BuildingInstallation",
        "CityFurniture",
        "CityObjectGroup",
        "GenericCityObject",
        "Railway",
        "SolitaryVegetationObject",
        "TINRelief",
        "Tunnel",
        "TunnelInstallation",
        "WaterBody",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    assert_eq!(
        types, expected,
        "co_types must be exactly the types present, sorted"
    );
}

/// `derive_from_footer` must populate `co_types`, not leave it to callers.
#[test]
fn derive_from_footer_populates_co_types() {
    let dir = tempfile::tempdir().unwrap();
    let pkg = convert_fixture("delft.city.jsonl", &dir);

    let tables = PackageTables::open(&pkg).expect("resolve tables");
    let props = derive_from_footer(&tables).expect("derive");

    assert_eq!(
        props.co_types,
        vec!["Building".to_string(), "BuildingPart".to_string()]
    );
}

/// delft carries semantic surfaces on most of its LoD2.2 geometry (1115 of the
/// source's objects have a `semantics` block), so the flag must be `true`.
///
/// This cannot come from the column's mere presence — `geometry_properties*`
/// is written for every LoD regardless — nor from its Parquet statistics,
/// because it is a JSON column and `recipe.rs` disables statistics for those
/// by default. The derivation has to consult the data.
#[test]
fn semantic_surfaces_true_for_delft() {
    let dir = tempfile::tempdir().unwrap();
    let pkg = convert_fixture("delft.city.jsonl", &dir);

    let tables = PackageTables::open(&pkg).expect("resolve tables");
    let props = derive_from_footer(&tables).expect("derive");

    assert_eq!(
        props.semantic_surfaces,
        Some(true),
        "delft has semantic surfaces; the flag must not be absent or false"
    );
}

/// The Compatibility profile writes `textures.parquet` and `materials.parquet`
/// for a dataset that carries appearance, so `textures`/`materials` must both
/// be `true`. This is the ONLY case that tells a working appearance derivation
/// apart from one hard-wired to `false`: every Core package (and every fixture
/// converted by default) ships no sidecar and reads `false`. A package that
/// carries appearance INDEX columns but no definition sidecar — the Core
/// profile — still reads `false`, which is the "not renderable here" semantics.
#[test]
fn appearance_flags_true_only_with_compatibility_sidecars() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("pkg");
    let (_crs_dir, railway_path) = railway_fixture_with_crs();
    let opts = ConvertOptions::new(railway_path, out.clone());
    convert(&opts).expect("convert lod3_railway");

    let tables = PackageTables::open(&out).expect("resolve tables");
    let props = derive_from_footer(&tables).expect("derive");

    assert_eq!(
        props.textures,
        Some(true),
        "lod3_railway carries textures; the Compatibility sidecar must set the flag"
    );
    assert_eq!(
        props.materials,
        Some(true),
        "lod3_railway carries materials; the Compatibility sidecar must set the flag"
    );
}

/// The package extent, in the SOURCE CRS. delft is EPSG:7415 (RD New + NAP),
/// so its coordinates are RD New metres, not degrees — reprojection to WGS84
/// happens later, when the Item is assembled.
#[test]
fn package_bbox_is_the_source_crs_extent() {
    let dir = tempfile::tempdir().unwrap();
    let pkg = convert_fixture("delft.city.jsonl", &dir);

    let tables = PackageTables::open(&pkg).expect("resolve tables");
    let bbox = package_bbox(&tables)
        .expect("bbox")
        .expect("delft has geometry, so it must have an extent");

    assert!(
        bbox.xmin < bbox.xmax && bbox.ymin < bbox.ymax,
        "extent must be non-degenerate, got {bbox:?}"
    );
    assert!(
        (80_000.0..95_000.0).contains(&bbox.xmin),
        "xmin {} is not in the RD New range for Delft — is this WGS84 by mistake?",
        bbox.xmin
    );
    assert!(
        (440_000.0..455_000.0).contains(&bbox.ymin),
        "ymin {} is not in the RD New range for Delft",
        bbox.ymin
    );
}

/// The whole point of the plan: a package on disk yields a STAC Item that the
/// published `city3d` extension schema accepts.
#[test]
fn derived_item_validates_against_the_city3d_schema() {
    let dir = tempfile::tempdir().unwrap();
    let pkg = convert_fixture("delft.city.jsonl", &dir);

    let item = item_for_package(
        &pkg,
        &ItemOptions {
            id: Some("delft-test".to_string()),
            datetime: Some("2024-01-15T12:00:00Z".to_string()),
            ..Default::default()
        },
    )
    .expect("build item");

    let instance = serde_json::to_value(&item).unwrap();
    let schema: serde_json::Value =
        serde_json::from_str(include_str!("data/stac-city3d-v0.2.0.json")).unwrap();
    let validator = jsonschema::validator_for(&schema).expect("compile schema");
    let errors: Vec<String> = validator
        .iter_errors(&instance)
        .map(|e| format!("{e} at {}", e.instance_path))
        .collect();
    assert!(
        errors.is_empty(),
        "derived Item violates the city3d schema:\n{}\n\ninstance:\n{}",
        errors.join("\n"),
        serde_json::to_string_pretty(&instance).unwrap()
    );
}

/// Assets must describe files that are actually present, and the bbox must be
/// WGS84 — STAC requires it, and delft's source coordinates are RD New metres.
#[test]
fn derived_item_assets_exist_and_bbox_is_wgs84() {
    let dir = tempfile::tempdir().unwrap();
    let pkg = convert_fixture("delft.city.jsonl", &dir);

    let item = item_for_package(&pkg, &ItemOptions::default()).expect("build item");

    assert!(!item.assets.is_empty(), "a package has files to describe");
    for (key, asset) in &item.assets {
        let path = pkg.join(asset.href.trim_start_matches("./"));
        assert!(
            path.exists(),
            "asset {key} points at {} which is not on disk",
            asset.href
        );
    }

    let bbox = item.bbox.as_ref().expect("delft has an extent");
    assert!(
        (4.0..5.0).contains(&bbox[0]) && (51.0..53.0).contains(&bbox[1]),
        "bbox must be reprojected to WGS84 degrees near Delft, got {bbox:?}"
    );

    // Defaulted id comes from the package directory name.
    assert_eq!(item.id, "pkg");
}

/// Plan 2b Task 4 step 2: `cityparquet:version` must land in the Item's
/// properties, footer-derived like everything else in `stac::mod`. Distinct
/// from `city3d:version`, which is the SOURCE CityJSON version, not
/// CityParquet's own. `cityparquet:profile` is GONE (spec-alignment gap 19:
/// the `Profile` concept it described no longer exists) — no `ItemOptions`
/// field to supply it with any more.
#[test]
fn cityparquet_version_property_is_present() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("pkg");
    let opts = ConvertOptions::new(fixture("delft.city.jsonl"), out.clone());
    convert(&opts).expect("convert delft");

    let item = item_for_package(&out, &ItemOptions::default()).expect("build item");

    assert_eq!(
        item.properties
            .additional_fields
            .get("cityparquet:version")
            .and_then(|v| v.as_str()),
        Some(cityparquet::schema::CITYPARQUET_VERSION),
        "cityparquet:version must be the footer-derived CityParquet format version"
    );
    assert!(
        !item
            .properties
            .additional_fields
            .contains_key("cityparquet:profile"),
        "cityparquet:profile must not be written at all any more (spec-alignment gap 19)"
    );
}

/// Plan 2b Task 4 step 3 (design decision 2026-07-21): a package with no
/// explicit `datetime` and no source `referenceDate` must still end up with
/// a non-null RFC 3339 `datetime` — the conversion-timestamp fallback, which
/// now applies uniformly rather than only from the CLI. delft carries no
/// `referenceDate` (see `stac::ItemOptions`'s doc comment), so this exercises
/// the fallback with a real fixture rather than an inline one.
#[test]
fn datetime_falls_back_to_a_conversion_timestamp_when_nothing_else_is_available() {
    let dir = tempfile::tempdir().unwrap();
    let pkg = convert_fixture("delft.city.jsonl", &dir);

    let before = chrono::Utc::now();
    let item = item_for_package(&pkg, &ItemOptions::default()).expect("build item");
    let after = chrono::Utc::now();

    let datetime = item
        .properties
        .datetime
        .expect("datetime must not be null when neither explicit nor source referenceDate exist");
    assert!(
        datetime >= before - chrono::Duration::seconds(1) && datetime <= after,
        "fallback datetime {datetime} must be close to the conversion time \
         ({before} .. {after})"
    );
}

/// `helsinki_address` has no `referenceSystem` in its CityJSON header at all
/// (a CRS is optional in CityJSON) — real, not synthetic: this is a committed
/// fixture. `write_package` now builds a STAC Item for every conversion
/// (Task 4), and building that Item must not fail just because the extent
/// cannot be expressed in WGS84 without a CRS: `convert` itself must still
/// succeed, and the written `metadata.json` must be an "unlocated" Item
/// (`geometry: null`, no `bbox` key) that still validates against the city3d
/// schema — not a wrong extent, and not a failed conversion either.
#[test]
fn a_package_with_no_crs_converts_to_an_unlocated_but_schema_valid_item() {
    // `helsinki_address` now carries a CRS (spec-alignment M3: coordinate-
    // bearing input with none is a hard conversion error, per "CRS rules") —
    // an "unlocated" Item is only still legitimate for a source with NO
    // CRS-bearing coordinate at all. Derived from the real CityJSONSeq
    // fixture: strip `geometry`/`address` from every feature's objects
    // (attributes-only) and the header's own CRS, so the source genuinely
    // carries none — never hand-written CityJSON, the same "mutate the real
    // parsed structure, line by line" technique used throughout this test
    // suite (e.g. `scan_real_data.rs`).
    let text = std::fs::read_to_string(data_fixture("helsinki_address.city.jsonl")).unwrap();
    let mut lines = text.lines();
    let mut header: serde_json::Value = serde_json::from_str(lines.next().unwrap()).unwrap();
    header["metadata"]
        .as_object_mut()
        .unwrap()
        .remove("referenceSystem");
    let mut out_lines = vec![serde_json::to_string(&header).unwrap()];
    for line in lines {
        let mut feature: serde_json::Value = serde_json::from_str(line).unwrap();
        for (_, co) in feature["CityObjects"].as_object_mut().unwrap() {
            co.as_object_mut().unwrap().remove("geometry");
            co.as_object_mut().unwrap().remove("address");
        }
        out_lines.push(serde_json::to_string(&feature).unwrap());
    }

    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("helsinki_attributes_only.city.jsonl");
    std::fs::write(&input, out_lines.join("\n")).unwrap();

    let out = dir.path().join("pkg");
    convert(&ConvertOptions::new(input, out.clone()))
        .expect("convert must succeed: an attributes-only source has no CRS-bearing coordinate");

    let text = std::fs::read_to_string(out.join("metadata.json")).unwrap();
    let instance: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert!(
        instance.get("geometry").is_some_and(|g| g.is_null()),
        "an unlocated Item still carries the (null) geometry key: {instance}"
    );
    assert!(
        instance.get("bbox").is_none(),
        "an unlocated Item must not carry a bbox key at all: {instance}"
    );

    let schema: serde_json::Value =
        serde_json::from_str(include_str!("data/stac-city3d-v0.2.0.json")).unwrap();
    let validator = jsonschema::validator_for(&schema).expect("compile schema");
    let errors: Vec<String> = validator
        .iter_errors(&instance)
        .map(|e| format!("{e} at {}", e.instance_path))
        .collect();
    assert!(
        errors.is_empty(),
        "the unlocated Item violates the city3d schema:\n{}\n\ninstance:\n{}",
        errors.join("\n"),
        serde_json::to_string_pretty(&instance).unwrap()
    );
}

/// `helsinki_address` carries a nested `address` object, which the encoder
/// stores as a JSON-typed column. It must surface as `Object`, not `String` —
/// a `Json` attribute is *stored* as Utf8, so anything relying on the raw
/// Arrow type alone would silently report `String`.
#[test]
fn json_attributes_are_reported_as_object_not_string() {
    let dir = tempfile::tempdir().unwrap();
    let pkg = convert_fixture_path(&data_fixture("helsinki_address.city.jsonl"), &dir);

    let tables = PackageTables::open(&pkg).expect("resolve tables");
    let props = derive_from_footer(&tables).expect("derive");

    // Name the attribute rather than merely counting Object-typed ones: a
    // check for "at least one Object" would also pass if EVERY attribute were
    // mistyped as Object.
    let address = props
        .attributes
        .iter()
        .find(|a| a.name == "Integrate_LoD[1]")
        .unwrap_or_else(|| {
            panic!(
                "expected an `Integrate_LoD[1]` attribute; got {:?}",
                props
                    .attributes
                    .iter()
                    .map(|a| (&a.name, a.attr_type))
                    .collect::<Vec<_>>()
            )
        });
    assert_eq!(
        address.attr_type,
        AttributeType::Object,
        "a nested JSON attribute must be Object, not String"
    );
    // And the other attributes must NOT all have collapsed to Object.
    assert!(
        props
            .attributes
            .iter()
            .any(|a| a.attr_type != AttributeType::Object),
        "not every attribute is JSON-typed; a blanket Object would be wrong"
    );
}
