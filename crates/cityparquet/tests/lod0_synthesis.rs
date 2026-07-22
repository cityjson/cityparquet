//! LoD0 synthesis over real fixtures: the cjseq adapters + the existing WKB
//! writer must turn a source solid into a GeoParquet-legal `MultiPolygon Z`
//! footprint.

use std::path::PathBuf;

use arrow_array::Array;
use cityparquet::compare::{CompareOptions, compare_datasets};
use cityparquet::export::{ExportOptions, export};
use cityparquet::lod0::{Lod0Options, faces_from_geometry, footprint_to_geometry, synthesize_lod0};
use cityparquet::package::{ConvertOptions, convert};
use cityparquet::source::Source;
use cityparquet::stac::properties::PackageTables;
use cityparquet::wkb_write::{VertexPool, geometry_to_wkb};
use cjseq::GeometryType;
use geo_traits::{GeometryTrait, GeometryType as GtGeometryType};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

/// `metadata.json`'s object-table file names for the package at `dir`
/// (`PackageTables::open`'s `cityparquet-objects`-role assets) — by-type is
/// the only, mandatory table layout, so this is 1..N main-table file names,
/// one per 1st-level CityObject family actually present.
fn manifest_tables(dir: &std::path::Path) -> Vec<String> {
    PackageTables::open(dir)
        .unwrap()
        .tables
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect()
}

/// Every synthesised footprint, fed back through the hand-rolled WKB writer,
/// must parse as an ISO `MultiPolygon` (type 1006) — the GeoParquet-legal shape
/// the `geometry` column needs — with a valid little-endian header.
#[test]
fn delft_solids_synthesise_valid_multipolygon_footprints() {
    let src = Source::open(&fixture("delft.city.jsonl")).unwrap();
    let header = src.header();
    let opts = Lod0Options::default();
    let mut synthesised = 0usize;

    for feature in src.features().unwrap() {
        let feature = feature.unwrap();
        let pool = VertexPool::new(&feature.vertices, &header.transform);
        for co in feature.city_objects.values() {
            let Some(geoms) = &co.geometry else {
                continue;
            };
            for geom in geoms {
                if !matches!(
                    geom.thetype,
                    GeometryType::Solid
                        | GeometryType::MultiSolid
                        | GeometryType::CompositeSolid
                        | GeometryType::MultiSurface
                        | GeometryType::CompositeSurface
                ) {
                    continue;
                }
                let (faces, mask) = faces_from_geometry(geom, &pool).unwrap();
                if faces.is_empty() {
                    continue;
                }
                let Some(fp) = synthesize_lod0(&faces, mask.as_deref(), &opts) else {
                    continue;
                };
                assert!(
                    !fp.surfaces.is_empty(),
                    "a footprint has at least one surface"
                );

                let (verts, ms) = footprint_to_geometry(&fp);
                let raw = VertexPool::raw(&verts);
                let outcome = geometry_to_wkb(&ms, &raw)
                    .unwrap()
                    .expect("a non-empty footprint yields WKB");
                assert_eq!(outcome.bytes[0], 0x01, "little-endian WKB marker");
                let parsed =
                    wkb::reader::read_wkb(&outcome.bytes).expect("the footprint WKB must parse");
                assert!(
                    matches!(parsed.as_type(), GtGeometryType::MultiPolygon(_)),
                    "a synthesised footprint must be a MultiPolygon (GeoParquet-legal)"
                );
                synthesised += 1;
                if synthesised >= 8 {
                    return; // a representative handful is enough
                }
            }
        }
    }
    assert!(
        synthesised > 0,
        "at least one delft geometry should synthesise a footprint"
    );
}

/// Convert `lod3_railway` (LoD3 solids only, no source LoD0) with synthesis
/// on/off, returning the package dir.
fn convert_railway(generate_lod0: bool) -> tempfile::TempDir {
    let pkg = tempfile::tempdir().unwrap();
    let mut opts = ConvertOptions::new(fixture("lod3_railway.city.json"), pkg.path().to_path_buf());
    opts.generate_lod0 = generate_lod0;
    convert(&opts).unwrap();
    pkg
}

/// Synthesis (opt-in) populates the primary `geometry` column for a Solid-only
/// dataset that has no source LoD0; disabling it leaves no such column.
///
/// railway has 10 1st-level families, so by-type conversion writes 10 main
/// tables — every table shares the IDENTICAL schema (the by-type writer
/// partitions strictly after encode), so the schema-level checks below only
/// need the first table, but the non-null footprint count must be summed
/// across every table (a synthesised footprint can land in any of them).
#[test]
fn synthesis_adds_a_primary_geometry_footprint_to_a_solid_only_dataset() {
    let with = convert_railway(true);
    let with_tables = manifest_tables(with.path());
    let first_file = std::fs::File::open(with.path().join(&with_tables[0])).unwrap();
    let first_builder = ParquetRecordBatchReaderBuilder::try_new(first_file).unwrap();
    assert!(
        first_builder
            .schema()
            .field_with_name("geometry_lod0_0")
            .is_ok(),
        "synthesis reserves the suffixed geometry_lod0_0 column"
    );
    let mut non_null = 0usize;
    for table in &with_tables {
        let file = std::fs::File::open(with.path().join(table)).unwrap();
        let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
        for batch in builder.build().unwrap() {
            let batch = batch.unwrap();
            let g = batch.column_by_name("geometry_lod0_0").unwrap();
            non_null += batch.num_rows() - g.null_count();
        }
    }
    assert!(non_null > 0, "at least one synthesised LoD0 footprint");

    let without = convert_railway(false);
    let without_tables = manifest_tables(without.path());
    let file = std::fs::File::open(without.path().join(&without_tables[0])).unwrap();
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
    assert!(
        builder.schema().field_with_name("geometry_lod0_0").is_err(),
        "no synthesised LoD0 column without opt-in"
    );
}

/// Synthesis is idempotent: exporting a synthesised package yields real `lod:"0"`
/// geometries, and reconverting that export with synthesis on adds nothing new,
/// so a second round trip reproduces the first exactly.
#[test]
fn synthesis_is_idempotent_through_a_round_trip() {
    let pkg1 = convert_railway(true);
    let dir1 = tempfile::tempdir().unwrap();
    let export1 = dir1.path().join("export1.city.jsonl");
    export(&ExportOptions {
        package_dir: pkg1.path().to_path_buf(),
        output: export1.clone(),
    })
    .unwrap();
    assert!(
        std::fs::read_to_string(&export1)
            .unwrap()
            .contains("\"lod\":\"0.0\""),
        "synthesised LoD0 is exported as a real lod 0.0 geometry (canonical spelling)"
    );

    // Reconvert the enriched export (now carrying a real LoD0) with synthesis
    // still on, and export again: the second synthesis pass is a no-op.
    let pkg2 = tempfile::tempdir().unwrap();
    let mut opts = ConvertOptions::new(export1.clone(), pkg2.path().to_path_buf());
    opts.generate_lod0 = true;
    convert(&opts).unwrap();
    let dir2 = tempfile::tempdir().unwrap();
    let export2 = dir2.path().join("export2.city.jsonl");
    export(&ExportOptions {
        package_dir: pkg2.path().to_path_buf(),
        output: export2.clone(),
    })
    .unwrap();

    let report = compare_datasets(&export1, &export2, &CompareOptions::default()).unwrap();
    assert!(
        report.equal,
        "second synthesis pass must be a no-op; differences: {:#?}",
        report.differences
    );
}
