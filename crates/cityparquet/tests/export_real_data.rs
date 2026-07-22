//! RED (M3 task 6): export — package -> CityJSON/CityJSONSeq, exercised
//! against real converted delft/railway packages.

use std::path::PathBuf;
use std::sync::Arc;

use arrow_array::RecordBatch;
use arrow_schema::{Field, Schema};
use city3d_stac_types::stac::types::{Asset, Item};
use cityparquet::CityParquetError;
use cityparquet::compare::{CompareOptions, compare_datasets};
use cityparquet::export::{ExportOptions, export};
use cityparquet::package::{ConvertOptions, convert};
use cityparquet::reader::CityParquetReaderBuilder;
use cityparquet::sidecar::{read_materials, read_templates, write_materials, write_templates};
use cityparquet::source::{Source, SourceFormat};
use cityparquet::stac::assets::{PARQUET_MEDIA_TYPE, ROLE_OBJECT_TABLE, ROLE_SIDECAR};
use parquet::arrow::ArrowWriter;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::file::metadata::KeyValue;
use parquet::file::properties::WriterProperties;
use serde_json::Value;

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
/// metadata (`city.other.source_metadata` — spec-alignment M3 folded
/// `source_metadata` into `other`, informational only) and restored into the
/// exported header. `fullMetadataUrl` is a documented exception — cjseq's
/// `Metadata` struct has no passthrough for unknown members, so it never
/// survives even the initial parse of the source header, let alone the round
/// trip.
#[test]
fn delft_source_metadata_reaches_kv_metadata_and_the_exported_header() {
    let package_dir = tempfile::tempdir().unwrap();
    convert(&ConvertOptions::new(
        fixture("delft.city.jsonl"),
        package_dir.path().to_path_buf(),
    ))
    .unwrap();

    // delft is a single 1st-level family, so by-type conversion writes
    // exactly one main table: building.parquet.
    let file = std::fs::File::open(package_dir.path().join("building.parquet")).unwrap();
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
    let meta = builder.cityparquet_metadata().unwrap();
    let other = meta
        .other
        .as_ref()
        .expect("delft's header sets metadata; `other` must be populated");
    let source_metadata = other
        .get("source_metadata")
        .expect("delft's header metadata must be preserved under other.source_metadata");
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

/// Recursively asserts every integer in a *localised* material tree (the
/// `values`/`value` payload attached to a restored `cjseq::Material`) is a
/// feature-local index `< limit`; `null` (no material) is skipped.
fn assert_material_indices_below(v: &Value, limit: usize) {
    match v {
        Value::Null => {}
        Value::Number(n) => {
            let idx = n
                .as_u64()
                .unwrap_or_else(|| panic!("expected a non-negative material index, got {n}"))
                as usize;
            assert!(
                idx < limit,
                "material index {idx} must be < feature-local materials len {limit}"
            );
        }
        Value::Array(items) => {
            for x in items {
                assert_material_indices_below(x, limit);
            }
        }
        other => panic!("unexpected node in a localised material tree: {other}"),
    }
}

/// Recursively asserts every innermost ring in a *localised* texture tree is
/// back to plain INDEX form `[t, uv_idx0, uv_idx1, ...]` (or `[null]`): `t` (a
/// feature-local texture index) `< tex_limit`, and every following element is
/// a bare integer `< uv_limit` — NOT an inlined `[u, v]` pair, which would
/// mean the encoder's UV-inlining rewrite was never undone.
fn assert_texture_rings_are_index_form(v: &Value, tex_limit: usize, uv_limit: usize) {
    match v {
        Value::Array(items) => {
            let is_ring = !items.is_empty() && matches!(items[0], Value::Number(_) | Value::Null);
            if is_ring {
                if let Value::Number(n) = &items[0] {
                    let t = n
                        .as_u64()
                        .unwrap_or_else(|| panic!("expected a non-negative texture index, got {n}"))
                        as usize;
                    assert!(t < tex_limit, "texture id {t} must be < {tex_limit}");
                }
                for uv in &items[1..] {
                    assert!(
                        !uv.is_array(),
                        "texture ring must be back to index form, found an inlined [u, v] pair: {uv}"
                    );
                    let idx = uv
                        .as_u64()
                        .unwrap_or_else(|| panic!("expected a UV index, got {uv}"))
                        as usize;
                    assert!(idx < uv_limit, "UV index {idx} must be < {uv_limit}");
                }
            } else {
                for x in items {
                    assert_texture_rings_are_index_form(x, tex_limit, uv_limit);
                }
            }
        }
        other => panic!("unexpected node in a localised texture tree: {other}"),
    }
}

/// M4 task 9: on a Compatibility-profile package (sidecars present), export
/// restores `material`/`texture` from the sidecars instead of dropping them —
/// each feature gets its own self-contained, feature-local `appearance`
/// block (the inverse of the encoder's global interning + UV inlining).
#[test]
fn railway_compatibility_export_restores_appearance_feature_local() {
    let package_dir = tempfile::tempdir().unwrap();
    let opts = ConvertOptions::new(
        fixture("lod3_railway.city.json"),
        package_dir.path().to_path_buf(),
    );
    convert(&opts).unwrap();

    let export_dir = tempfile::tempdir().unwrap();
    let output = export_dir.path().join("export.city.jsonl");
    let report = export(&ExportOptions {
        package_dir: package_dir.path().to_path_buf(),
        output: output.clone(),
    })
    .unwrap();

    // (a)
    assert_eq!(
        report.appearance_refs_dropped, 0,
        "compatibility-profile export must restore appearance, not drop it"
    );

    let exported = Source::open(&output).unwrap();
    let mut found_material = false;
    let mut found_texture = false;
    for feature in exported.features().unwrap() {
        let feature = feature.unwrap();
        let Some(appearance) = &feature.appearance else {
            continue;
        };
        let n_materials = appearance.materials.as_ref().map_or(0, Vec::len);
        let n_textures = appearance.textures.as_ref().map_or(0, Vec::len);
        let n_uvs = appearance.vertices_texture.as_ref().map_or(0, Vec::len);

        for co in feature.city_objects.values() {
            let Some(geoms) = &co.geometry else { continue };
            for g in geoms {
                // (b)
                if let Some(material) = &g.material {
                    assert!(
                        !material.is_empty(),
                        "a geometry carrying a material member must not be empty"
                    );
                    for m in material.values() {
                        if let Some(idx) = m.value {
                            assert!(
                                idx < n_materials,
                                "material index {idx} must be < feature-local materials len {n_materials}"
                            );
                            found_material = true;
                        }
                        if let Some(values) = &m.values {
                            assert_material_indices_below(values, n_materials);
                            found_material = true;
                        }
                    }
                }
                // (c)
                if let Some(texture) = &g.texture {
                    for t in texture.values() {
                        if let Some(values) = &t.values {
                            assert_texture_rings_are_index_form(values, n_textures, n_uvs);
                            found_texture = true;
                        }
                    }
                }
            }
        }
    }
    assert!(
        found_material,
        "expected at least one restored material reference across the exported features"
    );
    assert!(
        found_texture,
        "expected at least one restored texture reference across the exported features"
    );
}

/// G20 regression: a non-canonical source `lod` string must NOT lose its
/// appearance on round-trip. Under the old single-column layout the encoder
/// keyed the per-object appearance map by the raw `lod` (`"03"`) while export
/// looked it up by the canonical `"3"`, so the texture was silently dropped
/// (the old `appearance_lod_misses` counter existed only to notice this).
/// With per-LoD `texture_lod*` columns the appearance is paired to its
/// geometry by the shared `lod3` column suffix, so `"03"` restores exactly.
/// Derived from the railway fixture: object
/// `UUID_bd865e62-18de-40ff-85da-883709a86f0f`'s only (non-instance)
/// geometry — a `lod: "3"` `MultiSurface` carrying a `texture` block — has
/// its `lod` rewritten to `"03"` (same canonical LoD 3, so still the
/// `geometry_lod3` / `texture_lod3` columns). The derived document must
/// round-trip losslessly.
#[test]
fn railway_export_restores_appearance_under_a_non_canonical_lod() {
    const TARGET_OBJECT_ID: &str = "UUID_bd865e62-18de-40ff-85da-883709a86f0f";

    let mut doc: Value =
        serde_json::from_str(&std::fs::read_to_string(fixture("lod3_railway.city.json")).unwrap())
            .unwrap();
    let mut rewritten = false;
    {
        let co = doc["CityObjects"][TARGET_OBJECT_ID]
            .as_object_mut()
            .expect("precondition: target object must exist");
        let geoms = co
            .get_mut("geometry")
            .and_then(Value::as_array_mut)
            .expect("precondition: target object must carry geometry");
        for g in geoms.iter_mut() {
            let obj = g.as_object_mut().unwrap();
            if obj.get("type").and_then(Value::as_str) == Some("GeometryInstance") {
                continue;
            }
            assert_eq!(
                obj.get("lod").and_then(Value::as_str),
                Some("3"),
                "precondition: target geometry's source lod must be the canonical \"3\""
            );
            assert!(
                obj.contains_key("texture"),
                "precondition: target geometry must carry a texture block"
            );
            obj.insert("lod".to_string(), serde_json::json!("03"));
            rewritten = true;
        }
    }
    assert!(
        rewritten,
        "must have rewritten exactly the target geometry's lod"
    );

    let input_dir = tempfile::tempdir().unwrap();
    let input_path = input_dir.path().join("railway_noncanonical_lod.city.json");
    std::fs::write(&input_path, serde_json::to_string(&doc).unwrap()).unwrap();

    // Baseline: pristine railway (canonical "3") exports with its texture.
    let pristine_pkg = tempfile::tempdir().unwrap();
    let pristine_opts = ConvertOptions::new(
        fixture("lod3_railway.city.json"),
        pristine_pkg.path().to_path_buf(),
    );
    convert(&pristine_opts).unwrap();
    let pristine_export = tempfile::tempdir().unwrap();
    let pristine_out = pristine_export.path().join("export.city.jsonl");
    export(&ExportOptions {
        package_dir: pristine_pkg.path().to_path_buf(),
        output: pristine_out.clone(),
    })
    .unwrap();
    let (_, pristine_textures) = count_geometry_appearance_keys(&pristine_out);

    let package_dir = tempfile::tempdir().unwrap();
    let opts = ConvertOptions::new(input_path, package_dir.path().to_path_buf());
    convert(&opts).unwrap();

    let export_dir = tempfile::tempdir().unwrap();
    let export_path = export_dir.path().join("export.city.jsonl");
    export(&ExportOptions {
        package_dir: package_dir.path().to_path_buf(),
        output: export_path.clone(),
    })
    .unwrap();

    // Global count: the derived export carries the SAME number of texture
    // blocks as pristine railway. Under the old single-column layout this
    // geometry's texture was silently dropped (raw "03" key vs canonical "3"
    // lookup), so the count would be one lower.
    let (_, derived_textures) = count_geometry_appearance_keys(&export_path);
    assert_eq!(
        derived_textures, pristine_textures,
        "the non-canonical-lod geometry's texture must survive export, not be dropped"
    );
    assert!(
        derived_textures > 0,
        "precondition: railway export must carry at least one texture block"
    );

    // Targeted: the specific rewritten object's own LoD-3 geometry must carry a
    // texture in the export (a global count alone could mask a lost target
    // texture offset by a duplicate elsewhere).
    let target_texture = |path: &std::path::Path| -> Value {
        let text = std::fs::read_to_string(path).unwrap();
        for line in text.lines().skip(1) {
            let feature: Value = serde_json::from_str(line).unwrap();
            let Some(co) = feature["CityObjects"].get(TARGET_OBJECT_ID) else {
                continue;
            };
            for geom in co["geometry"].as_array().unwrap() {
                if geom.get("type").and_then(Value::as_str) == Some("GeometryInstance") {
                    continue;
                }
                return geom.get("texture").cloned().unwrap_or(Value::Null);
            }
        }
        Value::Null
    };
    let derived_target = target_texture(&export_path);
    assert!(
        derived_target.is_object(),
        "the rewritten object's own LoD-3 geometry must keep its texture, got: {derived_target:?}"
    );
    // And it must be the SAME texture the pristine object exports.
    assert_eq!(
        derived_target,
        target_texture(&pristine_out),
        "the rewritten object's texture must match pristine railway's"
    );
}

/// An object with geometries at SEVERAL lods, where appearance is defined for
/// only SOME of them, is legitimate CityJSON and must round-trip exactly: the
/// LoD that has a texture keeps it, the LoD that has none stays bare. With
/// per-LoD appearance columns (G20) this is structural — `texture_lod3` is
/// non-null, `texture_lod2` is null — so no cross-LoD confusion is possible.
/// Derived from the railway fixture: object
/// `UUID_bd865e62-18de-40ff-85da-883709a86f0f`'s only (non-instance)
/// geometry (`lod: "3"`, carrying a `texture` block) gains a SECOND geometry
/// at lod `"2"` (same boundaries) that carries no material/texture.
#[test]
fn multi_lod_object_with_single_lod_appearance_round_trips() {
    const TARGET_OBJECT_ID: &str = "UUID_bd865e62-18de-40ff-85da-883709a86f0f";

    let mut doc: Value =
        serde_json::from_str(&std::fs::read_to_string(fixture("lod3_railway.city.json")).unwrap())
            .unwrap();
    {
        let co = doc["CityObjects"][TARGET_OBJECT_ID]
            .as_object_mut()
            .expect("precondition: target object must exist");
        let geoms = co
            .get_mut("geometry")
            .and_then(Value::as_array_mut)
            .expect("precondition: target object must carry geometry");
        assert_eq!(
            geoms.len(),
            1,
            "precondition: target object must carry exactly one geometry"
        );
        let original = geoms[0].as_object().unwrap();
        assert_eq!(
            original.get("lod").and_then(Value::as_str),
            Some("3"),
            "precondition: target geometry's source lod must be the canonical \"3\""
        );
        assert!(
            original.contains_key("texture"),
            "precondition: target geometry must carry a texture block"
        );
        let mut second_lod_geom = original.clone();
        second_lod_geom.insert("lod".to_string(), serde_json::json!("2"));
        second_lod_geom.remove("material");
        second_lod_geom.remove("texture");
        geoms.push(Value::Object(second_lod_geom));
    }

    let input_dir = tempfile::tempdir().unwrap();
    let input_path = input_dir
        .path()
        .join("railway_multi_lod_single_appearance.city.json");
    std::fs::write(&input_path, serde_json::to_string(&doc).unwrap()).unwrap();

    let package_dir = tempfile::tempdir().unwrap();
    let opts = ConvertOptions::new(input_path.clone(), package_dir.path().to_path_buf());
    convert(&opts).unwrap();

    let export_dir = tempfile::tempdir().unwrap();
    let export_path = export_dir.path().join("export.city.jsonl");
    export(&ExportOptions {
        package_dir: package_dir.path().to_path_buf(),
        output: export_path.clone(),
    })
    .unwrap();

    let report = compare_datasets(&input_path, &export_path, &CompareOptions::default()).unwrap();
    assert!(
        report.equal,
        "a multi-lod object with single-lod appearance must round-trip; differences: {:#?}",
        report.differences
    );
}

/// G20 sol-review Finding 1: with per-LoD appearance columns the bare
/// `material` / `texture` names are no longer reserved, so a source attribute
/// may legally be named `material` (a plain `VARCHAR`) or `material_lod03`
/// (canonicalises to LoD 3, colliding with the real `material_lod3`). Export
/// must classify appearance columns by their reserved-role metadata, not by
/// name, so those attributes are never read as appearance — otherwise export
/// would try to parse the `VARCHAR` `"brick"` as JSON (a hard error) or let
/// `material_lod03` overwrite the genuine `material_lod3` restore.
/// Derived from the railway fixture by injecting the two lookalike attributes.
#[test]
fn attributes_named_like_appearance_columns_do_not_corrupt_export() {
    const TARGET_OBJECT_ID: &str = "UUID_bd865e62-18de-40ff-85da-883709a86f0f";

    let mut doc: Value =
        serde_json::from_str(&std::fs::read_to_string(fixture("lod3_railway.city.json")).unwrap())
            .unwrap();
    {
        let co = doc["CityObjects"][TARGET_OBJECT_ID]
            .as_object_mut()
            .expect("precondition: target object must exist");
        let attrs = co
            .entry("attributes")
            .or_insert_with(|| Value::Object(serde_json::Map::new()))
            .as_object_mut()
            .unwrap();
        // A plain string (invalid JSON) named exactly like the bare appearance
        // prefix, and a non-canonical-suffix lookalike of material_lod3.
        attrs.insert("material".to_string(), serde_json::json!("brick"));
        attrs.insert(
            "material_lod03".to_string(),
            serde_json::json!("also brick"),
        );
    }

    let input_dir = tempfile::tempdir().unwrap();
    let input_path = input_dir.path().join("railway_lookalike_attrs.city.json");
    std::fs::write(&input_path, serde_json::to_string(&doc).unwrap()).unwrap();

    let package_dir = tempfile::tempdir().unwrap();
    let opts = ConvertOptions::new(input_path.clone(), package_dir.path().to_path_buf());
    convert(&opts).unwrap();

    let export_dir = tempfile::tempdir().unwrap();
    let export_path = export_dir.path().join("export.city.jsonl");
    // Must not error: the lookalike attributes must be skipped by the
    // reserved-role filter, never parsed as appearance JSON.
    export(&ExportOptions {
        package_dir: package_dir.path().to_path_buf(),
        output: export_path.clone(),
    })
    .expect("export must not misread lookalike attributes as appearance");

    // The target object's real LoD-3 texture must still round-trip, and the
    // lookalike string attributes must survive as attributes.
    let text = std::fs::read_to_string(&export_path).unwrap();
    let mut checked = false;
    for line in text.lines().skip(1) {
        let feature: Value = serde_json::from_str(line).unwrap();
        let Some(co) = feature["CityObjects"].get(TARGET_OBJECT_ID) else {
            continue;
        };
        assert_eq!(
            co["attributes"]["material"].as_str(),
            Some("brick"),
            "the `material` string attribute must round-trip as an attribute"
        );
        let has_texture = co["geometry"]
            .as_array()
            .unwrap()
            .iter()
            .any(|g| g.get("texture").is_some());
        assert!(has_texture, "the real LoD-3 texture must survive");
        checked = true;
    }
    assert!(checked, "target object must be present in the export");
}

/// RED (G5): a CityObjectGroup's `children_roles` (CityJSON 2.0.1 §2.5, one role per
/// child) must round-trip. It lives in cjseq's private flatten, so the
/// encoder previously left the column null and the exporter dropped it.
/// Derived from railway: give a parent object one role per child, then
/// convert → export and require the roles back verbatim on that object.
#[test]
fn children_roles_round_trip() {
    const TARGET_OBJECT_ID: &str = "UUID_f488e8ce-b953-4b35-a3fe-a394fb203868";

    let mut doc: Value =
        serde_json::from_str(&std::fs::read_to_string(fixture("lod3_railway.city.json")).unwrap())
            .unwrap();
    let roles: Vec<String> = {
        let co = doc["CityObjects"][TARGET_OBJECT_ID]
            .as_object_mut()
            .unwrap();
        let n = co["children"].as_array().unwrap().len();
        assert!(n > 0, "precondition: target object must have children");
        let roles: Vec<String> = (0..n).map(|i| format!("role{i}")).collect();
        co.insert("children_roles".to_string(), serde_json::json!(roles));
        roles
    };

    let input_dir = tempfile::tempdir().unwrap();
    let input_path = input_dir.path().join("railway_children_roles.city.json");
    std::fs::write(&input_path, serde_json::to_string(&doc).unwrap()).unwrap();

    let package_dir = tempfile::tempdir().unwrap();
    convert(&ConvertOptions::new(
        input_path,
        package_dir.path().to_path_buf(),
    ))
    .unwrap();

    let export_dir = tempfile::tempdir().unwrap();
    let export_path = export_dir.path().join("export.city.jsonl");
    export(&ExportOptions {
        package_dir: package_dir.path().to_path_buf(),
        output: export_path.clone(),
    })
    .unwrap();

    let text = std::fs::read_to_string(&export_path).unwrap();
    let mut checked = false;
    for line in text.lines().skip(1) {
        let feature: Value = serde_json::from_str(line).unwrap();
        let Some(co) = feature["CityObjects"].get(TARGET_OBJECT_ID) else {
            continue;
        };
        assert_eq!(
            co["children_roles"],
            serde_json::json!(roles),
            "children_roles must round-trip verbatim on {TARGET_OBJECT_ID}"
        );
        // The children list must survive too, and in the same order the roles
        // are aligned to (a role is meaningless without its child).
        assert_eq!(
            co["children"].as_array().map(Vec::len),
            Some(roles.len()),
            "the children list must round-trip with one entry per role"
        );
        checked = true;
    }
    assert!(checked, "target object must be present in the export");
}

/// G5 sol-review Finding 1: a malformed `children_roles` (here, more roles
/// than children) is invalid CityJSON 2.0.1 (§2.5 requires one role per child)
/// and must be REJECTED at convert, not silently truncated or coerced.
/// Derived from railway's CityObjectGroup.
#[test]
fn mismatched_children_roles_is_rejected() {
    const TARGET_OBJECT_ID: &str = "UUID_f488e8ce-b953-4b35-a3fe-a394fb203868";

    let mut doc: Value =
        serde_json::from_str(&std::fs::read_to_string(fixture("lod3_railway.city.json")).unwrap())
            .unwrap();
    {
        let co = doc["CityObjects"][TARGET_OBJECT_ID]
            .as_object_mut()
            .unwrap();
        let n = co["children"].as_array().unwrap().len();
        // One MORE role than children — invalid.
        let roles: Vec<String> = (0..n + 1).map(|i| format!("role{i}")).collect();
        co.insert("children_roles".to_string(), serde_json::json!(roles));
    }
    let input_dir = tempfile::tempdir().unwrap();
    let input_path = input_dir.path().join("railway_bad_roles.city.json");
    std::fs::write(&input_path, serde_json::to_string(&doc).unwrap()).unwrap();

    let package_dir = tempfile::tempdir().unwrap();
    let err = convert(&ConvertOptions::new(
        input_path,
        package_dir.path().to_path_buf(),
    ))
    .expect_err("a children_roles length mismatch must be rejected");
    let msg = err.to_string();
    assert!(
        msg.contains(TARGET_OBJECT_ID) && msg.contains("children_roles"),
        "error must name the object and cite children_roles, got: {msg}"
    );
}

/// M5 debt item 2, rule half (a): `LocalAppearance::into_appearance` must
/// return `Some` only when the feature referenced a definition OR
/// dataset-wide defaults exist — never unconditionally for every feature.
/// Railway's header sets no default-theme members at all (recounted: neither
/// shipped fixture does — see `scan_real_data.rs`'s
/// `source_metadata_and_appearance_defaults_reach_scan_metadata`), so under
/// the Compatibility profile a feature that references no material/texture
/// definition of its own must export with NO `appearance` block at all, on
/// EVERY geometry. `GMLID_SO092422_3593_9527` is a real, top-level (no
/// `parents`), childless railway object whose only geometry is a
/// `GeometryInstance` — GeometryInstance geometries never carry
/// `material`/`texture` themselves — so it becomes its own single-object
/// feature that references nothing.
#[test]
fn railway_compatibility_export_attaches_appearance_only_to_referencing_features() {
    const NON_APPEARANCE_OBJECT_ID: &str = "GMLID_SO092422_3593_9527";

    // Precondition, checked straight against the source fixture: the object
    // is top-level, childless, and its only geometry carries neither
    // material nor texture.
    let source = Source::open(&fixture("lod3_railway.city.json")).unwrap();
    let mut confirmed_precondition = false;
    for feature in source.features().unwrap() {
        let feature = feature.unwrap();
        let Some(co) = feature.city_objects.get(NON_APPEARANCE_OBJECT_ID) else {
            continue;
        };
        assert!(
            co.parents.is_none(),
            "precondition: {NON_APPEARANCE_OBJECT_ID} must be a top-level object"
        );
        assert!(
            co.children.is_none(),
            "precondition: {NON_APPEARANCE_OBJECT_ID} must be childless"
        );
        let geoms = co
            .geometry
            .as_ref()
            .expect("precondition: object must carry geometry");
        assert!(
            geoms
                .iter()
                .all(|g| g.material.is_none() && g.texture.is_none()),
            "precondition: {NON_APPEARANCE_OBJECT_ID} must reference no material/texture"
        );
        confirmed_precondition = true;
    }
    assert!(
        confirmed_precondition,
        "the source fixture must contain {NON_APPEARANCE_OBJECT_ID}"
    );

    let package_dir = tempfile::tempdir().unwrap();
    let opts = ConvertOptions::new(
        fixture("lod3_railway.city.json"),
        package_dir.path().to_path_buf(),
    );
    convert(&opts).unwrap();

    let export_dir = tempfile::tempdir().unwrap();
    let output = export_dir.path().join("export.city.jsonl");
    export(&ExportOptions {
        package_dir: package_dir.path().to_path_buf(),
        output: output.clone(),
    })
    .unwrap();

    let exported = Source::open(&output).unwrap();
    let mut checked = false;
    for feature in exported.features().unwrap() {
        let feature = feature.unwrap();
        let Some(co) = feature.city_objects.get(NON_APPEARANCE_OBJECT_ID) else {
            continue;
        };
        assert!(
            feature.appearance.is_none(),
            "a feature referencing no material/texture definition must carry no appearance \
             block at all (railway's header sets no dataset-wide defaults), got: {:?}",
            feature.appearance
        );
        for g in co.geometry.as_ref().unwrap() {
            assert!(g.material.is_none() && g.texture.is_none());
        }
        checked = true;
    }
    assert!(
        checked,
        "the exported dataset must still contain {NON_APPEARANCE_OBJECT_ID}"
    );
}

/// M5 debt item 2, rule half (b): a dataset that declares dataset-wide
/// `default-theme-material`/`default-theme-texture` members but writes NO
/// `materials.parquet`/`textures.parquet` sidecars at all (any Core-profile
/// convert, regardless of what the header sets — sidecars are a
/// Compatibility-profile artefact) must still see those defaults attached to
/// every exported feature; before this fix `export` only ever constructed a
/// `LocalAppearance` (and therefore only ever attached defaults) when the
/// sidecars were present, so the defaults were lost entirely. Derived from a
/// mutated copy of the railway fixture (same precedent as
/// `source_metadata_and_appearance_defaults_reach_scan_metadata` in
/// `scan_real_data.rs`): inject `default-theme-material`/`-texture` into the
/// header's `appearance` object, then convert under the DEFAULT (Core)
/// profile.
#[test]
fn core_profile_export_attaches_dataset_wide_defaults_even_without_sidecars() {
    let mut doc: Value =
        serde_json::from_str(&std::fs::read_to_string(fixture("lod3_railway.city.json")).unwrap())
            .unwrap();
    doc["appearance"]["default-theme-material"] = serde_json::json!("theme-a");
    doc["appearance"]["default-theme-texture"] = serde_json::json!("theme-b");
    let input_dir = tempfile::tempdir().unwrap();
    let input_path = input_dir.path().join("railway_theme.city.json");
    std::fs::write(&input_path, serde_json::to_string(&doc).unwrap()).unwrap();

    let package_dir = tempfile::tempdir().unwrap();
    let report = convert(&ConvertOptions::new(
        input_path,
        package_dir.path().to_path_buf(),
    ))
    .unwrap();
    assert_eq!(
        report.materials_written, 0,
        "precondition: the Core profile never writes sidecars regardless of the header"
    );
    assert_eq!(report.textures_written, 0);
    assert!(!package_dir.path().join("materials.parquet").exists());
    assert!(!package_dir.path().join("textures.parquet").exists());

    let export_dir = tempfile::tempdir().unwrap();
    let output = export_dir.path().join("export.city.jsonl");
    let export_report = export(&ExportOptions {
        package_dir: package_dir.path().to_path_buf(),
        output: output.clone(),
    })
    .unwrap();
    // The Core-profile appearance-refs-dropped behaviour must be completely
    // unaffected by attaching defaults: real geometry material/texture index
    // maps still have no defs sidecar to resolve against.
    assert_eq!(export_report.appearance_refs_dropped, 105);

    let exported = Source::open(&output).unwrap();
    let mut checked = 0usize;
    for feature in exported.features().unwrap() {
        let feature = feature.unwrap();
        let appearance = feature.appearance.as_ref().unwrap_or_else(|| {
            panic!(
                "feature {:?} must carry an appearance block for the dataset-wide defaults",
                feature.id
            )
        });
        assert!(
            appearance.materials.is_none(),
            "no materials sidecar exists to restore real definitions from"
        );
        assert!(appearance.textures.is_none());
        assert!(appearance.vertices_texture.is_none());
        assert_eq!(
            appearance.default_theme_material.as_deref(),
            Some("theme-a")
        );
        assert_eq!(appearance.default_theme_texture.as_deref(), Some("theme-b"));
        checked += 1;
    }
    assert!(checked > 0, "expected at least one exported feature");
}

/// M4 task 10: on a Compatibility-profile package (`geometry_templates.parquet`
/// present), export rebuilds the header's `geometry-templates` and each
/// object's `GeometryInstance` geometry, instead of dropping them. Exercised
/// via the DOC (`.city.json`) output path deliberately — `cjseq_to_cj`'s
/// `add_cjfeature` merges each feature's own appearance into the document
/// appearance, and template `material`/`texture` are localised at HEADER
/// scope beforehand, so this path is the one that could show the header's
/// template indices getting disturbed by that merge (they must not: the
/// header's own materials/textures are installed before any feature is
/// merged in, so their positions never move — see `cjseq::Appearance::
/// add_material`'s dedup-by-value-equality, which only ever APPENDS new
/// entries after existing ones).
#[test]
fn railway_compatibility_export_rebuilds_geometry_templates_and_instances() {
    let package_dir = tempfile::tempdir().unwrap();
    let opts = ConvertOptions::new(
        fixture("lod3_railway.city.json"),
        package_dir.path().to_path_buf(),
    );
    convert(&opts).unwrap();

    let export_dir = tempfile::tempdir().unwrap();
    let output = export_dir.path().join("export.city.json");
    let report = export(&ExportOptions {
        package_dir: package_dir.path().to_path_buf(),
        output: output.clone(),
    })
    .unwrap();

    // (a)
    assert_eq!(
        report.instance_geometries_dropped, 0,
        "templates sidecar is present: instances must be rebuilt, not dropped"
    );

    let text = std::fs::read_to_string(&output).unwrap();
    let doc = cjseq::CityJSON::from_str(&text).expect("cjseq must parse the exported .city.json");

    // Read the SOURCE's own geometry-templates straight from the fixture —
    // types/lods are asserted against this, never hardcoded.
    let source_text = std::fs::read_to_string(fixture("lod3_railway.city.json")).unwrap();
    let source_doc = cjseq::CityJSON::from_str(&source_text).unwrap();
    let source_templates = source_doc
        .geometry_templates
        .as_ref()
        .expect("railway fixture has geometry-templates");
    let source_verts: Vec<Vec<f64>> =
        serde_json::from_value(source_templates.vertices_templates.clone()).unwrap();

    let rebuilt_templates = doc
        .geometry_templates
        .as_ref()
        .expect("exported doc must carry geometry-templates");

    // (b)
    assert_eq!(
        rebuilt_templates.templates.len(),
        source_templates.templates.len(),
        "rebuilt template count must match the source"
    );
    for (i, (rebuilt, source)) in rebuilt_templates
        .templates
        .iter()
        .zip(&source_templates.templates)
        .enumerate()
    {
        assert_eq!(
            rebuilt.thetype, source.thetype,
            "template {i}: type must match the source"
        );
        assert_eq!(
            rebuilt.lod, source.lod,
            "template {i}: lod must match the source"
        );
    }

    // Source object id -> its GeometryInstance's transformationMatrix, read
    // straight from the fixture. Used below to prove each EXPORTED
    // instance's matrix is not just present/well-shaped but actually equal
    // to the matrix its owning object carried in the source (M4 final-review
    // Fix 1's comparator strengthening only matters if the exporter really
    // does preserve this; this structural test pins that independently of
    // `compare`).
    let mut source_matrix_by_object: std::collections::HashMap<String, Vec<f64>> =
        std::collections::HashMap::new();
    for (id, co) in &source_doc.city_objects {
        let Some(geoms) = &co.geometry else { continue };
        for g in geoms {
            if g.thetype != cjseq::GeometryType::GeometryInstance {
                continue;
            }
            let matrix: Vec<f64> = serde_json::from_value(
                g.transformation_matrix
                    .clone()
                    .expect("source GeometryInstance must carry a transformationMatrix"),
            )
            .expect("source transformationMatrix must be an array of numbers");
            source_matrix_by_object.insert(id.clone(), matrix);
        }
    }

    // (c)
    let mut instance_count = 0usize;
    // Per-template instance counts, keyed by the EXPORTED doc's own
    // `template` index numbering (pinned equal to the source's numbering by
    // (b)/(d) above: templates are neither dropped nor reordered for this
    // fixture). Reviewer follow-up (M4 final-review Fix 1): before this fix
    // the comparator only ever checked the reference point, so a bug that
    // silently rewired every instance to template 0 (say) would still pass
    // the headline round-trip gate — pinning the real distribution here
    // closes that gap independently of `compare`.
    let mut instances_per_template: std::collections::HashMap<usize, usize> =
        std::collections::HashMap::new();
    for (id, co) in &doc.city_objects {
        let Some(geoms) = &co.geometry else { continue };
        for g in geoms {
            if g.thetype != cjseq::GeometryType::GeometryInstance {
                continue;
            }
            instance_count += 1;
            let template_idx = g
                .template
                .expect("a GeometryInstance geometry must carry a template index");
            *instances_per_template.entry(template_idx).or_insert(0) += 1;
            let matrix = g
                .transformation_matrix
                .as_ref()
                .expect("GeometryInstance must carry a transformationMatrix")
                .as_array()
                .expect("transformationMatrix must be a JSON array");
            assert_eq!(
                matrix.len(),
                16,
                "transformationMatrix must have 16 elements"
            );
            let boundaries: Vec<usize> = serde_json::from_value(g.boundaries.clone())
                .expect("GeometryInstance boundaries must be a flat array of vertex indices");
            assert_eq!(
                boundaries.len(),
                1,
                "GeometryInstance boundaries must reference exactly one vertex"
            );

            let exported_matrix: Vec<f64> =
                serde_json::from_value(serde_json::Value::Array(matrix.clone()))
                    .expect("transformationMatrix must be an array of numbers");
            let source_matrix = source_matrix_by_object.get(id).unwrap_or_else(|| {
                panic!("exported object {id} has no matching source GeometryInstance object")
            });
            assert_eq!(
                &exported_matrix, source_matrix,
                "object {id}: exported transformationMatrix must equal the source object's"
            );
        }
    }
    assert_eq!(
        instance_count, 15,
        "the recount in decode_real_data.rs: exactly 15 objects carry a template"
    );
    assert_eq!(
        instances_per_template,
        std::collections::HashMap::from([(0, 10), (1, 4), (2, 1)]),
        "railway's known per-template instance distribution must survive export exactly"
    );

    // (d) walk EVERY template's first ring (all 3 of railway's templates are
    // MultiSurface-shaped, per the fixture) through both the rebuilt and
    // source boundary trees and compare coordinates bitwise; index identity
    // is NOT required (the rebuilt vertices-templates pool may assign
    // different positions), only coordinate identity. Widened from
    // template 0 only (the controller's M4 task 10 review) so a bug
    // isolated to template 1 or 2's own coordinate rewrite can't hide behind
    // template 0 happening to round-trip correctly.
    let rebuilt_verts: Vec<Vec<f64>> =
        serde_json::from_value(rebuilt_templates.vertices_templates.clone()).unwrap();
    for t in 0..source_templates.templates.len() {
        let rebuilt_boundaries: Vec<Vec<Vec<usize>>> =
            serde_json::from_value(rebuilt_templates.templates[t].boundaries.clone())
                .unwrap_or_else(|e| {
                    panic!("template {t} must be a MultiSurface-shaped boundary tree: {e}")
                });
        let source_boundaries: Vec<Vec<Vec<usize>>> =
            serde_json::from_value(source_templates.templates[t].boundaries.clone()).unwrap();
        let rebuilt_ring0 = &rebuilt_boundaries[0][0];
        let source_ring0 = &source_boundaries[0][0];
        assert_eq!(
            rebuilt_ring0.len(),
            source_ring0.len(),
            "template {t}'s first ring must keep its source vertex count"
        );
        for (i, &rebuilt_idx) in rebuilt_ring0.iter().enumerate() {
            let source_idx = source_ring0[i];
            assert_eq!(
                rebuilt_verts[rebuilt_idx], source_verts[source_idx],
                "template {t} ring vertex {i}: rebuilt coordinate must equal the source's bitwise"
            );
        }
    }
}

/// Controller addition A (M4 task 10 review): a `template` reference that
/// names a row `geometry_templates.parquet` no longer carries must be a
/// `Schema` error naming both the dangling object and the missing template
/// id — never a panic or a silently-dropped/fabricated geometry. Derived
/// from a real converted railway package (sanctioned): the fixture's own
/// python recount (`{0: 10, 1: 4, 2: 1}` GeometryInstance-per-template
/// counts) confirms template id `"2"` is referenced by 1 real object, so
/// dropping ONLY its (trailing) sidecar row is guaranteed to hit the
/// dangling-reference path on export, not silently succeed because nothing
/// happened to reference it. The trailing row specifically (rather than the
/// heavier-referenced `"0"`, used before the M4 Codex-review Finding 2
/// hardening) — `read_templates` now additionally validates that every row's
/// `id` equals its position (dense `"0".."n"`, no gaps/duplicates); dropping
/// a middle row would trip THAT check first instead of reaching the
/// dangling-reference path this test targets, so the corruption must leave
/// the surviving rows densely numbered from `"0"`.
#[test]
fn export_errors_on_a_dangling_template_id_reference() {
    let package_dir = tempfile::tempdir().unwrap();
    let opts = ConvertOptions::new(
        fixture("lod3_railway.city.json"),
        package_dir.path().to_path_buf(),
    );
    convert(&opts).unwrap();

    let templates_path = package_dir.path().join("geometry_templates.parquet");
    let rows = read_templates(&templates_path).unwrap();
    assert_eq!(
        rows.len(),
        3,
        "railway must carry exactly 3 geometry templates (pinned elsewhere)"
    );
    let corrupted: Vec<_> = rows.into_iter().filter(|r| r.id != "2").collect();
    assert_eq!(
        corrupted.len(),
        2,
        "removing template id \"2\" must leave exactly the other 2 (still densely \"0\", \"1\") rows"
    );
    write_templates(&templates_path, &corrupted).unwrap();

    let export_dir = tempfile::tempdir().unwrap();
    let output = export_dir.path().join("export.city.jsonl");
    let err = export(&ExportOptions {
        package_dir: package_dir.path().to_path_buf(),
        output,
    })
    .unwrap_err();

    assert!(
        matches!(err, CityParquetError::Schema(_)),
        "expected a Schema error, got {err:?}"
    );
    let msg = err.to_string();
    assert!(
        msg.contains("object "),
        "the error must name the dangling object, got: {msg}"
    );
    assert!(
        msg.contains("\"2\""),
        "the error must name the missing template id \"2\", got: {msg}"
    );
}

/// M4 final-review Fix 5: a main-table `material` index that has gone out of
/// range of `materials.parquet` (a corrupted/truncated Compatibility
/// package — the sidecar the export-side `LocalAppearance` restore
/// dereferences global ids against) must be a `Schema` error naming the
/// offending object, never a panic or a silently-dropped/fabricated
/// material. Derived from a real converted railway package (sanctioned):
/// truncating `materials.parquet` alone is not enough to isolate the
/// per-OBJECT path on this fixture — railway's own geometry templates
/// happen to reference the two HIGHEST dataset-global material ids (83 and
/// 84 of 85, confirmed by probing the converted package's own
/// `geometry_templates.parquet`), so any truncation that stops short of
/// keeping every definition also strands a template reference, and the
/// header-level template rebuild (which runs before the per-object loop)
/// would report that instead. The template rows' own `material`/`texture`
/// are therefore additionally cleared (a template legitimately carries
/// neither) so the truncation's effect is isolated to real objects, which
/// is what this fix targets — the per-object appearance restore, not the
/// template one (already covered by the dangling-template-id test above).
#[test]
fn export_errors_on_an_out_of_range_material_global_id() {
    let package_dir = tempfile::tempdir().unwrap();
    let opts = ConvertOptions::new(
        fixture("lod3_railway.city.json"),
        package_dir.path().to_path_buf(),
    );
    convert(&opts).unwrap();

    let materials_path = package_dir.path().join("materials.parquet");
    let defs = read_materials(&materials_path).unwrap();
    assert_eq!(
        defs.len(),
        85,
        "railway must carry exactly 85 material definitions (pinned elsewhere)"
    );

    // Clear every template row's material/texture reference so truncating
    // materials.parquet cannot also strand the header-level template
    // rebuild (see the doc comment above).
    let templates_path = package_dir.path().join("geometry_templates.parquet");
    let mut template_rows = read_templates(&templates_path).unwrap();
    assert_eq!(template_rows.len(), 3, "railway has 3 geometry templates");
    assert!(
        template_rows.iter().any(|r| r.material.is_some()),
        "precondition: at least one template must actually reference a material \
         (or clearing it below would be a no-op)"
    );
    for row in &mut template_rows {
        row.material = None;
        row.texture = None;
    }
    write_templates(&templates_path, &template_rows).unwrap();

    // Truncate to a single definition (id 0): any real object referencing a
    // higher index is now dangling.
    let truncated = &defs[..1];
    write_materials(&materials_path, truncated).unwrap();
    assert_eq!(read_materials(&materials_path).unwrap().len(), 1);

    let export_dir = tempfile::tempdir().unwrap();
    let output = export_dir.path().join("export.city.jsonl");
    let err = export(&ExportOptions {
        package_dir: package_dir.path().to_path_buf(),
        output,
    })
    .unwrap_err();

    assert!(
        matches!(err, CityParquetError::Schema(_)),
        "expected a Schema error, got {err:?}"
    );
    let msg = err.to_string();
    assert!(
        msg.contains("object "),
        "the error must name the offending object, got: {msg}"
    );
    assert!(
        msg.contains("material global id"),
        "the error must name the out-of-range material global id, got: {msg}"
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

/// Read `metadata.json` back as the STAC Item it now is (Plan 2b) — the
/// counterpart to [`write_item`] the adversarial tests below use to inspect
/// and mutate a package's asset inventory.
fn read_item(package_dir: &std::path::Path) -> Item {
    let text = std::fs::read_to_string(package_dir.join("metadata.json")).unwrap();
    serde_json::from_str(&text).unwrap()
}

fn write_item(package_dir: &std::path::Path, item: &Item) {
    let text = serde_json::to_string_pretty(item).unwrap();
    std::fs::write(package_dir.join("metadata.json"), text).unwrap();
}

/// Whether `item` carries an asset with `role`, whose `href` (the
/// authoritative locator per `PackageTables::open`'s doc comment, not the
/// asset's map key) resolves to `name`.
fn has_asset_with_role(item: &Item, role: &str, name: &str) -> bool {
    item.assets
        .values()
        .any(|a| a.roles.iter().any(|r| r == role) && a.href.trim_start_matches("./") == name)
}

/// A `cityparquet-objects`/`cityparquet-sidecar` asset pointing at `./{name}`,
/// built the same way [`cityparquet::stac::mod::package_asset`] builds one —
/// media type plus the `data` + role-specific roles pair — so the adversarial
/// Items constructed below exercise exactly the shape `PackageTables::open`
/// expects, not a hand-simplified stand-in for it.
fn table_asset(name: &str, role: &str) -> Asset {
    Asset::new(format!("./{name}"))
        .with_media_type(PARQUET_MEDIA_TYPE)
        .with_roles(["data".to_string(), role.to_string()])
}

/// M4 Codex-review Finding 1(a): the package manifest is authoritative for
/// whether `geometry_templates.parquet` should be loaded — mirroring how
/// `materials.parquet`/`textures.parquet` are already gated. When the
/// manifest LISTS the sidecar but the file has been deleted (a
/// truncated/tampered package), export must fail loudly, never silently fall
/// back to dropping every instance geometry as if the profile carried no
/// templates at all. Derived from a real converted railway Compatibility
/// package (sanctioned): the manifest is left untouched, only the sidecar
/// file itself is removed.
#[test]
fn export_errors_when_manifest_lists_templates_but_the_sidecar_file_is_missing() {
    let package_dir = tempfile::tempdir().unwrap();
    let opts = ConvertOptions::new(
        fixture("lod3_railway.city.json"),
        package_dir.path().to_path_buf(),
    );
    convert(&opts).unwrap();

    let item = read_item(package_dir.path());
    assert!(
        has_asset_with_role(&item, ROLE_SIDECAR, "geometry_templates.parquet"),
        "precondition: the Compatibility Item carries a cityparquet-sidecar asset for \
         geometry_templates.parquet"
    );

    let templates_path = package_dir.path().join("geometry_templates.parquet");
    assert!(templates_path.exists());
    std::fs::remove_file(&templates_path).unwrap();

    let export_dir = tempfile::tempdir().unwrap();
    let output = export_dir.path().join("export.city.jsonl");
    let err = export(&ExportOptions {
        package_dir: package_dir.path().to_path_buf(),
        output,
    })
    .unwrap_err();

    assert!(
        matches!(err, CityParquetError::Io(_) | CityParquetError::Schema(_)),
        "expected an Io or Schema error naming the missing manifest-listed sidecar, got {err:?}"
    );
    assert!(
        err.to_string().contains("geometry_templates.parquet"),
        "the error must name the missing sidecar file, got: {err}"
    );
}

/// M4 Codex-review Finding 1(b): the inverse of the test above — when the
/// manifest does NOT list `geometry_templates.parquet` (edited out of
/// `sidecar_files`) but a `geometry_templates.parquet` file is still sitting
/// on disk (e.g. left over from a prior write, or planted by a third party),
/// export must ignore it outright and fall back to the counted-drop path,
/// exactly as if the file were never there — the manifest is the sole source
/// of truth, never the file's mere presence.
#[test]
fn export_ignores_an_unlisted_geometry_templates_file_left_on_disk() {
    let package_dir = tempfile::tempdir().unwrap();
    let opts = ConvertOptions::new(
        fixture("lod3_railway.city.json"),
        package_dir.path().to_path_buf(),
    );
    convert(&opts).unwrap();

    let mut item = read_item(package_dir.path());
    let before = item.assets.len();
    item.assets.retain(|_, a| {
        !(a.roles.iter().any(|r| r == ROLE_SIDECAR)
            && a.href.trim_start_matches("./") == "geometry_templates.parquet")
    });
    assert_eq!(
        item.assets.len(),
        before - 1,
        "precondition: the geometry_templates.parquet asset must actually have been removed"
    );
    write_item(package_dir.path(), &item);

    // The sidecar file itself is left on disk, untouched.
    assert!(
        package_dir
            .path()
            .join("geometry_templates.parquet")
            .exists()
    );

    let export_dir = tempfile::tempdir().unwrap();
    let output = export_dir.path().join("export.city.jsonl");
    let report = export(&ExportOptions {
        package_dir: package_dir.path().to_path_buf(),
        output,
    })
    .unwrap();

    assert_eq!(
        report.instance_geometries_dropped, 15,
        "an unlisted geometry_templates.parquet file on disk must be ignored: every \
         GeometryInstance-bearing object must fall back to the counted-drop path, matching a \
         package that never wrote the sidecar at all (pinned in \
         railway_exports_dropping_instance_geometries_but_keeping_their_objects)"
    );
}

/// M5 task 5 (Step 1, guard half): a manifest naming the same table twice is
/// a corrupt package — reading it would decode every object in that table
/// twice — so `export` must reject it outright rather than silently
/// tolerating it (which the pre-M5 `tables.first()`-only read happened to do,
/// since it never looked at the second entry at all).
#[test]
fn export_rejects_a_manifest_listing_the_same_table_twice() {
    let package_dir = tempfile::tempdir().unwrap();
    convert(&ConvertOptions::new(
        fixture("delft.city.jsonl"),
        package_dir.path().to_path_buf(),
    ))
    .unwrap();

    // delft is a single 1st-level family, so by-type conversion writes
    // exactly one main table: building.parquet.
    let mut item = read_item(package_dir.path());
    let object_table_hrefs: Vec<String> = item
        .assets
        .values()
        .filter(|a| a.roles.iter().any(|r| r == ROLE_OBJECT_TABLE))
        .map(|a| a.href.clone())
        .collect();
    assert_eq!(object_table_hrefs, vec!["./building.parquet".to_string()]);
    // A second asset, under a distinct map key, whose `href` resolves to the
    // SAME file — `PackageTables::open` dedups on the href-derived name, not
    // the map key, so this is what actually reaches its duplicate check.
    item.assets.insert(
        "building-duplicate.parquet".to_string(),
        table_asset("building.parquet", ROLE_OBJECT_TABLE),
    );
    write_item(package_dir.path(), &item);

    let export_dir = tempfile::tempdir().unwrap();
    let output = export_dir.path().join("export.city.jsonl");
    let error = export(&ExportOptions {
        package_dir: package_dir.path().to_path_buf(),
        output,
    })
    .unwrap_err();
    let msg = error.to_string();
    assert!(
        msg.contains("duplicate") && msg.contains("building.parquet"),
        "expected an error naming the duplicated table, got: {msg}"
    );
}

/// Splits `package_dir`'s single `building.parquet` main table (delft is a
/// single 1st-level family, so by-type conversion writes exactly one) into
/// two physically separate parquet files inside `package_dir`
/// (`building_a.parquet` / `building_b.parquet`, first half of rows /
/// second half), each carrying the identical Arrow schema and KV footer
/// metadata as the original file, then removes the original. A hand-rolled
/// stand-in for a MULTI-family by-type package (e.g. railway's 10 tables),
/// used to exercise export's multi-table read loop (M5 task 5, Step 1)
/// independently of the by-type writer path itself. Returns the two bare
/// file names, in the order the caller should list them in a rewritten
/// manifest.
fn split_main_table_into_two_files(package_dir: &std::path::Path) -> (String, String) {
    let source_path = package_dir.join("building.parquet");
    let file = std::fs::File::open(&source_path).unwrap();
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
    let kvs = builder
        .metadata()
        .file_metadata()
        .key_value_metadata()
        .cloned()
        .unwrap_or_default();
    let schema = Arc::clone(builder.schema());
    let reader = builder.build().unwrap();
    let batches: Vec<RecordBatch> = reader.map(|b| b.unwrap()).collect();
    let combined = if batches.len() == 1 {
        batches.into_iter().next().unwrap()
    } else {
        arrow_select::concat::concat_batches(&schema, &batches).unwrap()
    };
    let total = combined.num_rows();
    let mid = total / 2;
    let first = combined.slice(0, mid);
    let second = combined.slice(mid, total - mid);

    let props = WriterProperties::builder()
        .set_key_value_metadata(Some(kvs))
        .build();
    let name_a = "building_a.parquet".to_string();
    let name_b = "building_b.parquet".to_string();
    for (name, batch) in [(&name_a, &first), (&name_b, &second)] {
        let out_file = std::fs::File::create(package_dir.join(name)).unwrap();
        let mut writer =
            ArrowWriter::try_new(out_file, Arc::clone(&schema), Some(props.clone())).unwrap();
        writer.write(batch).unwrap();
        writer.close().unwrap();
    }
    std::fs::remove_file(&source_path).unwrap();
    (name_a, name_b)
}

/// M5 task 5 (Step 1, the real case): a manifest listing MULTIPLE distinct
/// tables must have every one of them read, not just the first — the bug
/// the pre-M5 `tables.first()`-only read had (half of delft's objects, the
/// ones in the second physical file, would simply vanish from the export).
#[test]
fn export_reads_every_table_a_manifest_lists_not_just_the_first() {
    let package_dir = tempfile::tempdir().unwrap();
    convert(&ConvertOptions::new(
        fixture("delft.city.jsonl"),
        package_dir.path().to_path_buf(),
    ))
    .unwrap();

    let (name_a, name_b) = split_main_table_into_two_files(package_dir.path());
    let mut item = read_item(package_dir.path());
    // The original single `building.parquet` object-table asset now names a
    // file that no longer exists (it was split and removed above); replace
    // it with assets for the two physical files that replaced it, in the
    // order the original manifest-based test listed them.
    item.assets
        .retain(|_, a| !a.roles.iter().any(|r| r == ROLE_OBJECT_TABLE));
    for name in [&name_a, &name_b] {
        item.assets
            .insert(name.clone(), table_asset(name, ROLE_OBJECT_TABLE));
    }
    write_item(package_dir.path(), &item);

    let export_dir = tempfile::tempdir().unwrap();
    let output = export_dir.path().join("export.city.jsonl");
    let report = export(&ExportOptions {
        package_dir: package_dir.path().to_path_buf(),
        output: output.clone(),
    })
    .unwrap();
    assert_eq!(
        report.object_count, 2231,
        "both split tables' objects must be read, not just the first table's"
    );

    let compare_report = compare_datasets(
        &fixture("delft.city.jsonl"),
        &output,
        &CompareOptions::default(),
    )
    .unwrap();
    assert!(
        compare_report.equal,
        "a manifest-listed second table's rows must round-trip losslessly too; differences: {:#?}",
        compare_report.differences
    );
    assert!(compare_report.differences.is_empty());
}

/// Spec-alignment (per-module Arrow/Parquet schema pruning): TWO tables in
/// ONE valid CityParquet package can legitimately carry different schemas —
/// each object table needs only the geometry/appearance columns its own rows
/// populate (spec "object-table-schema": "a table carries exactly the LoD
/// columns its data needs"). `export` must therefore decode EVERY table
/// against its OWN footer metadata and its OWN rendered Arrow schema, never
/// the first table's — this test proves that end to end with a hand-rolled
/// stand-in for "two tables, genuinely different schemas": a real delft
/// package split into two physical files (`split_main_table_into_two_files`,
/// also used by `export_reads_every_table_a_manifest_lists_not_just_the_first`),
/// then the SECOND file rewritten with its first attribute column renamed —
/// both in its own Arrow schema AND in its own rewritten KV metadata's
/// `attribute_columns` list, so the table stays internally self-consistent
/// (`cityparquet_arrow_schema()` resolves it cleanly on its own terms) even
/// though it now genuinely differs from the first table.
///
/// Before the per-table decode fix, `export` rejected this outright (a
/// dataset-wide "every table must share the first table's schema"
/// assumption — see the M5 Codex review this test used to document, and this
/// task's own regression proving the assumption became actively wrong the
/// moment per-module schemas could differ for real). Now it must succeed,
/// and each table's rows must come back under THEIR OWN table's attribute
/// name: table A's objects keep the original name, table B's carry the
/// renamed one.
#[test]
fn export_decodes_a_second_table_with_a_renamed_attribute_column_using_its_own_metadata() {
    let package_dir = tempfile::tempdir().unwrap();
    convert(&ConvertOptions::new(
        fixture("delft.city.jsonl"),
        package_dir.path().to_path_buf(),
    ))
    .unwrap();

    let (name_a, name_b) = split_main_table_into_two_files(package_dir.path());

    let table_b_path = package_dir.path().join(&name_b);
    let file = std::fs::File::open(&table_b_path).unwrap();
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
    let kvs = builder
        .metadata()
        .file_metadata()
        .key_value_metadata()
        .cloned()
        .unwrap_or_default();
    let schema = Arc::clone(builder.schema());
    let reader = builder.build().unwrap();
    let batches: Vec<RecordBatch> = reader.map(|b| b.unwrap()).collect();
    let combined = if batches.len() == 1 {
        batches.into_iter().next().unwrap()
    } else {
        arrow_select::concat::concat_batches(&schema, &batches).unwrap()
    };

    let attr_kv = kvs
        .iter()
        .find(|kv| kv.key == "attributes")
        .expect("delft's metadata must carry an attributes entry (§13.1)");
    let mut attrs: Vec<String> = serde_json::from_str(
        attr_kv
            .value
            .as_deref()
            .expect("the `attributes` KV must carry a value"),
    )
    .unwrap();
    assert!(
        !attrs.is_empty(),
        "delft must have at least one attribute column to rename"
    );
    let old_name = attrs[0].clone();
    let new_name = format!("{old_name}_renamed");
    attrs[0] = new_name.clone();

    let new_kvs: Vec<KeyValue> = kvs
        .into_iter()
        .map(|kv| {
            if kv.key == "attributes" {
                KeyValue::new(
                    "attributes".to_string(),
                    serde_json::to_string(&attrs).unwrap(),
                )
            } else {
                kv
            }
        })
        .collect();

    let renamed_schema = Arc::new(Schema::new(
        schema
            .fields()
            .iter()
            .map(|f| {
                if f.name() == &old_name {
                    Arc::new(
                        Field::new(new_name.clone(), f.data_type().clone(), f.is_nullable())
                            .with_metadata(f.metadata().clone()),
                    )
                } else {
                    Arc::clone(f)
                }
            })
            .collect::<Vec<_>>(),
    ));
    let renamed_batch =
        RecordBatch::try_new(Arc::clone(&renamed_schema), combined.columns().to_vec()).unwrap();

    let props = WriterProperties::builder()
        .set_key_value_metadata(Some(new_kvs))
        .build();
    let out_file = std::fs::File::create(&table_b_path).unwrap();
    let mut writer =
        ArrowWriter::try_new(out_file, Arc::clone(&renamed_schema), Some(props)).unwrap();
    writer.write(&renamed_batch).unwrap();
    writer.close().unwrap();

    let mut item = read_item(package_dir.path());
    item.assets
        .retain(|_, a| !a.roles.iter().any(|r| r == ROLE_OBJECT_TABLE));
    for name in [&name_a, &name_b] {
        item.assets
            .insert(name.clone(), table_asset(name, ROLE_OBJECT_TABLE));
    }
    write_item(package_dir.path(), &item);

    let export_dir = tempfile::tempdir().unwrap();
    let output = export_dir.path().join("export.city.jsonl");
    let report = export(&ExportOptions {
        package_dir: package_dir.path().to_path_buf(),
        output: output.clone(),
    })
    .expect(
        "two tables with genuinely different (but each internally self-consistent) schemas \
             must decode successfully, each against its own metadata",
    );
    assert_eq!(
        report.object_count, 2231,
        "both split tables' objects must still be read, not just the first table's"
    );

    // Every object exported carries EITHER the original attribute name (rows
    // that came from table A, decoded with table A's own metadata) OR the
    // renamed one (rows from table B, decoded with table B's own metadata) —
    // proof that decoding is genuinely per-table, not the first table's
    // metadata reused for both.
    let text = std::fs::read_to_string(&output).unwrap();
    let mut saw_old_name = false;
    let mut saw_new_name = false;
    for line in text.lines().skip(1) {
        let feature: serde_json::Value = serde_json::from_str(line).unwrap();
        for (_, co) in feature["CityObjects"].as_object().unwrap() {
            let Some(attrs) = co.get("attributes").and_then(|a| a.as_object()) else {
                continue;
            };
            if attrs.contains_key(&old_name) {
                saw_old_name = true;
            }
            if attrs.contains_key(&new_name) {
                saw_new_name = true;
            }
            assert!(
                !(attrs.contains_key(&old_name) && attrs.contains_key(&new_name)),
                "a single object must never carry both the old and the renamed attribute name"
            );
        }
    }
    assert!(
        saw_old_name,
        "table A's objects must still carry the attribute under its own (original) name '{old_name}'"
    );
    assert!(
        saw_new_name,
        "table B's objects must be decoded using ITS OWN metadata's renamed attribute name \
         '{new_name}', not table A's"
    );
}
