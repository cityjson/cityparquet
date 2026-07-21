//! Derive `city3d:*` properties from real converted CityParquet packages.
//!
//! Every input here is a real CityJSON file converted by this crate — no
//! inline artificial CityJSON, per this repo's testing discipline.

use std::path::PathBuf;

use city3d_stac_types::metadata::AttributeType;
use city3d_stac_types::stac::CityObjectsCount;
use cityparquet::package::{ConvertOptions, convert};
use cityparquet::stac::properties::{PackageTables, derive_from_footer};

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
    // Filled by later tasks; must be untouched here so their absence is
    // visible rather than accidentally satisfied.
    assert!(
        props.co_types.is_empty(),
        "co_types belongs to a later task"
    );
    assert!(
        props.semantic_surfaces.is_none(),
        "semantic_surfaces belongs to a later task"
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
