//! M3 task 7 (the milestone claim): full pipeline round-trip proof —
//! convert the real fixture into a package, export it back to CityJSON/Seq,
//! and prove the two are semantically equal via `compare::compare_datasets`.

use std::path::PathBuf;

use cityparquet::compare::{CompareOptions, Exclusions, compare_datasets};
use cityparquet::export::{ExportOptions, export};
use cityparquet::package::{ConvertOptions, RowOrder, TableLayout, convert};
use cityparquet::recipe::RecipePreset;
use cityparquet::schema::Profile;

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

/// Converts `input` into a fresh tempdir package, exports it back to
/// `.city.jsonl`, and returns the export's path alongside the tempdirs that
/// back both (kept alive so the caller can still read the file).
fn convert_and_export(input: &str) -> (PathBuf, tempfile::TempDir, tempfile::TempDir) {
    let package_dir = tempfile::tempdir().unwrap();
    convert(&ConvertOptions::new(
        fixture(input),
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

/// Same as [`convert_and_export`] but lets the caller pick the profile —
/// needed for the Compatibility-profile headline round-trip gate below,
/// where appearance/instances must actually be restored, not dropped.
fn convert_and_export_with_profile(
    input: &str,
    profile: Profile,
) -> (PathBuf, tempfile::TempDir, tempfile::TempDir) {
    let package_dir = tempfile::tempdir().unwrap();
    let mut opts = ConvertOptions::new(fixture(input), package_dir.path().to_path_buf());
    opts.profile = profile;
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
    let (exported, _package_dir, _export_dir) = convert_and_export("lod3_railway.city.json");
    let opts = CompareOptions {
        coord_tolerance: [0.0; 3],
        exclusions: Exclusions {
            appearance: true,
            geometry_instances: true,
        },
    };
    let report = compare_datasets(&fixture("lod3_railway.city.json"), &exported, &opts).unwrap();
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
    // GeometryInstance (instance_geometries_dropped). The degenerate count
    // was updated from the writer-only-index-drop figure of 3 to 23
    // alongside the comparator's coordinate-degenerate ring fix (3DBAG tile
    // `9-284-556.city.json` finding; see `crate::compare`'s module docs):
    // railway's real, unmutated source carries 20 MORE objects whose
    // boundaries include an index-distinct/coordinate-identical ring,
    // previously invisible to the INDEX-only degenerate check (the writer's
    // 3 pinned drops in `wkb_roundtrip_real_data.rs::geometries_with_drops`
    // are unaffected — that pin is about the WRITER's own index-based
    // normalisation, which this fix does not touch).
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
        (105, 15, 23),
        "exclusion breakdown must match the pinned pipeline counts, got: {:#?}",
        non_header_excluded
    );
    assert_eq!(
        non_header_excluded.len(),
        143,
        "105 appearance + 15 instances + 23 degenerate = 143 total non-header exclusions, \
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

/// M4 task 11 (the milestone's headline gate): a Compatibility-profile
/// round trip with NO exclusions at all — appearance and GeometryInstance
/// geometries are now restored from the sidecars (M4 tasks 6-10), so unlike
/// `railway_round_trips_losslessly_modulo_documented_drops` above (which
/// still excludes both, because that test converts Core), the only
/// remaining exclusions are the 23 pinned degenerate-ring drops (updated
/// alongside the comparator's coordinate-degenerate fix — see the comment in
/// `railway_round_trips_losslessly_modulo_documented_drops` above) and
/// whatever header metadata members railway's header sets (documented,
/// unbounded). Any OTHER exclusion here would mean appearance or an
/// instance silently failed to round-trip.
#[test]
fn railway_compatibility_round_trips_losslessly_with_no_exclusions() {
    let (exported, _package_dir, _export_dir) =
        convert_and_export_with_profile("lod3_railway.city.json", Profile::Compatibility);
    let report = compare_datasets(
        &fixture("lod3_railway.city.json"),
        &exported,
        &CompareOptions::default(),
    )
    .unwrap();
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
/// — the Compatibility profile writing (empty) sidecars must not introduce
/// any new difference or exclusion relative to the Core round trip already
/// proven by `delft_round_trips_losslessly`.
#[test]
fn delft_compatibility_round_trips_losslessly_with_only_header_exclusions() {
    let (exported, _package_dir, _export_dir) =
        convert_and_export_with_profile("delft.city.jsonl", Profile::Compatibility);
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

/// Like [`convert_and_export_with_profile`] but also lets the caller pick
/// `ordering` and `layout` — needed for the M5 task 5 `TableLayout::ByType`
/// round-trip gates below (and their Hilbert-composed variant).
fn convert_and_export_with(
    input: &str,
    profile: Profile,
    ordering: RowOrder,
    layout: TableLayout,
) -> (PathBuf, tempfile::TempDir, tempfile::TempDir) {
    let package_dir = tempfile::tempdir().unwrap();
    let mut opts = ConvertOptions::new(fixture(input), package_dir.path().to_path_buf());
    opts.profile = profile;
    opts.ordering = ordering;
    opts.layout = layout;
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

/// M5 task 5 (Step 3): delft under `TableLayout::ByType` (Core profile) must
/// round-trip exactly as losslessly as `TableLayout::Single` does in
/// `delft_round_trips_losslessly` above — the table layout is purely a
/// physical-file concern, never a semantic one. Per the family-grouping rule,
/// delft's Building + BuildingPart share the single `building.parquet` file
/// (see `by_type_convert_of_delft_writes_exactly_one_family_table` in
/// `convert_real_data.rs`), so `export` must still read the whole dataset
/// back from that one table.
#[test]
fn delft_by_type_round_trips_losslessly() {
    let (exported, package_dir, _export_dir) = convert_and_export_with(
        "delft.city.jsonl",
        Profile::Core,
        RowOrder::Source,
        TableLayout::ByType,
    );
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

/// M5 task 5 (Step 3): railway under `TableLayout::ByType` (Compatibility
/// profile) — the M4 headline round-trip gate
/// (`railway_compatibility_round_trips_losslessly_with_no_exclusions` above)
/// held with `TableLayout::Single`; it must hold identically under
/// `TableLayout::ByType`, across all 10 pinned family tables (railway's 14
/// distinct `object_type` values collapse to 10 distinct 1st-level families —
/// see `by_type_convert_of_railway_writes_ten_family_tables` in
/// `convert_real_data.rs` for the exact family membership).
#[test]
fn railway_by_type_compatibility_round_trips_losslessly_with_no_exclusions() {
    let (exported, package_dir, _export_dir) = convert_and_export_with(
        "lod3_railway.city.json",
        Profile::Compatibility,
        RowOrder::Source,
        TableLayout::ByType,
    );
    let manifest_text = std::fs::read_to_string(package_dir.path().join("metadata.json")).unwrap();
    let manifest: serde_json::Value = serde_json::from_str(&manifest_text).unwrap();
    let tables = manifest["tables"].as_array().unwrap();
    assert_eq!(
        tables.len(),
        10,
        "railway's pinned type set collapses to 10 distinct 1st-level families, got: {tables:?}"
    );

    let report = compare_datasets(
        &fixture("lod3_railway.city.json"),
        &exported,
        &CompareOptions::default(),
    )
    .unwrap();
    assert!(
        report.equal,
        "ByType-layout railway must round-trip losslessly with NO exclusions under the \
         Compatibility profile, exactly like TableLayout::Single; differences: {:#?}",
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

/// M5 task 5 (Step 3, the composed smoke gate): `RowOrder::Hilbert` and
/// `TableLayout::ByType` compose independently (Hilbert reorders FEATURES
/// before encode; ByType partitions ENCODED ROWS after — see
/// `TableLayout`'s doc comment) — one assertion proving delft still
/// round-trips losslessly with BOTH turned on at once.
#[test]
fn delft_hilbert_and_by_type_compose_and_round_trip_losslessly() {
    let (exported, package_dir, _export_dir) = convert_and_export_with(
        "delft.city.jsonl",
        Profile::Core,
        RowOrder::Hilbert,
        TableLayout::ByType,
    );
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
