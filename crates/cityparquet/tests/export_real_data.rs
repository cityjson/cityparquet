//! RED (M3 task 6): export — package -> CityJSON/CityJSONSeq, exercised
//! against real converted delft/railway packages.

use std::path::PathBuf;

use cityparquet::export::{ExportOptions, export};
use cityparquet::package::{ConvertOptions, convert};
use cityparquet::reader::CityParquetReaderBuilder;
use cityparquet::source::{Source, SourceFormat};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

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
    PathBuf,
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
    (report, exported, original, output, package_dir, export_dir)
}

/// Counts `"material"`/`"texture"` keys across every geometry of every
/// feature line of an exported Seq file, walked as raw JSON (not via
/// cjseq's typed `Geometry`, so nothing a lenient deserialiser might drop
/// can mask a present key).
fn count_geometry_appearance_keys(output: &std::path::Path) -> (usize, usize) {
    let text = std::fs::read_to_string(output).unwrap();
    let mut mat = 0usize;
    let mut tex = 0usize;
    for line in text.lines().skip(1) {
        let feature: serde_json::Value = serde_json::from_str(line).unwrap();
        for co in feature["CityObjects"].as_object().unwrap().values() {
            let Some(geoms) = co.get("geometry").and_then(|g| g.as_array()) else {
                continue;
            };
            for geom in geoms {
                let geom = geom.as_object().unwrap();
                mat += usize::from(geom.contains_key("material"));
                tex += usize::from(geom.contains_key("texture"));
            }
        }
    }
    (mat, tex)
}

#[test]
fn delft_exports_back_to_a_seq_matching_the_source_header_and_counts() {
    let (report, exported, original, _output, _package_dir, _export_dir) =
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
    assert_eq!(
        report.appearance_refs_dropped, 0,
        "recounted from the fixture: no delft geometry carries material or texture"
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
    let (report, exported, _original, output, _package_dir, _export_dir) =
        convert_and_export("lod3_railway.city.json");

    assert_eq!(exported.format(), SourceFormat::CityJsonSeq);
    assert_eq!(report.object_count, 121);
    assert_eq!(
        report.instance_geometries_dropped, 15,
        "the recount in decode_real_data.rs: exactly 15 objects carry a template"
    );

    // Recounted with python3 over the fixture, replaying the writer's
    // binding rules (per-(object, LoD) first geometry kept, GeometryInstance
    // excluded — the dataset's only LoD is "3"): 105 stored geometries, of
    // which 24 carry `material`, 95 carry `texture`, and 105 carry at least
    // one of the two. Core-profile packages store the index maps but not the
    // appearance definitions (M4 sidecars), so export must DROP them all —
    // exporting a dangling index map would be invalid CityJSON.
    assert_eq!(
        report.appearance_refs_dropped, 105,
        "every stored railway geometry carries material or texture (the recount above)"
    );
    let (mat_keys, tex_keys) = count_geometry_appearance_keys(&output);
    assert_eq!(
        (mat_keys, tex_keys),
        (0, 0),
        "exported geometries must not carry dangling material/texture index maps"
    );

    let mut object_count = 0usize;
    for feature in exported.features().unwrap() {
        let feature = feature.unwrap();
        object_count += feature.city_objects.len();
    }
    assert_eq!(object_count, 121);
}

/// M4 task 5: the source header's `metadata` object (title,
/// geographicalExtent, etc.) is captured verbatim into the package's KV
/// metadata (`source_metadata`) and restored into the exported header.
/// `fullMetadataUrl` is a documented exception — cjseq's `Metadata` struct
/// has no passthrough for unknown members, so it never survives even the
/// initial parse of the source header, let alone the round trip.
#[test]
fn delft_source_metadata_reaches_kv_metadata_and_the_exported_header() {
    let package_dir = tempfile::tempdir().unwrap();
    convert(&ConvertOptions::new(
        fixture("delft.city.jsonl"),
        package_dir.path().to_path_buf(),
    ))
    .unwrap();

    let file = std::fs::File::open(package_dir.path().join("cityobjects.parquet")).unwrap();
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
    let meta = builder.cityparquet_metadata().unwrap();
    let source_metadata = meta
        .source_metadata
        .as_ref()
        .expect("delft's header sets metadata; source_metadata must be populated");
    assert_eq!(source_metadata["title"], serde_json::json!("3DBAG"));
    assert!(
        source_metadata.get("geographicalExtent").is_some(),
        "expected geographicalExtent in {source_metadata}"
    );
    assert!(
        source_metadata.get("fullMetadataUrl").is_none(),
        "fullMetadataUrl is not part of cjseq::Metadata and cannot survive"
    );

    let export_dir = tempfile::tempdir().unwrap();
    let output = export_dir.path().join("export.city.jsonl");
    export(&ExportOptions {
        package_dir: package_dir.path().to_path_buf(),
        output: output.clone(),
    })
    .unwrap();

    let exported_header_line = std::fs::read_to_string(&output)
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .to_string();
    let exported_header: serde_json::Value = serde_json::from_str(&exported_header_line).unwrap();
    let source_header_line = std::fs::read_to_string(fixture("delft.city.jsonl"))
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .to_string();
    let source_header: serde_json::Value = serde_json::from_str(&source_header_line).unwrap();

    assert_eq!(
        exported_header["metadata"]["title"], source_header["metadata"]["title"],
        "exported header title must match the source"
    );
    assert_eq!(
        exported_header["metadata"]["geographicalExtent"],
        source_header["metadata"]["geographicalExtent"],
        "exported header geographicalExtent must match the source"
    );
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
