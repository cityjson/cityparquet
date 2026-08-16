//! M3 task 7 (the milestone claim): full pipeline round-trip proof —
//! convert the real fixture into a package, export it back to CityJSON/Seq,
//! and prove the two are semantically equal via `compare::compare_datasets`.

use std::path::{Path, PathBuf};

use arrow_array::{Array, StringArray};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

use cityparquet::compare::{CompareOptions, Exclusions, compare_datasets};
use cityparquet::export::{ExportOptions, export};
use cityparquet::package::{ConvertOptions, RowOrder, convert};
use cityparquet::recipe::RecipePreset;
use cityparquet::stac::properties::PackageTables;

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

/// A committed, in-tree fixture under `crates/cityparquet/tests/data/`, as
/// opposed to the large datasets `fixture()` fetches into the gitignored
/// `tests/fixtures/`. Used for small, hand-derived fixtures that have no public
/// download URL, so they must live in the repo to be reproducible on a fresh
/// clone / in CI (mirrors the `data_fixture` helper in the CityGML test files).
fn data_fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(name);
    assert!(p.exists(), "missing committed fixture {name} in tests/data");
    p
}

/// Converts `input` into a fresh tempdir package, exports it back to
/// `.city.jsonl`, and returns the export's path alongside the tempdirs that
/// back both (kept alive so the caller can still read the file).
fn convert_and_export(input: &str) -> (PathBuf, tempfile::TempDir, tempfile::TempDir) {
    convert_and_export_path(&fixture(input))
}

/// Like [`convert_and_export`] but takes a resolved path, so a caller can hand
/// it a [`data_fixture`] (committed, in-tree) rather than a fetched one.
fn convert_and_export_path(input: &Path) -> (PathBuf, tempfile::TempDir, tempfile::TempDir) {
    let package_dir = tempfile::tempdir().unwrap();
    convert(&ConvertOptions::new(
        input.to_path_buf(),
        package_dir.path().to_path_buf(),
    ))
    .unwrap();

    let export_dir = tempfile::tempdir().unwrap();
    let output = export_dir.path().join("export.city.jsonl");
    export(&ExportOptions {
        package_dir: package_dir.path().to_path_buf(),
        output: output.clone(),
    })
    .unwrap();

    (output, package_dir, export_dir)
}

/// The real `lod3_railway.city.json` fixture carries no `referenceSystem` at
/// all. Coordinate-bearing input with no resolvable CRS converts to an
/// explicit `city.crs: null` rather than failing (spec "CRS rules": "an
/// unresolvable CRS is declared, not fatal"), so tests below that want a
/// GEOREFERENCED railway conversion (or comparison) use a small on-disk COPY
/// with a CRS injected via JSON mutation of the real fixture — never
/// hand-written CityJSON. Used both as
/// the conversion INPUT and, where a test also compares against "the
/// source", as that comparison baseline.
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

/// spec "Column naming and reservation rules" (gap 14): `other_attributes` is
/// itself a reserved name, so a source attribute literally called that has
/// nowhere further to divert to and is rejected outright — the same error
/// path as any other undiverted reserved-name collision
/// ([`cityparquet_schema::model::CityParquetSchema::validate`]'s "collides
/// with a reserved or geometry column name" text). Derived from the real
/// `delft` fixture with one injected attribute named `other_attributes`.
#[test]
fn attribute_literally_named_other_attributes_is_rejected() {
    let text = std::fs::read_to_string(fixture("delft.city.jsonl")).unwrap();
    let mut injected = false;
    let mut out_lines = Vec::new();
    for line in text.lines() {
        let mut doc: serde_json::Value = serde_json::from_str(line).unwrap();
        if !injected && let Some(cos) = doc.get_mut("CityObjects").and_then(|v| v.as_object_mut()) {
            for co in cos.values_mut() {
                if let Some(attrs) = co.get_mut("attributes").and_then(|v| v.as_object_mut()) {
                    attrs.insert(
                        "other_attributes".to_string(),
                        serde_json::json!("should-be-rejected"),
                    );
                    injected = true;
                    break;
                }
            }
        }
        out_lines.push(serde_json::to_string(&doc).unwrap());
    }
    assert!(
        injected,
        "delft fixture must carry at least one object with attributes to inject into"
    );

    let dir = tempfile::tempdir().unwrap();
    let input = dir
        .path()
        .join("delft_other_attributes_collision.city.jsonl");
    std::fs::write(&input, out_lines.join("\n") + "\n").unwrap();

    let out = tempfile::tempdir().unwrap();
    let opts = ConvertOptions::new(input, out.path().to_path_buf());
    let err =
        convert(&opts).expect_err("an attribute literally named other_attributes must be rejected");
    let msg = format!("{err}");
    assert!(
        msg.contains("other_attributes") && msg.contains("collides"),
        "error must name the offending attribute and the reserved-name collision, got: {msg}"
    );
}

/// spec "Appearance & templates" (gap 13): a `transformationMatrix` that
/// isn't exactly 16 values is rejected at convert time, not silently
/// truncated/padded. Derived from the real `lod3_railway` fixture (with an
/// injected CRS, like [`railway_fixture_with_crs`]): the first
/// `GeometryInstance`'s real `transformationMatrix` is truncated to 3 values.
/// A 16-length matrix round-tripping exactly is already proven by every
/// other railway round-trip test above (real `GeometryInstance`s all carry
/// genuine 16-value matrices).
#[test]
fn template_transformation_matrix_wrong_length_is_rejected() {
    let mut doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(fixture("lod3_railway.city.json")).unwrap())
            .unwrap();
    doc["metadata"]["referenceSystem"] =
        serde_json::json!("https://www.opengis.net/def/crs/EPSG/0/7415");

    let mut truncated = false;
    if let Some(cos) = doc["CityObjects"].as_object_mut() {
        'outer: for co in cos.values_mut() {
            if let Some(geoms) = co["geometry"].as_array_mut() {
                for geom in geoms.iter_mut() {
                    if geom.get("type").and_then(|t| t.as_str()) == Some("GeometryInstance")
                        && geom.get("transformationMatrix").is_some()
                    {
                        geom["transformationMatrix"] = serde_json::json!([1.0, 0.0, 0.0]);
                        truncated = true;
                        break 'outer;
                    }
                }
            }
        }
    }
    assert!(
        truncated,
        "fixture must carry at least one GeometryInstance with a transformationMatrix"
    );

    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("railway_bad_matrix.city.json");
    std::fs::write(&input, serde_json::to_string(&doc).unwrap()).unwrap();

    let out = tempfile::tempdir().unwrap();
    let opts = ConvertOptions::new(input, out.path().to_path_buf());
    let err = convert(&opts).expect_err("a non-16-length transformationMatrix must be rejected");
    assert!(
        format!("{err}").contains("16"),
        "error must call out the exactly-16 requirement, got: {err}"
    );
}

/// M5 task 3 (the milestone claim): presets change bytes, never semantics.
/// Every named [`RecipePreset`] must still round-trip delft losslessly —
/// the per-column tuning a preset picks is purely a `WriterProperties`
/// concern and must never affect what a reader gets back.
#[test]
fn every_recipe_preset_round_trips_delft_losslessly() {
    assert_eq!(
        RecipePreset::ALL.len(),
        6,
        "this gate must cover exactly the 6 binding presets"
    );

    for preset in RecipePreset::ALL {
        let package_dir = tempfile::tempdir().unwrap();
        let mut opts = ConvertOptions::new(
            fixture("delft.city.jsonl"),
            package_dir.path().to_path_buf(),
        );
        opts.recipe = preset.recipe();
        convert(&opts).unwrap();

        let export_dir = tempfile::tempdir().unwrap();
        let output = export_dir.path().join("export.city.jsonl");
        export(&ExportOptions {
            package_dir: package_dir.path().to_path_buf(),
            output: output.clone(),
        })
        .unwrap();

        let report = compare_datasets(
            &fixture("delft.city.jsonl"),
            &output,
            &CompareOptions::default(),
        )
        .unwrap();
        assert!(
            report.equal,
            "preset {} must round-trip delft losslessly; differences: {:#?}",
            preset.name(),
            report.differences
        );
        assert!(
            report.differences.is_empty(),
            "preset {} produced non-empty differences",
            preset.name()
        );
        // Pinned counts updated alongside the comparator's coordinate-degenerate
        // ring fix (3DBAG tile `9-284-556.city.json` finding; see
        // `crate::compare`'s module docs and `delft_round_trips_losslessly`
        // below for the full explanation): delft's real, unmutated source
        // carries 8 objects with an index-distinct/coordinate-identical ring,
        // previously invisible to the INDEX-only degenerate check. Not a
        // regression — `report.equal`/`differences.is_empty()` above still hold.
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
            "preset {}'s only non-header exclusions must be the 16 pinned \
             coordinate-degenerate-ring drops (8 objects, source + export side each), got: {:#?}",
            preset.name(),
            non_header_excluded
        );
        assert!(
            !header_excluded.is_empty(),
            "preset {} must still document delft's header metadata members, got: {:#?}",
            preset.name(),
            report.excluded
        );
    }
}

/// delft's LoD0 footprint is stored in the un-suffixed `geometry` column, with
/// its LoD recorded in `geometry_properties.lod` (§9/§12). Export must recover
/// that LoD so the round-tripped CityJSON carries a genuine `lod:"0"` geometry
/// (not a lod-less one). This is the narrow behavioural pin behind the fuller
/// `delft_round_trips_losslessly` semantic-equality gate below.
#[test]
fn delft_lod0_footprint_round_trips_with_lod_restored() {
    let (exported, _package_dir, _export_dir) = convert_and_export("delft.city.jsonl");
    let text = std::fs::read_to_string(&exported).unwrap();
    assert!(
        text.contains("\"lod\":\"0.0\""),
        "exported delft must restore LoD0 geometry with the canonical lod \"0.0\""
    );
}

#[test]
fn delft_round_trips_losslessly() {
    let (exported, _package_dir, _export_dir) = convert_and_export("delft.city.jsonl");
    let report = compare_datasets(
        &fixture("delft.city.jsonl"),
        &exported,
        &CompareOptions::default(),
    )
    .unwrap();
    assert!(
        report.equal,
        "delft must round-trip losslessly; differences: {:#?}",
        report.differences
    );
    assert!(report.differences.is_empty());
    // delft has no appearance and no GeometryInstances, but its header DOES
    // set metadata members (title, geographicalExtent, pointOfContact) that
    // are documented exclusions, not comparisons.
    //
    // delft's boundaries ALSO turn out to carry 8 real, unmutated objects
    // with a ring whose vertex INDICES are pairwise distinct (so the
    // writer's index-only `normalise_ring` — deliberately, see its docs —
    // does not drop it) but which dequantise to fewer than 3 distinct
    // coordinates: the exact same class of defect as the 3DBAG tile
    // `9-284-556.city.json` finding that motivated extending the
    // comparator's normalisation (see `crate::compare`'s module docs). This
    // was invisible before that fix (both sides silently kept the
    // degenerate-but-structurally-valid ring, comparing equal despite it not
    // describing a real face) and is now caught and logged as excluded on
    // both the source and the exported side — 16 entries (8 objects x 2
    // sides). Every excluded line must be one of those 16 or a documented
    // header metadata member, and nothing else — any OTHER exclusion here
    // would mean something real (appearance, an instance, an unrelated
    // degenerate ring) slipped through undetected.
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
        "delft's only non-header exclusions must be the 16 pinned coordinate-degenerate-ring \
         drops (8 objects, source + export side each), got: {:#?}",
        non_header_excluded
    );
    assert!(
        !header_excluded.is_empty(),
        "delft's header sets metadata members; expected at least one documented header-metadata \
         exclusion, got none. Full excluded: {:#?}",
        report.excluded
    );
}

#[test]
fn railway_round_trips_losslessly_modulo_documented_drops() {
    let (_crs_dir, railway_path) = railway_fixture_with_crs();
    let (exported, _package_dir, _export_dir) = convert_and_export_path(&railway_path);
    let opts = CompareOptions {
        coord_tolerance: [0.0; 3],
        exclusions: Exclusions {
            appearance: true,
            geometry_instances: true,
        },
    };
    let report = compare_datasets(&railway_path, &exported, &opts).unwrap();
    assert!(
        report.equal,
        "railway must round-trip losslessly modulo the documented appearance/instance drops; \
         differences: {:#?}",
        report.differences
    );
    assert!(report.differences.is_empty());

    // Split header-metadata exclusions (documented, unbounded — whatever
    // metadata members railway's header happens to set) from everything
    // else, and pin the non-header set exactly as before: any non-header,
    // non-pinned exclusion must still fail this test.
    let (header_excluded, non_header_excluded): (Vec<&String>, Vec<&String>) = report
        .excluded
        .iter()
        .partition(|e| e.starts_with("header: metadata member"));

    // The exact exclusion breakdown, recounted by category. The
    // appearance/instances totals are pinned against counts already proven
    // elsewhere: 105 stored geometries carry material or texture
    // (export_real_data.rs's appearance_refs_dropped), 15 objects carry a
    // GeometryInstance (instance_geometries_dropped) — DOUBLED to 210/30
    // since spec-alignment gap 19: sidecars (materials/textures/
    // geometry_templates) are now written whenever the source has content
    // for them, so `export` actually RESTORES railway's appearance/instances
    // (unlike the old Core-profile default, which dropped them). With
    // `exclusions.appearance`/`exclusions.geometry_instances` on, the
    // comparator excludes rather than compares that data on EACH side that
    // carries it — now both the source and the export do, logging one
    // exclusion entry per side. The degenerate count was updated from the
    // writer-only-index-drop figure of 3 to 23 alongside the comparator's
    // coordinate-degenerate ring fix (3DBAG tile `9-284-556.city.json`
    // finding; see `crate::compare`'s module docs): railway's real,
    // unmutated source carries 20 MORE objects whose boundaries include an
    // index-distinct/coordinate-identical ring, previously invisible to the
    // INDEX-only degenerate check (the writer's 3 pinned drops in
    // `wkb_roundtrip_real_data.rs::geometries_with_drops` are unaffected —
    // that pin is about the WRITER's own index-based normalisation, which
    // this fix does not touch).
    let appearance = non_header_excluded
        .iter()
        .filter(|e| e.contains("exclusions.appearance"))
        .count();
    let instances = non_header_excluded
        .iter()
        .filter(|e| e.contains("exclusions.geometry_instances"))
        .count();
    let degenerate = non_header_excluded
        .iter()
        .filter(|e| e.contains("degenerate ring"))
        .count();
    assert_eq!(
        (appearance, instances, degenerate),
        (210, 30, 23),
        "exclusion breakdown must match the pinned pipeline counts, got: {:#?}",
        non_header_excluded
    );
    assert_eq!(
        non_header_excluded.len(),
        263,
        "210 appearance + 30 instances + 23 degenerate = 263 total non-header exclusions, \
         nothing else, got: {:#?}",
        non_header_excluded
    );

    // Railway's header sets `metadata.geographicalExtent`, a documented
    // exclusion: it must be logged, never silently dropped.
    assert!(
        !header_excluded.is_empty(),
        "railway's header sets metadata members; expected at least one documented \
         header-metadata exclusion, got none. Full excluded: {:#?}",
        report.excluded
    );
}

/// Gap 8 (texture ring nesting) evidence: `GMLID_BUI46739_1739_10911`'s
/// single (LoD 3, `MultiSurface`) geometry carries a real multi-ring
/// textured face — face index 2 of theme `"visual"` has 7 rings (an
/// exterior ring plus 6 interior rings, almost certainly window openings
/// cut into a wall), each independently textured with its OWN UV coordinate
/// count: 4 UVs on the exterior ring, 15 on each interior ring. Flattening a
/// face to a single `[texId, uv…]` (the loss gap 8 exists to rule out) would
/// irrecoverably merge these seven differently-sized UV lists into one, so
/// this is a real fixture proof — not a hand-built one — that ring
/// structure survives the full `convert` (encode + appearance interning) →
/// `export` (sidecar de-interning) pipeline intact.
#[test]
fn railway_interior_ring_texture_survives_round_trip() {
    let (_crs_dir, railway_path) = railway_fixture_with_crs();

    // Face 2's ring shape as `[ring0_uv_count, ring1_uv_count, ...]` (each
    // ring is `[texId, [u,v], [u,v], ...]`, so `len() - 1` is its UV count).
    let ring_shape = |doc: &serde_json::Value| -> Vec<usize> {
        let face = &doc["CityObjects"]["GMLID_BUI46739_1739_10911"]["geometry"][0]["texture"]["visual"]
            ["values"][2];
        face.as_array()
            .expect("face must be an array of rings")
            .iter()
            .map(|ring| ring.as_array().expect("ring must be an array").len() - 1)
            .collect()
    };

    let source_doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&railway_path).unwrap()).unwrap();
    let source_shape = ring_shape(&source_doc);
    assert_eq!(
        source_shape,
        vec![4, 15, 15, 15, 15, 15, 15],
        "precondition: the fixture's known multi-ring textured face has this exact shape"
    );

    let package_dir = tempfile::tempdir().unwrap();
    convert(&ConvertOptions::new(
        railway_path.clone(),
        package_dir.path().to_path_buf(),
    ))
    .unwrap();
    let export_dir = tempfile::tempdir().unwrap();
    let output = export_dir.path().join("export.city.json");
    export(&ExportOptions {
        package_dir: package_dir.path().to_path_buf(),
        output: output.clone(),
    })
    .unwrap();
    let exported_doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&output).unwrap()).unwrap();

    assert_eq!(
        ring_shape(&exported_doc),
        source_shape,
        "gap 8: every ring's independent UV count must survive convert -> export with no \
         flattening of interior rings into the exterior ring"
    );
}

/// M4 task 11 (the milestone's headline gate): a round trip with NO
/// exclusions at all — appearance and GeometryInstance geometries are
/// restored from the sidecars (M4 tasks 6-10, always written now that
/// sidecars are content-gated rather than profile-gated), so unlike
/// `railway_round_trips_losslessly_modulo_documented_drops` above (an older
/// pin from when Core-vs-Compatibility was a real choice), the only
/// remaining exclusions are the 23 pinned degenerate-ring drops (updated
/// alongside the comparator's coordinate-degenerate fix — see the comment in
/// `railway_round_trips_losslessly_modulo_documented_drops` above) and
/// whatever header metadata members railway's header sets (documented,
/// unbounded). Any OTHER exclusion here would mean appearance or an
/// instance silently failed to round-trip.
#[test]
fn railway_compatibility_round_trips_losslessly_with_no_exclusions() {
    let (_crs_dir, railway_path) = railway_fixture_with_crs();
    let (exported, _package_dir, _export_dir) = convert_and_export_path(&railway_path);
    let report = compare_datasets(&railway_path, &exported, &CompareOptions::default()).unwrap();
    assert!(
        report.equal,
        "railway must round-trip losslessly with NO exclusions under the Compatibility profile; \
         differences: {:#?}",
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
        degenerate, 23,
        "the 23 pinned coordinate-degenerate-ring drops must still be the only \
         non-header exclusions, got: {:#?}",
        non_header_excluded
    );
    assert_eq!(
        non_header_excluded.len(),
        23,
        "with appearance and instances now round-tripping, every non-header exclusion must be \
         one of the 23 pinned degenerate-ring notes, nothing else, got: {:#?}",
        non_header_excluded
    );
    assert!(
        !header_excluded.is_empty(),
        "railway's header sets metadata members; expected at least one documented \
         header-metadata exclusion, got none. Full excluded: {:#?}",
        report.excluded
    );
}

/// M4 task 11: delft carries no appearance and no GeometryInstances, so BOTH
/// profiles must round-trip with no exclusions beyond delft's own documented
/// header metadata members and the 16 pinned coordinate-degenerate-ring
/// drops (see `delft_round_trips_losslessly` above for the full explanation)
/// — delft has no appearance/templates to write sidecars for, so this must
/// not introduce any new difference or exclusion relative to the round trip
/// already proven by `delft_round_trips_losslessly`.
#[test]
fn delft_compatibility_round_trips_losslessly_with_only_header_exclusions() {
    let (exported, _package_dir, _export_dir) = convert_and_export("delft.city.jsonl");
    let report = compare_datasets(
        &fixture("delft.city.jsonl"),
        &exported,
        &CompareOptions::default(),
    )
    .unwrap();
    assert!(
        report.equal,
        "delft must round-trip losslessly under the Compatibility profile too; differences: {:#?}",
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
        (16, 16),
        "delft's only non-header exclusions must be the 16 pinned coordinate-degenerate-ring \
         drops (8 objects, source + export side each; see `delft_round_trips_losslessly` for \
         the full explanation), got: {:#?}",
        non_header_excluded
    );
    assert!(
        !header_excluded.is_empty(),
        "delft's header sets metadata members; expected at least one documented header-metadata \
         exclusion, got none. Full excluded: {:#?}",
        report.excluded
    );
}

/// M4 task 4 (Step 4): the comparator's own Solid-family degenerate
/// normalisation must agree with the writer's real drop, end-to-end.
/// Derived from delft's `NL.IMBAG.Pand.0503100000012869-0` lod-1.2 Solid
/// (single shell, 6 faces, `semantics.values == [[0,2,2,2,2,1]]`): face 2's
/// exterior ring degenerates to `[a, b, a]` (its own first two indices).
/// Compared against the mutated SOURCE (not the whole original delft file —
/// this tempdir file carries only the one mutated feature line, so it can't
/// compare equal against delft's other 1114 features). With
/// `CompareOptions::default()` the round trip must be `equal`, with the
/// only exclusions being the single degenerate-drop log line and delft's
/// own documented header metadata members (matching
/// `delft_round_trips_losslessly` above) — proving `compare_datasets`'s own
/// Solid realignment (fixed alongside the writer's in this same change)
/// agrees with what the writer actually produced, not merely with its own
/// unrealigned copy of the source.
#[test]
fn delft_derived_solid_face_drop_round_trips_and_comparator_agrees_with_the_writer() {
    let text = std::fs::read_to_string(fixture("delft.city.jsonl")).unwrap();
    let mut lines = text.lines();
    let header_line = lines.next().unwrap().to_string();

    const OBJ_ID: &str = "NL.IMBAG.Pand.0503100000012869-0";
    let mut mutated_line = None;
    for line in lines {
        if !line.contains(OBJ_ID) {
            continue;
        }
        let mut feature: serde_json::Value = serde_json::from_str(line).unwrap();
        let geom = feature["CityObjects"][OBJ_ID]["geometry"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|g| g["lod"] == "1.2" && g["type"] == "Solid")
            .expect("delft's Pand-0 must carry a lod 1.2 Solid");

        let sem_values: Vec<i64> =
            serde_json::from_value(geom["semantics"]["values"][0].clone()).unwrap();
        assert_eq!(sem_values.len(), 6, "fixture fact: shell 0 has 6 faces");

        let ring = &mut geom["boundaries"][0][2][0];
        let indices: Vec<i64> = serde_json::from_value(ring.clone()).unwrap();
        let (a, b) = (indices[0], indices[1]);
        *ring = serde_json::json!([a, b, a]);
        mutated_line = Some(serde_json::to_string(&feature).unwrap());
        break;
    }
    let mutated_line = mutated_line.expect("delft.city.jsonl must contain the target object");

    let source_dir = tempfile::tempdir().unwrap();
    let source_path = source_dir.path().join("delft_solid_derived.city.jsonl");
    std::fs::write(&source_path, format!("{header_line}\n{mutated_line}\n")).unwrap();

    let package_dir = tempfile::tempdir().unwrap();
    convert(&ConvertOptions::new(
        source_path.clone(),
        package_dir.path().to_path_buf(),
    ))
    .unwrap();
    let export_dir = tempfile::tempdir().unwrap();
    let output = export_dir.path().join("export.city.jsonl");
    export(&ExportOptions {
        package_dir: package_dir.path().to_path_buf(),
        output: output.clone(),
    })
    .unwrap();

    let report = compare_datasets(&source_path, &output, &CompareOptions::default()).unwrap();
    assert!(
        report.equal,
        "the derived source and its round-tripped export must compare equal; differences: {:#?}",
        report.differences
    );
    assert!(report.differences.is_empty());

    let (header_excluded, non_header_excluded): (Vec<&String>, Vec<&String>) = report
        .excluded
        .iter()
        .partition(|e| e.starts_with("header: metadata member"));
    assert_eq!(
        non_header_excluded.len(),
        1,
        "the only non-header exclusion must be the single degenerate-ring/surface drop, got: {:#?}",
        non_header_excluded
    );
    assert!(
        non_header_excluded[0].contains("normalised away 1 degenerate ring(s), 1 surface(s)"),
        "got: {}",
        non_header_excluded[0]
    );
    assert!(
        !header_excluded.is_empty(),
        "delft's header sets metadata members; expected at least one documented header exclusion, \
         got: {:#?}",
        report.excluded
    );
}

/// Like [`convert_and_export`] but also lets the caller pick `ordering` —
/// needed for the M5 task 5 by-type round-trip gates below (and their
/// Hilbert-composed variant).
fn convert_and_export_with(
    input: &str,
    ordering: RowOrder,
) -> (PathBuf, tempfile::TempDir, tempfile::TempDir) {
    convert_and_export_with_path(&fixture(input), ordering)
}

/// [`convert_and_export_with`] taking an already-resolved input path.
fn convert_and_export_with_path(
    input: &Path,
    ordering: RowOrder,
) -> (PathBuf, tempfile::TempDir, tempfile::TempDir) {
    let package_dir = tempfile::tempdir().unwrap();
    let mut opts = ConvertOptions::new(input.to_path_buf(), package_dir.path().to_path_buf());
    opts.ordering = ordering;
    convert(&opts).unwrap();

    let export_dir = tempfile::tempdir().unwrap();
    let output = export_dir.path().join("export.city.jsonl");
    export(&ExportOptions {
        package_dir: package_dir.path().to_path_buf(),
        output: output.clone(),
    })
    .unwrap();

    (output, package_dir, export_dir)
}

/// M5 task 5 (Step 3): delft under by-type (Core profile) must round-trip
/// exactly as losslessly as `delft_round_trips_losslessly` above — the
/// table layout is purely a physical-file concern, never a semantic one.
/// Per the family-grouping rule, delft's Building + BuildingPart share the
/// single `building.parquet` file (see
/// `by_type_convert_of_delft_writes_exactly_one_family_table` in
/// `convert_real_data.rs`), so `export` must still read the whole dataset
/// back from that one table.
#[test]
fn delft_by_type_round_trips_losslessly() {
    let (exported, package_dir, _export_dir) =
        convert_and_export_with("delft.city.jsonl", RowOrder::Source);
    assert!(
        package_dir.path().join("building.parquet").exists(),
        "sanity: this must actually be a split-by-type package"
    );
    assert!(
        !package_dir.path().join("buildingpart.parquet").exists(),
        "BuildingPart is 2nd-level and must share building.parquet, not get its own file"
    );
    let report = compare_datasets(
        &fixture("delft.city.jsonl"),
        &exported,
        &CompareOptions::default(),
    )
    .unwrap();
    assert!(
        report.equal,
        "ByType-layout delft must round-trip losslessly; differences: {:#?}",
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
        (16, 16),
        "delft's only non-header exclusions must be the 16 pinned coordinate-degenerate-ring \
         drops (8 objects, source + export side each; see `delft_round_trips_losslessly` for \
         the full explanation), got: {:#?}",
        non_header_excluded
    );
    assert!(
        !header_excluded.is_empty(),
        "delft's header sets metadata members; expected at least one documented header-metadata \
         exclusion, got none. Full excluded: {:#?}",
        report.excluded
    );
}

/// M5 task 5 (Step 3), updated for the by-module split (spec "By-module
/// object-table layout"): railway under by-module (Compatibility profile) —
/// the M4 headline round-trip gate
/// (`railway_compatibility_round_trips_losslessly_with_no_exclusions` above)
/// must hold across all 9 pinned module tables (railway's 14 distinct
/// `object_type` values collapse to 9 distinct CityGML 3.0 modules — see
/// `by_type_convert_of_railway_writes_nine_module_tables` in
/// `convert_real_data.rs` for the exact module membership).
#[test]
fn railway_by_type_compatibility_round_trips_losslessly_with_no_exclusions() {
    let (_crs_dir, railway_path) = railway_fixture_with_crs();
    let (exported, package_dir, _export_dir) =
        convert_and_export_with_path(&railway_path, RowOrder::Source);
    let tables = PackageTables::open(package_dir.path()).unwrap().tables;
    assert_eq!(
        tables.len(),
        9,
        "railway's pinned type set collapses to 9 distinct CityGML modules, got: {tables:?}"
    );

    let report = compare_datasets(&railway_path, &exported, &CompareOptions::default()).unwrap();
    assert!(
        report.equal,
        "By-type railway must round-trip losslessly with NO exclusions under the \
         Compatibility profile; differences: {:#?}",
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
        "the 23 pinned coordinate-degenerate-ring drops must still be the only \
         non-header exclusions, got: {:#?}",
        non_header_excluded
    );
    assert!(
        !header_excluded.is_empty(),
        "railway's header sets metadata members; expected at least one documented \
         header-metadata exclusion, got none. Full excluded: {:#?}",
        report.excluded
    );
}

/// M5 task 5 (Step 3, the composed smoke gate): `RowOrder::Hilbert` and the
/// by-type writer compose independently (Hilbert reorders FEATURES before
/// encode; by-type partitions ENCODED ROWS after — see `TableWriters`'s doc
/// comment) — one assertion proving delft still round-trips losslessly with
/// Hilbert ordering turned on.
#[test]
fn delft_hilbert_and_by_type_compose_and_round_trip_losslessly() {
    let (exported, package_dir, _export_dir) =
        convert_and_export_with("delft.city.jsonl", RowOrder::Hilbert);
    assert!(
        package_dir.path().join("building.parquet").exists(),
        "sanity: this must actually be a split-by-type package"
    );
    let report = compare_datasets(
        &fixture("delft.city.jsonl"),
        &exported,
        &CompareOptions::default(),
    )
    .unwrap();
    assert!(
        report.equal,
        "Hilbert-ordered, ByType-laid-out delft must still round-trip losslessly; \
         differences: {:#?}",
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
        (16, 16),
        "delft's only non-header exclusions must be the 16 pinned coordinate-degenerate-ring \
         drops (8 objects, source + export side each; see `delft_round_trips_losslessly` for \
         the full explanation), got: {:#?}",
        non_header_excluded
    );
    assert!(
        !header_excluded.is_empty(),
        "delft's header sets metadata members; expected at least one documented header-metadata \
         exclusion, got none. Full excluded: {:#?}",
        report.excluded
    );
}

/// G7 (null-shorthand canonicalisation): CityJSON permits a single `null` in
/// `semantics.values` to stand for a whole shell/solid with no per-face
/// semantics. G7 stores the expanded per-face form, so export yields expanded
/// nulls rather than the source shorthand — semantically equal, and §17's
/// round-trip is defined up to that canonicalisation. The comparator must
/// therefore treat the two as equal. Derived from delft by rewriting a Solid's
/// `semantics.values` to the shorthand form.
#[test]
fn null_shorthand_semantics_round_trips() {
    let text = std::fs::read_to_string(fixture("delft.city.jsonl")).unwrap();
    let header = text.lines().next().unwrap();
    let mut rewrote = false;
    let mut feature: serde_json::Value =
        serde_json::from_str(text.lines().nth(1).expect("delft has feature lines")).unwrap();
    for (_, co) in feature["CityObjects"].as_object_mut().unwrap() {
        for g in co
            .get_mut("geometry")
            .and_then(|g| g.as_array_mut())
            .into_iter()
            .flatten()
        {
            let is_solid_with_semantics = g.get("type").and_then(|t| t.as_str()) == Some("Solid")
                && g.get("semantics").is_some();
            if is_solid_with_semantics {
                // One `null` per shell (the whole shell carries no semantics).
                let nshells = g["boundaries"].as_array().map_or(0, Vec::len);
                g["semantics"]["values"] =
                    serde_json::json!(vec![serde_json::Value::Null; nshells]);
                rewrote = true;
            }
        }
    }
    assert!(rewrote, "delft feature must carry a Solid with semantics");

    let src_dir = tempfile::tempdir().unwrap();
    let src = src_dir.path().join("delft_shorthand.city.jsonl");
    std::fs::write(
        &src,
        format!("{header}\n{}", serde_json::to_string(&feature).unwrap()),
    )
    .unwrap();

    let package = tempfile::tempdir().unwrap();
    convert(&ConvertOptions::new(
        src.clone(),
        package.path().to_path_buf(),
    ))
    .unwrap();
    let export_dir = tempfile::tempdir().unwrap();
    let exported = export_dir.path().join("export.city.jsonl");
    export(&ExportOptions {
        package_dir: package.path().to_path_buf(),
        output: exported.clone(),
    })
    .unwrap();

    let report = compare_datasets(&src, &exported, &CompareOptions::default()).unwrap();
    assert!(
        report.equal,
        "null-shorthand semantics must round-trip up to canonicalisation; differences: {:#?}",
        report.differences
    );
}

/// G9 (§5.1): a per-object `geographicalExtent`, which has no dedicated
/// column, must survive the round-trip via `other`.
///
/// spec "Addresses" (gap 10): `helsinki_address.city.jsonl`'s `address`
/// entries use non-postal member names (`Country`, `Locality`,
/// `ThoroughfareName`, `ThoroughfareNumber` — none of which match the
/// recognised set in [`cityparquet::schema`]'s "Addresses" mapping, chiefly
/// because of casing), so nothing in them maps onto the reserved struct.
/// This is the spec's documented, accepted loss (non-postal source address
/// members are dropped) — not a bug: this test proves the drop happens
/// cleanly (every entry still round-trips as an empty object, in the same
/// cardinality) rather than corrupting anything, and that the UNRELATED
/// `geographicalExtent` member is entirely unaffected. The fixture has no
/// `address[].location` to exercise the `MultiPoint` round-trip — see
/// `address_location_round_trips` for that, using a dedicated small fixture.
///
/// The fixture is a small hand-derived subset of the City of Helsinki open 3D
/// city model. It has no public download URL, so it is committed in-tree under
/// `tests/data/` (via [`data_fixture`]) rather than fetched by `just fixtures`.
#[test]
fn helsinki_non_postal_address_members_are_dropped_not_corrupted() {
    let (exported, _package_dir, _export_dir) =
        convert_and_export_path(&data_fixture("helsinki_address.city.jsonl"));
    let report = compare_datasets(
        &data_fixture("helsinki_address.city.jsonl"),
        &exported,
        &CompareOptions::default(),
    )
    .unwrap();
    assert!(
        report.equal,
        "the comparator maps both sides through the same recognised-member \
         mapping (spec \"Addresses\"), so this fixture's non-recognised keys \
         compare equal on both sides once dropped; differences: {:#?}",
        report.differences
    );

    // Direct proof the drop is clean: every source object's TWO address
    // entries survive (same cardinality), each now an empty object (every
    // non-recognised member gone, nothing spuriously kept or corrupted),
    // and `geographicalExtent` is untouched.
    let read_field = |path: &std::path::Path, field: &str| -> Vec<(String, serde_json::Value)> {
        let text = std::fs::read_to_string(path).unwrap();
        let mut out = Vec::new();
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            let doc: serde_json::Value = serde_json::from_str(line).unwrap();
            let Some(cos) = doc.get("CityObjects").and_then(|v| v.as_object()) else {
                continue;
            };
            for (id, co) in cos {
                if let Some(v) = co.get(field) {
                    out.push((id.clone(), v.clone()));
                }
            }
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    };
    let source_addr = read_field(&data_fixture("helsinki_address.city.jsonl"), "address");
    assert!(
        !source_addr.is_empty(),
        "fixture must actually carry address members"
    );
    let export_addr = read_field(&exported, "address");
    assert_eq!(
        export_addr.len(),
        source_addr.len(),
        "every object with a source address must still carry one after export"
    );
    for (_, addr) in &export_addr {
        let arr = addr.as_array().expect("address must be an array");
        assert_eq!(
            arr.len(),
            2,
            "cardinality of each object's address list is preserved"
        );
        for entry in arr {
            assert_eq!(
                entry,
                &serde_json::json!({}),
                "a non-recognised-member entry drops to an empty object, not a corrupted one"
            );
        }
    }

    let source_extent = read_field(
        &data_fixture("helsinki_address.city.jsonl"),
        "geographicalExtent",
    );
    let export_extent = read_field(&exported, "geographicalExtent");
    assert!(
        !source_extent.is_empty(),
        "fixture must actually carry geographicalExtent"
    );
    assert_eq!(
        source_extent, export_extent,
        "geographicalExtent is unrelated to the address drop and must round-trip verbatim"
    );
}

/// spec "Addresses" (gap 10): a real address with a `location` `MultiPoint`
/// round-trips — CityJSON `address[].location` (vertex-indexed boundaries) ->
/// the reserved struct's WKB `MultiPointZ` -> back to a CityJSON `MultiPoint`
/// with fresh boundaries into the exported feature's own vertex pool. The
/// recognised postal members round-trip under their canonical names; a
/// non-recognised member (`Locality`, `id`) alongside a recognised one
/// (`locality`) in the SAME entry is dropped without disturbing the
/// recognised sibling (checklist item 2, self-contained).
///
/// `helsinki_address.city.jsonl` carries no `address[].location` at all (see
/// `helsinki_non_postal_address_members_are_dropped_not_corrupted`'s doc
/// comment), so this is a dedicated small fixture: real Helsinki geometry and
/// vertices (borrowed from `helsinki_address.city.jsonl`'s first object) with
/// a hand-added `address` array exercising both the location round-trip and
/// the recognised/non-recognised member split. No public URL for this exact
/// content, so committed in-tree under `tests/data/` (via [`data_fixture`])
/// rather than fetched.
#[test]
fn address_location_round_trips() {
    let (exported, _package_dir, _export_dir) =
        convert_and_export_path(&data_fixture("address_location.city.jsonl"));
    let report = compare_datasets(
        &data_fixture("address_location.city.jsonl"),
        &exported,
        &CompareOptions::default(),
    )
    .unwrap();
    assert!(
        report.equal,
        "address (incl. location) must round-trip losslessly; differences: {:#?}",
        report.differences
    );

    // Direct proof beyond the comparator: read the exported document and
    // check the address content itself. Each JSONL document is (header line
    // carrying `transform`, empty `CityObjects`) + (feature line carrying
    // `CityObjects`/`vertices`, no `transform` of its own) — `header_and_feature`
    // returns both, since a location's boundaries dequantise against the
    // HEADER's transform but live in the FEATURE's own vertices.
    let header_and_feature = |text: &str| -> (serde_json::Value, serde_json::Value) {
        let mut header = None;
        let mut feature = None;
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            let v: serde_json::Value = serde_json::from_str(line).unwrap();
            let has_objects = v
                .get("CityObjects")
                .and_then(|c| c.as_object())
                .is_some_and(|c| !c.is_empty());
            if has_objects {
                feature = Some(v);
            } else if header.is_none() {
                header = Some(v);
            }
        }
        (
            header.expect("a header line"),
            feature.expect("a feature line with the CityObject"),
        )
    };
    let (source_header, source_feature) = header_and_feature(
        &std::fs::read_to_string(data_fixture("address_location.city.jsonl")).unwrap(),
    );
    let (export_header, export_feature) =
        header_and_feature(&std::fs::read_to_string(&exported).unwrap());

    let source_co = source_feature["CityObjects"]
        .as_object()
        .unwrap()
        .values()
        .next()
        .unwrap();
    let export_co = export_feature["CityObjects"]
        .as_object()
        .unwrap()
        .values()
        .next()
        .unwrap();
    let export_addr = export_co["address"].as_array().unwrap();
    assert_eq!(export_addr.len(), 2, "both address entries survive");

    // Entry 0: every recognised postal member round-trips verbatim.
    let e0 = &export_addr[0];
    for key in [
        "country",
        "locality",
        "administrativeArea",
        "thoroughfareName",
        "thoroughfareNumber",
        "postcode",
        "postBox",
        "freeText",
    ] {
        assert_eq!(
            e0.get(key),
            source_co["address"][0].get(key),
            "recognised member '{key}' must round-trip verbatim"
        );
    }

    // `location` reconstructs as a MultiPoint whose boundaries resolve (via
    // the exported feature's OWN transform/vertices) to the same real-world
    // coordinates as the source's vertices 0 and 4.
    let location = e0.get("location").expect("location must be present");
    assert_eq!(location["type"], serde_json::json!("MultiPoint"));
    let boundaries: Vec<usize> = serde_json::from_value(location["boundaries"].clone()).unwrap();
    assert_eq!(boundaries.len(), 2, "the source location had two vertices");

    let dequantise =
        |header: &serde_json::Value, feature: &serde_json::Value, idx: usize| -> [f64; 3] {
            let transform = &header["transform"];
            let scale: Vec<f64> = serde_json::from_value(transform["scale"].clone()).unwrap();
            let translate: Vec<f64> =
                serde_json::from_value(transform["translate"].clone()).unwrap();
            let raw: Vec<f64> = serde_json::from_value(feature["vertices"][idx].clone()).unwrap();
            [
                translate[0] + scale[0] * raw[0],
                translate[1] + scale[1] * raw[1],
                translate[2] + scale[2] * raw[2],
            ]
        };
    let expected = [
        dequantise(&source_header, &source_feature, 0),
        dequantise(&source_header, &source_feature, 4),
    ];
    let actual = [
        dequantise(&export_header, &export_feature, boundaries[0]),
        dequantise(&export_header, &export_feature, boundaries[1]),
    ];
    for (e, a) in expected.iter().zip(actual.iter()) {
        for axis in 0..3 {
            assert!(
                (e[axis] - a[axis]).abs() < 1e-6,
                "location coordinate must round-trip: expected {e:?}, got {a:?}"
            );
        }
    }

    // Entry 1: the recognised `locality` survives; the non-recognised
    // `Locality`/`id` siblings are dropped without corrupting it.
    let e1 = &export_addr[1];
    assert_eq!(e1.get("locality"), Some(&serde_json::json!("Espoo")));
    assert!(
        e1.get("Locality").is_none(),
        "the non-recognised sibling member must not survive"
    );
    assert!(e1.get("id").is_none(), "non-postal `id` must not survive");
    assert_eq!(
        e1.as_object().unwrap().len(),
        1,
        "entry 1 must carry exactly the one recognised member"
    );
}

/// G12/gap 14 (§5.2): a source attribute whose name collides with a reserved
/// column name (here `bbox`) is diverted into the reserved `other_attributes`
/// column — a genuine physical column, not a magic
/// `cityparquet:diverted_attributes` key riding inside `other` (the pre-gap-14
/// mechanism) — and restored on export, rather than aborting the whole
/// conversion. Fixture is real Helsinki objects with one injected `bbox`
/// attribute alongside 27 genuine (non-colliding) attributes.
///
/// Hand-derived from the City of Helsinki open 3D city model (an injected
/// colliding attribute on real objects); no public URL, so committed in-tree
/// under `tests/data/` (via [`data_fixture`]) rather than fetched.
#[test]
fn colliding_attribute_is_diverted_and_round_trips() {
    let (exported, package_dir, _export_dir) =
        convert_and_export_path(&data_fixture("collision_attr.city.jsonl"));
    let report = compare_datasets(
        &data_fixture("collision_attr.city.jsonl"),
        &exported,
        &CompareOptions::default(),
    )
    .unwrap();
    assert!(
        report.equal,
        "a colliding attribute must round-trip via `other_attributes`; differences: {:#?}",
        report.differences
    );

    // The diverted attribute is restored into `attributes` on export (not left
    // as a top-level member), with its value intact; the non-colliding
    // attributes are unaffected.
    let text = std::fs::read_to_string(&exported).unwrap();
    let mut checked = 0;
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let doc: serde_json::Value = serde_json::from_str(line).unwrap();
        let Some(cos) = doc.get("CityObjects").and_then(|v| v.as_object()) else {
            continue;
        };
        for co in cos.values() {
            let attrs = co.get("attributes").and_then(|v| v.as_object()).unwrap();
            assert_eq!(
                attrs.get("bbox"),
                Some(&serde_json::json!("diverted-sentinel")),
                "the diverted `bbox` attribute must be restored with its value"
            );
            assert!(
                attrs.contains_key("measuredHeight"),
                "genuine attributes must still round-trip alongside the diverted one"
            );
            assert!(
                co.get("bbox").is_none(),
                "`bbox` must be an attribute, not promoted to a top-level member"
            );
            checked += 1;
        }
    }
    assert_eq!(checked, 2, "fixture has two objects, both must be checked");

    // Physical proof (not just decoded values): the PACKAGE's own
    // `other_attributes` Parquet column actually carries the diverted value,
    // and its `other` column does NOT — the divert-key transport mechanism
    // this superseded is gone.
    let tables = PackageTables::open(package_dir.path()).unwrap().tables;
    assert_eq!(tables.len(), 1, "both fixture objects are Buildings");
    let file = std::fs::File::open(&tables[0]).unwrap();
    let reader = ParquetRecordBatchReaderBuilder::try_new(file)
        .unwrap()
        .build()
        .unwrap();
    let mut other_attributes_seen = 0;
    for batch in reader {
        let batch = batch.unwrap();
        let other_attributes = batch
            .column_by_name("other_attributes")
            .expect("other_attributes must be a real physical column")
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        let other = batch
            .column_by_name("other")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        for row in 0..batch.num_rows() {
            assert!(
                !other_attributes.is_null(row),
                "the diverted attribute must land in the physical other_attributes cell"
            );
            let cell: serde_json::Value =
                serde_json::from_str(other_attributes.value(row)).unwrap();
            assert_eq!(cell, serde_json::json!({"bbox": "diverted-sentinel"}));
            if !other.is_null(row) {
                let other_cell: serde_json::Value = serde_json::from_str(other.value(row)).unwrap();
                assert!(
                    other_cell.get("bbox").is_none(),
                    "the diverted attribute must not ALSO ride inside other"
                );
            }
            other_attributes_seen += 1;
        }
    }
    assert_eq!(other_attributes_seen, 2);
}

/// Spec §metadata "CRS rules", as amended: the whole unknown-CRS path, on a
/// real fixture that genuinely carries no `referenceSystem` — convert ->
/// footer `city.crs: null` -> export -> **no** `referenceSystem` -> compare
/// clean against the source.
///
/// `lod3_railway.city.json` is used **unmodified** here (every other railway
/// test injects a CRS via [`railway_fixture_with_crs`]): the missing CRS is
/// the property under test, so the fixture is the evidence rather than
/// something to work around. The round trip is what proves the null state is
/// *faithful* and not merely tolerated — a writer that guessed CRS84, or an
/// exporter that materialised the absent-means-CRS84 reading, would show up
/// here as a `header: referenceSystem differs` difference.
#[test]
fn a_crs_less_source_round_trips_through_an_explicit_null_crs() {
    let source = fixture("lod3_railway.city.json");
    assert!(
        serde_json::from_str::<serde_json::Value>(&std::fs::read_to_string(&source).unwrap())
            .unwrap()["metadata"]
            .get("referenceSystem")
            .is_none(),
        "this test's premise: the real fixture declares no referenceSystem"
    );

    let package_dir = tempfile::tempdir().unwrap();
    let report = convert(&ConvertOptions::new(
        source.clone(),
        package_dir.path().to_path_buf(),
    ))
    .expect("a CRS-less source is declared, not fatal (spec CRS rules)");
    assert!(
        report.crs_diagnostic.is_some(),
        "the writer SHOULD surface a conversion diagnostic"
    );

    // Every object table's footer says `null` — explicitly, not by omission.
    let tables = PackageTables::open(package_dir.path()).unwrap();
    for table in &tables.tables {
        let builder =
            ParquetRecordBatchReaderBuilder::try_new(std::fs::File::open(table).unwrap()).unwrap();
        let city = builder
            .metadata()
            .file_metadata()
            .key_value_metadata()
            .unwrap()
            .iter()
            .find(|kv| kv.key == "city")
            .and_then(|kv| kv.value.clone())
            .expect("every table carries a city footer key");
        let city: serde_json::Value = serde_json::from_str(&city).unwrap();
        let members = city.as_object().unwrap();
        assert!(
            members.contains_key("crs") && members["crs"].is_null(),
            "{}: city.crs must be an explicit null: {members:?}",
            table.display()
        );
    }

    let export_dir = tempfile::tempdir().unwrap();
    let exported = export_dir.path().join("export.city.jsonl");
    export(&ExportOptions {
        package_dir: package_dir.path().to_path_buf(),
        output: exported.clone(),
    })
    .unwrap();

    let header: serde_json::Value = serde_json::from_str(
        std::fs::read_to_string(&exported)
            .unwrap()
            .lines()
            .next()
            .unwrap(),
    )
    .unwrap();
    assert!(
        header
            .get("metadata")
            .and_then(|m| m.get("referenceSystem"))
            .is_none(),
        "an unknown CRS exports NO referenceSystem — the model carries no \
         georeference, matching the source: {header}"
    );

    let compared = compare_datasets(&source, &exported, &CompareOptions::default()).unwrap();
    assert!(
        compared.equal,
        "the unknown-CRS round trip must still be semantically equal; differences: {:#?}",
        compared.differences
    );
}
