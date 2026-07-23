//! Per-module Arrow/Parquet schema pruning (spec-alignment, M2 follow-up):
//! each module's object table carries only the geometry/appearance columns
//! ITS OWN rows actually populate, not the dataset-wide union — and `export`
//! must decode each table with its own schema/metadata, not the first
//! table's (the companion fix this pruning makes non-optional).
//!
//! `delft.city.jsonl` alone is a single module (Building only); the real
//! `lod3_railway.city.json` fixture alone spans many modules but at one
//! uniform LoD (`3`) everywhere, so neither fixture on its own can prove two
//! modules genuinely need DIFFERENT column sets. [`delft_and_railway_merged`]
//! merges both real fixtures (via `merge_sources`, the same mechanism
//! `crate::partition` uses) into one dataset where `building.parquet` (fed
//! by BOTH fixtures) needs the union of delft's LoDs (0, 1.2, 1.3, 2.2) and
//! railway's (3), while a railway-only module needs LoD 3 alone — this
//! repo's testing discipline forbids inline hand-written CityJSON, so this
//! mutates/merges real, on-disk fixture data exactly as `merge_real_data.rs`
//! already does for its own CRS-mismatch fixtures.

use std::fs;
use std::path::{Path, PathBuf};

use cityparquet::compare::{CompareOptions, Exclusions, compare_datasets};
use cityparquet::export::{ExportOptions, export};
use cityparquet::merge::merge_sources;
use cityparquet::package::{ConvertOptions, convert, convert_source};
use cityparquet::partition::{PartitionSpec, convert_partitioned};
use cityparquet::reader::CityParquetReaderBuilder;
use cityparquet::source::{Source, SourceFormat};
use cjseq::CityJSONFeature;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

/// Real delft (Building only, LoDs {0, 1.2, 1.3, 2.2}) merged with real
/// railway (10 CityGML modules, all at LoD 3) into one CityJSONSeq source.
/// Railway's `referenceSystem` is forced to delft's own — a test-only
/// override so `merge_sources` doesn't reject the pairing on CRS mismatch
/// (spatial/coordinate correctness is irrelevant to what this suite proves:
/// per-module COLUMN SET pruning and export's per-table decode; `merge_sources`
/// requantises real coordinates onto a shared transform regardless of the CRS
/// label, exactly as `merge_real_data.rs`'s own heterogeneous-transform tests
/// already exercise). Written to `dst` as CityJSONSeq so the standard
/// `convert`/`export`/`compare_datasets` path applies unchanged.
fn write_delft_and_railway_merged(dst: &Path) {
    let delft = Source::open(&fixture("delft.city.jsonl")).unwrap();

    let railway_src = Source::open(&fixture("lod3_railway.city.json")).unwrap();
    let mut railway_header = railway_src.header().clone();
    let delft_crs = delft
        .header()
        .metadata
        .as_ref()
        .and_then(|m| m.reference_system.clone());
    railway_header
        .metadata
        .as_mut()
        .expect("railway fixture carries a metadata block")
        .reference_system = delft_crs;
    let railway_feats: Vec<CityJSONFeature> = railway_src
        .features()
        .unwrap()
        .map(|f| f.unwrap())
        .collect();
    let railway = Source::from_parts(
        railway_header,
        railway_feats,
        railway_src.doc_appearance().cloned(),
        SourceFormat::CityJsonSeq,
    );

    let merged = merge_sources(&[delft, railway]).unwrap();

    let mut out = serde_json::to_string(&merged.header).unwrap();
    out.push('\n');
    for f in &merged.features {
        out.push_str(&serde_json::to_string(f).unwrap());
        out.push('\n');
    }
    fs::write(dst, out).unwrap();
}

/// Convert `input` and return the package dir (kept alive) plus the written
/// tables' bare file names.
fn convert_to_package(input: &Path) -> (tempfile::TempDir, Vec<String>) {
    let package_dir = tempfile::tempdir().unwrap();
    let report = convert(&ConvertOptions::new(
        input.to_path_buf(),
        package_dir.path().to_path_buf(),
    ))
    .unwrap();
    let names: Vec<String> = report
        .files
        .iter()
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("parquet"))
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    (package_dir, names)
}

/// The rendered `geometry_lod*` column names present in `table`'s own
/// physical Arrow/Parquet schema (not row contents) — the direct, on-disk
/// proof [`module_files_carry_only_their_own_lod_columns`] needs.
fn geometry_lod_columns(table: &Path) -> Vec<String> {
    let file = fs::File::open(table).unwrap();
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
    let schema = builder.cityparquet_arrow_schema().unwrap();
    let mut names: Vec<String> = schema
        .fields()
        .iter()
        .filter(|f| f.name().starts_with("geometry_lod"))
        .map(|f| f.name().clone())
        .collect();
    names.sort();
    names
}

/// Checklist item 1: a module file's rendered Arrow/Parquet schema contains
/// ONLY the LoD/appearance columns its own rows actually use, asserted
/// against the PHYSICAL Parquet schema (not row contents) of two real
/// modules with genuinely different column needs — `building.parquet` (fed
/// by both delft's higher-LoD buildings and railway's LoD-3 ones) versus a
/// railway-only module (LoD 3 alone).
#[test]
fn module_files_carry_only_their_own_lod_columns() {
    let src_dir = tempfile::tempdir().unwrap();
    let src_path = src_dir.path().join("delft_and_railway.city.jsonl");
    write_delft_and_railway_merged(&src_path);

    let (package_dir, tables) = convert_to_package(&src_path);
    assert!(
        tables.contains(&"building.parquet".to_string()),
        "expected a building.parquet table, got {tables:?}"
    );
    assert!(
        tables.contains(&"transportation.parquet".to_string()),
        "expected a railway-only transportation.parquet table (Railway type), got {tables:?}"
    );

    let building_lods = geometry_lod_columns(&package_dir.path().join("building.parquet"));
    let transportation_lods =
        geometry_lod_columns(&package_dir.path().join("transportation.parquet"));

    assert_eq!(
        building_lods,
        vec![
            "geometry_lod0_0".to_string(),
            "geometry_lod1_2".to_string(),
            "geometry_lod1_3".to_string(),
            "geometry_lod2_2".to_string(),
            "geometry_lod3_0".to_string(),
        ],
        "building.parquet must carry the UNION of delft's LoDs and railway's Building LoD"
    );
    assert_eq!(
        transportation_lods,
        vec!["geometry_lod3_0".to_string()],
        "transportation.parquet (Railway objects only) must carry ONLY LoD 3 — none of \
         building.parquet's other LoD columns"
    );
    assert_ne!(
        building_lods, transportation_lods,
        "the two modules must have genuinely different column needs"
    );

    // spec-alignment M3, checklist item 6: the FOOTER `city.columns`/`geo`
    // must genuinely differ per file too — never a dataset-wide union
    // stamped identically onto every table (spec "The footer describes the
    // file it lives in — nothing wider").
    let building_city = footer_city(&package_dir.path().join("building.parquet"));
    let transportation_city = footer_city(&package_dir.path().join("transportation.parquet"));
    let building_cols: Vec<&str> = building_city["columns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    let transportation_cols: Vec<&str> = transportation_city["columns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    assert_ne!(
        building_cols, transportation_cols,
        "city.columns must genuinely differ between the two module files"
    );
    assert_eq!(
        building_city["primary_column"], "geometry_lod3_0",
        "building.parquet's own primary is its own highest LoD, not the whole dataset's"
    );
    assert_eq!(transportation_city["primary_column"], "geometry_lod3_0");
}

/// This table's own `city` footer key, parsed.
fn footer_city(table: &Path) -> serde_json::Value {
    let file = fs::File::open(table).unwrap();
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
    let kvs = builder
        .metadata()
        .file_metadata()
        .key_value_metadata()
        .unwrap();
    let city_kv = kvs
        .iter()
        .find(|kv| kv.key == "city")
        .expect("table must carry a city footer key");
    serde_json::from_str(city_kv.value.as_deref().unwrap()).unwrap()
}

/// Checklist item 2: the test that would have caught the `export.rs` bug
/// before the fix — a dataset spanning two modules with genuinely different
/// LoD sets still `convert -> export -> compare`s successfully. RED before
/// the export.rs/reader.rs per-table fixes (export rejected the package
/// outright, treating the legitimate per-module schema difference as
/// corruption); GREEN after.
#[test]
fn convert_export_compare_round_trips_a_dataset_with_genuinely_different_per_module_lod_sets() {
    let src_dir = tempfile::tempdir().unwrap();
    let src_path = src_dir.path().join("delft_and_railway.city.jsonl");
    write_delft_and_railway_merged(&src_path);

    let package_dir = tempfile::tempdir().unwrap();
    convert(&ConvertOptions::new(
        src_path.clone(),
        package_dir.path().to_path_buf(),
    ))
    .unwrap();

    let export_dir = tempfile::tempdir().unwrap();
    let output = export_dir.path().join("export.city.jsonl");
    export(&ExportOptions {
        package_dir: package_dir.path().to_path_buf(),
        output: output.clone(),
    })
    .expect(
        "export must decode a package whose modules have genuinely different schemas, \
         each table against its own metadata",
    );

    // Core profile (the default `ConvertOptions`) stores no appearance
    // sidecars and drops `GeometryInstance` geometry (see `export`'s module
    // doc) — the same documented, expected drops
    // `railway_round_trips_losslessly_modulo_documented_drops` already
    // excludes for railway alone; the merged dataset inherits railway's
    // appearance/instances, so the same exclusions apply here.
    let opts = CompareOptions {
        coord_tolerance: [0.0; 3],
        exclusions: Exclusions {
            appearance: true,
            geometry_instances: true,
        },
    };
    let compare_report = compare_datasets(&src_path, &output, &opts).unwrap();
    assert!(
        compare_report.equal,
        "a merged multi-module, multi-LoD-set dataset must round-trip losslessly (modulo the \
         documented Core-profile appearance/instance drops); differences: {:#?}",
        compare_report.differences
    );
    assert!(compare_report.differences.is_empty());
}

/// Checklist item 3: a spatially-partitioned convert still produces correct
/// per-module schemas across partitions where a module is present in some
/// partitions and absent in others. Delft's real-world coordinates (Dutch RD
/// New, ~84500/445800) and railway's (a small local scene near the origin)
/// are so far apart that any reasonable spatial grid separates them into
/// different partitions — this exercises `CanonicalSchema::module_lods`
/// (every partition must prune each module identically, since a glob read
/// across partitions needs one uniform schema per module file).
#[test]
fn partitioned_convert_prunes_consistently_when_a_module_is_absent_from_some_partitions() {
    let src_dir = tempfile::tempdir().unwrap();
    let src_path = src_dir.path().join("delft_and_railway.city.jsonl");
    write_delft_and_railway_merged(&src_path);
    let source = Source::open(&src_path).unwrap();

    let out_dir = tempfile::tempdir().unwrap();
    let opts = ConvertOptions::new(src_path.clone(), out_dir.path().to_path_buf());
    let report =
        convert_partitioned(&[source], &PartitionSpec::Box { cell: 10_000.0 }, &opts).unwrap();
    assert!(
        report.partitions.len() >= 2,
        "delft (Dutch RD New coordinates) and railway (a scene near the origin) must land in \
         different spatial cells, got {} partition(s)",
        report.partitions.len()
    );

    let module_sets: Vec<(String, std::collections::BTreeSet<String>)> = report
        .partitions
        .iter()
        .map(|(label, r)| {
            let names: std::collections::BTreeSet<String> = r
                .files
                .iter()
                .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("parquet"))
                .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
                .collect();
            (label.clone(), names)
        })
        .collect();

    // A railway-only module (present in the railway partition, absent from
    // the delft-only one) proves "a module present in some partitions but
    // absent in others".
    assert!(
        module_sets
            .iter()
            .any(|(_, s)| s.contains("water_body.parquet")),
        "at least one partition must carry the railway-only water_body.parquet module: {module_sets:?}"
    );
    assert!(
        module_sets
            .iter()
            .any(|(_, s)| !s.contains("water_body.parquet")),
        "at least one partition must NOT carry water_body.parquet: {module_sets:?}"
    );

    // building.parquet (fed by both delft and railway) is present in every
    // partition that carries it at all with a self-consistent, readable
    // schema — and, since a glob read across partitions needs ONE uniform
    // schema per module file, its FULL column set (not just that each
    // partition's own schema resolves) must be IDENTICAL across every
    // partition that carries it. This is the actual `CanonicalSchema::
    // module_lods` claim this test's doc comment makes: per-module pruning
    // must agree across partitions, not merely succeed within each one.
    let mut building_schemas: Vec<(String, Vec<String>)> = Vec::new();
    for (label, names) in &module_sets {
        if names.contains("building.parquet") {
            let path = out_dir.path().join(label).join("building.parquet");
            let file = fs::File::open(&path).unwrap();
            let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
            let schema = builder
                .cityparquet_arrow_schema()
                .expect("building.parquet's own schema must still resolve after partitioning");
            let mut column_names: Vec<String> =
                schema.fields().iter().map(|f| f.name().clone()).collect();
            column_names.sort();
            building_schemas.push((label.clone(), column_names));
        }
    }
    assert!(
        building_schemas.len() >= 2,
        "expected building.parquet (fed by both delft and railway) in more than one \
         partition, got {building_schemas:?}"
    );
    let (first_label, first_columns) = &building_schemas[0];
    for (label, columns) in &building_schemas[1..] {
        assert_eq!(
            columns, first_columns,
            "building.parquet's rendered column set must be identical across every \
             partition that carries it — partition {label:?} disagrees with partition \
             {first_label:?}, which would break a glob read across partitions that needs \
             one uniform schema per module file"
        );
    }

    // Same claim, narrowed to the `geometry_lod*` columns specifically
    // (the ones `CanonicalSchema::module_lods` derives per module) using
    // this file's own helper, and expressed the way a reader would notice
    // the breakage: two partitions' `geometry_lod_columns()` must match.
    let building_lod_columns: Vec<Vec<String>> = module_sets
        .iter()
        .filter(|(_, names)| names.contains("building.parquet"))
        .map(|(label, _)| {
            geometry_lod_columns(&out_dir.path().join(label).join("building.parquet"))
        })
        .collect();
    for pair in building_lod_columns.windows(2) {
        assert_eq!(
            pair[0], pair[1],
            "building.parquet's geometry_lod* columns must match across partitions"
        );
    }
}

/// `convert_source` (the already-open-`Source` entry point `convert_partitioned`
/// itself builds on) round-trips the merged dataset too — a second,
/// lighter-weight proof alongside checklist item 2's `convert`-path version,
/// exercising the API surface `crate::partition` actually calls.
#[test]
fn convert_source_of_the_merged_dataset_still_reports_the_full_object_count() {
    let src_dir = tempfile::tempdir().unwrap();
    let src_path = src_dir.path().join("delft_and_railway.city.jsonl");
    write_delft_and_railway_merged(&src_path);
    let source = Source::open(&src_path).unwrap();

    let package_dir = tempfile::tempdir().unwrap();
    let report = convert_source(
        &source,
        &ConvertOptions::new(src_path, package_dir.path().to_path_buf()),
    )
    .unwrap();

    // delft: 2231 objects (1115 Building + 1116 BuildingPart); railway: 121.
    assert_eq!(report.object_count, 2231 + 121);
}
