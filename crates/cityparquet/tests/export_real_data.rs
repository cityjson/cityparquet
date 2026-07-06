//! RED (M3 task 6): export — package -> CityJSON/CityJSONSeq, exercised
//! against real converted delft/railway packages.

use std::path::PathBuf;

use cityparquet::export::{ExportOptions, export};
use cityparquet::package::{ConvertOptions, convert};
use cityparquet::source::{Source, SourceFormat};

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

/// Converts `input` into a fresh tempdir package, then exports it back to
/// `.city.jsonl` in a second tempdir. Returns the export report plus the
/// re-opened export `Source` and the original `Source` (both kept alive so
/// callers can compare headers/features), alongside the tempdirs backing
/// them — the export `Source` re-opens its file on every `features()` call,
/// so the tempdir must outlive the whole test, not just this function.
fn convert_and_export(
    input: &str,
) -> (
    cityparquet::export::ExportReport,
    Source,
    Source,
    tempfile::TempDir,
    tempfile::TempDir,
) {
    let package_dir = tempfile::tempdir().unwrap();
    convert(&ConvertOptions::new(
        fixture(input),
        package_dir.path().to_path_buf(),
    ))
    .unwrap();

    let export_dir = tempfile::tempdir().unwrap();
    let output = export_dir.path().join("export.city.jsonl");
    let report = export(&ExportOptions {
        package_dir: package_dir.path().to_path_buf(),
        output: output.clone(),
    })
    .unwrap();

    let exported = Source::open(&output).unwrap();
    let original = Source::open(&fixture(input)).unwrap();
    (report, exported, original, package_dir, export_dir)
}

#[test]
fn delft_exports_back_to_a_seq_matching_the_source_header_and_counts() {
    let (report, exported, original, _package_dir, _export_dir) =
        convert_and_export("delft.city.jsonl");

    assert_eq!(exported.format(), SourceFormat::CityJsonSeq);

    // Exact JSON equality on the transform (compares scale/translate f64
    // vectors verbatim, not just a re-derived Lod string).
    let exported_transform = serde_json::to_value(&exported.header().transform).unwrap();
    let original_transform = serde_json::to_value(&original.header().transform).unwrap();
    assert_eq!(
        exported_transform, original_transform,
        "exported header transform must equal the source header transform exactly"
    );

    // referenceSystem survives as the same URL string.
    let exported_rs = exported
        .header()
        .metadata
        .as_ref()
        .and_then(|m| m.reference_system.as_ref())
        .map(cjseq::ReferenceSystem::to_url);
    let original_rs = original
        .header()
        .metadata
        .as_ref()
        .and_then(|m| m.reference_system.as_ref())
        .map(cjseq::ReferenceSystem::to_url);
    assert!(
        exported_rs.is_some(),
        "expected delft's source to carry a referenceSystem that survives export"
    );
    assert_eq!(exported_rs, original_rs);

    assert_eq!(report.feature_count, 1115);
    assert_eq!(report.object_count, 2231);
    assert_eq!(
        report.instance_geometries_dropped, 0,
        "delft has no GeometryInstance geometries"
    );

    // Every feature line parses via cjseq (Source::features() itself uses
    // CityJSONFeature::from_str, so a clean full iteration proves this) and
    // the feature/object counts recounted independently agree with the report.
    let mut feature_count = 0usize;
    let mut object_count = 0usize;
    for feature in exported.features().unwrap() {
        let feature = feature.unwrap();
        feature_count += 1;
        object_count += feature.city_objects.len();
    }
    assert_eq!(feature_count, 1115);
    assert_eq!(object_count, 2231);
}

#[test]
fn railway_exports_dropping_instance_geometries_but_keeping_their_objects() {
    let (report, exported, _original, _package_dir, _export_dir) =
        convert_and_export("lod3_railway.city.json");

    assert_eq!(exported.format(), SourceFormat::CityJsonSeq);
    assert_eq!(report.object_count, 121);
    assert_eq!(
        report.instance_geometries_dropped, 15,
        "the recount in decode_real_data.rs: exactly 15 objects carry a template"
    );

    let mut object_count = 0usize;
    for feature in exported.features().unwrap() {
        let feature = feature.unwrap();
        object_count += feature.city_objects.len();
    }
    assert_eq!(object_count, 121);
}

#[test]
fn delft_also_exports_as_a_single_whole_city_json_document() {
    let package_dir = tempfile::tempdir().unwrap();
    convert(&ConvertOptions::new(
        fixture("delft.city.jsonl"),
        package_dir.path().to_path_buf(),
    ))
    .unwrap();

    let export_dir = tempfile::tempdir().unwrap();
    let output = export_dir.path().join("export.city.json");
    let report = export(&ExportOptions {
        package_dir: package_dir.path().to_path_buf(),
        output: output.clone(),
    })
    .unwrap();
    assert_eq!(report.feature_count, 1115);

    let text = std::fs::read_to_string(&output).unwrap();
    let doc = cjseq::CityJSON::from_str(&text).expect("cjseq must parse the exported .city.json");
    assert_eq!(doc.thetype, "CityJSON");
    assert_eq!(doc.version, "2.0");
    assert_eq!(doc.number_of_city_objects(), 1115, "top-level objects only");
}
