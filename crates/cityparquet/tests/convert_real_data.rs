use std::collections::HashSet;
use std::path::PathBuf;

use arrow_array::types::Int32Type;
use arrow_array::{Array, DictionaryArray, Float64Array, StringArray, StructArray};
use cityparquet::CityParquetError;
use cityparquet::compare::{CompareOptions, compare_datasets};
use cityparquet::export::{ExportOptions, export};
use cityparquet::order::hilbert_index;
use cityparquet::package::{ConvertOptions, RowOrder, TableLayout, convert};
use cityparquet::reader::CityParquetReaderBuilder;
use cityparquet::schema::Profile;
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
    let report = convert(&ConvertOptions::new(
        fixture("delft.city.jsonl"),
        out.path().to_path_buf(),
    ))
    .unwrap();
    assert_eq!(report.object_count, 2231);

    let file = std::fs::File::open(out.path().join("cityobjects.parquet")).unwrap();
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
    let pq_meta = builder.metadata().file_metadata();
    let kvs = pq_meta.key_value_metadata().unwrap();
    assert!(kvs.iter().any(|kv| kv.key == "cityparquet_version"));
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

    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(out.path().join("metadata.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["profile"], "core");
}

#[test]
fn railway_full_convert_succeeds() {
    let out = tempfile::tempdir().unwrap();
    let report = convert(&ConvertOptions::new(
        fixture("lod3_railway.city.json"),
        out.path().to_path_buf(),
    ))
    .unwrap();
    assert_eq!(report.object_count, 121);
    assert!(out.path().join("cityobjects.parquet").exists());
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
    convert(&ConvertOptions::new(
        fixture("lod3_railway.city.json"),
        out.path().to_path_buf(),
    ))
    .unwrap();

    let file = std::fs::File::open(out.path().join("cityobjects.parquet")).unwrap();
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
    let reader = builder.build().unwrap();

    let mut checked_material = false;
    let mut checked_texture = false;
    for batch in reader {
        let batch = batch.unwrap();
        let material_col: &StringArray = batch
            .column_by_name("material")
            .unwrap()
            .as_any()
            .downcast_ref()
            .unwrap();
        let texture_col: &StringArray = batch
            .column_by_name("texture")
            .unwrap()
            .as_any()
            .downcast_ref()
            .unwrap();
        for row in 0..batch.num_rows() {
            if !material_col.is_null(row) {
                let map: Value = serde_json::from_str(material_col.value(row)).unwrap();
                assert_every_material_index_below(&map, 83);
                checked_material = true;
            }
            if !texture_col.is_null(row) {
                let map: Value = serde_json::from_str(texture_col.value(row)).unwrap();
                assert_every_texture_ring_valid(&map, 33);
                checked_texture = true;
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

/// The `sidecar_files` list recorded in the parquet footer's own key-value
/// metadata (appended post-encode via `ArrowWriter::append_key_value_metadata`,
/// so it reflects the files ACTUALLY written, matching `metadata.json`).
fn footer_sidecar_files(dir: &std::path::Path) -> Vec<String> {
    let file = std::fs::File::open(dir.join("cityobjects.parquet")).unwrap();
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
    builder.cityparquet_metadata().unwrap().sidecar_files
}

/// Compatibility profile: railway's feature-only appearance sweep (83
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
    let mut opts = ConvertOptions::new(fixture("lod3_railway.city.json"), out.path().to_path_buf());
    opts.profile = Profile::Compatibility;
    let report = convert(&opts).unwrap();

    assert_eq!(report.materials_written, 85);
    assert_eq!(report.textures_written, 34);
    assert_eq!(report.templates_written, 3);
    assert!(out.path().join("materials.parquet").exists());
    assert!(out.path().join("textures.parquet").exists());
    assert!(out.path().join("geometry_templates.parquet").exists());

    let manifest: Value =
        serde_json::from_str(&std::fs::read_to_string(out.path().join("metadata.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["profile"], "compatibility");
    assert_eq!(
        manifest["sidecar_files"],
        serde_json::json!([
            "materials.parquet",
            "textures.parquet",
            "geometry_templates.parquet"
        ])
    );
    assert_eq!(
        footer_sidecar_files(out.path()),
        vec![
            "materials.parquet".to_string(),
            "textures.parquet".to_string(),
            "geometry_templates.parquet".to_string()
        ],
        "the parquet footer's KV sidecar_files must agree with metadata.json"
    );

    // The written template rows must carry their LoD: railway's 3 templates
    // all declare lod "3", and the sidecar's single geometry_properties
    // column is the only place it can live (regression: the shared
    // main-table helper omits "lod" because there LoD is the column name).
    let template_rows =
        cityparquet::sidecar::read_templates(&out.path().join("geometry_templates.parquet"))
            .unwrap();
    assert_eq!(template_rows.len(), 3);
    for (i, row) in template_rows.iter().enumerate() {
        let props = row.geometry_properties.as_ref().unwrap();
        assert!(props.get("type").is_some(), "template {i} missing type");
        assert_eq!(
            props.get("lod").and_then(|v| v.as_str()),
            Some("3"),
            "template {i}: geometry_properties must carry lod"
        );
    }
}

/// Compatibility profile on a dataset with no appearance at all (delft):
/// no sidecar files are written, and both the manifest and the parquet
/// footer's KV metadata say so.
#[test]
fn delft_compatibility_convert_writes_no_sidecars() {
    let out = tempfile::tempdir().unwrap();
    let mut opts = ConvertOptions::new(fixture("delft.city.jsonl"), out.path().to_path_buf());
    opts.profile = Profile::Compatibility;
    let report = convert(&opts).unwrap();

    assert_eq!(report.materials_written, 0);
    assert_eq!(report.textures_written, 0);
    assert_eq!(report.templates_written, 0);
    assert!(!out.path().join("materials.parquet").exists());
    assert!(!out.path().join("textures.parquet").exists());

    let manifest: Value =
        serde_json::from_str(&std::fs::read_to_string(out.path().join("metadata.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["profile"], "compatibility");
    assert_eq!(manifest["sidecar_files"], serde_json::json!([]));
    assert_eq!(
        footer_sidecar_files(out.path()),
        Vec::<String>::new(),
        "the parquet footer's KV sidecar_files must be empty, matching metadata.json"
    );
}

/// M4 task 11 (Step 1/2): the overwrite-purge hazard the `TODO(M4)` comment
/// named. A Compatibility convert of railway into a fresh directory writes
/// `materials.parquet`/`textures.parquet`/`geometry_templates.parquet`
/// alongside `cityobjects.parquet`; overwriting that SAME directory with a
/// Core convert of an unrelated dataset (delft, no appearance/templates of
/// its own) must not leave any of the first run's sidecars behind — a
/// consumer reading the directory afterwards must see exactly what the
/// second run's own `metadata.json` describes (`sidecar_files == []`), never
/// a stale sidecar from a prior run that manifest no longer mentions.
#[test]
fn overwrite_purges_stale_sidecars_from_a_prior_compatibility_convert() {
    let out = tempfile::tempdir().unwrap();
    let mut first =
        ConvertOptions::new(fixture("lod3_railway.city.json"), out.path().to_path_buf());
    first.profile = Profile::Compatibility;
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
    assert!(out.path().join("cityobjects.parquet").exists());

    let manifest: Value =
        serde_json::from_str(&std::fs::read_to_string(out.path().join("metadata.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["profile"], "core");
    assert_eq!(
        manifest["sidecar_files"],
        serde_json::json!([]),
        "the second run's own manifest must say no sidecars, and none must be left on disk"
    );

    let file = std::fs::File::open(out.path().join("cityobjects.parquet")).unwrap();
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
    let mut first =
        ConvertOptions::new(fixture("lod3_railway.city.json"), out.path().to_path_buf());
    first.profile = Profile::Compatibility;
    let first_report = convert(&first).unwrap();
    assert_eq!(first_report.materials_written, 85);
    assert!(out.path().join("materials.parquet").exists());
    assert!(out.path().join("textures.parquet").exists());
    assert!(out.path().join("geometry_templates.parquet").exists());
    assert!(out.path().join("cityobjects.parquet").exists());
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
    // its files were purged, and the main table still has all its rows.
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
    assert!(
        out.path().join("cityobjects.parquet").exists(),
        "a failed overwrite must not purge the existing package's cityobjects.parquet"
    );
    assert!(
        out.path().join("metadata.json").exists(),
        "a failed overwrite must not purge the existing package's metadata.json"
    );

    let file = std::fs::File::open(out.path().join("cityobjects.parquet")).unwrap();
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
    let rows: usize = builder
        .build()
        .unwrap()
        .map(|b| b.unwrap().num_rows())
        .sum();
    assert_eq!(
        rows, 121,
        "the surviving package's own main table must still be intact and readable"
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
/// real bytes into the scratch `cityobjects.parquet` — succeed before the
/// corrupted feature's batch is ever reached, so this is a genuine
/// mid-encode failure, not merely a same-instant one.
#[test]
fn overwrite_with_a_mid_encode_failure_leaves_the_existing_package_intact() {
    // Pre-existing, valid Compatibility package at `out`.
    let out = tempfile::tempdir().unwrap();
    let mut first =
        ConvertOptions::new(fixture("lod3_railway.city.json"), out.path().to_path_buf());
    first.profile = Profile::Compatibility;
    let first_report = convert(&first).unwrap();
    assert_eq!(first_report.materials_written, 85);
    assert!(out.path().join("materials.parquet").exists());
    assert!(out.path().join("textures.parquet").exists());
    assert!(out.path().join("geometry_templates.parquet").exists());
    assert!(out.path().join("cityobjects.parquet").exists());
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
    assert!(out.path().join("cityobjects.parquet").exists());
    assert!(out.path().join("metadata.json").exists());

    let export_dir = tempfile::tempdir().unwrap();
    let exported = export_dir.path().join("export.city.jsonl");
    export(&ExportOptions {
        package_dir: out.path().to_path_buf(),
        output: exported.clone(),
    })
    .unwrap();
    let report = compare_datasets(
        &fixture("lod3_railway.city.json"),
        &exported,
        &CompareOptions::default(),
    )
    .unwrap();
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
/// delft these two bboxes coincide: delft has zero degenerate-geometry
/// drops (`delft_round_trips_losslessly` in `roundtrip_real_data.rs` pins
/// this — no excluded degenerate rings), so a feature's vertex pool and the
/// union of its stored object bboxes describe exactly the same extent.
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

    let file = std::fs::File::open(out.path().join("cityobjects.parquet")).unwrap();
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
    assert!(
        report
            .excluded
            .iter()
            .all(|e| e.starts_with("header: metadata member")),
        "Hilbert ordering must not introduce any new exclusion beyond delft's usual header \
         metadata members, got: {:#?}",
        report.excluded
    );
}

/// Same headline gate as `hilbert_ordering_never_changes_delft_semantics`,
/// for railway (Compatibility profile — the M4 headline round trip): the 3
/// documented degenerate-ring drops and header-metadata exclusions are the
/// ONLY exclusions, exactly as
/// `roundtrip_real_data.rs::railway_compatibility_round_trips_losslessly_with_no_exclusions`
/// pins for `RowOrder::Source`.
#[test]
fn hilbert_ordering_never_changes_railway_compatibility_semantics() {
    let out = tempfile::tempdir().unwrap();
    let mut opts = ConvertOptions::new(fixture("lod3_railway.city.json"), out.path().to_path_buf());
    opts.profile = Profile::Compatibility;
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
        &fixture("lod3_railway.city.json"),
        &exported,
        &CompareOptions::default(),
    )
    .unwrap();
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
        (3, 3),
        "the only non-header exclusions must be the 3 pinned degenerate-ring drops, got: {:#?}",
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
/// row count — used to assert `TableLayout::ByType`'s per-file uniformity
/// and to recombine row counts across the split tables.
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

/// M5 task 5 (Step 2, the pinned fixture facts): delft's only two
/// `object_type` values are `Building`/`BuildingPart`, so `TableLayout::ByType`
/// must write exactly `cityobjects_building.parquet` +
/// `cityobjects_buildingpart.parquet`, nothing else. The FIRST-APPEARANCE
/// order pins `Building` before `BuildingPart`: `crate::encode`'s
/// `BatchIter::advance` sorts each feature's CityObject ids lexicographically
/// before encoding them (`ids.sort()`), and delft's building-part parent id
/// (`"...012869"`) is a strict string PREFIX of its child's id
/// (`"...012869-0"`), so it sorts first — empirically verified against the
/// real fixture (not the raw, HashMap-ordered `CityObjects` JSON object,
/// whose iteration order is unspecified).
#[test]
fn by_type_convert_of_delft_writes_exactly_the_two_pinned_type_tables() {
    let out = tempfile::tempdir().unwrap();
    let mut opts = ConvertOptions::new(fixture("delft.city.jsonl"), out.path().to_path_buf());
    opts.layout = TableLayout::ByType;
    let report = convert(&opts).unwrap();
    assert_eq!(report.object_count, 2231);

    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(out.path().join("metadata.json")).unwrap())
            .unwrap();
    let tables: Vec<String> = serde_json::from_value(manifest["tables"].clone()).unwrap();
    assert_eq!(
        tables,
        vec![
            "cityobjects_building.parquet".to_string(),
            "cityobjects_buildingpart.parquet".to_string(),
        ],
        "expected exactly the two pinned tables in first-appearance order, got: {tables:?}"
    );
    assert!(!out.path().join("cityobjects.parquet").exists());
    for name in &tables {
        assert!(out.path().join(name).exists(), "missing {name}");
    }

    let (building_types, building_count) =
        table_object_types_and_count(&out.path().join("cityobjects_building.parquet"));
    assert_eq!(building_types, HashSet::from(["Building".to_string()]));
    let (part_types, part_count) =
        table_object_types_and_count(&out.path().join("cityobjects_buildingpart.parquet"));
    assert_eq!(part_types, HashSet::from(["BuildingPart".to_string()]));
    assert_eq!(
        building_count + part_count,
        2231,
        "row counts across the split tables must sum to delft's full object count"
    );

    // Every table's footer must carry `cityparquet_version` (required by
    // `cityparquet_metadata()`, which errors without it), and it must agree
    // across every table this run wrote.
    let mut versions = HashSet::new();
    for name in &tables {
        let file = std::fs::File::open(out.path().join(name)).unwrap();
        let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
        let meta = builder.cityparquet_metadata().unwrap();
        versions.insert(meta.cityparquet_version.clone());
    }
    assert_eq!(
        versions.len(),
        1,
        "every table's footer must carry the identical cityparquet_version, got: {versions:?}"
    );
}

/// M5 review follow-up (Minor a): the ByType writer's first-appearance
/// order and per-type writer index must persist ACROSS batches, not just
/// within one — a small `batch_size` (256 vs delft's 2231 objects, so ~9
/// encoded batches, with both types recurring in every one of them) proves
/// the same two pinned tables come out with the full row count, end-to-end.
#[test]
fn by_type_convert_of_delft_survives_many_small_batches() {
    let out = tempfile::tempdir().unwrap();
    let mut opts = ConvertOptions::new(fixture("delft.city.jsonl"), out.path().to_path_buf());
    opts.layout = TableLayout::ByType;
    opts.batch_size = 256;
    let report = convert(&opts).unwrap();
    assert_eq!(report.object_count, 2231);

    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(out.path().join("metadata.json")).unwrap())
            .unwrap();
    let tables: Vec<String> = serde_json::from_value(manifest["tables"].clone()).unwrap();
    assert_eq!(
        tables,
        vec![
            "cityobjects_building.parquet".to_string(),
            "cityobjects_buildingpart.parquet".to_string(),
        ],
        "the same two pinned tables, in the same first-appearance order, regardless of batching"
    );

    let (building_types, building_count) =
        table_object_types_and_count(&out.path().join("cityobjects_building.parquet"));
    assert_eq!(building_types, HashSet::from(["Building".to_string()]));
    let (part_types, part_count) =
        table_object_types_and_count(&out.path().join("cityobjects_buildingpart.parquet"));
    assert_eq!(part_types, HashSet::from(["BuildingPart".to_string()]));
    assert_eq!(
        building_count + part_count,
        2231,
        "rows written across ~9 batches must still sum to delft's full object count"
    );
}

/// M5 task 5 (Step 2/3): railway's 14-distinct-`object_type` fixture fact
/// (pinned in the milestone brief) round-tripped through `TableLayout::ByType` —
/// exactly 14 tables, one object type per file, row counts summing to
/// railway's full object count.
#[test]
fn by_type_convert_of_railway_writes_fourteen_type_tables() {
    let out = tempfile::tempdir().unwrap();
    let mut opts = ConvertOptions::new(fixture("lod3_railway.city.json"), out.path().to_path_buf());
    opts.layout = TableLayout::ByType;
    let report = convert(&opts).unwrap();
    assert_eq!(report.object_count, 121);

    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(out.path().join("metadata.json")).unwrap())
            .unwrap();
    let tables: Vec<String> = serde_json::from_value(manifest["tables"].clone()).unwrap();
    assert_eq!(
        tables.len(),
        14,
        "railway's pinned type set has 14 distinct object types, got: {tables:?}"
    );

    let expected_names: HashSet<String> = [
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
    .into_iter()
    .map(|t| format!("cityobjects_{}.parquet", t.to_lowercase()))
    .collect();
    let actual_names: HashSet<String> = tables.iter().cloned().collect();
    assert_eq!(actual_names, expected_names, "unexpected table name set");

    let mut total = 0usize;
    for name in &tables {
        let path = out.path().join(name);
        assert!(path.exists(), "missing {name}");
        let (types, count) = table_object_types_and_count(&path);
        assert_eq!(
            types.len(),
            1,
            "table {name} must carry exactly one object_type, got {types:?}"
        );
        total += count;
    }
    assert_eq!(
        total, 121,
        "row counts across every split table must sum to railway's full object count"
    );
}
