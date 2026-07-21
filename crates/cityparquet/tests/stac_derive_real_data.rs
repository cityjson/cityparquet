//! Derive `city3d:*` properties from real converted CityParquet packages.
//!
//! Every input here is a real CityJSON file converted by this crate — no
//! inline artificial CityJSON, per this repo's testing discipline.

use std::path::PathBuf;

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

/// Convert a real fixture into a temp package and hand back its directory.
fn convert_fixture(name: &str, dir: &tempfile::TempDir) -> PathBuf {
    let out = dir.path().join("pkg");
    convert(&ConvertOptions::new(fixture(name), out.clone())).expect("convert fixture");
    out
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
    // The default Core profile writes no appearance sidecars, and delft has
    // no per-LoD appearance columns.
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
    let pkg = convert_fixture("lod3_railway.city.json", &dir);

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

/// `helsinki_address` carries a nested `address` object, which the encoder
/// stores as a JSON-typed column. It must surface as `Object`, not `String` —
/// a `Json` attribute is *stored* as Utf8, so anything relying on the raw
/// Arrow type alone would silently report `String`.
#[test]
fn json_attributes_are_reported_as_object_not_string() {
    let dir = tempfile::tempdir().unwrap();
    let pkg = convert_fixture("helsinki_address.city.jsonl", &dir);

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
