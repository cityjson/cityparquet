//! End-to-end coverage of the `ModuleKey`-resolution hard-error path (spec
//! "extensions" — "A class with no resolvable `ModuleKey` ... is a hard
//! error") and the `object_type` namespace-prefix rule (spec "Extensions" —
//! an extension class's `object_type` carries the extension's namespace in
//! place of CityJSON's `+` marker), both driven through the
//! real `convert()` pipeline rather than the pure-function/synthetic-batch
//! unit tests already covering `resolve_module_key` in isolation.
//!
//! `crate::package::extension_registry` is an intentional stub that always
//! returns an empty `ExtensionRegistry` (no Extension/ADE schema parsing
//! yet), so a genuine `+`-marked CityJSON Extension type with no core-class
//! match always hard-errors today — proven by
//! [`unresolvable_extension_type_is_a_clean_schema_error`]. The only
//! `+`-marked type that resolves successfully through today's pipeline is one
//! whose stripped name is itself a recognised core CityGML 3.0 class (spec:
//! resolution recognises a core class by either its CityJSON or CityGML
//! spelling, `+`-marker-insensitively) — proven by
//! [`extension_marked_core_class_routes_to_the_core_file_with_a_prefixed_type`].
//!
//! Both fixtures are real, hand-crafted, on-disk CityJSON files under
//! `tests/data/` (this repo's testing discipline forbids inline CityJSON in a
//! `.rs` test) — a minimal header declaring one extension (so its `+` names
//! can be attributed to a namespace) plus one `CityJSONFeature` carrying a
//! single small box `Solid`, structurally mirroring the smallest fixtures
//! already committed there (`helsinki_address.city.jsonl`,
//! `collision_attr.city.jsonl`).

use std::path::PathBuf;

use arrow_array::types::Int32Type;
use arrow_array::{Array, DictionaryArray, StringArray};
use cityparquet::CityParquetError;
use cityparquet::export::{ExportOptions, export};
use cityparquet::package::{ConvertOptions, convert};
use cityparquet::source::Source;
use cityparquet::stac::properties::PackageTables;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

/// A committed, in-tree fixture under `crates/core/tests/data/` (small
/// hand-derived inputs with no public download URL — see
/// `roundtrip_real_data`'s `data_fixture`).
fn data_fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(name);
    assert!(p.exists(), "missing committed fixture {name} in tests/data");
    p
}

/// Every `object_type` dictionary value actually referenced by `path`'s rows.
fn object_types(path: &std::path::Path) -> Vec<String> {
    let file = std::fs::File::open(path).unwrap();
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
    let reader = builder.build().unwrap();
    let mut out = Vec::new();
    for batch in reader {
        let batch = batch.unwrap();
        let col = batch.column_by_name("object_type").unwrap();
        let dict: &DictionaryArray<Int32Type> = col.as_any().downcast_ref().unwrap();
        let values: &StringArray = dict.values().as_any().downcast_ref().unwrap();
        for row in 0..batch.num_rows() {
            let key = dict.keys().value(row) as usize;
            out.push(values.value(key).to_string());
        }
    }
    out
}

/// `extension_module_unresolvable.city.jsonl` declares the `Noise`
/// extension and carries one `+NoiseSensor` object: not a recognised core
/// CityGML 3.0 class, and (since
/// `extension_registry` is a stub returning an always-empty registry) not
/// declared by any extension either — `resolve_module_key`'s hard-error path
/// (spec "extensions"). `convert()` must surface this as a clean
/// `CityParquetError::Schema` naming the unresolved class, never a panic and
/// never a silently-dropped row, and must leave no partial package behind.
#[test]
fn unresolvable_extension_type_is_a_clean_schema_error() {
    let out = tempfile::tempdir().unwrap();
    let opts = ConvertOptions::new(
        data_fixture("extension_module_unresolvable.city.jsonl"),
        out.path().to_path_buf(),
    );

    let err = convert(&opts).expect_err("an unresolvable ModuleKey must reject the conversion");

    assert!(
        matches!(err, CityParquetError::Schema(_)),
        "expected a Schema error, got: {err:?}"
    );
    assert!(
        format!("{err}").contains("NoiseSensor"),
        "error must name the unresolved class, got: {err}"
    );
    assert!(
        !out.path().join("metadata.json").exists(),
        "a rejected conversion must leave no package behind"
    );
}

/// `extension_module_resolves_to_core.city.jsonl` declares the `Energy`
/// extension and carries one `+Building` object: not a taxonomy
/// `cityjson_type` exact match (the stored `+` marker makes the lookup
/// miss), but its `+`-stripped name IS a recognised core CityJSON spelling,
/// so `resolve_module_key` resolves it straight to `Core(Building)` without
/// needing any extension class declaration — the conversion succeeds and the
/// row lands in `building.parquet`.
///
/// The written `object_type` cell must be `"energy_Building"`: the class is
/// the extension's, so it carries the extension's namespace (spec
/// "Extensions"), which is what lets export restore `"+Building"`.
#[test]
fn extension_marked_core_class_routes_to_the_core_file_with_a_prefixed_type() {
    let out = tempfile::tempdir().unwrap();
    let opts = ConvertOptions::new(
        data_fixture("extension_module_resolves_to_core.city.jsonl"),
        out.path().to_path_buf(),
    );

    let report = convert(&opts).expect("a +-marked core-class specialisation must resolve");
    assert_eq!(report.object_count, 1);

    let tables = PackageTables::open(out.path()).expect("resolve tables");
    assert_eq!(
        tables.tables.len(),
        1,
        "the sole object must land in exactly one table"
    );
    assert_eq!(
        tables.tables[0].file_name().unwrap().to_string_lossy(),
        "building.parquet",
        "a +Building object must resolve into the Building module's table"
    );

    let types = object_types(&tables.tables[0]);
    assert_eq!(
        types,
        vec!["energy_Building".to_string()],
        "object_type must carry the extension's namespace in place of the CityJSON + marker"
    );

    let exported = out.path().join("export.city.jsonl");
    export(&ExportOptions {
        package_dir: out.path().to_path_buf(),
        output: exported.clone(),
    })
    .expect("export");
    let source = Source::open(&exported).unwrap();
    let feature = source.features().unwrap().next().unwrap().unwrap();
    assert_eq!(
        feature.city_objects["EXT_BUILDING_1"].thetype, "+Building",
        "export restores the CityJSON + marker from the namespace prefix"
    );
}
