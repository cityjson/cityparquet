//! Derive `city3d:*` properties from real converted CityParquet packages.
//!
//! Every input here is a real CityJSON file converted by this crate — no
//! inline artificial CityJSON, per this repo's testing discipline.

use std::path::PathBuf;

use city3d_stac_types::metadata::AttributeType;
use city3d_stac_types::stac::CityObjectsCount;
use cityparquet::package::{ConvertOptions, convert};
use cityparquet::stac::package_bbox;
use cityparquet::stac::properties::{PackageTables, derive_co_types, derive_from_footer};

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
    // Sorted and deduplicated, so a derived Item is byte-stable run to run.
    let mut sorted = props.lods.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(props.lods, sorted, "lods must be sorted and deduplicated");
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

    let json_typed: Vec<_> = props
        .attributes
        .iter()
        .filter(|a| a.attr_type == AttributeType::Object)
        .collect();
    assert!(
        !json_typed.is_empty(),
        "expected at least one Object-typed attribute from a JSON column; got {:?}",
        props
            .attributes
            .iter()
            .map(|a| (&a.name, a.attr_type))
            .collect::<Vec<_>>()
    );
}
