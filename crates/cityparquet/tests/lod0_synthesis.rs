//! LoD0 synthesis over real fixtures: the cjseq adapters + the existing WKB
//! writer must turn a source solid into a GeoParquet-legal `MultiPolygon Z`
//! footprint.

use std::path::PathBuf;

use arrow_array::Array;
use cityparquet::compare::{CompareOptions, compare_datasets};
use cityparquet::export::{ExportOptions, export};
use cityparquet::lod0::{Lod0Options, faces_from_geometry, footprint_to_geometry, synthesize_lod0};
use cityparquet::package::{ConvertOptions, convert};
use cityparquet::reader::CityParquetReaderBuilder;
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

/// The real `lod3_railway.city.json` fixture carries no `referenceSystem` at
/// all. Since `scan` now hard-fails on coordinate-bearing input with no
/// resolvable CRS (spec "CRS rules"), [`convert_railway`] writes a small
/// on-disk COPY with a CRS injected via JSON mutation of the real fixture —
/// never hand-written CityJSON.
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

/// Convert `lod3_railway` (LoD3 solids only, no source LoD0) with synthesis
/// on/off, returning the package dir.
fn convert_railway(generate_lod0: bool) -> tempfile::TempDir {
    let pkg = tempfile::tempdir().unwrap();
    let (_crs_dir, railway_path) = railway_fixture_with_crs();
    let mut opts = ConvertOptions::new(railway_path, pkg.path().to_path_buf());
    opts.generate_lod0 = generate_lod0;
    convert(&opts).unwrap();
    pkg
}

/// Synthesis (opt-in) populates the primary `geometry` column for a Solid-only
/// dataset that has no source LoD0; disabling it leaves no such column.
///
/// railway has 10 1st-level families, so by-type conversion writes 10 main
/// tables. Since each module's own table now carries only the LoD columns
/// its own rows need (spec "object-table-schema"), the tables no longer
/// share one identical schema: a module with NO analysis geometry of its own
/// (e.g. `Vegetation`, whose real objects here carry none) has nothing to
/// synthesise a footprint FROM, so it gets no `geometry_lod0_0` column at
/// all — the non-null footprint count is still summed across every table
/// that DOES carry the column (a synthesised footprint can land in any of
/// them), and the first table (`building.parquet`, which does have solids)
/// is still checked directly for the column's presence.
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
        "synthesis reserves the suffixed geometry_lod0_0 column on the first (Building) table"
    );
    let mut non_null = 0usize;
    for table in &with_tables {
        let file = std::fs::File::open(with.path().join(table)).unwrap();
        let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
        for batch in builder.build().unwrap() {
            let batch = batch.unwrap();
            // A module with no analysis geometry of its own carries no
            // geometry_lod0_0 column at all — skip it rather than assume
            // every table shares the identical schema.
            let Some(g) = batch.column_by_name("geometry_lod0_0") else {
                continue;
            };
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

/// spec-alignment M3, checklist item 4 (variant: two independent selectors
/// disagreeing WITHOUT a solid involved): railway's LoD3 geometry is
/// `MultiSurface`/`CompositeSurface` — already GeoParquet-legal — so once
/// LoD0 synthesis adds a legal footprint, `city.primary_column` (the highest
/// LoD present, unconditionally — LoD3) and `geo.primary_column` (the `0.*`
/// family, preferred over any higher LoD even when that LoD is ALSO legal)
/// still genuinely differ. Both columns are legal here, so both appear in
/// `geo.columns` — this is `lod0_synthesis::synthesised_railway...`'s
/// companion in `scan_real_data.rs`'s `delft_city_and_geo_for_file_has_independent_primaries`,
/// which covers the Solid-bearing case.
#[test]
fn synthesised_railway_has_independent_city_and_geo_primaries() {
    let pkg = convert_railway(true);
    // building.parquet: railway's Buildings/BuildingParts carry both the
    // source LoD3 surfaces and the synthesised LoD0 footprint.
    let file = std::fs::File::open(pkg.path().join("building.parquet")).unwrap();
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
    let (city, geo) = builder.cityparquet_footer().unwrap();

    assert_eq!(
        city.primary_column.as_deref(),
        Some("geometry_lod3_0"),
        "city.primary_column must be the highest LoD present"
    );
    let geo = geo.expect("both LoD0 and LoD3 are GeoParquet-legal here");
    assert_eq!(
        geo.primary_column, "geometry_lod0_0",
        "geo.primary_column prefers the 0.* family even over a higher, also-legal LoD"
    );
    assert_ne!(
        city.primary_column.as_deref(),
        Some(geo.primary_column.as_str()),
        "city.primary_column and geo.primary_column must genuinely differ here"
    );
    // Both columns are legal, so both are declared in geo.columns too.
    assert!(geo.columns.contains_key("geometry_lod0_0"));
    assert!(geo.columns.contains_key("geometry_lod3_0"));
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
