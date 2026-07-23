use std::collections::HashSet;
use std::path::PathBuf;

use arrow_array::types::Int32Type;
use arrow_array::{Array, DictionaryArray, Float64Array, StringArray, StructArray};
use cityparquet::CityParquetError;
use cityparquet::compare::{CompareOptions, compare_datasets};
use cityparquet::export::{ExportOptions, export};
use cityparquet::order::hilbert_index;
use cityparquet::package::{ConvertOptions, RowOrder, convert};
use cityparquet::reader::CityParquetReaderBuilder;
use cityparquet::stac::properties::PackageTables;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::basic::Encoding;
use serde_json::Value;

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

/// The real `lod3_railway.city.json` fixture carries no `referenceSystem` at
/// all. Since `scan` now hard-fails on coordinate-bearing input with no
/// resolvable CRS (spec "CRS rules"), tests below that convert (or compare
/// against) railway use a small on-disk COPY with a CRS injected via JSON
/// mutation of the real fixture — never hand-written CityJSON. Used both as
/// the conversion INPUT and, where a test also compares against "the
/// source", as that comparison baseline (the pristine original has no CRS to
/// compare the export's restored referenceSystem against).
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

/// `metadata.json`'s object-table file names for the package at `dir`
/// (`PackageTables::open`'s `cityparquet-objects`-role assets) — by-type is
/// the only, mandatory table layout, so this is 1..N main-table file names,
/// one per 1st-level CityObject family actually present, never a single
/// hardcoded main-table name.
fn manifest_tables(dir: &std::path::Path) -> Vec<String> {
    PackageTables::open(dir)
        .unwrap()
        .tables
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect()
}

/// `convert_source` (the seam the merge/partition pipeline drives) must
/// reproduce `convert`'s result when handed the same source directly.
#[test]
fn convert_source_matches_convert_object_count() {
    let out = tempfile::tempdir().unwrap();
    let opts = ConvertOptions::new(fixture("delft.city.jsonl"), out.path().to_path_buf());
    let src = cityparquet::source::Source::open(&opts.input).unwrap();
    let report = cityparquet::package::convert_source(&src, &opts).unwrap();
    assert_eq!(report.object_count, 2231);
    assert!(out.path().join("metadata.json").exists());
}

/// Recursively asserts every material index in a rewritten `material` map is
/// a dataset-global id `< limit` (mirrors `cityparquet::appearance`'s own
/// walk, duplicated here since this is a separate integration-test crate
/// with no access to that module's private test helpers).
fn assert_every_material_index_below(v: &Value, limit: usize) {
    match v {
        Value::Object(map) => {
            for val in map.values() {
                assert_every_material_index_below(val, limit);
            }
        }
        Value::Array(items) => {
            for x in items {
                assert_every_material_index_below(x, limit);
            }
        }
        Value::Number(n) => {
            let idx = n.as_u64().expect("material index must be a u64") as usize;
            assert!(idx < limit, "global material id {idx} must be < {limit}");
        }
        Value::Null => {}
        other => panic!("unexpected material map node: {other}"),
    }
}

/// Recursively asserts every innermost ring in a rewritten `texture` map:
/// the texture id (first element) is `< limit` (or the ring is `[null]`),
/// and every following element is an inlined `[u, v]` pair.
fn assert_every_texture_ring_valid(v: &Value, limit: usize) {
    match v {
        Value::Object(map) => {
            for val in map.values() {
                assert_every_texture_ring_valid(val, limit);
            }
        }
        Value::Array(items) => {
            let is_ring = !items.is_empty() && matches!(items[0], Value::Number(_) | Value::Null);
            if is_ring {
                if let Value::Number(n) = &items[0] {
                    let t = n.as_u64().expect("global texture id must be a u64") as usize;
                    assert!(t < limit, "global texture id {t} must be < {limit}");
                }
                for uv in &items[1..] {
                    let pair = uv
                        .as_array()
                        .unwrap_or_else(|| panic!("expected inlined [u, v] pair, got {uv}"));
                    assert_eq!(pair.len(), 2, "UV pair must have exactly 2 coordinates");
                    assert!(pair[0].is_number() && pair[1].is_number());
                }
            } else {
                for x in items {
                    assert_every_texture_ring_valid(x, limit);
                }
            }
        }
        _ => {}
    }
}

#[test]
fn delft_full_convert_round_trips_through_parquet() {
    let out = tempfile::tempdir().unwrap();
    // `geoarrow: true` here (rather than the library default of `false`,
    // added in the geoarrow-opt-in change) preserves this test's original
    // intent of checking a full GeoParquet-tagged round trip, including the
    // `geo` key below; the untagged default is covered separately by
    // `default_convert_writes_plain_blob_geometry_no_geoarrow_no_geo_key`.
    let mut opts = ConvertOptions::new(fixture("delft.city.jsonl"), out.path().to_path_buf());
    opts.geoarrow = true;
    let report = convert(&opts).unwrap();
    assert_eq!(report.object_count, 2231);

    // delft is a single 1st-level family (Building, BuildingPart folded in),
    // so by-type conversion writes exactly one main table: building.parquet.
    let file = std::fs::File::open(out.path().join("building.parquet")).unwrap();
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
    let pq_meta = builder.metadata().file_metadata();
    let kvs = pq_meta.key_value_metadata().unwrap();
    // spec-alignment M3: one JSON-valued `city` key (never a flat
    // `cityparquet_version` scalar key any more), plus `geo` since delft's
    // LoD0 footprint is GeoParquet-legal.
    assert!(kvs.iter().any(|kv| kv.key == "city"));
    assert!(kvs.iter().any(|kv| kv.key == "geo"));
    // bbox stats exist for row-group pruning
    let rg = builder.metadata().row_group(0);
    let bbox_xmin_col = (0..rg.num_columns())
        .map(|i| rg.column(i))
        .find(|c| c.column_path().string() == "bbox.xmin")
        .expect("bbox.xmin column chunk");
    assert!(bbox_xmin_col.statistics().is_some());
    // The recipe pins bbox leaves to BYTE_STREAM_SPLIT with dictionary
    // encoding off; this only holds if the recipe's per-column
    // `ColumnPath` for "bbox.xmin" actually matches the physical column's
    // nested path (`["bbox", "xmin"]`), not a single dotted-string part.
    let bbox_xmin_encodings: Vec<Encoding> = bbox_xmin_col.encodings().collect();
    assert!(
        bbox_xmin_encodings.contains(&Encoding::BYTE_STREAM_SPLIT),
        "expected BYTE_STREAM_SPLIT on bbox.xmin, got {bbox_xmin_encodings:?}"
    );
    assert!(
        !bbox_xmin_encodings.contains(&Encoding::RLE_DICTIONARY),
        "bbox.xmin should have dictionary encoding disabled, got {bbox_xmin_encodings:?}"
    );
    let rows: usize = builder
        .build()
        .unwrap()
        .map(|b| b.unwrap().num_rows())
        .sum();
    assert_eq!(rows, 2231);
}

#[test]
fn railway_full_convert_succeeds() {
    let out = tempfile::tempdir().unwrap();
    let (_crs_dir, railway_path) = railway_fixture_with_crs();
    let report = convert(&ConvertOptions::new(railway_path, out.path().to_path_buf())).unwrap();
    assert_eq!(report.object_count, 121);
    // railway's distinct object_type values resolve to 9 distinct CityGML
    // modules (see `by_type_convert_of_railway_writes_nine_module_tables`
    // below), so by-module conversion writes 9 main tables, never one —
    // every table the manifest lists must exist.
    let tables = manifest_tables(out.path());
    assert_eq!(tables.len(), 9);
    for name in &tables {
        assert!(out.path().join(name).exists(), "missing {name}");
    }
}

/// Core-profile convert of railway: the main table's `material`/`texture`
/// columns must carry dataset-GLOBAL ids (rewritten by the appearance
/// interner), not the feature-local indices the source CityJSON uses.
/// railway's feature-only appearance sweep resolves 83 materials / 33
/// textures (2 materials + 1 texture are referenced only from
/// geometry-templates, which the Core-profile encode loop never visits BY
/// DESIGN: templates are compatibility-profile sidecar data).
#[test]
fn railway_core_convert_rewrites_appearance_maps_to_global_ids() {
    let out = tempfile::tempdir().unwrap();
    let (_crs_dir, railway_path) = railway_fixture_with_crs();
    convert(&ConvertOptions::new(railway_path, out.path().to_path_buf())).unwrap();

    // Per-LoD appearance columns (§11.1, G20): each `material_lod*` /
    // `texture_lod*` cell holds the plain `{"<theme>": …}` shape. The index
    // checks are structure-agnostic (they recurse for integer leaves), so
    // each per-LoD column value can be walked directly.
    let per_lod_cols = |batch: &arrow_array::RecordBatch, prefix: &str| -> Vec<usize> {
        batch
            .schema()
            .fields()
            .iter()
            .enumerate()
            .filter(|(_, f)| {
                f.name()
                    .strip_prefix(prefix)
                    .is_some_and(|r| r.starts_with("_lod"))
            })
            .map(|(i, _)| i)
            .collect()
    };

    let mut checked_material = false;
    let mut checked_texture = false;
    // railway's object_type values resolve to 9 distinct CityGML modules,
    // so by-module conversion writes 9 main tables — walk every one of them
    // (never a single hardcoded main-table name).
    for table in manifest_tables(out.path()) {
        let file = std::fs::File::open(out.path().join(&table)).unwrap();
        let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
        let reader = builder.build().unwrap();

        for batch in reader {
            let batch = batch.unwrap();
            let material_idx = per_lod_cols(&batch, "material");
            let texture_idx = per_lod_cols(&batch, "texture");
            for row in 0..batch.num_rows() {
                for &i in &material_idx {
                    let col: &StringArray = batch.column(i).as_any().downcast_ref().unwrap();
                    if !col.is_null(row) {
                        let map: Value = serde_json::from_str(col.value(row)).unwrap();
                        assert_every_material_index_below(&map, 83);
                        checked_material = true;
                    }
                }
                for &i in &texture_idx {
                    let col: &StringArray = batch.column(i).as_any().downcast_ref().unwrap();
                    if !col.is_null(row) {
                        let map: Value = serde_json::from_str(col.value(row)).unwrap();
                        assert_every_texture_ring_valid(&map, 33);
                        checked_texture = true;
                    }
                }
            }
        }
    }
    assert!(
        checked_material,
        "expected at least one row with a material map"
    );
    assert!(
        checked_texture,
        "expected at least one row with a texture map"
    );
}

/// Railway's feature-only appearance sweep (83
/// materials / 33 textures — see the module doc on
/// `railway_core_convert_rewrites_appearance_maps_to_global_ids`) plus its 3
/// geometry templates (2 materials + 1 texture reachable ONLY from a
/// template, per `crate::package::build_template_rows` folding them into the
/// same interner) land at 85 materials / 34 textures, split across
/// `materials.parquet`/`textures.parquet`/`geometry_templates.parquet`, with
/// BOTH `metadata.json`'s `sidecar_files` and the parquet footer's KV
/// `sidecar_files` listing exactly the files actually written.
#[test]
fn railway_compatibility_convert_writes_materials_and_textures_sidecars() {
    let out = tempfile::tempdir().unwrap();
    let (_crs_dir, railway_path) = railway_fixture_with_crs();
    let opts = ConvertOptions::new(railway_path, out.path().to_path_buf());
    let report = convert(&opts).unwrap();

    assert_eq!(report.materials_written, 85);
    assert_eq!(report.textures_written, 34);
    assert_eq!(report.templates_written, 3);
    assert!(out.path().join("materials.parquet").exists());
    assert!(out.path().join("textures.parquet").exists());
    assert!(out.path().join("geometry_templates.parquet").exists());

    assert_eq!(
        PackageTables::open(out.path()).unwrap().sidecar_files,
        vec![
            "materials.parquet".to_string(),
            "textures.parquet".to_string(),
            "geometry_templates.parquet".to_string()
        ],
        "metadata.json's cityparquet-sidecar assets must list exactly the sidecars written"
    );

    // The written template rows must carry their LoD: railway's 3 templates
    // all declare lod "3". The geometry_properties struct itself has no
    // `lod` field (spec: "same struct, reused" — no lod field anywhere); a
    // template's LoD instead picks which physical per-LoD column set
    // (`geometry_lod3_0` etc.) its row lands in, exactly like the main
    // object table's own geometry columns (spec: "a template's LoD is
    // carried by its column name here exactly as it is in an object table").
    let template_rows =
        cityparquet::sidecar::read_templates(&out.path().join("geometry_templates.parquet"))
            .unwrap();
    assert_eq!(template_rows.len(), 3);
    let lod3 = cityparquet_schema::Lod::parse("3").unwrap();
    for (i, row) in template_rows.iter().enumerate() {
        let props = row.geometry_properties.as_ref().unwrap();
        assert!(props.get("type").is_some(), "template {i} missing type");
        assert!(
            props.get("lod").is_none(),
            "template {i}: geometry_properties struct must carry no lod field"
        );
        assert_eq!(
            row.lod, lod3,
            "template {i}: row.lod must carry the source lod"
        );
    }

    // Physical schema assertion (gap 12): the sidecar carries a per-LoD
    // suffixed column set, no un-suffixed geometry/geometry_properties/
    // material/texture columns, no `lod` column, and no `other` column.
    {
        let file = std::fs::File::open(out.path().join("geometry_templates.parquet")).unwrap();
        let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
        let schema = builder.schema();
        assert!(schema.field_with_name("geometry_lod3_0").is_ok());
        assert!(schema.field_with_name("geometry_properties_lod3_0").is_ok());
        assert!(schema.field_with_name("material_lod3_0").is_ok());
        assert!(schema.field_with_name("texture_lod3_0").is_ok());
        for col in ["geometry", "geometry_properties", "material", "texture", "lod", "other"] {
            assert!(
                schema.field_with_name(col).is_err(),
                "geometry_templates.parquet must not carry column '{col}'"
            );
        }
    }
}

/// A dataset with no appearance at all (delft): no sidecar files are
/// written, and the manifest says so — sidecars are written whenever the
/// source has content for them (spec-alignment gap 19), so delft (no
/// materials/textures/templates) simply writes none.
#[test]
fn delft_compatibility_convert_writes_no_sidecars() {
    let out = tempfile::tempdir().unwrap();
    let opts = ConvertOptions::new(fixture("delft.city.jsonl"), out.path().to_path_buf());
    let report = convert(&opts).unwrap();

    assert_eq!(report.materials_written, 0);
    assert_eq!(report.textures_written, 0);
    assert_eq!(report.templates_written, 0);
    assert!(!out.path().join("materials.parquet").exists());
    assert!(!out.path().join("textures.parquet").exists());

    assert_eq!(
        PackageTables::open(out.path()).unwrap().sidecar_files,
        Vec::<String>::new(),
        "metadata.json must list no cityparquet-sidecar assets"
    );
}

/// M4 task 11 (Step 1/2): the overwrite-purge hazard the `TODO(M4)` comment
/// named. A Compatibility convert of railway into a fresh directory writes
/// `materials.parquet`/`textures.parquet`/`geometry_templates.parquet`
/// alongside its main object tables; overwriting that SAME directory with a
/// Core convert of an unrelated dataset (delft, no appearance/templates of
/// its own) must not leave any of the first run's sidecars behind — a
/// consumer reading the directory afterwards must see exactly what the
/// second run's own `metadata.json` describes (`sidecar_files == []`), never
/// a stale sidecar from a prior run that manifest no longer mentions.
#[test]
fn overwrite_purges_stale_sidecars_from_a_prior_compatibility_convert() {
    let out = tempfile::tempdir().unwrap();
    let (_crs_dir, railway_path) = railway_fixture_with_crs();
    let first = ConvertOptions::new(railway_path, out.path().to_path_buf());
    let first_report = convert(&first).unwrap();
    assert_eq!(first_report.materials_written, 85);
    assert!(out.path().join("materials.parquet").exists());
    assert!(out.path().join("textures.parquet").exists());
    assert!(out.path().join("geometry_templates.parquet").exists());

    let mut second = ConvertOptions::new(fixture("delft.city.jsonl"), out.path().to_path_buf());
    second.overwrite = true;
    let second_report = convert(&second).unwrap();
    assert_eq!(second_report.object_count, 2231);
    assert_eq!(second_report.materials_written, 0);
    assert_eq!(second_report.textures_written, 0);
    assert_eq!(second_report.templates_written, 0);

    assert!(
        !out.path().join("materials.parquet").exists(),
        "stale materials.parquet from the first (Compatibility) convert must be purged"
    );
    assert!(
        !out.path().join("textures.parquet").exists(),
        "stale textures.parquet from the first (Compatibility) convert must be purged"
    );
    assert!(
        !out.path().join("geometry_templates.parquet").exists(),
        "stale geometry_templates.parquet from the first (Compatibility) convert must be purged"
    );
    // delft is a single 1st-level family, so the second (delft) convert
    // writes exactly one main table: building.parquet.
    assert!(out.path().join("building.parquet").exists());

    assert_eq!(
        PackageTables::open(out.path()).unwrap().sidecar_files,
        Vec::<String>::new(),
        "the second run's own metadata.json must say no sidecars, and none must be left on disk"
    );

    let file = std::fs::File::open(out.path().join("building.parquet")).unwrap();
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
    let rows: usize = builder
        .build()
        .unwrap()
        .map(|b| b.unwrap().num_rows())
        .sum();
    assert_eq!(rows, 2231, "the second run's own main table must be intact");
}

/// M4 final-review Fix 6: `convert`'s stale-package purge used to run
/// BEFORE `Source::open(&opts.input)`, so `convert /bad/path out
/// --overwrite` against a directory already holding a valid package would
/// destroy that package and only THEN fail on the bad input — leaving
/// neither the old package nor a new one. The purge must instead run only
/// after every fallible step that does not touch `opts.output_dir` (opening
/// and scanning the source, deriving the schema/writer properties) has
/// already succeeded, so a failing `convert --overwrite` leaves the
/// existing package untouched.
#[test]
fn overwrite_with_a_bad_input_path_leaves_the_existing_package_intact() {
    let out = tempfile::tempdir().unwrap();
    let (_crs_dir, railway_path) = railway_fixture_with_crs();
    let first = ConvertOptions::new(railway_path, out.path().to_path_buf());
    let first_report = convert(&first).unwrap();
    assert_eq!(first_report.materials_written, 85);
    assert!(out.path().join("materials.parquet").exists());
    assert!(out.path().join("textures.parquet").exists());
    assert!(out.path().join("geometry_templates.parquet").exists());
    // railway's object_type values resolve to 9 distinct CityGML modules,
    // so this convert wrote 9 main tables, never one.
    let tables = manifest_tables(out.path());
    assert_eq!(tables.len(), 9);
    for name in &tables {
        assert!(out.path().join(name).exists(), "missing {name}");
    }
    assert!(out.path().join("metadata.json").exists());

    let mut bad = ConvertOptions::new(
        PathBuf::from("/no/such/path/does-not-exist.city.jsonl"),
        out.path().to_path_buf(),
    );
    bad.overwrite = true;
    let err = convert(&bad).unwrap_err();
    assert!(
        matches!(err, CityParquetError::Io(_)),
        "expected an Io error opening the bad input path, got {err:?}"
    );

    // The first (valid) package must survive completely untouched: none of
    // its files were purged, and every main table still has all its rows.
    assert!(
        out.path().join("materials.parquet").exists(),
        "a failed overwrite must not purge the existing package's materials.parquet"
    );
    assert!(
        out.path().join("textures.parquet").exists(),
        "a failed overwrite must not purge the existing package's textures.parquet"
    );
    assert!(
        out.path().join("geometry_templates.parquet").exists(),
        "a failed overwrite must not purge the existing package's geometry_templates.parquet"
    );
    assert_eq!(
        manifest_tables(out.path()),
        tables,
        "a failed overwrite must not purge any of the existing package's main tables"
    );
    for name in &tables {
        assert!(
            out.path().join(name).exists(),
            "a failed overwrite must not purge the existing package's {name}"
        );
    }
    assert!(
        out.path().join("metadata.json").exists(),
        "a failed overwrite must not purge the existing package's metadata.json"
    );

    let mut rows = 0usize;
    for name in &tables {
        let file = std::fs::File::open(out.path().join(name)).unwrap();
        let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
        rows += builder
            .build()
            .unwrap()
            .map(|b| b.unwrap().num_rows())
            .sum::<usize>();
    }
    assert_eq!(
        rows, 121,
        "the surviving package's own main tables must still be intact and readable"
    );
}

/// M5 debt item 5: `convert` used to purge `output_dir`'s prior package
/// AFTER `Source::open`/`scan` but BEFORE encoding the new one — so a
/// failure DURING encode (as opposed to the bad-input-path case above, which
/// fails before ever reaching the purge) still destroyed the old package and
/// left nothing usable behind. The fix writes every new file into a hidden
/// scratch directory first and only swaps it into place once the ENTIRE new
/// package (including `metadata.json`) has been written successfully.
///
/// The corruption chosen is a dangling material index: derived from a copy
/// of delft (which has no material/texture of its own — see
/// `crate::compare`'s module docs), a LATE feature (index 1113 of 1116
/// lines, i.e. near the very end) gains a 1-entry `appearance.materials`
/// array and its first geometry references material index 99 — an index no
/// `scan` pass ever inspects (scan only calls `geometry_to_wkb` for bbox
/// purposes, never touching `material`/`texture` at all — see
/// `crate::scan::scan`), but `encode`'s per-object appearance rewrite
/// (`crate::encode::rewrite_geometry_appearance` /
/// `crate::appearance::AppearanceInterner`) must resolve every local index
/// against the feature's own `appearance.materials` and errors loudly on an
/// out-of-range one — confirmed empirically below: `convert` must fail with
/// a `Schema` error mentioning "material index", not merely tolerate it.
/// A small `batch_size` (50, versus delft's 2231 objects) ensures several
/// `RecordBatch`es — and therefore several `ArrowWriter::write` calls typing
/// real bytes into the scratch `building.parquet` (delft is a single
/// 1st-level family) — succeed before the corrupted feature's batch is ever
/// reached, so this is a genuine mid-encode failure, not merely a
/// same-instant one.
#[test]
fn overwrite_with_a_mid_encode_failure_leaves_the_existing_package_intact() {
    // Pre-existing, valid Compatibility package at `out`.
    let out = tempfile::tempdir().unwrap();
    let (_crs_dir, railway_path) = railway_fixture_with_crs();
    let first = ConvertOptions::new(railway_path.clone(), out.path().to_path_buf());
    let first_report = convert(&first).unwrap();
    assert_eq!(first_report.materials_written, 85);
    assert!(out.path().join("materials.parquet").exists());
    assert!(out.path().join("textures.parquet").exists());
    assert!(out.path().join("geometry_templates.parquet").exists());
    // railway's object_type values resolve to 9 distinct CityGML modules,
    // so this convert wrote 9 main tables, never one.
    let railway_tables = manifest_tables(out.path());
    assert_eq!(railway_tables.len(), 9);
    for name in &railway_tables {
        assert!(out.path().join(name).exists(), "missing {name}");
    }
    assert!(out.path().join("metadata.json").exists());

    // Derived delft copy: a late feature gains a dangling material
    // reference that only `encode` (never `scan`) validates.
    let original = fixture("delft.city.jsonl");
    let text = std::fs::read_to_string(&original).unwrap();
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    assert_eq!(lines.len(), 1116, "fixture fact: 1 header + 1115 features");
    const TARGET_LINE: usize = 1113;
    let mut feature: Value = serde_json::from_str(&lines[TARGET_LINE]).unwrap();
    let mut corrupted = false;
    {
        let objects = feature["CityObjects"].as_object_mut().unwrap();
        for co in objects.values_mut() {
            let Some(geoms) = co.get_mut("geometry").and_then(Value::as_array_mut) else {
                continue;
            };
            let Some(geom) = geoms.first_mut().and_then(Value::as_object_mut) else {
                continue;
            };
            assert!(
                !geom.contains_key("material"),
                "precondition: delft geometries carry no material of their own"
            );
            geom.insert(
                "material".to_string(),
                serde_json::json!({"visual": {"value": 99}}),
            );
            corrupted = true;
            break;
        }
    }
    assert!(
        corrupted,
        "target feature line must contain an object with geometry"
    );
    feature["appearance"] = serde_json::json!({"materials": [{}]});
    lines[TARGET_LINE] = serde_json::to_string(&feature).unwrap();

    let derived_dir = tempfile::tempdir().unwrap();
    let derived_input = derived_dir
        .path()
        .join("delft-dangling-material.city.jsonl");
    std::fs::write(&derived_input, lines.join("\n") + "\n").unwrap();

    // Overwrite the valid railway package with the derived, encode-time-
    // corrupted delft: must fail with a Schema error, not succeed and not
    // panic.
    let mut second = ConvertOptions::new(derived_input, out.path().to_path_buf());
    second.overwrite = true;
    second.batch_size = 50;
    let err = convert(&second).unwrap_err();
    assert!(
        matches!(err, CityParquetError::Schema(_)),
        "expected a Schema error for the dangling material index, got {err:?}"
    );
    assert!(
        err.to_string().contains("material index"),
        "the error must name the out-of-range material index, got: {err}"
    );

    // No leftover scratch directory.
    assert!(
        !out.path().join(".cityparquet-tmp").exists(),
        "the temp scratch directory must be cleaned up on a failed convert"
    );

    // The original railway package must be completely untouched: every file
    // survives, and it still round-trips losslessly with no exclusions
    // (the same headline gate as `roundtrip_real_data.rs`'s
    // `railway_compatibility_round_trips_losslessly_with_no_exclusions`).
    assert!(out.path().join("materials.parquet").exists());
    assert!(out.path().join("textures.parquet").exists());
    assert!(out.path().join("geometry_templates.parquet").exists());
    assert_eq!(
        manifest_tables(out.path()),
        railway_tables,
        "a failed mid-encode overwrite must not purge any of the existing package's main tables"
    );
    for name in &railway_tables {
        assert!(out.path().join(name).exists(), "missing {name}");
    }
    assert!(out.path().join("metadata.json").exists());

    let export_dir = tempfile::tempdir().unwrap();
    let exported = export_dir.path().join("export.city.jsonl");
    export(&ExportOptions {
        package_dir: out.path().to_path_buf(),
        output: exported.clone(),
    })
    .unwrap();
    let report = compare_datasets(&railway_path, &exported, &CompareOptions::default()).unwrap();
    assert!(
        report.equal,
        "the surviving package must still export losslessly; differences: {:#?}",
        report.differences
    );
}

/// One `(xmin, ymin, zmin, xmax, ymax, zmax)` row, or `None` when the
/// struct-level `bbox` value itself is null (a `CityObject` with no
/// geometry anywhere in its own subtree — see `crate::encode::resolve_bbox`).
fn read_bbox_row(bbox_col: &StructArray, row: usize) -> Option<[f64; 6]> {
    if bbox_col.is_null(row) {
        return None;
    }
    let leaf = |name: &str| -> f64 {
        bbox_col
            .column_by_name(name)
            .unwrap_or_else(|| panic!("bbox struct has no {name} field"))
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap_or_else(|| panic!("bbox.{name} is not Float64"))
            .value(row)
    };
    Some([
        leaf("xmin"),
        leaf("ymin"),
        leaf("zmin"),
        leaf("xmax"),
        leaf("ymax"),
        leaf("zmax"),
    ])
}

fn union_row_bbox(acc: &mut Option<[f64; 6]>, row: [f64; 6]) {
    *acc = Some(match acc.take() {
        None => row,
        Some(mut cur) => {
            for i in 0..3 {
                cur[i] = cur[i].min(row[i]);
                cur[i + 3] = cur[i + 3].max(row[i + 3]);
            }
            cur
        }
    });
}

/// M5 task 4 (Hilbert row ordering): converting with `RowOrder::Hilbert`
/// must (a) keep every feature's rows CONTIGUOUS — the ordering unit is the
/// whole `CityJSONFeature`, never an individual `CityObject` — and (b)
/// visit features in non-decreasing Hilbert-index order of their bbox
/// centroid.
///
/// The implementation's per-feature sort key is the FEATURE's own
/// vertex-pool min/max centroid (`crate::order::feature_hilbert_key`,
/// `pub(crate)` and so not reachable from this integration-test crate);
/// this test instead recomputes each feature run's key from the per-OBJECT
/// `bbox` column's stored values, unioned over every row in the run. On
/// delft these two bboxes coincide because the WRITER's index-based ring
/// normalisation (`crate::wkb_write::normalise_ring`) drops zero rings on
/// delft — every vertex the source carries lands in the vertex pool and
/// the stored bbox stats, unchanged by this fix. This does NOT contradict
/// `delft_round_trips_losslessly` (`roundtrip_real_data.rs`) now pinning 16
/// coordinate-degenerate-ring exclusions for delft: those are COMPARATOR-
/// side exclusions applied when checking export-vs-source semantic
/// equality (`crate::compare`), a separate concept from what the writer
/// actually stores — they do not affect the vertex pool or bbox this test
/// reads back.
#[test]
fn hilbert_ordering_keeps_features_contiguous_and_visits_them_in_non_decreasing_index_order() {
    let out = tempfile::tempdir().unwrap();
    let mut opts = ConvertOptions::new(fixture("delft.city.jsonl"), out.path().to_path_buf());
    opts.ordering = RowOrder::Hilbert;
    let report = convert(&opts).unwrap();
    assert_eq!(report.object_count, 2231);

    // Ground-truth normalisation range: the same `dataset_bbox` the writer
    // itself computed (via `crate::scan::scan`) and used to key every
    // feature's Hilbert index during the sort.
    let src = cityparquet::source::Source::open(&fixture("delft.city.jsonl")).unwrap();
    let scan_result = cityparquet::scan::scan(&src).unwrap();
    let dataset_bbox = scan_result
        .dataset_bbox
        .expect("delft has geometry, so a dataset bbox");

    // delft is a single 1st-level family, so by-type conversion writes
    // exactly one main table: building.parquet.
    let file = std::fs::File::open(out.path().join("building.parquet")).unwrap();
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
    let reader = builder.build().unwrap();

    // Walk rows in on-disk order, folding consecutive equal `feature_id`s
    // into runs and unioning each run's row bboxes as we go.
    let mut runs: Vec<(String, Option<[f64; 6]>)> = Vec::new();
    let mut seen_feature_ids: HashSet<String> = HashSet::new();

    for batch in reader {
        let batch = batch.unwrap();
        let feature_id_col: &StringArray = batch
            .column_by_name("feature_id")
            .unwrap()
            .as_any()
            .downcast_ref()
            .unwrap();
        let bbox_col: &StructArray = batch
            .column_by_name("bbox")
            .unwrap()
            .as_any()
            .downcast_ref()
            .unwrap();

        for row in 0..batch.num_rows() {
            let fid = feature_id_col.value(row).to_string();
            let row_bbox = read_bbox_row(bbox_col, row);

            match runs.last_mut() {
                Some((last_id, acc)) if *last_id == fid => {
                    if let Some(b) = row_bbox {
                        union_row_bbox(acc, b);
                    }
                }
                _ => {
                    assert!(
                        seen_feature_ids.insert(fid.clone()),
                        "feature_id {fid} appeared in two separated runs: row ordering must \
                         keep one feature's rows contiguous"
                    );
                    let mut acc = None;
                    if let Some(b) = row_bbox {
                        union_row_bbox(&mut acc, b);
                    }
                    runs.push((fid, acc));
                }
            }
        }
    }
    assert!(runs.len() > 1, "delft has many features, expected > 1 run");

    // Per-run Hilbert index: a feature with NO geometry anywhere (union
    // stayed `None`) gets key 0, mirroring `feature_hilbert_key`'s own rule
    // for a feature with no vertices at all.
    let indices: Vec<u32> = runs
        .iter()
        .map(|(_, bbox)| match bbox {
            None => 0,
            Some(b) => {
                let cx = (b[0] + b[3]) / 2.0;
                let cy = (b[1] + b[4]) / 2.0;
                hilbert_index(cx, cy, &dataset_bbox)
            }
        })
        .collect();
    for w in indices.windows(2) {
        assert!(
            w[0] <= w[1],
            "Hilbert-ordered feature runs must visit non-decreasing indices, got {:?} \
             (full sequence: {:?})",
            w,
            indices
        );
    }
}

/// M5 task 4 (Hilbert row ordering): reordering rows must never change what
/// the dataset MEANS — a Hilbert-ordered convert must still round-trip
/// losslessly through export, exactly like the Source-ordered path
/// (`roundtrip_real_data.rs`'s comparator is order-independent by
/// construction: it groups rows back into `CityObject`s/features before
/// comparing, so this test pins that property rather than re-deriving it).
#[test]
fn hilbert_ordering_never_changes_delft_semantics() {
    let out = tempfile::tempdir().unwrap();
    let mut opts = ConvertOptions::new(fixture("delft.city.jsonl"), out.path().to_path_buf());
    opts.ordering = RowOrder::Hilbert;
    convert(&opts).unwrap();

    let export_dir = tempfile::tempdir().unwrap();
    let exported = export_dir.path().join("export.city.jsonl");
    export(&ExportOptions {
        package_dir: out.path().to_path_buf(),
        output: exported.clone(),
    })
    .unwrap();

    let report = compare_datasets(
        &fixture("delft.city.jsonl"),
        &exported,
        &CompareOptions::default(),
    )
    .unwrap();
    assert!(
        report.equal,
        "Hilbert-ordered delft must still round-trip losslessly; differences: {:#?}",
        report.differences
    );
    assert!(report.differences.is_empty());

    // Pinned counts updated alongside the comparator's coordinate-degenerate
    // ring fix (3DBAG tile `9-284-556.city.json` finding, see
    // `crate::compare`'s module docs): delft's real, UNMUTATED source data
    // turns out to carry 8 objects whose LoD boundaries include a ring with
    // index-distinct but coordinate-identical vertices — previously
    // invisible to the INDEX-only degenerate check, now correctly dropped
    // (and logged as excluded) on both the source and the exported side.
    // This is not a regression: `report.equal`/`differences.is_empty()`
    // above still hold, proving the round trip stays lossless; only the
    // exclusion log grew to record real, previously-silent normalisation.
    let (header_excluded, non_header_excluded): (Vec<&String>, Vec<&String>) = report
        .excluded
        .iter()
        .partition(|e| e.starts_with("header: metadata member"));
    let degenerate = non_header_excluded
        .iter()
        .filter(|e| e.contains("degenerate ring"))
        .count();
    assert_eq!(
        (degenerate, non_header_excluded.len()),
        (16, 16),
        "Hilbert ordering must not introduce any new exclusion beyond delft's usual header \
         metadata members and the 16 pinned coordinate-degenerate-ring drops (8 objects, \
         source + export side each), got: {:#?}",
        non_header_excluded
    );
    assert!(
        !header_excluded.is_empty(),
        "delft's header sets metadata members; expected at least one documented header-metadata \
         exclusion, got none. Full excluded: {:#?}",
        report.excluded
    );
}

/// Same headline gate as `hilbert_ordering_never_changes_delft_semantics`,
/// for railway (Compatibility profile — the M4 headline round trip): the 23
/// documented degenerate-ring drops (updated alongside the comparator's
/// coordinate-degenerate fix — see the comment in
/// `hilbert_ordering_never_changes_delft_semantics` above) and
/// header-metadata exclusions are the ONLY exclusions, exactly as
/// `roundtrip_real_data.rs::railway_compatibility_round_trips_losslessly_with_no_exclusions`
/// pins for `RowOrder::Source`.
#[test]
fn hilbert_ordering_never_changes_railway_compatibility_semantics() {
    let out = tempfile::tempdir().unwrap();
    let (_crs_dir, railway_path) = railway_fixture_with_crs();
    let mut opts = ConvertOptions::new(railway_path.clone(), out.path().to_path_buf());
    opts.ordering = RowOrder::Hilbert;
    convert(&opts).unwrap();

    let export_dir = tempfile::tempdir().unwrap();
    let exported = export_dir.path().join("export.city.jsonl");
    export(&ExportOptions {
        package_dir: out.path().to_path_buf(),
        output: exported.clone(),
    })
    .unwrap();

    let report = compare_datasets(&railway_path, &exported, &CompareOptions::default()).unwrap();
    assert!(
        report.equal,
        "Hilbert-ordered railway (Compatibility) must round-trip losslessly with no exclusions \
         beyond the documented degenerate-ring drops and header metadata; differences: {:#?}",
        report.differences
    );
    assert!(report.differences.is_empty());

    let (header_excluded, non_header_excluded): (Vec<&String>, Vec<&String>) = report
        .excluded
        .iter()
        .partition(|e| e.starts_with("header: metadata member"));
    let degenerate = non_header_excluded
        .iter()
        .filter(|e| e.contains("degenerate ring"))
        .count();
    assert_eq!(
        (degenerate, non_header_excluded.len()),
        (23, 23),
        "the only non-header exclusions must be the 23 pinned degenerate-ring drops, got: {:#?}",
        non_header_excluded
    );
    assert!(
        !header_excluded.is_empty(),
        "railway's header sets metadata members; expected at least one documented \
         header-metadata exclusion, got none. Full excluded: {:#?}",
        report.excluded
    );
}

/// Every distinct `object_type` string in the main table at `path` (raw
/// dictionary decode, independent of `crate::decode`), and the file's total
/// row count — used to assert the by-type writer's per-family grouping
/// (a family table may legitimately carry MULTIPLE `object_type` values —
/// its 1st-level type plus any 2nd-level children) and to recombine row
/// counts across the split tables.
fn table_object_types_and_count(path: &std::path::Path) -> (HashSet<String>, usize) {
    let file = std::fs::File::open(path).unwrap();
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
    let reader = builder.build().unwrap();
    let mut types = HashSet::new();
    let mut count = 0usize;
    for batch in reader {
        let batch = batch.unwrap();
        count += batch.num_rows();
        let col = batch.column_by_name("object_type").unwrap();
        let dict: &DictionaryArray<Int32Type> = col.as_any().downcast_ref().unwrap();
        let values: &StringArray = dict.values().as_any().downcast_ref().unwrap();
        for row in 0..batch.num_rows() {
            let key = dict.keys().value(row) as usize;
            types.insert(values.value(key).to_string());
        }
    }
    (types, count)
}

/// delft's only two `object_type` values are `Building`/`BuildingPart` —
/// both resolve to the Building CityGML module (spec "By-module
/// object-table layout", via `cityparquet_schema::resolve_module_key`). The
/// by-module writer must therefore write exactly ONE table,
/// `building.parquet`, containing every row of BOTH types;
/// `buildingpart.parquet` must never be created.
#[test]
fn by_type_convert_of_delft_writes_exactly_one_family_table() {
    let out = tempfile::tempdir().unwrap();
    let opts = ConvertOptions::new(fixture("delft.city.jsonl"), out.path().to_path_buf());
    let report = convert(&opts).unwrap();
    assert_eq!(report.object_count, 2231);

    let tables = manifest_tables(out.path());
    assert_eq!(
        tables,
        vec!["building.parquet".to_string()],
        "expected exactly one family table (Building + BuildingPart share it), got: {tables:?}"
    );
    assert!(
        !out.path().join("buildingpart.parquet").exists(),
        "BuildingPart is a 2nd-level type and must not get its own table"
    );
    for name in &tables {
        assert!(out.path().join(name).exists(), "missing {name}");
    }

    let (building_types, building_count) =
        table_object_types_and_count(&out.path().join("building.parquet"));
    assert_eq!(
        building_types,
        HashSet::from(["Building".to_string(), "BuildingPart".to_string()]),
        "building.parquet must carry BOTH object_type values; the object_type column is what \
         still distinguishes them within the shared file"
    );
    assert_eq!(
        building_count, 2231,
        "the single family table must carry delft's full object count \
         (1115 Building + 1116 BuildingPart)"
    );

    // Every table's footer must carry `city.version` (required by
    // `cityparquet_metadata()`, which errors without a `city` key at all),
    // and it must agree across every table this run wrote.
    let mut versions = HashSet::new();
    for name in &tables {
        let file = std::fs::File::open(out.path().join(name)).unwrap();
        let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
        let meta = builder.cityparquet_metadata().unwrap();
        versions.insert(meta.version.clone());
    }
    assert_eq!(
        versions.len(),
        1,
        "every table's footer must carry the identical city.version, got: {versions:?}"
    );
}

/// M5 review follow-up (Minor a): the ByType writer's first-appearance
/// order and per-family writer index must persist ACROSS batches, not just
/// within one — a small `batch_size` (256 vs delft's 2231 objects, so ~9
/// encoded batches, with both types recurring in every one of them) proves
/// the single family table comes out with the full row count, end-to-end.
#[test]
fn by_type_convert_of_delft_survives_many_small_batches() {
    let out = tempfile::tempdir().unwrap();
    let mut opts = ConvertOptions::new(fixture("delft.city.jsonl"), out.path().to_path_buf());
    opts.batch_size = 256;
    let report = convert(&opts).unwrap();
    assert_eq!(report.object_count, 2231);

    let tables = manifest_tables(out.path());
    assert_eq!(
        tables,
        vec!["building.parquet".to_string()],
        "the same single family table, regardless of batching"
    );

    let (building_types, building_count) =
        table_object_types_and_count(&out.path().join("building.parquet"));
    assert_eq!(
        building_types,
        HashSet::from(["Building".to_string(), "BuildingPart".to_string()])
    );
    assert_eq!(
        building_count, 2231,
        "rows written across ~9 batches must still sum to delft's full object count"
    );
}

/// M5 Codex review (Important finding 1) originally flagged that the
/// by-type writer opens a per-type writer LAZILY, on that type's first row
/// (see `by_type_table_index`) — so an input that encodes to ZERO rows
/// opens no writer at all, and `finish` returns an empty table list, which
/// used to be papered over with a standalone, empty reserved-name fallback
/// table. Derived fixture (sanctioned): delft's own CityJSONSeq HEADER line
/// only, every feature line stripped — a genuine zero-feature stream, not a
/// hand-rolled artificial CityJSON.
///
/// Plan decision (2026-07-21, mandatory-by-type-layout): that fallback is
/// gone. `write_package` now rejects a zero-object conversion outright
/// (`scan_result.object_count == 0` — see `package.rs`); by-type is the
/// only layout, so there is nothing left to parametrise this test over.
#[test]
fn empty_input_is_rejected() {
    let src = std::fs::read_to_string(fixture("delft.city.jsonl")).unwrap();
    let header_only = format!(
        "{}\n",
        src.lines()
            .next()
            .expect("delft.city.jsonl must have a header line")
    );
    let empty_dir = tempfile::tempdir().unwrap();
    let empty_input = empty_dir.path().join("empty.city.jsonl");
    std::fs::write(&empty_input, &header_only).unwrap();

    let out = tempfile::tempdir().unwrap();
    let opts = ConvertOptions::new(empty_input.clone(), out.path().to_path_buf());
    let err = convert(&opts).expect_err("a zero-object conversion must fail, not succeed");
    assert!(
        format!("{err}").contains("no city objects"),
        "expected a clear 'no city objects' error, got: {err}"
    );
    assert!(
        !out.path().join("metadata.json").exists(),
        "a rejected conversion must leave no package behind"
    );
}

/// Spec-alignment: railway's 14-distinct-`object_type` fixture fact (pinned
/// in the M5 milestone brief) round-tripped through the by-MODULE writer
/// under the spec's by-module rule (spec "By-module object-table layout"):
/// railway's 14 distinct `object_type` values collapse to 9 distinct CityGML
/// 3.0 MODULES — `Bridge`, `BridgeConstructiveElement`, and
/// `BridgeInstallation` all land in `bridge.parquet` (Bridge module);
/// `Building` and `BuildingInstallation` in `building.parquet` (Building
/// module); `Tunnel` and `TunnelInstallation` in `tunnel.parquet` (Tunnel
/// module); `CityObjectGroup` and `GenericCityObject` (whose `object_type`
/// is stored as its CityGML class name `GenericOccupiedSpace` — spec
/// "object_type vocabulary") both fold into `generics.parquet` (spec: "On
/// `CityObjectGroup`"); the remaining 5 types (`CityFurniture`, `Railway`,
/// `SolitaryVegetationObject`, `TINRelief`, `WaterBody`) each own module's
/// sole member and so keep their own single-type file — row counts summing
/// to railway's full object count.
#[test]
fn by_type_convert_of_railway_writes_nine_module_tables() {
    let out = tempfile::tempdir().unwrap();
    let (_crs_dir, railway_path) = railway_fixture_with_crs();
    let opts = ConvertOptions::new(railway_path, out.path().to_path_buf());
    let report = convert(&opts).unwrap();
    assert_eq!(report.object_count, 121);

    let tables = manifest_tables(out.path());
    assert_eq!(
        tables.len(),
        9,
        "railway's pinned type set collapses to 9 distinct CityGML modules, got: {tables:?}"
    );

    let expected_names: HashSet<String> = [
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
    .into_iter()
    .map(|t| t.to_string())
    .collect();
    let actual_names: HashSet<String> = tables.iter().cloned().collect();
    assert_eq!(
        actual_names, expected_names,
        "unexpected module table name set"
    );
    assert!(
        !out.path()
            .join("bridgeconstructiveelement.parquet")
            .exists(),
        "BridgeConstructiveElement shares the Bridge module and must not get its own table"
    );
    assert!(
        !out.path().join("bridgeinstallation.parquet").exists(),
        "BridgeInstallation shares the Bridge module and must not get its own table"
    );
    assert!(
        !out.path().join("buildinginstallation.parquet").exists(),
        "BuildingInstallation shares the Building module and must not get its own table"
    );
    assert!(
        !out.path().join("tunnelinstallation.parquet").exists(),
        "TunnelInstallation shares the Tunnel module and must not get its own table"
    );
    assert!(
        !out.path().join("cityobjectgroup.parquet").exists(),
        "CityObjectGroup folds into generics.parquet and must not get its own table"
    );
    assert!(
        !out.path().join("genericcityobject.parquet").exists(),
        "GenericCityObject/GenericOccupiedSpace shares the Generics module and must not get \
         its own table"
    );

    // Each module table's expected member `object_type` set — stored values
    // are CityGML class names (spec "object_type vocabulary"), so
    // `GenericCityObject`'s row carries `GenericOccupiedSpace`.
    let expected_types: &[(&str, &[&str])] = &[
        (
            "bridge.parquet",
            &["Bridge", "BridgeConstructiveElement", "BridgeInstallation"],
        ),
        ("building.parquet", &["Building", "BuildingInstallation"]),
        ("city_furniture.parquet", &["CityFurniture"]),
        (
            "generics.parquet",
            &["CityObjectGroup", "GenericOccupiedSpace"],
        ),
        ("transportation.parquet", &["Railway"]),
        ("vegetation.parquet", &["SolitaryVegetationObject"]),
        ("relief.parquet", &["TINRelief"]),
        ("tunnel.parquet", &["Tunnel", "TunnelInstallation"]),
        ("water_body.parquet", &["WaterBody"]),
    ];

    let mut total = 0usize;
    for (name, member_types) in expected_types {
        let path = out.path().join(name);
        assert!(path.exists(), "missing {name}");
        let (types, count) = table_object_types_and_count(&path);
        let expected: HashSet<String> = member_types.iter().map(|t| t.to_string()).collect();
        assert_eq!(
            types, expected,
            "table {name} must carry exactly its module's object_type values"
        );
        total += count;
    }
    assert_eq!(
        total, 121,
        "row counts across every module table must sum to railway's full object count"
    );
}

/// G1: a default convert (no `--geoarrow`) still writes the GeoParquet `geo`
/// key — declaring ONLY the GeoParquet-legal columns — while the geometry
/// fields stay plain BLOB (no `geoarrow.wkb` field extension) so DuckDB reads
/// them with zero setup. delft's LoD0 `geometry` is a `MultiPolygon Z`
/// footprint (legal); its `lod1.2/1.3/2.2` are `Solid`s (PolyhedralSurfaceZ,
/// illegal) and must NOT appear in `geo.columns`.
#[test]
fn default_convert_writes_geo_key_for_legal_columns_only() {
    let out = tempfile::tempdir().unwrap();
    let mut opts = ConvertOptions::new(fixture("delft.city.jsonl"), out.path().to_path_buf());
    opts.overwrite = true;
    convert(&opts).unwrap();

    // delft is a single 1st-level family, so by-type conversion writes
    // exactly one main table: building.parquet.
    let file = std::fs::File::open(out.path().join("building.parquet")).unwrap();
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();

    // (a) The GeoParquet `geo` key IS present, listing only the legal LoD0
    // footprint column (never the Solid LoD columns).
    let kvs = builder
        .metadata()
        .file_metadata()
        .key_value_metadata()
        .unwrap();
    let geo_kv = kvs
        .iter()
        .find(|kv| kv.key == "geo")
        .expect("default output must carry the GeoParquet `geo` key (G1)");
    let geo: serde_json::Value = serde_json::from_str(geo_kv.value.as_deref().unwrap()).unwrap();
    let columns = geo["columns"].as_object().unwrap();
    assert!(
        columns.contains_key("geometry_lod0_0"),
        "the legal LoD0 MultiPolygon footprint must be declared in the geometry_lod0_0 column"
    );
    assert_eq!(
        columns["geometry_lod0_0"]["geometry_types"],
        serde_json::json!(["MultiPolygon Z"])
    );
    // The `0.*` family is preferred as the GeoParquet primary_column when
    // present (delft's higher LoDs are Solids/PolyhedralSurfaceZ and
    // GeoParquet-illegal in any case, so geometry_lod0_0 is also the only
    // legal column here).
    assert_eq!(geo["primary_column"], "geometry_lod0_0");
    for solid_lod in ["geometry_lod1_2", "geometry_lod1_3", "geometry_lod2_2"] {
        assert!(
            !columns.contains_key(solid_lod),
            "Solid column {solid_lod} (PolyhedralSurfaceZ) must NOT be in geo.columns"
        );
    }
    // The CRS is PROJJSON (resolved from delft's OGC URL to EPSG:7415).
    assert_eq!(columns["geometry_lod0_0"]["crs"]["id"]["code"], 7415);

    // (b) Geometry field is plain Binary with no geoarrow extension (default).
    let field = builder
        .schema()
        .fields()
        .iter()
        .find(|f| f.name().starts_with("geometry_lod"))
        .expect("a geometry_<lod> column exists");
    assert!(
        !field.metadata().contains_key("ARROW:extension:name"),
        "default output geometry column must not advertise geoarrow.wkb"
    );
}

/// spec-alignment M3, checklist item 3: a table whose geometry is entirely
/// Solid-family carries a `city` object but NO `geo` key at all (spec
/// "The declaration rule": GeoParquet requires a non-empty `columns` and a
/// non-empty `primary_column`, so a table with zero legal columns has no
/// legal `geo` object). Derived from the real delft fixture: every feature's
/// LoD0 footprint geometry is stripped, keeping only its Solid LoDs
/// (1.2/1.3/2.2) — never hand-written CityJSON.
#[test]
fn solid_only_table_has_city_but_no_geo_key() {
    let text = std::fs::read_to_string(fixture("delft.city.jsonl")).unwrap();
    let mut lines = text.lines();
    let header_line = lines.next().unwrap().to_string();
    let mut out_lines = vec![header_line];
    let mut stripped_any = false;
    for line in lines {
        let mut feature: Value = serde_json::from_str(line).unwrap();
        for (_, co) in feature["CityObjects"].as_object_mut().unwrap() {
            if let Some(geoms) = co.get_mut("geometry").and_then(Value::as_array_mut) {
                let before = geoms.len();
                geoms.retain(|g| g.get("lod").and_then(Value::as_str) != Some("0"));
                if geoms.len() != before {
                    stripped_any = true;
                }
            }
        }
        out_lines.push(serde_json::to_string(&feature).unwrap());
    }
    assert!(
        stripped_any,
        "precondition: delft must carry LoD0 geometry to strip"
    );

    let dir = tempfile::tempdir().unwrap();
    let input_path = dir.path().join("delft_solid_only.city.jsonl");
    std::fs::write(&input_path, out_lines.join("\n")).unwrap();

    let out = tempfile::tempdir().unwrap();
    convert(&ConvertOptions::new(input_path, out.path().to_path_buf())).unwrap();

    let file = std::fs::File::open(out.path().join("building.parquet")).unwrap();
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
    let kvs = builder
        .metadata()
        .file_metadata()
        .key_value_metadata()
        .unwrap();

    let city_kv = kvs
        .iter()
        .find(|kv| kv.key == "city")
        .expect("a solid-only table must still carry a city key");
    let city: Value = serde_json::from_str(city_kv.value.as_deref().unwrap()).unwrap();
    let columns = city["columns"]
        .as_array()
        .expect("city.columns must be present");
    assert!(
        !columns.is_empty(),
        "city.columns must describe the Solid columns: {columns:?}"
    );
    assert!(
        columns
            .iter()
            .all(|c| c["geometry_types"] == serde_json::json!(["PolyhedralSurface Z"])),
        "every remaining LoD must be Solid-family: {columns:?}"
    );
    assert!(
        city["primary_column"].is_string(),
        "city.primary_column must still name the highest (Solid) LoD"
    );

    assert!(
        !kvs.iter().any(|kv| kv.key == "geo"),
        "a solid-only table must carry no geo key at all"
    );
}

/// `ConvertOptions::geoarrow = true` restores the GeoParquet/GeoArrow
/// self-description: the `geoarrow.wkb` field extension plus the file-level
/// `geo` key, for GeoPandas/QGIS/GDAL interop.
#[test]
fn geoarrow_opt_in_restores_tag_and_geo_key() {
    let out = tempfile::tempdir().unwrap();
    let mut opts = ConvertOptions::new(fixture("delft.city.jsonl"), out.path().to_path_buf());
    opts.geoarrow = true;
    opts.overwrite = true;
    convert(&opts).unwrap();

    // delft is a single 1st-level family, so by-type conversion writes
    // exactly one main table: building.parquet.
    let file = std::fs::File::open(out.path().join("building.parquet")).unwrap();
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();

    let kvs = builder
        .metadata()
        .file_metadata()
        .key_value_metadata()
        .unwrap();
    assert!(
        kvs.iter().any(|kv| kv.key == "geo"),
        "--geoarrow must write the `geo` key"
    );

    // The LoD0 footprint is the suffixed `geometry_lod0_0` column (delft's
    // GeoParquet primary_column — see `default_convert_writes_geo_key_for_legal_columns_only`);
    // under --geoarrow it advertises the geoarrow.wkb extension.
    let field = builder
        .schema()
        .field_with_name("geometry_lod0_0")
        .unwrap()
        .clone();
    assert_eq!(
        field
            .metadata()
            .get("ARROW:extension:name")
            .map(String::as_str),
        Some("geoarrow.wkb"),
        "--geoarrow must advertise geoarrow.wkb"
    );
}
