//! Task 1 (mandatory-by-type-layout plan): characterization safety net.
//!
//! Pins the exact by-module object-table set the by-module layout produces
//! (spec "By-module object-table layout") — this test stayed green across
//! Task 3's removal of the single-file table layout (by-type/by-module is
//! now unconditional), proving the deletion did not perturb output by a
//! single byte. Updated for the ModuleKey-driven by-module split (spec
//! alignment): the file set is keyed by CityGML 3.0 module, not 1st-level
//! CityJSON family, so it is smaller and differently named than the old
//! by-family layout.

use std::collections::BTreeSet;
use std::path::PathBuf;

use cityparquet::package::{ConvertOptions, convert};

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

fn table_files(dir: &std::path::Path) -> BTreeSet<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".parquet"))
        .collect()
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

#[test]
fn railway_by_type_writes_one_file_per_citygml_module() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("pkg");
    let (_crs_dir, railway_path) = railway_fixture_with_crs();
    let opts = ConvertOptions::new(railway_path, out.clone());
    convert(&opts).expect("convert railway");

    let expected: BTreeSet<String> = [
        "bridge.parquet",
        "building.parquet",
        "city_furniture.parquet",
        "generics.parquet",
        "transportation.parquet",
        "vegetation.parquet",
        "relief.parquet",
        "tunnel.parquet",
        "water_body.parquet",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    // railway carries materials/textures/templates, now written whenever the
    // source has content for them (spec-alignment gap 19 dropped the
    // Profile choice this test used to rely on being Core-by-default) — the
    // sidecars aren't CityGML modules, so they're excluded from this
    // module-table-naming assertion.
    const SIDECARS: [&str; 3] = [
        "materials.parquet",
        "textures.parquet",
        "geometry_templates.parquet",
    ];
    let main_tables: BTreeSet<String> = table_files(&out)
        .into_iter()
        .filter(|f| !SIDECARS.contains(&f.as_str()))
        .collect();
    assert_eq!(
        main_tables,
        expected,
        "by-module must write exactly one file per CityGML module; 2nd-level types (and \
         CityObjectGroup, folded per spec) share their module's file"
    );
}

/// `tests/fixtures/empty.city.jsonl` is a single CityJSONSeq header line
/// (`CityObjects: {}`, no feature lines follow) — the minimal input that
/// scans to zero city-object rows (`scan_result.object_count == 0`).
/// `write_package` used to paper over a zero-row by-type run by writing a
/// standalone, empty reserved-name fallback table (the by-type writer opens
/// tables lazily per-family, so zero rows means zero writers ever open);
/// per plan decision (2026-07-21) that fallback is gone and this must be a
/// hard error instead, off `scan_result.object_count` — see
/// `crates/cityparquet/tests/convert_real_data.rs`'s
/// `empty_input_is_rejected` for the same assertion.
#[test]
fn zero_object_input_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("pkg");
    let opts = ConvertOptions::new(fixture("empty.city.jsonl"), out);
    let err = convert(&opts).unwrap_err();
    assert!(
        format!("{err}").contains("no city objects"),
        "a zero-object conversion must fail clearly, got: {err}"
    );
}
